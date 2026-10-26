//! Windows ownership is local v1 evidence. A deny-sharing file description is
//! inherited as the helper's stdin; collection cannot open it until every
//! writer handle closes, including after the parent process exits.
use super::Request;
use crate::{native_fs as fs, windows_files as win};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
const SCHEMA:&str="CREATE TABLE metadata(id INTEGER PRIMARY KEY CHECK(id=1),proof TEXT NOT NULL,hash TEXT NOT NULL); CREATE TABLE runs(id TEXT PRIMARY KEY,proof TEXT NOT NULL,hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('allocated','removed'))); CREATE TRIGGER metadata_fixed BEFORE UPDATE ON metadata BEGIN SELECT RAISE(ABORT,'immutable root'); END; CREATE TRIGGER metadata_retained BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'retained root'); END; CREATE TRIGGER runs_fixed BEFORE UPDATE ON runs WHEN NEW.id!=OLD.id OR NEW.proof!=OLD.proof OR NEW.hash!=OLD.hash OR (OLD.state='removed' AND NEW.state!='removed') BEGIN SELECT RAISE(ABORT,'immutable ownership'); END; CREATE TRIGGER runs_retained BEFORE DELETE ON runs BEGIN SELECT RAISE(ABORT,'retained ownership'); END;";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn encode<T: Serialize>(value: &T) -> Result<(String, String), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if text.len() > 16384 {
        return Err("テスト所有記録が上限を超えています".into());
    }
    let digest = hash(text.as_bytes());
    Ok((text, digest))
}
fn decode<T: for<'a> Deserialize<'a>>(text: &str, digest: &str) -> Result<T, String> {
    if text.len() > 16384 || hash(text.as_bytes()) != digest {
        return Err("テスト所有記録のSHA・容量が不正です".into());
    }
    serde_json::from_str(text).map_err(|e| e.to_string())
}
fn private(file: &File, directory: bool) -> Result<win::Identity, String> {
    let m = fs::metadata(file).map_err(|e| e.to_string())?;
    if m.is_dir() != directory
        || m.uid() != win::current_owner()?
        || m.mode() & 0o077 != 0
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err("テスト所有対象の実体・権限が不正です".into());
    }
    win::Identity::file(file)
}
fn open(path: &Path, new: bool, exclusive: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(new)
        .share_mode(if exclusive { 0 } else { 7 })
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|e| {
            format!(
                "テスト取得の所有を確認できません。取得中または共有ロック中の音声を保持します: {e}"
            )
        })?;
    // New files inherit the already verified private directory DACL. A second
    // open would conflict with the writer's intentional deny-sharing lease.
    private(&file, false)?;
    Ok(file)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RootProof {
    schema_version: u8,
    path: PathBuf,
    directory: win::Identity,
    database: win::Identity,
    lock: win::Identity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    schema_version: u8,
    pub(super) request: Request,
    directory: win::Identity,
    lease: win::Identity,
}
pub(super) struct Registry {
    root: PathBuf,
    pin: File,
    proof: RootProof,
    db: Mutex<Connection>,
    _lock: File,
}
pub(super) struct Area {
    registry: Arc<Registry>,
    record: Pending,
    lease: Mutex<Option<File>>,
}
pub(super) struct Quiet {
    registry: Arc<Registry>,
    run: String,
    lease: File,
}
impl Registry {
    pub(super) fn open(app_data: &Path) -> Result<Arc<Self>, String> {
        let root = app_data.join(".mon-capture-tests-windows-v1");
        let fresh = match std::fs::create_dir(&root) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.to_string()),
        };
        if fresh {
            win::set_private(&root)?;
        }
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        win::local_ntfs(&root)?;
        let pin = win::directory(&root)?;
        let directory = private(&pin, true)?;
        let lock = open(&root.join("registry.lock"), fresh, true)?;
        let database = root.join("ownership.sqlite3");
        if fresh {
            let file = open(&database, true, false)?;
            file.sync_all().map_err(|e| e.to_string())?;
        }
        let observed = fs::symlink_metadata(&database).map_err(|e| e.to_string())?;
        if observed.len() > 16 * 1024 * 1024 {
            return Err("テスト所有DBが上限を超えています".into());
        }
        let proof = RootProof {
            schema_version: 1,
            path: root.clone(),
            directory,
            database: private(&win::open_metadata(&database)?, false)?,
            lock: win::Identity::file(&lock)?,
        };
        let db = Connection::open(&database).map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
        )
        .map_err(|e| e.to_string())?;
        if fresh {
            db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            let (text, digest) = encode(&proof)?;
            db.execute(
                "INSERT INTO metadata VALUES(1,?1,?2)",
                params![text, digest],
            )
            .map_err(|e| e.to_string())?;
        }
        let (text, digest): (String, String) = db
            .query_row("SELECT proof,hash FROM metadata WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        if decode::<RootProof>(&text, &digest)? != proof {
            return Err("テスト所有DB・親・排他ファイルが変更されています".into());
        }
        // Reject modified schemas before accepting an executable SQLite trigger.
        let reference = Connection::open_in_memory().map_err(|e| e.to_string())?;
        reference.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
            db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type,name").map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())
        }
        if schema(&db)? != schema(&reference)? {
            return Err("テスト所有DBのスキーマが変更されています".into());
        }
        Ok(Arc::new(Self {
            root,
            pin,
            proof,
            db: Mutex::new(db),
            _lock: lock,
        }))
    }
    fn verify(&self) -> Result<(), String> {
        if private(&self.pin, true)? != self.proof.directory
            || win::Identity::at(&self.root)? != self.proof.directory
            || win::Identity::at(&self.root.join("ownership.sqlite3"))? != self.proof.database
            || win::Identity::file(&self._lock)? != self.proof.lock
        {
            return Err("テスト所有台帳が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn allocate(self: &Arc<Self>, request: Request) -> Result<Area, String> {
        request.validate()?;
        self.verify()?;
        let db = self.db.lock().map_err(|_| "テスト台帳の排他失敗")?;
        let blocked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 OR state='allocated') OR (SELECT count(*) FROM runs)>=4096",[&request.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if blocked {
            return Err("以前のテストの所有が残っているか、開始IDは受付済みです".into());
        }
        let lease = open(
            &self.root.join(format!("lease-{}", request.run_id)),
            true,
            true,
        )?;
        let path = self.root.join(format!("audio-{}", request.run_id));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        win::set_private(&path)?;
        let record = Pending {
            schema_version: 1,
            directory: win::Identity::at(&path)?,
            lease: win::Identity::file(&lease)?,
            request,
        };
        let (text, digest) = encode(&record)?;
        db.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,'allocated')",
            params![record.request.run_id, text, digest],
        )
        .map_err(|e| e.to_string())?;
        Ok(Area {
            registry: self.clone(),
            record,
            lease: Mutex::new(Some(lease)),
        })
    }
    pub(super) fn pending(&self) -> Result<Vec<Pending>, String> {
        self.verify()?;
        let db = self.db.lock().map_err(|_| "テスト台帳の排他失敗")?;
        let mut query = db
            .prepare("SELECT id,proof,hash FROM runs WHERE state='allocated' LIMIT 2")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let text: String = row.get(1).map_err(|e| e.to_string())?;
            let digest: String = row.get(2).map_err(|e| e.to_string())?;
            let record: Pending = decode(&text, &digest)?;
            record.request.validate()?;
            if record.schema_version != 1 || record.request.run_id != id {
                return Err("テスト所有のID・版が不正です".into());
            }
            result.push(record);
        }
        if result.len() > 1 {
            return Err("複数の未完テストを保持しています".into());
        }
        Ok(result)
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
    pub(super) fn path(&self) -> PathBuf {
        self.registry
            .root
            .join(format!("audio-{}", self.record.request.run_id))
    }
    fn lease_path(&self) -> PathBuf {
        self.registry
            .root
            .join(format!("lease-{}", self.record.request.run_id))
    }
    fn check_record(&self) -> Result<(), String> {
        self.registry.verify()?;
        let db = self
            .registry
            .db
            .lock()
            .map_err(|_| "テスト台帳の排他失敗")?;
        let row: Option<(String, String, String)> = db
            .query_row(
                "SELECT proof,hash,state FROM runs WHERE id=?1",
                [&self.record.request.run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (text, digest, state) = row.ok_or("テスト所有がありません")?;
        if state != "allocated"
            || encode(&decode::<Pending>(&text, &digest)?)?.0 != encode(&self.record)?.0
        {
            return Err("テスト所有が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn pin_directory(&self) -> Result<File, String> {
        self.validate_directory()?;
        win::open_metadata(&self.path())
    }
    pub(super) fn validate_directory(&self) -> Result<(), String> {
        self.check_record()?;
        if win::Identity::at(&self.path())? != self.record.directory {
            return Err("テスト音声の実体が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn release_parent_lease(&self) {
        self.lease.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
    pub(super) fn inherit(&self, command: &mut Command) -> Result<i32, String> {
        self.validate_directory()?;
        let guard = self.lease.lock().map_err(|_| "テスト取得所有の排他失敗")?;
        let lease = guard.as_ref().ok_or("回復したテストを再録音できません")?;
        if win::Identity::file(lease)? != self.record.lease {
            return Err("テスト取得所有が変更されています".into());
        }
        command.stdin(Stdio::from(lease.try_clone().map_err(|e| e.to_string())?));
        Ok(0)
    }
    pub(super) fn quiet(&self) -> Result<Quiet, String> {
        self.check_record()?;
        self.release_parent_lease();
        let lease = open(&self.lease_path(), false, true)?;
        if win::Identity::file(&lease)? != self.record.lease {
            return Err("テスト取得の所有が変更されています".into());
        }
        Ok(Quiet {
            registry: self.registry.clone(),
            run: self.record.request.run_id.clone(),
            lease,
        })
    }
    pub(super) fn finish(&self, quiet: &Quiet) -> Result<(), String> {
        self.check_record()?;
        if !Arc::ptr_eq(&self.registry, &quiet.registry)
            || quiet.run != self.record.request.run_id
            || win::Identity::file(&quiet.lease)? != self.record.lease
        {
            return Err("回収対象の所有が一致しません".into());
        }
        match fs::symlink_metadata(self.path()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
            Ok(_) => return Err("テスト音声の回収が完了していません".into()),
        };
        self.registry
            .db
            .lock()
            .map_err(|_| "テスト台帳の排他失敗")?
            .execute(
                "UPDATE runs SET state='removed' WHERE id=?1 AND state='allocated'",
                [&quiet.run],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_owned_five_second_helper_completes_with_inherited_lease() {
        owned_synthetic_case("mon-five-second-test");
    }
    #[test]
    fn windows_owned_pc_only_test_keeps_system_audio_without_a_microphone_file() {
        owned_synthetic_case("system-only");
    }
    fn owned_synthetic_case(scenario: &str) {
        let temp = tempfile::tempdir().unwrap();
        let app_data = temp.path().join("録音前テスト 日本語");
        std::fs::create_dir(&app_data).unwrap();
        let registry = Registry::open(&app_data).unwrap();
        let mut request = crate::capture_test::tests::request();
        request.microphone_uid = "synthetic-microphone".into();
        let directory =
            crate::capture_test::files::TestDirectory::new_owned(&registry, request.clone())
                .unwrap();
        let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe");
        let mut command =
            crate::capture_test::owned_command(&helper, &directory, &request).unwrap();
        command
            .arg("--synthetic")
            .args(["--scenario", scenario])
            .stderr(Stdio::piped());
        let child = command.spawn().unwrap();
        drop(command);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let audio = crate::capture_test::files::verify(&directory, &request, &|| false).unwrap();
        assert_eq!(audio.system.as_ref().unwrap().info.frames, 240_000);
        if scenario == "system-only" {
            assert!(audio.microphone.is_none());
            assert!(!directory.path.join("microphone.caf").exists());
            assert_eq!(
                audio
                    .system
                    .as_ref()
                    .unwrap()
                    .playback(&directory, "system", &|| false)
                    .unwrap()
                    .total_frames(),
                240_000
            );
        } else {
            assert_eq!(audio.microphone.as_ref().unwrap().info.frames, 240_000);
        }
        directory.cleanup().unwrap();
    }
    #[test]
    fn windows_owned_test_reports_early_lease_failure_and_rejects_another_parent() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(temp.path()).unwrap();
        let request = crate::capture_test::tests::request();
        let directory =
            crate::capture_test::files::TestDirectory::new_owned(&registry, request.clone())
                .unwrap();
        let other = temp.path().join("another-parent");
        std::fs::create_dir(&other).unwrap();
        let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe");
        let mut command =
            crate::capture_test::owned_command(&helper, &directory, &request).unwrap();
        command.arg("--synthetic").arg("--output").arg(&other);
        let child = crate::capture_test::Probe(Mutex::new(command.spawn().unwrap()));
        drop(command);
        let error = crate::capture_test::watch(
            &directory,
            &child,
            &request,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(5),
            || panic!("lease failure must precede recording"),
        )
        .unwrap_err();
        assert!(
            error.contains("永続所有ディレクトリが一致しません"),
            "{error}"
        );
        assert!(std::fs::read_dir(other).unwrap().next().is_none());
        directory.cleanup().unwrap();
    }
    #[test]
    fn windows_parity_test_lease_survives_parent_handles_and_recovers_after_helper_exit() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(temp.path()).unwrap();
        let request = crate::capture_test::tests::request();
        let area = registry.allocate(request.clone()).unwrap();
        let path = area.path();
        let directory = crate::capture_test::files::TestDirectory::restore_owned(area).unwrap();
        let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe")
            .canonicalize()
            .unwrap();
        let mut launch = crate::capture_test::owned_command(&helper, &directory, &request).unwrap();
        launch
            .arg("--synthetic")
            .arg("--synthetic-startup-delay-ms")
            .arg("2000");
        let mut child = launch.spawn().unwrap();
        drop(launch);
        drop(directory);
        drop(registry);
        let registry = Registry::open(temp.path()).unwrap();
        let record = registry.pending().unwrap().pop().unwrap();
        assert!(
            registry.collect(record.clone()).is_err(),
            "a live inherited lease must retain the audio"
        );
        assert!(path.exists());
        child.kill().unwrap();
        child.wait().unwrap();
        registry.collect(record).unwrap();
        assert!(!path.exists());
        assert!(registry.pending().unwrap().is_empty());
    }
    #[test]
    fn windows_parity_test_recovery_keeps_unknown_payload() {
        let temp = tempfile::tempdir().unwrap();
        let registry = Registry::open(temp.path()).unwrap();
        let area = registry
            .allocate(crate::capture_test::tests::request())
            .unwrap();
        let path = area.path();
        std::fs::write(path.join("unregistered"), b"external bytes").unwrap();
        drop(area);
        assert!(registry
            .collect(registry.pending().unwrap().pop().unwrap())
            .is_err());
        assert_eq!(
            std::fs::read(path.join("unregistered")).unwrap(),
            b"external bytes"
        );
    }
}
