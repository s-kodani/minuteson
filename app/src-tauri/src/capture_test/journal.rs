//! Durable ownership for one private test at a time. Never discover deletion
//! targets by scanning temporary filenames. A helper inherits only its run's
//! flock descriptor; parent death alone cannot authorize audio collection.
use super::Request;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

mod retired;
#[cfg(test)]
mod retired_tests;
#[cfg(test)]
mod tests;

const SCHEMA: &str = "
CREATE TABLE metadata(id INTEGER PRIMARY KEY CHECK(id=1),proof TEXT NOT NULL,hash TEXT NOT NULL);
CREATE TABLE runs(id TEXT PRIMARY KEY,proof TEXT NOT NULL,hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('allocated','removed')));
CREATE TRIGGER metadata_fixed BEFORE UPDATE ON metadata BEGIN SELECT RAISE(ABORT,'capture test root is immutable'); END;
CREATE TRIGGER metadata_retained BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'capture test root is retained'); END;
CREATE TRIGGER runs_fixed BEFORE UPDATE ON runs WHEN NEW.id!=OLD.id OR NEW.proof!=OLD.proof OR NEW.hash!=OLD.hash OR (OLD.state='removed' AND NEW.state!='removed') BEGIN SELECT RAISE(ABORT,'capture test ownership is immutable'); END;
CREATE TRIGGER runs_retained BEFORE DELETE ON runs BEGIN SELECT RAISE(ABORT,'capture test ownership is retained'); END;
";
const RECORD_LIMIT: usize = 16 * 1024;
const DATABASE_LIMIT: u64 = 16 * 1024 * 1024;
type Schema = Vec<(String, String, String)>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    birth_seconds: u64,
    birth_nanos: u32,
}
impl Identity {
    fn read(metadata: &Metadata) -> Result<Self, String> {
        let birth = metadata
            .created()
            .map_err(|e| e.to_string())?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_seconds: birth.as_secs(),
            birth_nanos: birth.subsec_nanos(),
        })
    }
    fn matches(&self, metadata: &Metadata) -> bool {
        Self::read(metadata).is_ok_and(|identity| identity == *self)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RootProof {
    schema_version: u8,
    path: PathBuf,
    directory: Identity,
    database: Identity,
    lock: Identity,
    certificate: Identity,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    schema_version: u8,
    pub(super) request: Request,
    directory: Identity,
    lease: Identity,
}
pub(super) struct Registry {
    root: PathBuf,
    pin: File,
    proof: RootProof,
    expected_schema: Schema,
    db: Mutex<Option<Connection>>,
    // Keep the registry lock until the SQLite connection is closed. It is
    // CLOEXEC, so the capture helper does not inherit this global owner.
    _lock: File,
}
impl Drop for Registry {
    fn drop(&mut self) {
        // A concurrent fork can temporarily retain a CLOEXEC duplicate of the
        // global description. Close SQLite first, then release this registry
        // lock explicitly. Unlike a run's capture lease it is never delegated
        // to a live writer, and must end with this registry's lifetime.
        let db = self.db.get_mut().unwrap_or_else(|error| error.into_inner());
        drop(db.take());
        unsafe {
            libc::flock(self._lock.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub(super) struct Area {
    registry: Arc<Registry>,
    record: Pending,
    // Closing, not LOCK_UN, relinquishes the parent's description: LOCK_UN
    // would incorrectly release a still-live helper's inherited flock too.
    lease: Mutex<Option<File>>,
}
pub(super) struct Quiet {
    registry: Arc<Registry>,
    run_id: String,
    lease: File,
}
impl Drop for Quiet {
    fn drop(&mut self) {
        // This fresh description was acquired only after all capture-writer
        // descriptions released their lock. Quiet is never delegated to a
        // helper. A concurrent pre-exec copy must not prolong collection
        // ownership after this guard ends. Do not unlock Area's writer lease.
        unsafe {
            libc::flock(self.lease.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn private(metadata: &Metadata) -> bool {
    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
}
fn directory(path: &Path) -> Result<Metadata, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || !private(&metadata)
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("テスト所有領域の実体・権限が変更されています".into());
    }
    Ok(metadata)
}
fn regular(file: &File, maximum: u64) -> Result<Metadata, String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || !private(&metadata)
        || metadata.nlink() != 1
        || metadata.len() > maximum
    {
        return Err("テスト所有ファイルの形式・権限・容量が不正です".into());
    }
    Ok(metadata)
}
fn open_at(pin: &File, name: &str, create: bool, maximum: u64) -> Result<File, String> {
    let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    let fd = unsafe {
        libc::openat(
            pin.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if create {
                    libc::O_CREAT | libc::O_EXCL
                } else {
                    0
                },
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    regular(&file, maximum)?;
    Ok(file)
}
fn lock(file: &File) -> Result<(), String> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!(
            "テストの所有または取得プロセスの終了が未確認です: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn encode<T: Serialize>(value: &T) -> Result<(String, String), String> {
    let proof = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if proof.len() > RECORD_LIMIT {
        return Err("テスト所有記録が大きすぎます".into());
    }
    let digest = hash(&proof);
    Ok((proof, digest))
}
fn decode<T: for<'de> Deserialize<'de>>(proof: &str, digest: &str) -> Result<T, String> {
    if proof.len() > RECORD_LIMIT || hash(proof) != digest {
        return Err("テスト所有記録の内容が一致しません".into());
    }
    serde_json::from_str(proof).map_err(|e| e.to_string())
}
fn schema(db: &Connection) -> Result<Schema, String> {
    db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name")
        .map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())
}
fn certificate(file: &File) -> Result<RootProof, String> {
    regular(file, RECORD_LIMIT as u64)?;
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > RECORD_LIMIT {
        return Err("テスト所有証明が大きすぎます".into());
    }
    let (proof, digest): (String, String) =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    decode(&proof, &digest)
}
fn same_certificate(pin: &File, proof: &RootProof) -> Result<bool, String> {
    let file = open_at(pin, "root-proof.json", false, RECORD_LIMIT as u64)?;
    Ok(proof
        .certificate
        .matches(&regular(&file, RECORD_LIMIT as u64)?)
        && certificate(&file)? == *proof)
}
fn validate_request(request: &Request) -> Result<(), String> {
    request.validate()?;
    if request.expected_import.revision > 9_007_199_254_740_991
        || request
            .expected_import
            .token
            .as_ref()
            .is_some_and(|id| !crate::input_levels::valid_owner(id))
    {
        return Err("テスト所有記録の取り込み条件が不正です".into());
    }
    Ok(())
}
fn payload_name(id: &str) -> String {
    format!("audio-{id}")
}
fn lease_name(id: &str) -> String {
    format!("lease-{id}")
}
impl Registry {
    pub(super) fn open(app_data: &Path) -> Result<Arc<Self>, String> {
        let parent = app_data.canonicalize().map_err(|e| e.to_string())?;
        let root = parent.join(".mon-capture-tests");
        let new = match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(error.to_string()),
        };
        let identity = Identity::read(&directory(&root)?)?;
        let pin = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root)
            .map_err(|e| e.to_string())?;
        if !identity.matches(&pin.metadata().map_err(|e| e.to_string())?) {
            return Err("テスト所有領域の固定に失敗しました".into());
        }
        let owner = open_at(&pin, "owner.lock", new, 0)?;
        lock(&owner)?;
        let file = open_at(&pin, "ownership.sqlite3", new, DATABASE_LIMIT)?;
        let mut certificate_file = open_at(&pin, "root-proof.json", new, RECORD_LIMIT as u64)?;
        let actual = RootProof {
            schema_version: 1,
            path: root.clone(),
            directory: identity,
            database: Identity::read(&file.metadata().map_err(|e| e.to_string())?)?,
            lock: Identity::read(&owner.metadata().map_err(|e| e.to_string())?)?,
            certificate: Identity::read(&certificate_file.metadata().map_err(|e| e.to_string())?)?,
        };
        if new {
            let proof = serde_json::to_vec(&encode(&actual)?).map_err(|e| e.to_string())?;
            if proof.len() > RECORD_LIMIT {
                return Err("テスト所有証明が大きすぎます".into());
            }
            certificate_file
                .write_all(&proof)
                .and_then(|_| certificate_file.sync_all())
                .map_err(|e| e.to_string())?;
            pin.sync_all().map_err(|e| e.to_string())?;
        } else {
            let saved = certificate(&certificate_file)?;
            if saved != actual {
                // Completed histories are retained verbatim. A changed device
                // number never authorizes collecting old audio or rewriting
                // its ownership proofs into authority over new objects.
                retired::archive(&parent, &pin, &actual, &saved)?;
                return Self::open(app_data);
            }
        }
        for name in ["ownership.sqlite3-wal", "ownership.sqlite3-shm"] {
            match std::fs::symlink_metadata(root.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => return Err("テスト所有台帳に未対応の共有ファイルがあります".into()),
            }
        }
        match std::fs::symlink_metadata(root.join("ownership.sqlite3-journal")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
            Ok(_) => {
                open_at(&pin, "ownership.sqlite3-journal", false, DATABASE_LIMIT)?;
            }
        }
        let db = Connection::open_with_flags(
            root.join("ownership.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if mode != "delete" {
            return Err("テスト所有台帳の保存方式が変更されています".into());
        }
        db.execute_batch("PRAGMA synchronous=FULL;")
            .map_err(|e| e.to_string())?;
        if new {
            let (proof, digest) = encode(&actual)?;
            db.execute_batch(&format!("BEGIN IMMEDIATE; {SCHEMA}"))
                .map_err(|e| e.to_string())?;
            db.execute(
                "INSERT INTO metadata VALUES(1,?1,?2)",
                params![proof, digest],
            )
            .map_err(|e| e.to_string())?;
            db.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            pin.sync_all().map_err(|e| e.to_string())?;
            File::open(&parent)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
        expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let expected_schema = schema(&expected)?;
        let registry = Arc::new(Self {
            root,
            pin,
            proof: actual,
            expected_schema,
            db: Mutex::new(Some(db)),
            _lock: owner,
        });
        registry.with_db(|_| Ok(()))?;
        Ok(registry)
    }
    fn check_files(&self) -> Result<(), String> {
        if !self.proof.directory.matches(&directory(&self.root)?)
            || !self
                .proof
                .directory
                .matches(&self.pin.metadata().map_err(|e| e.to_string())?)
            || !self.proof.database.matches(&regular(
                &open_at(&self.pin, "ownership.sqlite3", false, DATABASE_LIMIT)?,
                DATABASE_LIMIT,
            )?)
            || !self
                .proof
                .lock
                .matches(&regular(&open_at(&self.pin, "owner.lock", false, 0)?, 0)?)
            || !same_certificate(&self.pin, &self.proof)?
        {
            return Err("テスト所有台帳の保存先が変更されています".into());
        }
        Ok(())
    }
    fn with_db<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        self.check_files()?;
        let held = self.db.lock().map_err(|_| "テスト所有台帳のロック失敗")?;
        let db = held.as_ref().ok_or("テスト所有台帳は終了しています")?;
        if schema(&db)? != self.expected_schema {
            return Err("テスト所有台帳の形式が変更されています".into());
        }
        let (proof, digest): (String, String) = db
            .query_row("SELECT proof,hash FROM metadata WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        if decode::<RootProof>(&proof, &digest)? != self.proof {
            return Err("テスト所有台帳の実体が一致しません".into());
        }
        let result = action(&db)?;
        self.check_files()?;
        Ok(result)
    }
    pub(super) fn allocate(self: &Arc<Self>, request: Request) -> Result<Area, String> {
        validate_request(&request)?;
        self.with_db(|db| {
            let existing: i64 = db
                .query_row(
                    "SELECT count(*) FROM runs WHERE id=?1 OR state='allocated'",
                    [&request.run_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let count: i64 = db
                .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if existing != 0 || count >= 4096 {
                return Err("以前のテストの所有が残っているか、開始IDは受付済みです".into());
            }
            let lease = open_at(&self.pin, &lease_name(&request.run_id), true, 0)?;
            lock(&lease)?;
            let path = self.root.join(payload_name(&request.run_id));
            // No capture can start before this durable insert. A creation
            // interrupted before the insert may leave empty, unregistered
            // nodes; never adopt or erase them by filename on the next start.
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|e| e.to_string())?;
            let record = Pending {
                schema_version: 1,
                directory: Identity::read(&directory(&path)?)?,
                lease: Identity::read(&regular(&lease, 0)?)?,
                request,
            };
            File::open(&path)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
            lease.sync_all().map_err(|e| e.to_string())?;
            self.pin.sync_all().map_err(|e| e.to_string())?;
            let (proof, digest) = encode(&record)?;
            db.execute(
                "INSERT INTO runs VALUES(?1,?2,?3,'allocated')",
                params![record.request.run_id, proof, digest],
            )
            .map_err(|e| e.to_string())?;
            Ok(Area {
                registry: self.clone(),
                record,
                lease: Mutex::new(Some(lease)),
            })
        })
    }
    pub(super) fn pending(&self) -> Result<Vec<Pending>, String> {
        self.with_db(|db| {
            let rows = db
                .prepare(
                    "SELECT id,proof,hash FROM runs WHERE state='allocated' ORDER BY id LIMIT 2",
                )
                .map_err(|e| e.to_string())?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if rows.len() > 1 {
                return Err("複数の未完テスト記録があります。自動回収していません".into());
            }
            rows.into_iter()
                .map(|(id, proof, digest)| {
                    let record: Pending = decode(&proof, &digest)?;
                    validate_request(&record.request)?;
                    if record.schema_version != 1 || record.request.run_id != id {
                        return Err("テスト所有記録のID・版が不正です".into());
                    }
                    Ok(record)
                })
                .collect()
        })
    }
    pub(super) fn restore(self: &Arc<Self>, record: Pending) -> Result<Area, String> {
        let area = Area {
            registry: self.clone(),
            record,
            lease: Mutex::new(None),
        };
        area.check_record()?;
        Ok(area)
    }
    pub(super) fn collect(self: &Arc<Self>, record: Pending) -> Result<(), String> {
        super::files::TestDirectory::recover_owned(self.restore(record)?)
    }
}
impl Area {
    pub(super) fn release_parent_lease(&self) {
        // Never LOCK_UN: a live helper or a pre-exec child can still retain
        // the shared description. Future forks need not inherit this copy.
        self.lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
    pub(super) fn pin_directory(&self) -> Result<File, String> {
        self.check_record()?;
        self.check_payload()?;
        let pin = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.path())
            .map_err(|e| e.to_string())?;
        let metadata = pin.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() || !private(&metadata) || !self.record.directory.matches(&metadata) {
            return Err("テスト音声の所有実体を固定できません".into());
        }
        self.check_payload()?;
        self.registry.check_files()?;
        Ok(pin)
    }
    pub(super) fn validate_directory(&self) -> Result<(), String> {
        self.check_record()?;
        self.check_payload()?;
        self.lease_file()?;
        Ok(())
    }
    pub(super) fn path(&self) -> PathBuf {
        self.registry
            .root
            .join(payload_name(&self.record.request.run_id))
    }
    fn check_record(&self) -> Result<(), String> {
        self.registry.with_db(|db| {
            let row: Option<(String, String, String)> = db
                .query_row(
                    "SELECT proof,hash,state FROM runs WHERE id=?1",
                    [&self.record.request.run_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let (proof, digest, state) = row.ok_or("テスト所有記録がありません")?;
            if state != "allocated"
                || encode(&self.record)?.0 != encode(&decode::<Pending>(&proof, &digest)?)?.0
            {
                return Err("テスト所有記録が変更されています".into());
            }
            Ok(())
        })
    }
    fn check_payload(&self) -> Result<(), String> {
        match std::fs::symlink_metadata(self.path()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
            Ok(_) => {
                if self.record.directory.matches(&directory(&self.path())?) {
                    Ok(())
                } else {
                    Err("テスト音声の保存先が差し替えられています".into())
                }
            }
        }
    }
    fn lease_file(&self) -> Result<File, String> {
        let file = open_at(
            &self.registry.pin,
            &lease_name(&self.record.request.run_id),
            false,
            0,
        )?;
        if !self.record.lease.matches(&regular(&file, 0)?) {
            return Err("テスト取得の所有ファイルが変更されています".into());
        }
        Ok(file)
    }
    // A restored allocation cannot spawn capture. Only a new allocation owns
    // the original locked description, and this copy survives exec explicitly.
    pub(super) fn inherit(&self, command: &mut Command) -> Result<i32, String> {
        self.check_record()?;
        self.check_payload()?;
        if !self.path().try_exists().map_err(|e| e.to_string())? {
            return Err("テスト音声の保存先がありません".into());
        }
        self.lease_file()?;
        let held = self
            .lease
            .lock()
            .map_err(|_| "テスト取得所有のロック失敗")?;
        let retained = held
            .as_ref()
            .ok_or("回復したテストを再録音することはできません")?
            .try_clone()
            .map_err(|e| e.to_string())?;
        let fd = retained.as_raw_fd();
        unsafe {
            command.pre_exec(move || {
                let fd = retained.as_raw_fd();
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(fd)
    }
    pub(super) fn quiet(&self) -> Result<Quiet, String> {
        self.check_record()?;
        self.check_payload()?;
        // Never explicitly unlock the description shared with the helper.
        self.release_parent_lease();
        let file = self.lease_file()?;
        lock(&file)?;
        let quiet = Quiet {
            registry: self.registry.clone(),
            run_id: self.record.request.run_id.clone(),
            lease: file,
        };
        // If either post-lock identity check fails, the same non-delegated
        // description must be released by Quiet, not just close one copy.
        self.check_payload()?;
        self.check_record()?;
        Ok(quiet)
    }
    pub(super) fn finish(&self, quiet: &Quiet) -> Result<(), String> {
        if !Arc::ptr_eq(&self.registry, &quiet.registry)
            || quiet.run_id != self.record.request.run_id
            || !self.record.lease.matches(&regular(&quiet.lease, 0)?)
        {
            return Err("回収したテストの所有が一致しません".into());
        }
        self.check_record()?;
        self.check_payload()?;
        self.lease_file()?;
        match std::fs::symlink_metadata(self.path()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
            Ok(_) => return Err("テスト音声の回収が終わっていません".into()),
        }
        self.registry.pin.sync_all().map_err(|e| e.to_string())?;
        self.registry.with_db(|db| {
            let changed = db
                .execute(
                    "UPDATE runs SET state='removed' WHERE id=?1 AND state='allocated'",
                    [&self.record.request.run_id],
                )
                .map_err(|e| e.to_string())?;
            if changed != 1 {
                return Err("回収対象のテスト所有が変更されています".into());
            }
            Ok(())
        })
    }
}
