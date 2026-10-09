//! Historical delivery, not a live capture projection. Preserve the two source
//! channels independently: their revision numbers have different clocks.
use super::{fence_queue, file, Record as LiveRecord, LIMIT};
use crate::input_status::{
    journal,
    queue::{Action, Loss, Queue},
    Notice, Publication, Receipt, SAFE,
};
use serde::{Deserialize, Serialize};
use std::{fs::File, time::Instant};

// The portable codec does not acquire a second mutable writer. Storage binds
// its immutable result to logical backup/restore ownership separately.
pub(crate) mod portable;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    live_evidence: Option<Publication>,
    retained_evidence: Option<journal::Record>,
    history_issue: Option<String>,
    queue: Queue,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Batch {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    notices: Vec<Notice>,
    notice_loss: Loss,
    history_issue: Option<String>,
}
impl Batch {
    pub(in crate::input_status) fn hide_notices(&mut self) {
        self.notices.clear();
    }
    pub(in crate::input_status) fn account_wait(&mut self, elapsed: std::time::Duration) {
        for notice in &mut self.notices {
            if let Some(delivery) = &mut notice.delivery {
                delivery.account_persistence_time(elapsed);
            }
        }
    }
}

// A validation/ingestion context only. Do not persist or expose this combination
// as an atomic producer publication or use it to infer current connection.
fn bounds(record: &Record) -> Result<Publication, String> {
    let live = record.live_evidence.as_ref();
    let retained = record.retained_evidence.as_ref().map(|r| &r.report);
    let mut result = retained
        .or(live)
        .ok_or("通知履歴の取得証拠がありません")?
        .clone();
    if let Some(p) = live {
        crate::input_status::parse(
            &serde_json::to_vec(p).map_err(|e| e.to_string())?,
            &record.owner_id,
        )?;
    }
    if let Some(p) = &record.retained_evidence {
        journal::parse(
            &serde_json::to_vec(p).map_err(|e| e.to_string())?,
            &record.owner_id,
        )?;
    }
    if let (Some(a), Some(b)) = (live, retained) {
        if a.system.is_some() != b.system.is_some() {
            return Err("通知履歴の入力構成が一致しません".into());
        }
        for index in 0..2 {
            let (ah, bh, ac, bc) = if index == 0 {
                (
                    a.microphone_history.as_ref(),
                    b.microphone_history.as_ref(),
                    Some(&a.microphone),
                    Some(&b.microphone),
                )
            } else {
                (
                    a.system_history.as_ref(),
                    b.system_history.as_ref(),
                    a.system.as_ref(),
                    b.system.as_ref(),
                )
            };
            let (Some(ah), Some(bh), Some(ac), Some(bc)) = (ah, bh, ac, bc) else {
                continue;
            };
            let choose_live = ah.last_sequence > bh.last_sequence
                || (ah.last_sequence == bh.last_sequence && ac.source_epoch > bc.source_epoch);
            let (latest, older) = if choose_live { (ah, bh) } else { (bh, ah) };
            if !latest.advances(older) {
                return Err("通知履歴の重なる遷移が変更されています".into());
            }
            if choose_live {
                if index == 0 {
                    result.microphone = ac.clone();
                    result.microphone_history = Some(ah.clone());
                } else {
                    result.system = Some(ac.clone());
                    result.system_history = Some(ah.clone());
                }
            }
        }
    }
    Ok(result)
}

fn validate(record: &Record) -> Result<(), String> {
    if record.schema_version != 2
        || !(1..=SAFE).contains(&record.revision)
        || !crate::input_levels::valid_owner(&record.owner_id)
        || record
            .history_issue
            .as_deref()
            .is_some_and(|v| !matches!(v, "missing" | "invalid"))
    {
        return Err("保存後通知の版・所有または履歴診断が不正です".into());
    }
    let context = bounds(record)?;
    if context.owner_id != record.owner_id {
        return Err("保存後通知の録音IDが一致しません".into());
    }
    super::validate(&LiveRecord {
        schema_version: 1,
        owner_id: record.owner_id.clone(),
        publication: context,
        queue: record.queue.clone(),
    })
}
fn decode(bytes: &[u8], owner: &str) -> Result<Record, String> {
    if bytes.is_empty() || bytes.len() as u64 > LIMIT {
        return Err("保存後通知の容量が不正です".into());
    }
    let record: Record = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if record.owner_id != owner
        || serde_json::to_value(&record).map_err(|e| e.to_string())?
            != serde_json::from_slice::<serde_json::Value>(bytes).map_err(|e| e.to_string())?
    {
        return Err("保存後通知の所有または必須項目が不正です".into());
    }
    validate(&record)?;
    Ok(record)
}

pub(in crate::input_status) struct ArchiveQueue {
    store: file::Store,
    record: Record,
    committed: bool,
}
impl ArchiveQueue {
    pub(in crate::input_status) fn open(
        directory: &File,
        owner: &str,
        source: Option<journal::Record>,
        issue: Option<&str>,
    ) -> Result<Option<Self>, String> {
        if !crate::input_levels::valid_owner(owner) {
            return Err("保存後通知の録音IDが不正です".into());
        }
        if source.is_none()
            && crate::input_levels::open_capture_file_at_optional(directory, "input-notices.json")?
                .is_none()
        {
            if issue == Some("invalid") {
                return Err("保存した取得履歴を確認できません".into());
            }
            return Ok(None);
        }
        let (store, bytes) = file::Store::open(directory)?;
        let (record, committed) = if let Some(bytes) = bytes {
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            match value["schemaVersion"].as_u64() {
                Some(1) => {
                    let live = super::decode(&bytes, owner)?;
                    (
                        Record {
                            schema_version: 2,
                            owner_id: owner.into(),
                            revision: 0,
                            live_evidence: Some(live.publication),
                            retained_evidence: None,
                            history_issue: None,
                            queue: live.queue,
                        },
                        false,
                    )
                }
                Some(2) => (decode(&bytes, owner)?, true),
                _ => return Err("保存後通知の形式版が不正です".into()),
            }
        } else {
            (
                Record {
                    schema_version: 2,
                    owner_id: owner.into(),
                    revision: 0,
                    live_evidence: None,
                    retained_evidence: None,
                    history_issue: None,
                    queue: Default::default(),
                },
                false,
            )
        };
        let mut result = Self {
            store,
            record,
            committed,
        };
        let mut candidate = result.record.clone();
        fence_queue(&mut candidate.queue)?;
        Self::source(&mut candidate, source, issue)?;
        let context = bounds(&candidate)?;
        candidate.queue.observe_histories(
            owner,
            [
                context.microphone_history.as_ref(),
                context.system_history.as_ref(),
            ],
            false,
            None,
            Instant::now(),
        );
        result.save(candidate)?;
        Ok(Some(result))
    }
    fn source(
        candidate: &mut Record,
        source: Option<journal::Record>,
        issue: Option<&str>,
    ) -> Result<(), String> {
        candidate.history_issue = issue.map(str::to_owned);
        if let Some(source) = source {
            journal::parse(
                &serde_json::to_vec(&source).map_err(|e| e.to_string())?,
                &candidate.owner_id,
            )?;
            let advances = candidate.retained_evidence.as_ref().is_none_or(|old| {
                (!old.sealed || source == *old)
                    && (source.report.revision != old.report.revision || source == *old)
                    && super::Ledger::advances(&source.report, &old.report).is_ok()
            });
            let mut checked = candidate.clone();
            checked.retained_evidence = Some(source);
            if advances && bounds(&checked).is_ok() {
                candidate.retained_evidence = checked.retained_evidence;
            } else {
                candidate.history_issue = Some("invalid".into());
            }
        }
        Ok(())
    }
    fn save(&mut self, mut candidate: Record) -> Result<(), String> {
        if self.committed
            && serde_json::to_value(&candidate).map_err(|e| e.to_string())?
                == serde_json::to_value(&self.record).map_err(|e| e.to_string())?
        {
            self.store.verify()?;
            // Keep only process-local lease renewal; disk never manufactures it.
            self.record = candidate;
            return Ok(());
        }
        candidate.revision = self
            .record
            .revision
            .checked_add(1)
            .filter(|v| *v <= SAFE)
            .ok_or("保存後通知の版が上限に達しました")?;
        validate(&candidate)?;
        let bytes = serde_json::to_vec(&candidate).map_err(|e| e.to_string())?;
        self.store.commit(&bytes)?;
        self.record = candidate;
        self.committed = true;
        Ok(())
    }
    pub(in crate::input_status) fn poll(
        &mut self,
        source: Option<journal::Record>,
        issue: Option<&str>,
        visible: bool,
    ) -> Result<Batch, String> {
        let mut candidate = self.record.clone();
        Self::source(&mut candidate, source, issue)?;
        let context = bounds(&candidate)?;
        let started = Instant::now();
        let mut notices = candidate.queue.observe_histories(
            &candidate.owner_id,
            [
                context.microphone_history.as_ref(),
                context.system_history.as_ref(),
            ],
            visible,
            visible.then_some("main"),
            started,
        );
        self.save(candidate)?;
        for notice in &mut notices {
            if let Some(delivery) = &mut notice.delivery {
                delivery.account_persistence_time(started.elapsed());
            }
        }
        Ok(Batch {
            schema_version: 1,
            owner_id: self.record.owner_id.clone(),
            revision: self.record.revision,
            notices,
            notice_loss: self.record.queue.loss(),
            history_issue: self.record.history_issue.clone(),
        })
    }
    pub(in crate::input_status) fn act(
        &mut self,
        id: &str,
        recipient: &str,
        generation: u64,
        action: Action,
        receive_id: Option<&str>,
        visible: bool,
    ) -> Result<Receipt, String> {
        if !visible
            || !crate::input_levels::valid_owner(recipient)
            || id.len() > 128
            || !(1..=SAFE).contains(&generation)
        {
            return Err("保存後通知の画面または受付引数が不正です".into());
        }
        let mut candidate = self.record.clone();
        let started = Instant::now();
        let mut receipt = candidate.queue.act(
            &candidate.owner_id,
            id,
            recipient,
            "main",
            generation,
            action,
            started,
            receive_id,
        )?;
        self.save(candidate)?;
        if let Some(delivery) = &mut receipt.delivery {
            delivery.account_persistence_time(started.elapsed());
        }
        Ok(receipt)
    }
}

#[cfg(all(test, unix))]
mod tests;
