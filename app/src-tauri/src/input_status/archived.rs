//! Closed-recording delivery has no current-input projection. Only DB-owned
//! terminal sessions may migrate the live ledger; actual capture is never read,
//! restarted or rebound here. Companion fsync holds neither SQLite nor recorder.
use super::{
    journal,
    queue::{Action, ArchiveQueue, ArchivedBatch},
    Receipt,
};
use crate::{analysis_worker::data_maintenance::NoticeStorageLease, storage::Storage};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::{collections::BTreeMap, fs::File, sync::Mutex};

const CACHE_LIMIT: usize = 128;

#[derive(Default)]
pub(crate) struct ArchivedInputs(Mutex<BTreeMap<String, Entry>>);
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
    meeting_id: String,
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
    #[cfg(not(unix))]
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
    fn entry<'a>(
        entries: &'a mut BTreeMap<String, Entry>,
        storage: &Storage,
        lease: &NoticeStorageLease,
        meeting: &str,
        owner: &str,
    ) -> Result<&'a mut Entry, String> {
        lease.validate_active()?;
        let directory = storage.closed_input_directory(meeting, owner)?;
        if let Some(entry) = entries.get(owner) {
            if entry.meeting != meeting {
                return Err("保存後通知の会議が変更されています".into());
            }
            same_directory(&entry.directory, &directory)?;
        } else {
            // Dropping a cached owner releases its writer. A later reopen fences
            // old WebView claims and suppresses durably rendered heads.
            if entries.len() >= CACHE_LIMIT {
                if let Some(old) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.touched)
                    .map(|(key, _)| key.clone())
                {
                    entries.remove(&old);
                }
            }
            let reader = journal::Reader::pin_at(&directory)?;
            entries.insert(
                owner.into(),
                Entry {
                    meeting: meeting.into(),
                    directory,
                    reader,
                    queue: None,
                    touched: Instant::now(),
                },
            );
        }
        lease.validate_active()?;
        let entry = entries.get_mut(owner).unwrap();
        entry.touched = Instant::now();
        Ok(entry)
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

    pub(crate) fn page(
        &self,
        storage: &Storage,
        lease: &NoticeStorageLease,
        after: Option<&str>,
        visible: &impl Fn() -> bool,
    ) -> Result<Page, String> {
        lease.validate_active()?;
        let (rows, next_cursor) = storage.closed_input_recordings(after)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "保存後通知の所有を取得できません")?;
        let mut recordings = Vec::with_capacity(rows.len());
        let mut batch_times = Vec::with_capacity(rows.len());
        for row in rows {
            // Independent from the delivery ledger and its revision. A readable
            // retained suffix does not prove that the final writer committed.
            let history_save_issue = Self::entry(
                &mut entries,
                storage,
                lease,
                &row.meeting_id,
                &row.session_id,
            )
            .and_then(|entry| {
                let issue = super::persistence::archived_issue(&entry.directory, &row.session_id);
                Self::verify(storage, lease, entry, &row.session_id)?;
                Ok(issue.map(str::to_owned))
            })
            .unwrap_or(Some("unavailable".into()));
            let batch = (|| {
                let entry = Self::entry(
                    &mut entries,
                    storage,
                    lease,
                    &row.meeting_id,
                    &row.session_id,
                )?;
                let (source, issue) = entry.refresh(&row.session_id, lease)?;
                Self::verify(storage, lease, entry, &row.session_id)?;
                let mut batch = entry
                    .queue
                    .as_mut()
                    .map(|q| q.poll(source, issue, visible()))
                    .transpose()?;
                let observed_at = Instant::now();
                Self::verify(storage, lease, entry, &row.session_id)?;
                if !visible() {
                    if let Some(b) = &mut batch {
                        b.hide_notices();
                    }
                }
                Ok::<_, String>((batch, observed_at))
            })();
            // A broken/missing one-owner directory must not erase other rows or
            // turn a valid-but-empty legacy history into fabricated delivery.
            let (batch, issue, at) = match batch {
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
        storage.verify_closed_input_storage()?;
        lease.validate_active()?;
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
            schema_version: 2,
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
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "保存後通知の所有を取得できません")?;
        let entry = Self::entry(
            &mut entries,
            storage,
            lease,
            &request.meeting_id,
            &request.session_id,
        )?;
        let (source, issue) = entry.refresh(&request.session_id, lease)?;
        Self::verify(storage, lease, entry, &request.session_id)?;
        let queue = entry
            .queue
            .as_mut()
            .ok_or("この録音には保存後通知がありません")?;
        queue.poll(source, issue, visible())?;
        let action = match request.action {
            AckAction::Receive => Action::Receive,
            AckAction::Render => Action::Render,
            AckAction::Close => Action::Close,
        };
        let mut receipt = queue.act(
            &request.notice_id,
            &request.recipient_id,
            request.generation,
            action,
            request.receive_id.as_deref(),
            visible(),
        )?;
        let committed_at = Instant::now();
        Self::verify(storage, lease, entry, &request.session_id)?;
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
