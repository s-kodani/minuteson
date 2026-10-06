//! Append-only model scope reviews. No client JSON constructs a plan.
//! A review may include only the model and exact historically derived roots.
use super::super as intent;
use super::super::super as models;
use super::{active, Phase, Prepared, Progress, Receipt};
use crate::storage::deletion::fs::Directory;
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use models::{Entry, Observation};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const MAX_GENERATIONS: u32 = 32;
const MAX_ROOTS: usize = 64;
const AUDIT_PROGRESS: &str = r#"{"modelReconfirmationRecorded":true}"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Reference {
    operation_id: String,
    intent_hash: String,
    pub(super) generation: u32,
}
impl Reference {
    pub(super) fn valid(&self) -> bool {
        intent::uuid_valid(&self.operation_id)
            && intent::hash_valid(&self.intent_hash)
            && self.generation > 0
            && self.generation <= MAX_GENERATIONS
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    roots: Vec<String>,
    pub(super) observations: Vec<Observation>,
}
impl Plan {
    pub(super) fn original(saved: &Prepared) -> Self {
        let package = &saved.header.package_id;
        let observations = saved
            .snapshot
            .observations
            .iter()
            .cloned()
            .map(|mut o| {
                o.entry.relative_path = if o.entry.relative_path.is_empty() {
                    package.clone()
                } else {
                    format!("{package}/{}", o.entry.relative_path)
                };
                o
            })
            .collect();
        Self {
            roots: vec![package.clone()],
            observations,
        }
    }
    fn validate(&self, saved: &Prepared, roots: &[String]) -> Result<(), String> {
        if self.roots != roots
            || roots.is_empty()
            || roots.len() > MAX_ROOTS
            || roots.windows(2).any(|r| r[0] >= r[1])
            || roots.iter().any(|r| {
                r.is_empty()
                    || r.len() > 255
                    || r.contains(['/', '\\', '\0', ':'])
                    || matches!(r.as_str(), "." | "..")
            })
            || self.observations.len() > models::MAX_ENTRIES
        {
            return Err("モデル再確認のルート・件数が不正です".into());
        }
        let registered = registered(saved);
        let mut paths: BTreeMap<&str, &Entry> = BTreeMap::new();
        let mut previous: Option<&str> = None;
        for o in &self.observations {
            let e = &o.entry;
            let path = e.relative_path.as_str();
            let root = path
                .split('/')
                .next()
                .ok_or("モデル再確認のパスがありません")?;
            if !roots.iter().any(|r| r == root)
                || path.len() > models::MAX_PATH + 256
                || path.split('/').count() > models::MAX_DEPTH + 1
                || path.split('/').any(|s| {
                    s.is_empty()
                        || s.len() > 255
                        || matches!(s, "." | "..")
                        || s.contains(['\\', '\0', ':'])
                })
                || previous.is_some_and(|p| p >= path)
                || e.registered != registered.contains_key(path)
                || registered
                    .get(path)
                    .is_some_and(|v| v.is_none() != e.directory)
                || e.present != o.proof.is_some()
                || (!e.present && !e.registered)
            {
                return Err("モデル再確認の対象・順序・登録分類が不正です".into());
            }
            if let Some(p) = &o.proof {
                p.validate(e.directory)?;
            }
            if e.directory || !e.present {
                if e.bytes.is_some() || e.sha256.is_some() || e.changed {
                    return Err("モデル再確認の容量・変更分類が不正です".into());
                }
            } else {
                let p = o
                    .proof
                    .as_ref()
                    .ok_or("モデル再確認の実体証拠がありません")?;
                let sha = e
                    .sha256
                    .as_deref()
                    .ok_or("モデル再確認の内容証拠がありません")?;
                let changed = registered
                    .get(path)
                    .and_then(Option::as_ref)
                    .is_some_and(|(size, hash)| size.is_some_and(|s| s != p.size) || hash != sha);
                if e.bytes != Some(p.size) || !intent::hash_valid(sha) || e.changed != changed {
                    return Err("モデル再確認の内容証拠が不正です".into());
                }
            }
            if let Some((parent, _)) = path.rsplit_once('/') {
                let parent = paths
                    .get(parent)
                    .ok_or("モデル再確認の親対象がありません")?;
                if !parent.directory || (e.present && !parent.present) {
                    return Err("モデル再確認の親対象が不正です".into());
                }
            } else if !e.directory {
                return Err("モデル再確認のルートはフォルダーに限ります".into());
            }
            paths.insert(path, e);
            previous = Some(path);
        }
        Ok(())
    }
}
fn registered(saved: &Prepared) -> BTreeMap<String, Option<(Option<u64>, String)>> {
    models::expected(&saved.snapshot.record)
        .into_iter()
        .map(|(path, value)| {
            let root = &saved.header.package_id;
            (
                if path.is_empty() {
                    root.clone()
                } else {
                    format!("{root}/{path}")
                },
                value,
            )
        })
        .collect()
}
pub(super) fn review_roots(
    plan: &Plan,
    progress: &Progress,
    operation: &str,
) -> Result<Vec<String>, String> {
    let mut roots: BTreeSet<String> = plan.roots.iter().cloned().collect();
    for (index, o) in plan.observations.iter().rev().enumerate() {
        if !o.entry.relative_path.contains('/') {
            roots.insert(progress.reserved(operation, index));
        }
    }
    if roots.len() > MAX_ROOTS {
        return Err("モデル再確認の予約ルートが安全上限を超えています".into());
    }
    Ok(roots.into_iter().collect())
}
pub(super) fn inventory(
    models_root: &Directory,
    saved: &Prepared,
    roots: &[String],
    check: &impl Fn() -> bool,
) -> Result<Vec<Observation>, String> {
    let mut all = BTreeMap::new();
    for root in roots {
        models::cancel(check)?;
        if let Some(child) = models_root.child(root)? {
            let registered = if *root == saved.header.package_id {
                models::expected(&saved.snapshot.record)
            } else {
                BTreeMap::new()
            };
            let mut local = BTreeMap::new();
            models::walk(&child, "", &registered, &mut local, check)?;
            if all.len() + local.len() > models::MAX_ENTRIES {
                return Err("モデル再確認の全対象が256件の上限を超えています".into());
            }
            for (path, mut o) in local {
                let full = if path.is_empty() {
                    root.clone()
                } else {
                    format!("{root}/{path}")
                };
                o.entry.relative_path = full.clone();
                all.insert(full, o);
            }
        }
    }
    models_root.verify()?;
    Ok(all.into_values().collect())
}

#[derive(Serialize, Deserialize)]
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
    include_unregistered: bool,
}
impl Record {
    fn encode(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}
pub(in crate::storage::deletion::models) fn audit_source(json: &str) -> Result<String, String> {
    let record: Record = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if record.version != 1
        || record.subtype != "modelDeletionReconfirmation"
        || record.encode()? != json
        || !uuid::Uuid::parse_str(&record.source_operation)
            .is_ok_and(|id| id.to_string() == record.source_operation)
    {
        return Err("モデル再同意の接続先が不正です".into());
    }
    Ok(record.source_operation)
}

pub(super) fn audit_ids(
    db: &Connection,
    saved: &Prepared,
    progress: &Progress,
) -> Result<Vec<String>, String> {
    active_plan(db, saved, progress)?;
    let mut current = progress.confirmation.clone();
    let mut ids = Vec::new();
    while let Some(reference) = current {
        if ids.len() >= MAX_GENERATIONS as usize || ids.contains(&reference.operation_id) {
            return Err("モデル再同意の世代・識別子が不正です".into());
        }
        current = read_record(db, &reference, saved)?
            .0
            .previous_progress
            .confirmation;
        ids.push(reference.operation_id);
    }
    Ok(ids)
}
fn source_hash(saved: &Prepared) -> Result<String, String> {
    Ok(intent::digest(
        serde_json::to_string(&saved.header)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    ))
}
pub(super) fn recorded_policy(
    db: &Connection,
    saved: &Prepared,
    progress: &Progress,
) -> Result<bool, String> {
    match &progress.confirmation {
        Some(reference) => Ok(read_record(db, reference, saved)?.0.include_unregistered),
        None => Ok(saved.include_unregistered()),
    }
}
fn read_record(
    db: &Connection,
    reference: &Reference,
    saved: &Prepared,
) -> Result<(Record, Plan, i64), String> {
    let (meeting, batch, kind, state, json, sha, consent, progress, error, created, updated): (Option<String>,Option<String>,String,String,String,String,Option<i64>,String,Option<String>,i64,i64) = db.query_row("SELECT meeting_id,restore_batch_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1", params![reference.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).map_err(|e|e.to_string())?;
    if meeting.is_some()
        || batch.is_some()
        || kind != "delete_assets"
        || state != "completed"
        || progress != AUDIT_PROGRESS
        || error.is_some()
        || json.len() > 65536
        || sha != reference.intent_hash
        || intent::digest(json.as_bytes()) != sha
        || created < saved.consent
        || updated != created
        || consent != Some(created)
    {
        return Err("モデル再同意の固定記録を検証できません".into());
    }
    let record: Record = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if record.version != 1
        || record.subtype != "modelDeletionReconfirmation"
        || record.source_operation != saved.header.operation_id
        || record.source_hash != source_hash(saved)?
        || record.encode()? != json
        || record.snapshot_bytes == 0
        || record.snapshot_bytes > intent::MAX_BYTES
        || record.chunk_count != record.snapshot_bytes.div_ceil(intent::CHUNK)
        || record.chunk_count > intent::MAX_CHUNKS
        || !intent::hash_valid(&record.snapshot_hash)
        || reference.generation
            != record
                .previous_progress
                .confirmation
                .as_ref()
                .map_or(1, |r| r.generation.saturating_add(1))
    {
        return Err("モデル再同意と元の対象・世代の対応が不正です".into());
    }
    let mut query = db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT ?2").map_err(|e|e.to_string())?;
    let mut rows = query
        .query(params![
            reference.operation_id,
            intent::MAX_CHUNKS as i64 + 1
        ])
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(record.snapshot_bytes);
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(0).map_err(|e| e.to_string())?;
        let kind: String = row.get(1).map_err(|e| e.to_string())?;
        let size: i64 = row.get(2).map_err(|e| e.to_string())?;
        let sha: String = row.get(3).map_err(|e| e.to_string())?;
        let json: String = row.get(4).map_err(|e| e.to_string())?;
        let json_sha: String = row.get(5).map_err(|e| e.to_string())?;
        if json.len() > 1024 {
            return Err("モデル再同意の分割記録が容量上限を超えています".into());
        }
        let part = intent::decode_chunk(&json)?;
        if count >= record.chunk_count
            || name != intent::slot(count)
            || kind != "file"
            || size != part.len() as i64
            || sha != intent::digest(&part)
            || json_sha != intent::digest(json.as_bytes())
            || part.is_empty()
            || (count + 1 < record.chunk_count && part.len() != intent::CHUNK)
            || bytes.len() + part.len() > record.snapshot_bytes
        {
            return Err("モデル再同意の分割証拠・順序・容量が一致しません".into());
        }
        bytes.extend(part);
        count += 1;
    }
    if count != record.chunk_count
        || bytes.len() != record.snapshot_bytes
        || intent::digest(&bytes) != record.snapshot_hash
    {
        return Err("モデル再同意の対象証拠が欠落・変更されています".into());
    }
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&plan).map_err(|e| e.to_string())? != bytes
        || plan.observations.iter().any(|o| !o.entry.present)
        || (!record.include_unregistered && plan.observations.iter().any(|o| !o.entry.registered))
    {
        return Err("モデル再同意の対象・未登録物の方針が不正です".into());
    }
    Ok((record, plan, created))
}
pub(super) fn active_plan(
    db: &Connection,
    saved: &Prepared,
    progress: &Progress,
) -> Result<Plan, String> {
    let mut current = progress.confirmation.clone();
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    while let Some(reference) = current {
        if !reference.valid()
            || reference.operation_id == saved.header.operation_id
            || !seen.insert(reference.operation_id.clone())
            || chain.len() >= MAX_GENERATIONS as usize
        {
            return Err("モデル再同意の世代・識別子が不正です".into());
        }
        let (record, plan, consent) = read_record(db, &reference, saved)?;
        current = record.previous_progress.confirmation.clone();
        chain.push((reference, record, plan, consent));
    }
    let mut plan = Plan::original(saved);
    let mut previous = None;
    let mut completed = false;
    let mut latest_consent = saved.consent;
    for (reference, record, next, consent) in chain.into_iter().rev() {
        if record.previous_progress.confirmation != previous
            || record.previous_progress.previously_completed != completed
            || consent < latest_consent
        {
            return Err("モデル再同意の進捗・同意履歴の接続が不正です".into());
        }
        record.previous_progress.validate(saved, &plan)?;
        let roots = review_roots(&plan, &record.previous_progress, &saved.header.operation_id)?;
        next.validate(saved, &roots)?;
        completed |= record.previous_progress.phase == Phase::Completed;
        plan = next;
        previous = Some(reference);
        latest_consent = consent;
    }
    if previous != progress.confirmation || progress.previously_completed != completed {
        return Err("モデル削除の進捗が固定した再同意と一致しません".into());
    }
    progress.validate(saved, &plan)?;
    Ok(plan)
}

#[derive(Clone, Debug)]
pub(crate) struct Review {
    receipt: Receipt,
    plan: Plan,
    census_hash: String,
}
impl Review {
    pub(in crate::storage::deletion::models) fn previous(&self) -> &Receipt {
        &self.receipt
    }
    pub(in crate::storage::deletion::models) fn roots(&self) -> Vec<String> {
        self.plan.roots.clone()
    }
    pub(in crate::storage::deletion::models) fn verify_live(
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
    pub(crate) fn census_hash(&self) -> &str {
        &self.census_hash
    }
    pub(crate) fn entries(&self) -> Vec<Entry> {
        self.plan
            .observations
            .iter()
            .map(|o| o.entry.clone())
            .collect()
    }
    fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.receipt.verify_saved(db)?;
        self.receipt.metadata(storage, db)?;
        let models = Directory::root(Path::new(&self.receipt.saved.header.models_root))?;
        models.verify_identity(&self.receipt.saved.snapshot.models_root)?;
        if inventory(&models, &self.receipt.saved, &self.plan.roots, check)?
            != self.plan.observations
        {
            return Err("モデル再確認後に残りの対象が変わりました。再確認してください".into());
        }
        Ok(())
    }
}
pub(crate) fn review(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Review, String> {
    active(lease, check)?;
    let cancelled = || check() || lease.cancelled();
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let receipt = Receipt::load(&db, operation)?;
    receipt.metadata(storage, &db)?;
    let roots = review_roots(&receipt.plan, &receipt.progress, operation)?;
    let models = Directory::root(Path::new(&receipt.saved.header.models_root))?;
    models.verify_identity(&receipt.saved.snapshot.models_root)?;
    let plan = Plan {
        observations: inventory(&models, &receipt.saved, &roots, &cancelled)?,
        roots,
    };
    plan.validate(&receipt.saved, &plan.roots)?;
    let evidence = serde_json::to_vec(&(source_hash(&receipt.saved)?, &receipt.progress, &plan))
        .map_err(|e| e.to_string())?;
    let review = Review {
        receipt,
        plan,
        census_hash: intent::digest(&evidence),
    };
    review.verify(storage, &db, &cancelled)?;
    active(lease, check)?;
    Ok(review)
}
pub(crate) fn accept(
    storage: &Storage,
    review: &Review,
    expected_hash: &str,
    include_unregistered: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    accept_inner(
        storage,
        review,
        expected_hash,
        include_unregistered,
        lease,
        check,
        &|_| {},
    )
}
fn accept_inner(
    storage: &Storage,
    review: &Review,
    expected_hash: &str,
    include_unregistered: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    active(lease, check)?;
    if expected_hash != review.census_hash
        || (!include_unregistered && review.plan.observations.iter().any(|o| !o.entry.registered))
    {
        return Err("モデル再同意の確認対象・未登録物への明示同意が一致しません".into());
    }
    let cancelled = || check() || lease.cancelled();
    let old = &review.receipt;
    let generation = old
        .progress
        .confirmation
        .as_ref()
        .map_or(1, |r| r.generation + 1);
    if generation > MAX_GENERATIONS {
        return Err("モデル再同意の世代が安全上限を超えています".into());
    }
    let bytes = serde_json::to_vec(&review.plan).map_err(|e| e.to_string())?;
    if bytes.len() > intent::MAX_BYTES {
        return Err("モデル再同意の対象証拠が容量上限を超えています".into());
    }
    let record = Record {
        version: 1,
        subtype: "modelDeletionReconfirmation".into(),
        source_operation: old.saved.header.operation_id.clone(),
        source_hash: source_hash(&old.saved)?,
        previous_progress: old.progress.clone(),
        snapshot_hash: intent::digest(&bytes),
        snapshot_bytes: bytes.len(),
        chunk_count: bytes.len().div_ceil(intent::CHUNK),
        include_unregistered,
    };
    let json = record.encode()?;
    let reference = Reference {
        operation_id: uuid::Uuid::new_v4().to_string(),
        intent_hash: intent::digest(json.as_bytes()),
        generation,
    };
    let next = Progress {
        version: 3,
        phase: Phase::Removing,
        census_hash: old.saved.header.census_hash.clone(),
        next: 0,
        pending: None,
        confirmation: Some(reference.clone()),
        previously_completed: old.progress.previously_completed || old.completed(),
    };
    next.validate(&old.saved, &review.plan)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let current = Receipt::load(&db, &old.saved.header.operation_id)?;
    if current != *old {
        let reference = current
            .progress
            .confirmation
            .as_ref()
            .ok_or("モデル再同意の再送記録がありません")?;
        let (record, plan, _) = read_record(&db, reference, &old.saved)?;
        if current.saved != old.saved
            || record.previous_progress != old.progress
            || plan != review.plan
            || record.include_unregistered != include_unregistered
        {
            return Err("モデル再同意の再送が保存した同意と一致しません".into());
        }
        super::verify_remaining(storage, &db, &current, &cancelled)?;
        active(lease, check)?;
        return Ok(current);
    }
    review.verify(storage, &db, &cancelled)?;
    let models = Directory::root(Path::new(&old.saved.header.models_root))?;
    for (index, o) in review.plan.observations.iter().rev().enumerate() {
        let reserved = next.reserved(&old.saved.header.operation_id, index);
        let path = o
            .entry
            .relative_path
            .rsplit_once('/')
            .map_or_else(|| reserved.clone(), |(p, _)| format!("{p}/{reserved}"));
        if review
            .plan
            .observations
            .iter()
            .any(|o| o.entry.relative_path == path)
            || (!path.contains('/') && models.optional_kind(&reserved)?.is_some())
        {
            return Err("モデル再同意の新しい予約名が既に存在します".into());
        }
    }
    // Completed sources retain their SQL completed state. The v3 residual
    // cursor is conservatively fenced; no original fixed evidence is changed.
    storage.deletion_fence.arm(&db)?;
    let _writer = crate::storage::deletion::fence::write_model_removal(storage, &db, old, lease)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let updated: i64 = tx
        .query_row(
            "SELECT updated_at FROM maintenance_operations WHERE id=?1",
            params![old.saved.header.operation_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let consent = crate::storage::now_ms()?
        .max(old.saved.consent)
        .max(updated);
    review.verify(storage, &tx, &cancelled)?;
    tx.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','prepared',?2,?3,?4,?5,?4,?4)",params![reference.operation_id,json,reference.intent_hash,consent,AUDIT_PROGRESS]).map_err(|e|e.to_string())?;
    event("auditStored");
    for (index, part) in bytes.chunks(intent::CHUNK).enumerate() {
        active(lease, check)?;
        let json = intent::chunk_json(part)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)",params![reference.operation_id,intent::slot(index),part.len() as i64,intent::digest(part),json,intent::digest(json.as_bytes())]).map_err(|e|e.to_string())?;
        event("proofStored");
    }
    tx.execute(
        "UPDATE maintenance_operations SET state='completed' WHERE id=?1 AND state='prepared'",
        params![reference.operation_id],
    )
    .map_err(|e| e.to_string())?;
    event("auditCompleted");
    review.verify(storage, &tx, &cancelled)?;
    if tx.execute("UPDATE maintenance_operations SET progress_json=?2,updated_at=max(updated_at,?3) WHERE id=?1 AND kind='delete_assets' AND state=?4 AND progress_json=?5",params![old.saved.header.operation_id,next.encode()?,consent,old.progress.state(),old.progress.encode()?]).map_err(|e|e.to_string())?!=1 {
        return Err("モデル再同意の切替対象が変更されています".into());
    }
    let saved = Receipt::load(&tx, &old.saved.header.operation_id)?;
    if saved.plan != review.plan {
        return Err("モデル再同意の保存結果が一致しません".into());
    }
    event("beforeCommit");
    active(lease, check)?;
    super::verify_remaining(storage, &tx, &saved, &cancelled)?;
    tx.commit().map_err(|e| e.to_string())?;
    event("committed");
    Ok(saved)
}
#[cfg(test)]
pub(in crate::storage::deletion::models) fn accept_observed(
    storage: &Storage,
    review: &Review,
    include_unregistered: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    accept_inner(
        storage,
        review,
        review.census_hash(),
        include_unregistered,
        lease,
        check,
        event,
    )
}
