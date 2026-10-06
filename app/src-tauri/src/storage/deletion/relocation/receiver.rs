//! Qualified read-only inspection of an installed relocation image. Aliases
//! cannot be deserialized or used to change an anchor's current identity.
//! Cold-start admission is qualified by the selected bootstrap's live owner.
//! Fresh-consent resolution and repeated moves remain required before cutover.
use super::{
    check, database, fs, intent, operation_row, paths, row_hash, startup, BoundDirectory,
    Destination, HistoricalDirectory, HistoricalRoot, Record, Source, MAX_BYTES, MAX_OPERATIONS,
    MAX_PATHS, MAX_ROOTS_PER_RECORD,
};
use crate::{
    bootstrap::placement::Placement, storage::backup::relocation::control::ControlHandoff,
};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
mod startup_admission;
pub(in crate::storage) use startup_admission::admitted_history;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub(crate) operation_id: String,
    pub(crate) checked_records: usize,
    pub(crate) completed_records: usize,
    pub(crate) kept_records: usize,
    pub(crate) attention: Vec<(String, String)>,
    read_only: bool,
    execute_allowed: bool,
}
impl Report {
    pub(crate) fn require_clean(&self) -> Result<(), String> {
        if self.attention.is_empty() {
            Ok(())
        } else {
            Err("移動先の削除履歴に確認が必要です。コピーと旧保存先を保持します".into())
        }
    }
}

// inspect() constructs a pre-cutover correspondence from a held source and
// owned copy; startup_admission constructs a cold correspondence only behind
// the selected bootstrap's live owner. Neither path grants deletion authority.
pub(in crate::storage) struct Aliases {
    target: PathBuf,
    original: (fs::Identity, fs::Identity),
    current: (fs::Identity, fs::Identity),
    records: BTreeMap<String, Record>,
    directories: BTreeMap<(String, Vec<u8>), BoundDirectory>,
    cold: bool,
}
impl Aliases {
    pub(in crate::storage::deletion) fn verify_anchor(
        &self,
        path: &Path,
        current: &(fs::Identity, fs::Identity),
    ) -> Result<(), String> {
        if path != self.target || current != &self.current {
            return Err("検査用の履歴対応と現在の保存先・DB実体が一致しません".into());
        }
        Ok(())
    }
    fn eligible(&self, db: &Connection, operation: &str) -> Result<(), String> {
        let expected = self
            .records
            .get(operation)
            .ok_or("所有対応にない削除履歴を採用しません")?;
        let row = db.query_row("SELECT rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1 AND length(CAST(target_manifest_json AS BLOB))+length(CAST(progress_json AS BLOB))+512<=67108864", [operation], operation_row).map_err(|e|e.to_string())?;
        let matches = if self.cold {
            match &expected.immutable_hash {
                Some(hash) => super::immutable_hash(&row)? == *hash,
                None => row_hash(&row)? == expected.row_hash,
            }
        } else {
            row_hash(&row)? == expected.row_hash
        };
        if !matches || row.7 != expected.target_hash || row.4 != expected.state {
            return Err("所有対応を固定した後に削除履歴が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn matches_root(
        &self,
        db: &Connection,
        operation: &str,
        root: &fs::Identity,
        database: &fs::Identity,
        current: &(fs::Identity, fs::Identity),
    ) -> Result<bool, String> {
        if current != &self.current {
            return Ok(false);
        }
        let Some(record) = self.records.get(operation) else {
            return Ok(false);
        };
        if (&self.original.0, &self.original.1) != (root, database)
            && !record
                .historical_roots
                .iter()
                .any(|r| (&r.root, &r.database) == (root, database))
        {
            return Ok(false);
        }
        self.eligible(db, operation)?;
        Ok(true)
    }
    // This permits only capturing a new current-root review. It is not a
    // writer/unlink admission. Legacy whole-row seals cannot survive appended
    // progress, so they remain readable but are not silently upgraded here.
    pub(in crate::storage::deletion) fn matches_reconfirmation_root(
        &self,
        db: &Connection,
        operation: &str,
        root: &fs::Identity,
        database: &fs::Identity,
        current: &(fs::Identity, fs::Identity),
    ) -> Result<bool, String> {
        let Some(record) = self.records.get(operation) else {
            return Ok(false);
        };
        if !self.cold
            || record.immutable_hash.is_none()
            || record.kind != "delete_meeting"
            || record.state != "completed"
        {
            return Ok(false);
        }
        self.matches_root(db, operation, root, database, current)
    }
    pub(in crate::storage::deletion) fn matches_model_reconfirmation_root(
        &self,
        db: &Connection,
        operation: &str,
        root: &fs::Identity,
        database: &fs::Identity,
        current: &(fs::Identity, fs::Identity),
    ) -> Result<bool, String> {
        let Some(record) = self.records.get(operation) else {
            return Ok(false);
        };
        if !self.cold
            || record.immutable_hash.is_none()
            || record.kind != "delete_assets"
            || record.state != "completed"
            || record.validation != "completedModel"
        {
            return Ok(false);
        }
        self.matches_root(db, operation, root, database, current)
    }
    pub(in crate::storage::deletion) fn directory(
        &self,
        db: &Connection,
        operation: &str,
        path: &str,
        expected: &fs::Identity,
    ) -> Result<Option<&fs::Identity>, String> {
        self.eligible(db, operation)?;
        Ok(self
            .directories
            .get(&(path.to_owned(), super::identity_key(expected)?))
            .map(|d| &d.copied))
    }
    fn new(source: &Source, destination: Destination) -> Result<Self, String> {
        Ok(Self {
            target: destination.target_root,
            original: (source.root.clone(), source.database.clone()),
            current: (destination.root, destination.database),
            records: source
                .records
                .iter()
                .map(|r| (r.operation_id.clone(), r.clone()))
                .collect(),
            directories: directory_index(
                destination.directories,
                destination.historical_directories,
            )?,
            cold: false,
        })
    }
    fn verify_inheritance(&self, current: &(fs::Identity, fs::Identity)) -> Result<(), String> {
        if !self.cold || current != &self.current {
            return Err("再移動へ引き継ぐ履歴は選択済みの所有確認した保存先ではありません".into());
        }
        Ok(())
    }
    fn eligible_inheritance(
        &self,
        db: &Connection,
        operation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check(cancelled)?;
        self.eligible(db, operation)?;
        let record = self
            .records
            .get(operation)
            .ok_or("引き継ぐ元の履歴がありません")?;
        if paths(db, operation, cancelled)? != (record.path_count, record.paths_hash.clone()) {
            return Err("再移動へ引き継ぐ元の対象行が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn record_roots(
        &self,
        db: &Connection,
        operation: &str,
        current: &(fs::Identity, fs::Identity),
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<HistoricalRoot>, String> {
        check(cancelled)?;
        self.verify_inheritance(current)?;
        let Some(record) = self.records.get(operation) else {
            return Ok(Vec::new());
        };
        self.eligible_inheritance(db, operation, cancelled)?;
        let mut roots = BTreeMap::new();
        for root in record
            .historical_roots
            .iter()
            .cloned()
            .chain(std::iter::once(HistoricalRoot {
                root: self.original.0.clone(),
                database: self.original.1.clone(),
            }))
        {
            check(cancelled)?;
            if (&root.root, &root.database) != (&current.0, &current.1) {
                roots.insert(super::root_key(&root)?, root);
            }
        }
        if roots.len() > MAX_ROOTS_PER_RECORD {
            return Err("再移動の履歴実体が操作ごとの安全上限を超えています".into());
        }
        Ok(roots.into_values().collect())
    }
    pub(in crate::storage::deletion) fn directory_history(
        &self,
        db: &Connection,
        operations: &[&str],
        actual: &[super::Directory],
        current: &(fs::Identity, fs::Identity),
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<HistoricalDirectory>, String> {
        check(cancelled)?;
        self.verify_inheritance(current)?;
        let operations: BTreeSet<_> = operations.iter().copied().collect();
        for operation in self.records.keys() {
            check(cancelled)?;
            if !operations.contains(operation.as_str()) {
                return Err("再移動へ引き継ぐ元の操作が欠落しています".into());
            }
            self.eligible_inheritance(db, operation, cancelled)?;
        }
        let actual: BTreeMap<_, _> = actual
            .iter()
            .map(|d| (d.relative_path.as_str(), &d.identity))
            .collect();
        let mut result = Vec::new();
        for d in self.directories.values() {
            check(cancelled)?;
            // Absent/replaced directories are not adopted through an old
            // correspondence. The typed receipt scan must independently be
            // clean before capture; it decides whether any old parent is needed.
            if actual.get(d.relative_path.as_str()) == Some(&&d.copied) && d.original != d.copied {
                if result.len() >= MAX_PATHS {
                    return Err("再移動の親実体履歴が安全上限を超えています".into());
                }
                result.push(HistoricalDirectory {
                    relative_path: d.relative_path.clone(),
                    historical: d.original.clone(),
                    current: d.copied.clone(),
                });
            }
        }
        Ok(result)
    }
}

fn directory_index(
    actual: Vec<BoundDirectory>,
    historical: Vec<BoundDirectory>,
) -> Result<BTreeMap<(String, Vec<u8>), BoundDirectory>, String> {
    let mut result = BTreeMap::new();
    for d in actual.into_iter().chain(historical) {
        let key = (d.relative_path.clone(), super::identity_key(&d.original)?);
        if result.len() >= MAX_PATHS || result.insert(key, d).is_some() {
            return Err("履歴の親実体対応が重複または上限超過しています".into());
        }
    }
    Ok(result)
}

fn verify_records(
    db: &Connection,
    source: &Source,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    if database::trusted_schema(db)? != source.schema_hash {
        return Err("コピー先の削除履歴のDB定義が一致しません".into());
    }
    let (count, bytes): (i64, i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(target_manifest_json AS BLOB))+length(CAST(progress_json AS BLOB))+512),0) FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets')",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let (path_count, path_bytes): (i64, i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(p.relative_path AS BLOB))+coalesce(length(CAST(p.identity_json AS BLOB)),0)+256),0) FROM maintenance_paths p JOIN maintenance_operations m ON m.id=p.operation_id WHERE m.kind IN ('delete_meeting','delete_assets')",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if count != source.records.len() as i64
        || !(0..=MAX_OPERATIONS as i64).contains(&count)
        || !(0..=MAX_BYTES as i64).contains(&bytes)
        || !(0..=MAX_BYTES as i64).contains(&path_bytes)
        || bytes + path_bytes > MAX_BYTES as i64
        || !(0..=MAX_PATHS as i64).contains(&path_count)
    {
        return Err("コピー先の削除履歴が件数・容量の固定記録と一致しません".into());
    }
    let mut q = db.prepare("SELECT rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets') ORDER BY id LIMIT 10001").map_err(|e|e.to_string())?;
    let mut rows = q.query([]).map_err(|e| e.to_string())?;
    for expected in &source.records {
        check(cancelled)?;
        let row = operation_row(
            rows.next()
                .map_err(|e| e.to_string())?
                .ok_or("コピー先の削除履歴が欠落しています")?,
        )
        .map_err(|e| e.to_string())?;
        if row.1 != expected.operation_id
            || row.3 != expected.kind
            || row.4 != expected.state
            || row.7 != expected.target_hash
            || row_hash(&row)? != expected.row_hash
            || paths(db, &row.1, cancelled)? != (expected.path_count, expected.paths_hash.clone())
        {
            return Err("コピー先の削除同意・対象・進捗が元の履歴と一致しません".into());
        }
    }
    if rows.next().map_err(|e| e.to_string())?.is_some() {
        return Err("コピー先に未確認の削除履歴があります".into());
    }
    Ok(())
}

pub(crate) fn inspect(
    handoff: &ControlHandoff<'_, '_>,
    placement: &Placement<'_>,
    installed_control: &str,
    installed_hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Report, String> {
    check(cancelled)?;
    handoff.verify(cancelled)?;
    handoff.check_compatible(installed_control.as_bytes())?;
    placement.verify(handoff.snapshot(), cancelled)?;
    let progress = placement.progress()?;
    let mut report = Report {
        operation_id: progress.operation_id.clone(),
        checked_records: 0,
        completed_records: 0,
        kept_records: 0,
        attention: vec![],
        read_only: true,
        execute_allowed: false,
    };
    let Some(source) = handoff.native_deletion_history() else {
        return Ok(report);
    };
    let destination = source.bind_destination(
        handoff.snapshot(),
        placement,
        &progress.operation_id,
        cancelled,
    )?;
    let root = fs::Directory::root(&progress.target)?;
    let registry_file = root.regular(".mon-scratch.sqlite3")?;
    let registry = Connection::open_with_flags(
        progress.target.join(".mon-scratch.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    registry
        .execute_batch("PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    let (local, hash, control, native): (String, String, String, String) = registry.query_row("SELECT local_json,local_hash,control_hash,native_hash FROM relocation_history WHERE operation_id=?1 AND length(CAST(local_json AS BLOB))<=67108864",[&progress.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    if format!("{:x}", Sha256::digest(local.as_bytes())) != hash
        || control != installed_hash
        || native != handoff.snapshot().manifest_hash()
    {
        return Err("配置済みの削除履歴の操作・ハッシュが一致しません".into());
    }
    let local: serde_json::Value = serde_json::from_str(&local).map_err(|e| e.to_string())?;
    if local.get("deletion_history")
        != Some(&serde_json::to_value(source).map_err(|e| e.to_string())?)
        || local.get("deletion_binding")
            != Some(&serde_json::to_value(&destination).map_err(|e| e.to_string())?)
    {
        return Err("配置済みの削除履歴と所有確認したコピー先対応が一致しません".into());
    }
    root.verify_file(".mon-scratch.sqlite3", &registry_file)?;
    let aliases = Arc::new(Aliases::new(source, destination)?);
    let mut anchor = fs::RootAnchor::bind(fs::RootAnchor::directory(&progress.target)?)?
        .with_inspection_history(aliases)?;
    anchor.seal_controls()?;
    let db = Connection::open_with_flags(
        progress.target.join("meetings.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    anchor.verify()?;
    verify_records(&db, source, cancelled)?;
    let models = handoff.snapshot().native_models_root();
    if models != source.models_root
        || source
            .models_identity
            .as_ref()
            .is_some_and(|id| fs::Directory::root(models).map_or(true, |d| &d.identity != id))
    {
        return Err("承認済みのモデル保存先が移動元と一致しません".into());
    }
    for record in &source.records {
        check(cancelled)?;
        if record.state == "recovered" {
            match record.validation.as_str() {
                "keptMeeting" => {
                    let saved = intent::read_kept_database(&db, &record.operation_id, cancelled)?;
                    if !anchor.matches_history_root(
                        &db,
                        &record.operation_id,
                        &saved.manifest().root,
                        &saved.manifest().database,
                    )? {
                        return Err("保持した会議削除履歴の実体が一致しません".into());
                    }
                }
                "keptAudio" => {
                    super::super::audio::intent::read(&db, &record.operation_id, cancelled)?
                        .verify_kept_history_anchor(&db, &anchor)?
                }
                "keptModel" => super::super::models::intent::read(&db, &record.operation_id)?
                    .verify_kept_history_anchor(&db, &anchor, models)?,
                _ => return Err("保持した削除履歴の型が一致しません".into()),
            }
            report.kept_records += 1;
        } else {
            report.completed_records += 1;
        }
    }
    report.attention = startup::completed_attention(&db, &anchor, models)?
        .into_iter()
        .map(|(operation, reason)| (operation, reason.into()))
        .collect();
    report.checked_records = source.records.len();
    verify_records(&db, source, cancelled)?;
    anchor.verify()?;
    root.verify_file(".mon-scratch.sqlite3", &registry_file)?;
    handoff.verify(cancelled)?;
    placement.verify(handoff.snapshot(), cancelled)?;
    check(cancelled)?;
    Ok(report)
}

// Test fixtures own every supplied storage/model/audio path. Exercise the real
// bootstrap, held snapshot, native copy and installed journal, not JSON mocks.
#[cfg(test)]
pub(crate) fn synthetic_remove_kept_audio_record(
    storage: &crate::storage::Storage,
    operation: &str,
) {
    use rusqlite::config::DbConfig;
    let db = storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT kind||':'||state FROM maintenance_operations WHERE id=?1",
            [operation],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "delete_assets:recovered"
    );
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute(
        "DELETE FROM maintenance_paths WHERE operation_id=?1",
        [operation],
    )
    .unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM maintenance_operations WHERE id=?1",
            [operation]
        )
        .unwrap(),
        1
    );
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
}

#[cfg(test)]
pub(crate) fn synthetic_consent_record(
    storage: &crate::storage::Storage,
    operation: &str,
) -> String {
    storage.connection.lock().unwrap().query_row("SELECT json_array(target_manifest_json,target_hash,consent_at,created_at) FROM maintenance_operations WHERE id=?1",[operation],|r|r.get(0)).unwrap()
}

#[cfg(test)]
pub(crate) fn synthetic_meeting_reconfirmation_records(
    storage: &crate::storage::Storage,
) -> Vec<(String, String, i64)> {
    let db = storage.connection.lock().unwrap();
    let mut query = db.prepare("SELECT id,target_manifest_json,consent_at FROM maintenance_operations WHERE kind='delete_meeting' AND progress_json='{\"confirmationRecorded\":true}' ORDER BY created_at,id").unwrap();
    let rows = query
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows
}

#[cfg(test)]
pub(crate) fn synthetic_model_reconfirmation_records(
    storage: &crate::storage::Storage,
) -> Vec<(String, String, String, i64)> {
    let db = storage.connection.lock().unwrap();
    let rows: Vec<(String, String, i64)> = db.prepare("SELECT id,target_manifest_json,consent_at FROM maintenance_operations WHERE kind='delete_assets' AND progress_json='{\"modelReconfirmationRecorded\":true}' ORDER BY created_at,id").unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    rows.into_iter().map(|(id, record, consent)| {
        let chunks: Vec<String> = db.prepare("SELECT identity_json FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").unwrap().query_map([&id], |r| r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        let mut bytes = Vec::new();
        for chunk in chunks {
            let value: serde_json::Value = serde_json::from_str(&chunk).unwrap();
            for part in value["data"].as_str().unwrap().as_bytes().chunks_exact(2) {
                bytes.push(u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap());
            }
        }
        (id, record, String::from_utf8(bytes).unwrap(), consent)
    }).collect()
}

#[cfg(test)]
pub(crate) fn synthetic_reconfirmation_cancel_at(
    storage: &crate::storage::Storage,
    operation: &str,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    checkpoint: &str,
) {
    use super::super::{completion, renewal};
    let receipt = completion::read(storage, operation, &|| false).unwrap();
    let review = renewal::Review::capture_database(storage, &receipt, &|| false).unwrap();
    let stop = std::cell::Cell::new(false);
    assert!(renewal::accept_database_observed(
        storage,
        &receipt,
        &review,
        true,
        lease,
        &|| stop.get(),
        &|event| {
            if event == checkpoint {
                stop.set(true);
            }
        }
    )
    .is_err());
    assert!(stop.get());
    receipt
        .verify_saved(&storage.connection.lock().unwrap(), &|| false)
        .unwrap();
}

#[cfg(test)]
pub(crate) fn inspect_synthetic_copy(storage: &crate::storage::Storage) -> Report {
    use crate::bootstrap::Bootstrap;
    use std::os::unix::fs::DirBuilderExt;
    let base = tempfile::tempdir().unwrap();
    let base = base.path().canonicalize().unwrap();
    let bootstrap = Bootstrap::open(&base.join("config"), storage.root()).unwrap();
    bootstrap.initialize(storage).unwrap();
    let target = base.join("copy");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    bootstrap
        .begin_move(
            &operation,
            &uuid::Uuid::new_v4().to_string(),
            &target,
            snapshot.manifest_hash(),
            0,
        )
        .unwrap();
    let mut placement = bootstrap.pending_placement(&operation, &snapshot).unwrap();
    placement.copy(&snapshot, &|| false).unwrap();
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    placement.install_history(&handoff, &|| false).unwrap();
    let before = std::fs::read(target.join("meetings.sqlite3")).unwrap();
    let registry_before = std::fs::read(target.join(".mon-scratch.sqlite3")).unwrap();
    let report = placement
        .inspect_deletion_handoff(&handoff, &|| false)
        .unwrap();
    report.require_clean().unwrap();
    assert_eq!(
        std::fs::read(target.join("meetings.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(target.join(".mon-scratch.sqlite3")).unwrap(),
        registry_before
    );
    assert_eq!(bootstrap.view().unwrap().meeting_root, storage.root());
    report
}

#[cfg(test)]
pub(crate) fn synthetic_keep_meeting(storage: &crate::storage::Storage, meeting: &str) -> String {
    let census = super::super::files::Census::capture(storage, meeting, &|| false).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let prepared = intent::prepare(storage, &operation, &census, &|| false).unwrap();
    intent::keep_unstarted(storage, &prepared, &|| false).unwrap();
    operation
}

#[cfg(test)]
pub(crate) fn synthetic_restored_audio_operation(
    storage: &crate::storage::Storage,
    meeting: &str,
) -> String {
    let db = storage.connection.lock().unwrap();
    let operation: String = db.query_row("SELECT id FROM maintenance_operations WHERE meeting_id=?1 AND kind='delete_assets' AND state='completed' AND consent_at IS NULL", [meeting], |r|r.get(0)).unwrap();
    super::super::audio::restored::deleted(&db, &operation, meeting).unwrap();
    operation
}

#[cfg(test)]
pub(crate) fn synthetic_restored_audio_paths(
    storage: &crate::storage::Storage,
    operation: &str,
) -> Vec<std::path::PathBuf> {
    let db = storage.connection.lock().unwrap();
    let source = super::super::audio::restored::Source::load(&db, operation, &|| false).unwrap();
    source.verify_anchor(&db, &storage.managed_anchor).unwrap();
    source
        .paths()
        .iter()
        .map(|p| storage.root().join(p))
        .collect()
}

#[cfg(test)]
pub(crate) fn synthetic_audio_operation_record(
    storage: &crate::storage::Storage,
    operation: &str,
) -> String {
    storage.connection.lock().unwrap().query_row("SELECT json_array(rowid,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1", [operation], |r|r.get(0)).unwrap()
}
