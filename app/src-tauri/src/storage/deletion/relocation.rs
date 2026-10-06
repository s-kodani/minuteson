//! Source-side native deletion history for a held relocation image. This is
//! evidence, not a deletion permit or a destination-root admission. In
//! particular, it is deliberately not Deserialize: saved JSON cannot recreate
//! the typed source validation performed while the native write fence is held.
use super::{database, fs, intent, startup};
use crate::storage::Storage;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::Metadata, path::PathBuf};

pub(crate) mod receiver;

const MAX_OPERATIONS: usize = 10_000;
const MAX_PATHS: usize = 200_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ROOTS_PER_RECORD: usize = 64;

// Flat observations only: never an origin JSON, nested history, or a permit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct HistoricalRoot {
    root: fs::Identity,
    database: fs::Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct HistoricalDirectory {
    relative_path: String,
    historical: fs::Identity,
    current: fs::Identity,
}
fn identity_key(identity: &fs::Identity) -> Result<Vec<u8>, String> {
    serde_json::to_vec(identity).map_err(|e| e.to_string())
}
fn root_key(root: &HistoricalRoot) -> Result<Vec<u8>, String> {
    serde_json::to_vec(root).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    operation_id: String,
    kind: String,
    state: String,
    validation: String,
    target_hash: String,
    row_hash: String,
    path_count: usize,
    paths_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    immutable_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    historical_roots: Vec<HistoricalRoot>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Directory {
    relative_path: String,
    identity: fs::Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::storage) struct Source {
    version: u32,
    source_root: PathBuf,
    native_hash: String,
    root: fs::Identity,
    database: fs::Identity,
    models_root: PathBuf,
    models_identity: Option<fs::Identity>,
    schema_hash: String,
    records: Vec<Record>,
    directories: Vec<Directory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    historical_directories: Vec<HistoricalDirectory>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BoundDirectory {
    relative_path: String,
    original: fs::Identity,
    copied: fs::Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::storage) struct Destination {
    version: u32,
    operation: String,
    native_hash: String,
    target_root: PathBuf,
    root: fs::Identity,
    database: fs::Identity,
    directories: Vec<BoundDirectory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    historical_directories: Vec<BoundDirectory>,
}
type OperationRow = (
    i64,
    String,
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
type PathRow = (
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);
fn operation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
}
fn row_hash(row: &OperationRow) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"mon/relocation-deletion-row/v1\0");
    frame(&mut h, row)?;
    Ok(format!("{:x}", h.finalize()))
}
// FK detachment, progress and updated_at may change only through the typed
// native completion/reconfirmation readers. Original consent and target do not.
fn immutable_hash(row: &OperationRow) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"mon/relocation-deletion-immutable/v1\0");
    frame(
        &mut h,
        &(row.0, &row.1, &row.3, &row.5, &row.6, &row.7, row.8, row.11),
    )?;
    Ok(format!("{:x}", h.finalize()))
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("削除履歴の移動準備を取り消しました。元の同意・対象は保持します".into())
    } else {
        Ok(())
    }
}
fn frame(h: &mut Sha256, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
    Ok(())
}
fn paths(
    db: &Connection,
    operation: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(usize, String), String> {
    let mut h = Sha256::new();
    h.update(b"mon/relocation-deletion-paths/v1\0");
    frame(&mut h, &operation)?;
    let mut q = db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT 200001").map_err(|e| e.to_string())?;
    let mut rows = q.query(params![operation]).map_err(|e| e.to_string())?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check(cancelled)?;
        if count >= MAX_PATHS {
            return Err("移動する削除履歴の対象件数が上限を超えています".into());
        }
        let value: PathRow = (
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            row.get(3).map_err(|e| e.to_string())?,
            row.get(4).map_err(|e| e.to_string())?,
            row.get(5).map_err(|e| e.to_string())?,
        );
        frame(&mut h, &value)?;
        count += 1;
    }
    Ok((count, format!("{:x}", h.finalize())))
}
fn validate_kept(
    storage: &Storage,
    db: &Connection,
    row: &OperationRow,
    cancelled: &impl Fn() -> bool,
) -> Result<&'static str, String> {
    let operation = &row.1;
    if row.3 == "delete_meeting" {
        let saved = intent::read_kept_database(db, operation, cancelled)?;
        if !storage.managed_anchor.matches_history_root(
            db,
            operation,
            &saved.manifest().root,
            &saved.manifest().database,
        )? {
            return Err("保持した会議削除履歴の保存先・DB実体が一致しません".into());
        }
        return Ok("keptMeeting");
    }
    let routing: serde_json::Value = serde_json::from_str(&row.6).map_err(|e| e.to_string())?;
    match routing["subtype"].as_str() {
        Some("audioOnlyDeletion") => {
            super::audio::intent::read(db, operation, cancelled)?
                .verify_kept_history_anchor(db, &storage.managed_anchor)?;
            Ok("keptAudio")
        }
        Some("modelDeletion") => {
            super::models::intent::read(db, operation)?.verify_kept_history_anchor(
                db,
                &storage.managed_anchor,
                &storage.native_models_root,
            )?;
            Ok("keptModel")
        }
        _ => Err("保持した削除履歴に未対応の種類があります。元の領域を維持します".into()),
    }
}
fn completed_kind(
    storage: &Storage,
    db: &Connection,
    row: &OperationRow,
    cancelled: &impl Fn() -> bool,
) -> Result<&'static str, String> {
    // completed_attention has already validated these exact rows, including
    // canonical audit membership, restored origin ledgers and supersession.
    if row.3 == "delete_meeting" {
        if row.9 == "{\"confirmationRecorded\":true}" {
            return Ok("meetingReconfirmation");
        }
        if serde_json::from_str::<database::Scope>(&row.6).is_ok() {
            return Ok("legacyDatabaseOnly");
        }
        let (saved, _) = startup::supersession_source(db, &row.1, cancelled)?;
        if !storage.managed_anchor.matches_history_root(
            db,
            &row.1,
            &saved.manifest().root,
            &saved.manifest().database,
        )? {
            return Err("完了した会議削除履歴の保存先・DB実体が一致しません".into());
        }
        return Ok("completedMeeting");
    }
    if row.9 == "{\"modelReconfirmationRecorded\":true}" {
        return Ok("modelReconfirmation");
    }
    if row.9 == super::audio::intent::removal::renewal::AUDIT_PROGRESS {
        return Ok("audioRemainingReconfirmation");
    }
    let routing: serde_json::Value = serde_json::from_str(&row.6).map_err(|e| e.to_string())?;
    match routing["subtype"].as_str() {
        Some("audioOnlyDeletion") => Ok("completedAudio"),
        Some("restoredAudioUnavailable") => Ok("restoredAudioUnavailable"),
        Some("audioHistorySupersession") => Ok("audioHistorySupersession"),
        Some("modelDeletion") => Ok("completedModel"),
        _ => Err("完了した削除履歴に未対応の種類があります。元の領域を維持します".into()),
    }
}
impl Source {
    pub(in crate::storage) fn capture(
        storage: &Storage,
        snapshot: &crate::storage::backup::relocation::NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        check(cancelled)?;
        if !storage
            .maintenance
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("削除履歴の移動元の書込停止を確認できません".into());
        }
        if snapshot.source_root() != storage.root() {
            return Err("削除履歴の元の領域とネイティブコピーが一致しません".into());
        }
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        let db = Connection::open_with_flags(
            snapshot.image().join("meetings.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
            .map_err(|e| e.to_string())?;
        let db = &db;
        let schema_hash = database::trusted_schema(db)?;
        let (count, bytes): (i64, i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(target_manifest_json AS BLOB))+length(CAST(progress_json AS BLOB))+512),0) FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets')", [], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
        if !(0..=MAX_OPERATIONS as i64).contains(&count) || !(0..=MAX_BYTES as i64).contains(&bytes)
        {
            return Err("移動する削除履歴が件数・容量上限を超えています".into());
        }
        if count == 0 {
            // An admitted predecessor cannot disappear into a no-history
            // handoff. Check its retained operation set even in this fast path.
            storage
                .managed_anchor
                .relocation_directory_history(db, &[], &[], cancelled)?;
            return Ok(None);
        }
        let (path_count, path_bytes): (i64, i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(p.relative_path AS BLOB))+coalesce(length(CAST(p.identity_json AS BLOB)),0)+256),0) FROM maintenance_paths p JOIN maintenance_operations m ON m.id=p.operation_id WHERE m.kind IN ('delete_meeting','delete_assets')", [], |r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if !(0..=MAX_PATHS as i64).contains(&path_count)
            || !(0..=MAX_BYTES as i64).contains(&path_bytes)
            || bytes + path_bytes > MAX_BYTES as i64
        {
            return Err("移動する削除対象履歴が件数・容量上限を超えています".into());
        }
        if super::fence::has_pending(db)?
            || !startup::completed_attention(
                db,
                &storage.managed_anchor,
                &storage.native_models_root,
            )?
            .is_empty()
        {
            return Err(
                "未完了・不正・再出現した削除履歴があります。先に元の領域で確認してください".into(),
            );
        }
        check(cancelled)?;
        let (root, database) = storage.managed_anchor.identity()?;
        let mut q = db.prepare("SELECT rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets') ORDER BY id LIMIT 10001").map_err(|e|e.to_string())?;
        let mut rows = q.query([]).map_err(|e| e.to_string())?;
        let mut records = Vec::new();
        let mut has_models = false;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check(cancelled)?;
            if records.len() >= MAX_OPERATIONS {
                return Err("移動する削除履歴の件数が上限を超えています".into());
            }
            let value = operation_row(row).map_err(|e| e.to_string())?;
            let validation = match value.4.as_str() {
                "recovered" => validate_kept(storage, db, &value, cancelled)?,
                "completed" => completed_kind(storage, db, &value, cancelled)?,
                _ => return Err("移動する削除履歴が未完了です。元の領域を維持します".into()),
            };
            has_models |= matches!(
                validation,
                "keptModel" | "completedModel" | "modelReconfirmation"
            );
            let row_hash = row_hash(&value)?;
            let immutable_hash = Some(immutable_hash(&value)?);
            let (path_count, paths_hash) = paths(db, &value.1, cancelled)?;
            let historical_roots = storage
                .managed_anchor
                .relocation_record_roots(db, &value.1, cancelled)?;
            records.push(Record {
                operation_id: value.1,
                kind: value.3,
                state: value.4,
                validation: validation.into(),
                target_hash: value.7,
                row_hash,
                path_count,
                paths_hash,
                immutable_hash,
                historical_roots,
            });
        }
        let models_identity = if has_models {
            Some(fs::Directory::root(&storage.native_models_root)?.identity)
        } else {
            None
        };
        storage.managed_anchor.verify()?;
        check(cancelled)?;
        let mut result = Self {
            version: 2,
            source_root: storage.root.clone(),
            native_hash: snapshot.manifest_hash().into(),
            root,
            database,
            models_root: storage.native_models_root.clone(),
            models_identity,
            schema_hash,
            records,
            directories: Vec::new(),
            historical_directories: Vec::new(),
        };
        for node in snapshot.manifest().entries.iter().filter(|n| n.directory) {
            check(cancelled)?;
            result.push_directory(
                &node.relative_path,
                &snapshot
                    .source_metadata(&node.relative_path)?
                    .ok_or("元の親フォルダーがありません")?,
            )?;
        }
        result.historical_directories = storage.managed_anchor.relocation_directory_history(
            db,
            &result
                .records
                .iter()
                .map(|r| r.operation_id.as_str())
                .collect::<Vec<_>>(),
            &result.directories,
            cancelled,
        )?;
        if !result.historical_directories.is_empty()
            || result
                .records
                .iter()
                .any(|r| !r.historical_roots.is_empty())
        {
            result.version = 3;
        }
        result.verify_size()?;
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        Ok(Some(result))
    }
    // Metadata must come from NativeSnapshot's held source inventory, not an
    // arbitrary caller path. These identities will later be mapped only to
    // freshly owned destination directories by the placement coordinator.
    fn push_directory(&mut self, relative_path: &str, metadata: &Metadata) -> Result<(), String> {
        if self.directories.len() >= MAX_PATHS
            || !matches!(relative_path.split('/').next(), Some("meetings" | "temp"))
            || relative_path.len() > 1024
            || relative_path.contains(['\\', '\0'])
            || relative_path
                .split('/')
                .any(|p| p.is_empty() || matches!(p, "." | ".."))
            || self
                .directories
                .last()
                .is_some_and(|d| d.relative_path.as_str() >= relative_path)
        {
            return Err("移動する削除履歴の親フォルダー記録が不正です".into());
        }
        let proof = fs::Proof::read(metadata)?;
        proof.validate(true)?;
        self.directories.push(Directory {
            relative_path: relative_path.into(),
            identity: proof.identity,
        });
        Ok(())
    }
    pub(in crate::storage) fn verify_size(&self) -> Result<(), String> {
        if self
            .records
            .iter()
            .any(|r| r.historical_roots.len() > MAX_ROOTS_PER_RECORD)
            || self.directories.len()
                + self.historical_directories.len()
                + self
                    .records
                    .iter()
                    .map(|r| r.historical_roots.len())
                    .sum::<usize>()
                > MAX_PATHS
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES
        {
            return Err("移動する削除履歴の固定記録が容量上限を超えています".into());
        }
        Ok(())
    }
    pub(in crate::storage) fn bind_destination(
        &self,
        snapshot: &crate::storage::backup::relocation::NativeSnapshot<'_>,
        placement: &crate::bootstrap::placement::Placement<'_>,
        operation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Destination, String> {
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        placement.verify(snapshot, cancelled)?;
        let progress = placement.progress()?;
        if progress.operation_id != operation
            || progress.state != "ready"
            || self.source_root != snapshot.source_root()
            || self.native_hash != snapshot.manifest_hash()
        {
            return Err("削除履歴のコピー先・移動操作が一致しません".into());
        }
        let root = fs::Directory::root(&progress.target)?;
        let db = placement
            .owned_path("meetings.sqlite3")?
            .ok_or("移動先の所有DBがありません")?;
        let database = fs::Proof::read(&db)?;
        database.validate(false)?;
        let mut directories = Vec::with_capacity(self.directories.len());
        for directory in &self.directories {
            check(cancelled)?;
            let original = snapshot
                .source_metadata(&directory.relative_path)?
                .ok_or("元の親フォルダーが欠落しています")?;
            let original = fs::Proof::read(&original)?;
            original.validate(true)?;
            if original.identity != directory.identity {
                return Err("元の削除履歴の親フォルダーが変更されています".into());
            }
            let copied = placement
                .owned_path(&directory.relative_path)?
                .ok_or("移動先の所有親フォルダーがありません")?;
            let copied = fs::Proof::read(&copied)?;
            copied.validate(true)?;
            directories.push(BoundDirectory {
                relative_path: directory.relative_path.clone(),
                original: directory.identity.clone(),
                copied: copied.identity,
            });
        }
        let mut historical_directories = Vec::with_capacity(self.historical_directories.len());
        for history in &self.historical_directories {
            check(cancelled)?;
            let held = snapshot
                .source_metadata(&history.relative_path)?
                .ok_or("引継ぐ親フォルダーの現在実体が欠落しています")?;
            if fs::Proof::read(&held)?.identity != history.current {
                return Err("引継ぐ親フォルダーの現在実体が変更されています".into());
            }
            let copied = placement
                .owned_path(&history.relative_path)?
                .ok_or("引継ぐ親フォルダーの新しい所有実体がありません")?;
            let copied = fs::Proof::read(&copied)?;
            copied.validate(true)?;
            historical_directories.push(BoundDirectory {
                relative_path: history.relative_path.clone(),
                original: history.historical.clone(),
                copied: copied.identity,
            });
        }
        let result = Destination {
            version: if self.version == 3 { 2 } else { 1 },
            operation: operation.into(),
            native_hash: snapshot.manifest_hash().into(),
            target_root: progress.target,
            root: root.identity.clone(),
            database: database.identity,
            directories,
            historical_directories,
        };
        if serde_json::to_vec(&result)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_BYTES
        {
            return Err("削除履歴のコピー先対応が容量上限を超えています".into());
        }
        placement.verify(snapshot, cancelled)?;
        snapshot.verify_source(cancelled)?;
        root.verify()?;
        check(cancelled)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
