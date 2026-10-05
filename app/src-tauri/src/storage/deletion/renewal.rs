//! Append-only evidence for an explicitly reviewed remaining file scope.
//! These internal observations are never reconstructed from client JSON.
use super::{
    files::{Census, Manifest},
    intent::{self, Observations, Prepared},
    removal::{self, Progress},
};
use crate::{
    analysis_worker::data_maintenance::DataMaintenanceLease,
    storage::{now_ms, Storage},
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_GENERATIONS: u32 = 32;
const AUDIT_PROGRESS: &str = "{\"confirmationRecorded\":true}";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("残りの削除範囲の再確認を停止しました。元の対象と進捗を保持します".into())
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Reference {
    pub(super) operation_id: String,
    pub(super) intent_hash: String,
    pub(super) generation: u32,
}
impl Reference {
    pub(super) fn valid(&self) -> bool {
        self.generation > 0
            && self.generation <= MAX_GENERATIONS
            && uuid::Uuid::parse_str(&self.operation_id)
                .is_ok_and(|id| id.to_string() == self.operation_id)
            && self.intent_hash.len() == 64
            && self
                .intent_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    version: u32,
    source_operation: String,
    source_intent_hash: String,
    previous_progress: Progress,
    observations: intent::Intent,
}
impl Record {
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 65536 {
            Err("再確認記録が安全上限を超えています".into())
        } else {
            Ok(json)
        }
    }
}

// Loading one generation at a time bounds retained inventories. The immutable
// audit links prove every earlier cursor/policy; no cursor is silently reset.
pub(super) fn active_manifest(
    db: &Connection,
    original: &Prepared,
    progress: &Progress,
    check: &impl Fn() -> bool,
) -> Result<Manifest, String> {
    let mut chain = Vec::new();
    let mut current = progress.confirmation.clone();
    let source_hash = original.intent_hash()?;
    while let Some(reference) = current {
        cancel(check)?;
        if !reference.valid()
            || reference.operation_id == original.operation()
            || chain.len() >= MAX_GENERATIONS as usize
        {
            return Err("削除再確認の世代・識別子が不正です".into());
        }
        let (meeting, kind, state, json, sha, consent, saved, created, updated): (Option<String>,String,String,String,String,Option<i64>,String,i64,i64) = db.query_row(
            "SELECT meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1",
            params![reference.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|e| e.to_string())?;
        if meeting.is_some()
            || kind != "delete_meeting"
            || state != "completed"
            || json.len() > 65536
            || sha != reference.intent_hash
            || hash(json.as_bytes()) != sha
            || saved != AUDIT_PROGRESS
            || created < original.consent_at()
            || updated != created
            || consent != Some(created)
        {
            return Err("残りの削除範囲の固定確認記録を検証できません".into());
        }
        let record: Record = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if record.version != 1
            || record.source_operation != original.operation()
            || record.source_intent_hash != source_hash
            || record.encode()? != json
            || reference.generation
                != record
                    .previous_progress
                    .confirmation
                    .as_ref()
                    .map_or(1, |p| p.generation.saturating_add(1))
            || created < record.previous_progress.consent_at()
        {
            return Err("再確認と元の削除記録・同意・進捗の対応が不正です".into());
        }
        current = record.previous_progress.confirmation.clone();
        chain.push((reference, record, created));
    }
    let mut manifest = original.manifest().clone();
    let mut previous: Option<Reference> = None;
    let mut latest_consent = None;
    for (reference, record, consent) in chain.into_iter().rev() {
        if record.previous_progress.confirmation != previous
            || latest_consent.is_some_and(|c| c != record.previous_progress.consent_at())
        {
            return Err("再確認履歴の接続が一致しません".into());
        }
        record.previous_progress.validate(original, &manifest)?;
        let observations =
            Observations::load(db, &reference.operation_id, record.observations, check)?;
        if observations.scope() != original.scope()
            || observations.manifest.root != original.manifest().root
            || observations.manifest.database != original.manifest().database
            || observations.manifest.meeting_id != original.manifest().meeting_id
        {
            return Err("元のDB範囲・起動時実体を再確認で変更できません".into());
        }
        manifest = observations.manifest;
        previous = Some(reference);
        latest_consent = Some(consent);
    }
    if previous != progress.confirmation
        || latest_consent.is_some_and(|c| c != progress.consent_at())
    {
        return Err("現在の削除計画が固定した再同意と一致しません".into());
    }
    progress.validate(original, &manifest)?;
    Ok(manifest)
}

pub(super) struct Review {
    operation: String,
    source_hash: String,
    previous_progress: String,
    census: Census,
}
impl Review {
    pub(super) fn capture(
        storage: &Storage,
        receipt: &removal::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        if receipt
            .progress
            .confirmation
            .as_ref()
            .is_some_and(|r| r.generation >= MAX_GENERATIONS)
        {
            return Err("残りの削除範囲の再確認が世代上限に達しました。対象を保持します".into());
        }
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        receipt.verify_saved(&db, check)?;
        let census = receipt.review_census(storage, &db, check)?;
        Ok(Self {
            operation: receipt.confirmed().operation().into(),
            source_hash: receipt.confirmed().intent_hash()?,
            previous_progress: receipt.progress_json()?,
            census,
        })
    }
    pub(super) fn manifest(&self) -> &Manifest {
        self.census.manifest()
    }
    fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        receipt: &removal::Receipt,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        receipt.verify_saved(db, check)?;
        if self.operation != receipt.confirmed().operation()
            || self.source_hash != receipt.confirmed().intent_hash()?
            || self.previous_progress != receipt.progress_json()?
        {
            return Err("再確認中に元の進捗が変わりました。新しい一覧を確認してください".into());
        }
        let current = receipt.review_census(storage, db, check)?;
        if current.scope() != self.census.scope() || current.manifest() != self.census.manifest() {
            return Err("再同意後に残りの対象が変わりました。新しい一覧を確認してください".into());
        }
        Ok(())
    }
}

pub(super) fn accept(
    storage: &Storage,
    receipt: &mut removal::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    accept_inner(
        storage,
        receipt,
        review,
        include_unregistered,
        admission,
        check,
        &|_| {},
    )
}
fn accept_inner(
    storage: &Storage,
    receipt: &mut removal::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    admission.validate_active()?;
    let check = || check() || admission.cancelled();
    cancel(&check)?;
    // Reuse policy validation; unknown files are never approved by renewal alone.
    removal::Reviewed::confirm(&review.census, include_unregistered)?;
    let original = receipt.confirmed();
    let observations = Observations::capture(&review.census)?;
    let record = Record {
        version: 1,
        source_operation: original.operation().into(),
        source_intent_hash: original.intent_hash()?,
        previous_progress: receipt.progress.clone(),
        observations: observations.header.clone(),
    };
    let json = record.encode()?;
    let reference = Reference {
        operation_id: uuid::Uuid::new_v4().to_string(),
        intent_hash: hash(json.as_bytes()),
        generation: receipt
            .progress
            .confirmation
            .as_ref()
            .map_or(1, |r| r.generation + 1),
    };
    let consent = now_ms()?.max(receipt.progress.consent_at());
    let next = Progress::renewed(
        original,
        review.manifest(),
        reference.clone(),
        include_unregistered,
        consent,
    )?;
    receipt.verify_reservation_names(review.manifest(), &next)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    review.verify(storage, &db, receipt, &check)?;
    let _writer = super::fence::write_removal(storage, &db, receipt, &check)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    review.verify(storage, &tx, receipt, &check)?;
    tx.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,'delete_meeting','prepared',?2,?3,?4,?5,?4,?4)",params![reference.operation_id,json,reference.intent_hash,consent,AUDIT_PROGRESS]).map_err(|e| e.to_string())?;
    event("reviewIntentStored");
    observations.store_paths(&tx, &reference.operation_id, &check, event)?;
    tx.execute(
        "UPDATE maintenance_operations SET state='completed' WHERE id=?1 AND state='prepared'",
        params![reference.operation_id],
    )
    .map_err(|e| e.to_string())?;
    event("reviewRecorded");
    // The audit has no meeting FK; it does not change the original DB scope.
    review.verify(storage, &tx, receipt, &check)?;
    if tx.execute("UPDATE maintenance_operations SET progress_json=?2,error_code=NULL,updated_at=max(updated_at,?3) WHERE id=?1 AND kind='delete_meeting' AND state='installing' AND progress_json=?4",params![original.operation(),next.encode()?,consent,receipt.progress_json()?]).map_err(|e| e.to_string())? != 1 {
        return Err("再同意の切替対象が変更されています".into());
    }
    if active_manifest(&tx, original, &next, &check)? != *review.manifest() {
        return Err("再同意の保存結果が一致しません".into());
    }
    storage.managed_anchor.verify()?;
    event("beforeReviewCommit");
    cancel(&check)?;
    // Recheck the actual inventory once more after publication, without using
    // the stale cursor that was just archived in the immutable audit record.
    let live = receipt.review_census(storage, &tx, &check)?;
    if live.manifest() != review.manifest() || live.scope() != original.scope() {
        return Err("再確認の確定前に対象が変わりました".into());
    }
    cancel(&check)?;
    tx.commit().map_err(|e| e.to_string())?;
    receipt.progress = next;
    receipt.manifest = review.manifest().clone();
    event("reviewCommitted");
    Ok(())
}
#[cfg(test)]
pub(super) fn accept_observed(
    storage: &Storage,
    receipt: &mut removal::Receipt,
    review: &Review,
    include_unregistered: bool,
    admission: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    accept_inner(
        storage,
        receipt,
        review,
        include_unregistered,
        admission,
        check,
        event,
    )
}
