use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        Self {
            config: root.join("config"),
            data: root.join("app"),
            _temp: temp,
        }
    }
    fn start(&self) -> (Bootstrap, Storage) {
        let bootstrap = Bootstrap::open(&self.config, &self.data).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        (bootstrap, storage)
    }
    fn change_device(&self) {
        let path = self.config.join(NAME);
        let mut doc: Document = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        doc.record.current.directory.device += 100;
        doc.record.current.database.device += 100;
        if let Some(pending) = &mut doc.record.pending_move {
            pending.source = doc.record.current.clone();
        }
        doc.sha256 = hash(&serde_json::to_vec(&doc.record).unwrap());
        std::fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
    }
}

#[test]
fn device_rebinding_reopens_same_meetings_and_is_idempotent_across_restarts() {
    let f = Fixture::new();
    let (b, s) = f.start();
    assert!(b.startup_recovery_view(&s).unwrap().is_none());
    let meeting = s.create_meeting("再起動後も保持").unwrap();
    drop(s);
    drop(b);
    f.change_device();
    let old = std::fs::read(f.config.join(NAME)).unwrap();
    for round in 0..2 {
        let (b, s) = f.start();
        assert_eq!(s.list_meetings().unwrap(), vec![meeting.clone()]);
        assert!(!s.data_write_blocked());
        let recovery = b.recovery_view().unwrap().unwrap();
        assert_eq!(recovery.kind, "device_changed");
        assert!(!recovery.review_required);
        assert_eq!(b.startup_recovery_view(&s).unwrap(), Some(recovery.clone()));
        if round == 0 {
            assert_eq!(std::fs::read(&recovery.record_path).unwrap(), old);
        }
        let saved = std::fs::read(f.config.join(NAME)).unwrap();
        drop(s);
        drop(b);
        let (b, s) = f.start();
        assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), saved);
        assert_eq!(s.list_meetings().unwrap(), vec![meeting.clone()]);
        assert!(b.startup_recovery_view(&s).unwrap().is_none());
        assert_eq!(b.recovery_view_for_storage(&s).unwrap(), Some(recovery));
        drop(s);
        drop(b);
        if round == 0 {
            f.change_device();
        }
    }
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir(target).unwrap();
    std::fs::set_permissions(target, std::fs::metadata(source).unwrap().permissions()).unwrap();
    for item in std::fs::read_dir(source).unwrap() {
        let item = item.unwrap();
        let to = target.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            copy_tree(&item.path(), &to);
        } else {
            std::fs::copy(item.path(), to).unwrap();
        }
    }
}

#[test]
fn copied_root_retains_stale_scratch_and_audio_and_uses_new_workspace() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let meeting = s.create_meeting("コピーから復旧").unwrap();
    let root = b.root().to_owned();
    std::fs::create_dir_all(root.join("meetings/audio")).unwrap();
    std::fs::write(root.join("meetings/audio/keep.flac"), b"preserved audio").unwrap();
    drop(s);
    drop(b);
    let old_root = f.data.join("retained");
    std::fs::rename(&root, &old_root).unwrap();
    copy_tree(&old_root, &root);
    let scratch = std::fs::read(root.join(".mon-scratch.sqlite3")).unwrap();
    // A rejected old read-cache must not be opened or erased on this path.
    std::fs::create_dir(f.config.join(".mon-read-cache")).unwrap();
    std::fs::write(f.config.join(".mon-read-cache/unknown"), b"retained cache").unwrap();
    let (b, s) = f.start();
    assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
    assert!(!s.data_write_blocked());
    assert_eq!(b.recovery_view().unwrap().unwrap().kind, "copy_restored");
    assert_eq!(
        std::fs::read(root.join(".mon-scratch.sqlite3")).unwrap(),
        scratch
    );
    assert_eq!(
        std::fs::read(root.join("meetings/audio/keep.flac")).unwrap(),
        b"preserved audio"
    );
    assert_eq!(
        std::fs::read(f.config.join(".mon-read-cache/unknown")).unwrap(),
        b"retained cache"
    );
    s.create_meeting("復旧後の新規会議").unwrap();
    let backup = f._temp.path().join("after-recovery.monbackup");
    s.create_backup_archive(&backup, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(backup.exists());
}

#[test]
fn completed_deletion_stays_completed_after_copy_without_fencing_normal_use() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let deleted = s.create_meeting("削除済み").unwrap();
    let kept = s.create_meeting("残す会議").unwrap();
    let analysis = crate::analysis_worker::AnalysisController::default();
    let lease = analysis
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let mut prepared = crate::storage::deletion::operations::Prepared::fresh(
        &s,
        &deleted.id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = prepared.preview();
    let receipt = prepared
        .execute(
            &s,
            &lease,
            &preview.confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted);
    drop(prepared);
    drop(lease);
    let root = b.root().to_owned();
    drop(s);
    drop(b);
    let old = f.data.join("before-copy");
    std::fs::rename(&root, &old).unwrap();
    copy_tree(&old, &root);
    let (b, s) = f.start();
    assert_eq!(s.list_meetings().unwrap(), vec![kept]);
    assert!(!s.data_write_blocked());
    assert!(
        !b.recovery_view_for_storage(&s)
            .unwrap()
            .unwrap()
            .review_required
    );
}

#[test]
fn relocation_after_rebinding_uses_current_journal_and_cache_generation() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let meeting = s.create_meeting("復旧後に移動").unwrap();
    drop(s);
    drop(b);
    f.change_device();
    std::fs::create_dir(f.config.join(".mon-read-cache")).unwrap();
    std::fs::write(f.config.join(".mon-read-cache/unknown"), b"retained").unwrap();
    let (b, s) = f.start();
    let chosen = f._temp.path().canonicalize().unwrap().join("chosen");
    std::fs::create_dir(&chosen).unwrap();
    let mut prepared = b
        .prepare_move(&s, &chosen, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let preview = prepared.preview();
    let receipt = prepared
        .publish(
            &b,
            &s,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    drop(prepared);
    drop(s);
    drop(b);
    let (b, s) = f.start();
    assert_eq!(b.root(), receipt.destination);
    assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
    assert!(!s.data_write_blocked());
    assert_eq!(
        std::fs::read(f.config.join(".mon-read-cache/unknown")).unwrap(),
        b"retained"
    );
}

#[test]
fn copied_database_is_adopted_but_empty_or_foreign_schema_preserves_configuration() {
    for case in ["copy", "empty", "foreign", "corrupt"] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let meeting = s.create_meeting("保持する会議").unwrap();
        let root = b.root().to_owned();
        drop(s);
        drop(b);
        let db = root.join("meetings.sqlite3");
        let retained = root.join("retained.db");
        std::fs::rename(&db, &retained).unwrap();
        match case {
            "copy" => {
                std::fs::copy(&retained, &db).unwrap();
            }
            "empty" => {
                rusqlite::Connection::open(&db)
                    .unwrap()
                    .execute_batch("VACUUM;")
                    .unwrap();
            }
            "foreign" => {
                rusqlite::Connection::open(&db)
                    .unwrap()
                    .execute_batch("CREATE TABLE foreign_data(id);")
                    .unwrap();
            }
            _ => std::fs::write(&db, vec![b'x'; 4096]).unwrap(),
        }
        let config = std::fs::read(f.config.join(NAME)).unwrap();
        if case == "copy" {
            assert_eq!(f.start().1.list_meetings().unwrap(), vec![meeting]);
        } else {
            assert!(Bootstrap::open(&f.config, &f.data).is_err(), "{case}");
            assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), config);
        }
        assert!(retained.exists());
    }
}

#[test]
fn runtime_database_replacement_remains_rejected_after_rebinding() {
    let f = Fixture::new();
    let (b, s) = f.start();
    drop(s);
    drop(b);
    f.change_device();
    let (b, s) = f.start();
    let db = b.root().join("meetings.sqlite3");
    std::fs::rename(&db, db.with_extension("retained")).unwrap();
    std::fs::copy(db.with_extension("retained"), &db).unwrap();
    assert!(b.view().is_err());
    assert!(b.initialize(&s).is_err());
}

#[test]
fn pending_maintenance_is_retained_readonly_across_restarts() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let meeting = s.create_meeting("中断操作を保持").unwrap();
    crate::storage::with_test_database(&s, |db| {
        db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'prune_revisions','attention','{}',?2,'{}',1,1)", rusqlite::params![Uuid::new_v4().to_string(), "a".repeat(64)]).unwrap();
    });
    drop(s);
    drop(b);
    f.change_device();
    for _ in 0..2 {
        let (b, s) = f.start();
        assert_eq!(s.list_meetings().unwrap(), vec![meeting.clone()]);
        assert!(s.data_write_blocked());
        assert!(s.create_meeting("禁止").is_err());
        assert!(s.require_data_writable().is_err());
        assert!(b.recovery_view().unwrap().unwrap().review_required);
        assert!(
            b.startup_recovery_view(&s)
                .unwrap()
                .unwrap()
                .review_required
        );
        crate::storage::with_test_database(&s, |db| {
            assert_eq!(
                db.query_row("SELECT state FROM maintenance_operations", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "attention"
            );
        });
        drop(s);
        drop(b);
    }
}

#[test]
fn pending_move_is_archived_without_activating_or_erasing_destination() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let target = f._temp.path().canonicalize().unwrap().join("target");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    b.begin_move(
        &Uuid::new_v4().to_string(),
        &Uuid::new_v4().to_string(),
        &target,
        &"a".repeat(64),
        0,
    )
    .unwrap();
    std::fs::write(target.join("unknown"), b"retained destination").unwrap();
    drop(s);
    drop(b);
    f.change_device();
    let saved = std::fs::read(f.config.join(NAME)).unwrap();
    let (b, s) = f.start();
    assert!(s.data_write_blocked());
    let recovery = b.recovery_view().unwrap().unwrap();
    assert!(recovery.reasons.iter().any(|s| s.contains("保存先移動")));
    assert_eq!(std::fs::read(recovery.record_path).unwrap(), saved);
    assert_eq!(
        std::fs::read(target.join("unknown")).unwrap(),
        b"retained destination"
    );
    assert_ne!(b.root(), target);
}

fn stale_journal(root: &Path) {
    let db = rusqlite::Connection::open(root.join(".mon-scratch.sqlite3")).unwrap();
    let json: String = db
        .query_row("SELECT proof FROM metadata WHERE id=1", [], |r| r.get(0))
        .unwrap();
    let mut proof: serde_json::Value = serde_json::from_str(&json).unwrap();
    proof["device"] = serde_json::json!(proof["device"].as_u64().unwrap() + 100);
    let json = serde_json::to_string(&proof).unwrap();
    db.execute_batch("DROP TRIGGER metadata_fixed;").unwrap();
    db.execute(
        "UPDATE metadata SET proof=?1,hash=?2 WHERE id=1",
        rusqlite::params![json, hash(json.as_bytes())],
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER metadata_fixed BEFORE UPDATE ON metadata BEGIN SELECT RAISE(ABORT,'scratch root is immutable'); END;").unwrap();
}

#[test]
fn device_change_with_old_scratch_and_read_cache_devices_still_starts_normally() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let meeting = s.create_meeting("台帳の番号も変更").unwrap();
    let root = b.root().to_owned();
    drop(s);
    drop(b);
    drop(crate::storage::read_cache::Snapshot::create(&f.config).unwrap());
    stale_journal(&root);
    stale_journal(&f.config.join(".mon-read-cache"));
    let old_journal = std::fs::read(root.join(".mon-scratch.sqlite3")).unwrap();
    let old_cache = std::fs::read(f.config.join(".mon-read-cache/.mon-scratch.sqlite3")).unwrap();
    f.change_device();
    let (b, s) = f.start();
    assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
    assert!(!s.data_write_blocked());
    assert!(!b.recovery_view().unwrap().unwrap().review_required);
    assert_eq!(
        std::fs::read(root.join(".mon-scratch.sqlite3")).unwrap(),
        old_journal
    );
    assert_eq!(
        std::fs::read(f.config.join(".mon-read-cache/.mon-scratch.sqlite3")).unwrap(),
        old_cache
    );
}

#[test]
fn cache_device_change_alone_rebinds_even_when_meeting_volume_did_not_change() {
    for cache_only in [true, false] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let meeting = s.create_meeting("会議の媒体は変更なし").unwrap();
        let root = b.root().to_owned();
        drop(s);
        drop(b);
        drop(crate::storage::read_cache::Snapshot::create(&f.config).unwrap());
        stale_journal(
            if cache_only {
                f.config.join(".mon-read-cache")
            } else {
                root.clone()
            }
            .as_path(),
        );
        let (b, s) = f.start();
        assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
        assert!(!s.data_write_blocked());
        assert_eq!(b.recovery_view().unwrap().unwrap().kind, "device_changed");
    }
}

#[test]
fn recovery_workspace_device_change_alone_selects_another_generation() {
    let f = Fixture::new();
    let (b, s) = f.start();
    drop(s);
    drop(b);
    f.change_device();
    let (b, s) = f.start();
    let retained = b.recovery_view().unwrap().unwrap().record_path;
    drop(s);
    drop(b);
    let path = f.config.join(NAME);
    let mut doc: Document = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc.record
        .recovery
        .as_mut()
        .unwrap()
        .workspace_identity
        .device += 100;
    doc.sha256 = hash(&serde_json::to_vec(&doc.record).unwrap());
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let (b, s) = f.start();
    assert!(!s.data_write_blocked());
    assert_ne!(b.recovery_view().unwrap().unwrap().record_path, retained);
    assert!(retained.exists());
}

fn spawn_fixture(f: &Fixture, action: &str) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bootstrap::recovery::tests::rebinding_subprocess_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("MON_REBINDING_CONFIG", &f.config)
        .env("MON_REBINDING_DATA", &f.data)
        .env("MON_REBINDING_ACTION", action)
        .env("MON_REBINDING_CHECKPOINT", action)
        .env("MON_REBINDING_RESUME_FILE", f._temp.path().join("resume"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
fn wait_ready(child: &mut std::process::Child, expected: &str) {
    use std::io::{BufRead, BufReader};
    let out = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let expected = expected.to_owned();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            if line.unwrap().contains(&expected) {
                let _ = send.send(());
                return;
            }
        }
    });
    if receive
        .recv_timeout(std::time::Duration::from_secs(30))
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child did not reach checkpoint");
    }
}

#[test]
fn recovery_after_sigkill_preserves_uncheckpointed_wal_rows() {
    for action in ["wal_writer", "wal_shared_writer"] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let root = b.root().to_owned();
        drop(s);
        drop(b);
        let mut child = spawn_fixture(&f, action);
        wait_ready(&mut child, "WAL_READY");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            std::fs::metadata(root.join("meetings.sqlite3-wal"))
                .unwrap()
                .len()
                > 0
        );
        f.change_device();
        let (_, s) = f.start();
        assert_eq!(s.list_meetings().unwrap()[0].title, "WAL内の会議");
        assert!(!s.data_write_blocked());
    }
}

#[test]
fn rebinding_publication_survives_sigkill_before_and_after_selection() {
    for point in ["before_publication", "after_publication"] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let meeting = s.create_meeting("設定公開境界").unwrap();
        drop(s);
        drop(b);
        f.change_device();
        let old = std::fs::read(f.config.join(NAME)).unwrap();
        let mut child = spawn_fixture(&f, point);
        wait_ready(&mut child, &format!("MON_REBINDING_READY={point}"));
        child.kill().unwrap();
        child.wait().unwrap();
        if point == "before_publication" {
            assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), old);
        }
        let (b, s) = f.start();
        assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
        assert!(!s.data_write_blocked());
        assert_eq!(
            std::fs::read(b.recovery_view().unwrap().unwrap().record_path).unwrap(),
            old
        );
    }
}

#[test]
fn source_modified_after_snapshot_is_rejected_before_publication() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let root = b.root().to_owned();
    drop(s);
    drop(b);
    f.change_device();
    let saved = std::fs::read(f.config.join(NAME)).unwrap();
    let mut child = spawn_fixture(&f, "before_publication");
    wait_ready(&mut child, "MON_REBINDING_READY=before_publication");
    std::fs::OpenOptions::new()
        .append(true)
        .open(root.join("meetings.sqlite3"))
        .unwrap()
        .write_all(b"changed source")
        .unwrap();
    std::fs::write(f._temp.path().join("resume"), b"resume").unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("復旧の検査中にDBまたはWALが変更"));
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), saved);
}

#[test]
#[ignore = "parent-owned startup and WAL crash fixture"]
fn rebinding_subprocess_fixture() {
    let config = PathBuf::from(std::env::var_os("MON_REBINDING_CONFIG").unwrap());
    let data = PathBuf::from(std::env::var_os("MON_REBINDING_DATA").unwrap());
    let b = Bootstrap::open(&config, &data).unwrap();
    if std::env::var("MON_REBINDING_ACTION").as_deref() == Ok("wal_shared_writer") {
        let db = rusqlite::Connection::open(b.root().join("meetings.sqlite3")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,'WAL内の会議','draft',1,1)", [Uuid::new_v4().to_string()]).unwrap();
        println!("WAL_READY");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    let s = b.open_storage().unwrap();
    if std::env::var("MON_REBINDING_ACTION").as_deref() == Ok("wal_writer") {
        crate::storage::with_test_database(&s, |db| {
            db.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
        });
        s.create_meeting("WAL内の会議").unwrap();
        println!("WAL_READY");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
}
