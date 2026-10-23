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
    historical_directories: Vec<BoundDirectory>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    version: u32,
    native_manifest_hash: String,
    history: serde_json::Value,
    deletion_history: Option<serde_json::Value>,
    flac_history: Option<serde_json::Value>,
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
        && version == 4
        && sha(&record.immutable_hash)
        && match (
            record.kind.as_str(),
            record.state.as_str(),
            record.validation.as_str(),
        ) {
            ("delete_meeting", "recovered", "keptMeeting")
            | ("delete_assets", "recovered", "keptAudio" | "keptModel")
            | ("delete_meeting", "completed", "completedMeeting" | "meetingReconfirmation")
            | (
                "delete_assets",
                "completed",
                "completedAudio"
                | "restoredAudioUnavailable"
                | "audioHistorySupersession"
                | "completedModel"
                | "modelReconfirmation"
                | "audioRemainingReconfirmation",
            ) => true,
            _ => false,
        }
}
fn records(db: &Connection, source: &SourceFacts) -> Result<(), String> {
    if source.version != 4
        || source
            .records
            .iter()
            .any(|record| !record_valid(record, source.version))
    {
        return Err("削除履歴は現行の照合ハッシュ付き形式である必要があります".into());
    }
    database::verify_history_schema_hash(db, &source.schema_hash)?;
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
        let fixed = super::super::immutable_hash(&row)? == expected.immutable_hash;
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
        if control.version != if control.flac_history.is_some() { 5 } else { 3 }
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
    if control.version != if control.flac_history.is_some() { 5 } else { 4 }
        || facts.is_null()
        || local.get("deletion_history") != Some(&facts)
    {
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
    if source.version != 4
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
        || source
            .records
            .windows(2)
            .any(|r| r[0].operation_id >= r[1].operation_id)
        || source
            .records
            .iter()
            .any(|r| !record_valid(r, source.version))
        || source.records.iter().map(|r| r.path_count).sum::<usize>() > MAX_PATHS
        || destination.version != 3
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
    let db = inspection_database(&anchor, admission.root(), &|| admission.read_snapshot())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    anchor.verify()?;
    records(&db, &source)?;
    db.verify()?;
    for record in &source.records {
        match record.validation.as_str() {
            "keptMeeting" => super::super::super::audio_supersession::inspect_kept_meeting(
                &db,
                &anchor,
                &record.operation_id,
                &|| false,
            )?,
            "keptAudio" => super::super::super::audio_supersession::inspect_kept(
                &db,
                &anchor,
                &record.operation_id,
                &|| false,
            )?,
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
    snapshot: Option<crate::storage::read_cache::Snapshot>,
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
fn inspection_database(
    anchor: &fs::RootAnchor,
    path: &Path,
    create: &dyn Fn() -> Result<crate::storage::read_cache::Snapshot, String>,
) -> Result<InspectionDatabase, String> {
    inspection_database_inner(anchor, path, create, false)
}

// Startup rebinding always reads an owned copy, including when SHM exists.
// No rejected source DB or WAL is opened by SQLite or changed by inspection.
pub(in crate::storage) fn inspect_rebinding<T>(
    path: &Path,
    cache: &Path,
    inspect: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let mut anchor = fs::RootAnchor::bind(fs::RootAnchor::directory(path)?)?;
    anchor.seal_controls()?;
    let db = inspection_database_inner(
        &anchor,
        path,
        &|| crate::storage::read_cache::Snapshot::create(cache),
        true,
    )?;
    db.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    let value = inspect(&db)?;
    db.finish()?;
    anchor.verify()?;
    Ok(value)
}

fn inspection_database_inner(
    anchor: &fs::RootAnchor,
    path: &Path,
    create: &dyn Fn() -> Result<crate::storage::read_cache::Snapshot, String>,
    force_copy: bool,
) -> Result<InspectionDatabase, String> {
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
    let snapshot = if force_copy || (wal && !shm) {
        Some(private_wal_snapshot(&owned_root, &files, create)?)
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
    let option = match (wal, shm, snapshot.is_some()) {
        (true, _, true) => "",
        (true, true, false) => "readonly_shm=1",
        (true, false, false) => "",
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
    create: &dyn Fn() -> Result<crate::storage::read_cache::Snapshot, String>,
) -> Result<crate::storage::read_cache::Snapshot, String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    verify_inspection_files(root, files)?;
    let bytes = files.iter().try_fold(0u64, |sum, (_, _, p)| {
        sum.checked_add(p.size)
            .ok_or("読取用コピーの容量が範囲外です")
    })?;
    let needed = bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(64 * 1024 * 1024))
        .ok_or("読取用コピーの容量が範囲外です")?;
    let snapshot = create()?;
    if crate::audio_import::available_bytes(snapshot.path())? < needed {
        return Err(
            "削除履歴の非破壊読取に必要な一時容量がありません。元のDB・WALを保持します".into(),
        );
    }
    for (name, source, proof) in files {
        if name == "meetings.sqlite3-shm" {
            continue;
        }
        if !matches!(name.as_str(), "meetings.sqlite3" | "meetings.sqlite3-wal") {
            return Err("読取用コピーへ未知の制御ファイルを採用しません".into());
        }
        root.verify_file(name, source)?;
        let mut input = source.try_clone().map_err(|e| e.to_string())?;
        input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut output = snapshot.create_file(name)?;
        #[cfg(test)]
        if name == "meetings.sqlite3" {
            read_copy_checkpoint("database_registered", snapshot.path());
        }
        let mut input = input.take(
            proof
                .size
                .checked_add(1)
                .ok_or("読取用コピーの容量が範囲外です")?,
        );
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            copied = copied
                .checked_add(count as u64)
                .ok_or("読取用コピーの容量が範囲外です")?;
            #[cfg(test)]
            if name == "meetings.sqlite3" {
                read_copy_checkpoint("database_chunk", snapshot.path());
            }
        }
        if copied != proof.size {
            return Err("読取用コピー中にDB・WALの容量が変更されています".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        verify_inspection_files(root, files)?;
    }
    #[cfg(test)]
    read_copy_checkpoint("wal_copied", snapshot.path());
    // SQLite may rebuild the WAL index, but only inside the owned copy. Record
    // its empty inode before SQLite can write any shared-memory payload.
    snapshot
        .create_file("meetings.sqlite3-shm")?
        .sync_all()
        .map_err(|e| e.to_string())?;
    #[cfg(test)]
    read_copy_checkpoint("index_registered", snapshot.path());
    verify_inspection_files(root, files)?;
    Ok(snapshot)
}

#[cfg(test)]
fn read_copy_checkpoint(label: &str, path: &Path) {
    if std::env::var("MON_SYNTHETIC_READ_COPY_CHECKPOINT")
        .ok()
        .as_deref()
        != Some(label)
    {
        return;
    }
    let marker = std::env::var_os("MON_SYNTHETIC_READ_COPY_READY").expect("owned ready marker");
    publish_read_copy_ready(Path::new(&marker), path, || {}).unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[cfg(test)]
fn publish_read_copy_ready(
    marker: &Path,
    path: &Path,
    before_publish: impl FnOnce(),
) -> std::io::Result<()> {
    use std::io::Write;
    let parent = marker
        .parent()
        .ok_or_else(|| std::io::Error::other("marker parent missing"))?;
    let mut ready = tempfile::Builder::new()
        .prefix(".read-ready-")
        .tempfile_in(parent)?;
    ready.write_all(path.as_os_str().as_encoded_bytes())?;
    ready.as_file().sync_all()?;
    before_publish();
    // A parent observing the marker must see the complete path, not the empty
    // create_new/write gap. Never overwrite even an unexpected fixture marker.
    ready
        .persist_noclobber(marker)
        .map_err(|error| error.error)?;
    std::fs::File::open(parent)?.sync_all()
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

    struct TestInspection {
        db: InspectionDatabase,
        _config: tempfile::TempDir,
    }
    impl std::ops::Deref for TestInspection {
        type Target = InspectionDatabase;
        fn deref(&self) -> &InspectionDatabase {
            &self.db
        }
    }
    impl TestInspection {
        fn finish(self) -> Result<tempfile::TempDir, String> {
            self.db.finish()?;
            Ok(self._config)
        }
    }
    fn inspect(anchor: &fs::RootAnchor, path: &Path) -> Result<TestInspection, String> {
        let config = tempfile::tempdir().map_err(|e| e.to_string())?;
        let parent = config.path().canonicalize().map_err(|e| e.to_string())?;
        let db = inspection_database(anchor, path, &|| {
            crate::storage::read_cache::Snapshot::create(&parent)
        })?;
        Ok(TestInspection {
            db,
            _config: config,
        })
    }

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
    fn deletion_startup_admission_requires_current_immutable_hash_and_rejects_old_shape() {
        use crate::{
            analysis_worker::AnalysisController, storage::deletion::operations, storage::Storage,
        };
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("SYNTHETIC CURRENT RECEIPT").unwrap();
        let mut deletion = operations::Prepared::fresh(
            &storage,
            &meeting.id,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        deletion
            .execute(
                &storage,
                &lease,
                &deletion.preview().confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .unwrap();
        drop(lease);
        let snapshot = storage
            .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
            .unwrap();
        let source = Source::capture(&storage, &snapshot, &crate::test_support::never_cancelled)
            .unwrap()
            .unwrap();
        let current: SourceFacts =
            serde_json::from_value(serde_json::to_value(&source).unwrap()).unwrap();
        drop(snapshot);
        let db = storage.connection.lock().unwrap();
        records(&db, &current).unwrap();
        let mut old_label = serde_json::to_value(&source).unwrap();
        old_label["records"][0]["validation"] = "legacyDatabaseOnly".into();
        let old_label: SourceFacts = serde_json::from_value(old_label).unwrap();
        assert!(!record_valid(&old_label.records[0], old_label.version));
        assert!(records(&db, &old_label).is_err());
        for version in [0, 1, 2, 3, 5] {
            let mut unsupported = serde_json::to_value(&source).unwrap();
            unsupported["version"] = version.into();
            let unsupported: SourceFacts = serde_json::from_value(unsupported).unwrap();
            assert!(records(&db, &unsupported).is_err());
        }
        for replacement in [None, Some(serde_json::Value::Null)] {
            let mut missing = serde_json::to_value(&source).unwrap();
            let record = missing["records"][0].as_object_mut().unwrap();
            if let Some(value) = replacement {
                record.insert("immutableHash".into(), value);
            } else {
                record.remove("immutableHash");
            }
            assert!(serde_json::from_value::<SourceFacts>(missing).is_err());
        }
        for hash in ["invalid".to_owned(), "0".repeat(64)] {
            let mut altered = serde_json::to_value(&source).unwrap();
            altered["records"][0]["immutableHash"] = hash.into();
            let altered: SourceFacts = serde_json::from_value(altered).unwrap();
            assert!(records(&db, &altered).is_err());
        }
        db.execute(
            "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
            [&current.records[0].operation_id],
        )
        .unwrap();
        records(&db, &current).unwrap();
        // The current DB also prevents consent mutation before the reader is
        // reached; do not disable its trigger to make a legacy fixture pass.
        assert!(db
            .execute(
                "UPDATE maintenance_operations SET consent_at=consent_at+1 WHERE id=?1",
                [&current.records[0].operation_id],
            )
            .is_err());
        records(&db, &current).unwrap();
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
        let read = inspect(&a, &path).unwrap();
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
            let read = inspect(&a, &path).unwrap();
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
            let _config = read.finish().unwrap();
            if let Some(temporary) = temporary {
                assert!(!temporary.exists());
            }
            a.verify().unwrap();
            assert_eq!(files(&path), before);
        }
    }

    #[test]
    fn read_cache_sigkill_during_owned_wal_copy_reopens_bootstrap_and_preserves_source_and_meeting()
    {
        use std::os::unix::process::ExitStatusExt;
        use std::time::{Duration, Instant};
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        for checkpoint in [
            "database_registered",
            "database_chunk",
            "wal_copied",
            "index_registered",
            "index_read",
        ] {
            let config_temp = tempfile::tempdir().unwrap();
            let legacy = tempfile::tempdir().unwrap();
            let config = config_temp.path().canonicalize().unwrap();
            let legacy_path = legacy.path().canonicalize().unwrap();
            let bootstrap = crate::bootstrap::Bootstrap::open(&config, &legacy_path).unwrap();
            let storage = bootstrap.open_storage().unwrap();
            let meeting = storage
                .create_meeting("SYNTHETIC RETAINED MEETING")
                .unwrap();
            let source = storage.root.clone();
            drop(storage);
            drop(bootstrap);
            let db = Connection::open(source.join("meetings.sqlite3")).unwrap();
            db.execute_batch("CREATE TABLE retained_payload(data BLOB); INSERT INTO retained_payload VALUES(zeroblob(196608));").unwrap();
            drop(db);
            crashed_wal(&source);
            std::fs::rename(
                source.join("meetings.sqlite3-shm"),
                source.join("retained.shm"),
            )
            .unwrap();
            let original = files(&source);
            let retained = std::fs::read(source.join("retained.shm")).unwrap();
            let ready = config.join("synthetic-read-ready");
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "storage::deletion::relocation::receiver::startup_admission::tests::read_cache_sigkill_copy_child", "--ignored", "--nocapture"])
                .env("MON_SYNTHETIC_READ_COPY_CONFIG", &config)
                .env("MON_SYNTHETIC_READ_COPY_LEGACY", &legacy_path)
                .env("MON_SYNTHETIC_READ_COPY_SOURCE", &source)
                .env("MON_SYNTHETIC_READ_COPY_READY", &ready)
                .env("MON_SYNTHETIC_READ_COPY_CHECKPOINT", checkpoint)
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit())
                .spawn().unwrap();
            let mut child = Child(child);
            let deadline = Instant::now() + Duration::from_secs(20);
            while !ready.exists() {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "child ended before {checkpoint}"
                );
                assert!(Instant::now() < deadline, "checkpoint timeout {checkpoint}");
                std::thread::sleep(Duration::from_millis(20));
            }
            let snapshot = PathBuf::from(std::fs::read_to_string(&ready).unwrap());
            assert_eq!(
                snapshot.parent(),
                Some(config.join(".mon-read-cache").as_path())
            );
            assert!(snapshot
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mon-stage-"));
            let copied = std::fs::metadata(snapshot.join("meetings.sqlite3"))
                .unwrap()
                .len();
            match checkpoint {
                "database_registered" => assert_eq!(copied, 0),
                "database_chunk" => {
                    assert_eq!(copied, 65536);
                    assert!(copied < original[0].as_ref().unwrap().len() as u64);
                }
                _ => assert_eq!(copied, original[0].as_ref().unwrap().len() as u64),
            }
            child.0.kill().unwrap();
            assert_eq!(child.0.wait().unwrap().signal(), Some(9));
            assert_eq!(
                files(&source),
                original,
                "inspection must not write source at {checkpoint}"
            );
            assert_eq!(
                std::fs::read(source.join("retained.shm")).unwrap(),
                retained
            );
            let bootstrap = crate::bootstrap::Bootstrap::open(&config, &legacy_path).unwrap();
            let storage = bootstrap.open_storage().unwrap();
            assert!(
                !snapshot.exists(),
                "owned abandoned copy survived {checkpoint}"
            );
            assert_eq!(
                storage.get_meeting(&meeting.id).unwrap().unwrap().title,
                "SYNTHETIC RETAINED MEETING"
            );
            assert_eq!(
                std::fs::read(source.join("retained.shm")).unwrap(),
                retained
            );
            let db = storage.connection.lock().unwrap();
            assert_eq!(
                db.query_row("SELECT value FROM synthetic", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "committed in WAL only"
            );
            assert_eq!(
                db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        }
    }

    #[test]
    fn read_copy_ready_marker_is_complete_at_publication_and_never_overwrites_existing_material() {
        let base = tempfile::tempdir().unwrap();
        let marker = base.path().join("ready");
        let snapshot = base.path().join(".mon-read-cache/.mon-stage-synthetic");
        publish_read_copy_ready(&marker, &snapshot, || {
            assert!(
                !marker.exists(),
                "ready must not appear before the path is fully written"
            );
        })
        .unwrap();
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            snapshot.as_os_str().as_encoded_bytes()
        );
        let other = base.path().join("different-snapshot");
        assert!(publish_read_copy_ready(&marker, &other, || {}).is_err());
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            snapshot.as_os_str().as_encoded_bytes()
        );
        assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "child process only; owned synthetic read-copy checkpoint required"]
    fn read_cache_sigkill_copy_child() {
        let env_path = |name| PathBuf::from(std::env::var_os(name).expect("owned synthetic path"));
        let config = env_path("MON_SYNTHETIC_READ_COPY_CONFIG");
        let legacy = env_path("MON_SYNTHETIC_READ_COPY_LEGACY");
        let source = env_path("MON_SYNTHETIC_READ_COPY_SOURCE");
        let bootstrap = crate::bootstrap::Bootstrap::open(&config, &legacy).unwrap();
        bootstrap.view().unwrap();
        let a = anchor(&source);
        let read = inspection_database(&a, &source, &|| {
            crate::storage::read_cache::Snapshot::create(&config)
        })
        .unwrap();
        assert_eq!(
            read.query_row("SELECT value FROM synthetic", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "committed in WAL only"
        );
        read_copy_checkpoint("index_read", read.snapshot.as_ref().unwrap().path());
        panic!("requested checkpoint did not hold the child");
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
            assert!(inspect(&a, &path).is_err(), "{kind}");
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
        assert!(inspect(&a, &path).is_err());
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
                let read = inspect(&a, &path).unwrap();
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
