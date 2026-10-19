//! Durable audio-only consent. Proof-slot names encode observations, NEVER
//! filesystem targets. This module does not delete files or historical rows.
use super::{audio, Census, Disposition, Snapshot};
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const CHUNK: usize = 384;
const MAX_CHUNKS: usize = MAX_BYTES.div_ceil(CHUNK);
const INITIAL: &str = r#"{"version":1,"phase":"audioNotStarted"}"#;
const KEPT: &str = r#"{"version":1,"phase":"audioKeptUnstarted"}"#;

#[path = "audio_reconfirmation.rs"]
pub(crate) mod reconfirmation;
#[path = "audio_removal.rs"]
pub(crate) mod removal;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Policy {
    pub include_unregistered_audio: bool,
    pub retain_unclassified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Header {
    version: u32,
    subtype: String,
    operation_id: String,
    meeting_id: String,
    census_hash: String,
    snapshot_hash: String,
    snapshot_bytes: usize,
    chunk_count: usize,
    policy: Policy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<reconfirmation::Origin>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    version: u32,
    data: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prepared {
    header: Header,
    snapshot: Snapshot,
    consent: i64,
    kept: bool,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn active(lease: &DataMaintenanceLease, check: &impl Fn() -> bool) -> Result<(), String> {
    lease.validate_active()?;
    if check() {
        Err("音声だけの削除の操作を取り消しました。未処理の対象と文書履歴を保持します".into())
    } else {
        Ok(())
    }
}
fn slot(index: usize) -> String {
    format!("audio-deletion-proof/{index:06}")
}
fn chunk_json(bytes: &[u8]) -> Result<String, String> {
    use std::fmt::Write;
    let mut data = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut data, "{byte:02x}").map_err(|e| e.to_string())?;
    }
    serde_json::to_string(&Chunk { version: 1, data }).map_err(|e| e.to_string())
}
fn decode_chunk(json: &str) -> Result<Vec<u8>, String> {
    let value: Chunk = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if value.version != 1
        || value.data.is_empty()
        || value.data.len() > CHUNK * 2
        || value.data.len() % 2 != 0
    {
        return Err("音声削除の分割証拠の形式・容量が不正です".into());
    }
    let digit = |b| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err("音声削除の分割証拠が正規の16進形式ではありません".to_string()),
    };
    let bytes = value
        .data
        .as_bytes()
        .chunks_exact(2)
        .map(|p| Ok(digit(p[0])? * 16 + digit(p[1])?))
        .collect::<Result<Vec<_>, String>>()?;
    if chunk_json(&bytes)? != json {
        return Err("音声削除の分割証拠が正規形式ではありません".into());
    }
    Ok(bytes)
}
impl Prepared {
    fn validate(&self, db: &Connection) -> Result<(), String> {
        self.snapshot.validate(db)?;
        let bytes = self.snapshot.encode()?;
        let h = &self.header;
        let entries = self.snapshot.entries()?;
        if h.version != 3
            || h.origin.as_ref().is_some_and(|origin| {
                !super::uuid(&origin.operation_id)
                    || origin.operation_id == h.operation_id
                    || !hash_valid(&origin.receipt_hash)
            })
            || (h.origin.is_none() && !self.snapshot.historical_audio.is_empty())
            || h.subtype != "audioOnlyDeletion"
            || !super::uuid(&h.operation_id)
            || !super::uuid(&h.meeting_id)
            || h.meeting_id != self.snapshot.scope.meeting_id
            || !hash_valid(&h.census_hash)
            || h.census_hash != self.snapshot.hash()?
            || !hash_valid(&h.snapshot_hash)
            || h.snapshot_hash != hash(&bytes)
            || h.snapshot_bytes != bytes.len()
            || bytes.is_empty()
            || h.chunk_count != bytes.len().div_ceil(CHUNK)
            || h.chunk_count > MAX_CHUNKS
            || self.consent < 0
            || (!h.policy.include_unregistered_audio
                && entries
                    .iter()
                    .any(|e| audio(&e.disposition) && e.asset_id.is_none()))
            || (!h.policy.retain_unclassified
                && entries
                    .iter()
                    .any(|e| e.disposition == Disposition::Unclassified))
        {
            return Err("音声だけの削除の保存された同意・方針・対象が不正です".into());
        }
        Ok(())
    }
    pub(crate) fn operation(&self) -> &str {
        &self.header.operation_id
    }
    pub(crate) fn meeting(&self) -> &str {
        &self.header.meeting_id
    }
    pub(crate) fn kept(&self) -> bool {
        self.kept
    }
    pub(super) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub(super) fn policy(&self) -> &Policy {
        &self.header.policy
    }
    pub(super) fn origin_operation(&self) -> Option<&str> {
        self.header.origin.as_ref().map(|o| o.operation_id.as_str())
    }
    fn capture_scope(
        &self,
        db: &Connection,
        meeting: &str,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<crate::storage::deletion::database::Scope, String> {
        if self.header.origin.is_some() {
            let source = reconfirmation::Source::for_prepared(db, self, check)?;
            crate::storage::deletion::database::Scope::capture_audio_reconfirmation(
                db, meeting, operation, &source, check,
            )
        } else {
            crate::storage::deletion::database::Scope::capture(db, meeting, operation, check)
        }
    }
    pub(in crate::storage::deletion) fn verify_kept_history_anchor(
        &self,
        db: &Connection,
        anchor: &crate::storage::deletion::fs::RootAnchor,
    ) -> Result<(), String> {
        if !self.kept
            || !anchor.matches_history_root(
                db,
                self.operation(),
                &self.snapshot.manifest.root,
                &self.snapshot.manifest.database,
            )?
        {
            return Err("保持した音声削除履歴の保存先・DB実体が一致しません".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_kept_ancestry(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if !self.kept {
            return Err("未開始保持の音声記録ではありません".into());
        }
        if self.header.origin.is_some() {
            let source = reconfirmation::Source::for_prepared(db, self, check)?;
            if source
                .source_bytes()
                .checked_add(self.snapshot.encode()?.len())
                .is_none_or(|n| n > MAX_BYTES)
            {
                return Err("音声保持履歴の証拠合計が容量上限を超えています".into());
            }
        }
        super::cancel(check)
    }
    pub(in crate::storage::deletion) fn verify_saved(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if read(db, self.operation(), check)? != *self {
            return Err("音声だけの削除の保存確認が変更されています".into());
        }
        Ok(())
    }
    fn verify_anchor(&self, storage: &Storage) -> Result<(), String> {
        if storage.retired.load(Ordering::Acquire)
            || storage.maintenance.load(Ordering::Acquire)
            || storage.managed_anchor.identity()?
                != (
                    self.snapshot.manifest.root.clone(),
                    self.snapshot.manifest.database.clone(),
                )
        {
            return Err("音声削除の保存先・DBの実体が一致しません".into());
        }
        Ok(())
    }
    fn verify_locked(
        &self,
        storage: &Storage,
        db: &Connection,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        self.verify_anchor(storage)?;
        let current = if self.header.origin.is_some() {
            let source = reconfirmation::Source::for_prepared(db, self, check)?;
            reconfirmation::capture(storage, db, &source, self.operation(), check)?
        } else {
            Census::capture_locked(storage, db, self.meeting(), self.operation(), &|| {
                check() || lease.cancelled()
            })?
            .snapshot()
        };
        if self.kept || current != self.snapshot {
            return Err(
                "音声削除の確認後に音声・保持履歴が変更されています。再確認してください".into(),
            );
        }
        active(lease, check)
    }
    pub(crate) fn verify_current(
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
        self.verify_saved(&db, &|| check() || lease.cancelled())?;
        self.verify_locked(storage, &db, lease, check)
    }
}

// Reading canonical saved evidence grants neither shared admission nor a writer.
pub(crate) fn read(
    db: &Connection,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    read_with_progress(db, operation, None, check)
}
fn read_with_progress(
    db: &Connection,
    operation: &str,
    admitted: Option<(&str, &str)>,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    read_frame(db, operation, admitted, None, check)
}
fn read_frame(
    db: &Connection,
    operation: &str,
    admitted: Option<(&str, &str)>,
    supersession: Option<&crate::storage::deletion::audio_supersession::Proof>,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    super::cancel(check)?;
    if !super::uuid(operation) {
        return Err("音声削除の操作IDが不正です".into());
    }
    let header_bytes: i64 = db.query_row("SELECT length(CAST(target_manifest_json AS BLOB)) FROM maintenance_operations WHERE id=?1",params![operation],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !(1..=65536).contains(&header_bytes) {
        return Err("音声削除の保存確認が容量上限を超えています".into());
    }
    type Row = (
        Option<String>,
        String,
        String,
        Option<String>,
        String,
        String,
        Option<i64>,
        String,
        Option<String>,
        i64,
        i64,
    );
    let row:Row=db.query_row("SELECT meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1",params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).map_err(|e|e.to_string())?;
    let (meeting, kind, state, batch, json, digest, consent, progress, error, created, updated) =
        row;
    let header: Header = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let kept = state == "recovered";
    let consent = consent.ok_or("音声削除の同意時刻がありません")?;
    let meeting_valid = match supersession {
        None => meeting.as_deref() == Some(header.meeting_id.as_str()),
        Some(proof) => {
            meeting.is_none()
                && if kept {
                    proof.authorizes_kept(operation, &header.meeting_id)
                } else {
                    proof.authorizes(operation, &header.meeting_id)
                }
        }
    };
    if !meeting_valid
        || kind != "delete_assets"
        || batch.is_some()
        || !match admitted {
            None => (state == "prepared" || kept) && progress == if kept { KEPT } else { INITIAL },
            Some((expected_state, expected_progress)) => {
                matches!(expected_state, "installing" | "cleanup" | "completed")
                    && state == expected_state
                    && progress == expected_progress
                    && !kept
            }
        }
        || error.is_some()
        || created != consent
        || created < 0
        || updated < created
        || header.operation_id != operation
        || header.version != 3
        || header.subtype != "audioOnlyDeletion"
        || serde_json::to_string(&header).map_err(|e| e.to_string())? != json
        || hash(json.as_bytes()) != digest
        || header.chunk_count == 0
        || header.chunk_count > MAX_CHUNKS
        || header.snapshot_bytes == 0
        || header.snapshot_bytes > MAX_BYTES
        || header.chunk_count != header.snapshot_bytes.div_ceil(CHUNK)
    {
        return Err("音声削除の保存確認の種類・形式・状態が不正です".into());
    }
    let snapshot = read_snapshot(
        db,
        operation,
        header.snapshot_bytes,
        &header.snapshot_hash,
        header.chunk_count,
        check,
    )?;
    let saved = Prepared {
        header,
        snapshot,
        consent,
        kept,
    };
    saved.validate(db)?;
    super::cancel(check)?;
    Ok(saved)
}

fn read_snapshot(
    db: &Connection,
    operation: &str,
    snapshot_bytes: usize,
    snapshot_hash: &str,
    chunk_count: usize,
    check: &impl Fn() -> bool,
) -> Result<Snapshot, String> {
    if snapshot_bytes == 0
        || snapshot_bytes > MAX_BYTES
        || !hash_valid(snapshot_hash)
        || chunk_count == 0
        || chunk_count > MAX_CHUNKS
        || chunk_count != snapshot_bytes.div_ceil(CHUNK)
    {
        return Err("音声の対象証拠の容量・形式が不正です".into());
    }
    let mut q=db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash,length(CAST(identity_json AS BLOB)),length(CAST(relative_path AS BLOB)) FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT ?2").map_err(|e|e.to_string())?;
    let mut rows = q
        .query(params![operation, MAX_CHUNKS as i64 + 1])
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(snapshot_bytes);
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        super::cancel(check)?;
        let json_len: i64 = row.get(6).map_err(|e| e.to_string())?;
        let path_len: i64 = row.get(7).map_err(|e| e.to_string())?;
        if !(1..=1024).contains(&json_len) || !(1..=256).contains(&path_len) {
            return Err("音声削除の分割証拠の容量が不正です".into());
        }
        let name: String = row.get(0).map_err(|e| e.to_string())?;
        let kind: String = row.get(1).map_err(|e| e.to_string())?;
        let size: i64 = row.get(2).map_err(|e| e.to_string())?;
        let digest: String = row.get(3).map_err(|e| e.to_string())?;
        let json: String = row.get(4).map_err(|e| e.to_string())?;
        let json_hash: String = row.get(5).map_err(|e| e.to_string())?;
        let part = decode_chunk(&json)?;
        if count >= chunk_count
            || name != slot(count)
            || kind != "file"
            || size != part.len() as i64
            || digest != hash(&part)
            || json_hash != hash(json.as_bytes())
            || (count + 1 < chunk_count && part.len() != CHUNK)
            || bytes.len() + part.len() > snapshot_bytes
        {
            return Err("音声削除の分割証拠・順序・容量が一致しません".into());
        }
        bytes.extend(part);
        count += 1;
    }
    if count != chunk_count || bytes.len() != snapshot_bytes || hash(&bytes) != snapshot_hash {
        return Err("音声削除の保存対象証拠が欠落・変更されています".into());
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if snapshot.encode()? != bytes {
        return Err("音声削除の保存対象が正規形式ではありません".into());
    }
    snapshot.validate(db)?;
    super::cancel(check)?;
    Ok(snapshot)
}

pub(crate) fn authorize_review(storage: &Storage, operation: &str) -> Result<(), String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let state: String = db
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if state == "completed" && super::restored::hinted(&db, operation)? {
        reconfirmation::Source::load(&db, operation, &|| false)?.verify_review_anchor(
            storage,
            &db,
            &|| false,
        )?;
    } else if matches!(state.as_str(), "installing" | "cleanup" | "completed") {
        let receipt = removal::Receipt::load(&db, operation, &|| false)?;
        if receipt.completed() {
            reconfirmation::Source::load(&db, operation, &|| false)?.verify_review_anchor(
                storage,
                &db,
                &|| false,
            )?;
        } else {
            receipt.verify_anchor(storage)?;
        }
    } else {
        read(&db, operation, &|| false)?.verify_anchor(storage)?;
    }
    if super::super::fence::other_pending(&db, Some(operation))? {
        return Err("別の未完了保守操作を先に確認してください".into());
    }
    Ok(())
}

pub(crate) fn prepare(
    storage: &Storage,
    operation: &str,
    confirmed: &Census,
    policy: Policy,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    prepare_inner(storage, operation, confirmed, policy, lease, check, &|_| {})
}
#[allow(clippy::too_many_arguments)]
fn prepare_inner(
    storage: &Storage,
    operation: &str,
    confirmed: &Census,
    policy: Policy,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Prepared, String> {
    active(lease, check)?;
    if !super::uuid(operation) {
        return Err("音声削除の操作IDが不正です".into());
    }
    let snapshot = confirmed.snapshot();
    let mut db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_some() {
        let saved = read(&db, operation, &|| check() || lease.cancelled())?;
        if saved.snapshot != snapshot
            || saved.header.policy != policy
            || saved.header.census_hash != confirmed.hash()?
        {
            return Err("同じ音声削除操作IDへ別の確認・方針を採用しません".into());
        }
        saved.verify_locked(storage, &db, lease, check)?;
        storage.deletion_fence.arm(&db)?;
        return Ok(saved);
    }
    storage.require_data_writable()?;
    super::require_source(&db, &snapshot.scope.meeting_id)?;
    if super::super::fence::other_pending(&db, None)? {
        return Err("別の保守操作が未完了です".into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let bytes = snapshot.encode()?;
    let saved = Prepared {
        header: Header {
            version: 3,
            subtype: "audioOnlyDeletion".into(),
            operation_id: operation.into(),
            meeting_id: snapshot.scope.meeting_id.clone(),
            census_hash: confirmed.hash()?,
            snapshot_hash: hash(&bytes),
            snapshot_bytes: bytes.len(),
            chunk_count: bytes.len().div_ceil(CHUNK),
            policy,
            origin: None,
        },
        snapshot,
        consent: crate::storage::now_ms()?,
        kept: false,
    };
    saved.validate(&tx)?;
    saved.verify_locked(storage, &tx, lease, check)?;
    let json = serde_json::to_string(&saved.header).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','prepared',?3,?4,?5,?6,?5,?5)",params![operation,saved.meeting(),json,hash(json.as_bytes()),saved.consent,INITIAL]).map_err(|e|e.to_string())?;
    event("audioIntentStored");
    for (index, part) in bytes.chunks(CHUNK).enumerate() {
        active(lease, check)?;
        let json = chunk_json(part)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)",params![operation,slot(index),part.len() as i64,hash(part),json,hash(json.as_bytes())]).map_err(|e|e.to_string())?;
        event("audioProofStored");
    }
    saved.verify_saved(&tx, &|| check() || lease.cancelled())?;
    saved.verify_locked(storage, &tx, lease, check)?;
    event("audioConsentValidated");
    active(lease, check)?;
    event("beforeAudioConsentCommit");
    saved.verify_locked(storage, &tx, lease, check)?;
    tx.commit().map_err(|e| e.to_string())?;
    // The durable consent wins a racing cancellation; always arm the fence.
    storage.deletion_fence.arm(&db)?;
    event("audioConsentCommitted");
    Ok(saved)
}

pub(crate) fn keep_unstarted(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    keep_inner(storage, operation, lease, check, &|_| {})
}
fn keep_inner(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    active(lease, check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let saved = read(&db, operation, &|| check() || lease.cancelled())?;
    saved.verify_anchor(storage)?;
    if super::super::fence::other_pending(&db, Some(operation))? {
        return Err("別の未完了保守操作を先に確認してください".into());
    }
    if saved.kept {
        return active(lease, check);
    }
    let writer = super::super::fence::write_audio_prepared(storage, &db, &saved, lease, check)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    saved.verify_saved(&tx, &|| check() || lease.cancelled())?;
    active(lease, check)?;
    if tx.execute("UPDATE maintenance_operations SET state='recovered',progress_json=?2,updated_at=max(created_at,?3) WHERE id=?1 AND kind='delete_assets' AND state='prepared' AND progress_json=?4",params![operation,KEPT,crate::storage::now_ms()?,INITIAL]).map_err(|e|e.to_string())?!=1 {
        return Err("音声削除の未開始状態が一致しません".into());
    }
    event("audioKeepStored");
    let mut kept = saved.clone();
    kept.kept = true;
    kept.verify_saved(&tx, &|| check() || lease.cancelled())?;
    active(lease, check)?;
    saved.verify_anchor(storage)?;
    tx.commit().map_err(|e| e.to_string())?;
    drop(writer);
    event("audioKeepCommitted");
    Ok(())
}

#[cfg(test)]
#[path = "audio_intent_tests.rs"]
mod tests;
