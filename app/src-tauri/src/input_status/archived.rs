//! Closed-recording delivery has no current-input projection. Only DB-owned
//! terminal sessions may migrate the live ledger; actual capture is never read,
//! restarted or rebound here. Companion fsync holds neither SQLite nor recorder.
use super::{
    journal,
    queue::{Action, ArchiveQueue, ArchivedBatch},
    Receipt,
};
use crate::{
    analysis_worker::{data_maintenance::NoticeStorageLease, AnalysisController},
    storage::Storage,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::{
    collections::BTreeMap,
    fs::File,
    sync::{Arc, Mutex},
};

const CACHE_LIMIT: usize = 128;
pub(crate) const PAUSED: &str = "notification_history_paused";

#[derive(Default)]
pub(crate) struct ArchivedInputs(Mutex<BTreeMap<String, CachedEntry>>);
struct CachedEntry {
    meeting: String,
    touched: Instant,
    value: Arc<Mutex<Entry>>,
}
struct Entry {
    meeting: String,
    directory: File,
    reader: journal::Reader,
    queue: Option<ArchiveQueue>,
    touched: Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    schema_version: u8,
    recordings: Vec<Row>,
    next_cursor: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    meeting_id: String,
    session_id: String,
    title: String,
    batch: Option<ArchivedBatch>,
    issue: Option<String>,
    history_save_issue: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Acknowledge {
    pub(crate) meeting_id: String,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: u64,
    action: AckAction,
    receive_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum AckAction {
    Receive,
    Render,
    Close,
}

fn same_directory(a: &File, b: &File) -> Result<(), String> {
    #[cfg(windows)]
    {
        if a.metadata().map_err(|e| e.to_string())?.is_dir()
            && b.metadata().map_err(|e| e.to_string())?.is_dir()
            && crate::windows_files::Identity::file(a)? == crate::windows_files::Identity::file(b)?
        {
            return Ok(());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = a.metadata().map_err(|e| e.to_string())?;
        let b = b.metadata().map_err(|e| e.to_string())?;
        if a.is_dir()
            && b.is_dir()
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.uid() == b.uid()
        {
            return Ok(());
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (a, b);
    Err("保存後通知の管理先が変更されたか、このOSでは未対応です".into())
}
impl Entry {
    fn source(&mut self, owner: &str) -> (Option<journal::Record>, Option<&'static str>) {
        match crate::input_levels::open_capture_file_at_optional(
            &self.directory,
            "input-events.json",
        ) {
            Ok(None) => (None, Some("missing")),
            Err(_) => (None, Some("invalid")),
            Ok(Some(_)) => match self.reader.read_record(owner) {
                Ok(record) => (Some(record), None),
                Err(_) => (None, Some("invalid")),
            },
        }
    }
    fn refresh(
        &mut self,
        owner: &str,
        lease: &NoticeStorageLease,
    ) -> Result<(Option<journal::Record>, Option<&'static str>), String> {
        let (source, issue) = self.source(owner);
        lease.validate_active()?;
        if self.queue.is_none() {
            self.queue = ArchiveQueue::open(&self.directory, owner, source.clone(), issue)?;
        }
        Ok((source, issue))
    }
}
impl ArchivedInputs {
    fn entry(
        &self,
        storage: &Storage,
        lease: &NoticeStorageLease,
        meeting: &str,
        owner: &str,
    ) -> Result<Arc<Mutex<Entry>>, String> {
        lease.validate_active()?;
        let directory = storage.closed_input_directory(meeting, owner)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "保存後通知の所有を取得できません")?;
        let value = if let Some(cached) = entries.get_mut(owner) {
            if cached.meeting != meeting {
                return Err("保存後通知の会議が変更されています".into());
            }
            cached.touched = Instant::now();
            cached.value.clone()
        } else {
            if entries.len() >= CACHE_LIMIT {
                let old = entries
                    .iter()
                    .filter(|(_, e)| Arc::strong_count(&e.value) == 1)
                    .min_by_key(|(_, e)| e.touched)
                    .map(|(key, _)| key.clone());
                if let Some(old) = old {
                    entries.remove(&old);
                } else {
                    return Err("通知履歴の処理数が上限に達しました".into());
                }
            }
            let reader = journal::Reader::pin_at(&directory)?;
            let value = Arc::new(Mutex::new(Entry {
                meeting: meeting.into(),
                directory: directory.try_clone().map_err(|e| e.to_string())?,
                reader,
                queue: None,
                touched: Instant::now(),
            }));
            entries.insert(
                owner.into(),
                CachedEntry {
                    meeting: meeting.into(),
                    touched: Instant::now(),
                    value: value.clone(),
                },
            );
            value
        };
        drop(entries);
        same_directory(
            &value
                .lock()
                .map_err(|_| "保存後通知の入力を確認できません")?
                .directory,
            &directory,
        )?;
        lease.validate_active()?;
        Ok(value)
    }
    fn verify(
        storage: &Storage,
        lease: &NoticeStorageLease,
        entry: &Entry,
        owner: &str,
    ) -> Result<(), String> {
        lease.validate_active()?;
        same_directory(
            &entry.directory,
            &storage.closed_input_directory(&entry.meeting, owner)?,
        )
    }

    pub(crate) fn evict(&self, meeting: Option<&str>) -> Result<(), String> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "通知履歴のキャッシュを解放できません")?;
        entries.retain(|_, entry| meeting.is_some_and(|id| entry.meeting != id));
        Ok(())
    }

    pub(crate) fn page_scoped(
        &self,
        storage: &Storage,
        analysis: &AnalysisController,
        after: Option<&str>,
        visible: &impl Fn() -> bool,
    ) -> Result<Page, String> {
        self.page_inner(storage, None, Some(analysis), after, visible)
    }

    pub(crate) fn page(
        &self,
        storage: &Storage,
        lease: &NoticeStorageLease,
        after: Option<&str>,
        visible: &impl Fn() -> bool,
    ) -> Result<Page, String> {
        self.page_inner(storage, Some(lease), None, after, visible)
    }

    fn page_inner(
        &self,
        storage: &Storage,
        fallback: Option<&NoticeStorageLease>,
        analysis: Option<&AnalysisController>,
        after: Option<&str>,
        visible: &impl Fn() -> bool,
    ) -> Result<Page, String> {
        if let Some(lease) = fallback {
            lease.validate_active()?;
        }
        let discovery = match analysis {
            Some(a) => Some(a.try_begin_scoped_notice_storage(None)?.ok_or(PAUSED)?),
            None => None,
        };
        let (rows, next_cursor) = storage.closed_input_recordings(after)?;
        drop(discovery);
        let mut recordings = Vec::with_capacity(rows.len());
        let mut batch_times = Vec::with_capacity(rows.len());
        for row in rows {
            let owned =
                match analysis.map(|a| a.try_begin_scoped_notice_storage(Some(&row.meeting_id))) {
                    Some(Ok(Some(lease))) => Some(lease),
                    Some(Ok(None)) => {
                        recordings.push(Row {
                            meeting_id: row.meeting_id,
                            session_id: row.session_id,
                            title: row.title,
                            batch: None,
                            issue: Some("paused".into()),
                            history_save_issue: None,
                        });
                        batch_times.push(Instant::now());
                        continue;
                    }
                    Some(Err(error)) => return Err(error),
                    None => None,
                };
            let lease = owned
                .as_ref()
                .or(fallback)
                .ok_or("通知履歴の所有がありません")?;
            let imported = match storage.closed_input_snapshot(&row.meeting_id, &row.session_id) {
                Ok(imported) => imported,
                Err(_) => {
                    recordings.push(Row {
                        meeting_id: row.meeting_id,
                        session_id: row.session_id,
                        title: row.title,
                        batch: None,
                        issue: Some("unavailable".into()),
                        history_save_issue: Some("unavailable".into()),
                    });
                    batch_times.push(Instant::now());
                    lease.validate_active()?;
                    continue;
                }
            };
            if let Some(snapshot) = imported {
                // A DB-bound destination writer, not a producer-directory
                // migration. On failure retain diagnostics but offer no claim.
                let (batch, issue) = match storage.restored_input_page(
                    &row.meeting_id,
                    &row.session_id,
                    lease,
                    visible,
                ) {
                    Ok(batch) => (batch, None),
                    Err(_) => {
                        let (batch, pending) = snapshot.retained_diagnostics(&row.session_id)?;
                        let issue = if pending {
                            "deliveryUnavailable"
                        } else {
                            "unavailable"
                        };
                        (if pending { batch } else { None }, Some(issue.into()))
                    }
                };
                let history_save_issue = snapshot
                    .history_save_issue()
                    .unwrap_or(Some("unavailable"))
                    .map(str::to_owned);
                recordings.push(Row {
                    meeting_id: row.meeting_id,
                    session_id: row.session_id,
                    title: row.title,
                    batch,
                    issue,
                    history_save_issue,
                });
                batch_times.push(Instant::now());
                lease.validate_active()?;
                continue;
            }
            // Final producer durability remains independent of delivery errors.
            let mut history_save_issue = Some("unavailable".into());
            let result = (|| {
                let handle = self.entry(storage, lease, &row.meeting_id, &row.session_id)?;
                let mut entry = handle
                    .lock()
                    .map_err(|_| "保存後通知の入力を確認できません")?;
                let history = super::persistence::archived_issue(&entry.directory, &row.session_id)
                    .map(str::to_owned);
                Self::verify(storage, lease, &entry, &row.session_id)?;
                history_save_issue = history;
                let (source, issue) = entry.refresh(&row.session_id, lease)?;
                Self::verify(storage, lease, &entry, &row.session_id)?;
                let mut batch = entry
                    .queue
                    .as_mut()
                    .map(|q| q.poll(source, issue, visible()))
                    .transpose()?;
                let observed_at = Instant::now();
                Self::verify(storage, lease, &entry, &row.session_id)?;
                if !visible() {
                    if let Some(b) = &mut batch {
                        b.hide_notices();
                    }
                }
                Ok::<_, String>((batch, observed_at))
            })();
            let (batch, issue, at) = match result {
                Ok((batch, at)) => (batch, None, at),
                Err(_) => (None, Some("unavailable".into()), Instant::now()),
            };
            batch_times.push(at);
            recordings.push(Row {
                meeting_id: row.meeting_id,
                session_id: row.session_id,
                title: row.title,
                batch,
                issue,
                history_save_issue,
            });
            lease.validate_active()?;
        }
        let _final_check = match analysis {
            Some(a) => Some(a.try_begin_scoped_notice_storage(None)?.ok_or(PAUSED)?),
            None => None,
        };
        storage.verify_closed_input_storage()?;
        if let Some(lease) = fallback {
            lease.validate_active()?;
        }
        let still_visible = visible();
        for (row, at) in recordings.iter_mut().zip(batch_times) {
            if let Some(batch) = &mut row.batch {
                batch.account_wait(at.elapsed());
                if !still_visible {
                    batch.hide_notices();
                }
            }
        }
        Ok(Page {
            schema_version: 3,
            recordings,
            next_cursor,
        })
    }
    pub(crate) fn acknowledge(
        &self,
        storage: &Storage,
        lease: &NoticeStorageLease,
        request: Acknowledge,
        visible: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        if !visible() {
            return Err("保存後通知のメイン画面が表示されていません".into());
        }
        if !crate::input_levels::valid_owner(&request.recipient_id)
            || request.notice_id.len() > 128
            || !(1..=super::SAFE).contains(&request.generation)
            || (matches!(request.action, AckAction::Receive)
                && !request
                    .receive_id
                    .as_deref()
                    .is_some_and(crate::input_levels::valid_owner))
            || (!matches!(request.action, AckAction::Receive) && request.receive_id.is_some())
        {
            return Err("保存後通知の受付引数が不正です".into());
        }
        let action = match request.action {
            AckAction::Receive => Action::Receive,
            AckAction::Render => Action::Render,
            AckAction::Close => Action::Close,
        };
        if storage
            .closed_input_snapshot(&request.meeting_id, &request.session_id)?
            .is_some()
        {
            return storage.restored_input_acknowledge(
                &request.meeting_id,
                &request.session_id,
                lease,
                &request.notice_id,
                &request.recipient_id,
                request.generation,
                action,
                request.receive_id.as_deref(),
                visible,
            );
        }
        let handle = self.entry(storage, lease, &request.meeting_id, &request.session_id)?;
        let mut entry = handle
            .lock()
            .map_err(|_| "保存後通知の入力を確認できません")?;
        let (source, issue) = entry.refresh(&request.session_id, lease)?;
        Self::verify(storage, lease, &entry, &request.session_id)?;
        let queue = entry
            .queue
            .as_mut()
            .ok_or("この録音には保存後通知がありません")?;
        queue.poll(source, issue, visible())?;
        let mut receipt = queue.act(
            &request.notice_id,
            &request.recipient_id,
            request.generation,
            action,
            request.receive_id.as_deref(),
            visible(),
        )?;
        let committed_at = Instant::now();
        Self::verify(storage, lease, &entry, &request.session_id)?;
        if !visible() {
            return Err("保存後通知の応答前にメイン画面が非表示になりました".into());
        }
        if let Some(delivery) = &mut receipt.delivery {
            delivery.account_persistence_time(committed_at.elapsed());
        }
        Ok(receipt)
    }
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod windows_tests;
