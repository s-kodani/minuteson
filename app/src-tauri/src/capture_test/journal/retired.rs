//! Rotate only fully retired journals after a device-number-only change.
//! Keep the old database, certificate and leases unchanged as retained history.
use super::*;
use std::collections::HashSet;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

fn rebound(old: &Identity, actual: &Identity, old_device: u64, device: u64) -> bool {
    old.device == old_device
        && actual.device == device
        && old.inode == actual.inode
        && old.birth_seconds == actual.birth_seconds
        && old.birth_nanos == actual.birth_nanos
}

pub(super) fn archive(
    parent: &Path,
    pin: &File,
    actual: &RootProof,
    saved: &RootProof,
) -> Result<(), String> {
    let refuse =
        || "テスト所有証明と台帳の実体が一致しません。既存データを保持しています".to_string();
    let old_device = saved.directory.device;
    let device = actual.directory.device;
    if saved.schema_version != 1
        || actual.schema_version != 1
        || saved.path != actual.path
        || old_device == device
        || ![
            (&saved.directory, &actual.directory),
            (&saved.database, &actual.database),
            (&saved.lock, &actual.lock),
            (&saved.certificate, &actual.certificate),
        ]
        .into_iter()
        .all(|(old, now)| rebound(old, now, old_device, device))
    {
        return Err(refuse());
    }
    // Refuse SQLite recovery state before opening a connection. Even a read
    // of a WAL database can require creating shared-memory control files.
    for name in [
        "ownership.sqlite3-wal",
        "ownership.sqlite3-shm",
        "ownership.sqlite3-journal",
    ] {
        match std::fs::symlink_metadata(actual.path.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
            Ok(_) => return Err(refuse()),
        }
    }
    let database = open_at(pin, "ownership.sqlite3", false, DATABASE_LIMIT)?;
    let mut format = [0; 2];
    std::os::unix::fs::FileExt::read_exact_at(&database, &mut format, 18)
        .map_err(|e| e.to_string())?;
    if format != [1, 1] {
        return Err(refuse());
    }
    let db = Connection::open_with_flags(
        actual.path.join("ownership.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    if schema(&db)? != schema(&expected)? {
        return Err(refuse());
    }
    let (proof, digest): (String, String) = db
        .query_row("SELECT proof,hash FROM metadata WHERE id=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    if decode::<RootProof>(&proof, &digest)? != *saved {
        return Err(refuse());
    }
    let mut statement = db
        .prepare("SELECT id,proof,hash,state FROM runs ORDER BY id LIMIT 4097")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if rows.len() > 4096 {
        return Err(refuse());
    }
    let mut names: HashSet<String> = ["owner.lock", "ownership.sqlite3", "root-proof.json"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut leases = Vec::new();
    for (id, proof, digest, state) in rows {
        let record: Pending = decode(&proof, &digest)?;
        validate_request(&record.request)?;
        if state != "removed"
            || record.schema_version != 1
            || record.request.run_id != id
            || record.directory.device != old_device
        {
            return Err(refuse());
        }
        let name = lease_name(&id);
        let lease = open_at(pin, &name, false, 0)?;
        let identity = Identity::read(&regular(&lease, 0)?)?;
        if !rebound(&record.lease, &identity, old_device, device) {
            return Err(refuse());
        }
        // Even a retired row cannot bypass a live inherited writer lease.
        lock(&lease)?;
        leases.push(lease);
        names.insert(name);
    }
    // Pending/unknown payloads and SQLite recovery files block rotation.
    let entries = std::fs::read_dir(&actual.path)
        .map_err(|e| e.to_string())?
        .map(|entry| {
            entry
                .map_err(|e| e.to_string())?
                .file_name()
                .into_string()
                .map_err(|_| refuse())
        })
        .collect::<Result<HashSet<_>, _>>()?;
    if entries != names {
        return Err(refuse());
    }
    drop(statement);
    drop(db);
    for (name, identity, limit) in [
        ("ownership.sqlite3", &actual.database, DATABASE_LIMIT),
        ("owner.lock", &actual.lock, 0),
        ("root-proof.json", &actual.certificate, RECORD_LIMIT as u64),
    ] {
        if !identity.matches(&regular(&open_at(pin, name, false, limit)?, limit)?) {
            return Err(refuse());
        }
    }
    if certificate(&open_at(
        pin,
        "root-proof.json",
        false,
        RECORD_LIMIT as u64,
    )?)? != *saved
        || !actual.directory.matches(&directory(&actual.path)?)
        || !actual
            .directory
            .matches(&pin.metadata().map_err(|e| e.to_string())?)
    {
        return Err(refuse());
    }
    let parent_pin = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)
        .map_err(|e| e.to_string())?;
    let archive_name = format!(".mon-capture-tests-retained-{}", uuid::Uuid::new_v4());
    let source = CString::new(actual.path.file_name().ok_or_else(refuse)?.as_bytes())
        .map_err(|_| refuse())?;
    let destination = CString::new(archive_name.as_bytes()).map_err(|_| refuse())?;
    if unsafe {
        libc::renameatx_np(
            parent_pin.as_raw_fd(),
            source.as_ptr(),
            parent_pin.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if !actual
        .directory
        .matches(&directory(&parent.join(&archive_name))?)
    {
        return Err(refuse());
    }
    parent_pin.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}
