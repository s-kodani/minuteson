//! Persisted observations are parsed only behind the selected bootstrap's live
//! owned admission. They cannot recreate Source, Destination, or delete consent.
use super::*;
use crate::bootstrap::placement::NativeHistoryAdmission;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourceFacts {
    version: u32,
    source_root: PathBuf,
    native_hash: String,
    root: fs::Identity,
    database: fs::Identity,
    models_root: PathBuf,
    models_identity: Option<fs::Identity>,
    schema_hash: String,
    records: Vec<Record>,
    directories: Vec<super::super::Directory>,
    #[serde(default)]
    historical_directories: Vec<super::super::HistoricalDirectory>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DestinationFacts {
    version: u32,
    operation: String,
    native_hash: String,
    target_root: PathBuf,
    root: fs::Identity,
    database: fs::Identity,
    directories: Vec<BoundDirectory>,
    #[serde(default)]
    historical_directories: Vec<BoundDirectory>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    version: u32,
    native_manifest_hash: String,
    history: serde_json::Value,
    deletion_history: Option<serde_json::Value>,
    excluded_area: String,
}
fn sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn relative(path: &str) -> bool {
    path.len() <= 1024
        && !path.contains(['\\', '\0'])
        && matches!(path.split('/').next(), Some("meetings" | "temp"))
        && path
            .split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
}
fn record_valid(record: &Record, version: u32) -> bool {
    uuid::Uuid::parse_str(&record.operation_id)
        .is_ok_and(|id| id.to_string() == record.operation_id)
        && sha(&record.target_hash)
        && sha(&record.row_hash)
        && sha(&record.paths_hash)
        && record.path_count <= MAX_PATHS
        && record.historical_roots.len() <= MAX_ROOTS_PER_RECORD
        && (version == 3 || record.historical_roots.is_empty())
        && match (version, &record.immutable_hash) {
            (1, None) => true,
            (2 | 3, Some(h)) => sha(h),
            _ => false,
        }
        && match (
            record.kind.as_str(),
            record.state.as_str(),
            record.validation.as_str(),
        ) {
            ("delete_meeting", "recovered", "keptMeeting")
            | ("delete_assets", "recovered", "keptAudio" | "keptModel")
            | (
                "delete_meeting",
                "completed",
                "completedMeeting" | "meetingReconfirmation" | "legacyDatabaseOnly",
            )
            | (
                "delete_assets",
                "completed",
                "completedAudio"
                | "restoredAudioUnavailable"
                | "audioHistorySupersession"
                | "completedModel"
                | "modelReconfirmation",
            ) => true,
            _ => false,
        }
}
fn records(db: &Connection, source: &SourceFacts) -> Result<(), String> {
    if database::trusted_schema(db)? != source.schema_hash {
        return Err("承認済み削除履歴のDB定義が一致しません".into());
    }
    let (count, bytes): (i64,i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(target_manifest_json AS BLOB))+length(CAST(progress_json AS BLOB))+512),0) FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets')", [], |r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let (count_paths, bytes_paths): (i64,i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(p.relative_path AS BLOB))+coalesce(length(CAST(p.identity_json AS BLOB)),0)+256),0) FROM maintenance_paths p JOIN maintenance_operations m ON m.id=p.operation_id WHERE m.kind IN ('delete_meeting','delete_assets')",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if !(0..=MAX_OPERATIONS as i64).contains(&count)
        || !(0..=MAX_BYTES as i64).contains(&bytes)
        || !(0..=MAX_PATHS as i64).contains(&count_paths)
        || !(0..=MAX_BYTES as i64).contains(&bytes_paths)
        || bytes + bytes_paths > MAX_BYTES as i64
    {
        return Err("承認済みの削除履歴が安全上限を超えています".into());
    }
    // Other meetings can legitimately add receipts after startup. Only the
    // original, admitted records receive a historical-identity correspondence.
    for expected in &source.records {
        let row = db.query_row("SELECT rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1 AND length(CAST(target_manifest_json AS BLOB))+length(CAST(progress_json AS BLOB))+512<=67108864", [&expected.operation_id], operation_row).map_err(|e|e.to_string())?;
        let fixed = match &expected.immutable_hash {
            Some(hash) => super::super::immutable_hash(&row)? == *hash,
            None => row_hash(&row)? == expected.row_hash,
        };
        if !fixed
            || row.3 != expected.kind
            || row.4 != expected.state
            || row.7 != expected.target_hash
            || paths(db, &row.1, &|| false)? != (expected.path_count, expected.paths_hash.clone())
        {
            return Err("承認済みの元の削除同意・対象が変更・欠落しています".into());
        }
    }
    Ok(())
}

pub(in crate::storage) fn admitted_history(
    admission: &NativeHistoryAdmission<'_, '_>,
) -> Result<Option<Arc<Aliases>>, String> {
    admission.verify()?;
    let saved = admission.control()?;
    if saved.len() > MAX_BYTES
        || format!("{:x}", Sha256::digest(saved.as_bytes())) != admission.control_hash()
    {
        return Err("承認済み制御記録の容量・SHAが一致しません".into());
    }
    let control: Control = serde_json::from_str(&saved).map_err(|e| e.to_string())?;
    if control.native_manifest_hash != admission.native_hash()
        || !sha(&control.native_manifest_hash)
        || !control.history.is_object()
        || control.excluded_area.is_empty()
        || control.excluded_area.len() > 255
    {
        return Err("承認済み制御記録の対象が不正です".into());
    }
    crate::storage::backup::relocation::control::verify_registry_image(
        admission.root(),
        &admission.root().join(".mon-scratch.sqlite3"),
        admission.operation(),
        admission.native_hash(),
        admission.control_hash(),
    )?;
    let root = fs::Directory::root(admission.root())?;
    let held_registry = root.regular(".mon-scratch.sqlite3")?;
    let registry = Connection::open_with_flags(
        admission.root().join(".mon-scratch.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    registry
        .execute_batch("PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    let local_json: String = registry.query_row("SELECT local_json FROM relocation_history WHERE operation_id=?1 AND length(CAST(local_json AS BLOB))<=67108864",[admission.operation()],|r|r.get(0)).map_err(|e|e.to_string())?;
    let local: serde_json::Value = serde_json::from_str(&local_json).map_err(|e| e.to_string())?;
    let Some(facts) = control.deletion_history else {
        if control.version != 3
            || local.get("deletion_history").is_some()
            || local.get("deletion_binding").is_some()
        {
            return Err("削除履歴のない承認へ未知の履歴対応を採用しません".into());
        }
        root.verify_file(".mon-scratch.sqlite3", &held_registry)?;
        admission.verify()?;
        verify_registry_unchanged(admission, &registry, &local_json)?;
        return Ok(None);
    };
    if control.version != 4 || facts.is_null() || local.get("deletion_history") != Some(&facts) {
        return Err("移動由来の削除履歴が元の制御記録と一致しません".into());
    }
    let source: SourceFacts = serde_json::from_value(facts).map_err(|e| e.to_string())?;
    let destination: DestinationFacts = serde_json::from_value(
        local
            .get("deletion_binding")
            .ok_or("削除履歴の所有対応がありません")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    source.root.validate(true)?;
    source.database.validate(false)?;
    destination.root.validate(true)?;
    destination.database.validate(false)?;
    let (path, root_id, db_id) = admission.source();
    if !matches!(source.version, 1 | 2 | 3)
        || source.source_root != path
        || source.root.registered()? != root_id
        || source.database.registered()? != db_id
        || source.native_hash != admission.native_hash()
        || !sha(&source.schema_hash)
        || source.models_root != admission.models_root()
        || source.records.is_empty()
        || source.records.len() > MAX_OPERATIONS
        || source.directories.len() > MAX_PATHS
        || source.historical_directories.len() > MAX_PATHS
        || source.directories.len()
            + source.historical_directories.len()
            + source
                .records
                .iter()
                .map(|r| r.historical_roots.len())
                .sum::<usize>()
            > MAX_PATHS
        || (source.version != 3 && !source.historical_directories.is_empty())
        || (source.version == 3
            && source.historical_directories.is_empty()
            && source.records.iter().all(|r| r.historical_roots.is_empty()))
        || source
            .records
            .windows(2)
            .any(|r| r[0].operation_id >= r[1].operation_id)
        || source
            .records
            .iter()
            .any(|r| !record_valid(r, source.version))
        || source.records.iter().map(|r| r.path_count).sum::<usize>() > MAX_PATHS
        || destination.version != if source.version == 3 { 2 } else { 1 }
        || destination.operation != admission.operation()
        || destination.native_hash != admission.native_hash()
        || destination.target_root != admission.root()
        || destination.root != root.identity
        || destination.directories.len() != source.directories.len()
        || destination.historical_directories.len() != source.historical_directories.len()
    {
        return Err("承認済みの削除履歴・元領域・コピー先が一致しません".into());
    }
    for record in &source.records {
        let mut previous = None;
        for root in &record.historical_roots {
            root.root.validate(true)?;
            root.database.validate(false)?;
            let key = super::super::root_key(root)?;
            if (&root.root, &root.database) == (&source.root, &source.database)
                || previous.as_ref().is_some_and(|p| p >= &key)
            {
                return Err("再移動の領域・DB履歴が重複または順序不正です".into());
            }
            previous = Some(key);
        }
    }
    let has_models = source.records.iter().any(|r| {
        matches!(
            r.validation.as_str(),
            "keptModel" | "completedModel" | "modelReconfirmation"
        )
    });
    if has_models != source.models_identity.is_some() {
        return Err("承認済みモデル領域の実体記録が欠落しています".into());
    }
    if let Some(expected) = &source.models_identity {
        expected.validate(true)?;
        fs::Directory::root(admission.models_root())?.verify_identity(expected)?;
    }
    if source
        .directories
        .windows(2)
        .any(|d| d[0].relative_path >= d[1].relative_path)
    {
        return Err("承認済み親フォルダーの順序が不正です".into());
    }
    for (original, copied) in source.directories.iter().zip(&destination.directories) {
        original.identity.validate(true)?;
        copied.original.validate(true)?;
        copied.copied.validate(true)?;
        if !relative(&original.relative_path)
            || copied.relative_path != original.relative_path
            || copied.original != original.identity
            || copied.copied.registered()? != admission.owned_directory(&copied.relative_path)?
        {
            return Err("履歴の親フォルダーを配置台帳の所有実体へ対応付けられません".into());
        }
    }
    let actual: BTreeMap<_, _> = source
        .directories
        .iter()
        .map(|d| (d.relative_path.as_str(), &d.identity))
        .collect();
    let mut previous = None;
    for (history, bound) in source
        .historical_directories
        .iter()
        .zip(&destination.historical_directories)
    {
        history.historical.validate(true)?;
        history.current.validate(true)?;
        bound.original.validate(true)?;
        bound.copied.validate(true)?;
        let key = (
            history.relative_path.clone(),
            super::super::identity_key(&history.historical)?,
        );
        if !relative(&history.relative_path)
            || history.historical == history.current
            || previous.as_ref().is_some_and(|p| p >= &key)
            || actual.get(history.relative_path.as_str()) != Some(&&history.current)
            || bound.relative_path != history.relative_path
            || bound.original != history.historical
            || bound.copied.registered()? != admission.owned_directory(&history.relative_path)?
        {
            return Err("再移動の過去の親実体と今回の所有コピーが一致しません".into());
        }
        previous = Some(key);
    }
    let aliases = Arc::new(Aliases {
        target: destination.target_root,
        original: (source.root.clone(), source.database.clone()),
        current: (destination.root, destination.database),
        records: source
            .records
            .iter()
            .map(|r| (r.operation_id.clone(), r.clone()))
            .collect(),
        directories: directory_index(destination.directories, destination.historical_directories)?,
        cold: true,
    });
    let mut anchor = fs::RootAnchor::bind(fs::RootAnchor::directory(admission.root())?)?
        .with_inspection_history(aliases.clone())?;
    anchor.seal_controls()?;
    let db = inspection_database(&anchor, admission.root())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    anchor.verify()?;
    records(&db, &source)?;
    db.verify()?;
    for record in &source.records {
        match record.validation.as_str() {
            "keptMeeting" => {
                let saved = intent::read_kept_database(&db, &record.operation_id, &|| false)?;
                if !anchor.matches_history_root(
                    &db,
                    &record.operation_id,
                    &saved.manifest().root,
                    &saved.manifest().database,
                )? {
                    return Err("保持した会議削除の元実体が一致しません".into());
                }
            }
            "keptAudio" => {
                super::super::super::audio::intent::read(&db, &record.operation_id, &|| false)?
                    .verify_kept_history_anchor(&db, &anchor)?
            }
            "keptModel" => super::super::super::models::intent::read(&db, &record.operation_id)?
                .verify_kept_history_anchor(&db, &anchor, admission.models_root())?,
            _ => {}
        }
    }
    // Reappearance/inspection trouble is handled by the normal startup TEMP
    // fence. No files, consent, canonical rows or attention hints are written here.
    records(&db, &source)?;
    db.finish()?;
    anchor.verify()?;
    root.verify_file(".mon-scratch.sqlite3", &held_registry)?;
    admission.verify()?;
    if admission.control()? != saved {
        return Err("起動検査中に元の制御記録が変更されています".into());
    }
    verify_registry_unchanged(admission, &registry, &local_json)?;
    Ok(Some(aliases))
}

struct InspectionDatabase {
    db: Connection,
    root: fs::Directory,
    files: Vec<(String, std::fs::File, fs::Proof)>,
    snapshot: Option<tempfile::TempDir>,
}
impl std::ops::Deref for InspectionDatabase {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.db
    }
}
fn verify_inspection_files(
    root: &fs::Directory,
    files: &[(String, std::fs::File, fs::Proof)],
) -> Result<(), String> {
    for (name, file, proof) in files {
        root.verify_file(name, file)?;
        if fs::Proof::read(&file.metadata().map_err(|e| e.to_string())?)? != *proof {
            return Err("削除履歴の読取中にDB・WAL・共有ファイルが変更されています".into());
        }
    }
    root.verify()
}
impl InspectionDatabase {
    fn verify(&self) -> Result<(), String> {
        verify_inspection_files(&self.root, &self.files)
    }
    fn finish(self) -> Result<(), String> {
        self.verify()?;
        let Self {
            db,
            root,
            files,
            snapshot,
        } = self;
        db.close().map_err(|(_, e)| e.to_string())?;
        verify_inspection_files(&root, &files)?;
        drop(snapshot);
        Ok(())
    }
}

// immutable is used ONLY when all controls are absent. Existing SHM is read-
// only. WAL without SHM is copied from held descriptors into a private snapshot,
// where SQLite may construct its index. This snapshot grants no root/admission
// authority. Held full proofs reject concurrent changes to the actual files.
fn inspection_database(anchor: &fs::RootAnchor, path: &Path) -> Result<InspectionDatabase, String> {
    use std::os::unix::ffi::OsStrExt;
    anchor.verify()?;
    let root = anchor.directory_ref()?;
    let wal = root.optional_kind("meetings.sqlite3-wal")?.is_some();
    let shm = root.optional_kind("meetings.sqlite3-shm")?.is_some();
    if root.optional_kind("meetings.sqlite3-journal")?.is_some() || (!wal && shm) {
        return Err(
            "削除履歴の読取前にDB制御ファイルの回復確認が必要です。元のファイルを保持します".into(),
        );
    }
    let owned_root = fs::Directory::root(path)?;
    owned_root.verify_identity(&root.identity)?;
    let mut files = Vec::new();
    for name in [
        "meetings.sqlite3",
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
    ] {
        if owned_root.optional_kind(name)?.is_some() {
            let file = owned_root.regular(name)?;
            let proof = fs::Proof::read(&file.metadata().map_err(|e| e.to_string())?)?;
            proof.validate(false)?;
            files.push((name.into(), file, proof));
        }
    }
    let snapshot = if wal && !shm {
        Some(private_wal_snapshot(&owned_root, &files)?)
    } else {
        None
    };
    let reading_root = match &snapshot {
        Some(snapshot) => snapshot.path().canonicalize().map_err(|e| e.to_string())?,
        None => path.to_owned(),
    };
    let path = reading_root.join("meetings.sqlite3");
    let encoded = path
        .as_os_str()
        .as_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let option = match (wal, shm) {
        (true, true) => "readonly_shm=1",
        (true, false) => "",
        _ => "immutable=1",
    };
    let db = Connection::open_with_flags(
        format!("file:{encoded}?mode=ro&{option}"),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    anchor.verify()?;
    let inspected = InspectionDatabase {
        db,
        root: owned_root,
        files,
        snapshot,
    };
    inspected.verify()?;
    Ok(inspected)
}

fn private_wal_snapshot(
    root: &fs::Directory,
    files: &[(String, std::fs::File, fs::Proof)],
) -> Result<tempfile::TempDir, String> {
    use std::{
        io::{Read, Seek, SeekFrom},
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    verify_inspection_files(root, files)?;
    let bytes = files.iter().try_fold(0u64, |sum, (_, _, p)| {
        sum.checked_add(p.size)
            .ok_or("読取用コピーの容量が範囲外です")
    })?;
    let needed = bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(64 * 1024 * 1024))
        .ok_or("読取用コピーの容量が範囲外です")?;
    let snapshot = tempfile::Builder::new()
        .prefix(".mon-deletion-read-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|e| e.to_string())?;
    if crate::audio_import::available_bytes(snapshot.path())? < needed {
        return Err(
            "削除履歴の非破壊読取に必要な一時容量がありません。元のDB・WALを保持します".into(),
        );
    }
    for (name, source, proof) in files {
        if !matches!(name.as_str(), "meetings.sqlite3" | "meetings.sqlite3-wal") {
            return Err("読取用コピーへ未知の制御ファイルを採用しません".into());
        }
        root.verify_file(name, source)?;
        let mut input = source.try_clone().map_err(|e| e.to_string())?;
        input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(snapshot.path().join(name))
            .map_err(|e| e.to_string())?;
        let copied = std::io::copy(
            &mut input.take(
                proof
                    .size
                    .checked_add(1)
                    .ok_or("読取用コピーの容量が範囲外です")?,
            ),
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        if copied != proof.size {
            return Err("読取用コピー中にDB・WALの容量が変更されています".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        verify_inspection_files(root, files)?;
    }
    verify_inspection_files(root, files)?;
    Ok(snapshot)
}

fn verify_registry_unchanged(
    admission: &NativeHistoryAdmission<'_, '_>,
    registry: &Connection,
    expected: &str,
) -> Result<(), String> {
    crate::storage::backup::relocation::control::verify_registry_image(
        admission.root(),
        &admission.root().join(".mon-scratch.sqlite3"),
        admission.operation(),
        admission.native_hash(),
        admission.control_hash(),
    )?;
    let current: String = registry.query_row("SELECT local_json FROM relocation_history WHERE operation_id=?1 AND length(CAST(local_json AS BLOB))<=67108864",[admission.operation()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if current != expected {
        return Err("起動検査中に削除履歴の所有対応が変更されています".into());
    }
    admission.verify()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(path: &Path) -> fs::RootAnchor {
        let mut anchor = fs::RootAnchor::bind(fs::RootAnchor::directory(path).unwrap()).unwrap();
        anchor.seal_controls().unwrap();
        anchor
    }
    fn files(path: &Path) -> Vec<Option<Vec<u8>>> {
        [
            "meetings.sqlite3",
            "meetings.sqlite3-wal",
            "meetings.sqlite3-shm",
            "meetings.sqlite3-journal",
        ]
        .iter()
        .map(|n| match std::fs::read(path.join(n)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => panic!("{e}"),
        })
        .collect()
    }
    #[test]
    fn deletion_startup_admission_legacy_version_one_remains_exact_and_cannot_use_new_stamp_relaxation(
    ) {
        use crate::{
            analysis_worker::AnalysisController, storage::deletion::operations, storage::Storage,
        };
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("SYNTHETIC LEGACY RECEIPT").unwrap();
        let mut deletion = operations::Prepared::fresh(&storage, &meeting.id, &|| false).unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        deletion
            .execute(
                &storage,
                &lease,
                &deletion.preview().confirmation_id,
                true,
                &|| false,
                &|_| {},
            )
            .unwrap();
        drop(lease);
        let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
        let source = Source::capture(&storage, &snapshot, &|| false)
            .unwrap()
            .unwrap();
        let current: SourceFacts =
            serde_json::from_value(serde_json::to_value(&source).unwrap()).unwrap();
        let mut legacy = serde_json::to_value(&source).unwrap();
        legacy["version"] = 1.into();
        for record in legacy["records"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("immutableHash");
        }
        let legacy: SourceFacts = serde_json::from_value(legacy).unwrap();
        assert!(legacy.records.iter().all(|r| record_valid(r, 1)));
        drop(snapshot);
        let db = storage.connection.lock().unwrap();
        records(&db, &current).unwrap();
        records(&db, &legacy).unwrap();
        db.execute(
            "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
            [&current.records[0].operation_id],
        )
        .unwrap();
        records(&db, &current).unwrap();
        assert!(records(&db, &legacy).is_err());
    }
    #[test]
    fn deletion_startup_admission_checkpointed_wal_mode_and_uri_characters_create_no_controls() {
        let base = tempfile::tempdir().unwrap();
        let path = base.path().join("owned ? # 日本語 %");
        std::fs::create_dir(&path).unwrap();
        let path = path.canonicalize().unwrap();
        let db = Connection::open(path.join("meetings.sqlite3")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE synthetic(value TEXT); INSERT INTO synthetic VALUES('checkpointed');").unwrap();
        drop(db);
        let before = files(&path);
        assert!(before[1..].iter().all(Option::is_none));
        let a = anchor(&path);
        let read = inspection_database(&a, &path).unwrap();
        assert_eq!(
            read.query_row("SELECT value FROM synthetic", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "checkpointed"
        );
        assert!(read
            .execute("INSERT INTO synthetic VALUES('NEVER WRITE')", [])
            .is_err());
        drop(read);
        a.verify().unwrap();
        assert_eq!(files(&path), before);
    }

    fn crashed_wal(path: &Path) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::deletion::relocation::receiver::startup_admission::tests::deletion_startup_admission_wal_child", "--ignored", "--nocapture"])
            .env("MON_SYNTHETIC_STARTUP_WAL_ROOT", path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn deletion_startup_admission_crashed_wal_committed_frames_are_read_without_control_mutation() {
        for shm in [false, true] {
            let base = tempfile::tempdir().unwrap();
            let path = base.path().canonicalize().unwrap();
            crashed_wal(&path);
            if !shm {
                std::fs::rename(path.join("meetings.sqlite3-shm"), path.join("retained.shm"))
                    .unwrap();
            }
            let before = files(&path);
            assert!(before[1].as_ref().is_some_and(|b| b.len() > 32));
            assert_eq!(before[2].is_some(), shm);
            let a = anchor(&path);
            let read = inspection_database(&a, &path).unwrap();
            let temporary = read.snapshot.as_ref().map(|s| s.path().to_owned());
            assert_eq!(temporary.is_some(), !shm);
            if let Some(temporary) = &temporary {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(temporary).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                assert_eq!(
                    std::fs::metadata(temporary.join("meetings.sqlite3"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
            assert_eq!(
                read.query_row("SELECT value FROM synthetic", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "committed in WAL only"
            );
            assert!(read
                .execute("INSERT INTO synthetic VALUES('NEVER WRITE')", [])
                .is_err());
            read.finish().unwrap();
            if let Some(temporary) = temporary {
                assert!(!temporary.exists());
            }
            a.verify().unwrap();
            assert_eq!(files(&path), before);
        }
    }

    #[test]
    fn deletion_startup_admission_orphan_index_or_rollback_journal_refuses_without_recovery_writes()
    {
        for kind in ["wal", "rollback"] {
            let base = tempfile::tempdir().unwrap();
            let path = base.path().canonicalize().unwrap();
            crashed_wal(&path);
            if kind == "rollback" {
                std::fs::write(
                    path.join("meetings.sqlite3-journal"),
                    b"DO NOT RECOVER OR DELETE",
                )
                .unwrap();
            } else {
                let name = format!("meetings.sqlite3-{kind}");
                std::fs::rename(path.join(&name), path.join(format!("{name}.retained"))).unwrap();
            }
            let before = files(&path);
            let a = anchor(&path);
            assert!(inspection_database(&a, &path).is_err(), "{kind}");
            a.verify().unwrap();
            assert_eq!(files(&path), before);
        }
    }

    #[test]
    fn deletion_startup_admission_control_appearance_after_anchor_is_not_adopted() {
        let base = tempfile::tempdir().unwrap();
        let path = base.path().canonicalize().unwrap();
        let db = Connection::open(path.join("meetings.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE synthetic(value TEXT);")
            .unwrap();
        drop(db);
        let a = anchor(&path);
        std::fs::write(path.join("meetings.sqlite3-wal"), b"UNOWNED DO NOT DELETE").unwrap();
        let before = files(&path);
        assert!(inspection_database(&a, &path).is_err());
        assert_eq!(files(&path), before);
    }

    #[test]
    fn deletion_startup_admission_file_content_change_during_read_is_rejected_without_repair() {
        for name in [
            "meetings.sqlite3",
            "meetings.sqlite3-wal",
            "meetings.sqlite3-shm",
        ] {
            for shm in [false, true] {
                if !shm && name == "meetings.sqlite3-shm" {
                    continue;
                }
                let base = tempfile::tempdir().unwrap();
                let path = base.path().canonicalize().unwrap();
                crashed_wal(&path);
                if !shm {
                    std::fs::rename(path.join("meetings.sqlite3-shm"), path.join("retained.shm"))
                        .unwrap();
                }
                let a = anchor(&path);
                let read = inspection_database(&a, &path).unwrap();
                read.query_row("SELECT value FROM synthetic", [], |r| r.get::<_, String>(0))
                    .unwrap();
                let file = path.join(name);
                let mut changed = std::fs::read(&file).unwrap();
                changed.push(0);
                std::fs::write(&file, &changed).unwrap();
                assert!(read.verify().is_err(), "{name}");
                assert_eq!(std::fs::read(&file).unwrap(), changed);
            }
        }
    }

    #[test]
    #[ignore = "child process only; synthetic WAL fixture handoff required"]
    fn deletion_startup_admission_wal_child() {
        let path =
            std::env::var_os("MON_SYNTHETIC_STARTUP_WAL_ROOT").expect("owned synthetic root");
        let db = Connection::open(PathBuf::from(path).join("meetings.sqlite3")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE synthetic(value TEXT); INSERT INTO synthetic VALUES('committed in WAL only');").unwrap();
        // Bypass connection Drop, leaving the committed WAL and index in place.
        std::process::exit(0);
    }
}
