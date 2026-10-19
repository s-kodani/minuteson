//! Append-only reviews of an unfinished audio plan. A serialized reference is
//! not consent: the complete canonical chain and a live, server-held Review
//! must be checked before granting the fenced transaction writer.
use super::super as intent;
use super::super::super as audio;
use super::{active, Phase, Prepared, Progress, Receipt};
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_GENERATIONS: u32 = 32;
const MAX_PATHS: usize = 200_000;
pub(in crate::storage::deletion) const AUDIT_PROGRESS: &str =
    r#"{"audioRemainingConfirmationRecorded":true}"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Reference {
    operation_id: String,
    intent_hash: String,
    pub(super) generation: u32,
}
impl Reference {
    pub(super) fn valid(&self) -> bool {
        audio::uuid(&self.operation_id)
            && intent::hash_valid(&self.intent_hash)
            && (1..=MAX_GENERATIONS).contains(&self.generation)
    }
}
pub(super) fn reserved(operation: &str, generation: u32, index: usize) -> String {
    if generation == 0 {
        format!(".mon-delete-audio-{operation}-{index}")
    } else {
        format!(".mon-delete-audio-{operation}-r{generation}-{index}")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    version: u32,
    subtype: String,
    source_operation: String,
    source_hash: String,
    previous_progress: Progress,
    snapshot_hash: String,
    snapshot_bytes: usize,
    chunk_count: usize,
    policy: intent::Policy,
}
impl Record {
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 65536 {
            return Err("残り音声への再同意記録が容量上限を超えています".into());
        }
        Ok(json)
    }
}
fn source_hash(saved: &Prepared) -> Result<String, String> {
    Ok(intent::hash(
        &serde_json::to_vec(&(&saved.header, saved.consent)).map_err(|e| e.to_string())?,
    ))
}
pub(super) struct Plan {
    pub(super) snapshot: audio::Snapshot,
    pub(super) policy: intent::Policy,
    pub(super) paths: BTreeSet<String>,
    pub(super) audits: Vec<String>,
    pub(super) bytes: usize,
}
fn add_paths(
    paths: &mut BTreeSet<String>,
    saved: &Prepared,
    snapshot: &audio::Snapshot,
    generation: u32,
) -> Result<(), String> {
    for (index, target) in super::targets(snapshot)?.iter().enumerate() {
        let parent = target
            .path
            .rsplit_once('/')
            .ok_or("音声対象の親がありません")?
            .0;
        paths.insert(target.path.clone());
        paths.insert(format!(
            "{parent}/{}",
            reserved(saved.operation(), generation, index)
        ));
        if paths.len() > MAX_PATHS {
            return Err("残り音声の履歴対象数が安全上限を超えています".into());
        }
    }
    Ok(())
}
fn historical(snapshot: &audio::Snapshot, paths: &BTreeSet<String>) -> BTreeSet<String> {
    snapshot
        .manifest
        .entries
        .iter()
        .filter(|e| {
            !e.directory && e.registered.is_none() && e.proof.is_some() && paths.contains(&e.path)
        })
        .map(|e| e.path.clone())
        .collect()
}
fn policy_valid(snapshot: &audio::Snapshot, policy: &intent::Policy) -> Result<(), String> {
    if !policy.retain_unclassified
        || (!policy.include_unregistered_audio
            && snapshot
                .entries()?
                .iter()
                .any(|e| audio::audio(&e.disposition) && e.asset_id.is_none()))
    {
        return Err("未登録音声への明示同意と不明ファイルの保持を確認してください".into());
    }
    Ok(())
}
fn read_record(
    db: &Connection,
    reference: &Reference,
    saved: &Prepared,
    budget: &mut usize,
    check: &impl Fn() -> bool,
) -> Result<(Record, audio::Snapshot, i64), String> {
    audio::cancel(check)?;
    let (meeting,batch,kind,state,json,sha,consent,progress,error,created,updated):
        (Option<String>,Option<String>,String,String,Option<String>,String,Option<i64>,Option<String>,Option<String>,i64,i64) =
        db.query_row("SELECT meeting_id,restore_batch_id,kind,state,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END,target_hash,consent_at,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json END,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1", params![reference.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).map_err(|e| e.to_string())?;
    let json = json.ok_or("残り音声の確認記録が容量上限を超えています")?;
    if !reference.valid()
        || reference.operation_id == saved.operation()
        || meeting.is_some()
        || batch.is_some()
        || kind != "delete_assets"
        || state != "completed"
        || progress.as_deref() != Some(AUDIT_PROGRESS)
        || error.is_some()
        || sha != reference.intent_hash
        || intent::hash(json.as_bytes()) != sha
        || created < saved.consent
        || updated != created
        || consent != Some(created)
    {
        return Err("残り音声への再同意の固定記録が不正です".into());
    }
    let record: Record = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if record.version != 3
        || record.subtype != "audioRemainingConfirmation"
        || record.source_operation != saved.operation()
        || record.source_hash != source_hash(saved)?
        || record.encode()? != json
        || record.previous_progress.phase == Phase::AudioDeletionCompleted
        || reference.generation
            != record
                .previous_progress
                .confirmation
                .as_ref()
                .map_or(1, |r| r.generation.saturating_add(1))
    {
        return Err("残り音声の確認と元の同意・世代が一致しません".into());
    }
    // Charge the whole chain before allocating each snapshot. A valid per-row
    // limit alone is not enough when multiple canonical plans are linked.
    *budget = budget
        .checked_add(json.len())
        .and_then(|n| n.checked_add(record.snapshot_bytes))
        .ok_or("残り音声の確認証拠容量が範囲外です")?;
    if *budget > intent::MAX_BYTES {
        return Err("残り音声の確認証拠合計が容量上限を超えています".into());
    }
    let snapshot = intent::read_snapshot(
        db,
        &reference.operation_id,
        record.snapshot_bytes,
        &record.snapshot_hash,
        record.chunk_count,
        check,
    )?;
    policy_valid(&snapshot, &record.policy)?;
    Ok((record, snapshot, created))
}
pub(super) fn load(
    db: &Connection,
    saved: &Prepared,
    progress: &Progress,
    ancestors: Option<&BTreeSet<String>>,
    check: &impl Fn() -> bool,
) -> Result<Plan, String> {
    let mut plan = Plan {
        snapshot: saved.snapshot.clone(),
        policy: saved.header.policy.clone(),
        paths: BTreeSet::new(),
        audits: Vec::new(),
        bytes: saved.header.snapshot_bytes,
    };
    add_paths(&mut plan.paths, saved, &plan.snapshot, 0)?;
    let mut reference = progress.confirmation.clone();
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut budget = saved.header.snapshot_bytes;
    while let Some(current) = reference {
        audio::cancel(check)?;
        if chain.len() >= MAX_GENERATIONS as usize
            || !current.valid()
            || !seen.insert(current.operation_id.clone())
        {
            return Err("残り音声の再同意世代・循環が不正です".into());
        }
        let (record, snapshot, consent) = read_record(db, &current, saved, &mut budget, check)?;
        reference = record.previous_progress.confirmation.clone();
        chain.push((current, record, snapshot, consent));
    }
    let mut previous = None;
    let mut latest = saved.consent;
    for (reference, record, next, consent) in chain.into_iter().rev() {
        audio::cancel(check)?;
        record
            .previous_progress
            .validate(saved, &super::targets(&plan.snapshot)?)?;
        // Classify typed ancestor paths on the oldest-to-newest validation pass.
        let mut paths = plan.paths.clone();
        if let Some(ancestors) = ancestors {
            paths.extend(ancestors.iter().cloned());
            if paths.len() > MAX_PATHS {
                return Err("残り音声と祖先の履歴対象数が安全上限を超えています".into());
            }
        }
        let own = historical(&next, &plan.paths);
        let history_valid = if ancestors.is_some() || saved.header.origin.is_none() {
            next.historical_audio == historical(&next, &paths)
        } else {
            own.is_subset(&next.historical_audio)
        };
        if record.previous_progress.confirmation != previous
            || consent < latest
            || next.scope != saved.snapshot.scope
            || next.records != saved.snapshot.records
            || next.manifest.root != saved.snapshot.manifest.root
            || next.manifest.database != saved.snapshot.manifest.database
            || !history_valid
        {
            return Err("残り音声の固定範囲・保持履歴・同意順序が一致しません".into());
        }
        add_paths(&mut plan.paths, saved, &next, reference.generation)?;
        plan.snapshot = next;
        plan.policy = record.policy;
        plan.audits.push(reference.operation_id.clone());
        previous = Some(reference);
        latest = consent;
    }
    if previous != progress.confirmation {
        return Err("残り音声への再同意と現在の進捗が接続していません".into());
    }
    progress.validate(saved, &super::targets(&plan.snapshot)?)?;
    plan.bytes = budget;
    Ok(plan)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Review {
    receipt: Receipt,
    snapshot: audio::Snapshot,
}
fn capture_snapshot(
    storage: &Storage,
    db: &Connection,
    receipt: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<audio::Snapshot, String> {
    receipt.verify_saved(db, check)?;
    receipt.metadata(storage, db, check)?;
    if receipt.completed()
        || receipt
            .progress
            .confirmation
            .as_ref()
            .is_some_and(|r| r.generation >= MAX_GENERATIONS)
    {
        return Err("残り音声の再同意対象ではないか、世代上限に達しています".into());
    }
    let reserved = receipt
        .progress
        .pending
        .as_ref()
        .map(|_| receipt.reserved_path(receipt.progress.next));
    let source = receipt
        .saved
        .header
        .origin
        .as_ref()
        .map(|_| intent::reconfirmation::Source::for_prepared(db, &receipt.saved, check))
        .transpose()?;
    let census = || {
        audio::MeetingCensus::capture_audio_remaining_locked(
            storage,
            db,
            receipt,
            check,
            reserved.as_deref(),
        )
    };
    let first = census()?;
    let mut snapshot = audio::Snapshot {
        scope: first.scope().clone(),
        manifest: first.manifest().clone(),
        records: audio::records(db, receipt.saved.meeting(), check)?,
        historical_audio: BTreeSet::new(),
    };
    let mut paths = receipt.historical_paths.clone();
    if let Some(source) = &source {
        paths.extend(source.paths());
        if paths.len() > MAX_PATHS {
            return Err("残り音声と祖先の履歴対象数が安全上限を超えています".into());
        }
    }
    snapshot.historical_audio = historical(&snapshot, &paths);
    snapshot.validate(db)?;
    if snapshot.scope != receipt.saved.snapshot.scope
        || snapshot.records != receipt.saved.snapshot.records
    {
        return Err("元の同意で保持する音声・文書・履歴が変更されています".into());
    }
    let second = census()?;
    if first.manifest() != second.manifest()
        || first.scope() != second.scope()
        || audio::records(db, receipt.saved.meeting(), check)? != snapshot.records
    {
        return Err("残り音声の確認中に対象・保持履歴が変更されました".into());
    }
    receipt.metadata(storage, db, check)?;
    Ok(snapshot)
}
impl Review {
    pub(crate) fn capture(
        storage: &Storage,
        operation: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        let cancelled = || check() || lease.cancelled();
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let receipt = Receipt::load(&db, operation, &cancelled)?;
        let snapshot = capture_snapshot(storage, &db, &receipt, &cancelled)?;
        active(lease, check)?;
        Ok(Self { receipt, snapshot })
    }
    pub(in crate::storage::deletion) fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if capture_snapshot(storage, db, &self.receipt, check)? != self.snapshot {
            return Err(
                "残り音声の再同意前に対象が変更されました。保持して再確認してください".into(),
            );
        }
        Ok(())
    }
    pub(in crate::storage::deletion::audio) fn snapshot(&self) -> &audio::Snapshot {
        &self.snapshot
    }
    pub(in crate::storage::deletion::audio) fn verify_live(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.verify(storage, &db, &|| check() || lease.cancelled())?;
        active(lease, check)
    }
    pub(in crate::storage::deletion::audio) fn operation(&self) -> &str {
        self.receipt.saved.operation()
    }
}
pub(crate) fn accept(
    storage: &Storage,
    review: &Review,
    include_unregistered_audio: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    accept_inner(
        storage,
        review,
        include_unregistered_audio,
        lease,
        check,
        &|_| {},
    )
}
fn accept_inner(
    storage: &Storage,
    review: &Review,
    include_unregistered_audio: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    active(lease, check)?;
    let cancelled = || check() || lease.cancelled();
    let old = &review.receipt;
    let policy = intent::Policy {
        include_unregistered_audio,
        retain_unclassified: true,
    };
    policy_valid(&review.snapshot, &policy)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let current = Receipt::load(&db, old.saved.operation(), &cancelled)?;
    if current != *old {
        let reference = current
            .progress
            .confirmation
            .as_ref()
            .ok_or("残り音声への再同意の再送記録がありません")?;
        let mut budget = old.saved.header.snapshot_bytes;
        let (record, snapshot, _) =
            read_record(&db, reference, &old.saved, &mut budget, &cancelled)?;
        if current.saved != old.saved
            || record.previous_progress != old.progress
            || snapshot != review.snapshot
            || record.policy != policy
        {
            return Err("残り音声への再同意の再送が確定した内容と一致しません".into());
        }
        super::verify_remaining(storage, &db, &current, &cancelled)?;
        active(lease, check)?;
        return Ok(current);
    }
    review.verify(storage, &db, &cancelled)?;
    let bytes = review.snapshot.encode()?;
    let generation = old
        .progress
        .confirmation
        .as_ref()
        .map_or(1, |r| r.generation + 1);
    let record = Record {
        version: 3,
        subtype: "audioRemainingConfirmation".into(),
        source_operation: old.saved.operation().into(),
        source_hash: source_hash(&old.saved)?,
        previous_progress: old.progress.clone(),
        snapshot_hash: intent::hash(&bytes),
        snapshot_bytes: bytes.len(),
        chunk_count: bytes.len().div_ceil(intent::CHUNK),
        policy,
    };
    let json = record.encode()?;
    let reference = Reference {
        operation_id: uuid::Uuid::new_v4().to_string(),
        intent_hash: intent::hash(json.as_bytes()),
        generation,
    };
    let next = Progress {
        version: 4,
        phase: Phase::RemovingAudio,
        census_hash: old.saved.header.census_hash.clone(),
        next: 0,
        pending: None,
        final_audio: None,
        confirmation: Some(reference.clone()),
    };
    let targets = super::targets(&review.snapshot)?;
    next.validate(&old.saved, &targets)?;
    if review.snapshot.manifest.entries.len() >= crate::storage::deletion::files::MAX_ENTRIES
        && targets
            .iter()
            .any(|e| e.proof.is_some() && e.registered.is_some())
    {
        return Err("残り音声削除の予約観測容量を確保できません".into());
    }
    for (index, target) in targets.iter().enumerate() {
        let path = format!(
            "{}/{}",
            target
                .path
                .rsplit_once('/')
                .ok_or("音声対象の親がありません")?
                .0,
            reserved(old.saved.operation(), generation, index)
        );
        if review
            .snapshot
            .manifest
            .entries
            .iter()
            .any(|e| e.path == path)
        {
            return Err("残り音声削除の新しい予約名が既に存在します".into());
        }
    }
    let _writer = crate::storage::deletion::fence::write_audio_renewal(
        storage, &db, review, lease, &cancelled,
    )?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    review.verify(storage, &tx, &cancelled)?;
    let latest: i64 = tx
        .query_row(
            "SELECT updated_at FROM maintenance_operations WHERE id=?1",
            params![old.saved.operation()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let consent = crate::storage::now_ms()?;
    if consent < latest || consent < old.saved.consent {
        return Err("音声再同意の時刻が保存した進捗より前です。対象を保持します".into());
    }
    tx.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','prepared',?2,?3,?4,?5,?4,?4)",params![reference.operation_id,json,reference.intent_hash,consent,AUDIT_PROGRESS]).map_err(|e|e.to_string())?;
    event("audioRenewalAuditStored");
    for (index, part) in bytes.chunks(intent::CHUNK).enumerate() {
        active(lease, check)?;
        let json = intent::chunk_json(part)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)",params![reference.operation_id,intent::slot(index),part.len() as i64,intent::hash(part),json,intent::hash(json.as_bytes())]).map_err(|e|e.to_string())?;
        event("audioRenewalProofStored");
    }
    if tx
        .execute(
            "UPDATE maintenance_operations SET state='completed' WHERE id=?1 AND state='prepared'",
            params![reference.operation_id],
        )
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("残り音声への再同意記録を確定できません".into());
    }
    review.verify(storage, &tx, &cancelled)?;
    if tx.execute("UPDATE maintenance_operations SET state='installing',progress_json=?2,updated_at=?3 WHERE id=?1 AND kind='delete_assets' AND state=?4 AND progress_json=?5",params![old.saved.operation(),next.encode()?,consent,old.progress.state(),old.progress.encode()?]).map_err(|e|e.to_string())? != 1 {
        return Err("残り音声への再同意の切替対象が変更されています".into());
    }
    let saved = Receipt::load(&tx, old.saved.operation(), &cancelled)?;
    if saved.plan != review.snapshot || saved.policy != record.policy {
        return Err("残り音声への再同意の保存結果が一致しません".into());
    }
    super::verify_remaining(storage, &tx, &saved, &cancelled)?;
    event("beforeAudioRenewalCommit");
    active(lease, check)?;
    tx.commit().map_err(|e| e.to_string())?;
    event("audioRenewalCommitted");
    Ok(saved)
}
pub(in crate::storage::deletion) fn audit_source(json: &str) -> Result<String, String> {
    let record: Record = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if record.version != 3
        || record.subtype != "audioRemainingConfirmation"
        || !audio::uuid(&record.source_operation)
        || record.encode()? != json
    {
        return Err("残り音声の確認記録の接続先が不正です".into());
    }
    Ok(record.source_operation)
}
#[cfg(test)]
#[path = "audio_renewal_tests.rs"]
mod tests;
