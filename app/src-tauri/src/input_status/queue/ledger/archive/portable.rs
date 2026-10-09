//! Immutable historical input evidence, NOT a writable delivery store. Original
//! producer ownership stays intact. This codec grants no destination ownership,
//! current-input freshness, audio durability or old IPC receipt authority.
use super::{bounds, validate, Record};
use crate::input_status::{journal, persistence, queue::Queue};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) mod restored;

// Original ledger proof and inert pending-delivery state are separate. Their
// individual bounds remain 128KiB each, plus the independent 65KiB journal.
const LIMIT: usize = 512 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "record",
    rename_all = "camelCase"
)]
enum SourceLedger {
    Live(super::super::Record),
    Archived(Record),
}
impl SourceLedger {
    fn decode(bytes: &[u8], owner: &str) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() as u64 > super::super::LIMIT {
            return Err("持ち運ぶ通知台帳の容量が不正です".into());
        }
        // Value selects the format only; validate original typed bytes next.
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        match value["schemaVersion"].as_u64() {
            Some(1) => super::super::decode(bytes, owner).map(Self::Live),
            Some(2) => super::decode(bytes, owner).map(Self::Archived),
            _ => Err("持ち運ぶ通知台帳の形式版が不正です".into()),
        }
    }
    fn validate(&self, owner: &str) -> Result<(), String> {
        match self {
            Self::Live(r) => {
                super::super::decode(&serde_json::to_vec(r).map_err(|e| e.to_string())?, owner)?;
            }
            Self::Archived(r) => {
                super::decode(&serde_json::to_vec(r).map_err(|e| e.to_string())?, owner)?;
            }
        }
        Ok(())
    }
}

fn normalize(
    owner: &str,
    source: &Option<journal::Record>,
    ledger: &Option<SourceLedger>,
) -> Result<Option<Record>, String> {
    if source.is_none() && ledger.is_none() {
        return Ok(None);
    }
    let mut record = match ledger {
        Some(SourceLedger::Live(live)) => Record {
            schema_version: 2,
            owner_id: owner.into(),
            revision: 1,
            live_evidence: Some(live.publication.clone()),
            retained_evidence: None,
            history_issue: None,
            queue: live.queue.clone(),
        },
        Some(SourceLedger::Archived(record)) => record.clone(),
        None => Record {
            schema_version: 2,
            owner_id: owner.into(),
            revision: 1,
            live_evidence: None,
            retained_evidence: None,
            history_issue: None,
            queue: Queue::default(),
        },
    };
    if let (Some(old), Some(new)) = (&record.retained_evidence, source) {
        if (old.sealed && old != new)
            || (old.report.revision == new.report.revision && old != new)
            || super::super::Ledger::advances(&new.report, &old.report).is_err()
        {
            return Err("持ち運ぶ保存履歴が通知台帳の証拠に対して逆行しています".into());
        }
    }
    if let Some(source) = source {
        record.retained_evidence = Some(source.clone());
        record.history_issue = None;
    } else if record.history_issue.as_deref() != Some("invalid") {
        record.history_issue = Some("missing".into());
    }
    // Compare overlapping transitions, not unrelated publication revisions.
    let context = bounds(&record)?;
    super::super::fence_queue(&mut record.queue)?;
    record.queue.observe_histories(
        owner,
        [
            context.microphone_history.as_ref(),
            context.system_history.as_ref(),
        ],
        false,
        None,
        Instant::now(),
    );
    validate(&record)?;
    Ok(Some(record))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    source_owner_id: String,
    // This is the independently retained producer journal. Never rewrite its
    // owner to the restored session ID or merge revision domains.
    source_journal: Option<journal::Record>,
    // Immutable original receipt/render proof. This is NEVER a mutable seed or
    // acceptance API. Only `delivery` has old claims removed.
    source_delivery: Option<SourceLedger>,
    delivery: Option<Record>,
    final_diagnostic: Option<persistence::HistoricalFinal>,
}

impl Snapshot {
    pub(crate) fn source_owner_id(&self) -> &str {
        &self.source_owner_id
    }

    /// Only the independently saved producer journal is returned. A delivery
    /// publication must not be relabelled as that journal or as current input.
    pub(crate) fn retained_history(&self) -> Result<Option<journal::Retained>, String> {
        self.validate(&self.source_owner_id)?;
        Ok(self.source_journal.as_ref().map(journal::Record::retained))
    }

    pub(crate) fn history_save_issue(&self) -> Result<Option<&'static str>, String> {
        self.validate(&self.source_owner_id)?;
        match &self.final_diagnostic {
            Some(final_report) => final_report.archived_issue(self.source_journal.as_ref()),
            None => Ok(self
                .source_journal
                .as_ref()
                .filter(|r| !r.sealed)
                .map(|_| "unconfirmed")),
        }
    }

    /// Diagnostic-only view of immutable retention. There is no writable
    /// destination queue yet: NEVER offer a notice or a transferred receipt.
    /// The caller must report pending delivery as unavailable until that queue
    /// has been durably bound to the logical destination owner.
    pub(crate) fn retained_diagnostics(
        &self,
        logical_owner: &str,
    ) -> Result<(Option<super::Batch>, bool), String> {
        self.validate(&self.source_owner_id)?;
        if !crate::input_levels::valid_owner(logical_owner) {
            return Err("復元先の録音入力診断の所属が不正です".into());
        }
        Ok(match &self.delivery {
            Some(record) => (
                Some(super::Batch {
                    schema_version: 1,
                    owner_id: logical_owner.into(),
                    revision: record.revision,
                    notices: vec![],
                    notice_loss: record.queue.loss(),
                    history_issue: record.history_issue.clone(),
                }),
                record
                    .queue
                    .lanes
                    .iter()
                    .any(|lane| !lane.pending.is_empty()),
            ),
            None => (None, false),
        })
    }
    /// Inputs must be bounded bytes read under the caller's DB/source ownership
    /// and data-maintenance fence. No file open or mutable writer is performed.
    /// Reject malformed original bytes before any Value conversion erases keys.
    pub(crate) fn from_evidence(
        owner: &str,
        journal_bytes: Option<&[u8]>,
        ledger_bytes: Option<&[u8]>,
        manifest_bytes: Option<&[u8]>,
    ) -> Result<Option<Self>, String> {
        if !crate::input_levels::valid_owner(owner) {
            return Err("持ち運ぶ入力履歴の元録音IDが不正です".into());
        }
        let source = journal_bytes
            .map(|b| journal::parse(b, owner))
            .transpose()?;
        let diagnostic = manifest_bytes
            .map(|b| persistence::HistoricalFinal::from_manifest(b, owner))
            .transpose()?
            .flatten();
        if source.is_none() && ledger_bytes.is_none() && diagnostic.is_none() {
            return Ok(None); // Legacy unknown, not a fabricated successful save.
        }
        let source_delivery = ledger_bytes
            .map(|b| SourceLedger::decode(b, owner))
            .transpose()?;
        let delivery = normalize(owner, &source, &source_delivery)?;
        let snapshot = Self {
            schema_version: 1,
            source_owner_id: owner.into(),
            source_journal: source,
            source_delivery,
            delivery,
            final_diagnostic: diagnostic,
        };
        snapshot.validate(owner)?;
        snapshot.encode()?;
        Ok(Some(snapshot))
    }

    fn validate(&self, owner: &str) -> Result<(), String> {
        if self.schema_version != 1
            || self.source_owner_id != owner
            || !crate::input_levels::valid_owner(owner)
            || (self.source_journal.is_none()
                && self.delivery.is_none()
                && self.final_diagnostic.is_none())
        {
            return Err("持ち運ぶ入力記録の版・所有または内容が不正です".into());
        }
        if let Some(source) = &self.source_journal {
            journal::parse(
                &serde_json::to_vec(source).map_err(|e| e.to_string())?,
                owner,
            )?;
        }
        if let Some(diagnostic) = &self.final_diagnostic {
            diagnostic.validate(owner)?;
        }
        if let Some(source) = &self.source_delivery {
            source.validate(owner)?;
        }
        let expected = normalize(owner, &self.source_journal, &self.source_delivery)?;
        if serde_json::to_value(&self.delivery).map_err(|e| e.to_string())?
            != serde_json::to_value(expected).map_err(|e| e.to_string())?
        {
            return Err("持ち運ぶ通知保持範囲が元の証拠と一致しません".into());
        }
        match &self.delivery {
            Some(record) => {
                validate(record)?;
                if serde_json::to_vec(record).map_err(|e| e.to_string())?.len() as u64
                    > super::super::LIMIT
                {
                    return Err("持ち運ぶ通知保持範囲が容量上限を超えました".into());
                }
                for lane in &record.queue.lanes {
                    if lane.closed.is_some()
                        || lane.pending.iter().any(|p| {
                            p.target.is_some()
                                || p.recipient.is_some()
                                || p.receive_id.is_some()
                                || p.claimed.is_some()
                                || p.rendered
                        })
                    {
                        return Err("持ち運ぶ通知記録に旧画面の受付権限が残っています".into());
                    }
                }
            }
            None if self.source_journal.is_some() => {
                return Err("持ち運ぶ保存履歴に通知保持範囲がありません".into());
            }
            None => {}
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate(&self.source_owner_id)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("持ち運ぶ入力記録が容量上限を超えました".into());
        }
        Ok(bytes)
    }

    /// The caller binds expected_owner to the original DB-owned recording or
    /// verified immutable archive provenance, not to arbitrary JSON alone.
    pub(crate) fn decode(bytes: &[u8], expected_owner: &str) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > LIMIT {
            return Err("持ち運ぶ入力記録の容量が不正です".into());
        }
        let record: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if serde_json::to_value(&record).map_err(|e| e.to_string())?
            != serde_json::from_slice::<serde_json::Value>(bytes).map_err(|e| e.to_string())?
        {
            return Err("持ち運ぶ入力記録の必須項目が不正です".into());
        }
        record.validate(expected_owner)?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests;
