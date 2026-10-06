//! A new audio-only operation, never an upgrade of the old removal permit.
//! Sources and reviews are live server-held values, not deserializable grants.
use super::super::{audio, records, Disposition, MeetingCensus, Preview, Snapshot};
use super::removal::{completion, Receipt};
use super::{active, hash, read, Header, Policy, Prepared, CHUNK, INITIAL, MAX_BYTES};
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::atomic::Ordering};

const MAX_GENERATIONS: usize = 32;
const MAX_PATHS: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Origin {
    pub(super) operation_id: String,
    pub(super) receipt_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::storage::deletion) struct Source {
    receipts: Vec<Receipt>,
    restored: Option<super::super::restored::Source>,
    paths: BTreeSet<String>,
    origin: Origin,
}
impl Source {
    pub(super) fn load(
        db: &Connection,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let mut receipts: Vec<Receipt> = Vec::new();
        let mut next = operation.to_owned();
        let mut expected: Option<Origin> = None;
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        let mut restored = None;
        loop {
            super::super::cancel(check)?;
            if receipts.len() >= MAX_GENERATIONS || !seen.insert(next.clone()) {
                return Err("音声再同意の世代上限・循環を検出しました。履歴を保持します".into());
            }
            // A route hint chooses the strict loader only. This terminal
            // baseline has no consent and cannot become a removal receipt.
            if super::super::restored::hinted(db, &next)? {
                let baseline = super::super::restored::Source::load(db, &next, check)?;
                if expected
                    .as_ref()
                    .is_some_and(|e| e.operation_id != next || e.receipt_hash != baseline.seal())
                    || receipts
                        .first()
                        .is_some_and(|r| r.source_saved().meeting() != baseline.meeting())
                    || receipts
                        .last()
                        .is_some_and(|r| r.source_saved().consent < baseline.published())
                {
                    return Err("音声再同意の復元元・会議・公開順序が一致しません".into());
                }
                bytes = bytes
                    .checked_add(baseline.bytes())
                    .ok_or("音声再同意の証拠容量が範囲外です")?;
                if bytes > MAX_BYTES {
                    return Err("音声再同意の証拠合計が容量上限を超えています".into());
                }
                restored = Some(baseline);
                break;
            }
            let receipt = Receipt::load(db, &next, check)?;
            // Do not recurse through JSON-derived parents. Check the bounded
            // chain iteratively; this inner check examines availability only.
            completion::verify_available_state_inner(db, &receipt, check)?;
            let saved = receipt.source_saved();
            let seal = receipt.source_seal(db)?;
            if expected
                .as_ref()
                .is_some_and(|e| e.operation_id != next || e.receipt_hash != seal)
                || receipts
                    .first()
                    .is_some_and(|r| r.source_saved().meeting() != saved.meeting())
                || receipts
                    .last()
                    .is_some_and(|r| r.source_saved().consent < saved.consent)
            {
                return Err("音声再同意の元記録・会議・同意順序が一致しません".into());
            }
            bytes = bytes
                .checked_add(receipt.source_bytes())
                .ok_or("音声再同意の証拠容量が範囲外です")?;
            if bytes > MAX_BYTES {
                return Err("音声再同意の証拠合計が容量上限を超えています".into());
            }
            expected = saved.header.origin.clone();
            receipts.push(receipt);
            let Some(parent) = &expected else {
                break;
            };
            next = parent.operation_id.clone();
        }
        let mut paths = restored.as_ref().map_or_else(BTreeSet::new, |r| r.paths());
        for receipt in receipts.iter().rev() {
            let saved = receipt.source_saved();
            validate_historical(&saved.snapshot, &paths)?;
            paths.extend(receipt.source_paths());
            if paths.len() > MAX_PATHS {
                return Err("音声再同意の対象履歴が安全上限を超えています".into());
            }
        }
        let origin = Origin {
            operation_id: operation.to_owned(),
            receipt_hash: match receipts.first() {
                Some(r) => r.source_seal(db)?,
                None => restored
                    .as_ref()
                    .ok_or("音声再同意の元記録がありません")?
                    .seal()
                    .to_owned(),
            },
        };
        Ok(Self {
            receipts,
            restored,
            paths,
            origin,
        })
    }
    pub(super) fn for_prepared(
        db: &Connection,
        saved: &Prepared,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let origin = saved
            .header
            .origin
            .as_ref()
            .ok_or("音声再同意の元記録がありません")?;
        let source = Self::load(db, &origin.operation_id, check)?;
        if source.origin != *origin
            || source.meeting() != saved.meeting()
            || source.recorded_at() > saved.consent
            || source.generations() >= MAX_GENERATIONS
        {
            return Err("音声再同意の元記録・世代が一致しません".into());
        }
        validate_historical(&saved.snapshot, &source.paths)?;
        Ok(source)
    }
    pub(in crate::storage::deletion) fn meeting(&self) -> &str {
        match self.receipts.first() {
            Some(r) => r.source_saved().meeting(),
            None => self
                .restored
                .as_ref()
                .expect("validated source has a baseline")
                .meeting(),
        }
    }
    fn generations(&self) -> usize {
        self.receipts.len() + usize::from(self.restored.is_some())
    }
    fn recorded_at(&self) -> i64 {
        match self.receipts.first() {
            Some(r) => r.source_saved().consent,
            None => self
                .restored
                .as_ref()
                .expect("validated source has a baseline")
                .published(),
        }
    }
    pub(in crate::storage::deletion) fn verify_saved(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if Self::load(db, &self.origin.operation_id, check)? != *self {
            return Err("音声再同意の元記録が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_review_anchor(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.verify_saved(db, check)?;
        if storage.retired.load(Ordering::Acquire) || storage.maintenance.load(Ordering::Acquire) {
            return Err("音声再同意の元履歴と現在の所有領域が一致しません".into());
        }
        if let Some(receipt) = self.receipts.first() {
            let saved = receipt.source_saved();
            if !storage.managed_anchor.matches_history_root(
                db,
                saved.operation(),
                &saved.snapshot.manifest.root,
                &saved.snapshot.manifest.database,
            )? {
                return Err("音声再同意の元履歴と現在の所有領域が一致しません".into());
            }
        }
        if let Some(baseline) = &self.restored {
            baseline.verify_anchor(db, &storage.managed_anchor)?;
        }
        Ok(())
    }
}
fn historical(snapshot: &Snapshot, paths: &BTreeSet<String>) -> BTreeSet<String> {
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
fn validate_historical(snapshot: &Snapshot, paths: &BTreeSet<String>) -> Result<(), String> {
    if snapshot.historical_audio != historical(snapshot, paths) {
        return Err("音声の履歴対象は元の型付き同意から導いた範囲に限定します".into());
    }
    Ok(())
}
pub(super) fn capture(
    storage: &Storage,
    db: &Connection,
    source: &Source,
    omitted: &str,
    check: &impl Fn() -> bool,
) -> Result<Snapshot, String> {
    if crate::storage::deletion::fence::other_pending(db, (!omitted.is_empty()).then_some(omitted))?
    {
        return Err("別の未完了保守操作を先に確認してください".into());
    }
    let census = MeetingCensus::capture_audio_reconfirmation_locked(
        storage, db, source, omitted, check, None,
    )?;
    let records = records(db, source.meeting(), check)?;
    let mut snapshot = Snapshot {
        scope: census.scope().clone(),
        manifest: census.manifest().clone(),
        records,
        historical_audio: BTreeSet::new(),
    };
    snapshot.historical_audio = historical(&snapshot, &source.paths);
    snapshot.validate(db)?;
    let again = MeetingCensus::capture_audio_reconfirmation_locked(
        storage, db, source, omitted, check, None,
    )?;
    if again.manifest() != census.manifest()
        || again.scope() != census.scope()
        || self::records(db, source.meeting(), check)? != snapshot.records
    {
        return Err("音声再同意の検査中に対象・保持履歴が変更されました".into());
    }
    Ok(snapshot)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Review {
    source: Source,
    snapshot: Snapshot,
}
impl Review {
    pub(in crate::storage::deletion::audio) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub(in crate::storage::deletion::audio) fn source_operation(&self) -> &str {
        &self.source.origin.operation_id
    }
    pub(in crate::storage::deletion::audio) fn source_was_restored(&self) -> bool {
        self.source.receipts.is_empty() && self.source.restored.is_some()
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
    pub(crate) fn capture(
        storage: &Storage,
        operation: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        let cancelled = || check() || lease.cancelled();
        let check = &cancelled;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let source = Source::load(&db, operation, check)?;
        if source.generations() >= MAX_GENERATIONS {
            return Err("音声再同意の世代が上限に達しています".into());
        }
        if !storage.deletion_fence.resume_restart_required() {
            crate::storage::deletion::database::stopped(&db)?;
        }
        let snapshot = capture(storage, &db, &source, "", check)?;
        active(lease, check)?;
        Ok(Self { source, snapshot })
    }
    pub(in crate::storage::deletion) fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if capture(storage, db, &self.source, "", check)? != self.snapshot {
            return Err("音声再同意の確認後に対象・保持履歴が変更されています".into());
        }
        Ok(())
    }
    pub(crate) fn preview(&self, inspection: &str) -> Result<Preview, String> {
        if !super::super::uuid(inspection) {
            return Err("音声再同意の検査IDが不正です".into());
        }
        let entries = self.snapshot.entries()?;
        let targets: Vec<_> = entries.iter().filter(|e| audio(&e.disposition)).collect();
        let bytes = targets.iter().try_fold(0u64, |total, entry| {
            total
                .checked_add(entry.bytes.unwrap_or(0))
                .ok_or("音声容量が範囲外です")
        })?;
        Ok(Preview {
            schema_version: 2,
            inspection_id: inspection.into(),
            meeting_id: self.source.meeting().into(),
            census_hash: self.snapshot.hash()?,
            tracks: self.snapshot.records.tracks.clone(),
            retained: self.snapshot.retained()?,
            audio_files: targets.len(),
            audio_bytes: bytes,
            missing_audio_files: targets.iter().filter(|e| !e.present).count(),
            changed_audio_files: targets.iter().filter(|e| e.changed).count(),
            unregistered_audio_files: targets.iter().filter(|e| e.asset_id.is_none()).count(),
            unclassified_files: entries
                .iter()
                .filter(|e| e.disposition == Disposition::Unclassified)
                .count(),
            entries,
            execute_allowed: false,
        })
    }
}

pub(crate) fn accept(
    storage: &Storage,
    operation: &str,
    review: &Review,
    policy: Policy,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    accept_inner(storage, operation, review, policy, lease, check, &|_| {})
}
#[allow(clippy::too_many_arguments)]
fn accept_inner(
    storage: &Storage,
    operation: &str,
    review: &Review,
    policy: Policy,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Prepared, String> {
    active(lease, check)?;
    let cancelled = || check() || lease.cancelled();
    let check = &cancelled;
    if !super::super::uuid(operation) || operation == review.source.origin.operation_id {
        return Err("音声再同意には新しい操作IDが必要です".into());
    }
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE id=?1)",
            params![operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        let saved = read(&db, operation, check)?;
        if saved.snapshot != review.snapshot
            || saved.header.policy != policy
            || saved.header.origin.as_ref() != Some(&review.source.origin)
        {
            return Err("同じ音声再同意IDへ別の確認を採用しません".into());
        }
        saved.verify_locked(storage, &db, lease, check)?;
        storage.deletion_fence.arm(&db)?;
        return Ok(saved);
    }
    let writer = crate::storage::deletion::fence::write_audio_reconfirmation(
        storage, &db, review, lease, check,
    )?;
    let bytes = review.snapshot.encode()?;
    let saved = Prepared {
        header: Header {
            version: 2,
            subtype: "audioOnlyDeletion".into(),
            operation_id: operation.into(),
            meeting_id: review.source.meeting().into(),
            census_hash: review.snapshot.hash()?,
            snapshot_hash: hash(&bytes),
            snapshot_bytes: bytes.len(),
            chunk_count: bytes.len().div_ceil(CHUNK),
            policy,
            origin: Some(review.source.origin.clone()),
        },
        snapshot: review.snapshot.clone(),
        consent: crate::storage::now_ms()?,
        kept: false,
    };
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    saved.validate(&tx)?;
    saved.verify_locked(storage, &tx, lease, check)?;
    let json = serde_json::to_string(&saved.header).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','prepared',?3,?4,?5,?6,?5,?5)",params![operation,saved.meeting(),json,hash(json.as_bytes()),saved.consent,INITIAL]).map_err(|e|e.to_string())?;
    event("audioReconfirmationStored");
    for (index, part) in bytes.chunks(CHUNK).enumerate() {
        active(lease, check)?;
        let json = super::chunk_json(part)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)",params![operation,super::slot(index),part.len() as i64,hash(part),json,hash(json.as_bytes())]).map_err(|e|e.to_string())?;
        event("audioReconfirmationProofStored");
    }
    saved.verify_saved(&tx, check)?;
    saved.verify_locked(storage, &tx, lease, check)?;
    event("audioReconfirmationValidated");
    active(lease, check)?;
    event("beforeAudioReconfirmationCommit");
    saved.verify_locked(storage, &tx, lease, check)?;
    tx.commit().map_err(|e| e.to_string())?;
    storage.deletion_fence.arm(&db)?;
    drop(writer);
    event("audioReconfirmationCommitted");
    Ok(saved)
}

pub(super) fn clear_resolved_attention(
    storage: &Storage,
    db: &Connection,
    saved: &Prepared,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let Some(_) = &saved.header.origin else {
        return Ok(());
    };
    let source = Source::for_prepared(db, saved, check)?;
    for receipt in &source.receipts {
        super::super::cancel(check)?;
        let operation = receipt.source_saved().operation();
        if crate::storage::deletion::startup::contains(db, operation)?
            && completion::inspect_one(db, &storage.managed_anchor, operation)?.is_none()
        {
            db.execute(
                "DELETE FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
                params![operation],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if let Some(baseline) = &source.restored {
        super::super::cancel(check)?;
        let operation = baseline.operation();
        if crate::storage::deletion::startup::contains(db, operation)?
            && super::super::restored::inspect_one(db, &storage.managed_anchor, operation)?
                .is_none()
        {
            db.execute(
                "DELETE FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
                params![operation],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// Read-only fixture observation through the already-owned connection. A
// second SQLite connection would conflict with its exclusive locking mode.
#[cfg(test)]
pub(crate) fn synthetic_saved(storage: &Storage, operation: &str) -> (String, String) {
    let db = storage.connection.lock().unwrap();
    let receipt = Receipt::load(&db, operation, &|| false).unwrap();
    completion::verify_available_state(&db, &receipt, &|| false).unwrap();
    let saved = receipt.source_saved();
    (
        serde_json::to_string(&saved.header).unwrap(),
        String::from_utf8(saved.snapshot.encode().unwrap()).unwrap(),
    )
}

#[cfg(test)]
#[path = "audio_reconfirmation_tests.rs"]
mod tests;
