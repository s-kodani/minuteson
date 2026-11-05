//! Durable model consent, not a deletion executor. Proof slots below contain
//! canonical snapshot bytes; their logical names MUST NEVER be used as paths
//! for filesystem deletion. A saved ID/hash is not shared admission authority.
use super::Snapshot;
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::atomic::Ordering};

const MAX_BYTES: usize = 1_048_576;
const CHUNK: usize = 384;
const MAX_CHUNKS: usize = MAX_BYTES.div_ceil(CHUNK);
const INITIAL: &str = r#"{"version":1,"phase":"notStarted"}"#;
const KEPT: &str = r#"{"version":1,"phase":"keptUnstarted"}"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Header {
    version: u32,
    subtype: String,
    operation_id: String,
    package_id: String,
    review_hash: String,
    models_root: String,
    schema_hash: String,
    census_hash: String,
    snapshot_bytes: usize,
    chunk_count: usize,
    include_unregistered: bool,
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
#[path = "model_removal.rs"]
pub(crate) mod removal;
fn uuid_valid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
}
pub(super) fn hash_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn slot(index: usize) -> String {
    format!("model-deletion-proof/{index:06}")
}
fn chunk_json(bytes: &[u8]) -> Result<String, String> {
    let mut data = String::with_capacity(bytes.len() * 2);
    use std::fmt::Write;
    for byte in bytes {
        write!(&mut data, "{byte:02x}").map_err(|e| e.to_string())?;
    }
    serde_json::to_string(&Chunk { version: 1, data }).map_err(|e| e.to_string())
}
fn decode_chunk(json: &str) -> Result<Vec<u8>, String> {
    let value: Chunk = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if value.version != 1 || value.data.len() > CHUNK * 2 || value.data.len() % 2 != 0 {
        return Err("モデル削除の分割証拠が不正です".into());
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err("モデル削除の分割証拠が不正です".to_string()),
    };
    let bytes = value
        .data
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(digit(pair[0])? * 16 + digit(pair[1])?))
        .collect::<Result<Vec<_>, String>>()?;
    if chunk_json(&bytes)? != json {
        return Err("モデル削除の分割証拠が正規形式ではありません".into());
    }
    Ok(bytes)
}
fn active(lease: &DataMaintenanceLease, check: &impl Fn() -> bool) -> Result<(), String> {
    lease.validate_model_deletion()?;
    if check() {
        Err("モデル削除の操作を取り消しました。未処理の対象を保持し、既に削除したファイルは復元しません".into())
    } else {
        Ok(())
    }
}
impl Prepared {
    pub(in crate::storage::deletion) fn verify_kept_history_anchor(
        &self,
        db: &Connection,
        anchor: &crate::storage::deletion::fs::RootAnchor,
        models_root: &Path,
    ) -> Result<(), String> {
        if !self.kept
            || !anchor.matches_history_root(
                db,
                &self.header.operation_id,
                &self.snapshot.storage_root,
                &self.snapshot.database,
            )?
            || models_root != self.models_root()
        {
            return Err("保持したモデル削除履歴の保存先・DB実体が一致しません".into());
        }
        super::Directory::root(models_root)?.verify_identity(&self.snapshot.models_root)
    }
    pub(in crate::storage::deletion::models) fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub(in crate::storage::deletion::models) fn models_root(&self) -> &Path {
        Path::new(&self.header.models_root)
    }
    pub(in crate::storage::deletion::models) fn include_unregistered(&self) -> bool {
        self.header.include_unregistered
    }
    fn validate(&self, db: &Connection) -> Result<(), String> {
        #[cfg(not(test))]
        if !self.snapshot.directory_scope {
            return Err("モデルの削除記録は現行のフォルダー確認形式が必要です".into());
        }
        let h = &self.header;
        self.snapshot.validate()?;
        super::super::database::verify_history_schema_hash(db, &h.schema_hash)?;
        let bytes = serde_json::to_vec(&self.snapshot).map_err(|e| e.to_string())?;
        if h.version != if self.snapshot.directory_scope { 2 } else { 1 }
            || h.subtype != "modelDeletion"
            || !uuid_valid(&h.operation_id)
            || !uuid_valid(&h.package_id)
            || h.package_id != self.snapshot.record.package_id
            || h.review_hash != self.snapshot.record.review_hash
            || !hash_valid(&h.review_hash)
            || !hash_valid(&h.census_hash)
            || !hash_valid(&h.schema_hash)
            || !Path::new(&h.models_root).is_absolute()
            || Path::new(&h.models_root).file_name() != Some(std::ffi::OsStr::new("models"))
            || h.models_root.contains('\0')
            || h.snapshot_bytes != bytes.len()
            || bytes.is_empty()
            || bytes.len() > MAX_BYTES
            || digest(&bytes) != h.census_hash
            || h.chunk_count != bytes.len().div_ceil(CHUNK)
            || self.consent < 0
            || self.snapshot.active_jobs != 0
            || (!h.include_unregistered
                && self
                    .snapshot
                    .observations
                    .iter()
                    .any(|o| !o.entry.registered))
        {
            return Err("モデル削除の保存された削除要求・対象・構造が不正です".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_saved(&self, db: &Connection) -> Result<(), String> {
        if read(db, &self.header.operation_id)? != *self {
            return Err("モデル削除の保存確認が変更されています".into());
        }
        Ok(())
    }
    fn verify_anchor(&self, storage: &Storage) -> Result<(), String> {
        if storage.retired.load(Ordering::Acquire)
            || storage.maintenance.load(Ordering::Acquire)
            || storage.managed_anchor.identity()?
                != (
                    self.snapshot.storage_root.clone(),
                    self.snapshot.database.clone(),
                )
        {
            return Err("モデル削除の保存先実体が一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn verify_current(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        self.verify_anchor(storage)?;
        let current = super::capture_mode(
            storage,
            Path::new(&self.header.models_root),
            &self.header.package_id,
            &self.header.review_hash,
            &|| check() || lease.cancelled(),
            self.snapshot.directory_scope,
        )?;
        let db = crate::operation_observation::lock(&storage.connection)?;
        self.verify_saved(&db)?;
        super::super::database::stopped_for_model_consent(&db)?;
        if self.kept
            || super::super::fence::other_pending(&db, Some(&self.header.operation_id))?
            || current != self.snapshot
        {
            return Err("モデル削除の確認後に対象が変更されました。再確認が必要です".into());
        }
        self.snapshot
            .verify_metadata(storage, &db, Path::new(&self.header.models_root), check)?;
        active(lease, check)
    }
}

// Strict typed readback. Nothing in this function removes files or obtains a
// privileged DB writer. Unknown/corrupt delete_assets records stay fenced.
pub(crate) fn read(db: &Connection, operation: &str) -> Result<Prepared, String> {
    read_with_progress(db, operation, None)
}
// Only the typed removal reader supplies an exact canonical state/progress.
// This does not make installing/completed receipts usable as unstarted consent.
fn read_with_progress(
    db: &Connection,
    operation: &str,
    removing: Option<(&str, &str)>,
) -> Result<Prepared, String> {
    if !uuid_valid(operation) {
        return Err("モデル削除の操作IDが不正です".into());
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
    let row: Row = db.query_row("SELECT meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1",params![operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).map_err(|e|e.to_string())?;
    let (meeting, kind, state, batch, json, hash, consent, progress, error, created, updated) = row;
    let header: Header = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let kept = state == "recovered";
    let consent = consent.ok_or("モデル削除の受付時刻がありません")?;
    if meeting.is_some()
        || batch.is_some()
        || kind != "delete_assets"
        || !removing.map_or(state == "prepared" || kept, |(expected, _)| {
            state == expected
        })
        || progress != removing.map_or(if kept { KEPT } else { INITIAL }, |(_, progress)| progress)
        || error.is_some()
        || created != consent
        || updated < created
        || json.len() > 65536
        || serde_json::to_string(&header).map_err(|e| e.to_string())? != json
        || digest(json.as_bytes()) != hash
        || header.operation_id != operation
        || header.chunk_count == 0
        || header.chunk_count > MAX_CHUNKS
        || header.snapshot_bytes == 0
        || header.snapshot_bytes > MAX_BYTES
    {
        return Err("モデル削除の保存確認の形式・状態が不正です".into());
    }
    let mut statement = db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT ?2").map_err(|e|e.to_string())?;
    let mut rows = statement
        .query(params![operation, MAX_CHUNKS as i64 + 1])
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(header.snapshot_bytes);
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(0).map_err(|e| e.to_string())?;
        let kind: String = row.get(1).map_err(|e| e.to_string())?;
        let size: i64 = row.get(2).map_err(|e| e.to_string())?;
        let hash: String = row.get(3).map_err(|e| e.to_string())?;
        let json: String = row.get(4).map_err(|e| e.to_string())?;
        let json_hash: String = row.get(5).map_err(|e| e.to_string())?;
        let part = decode_chunk(&json)?;
        if count >= header.chunk_count
            || name != slot(count)
            || kind != "file"
            || size != part.len() as i64
            || hash != digest(&part)
            || json.len() > 1024
            || json_hash != digest(json.as_bytes())
            || part.is_empty()
            || (count + 1 < header.chunk_count && part.len() != CHUNK)
            || bytes.len() + part.len() > header.snapshot_bytes
        {
            return Err("モデル削除の分割証拠・順序・容量が一致しません".into());
        }
        bytes.extend(part);
        count += 1;
    }
    if count != header.chunk_count
        || bytes.len() != header.snapshot_bytes
        || digest(&bytes) != header.census_hash
    {
        return Err("モデル削除の保存対象証拠が欠落・変更されています".into());
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&snapshot).map_err(|e| e.to_string())? != bytes {
        return Err("モデル削除の保存対象が正規形式ではありません".into());
    }
    let result = Prepared {
        header,
        snapshot,
        consent,
        kept,
    };
    result.validate(db)?;
    Ok(result)
}

#[cfg(test)]
pub(in crate::storage) fn mismatched_schema_header_fixture(db: &Connection, operation: &str) {
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [operation],
            |r| r.get(0),
        )
        .unwrap();
    let mut header: Header = serde_json::from_str(&json).unwrap();
    header.schema_hash = "a".repeat(64);
    let json = serde_json::to_string(&header).unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )
    .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",
        params![operation, json, digest(json.as_bytes())],
    )
    .unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        true,
    )
    .unwrap();
}

pub(crate) fn authorize_review(storage: &Storage, operation: &str) -> Result<(), String> {
    let db = crate::operation_observation::lock(&storage.connection)?;
    let state: String = db
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if matches!(state.as_str(), "installing" | "completed") {
        removal::Receipt::load(&db, operation)?.verify_review_anchor(storage, &db)?;
    } else {
        read(&db, operation)?.verify_anchor(storage)?;
    }
    if super::super::fence::other_pending(&db, Some(operation))? {
        return Err("別の未完了保守操作を先に確認してください".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn prepare(
    storage: &Storage,
    root: &Path,
    operation: &str,
    package: &str,
    review: &str,
    census: &str,
    include_unregistered: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Prepared, String> {
    prepare_mode(
        storage,
        root,
        operation,
        package,
        review,
        census,
        include_unregistered,
        lease,
        check,
        false,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_mode(
    storage: &Storage,
    root: &Path,
    operation: &str,
    package: &str,
    review: &str,
    census: &str,
    include_unregistered: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    directory_scope: bool,
) -> Result<Prepared, String> {
    active(lease, check)?;
    if !uuid_valid(operation) || !hash_valid(census) {
        return Err("モデル削除の操作ID・確認ハッシュが不正です".into());
    }
    let snapshot = super::capture_mode(
        storage,
        root,
        package,
        review,
        &|| check() || lease.cancelled(),
        directory_scope,
    )?;
    let mut db = crate::operation_observation::lock(&storage.connection)?;
    if db
        .query_row(
            "SELECT id FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        let saved = read(&db, operation)?;
        saved.verify_anchor(storage)?;
        super::super::database::stopped_for_model_consent(&db)?;
        if saved.kept
            || saved.header.package_id != package
            || saved.header.review_hash != review
            || saved.header.census_hash != census
            || saved.header.include_unregistered != include_unregistered
            || Path::new(&saved.header.models_root) != root
            || super::super::fence::other_pending(&db, Some(operation))?
            || snapshot != saved.snapshot
        {
            return Err("モデル削除の再送が保存した確認と一致しません".into());
        }
        snapshot.verify_metadata(storage, &db, root, check)?;
        active(lease, check)?;
        storage.deletion_fence.arm(&db)?;
        return Ok(saved);
    }
    storage.require_data_writable()?;
    if storage.maintenance.load(Ordering::Acquire) || super::super::fence::other_pending(&db, None)?
    {
        return Err("別の保守操作が未完了です".into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let schema_hash = super::super::database::trusted_schema(&tx)?;
    super::super::database::stopped_for_model_consent(&tx)?;
    snapshot.verify_metadata(storage, &tx, root, check)?;
    let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    let header = Header {
        version: if snapshot.directory_scope { 2 } else { 1 },
        subtype: "modelDeletion".into(),
        operation_id: operation.into(),
        package_id: package.into(),
        review_hash: review.into(),
        models_root: root
            .to_str()
            .ok_or("モデル管理パスがUTF-8ではありません")?
            .into(),
        schema_hash,
        census_hash: digest(&bytes),
        snapshot_bytes: bytes.len(),
        chunk_count: bytes.len().div_ceil(CHUNK),
        include_unregistered,
    };
    if header.census_hash != census {
        return Err("モデル削除の確認後に対象が変更されました。再確認してください".into());
    }
    let saved = Prepared {
        header,
        snapshot,
        consent: crate::storage::now_ms()?,
        kept: false,
    };
    saved.validate(&tx)?;
    saved.verify_anchor(storage)?;
    let json = serde_json::to_string(&saved.header).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','prepared',?2,?3,?4,?5,?4,?4)",params![operation,json,digest(json.as_bytes()),saved.consent,INITIAL]).map_err(|e|e.to_string())?;
    for (index, part) in bytes.chunks(CHUNK).enumerate() {
        active(lease, check)?;
        let json = chunk_json(part)?;
        tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)",params![operation,slot(index),part.len() as i64,digest(part),json,digest(json.as_bytes())]).map_err(|e|e.to_string())?;
    }
    saved.verify_saved(&tx)?;
    saved.snapshot.verify_metadata(storage, &tx, root, check)?;
    active(lease, check)?;
    saved.verify_anchor(storage)?;
    tx.commit().map_err(|e| e.to_string())?;
    // A cancellation racing COMMIT must never leave ordinary writes admitted.
    storage.deletion_fence.arm(&db)?;
    Ok(saved)
}

// Keep is intentionally independent of the current package tree: it preserves
// externally changed/missing files, and never adopts them as deletion targets.
pub(crate) fn keep_unstarted(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    active(lease, check)?;
    let db = crate::operation_observation::lock(&storage.connection)?;
    let saved = read(&db, operation)?;
    saved.verify_anchor(storage)?;
    if super::super::fence::other_pending(&db, Some(operation))? {
        return Err("別の未完了保守操作を先に確認してください".into());
    }
    if saved.kept {
        return active(lease, check);
    }
    let writer = super::super::fence::write_model_prepared(storage, &db, &saved, lease)?;
    // Writer borrows the Connection; this transaction still owns a proper RAII
    // rollback and we assert autocommit at the writer admission boundary.
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    saved.verify_saved(&tx)?;
    active(lease, check)?;
    let count = tx.execute("UPDATE maintenance_operations SET state='recovered',progress_json=?2,updated_at=max(created_at,?3) WHERE id=?1 AND kind='delete_assets' AND state='prepared' AND progress_json=?4",params![operation,KEPT,crate::storage::now_ms()?,INITIAL]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("モデル削除の未開始状態が一致しません".into());
    }
    let mut kept = saved.clone();
    kept.kept = true;
    kept.verify_saved(&tx)?;
    active(lease, check)?;
    saved.verify_anchor(storage)?;
    tx.commit().map_err(|e| e.to_string())?;
    drop(writer);
    Ok(())
}
