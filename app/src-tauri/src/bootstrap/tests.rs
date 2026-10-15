use super::*;
use std::os::unix::fs::symlink;

struct Fixture {
    _directory: tempfile::TempDir,
    config: PathBuf,
    legacy: PathBuf,
    target: PathBuf,
}
impl Fixture {
    fn create_target(&self) {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&self.target)
            .unwrap();
    }
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        Self {
            config: base.join("config"),
            legacy: base.join("app-data"),
            target: base.join("target"),
            _directory: directory,
        }
    }
    fn start(&self) -> (Bootstrap, Storage) {
        let b = Bootstrap::open(&self.config, &self.legacy).unwrap();
        let s = Storage::open(b.root().into()).unwrap();
        b.initialize(&s).unwrap();
        (b, s)
    }
    fn document(&self) -> Document {
        serde_json::from_slice(&std::fs::read(self.config.join(NAME)).unwrap()).unwrap()
    }
    fn replace_document(&self, doc: &Document) {
        std::fs::write(self.config.join(NAME), serde_json::to_vec(doc).unwrap()).unwrap();
    }
}
fn resign(doc: &mut Document) {
    doc.sha256 = hash(&serde_json::to_vec(&doc.record).unwrap());
}
fn tree_file(path: &Path, name: &str, content: &[u8]) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(path.join(name), content).unwrap();
}

#[test]
fn bootstrap_live_identity_checks_preserve_database_posix_locks() {
    use std::os::fd::AsRawFd;
    for action in ["initialize", "reinitialize", "view"] {
        let f = Fixture::new();
        let bootstrap = Bootstrap::open(&f.config, &f.legacy).unwrap();
        let storage = Storage::open(bootstrap.root().into()).unwrap();
        if action != "initialize" {
            bootstrap.initialize(&storage).unwrap();
        }
        let database = storage.root().join("meetings.sqlite3");
        let held = File::open(&database).unwrap();
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_RDLCK;
        lock.l_whence = libc::SEEK_SET as libc::c_short;
        lock.l_start = 128;
        lock.l_len = 1;
        assert_eq!(
            unsafe { libc::fcntl(held.as_raw_fd(), libc::F_SETLK, &lock) },
            0
        );
        let probe = || {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::deletion::fs::sqlite_lock_tests::external_control_lock_fixture",
                    "--ignored",
                ])
                .env("MON_CONTROL_LOCK_TEST_FILE", &database)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "bootstrap {action} released a database lock: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        probe();
        match action {
            "initialize" | "reinitialize" => bootstrap.initialize(&storage).unwrap(),
            "view" => {
                bootstrap.view().unwrap();
            }
            _ => unreachable!(),
        }
        probe();
        let reader = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::tests::exclusive_wal_external_reader_fixture",
                "--ignored",
            ])
            .env("MON_WAL_REOPEN_TEST_DATABASE", &database)
            .output()
            .unwrap();
        assert!(
            reader.status.success(),
            "bootstrap {action} allowed an external SQLite reader: {}",
            String::from_utf8_lossy(&reader.stdout)
        );
    }
}

#[test]
fn bootstrap_fresh_initialization_separates_data_models_and_control_then_reopens_same_database() {
    let f = Fixture::new();
    tree_file(&f.legacy.join("models"), "model.bin", b"model remains");
    let (b, s) = f.start();
    assert_eq!(b.root(), f.legacy.join("data"));
    assert!(!f.legacy.join("meetings.sqlite3").exists());
    let meeting = s.create_meeting("点検日").unwrap();
    assert_eq!(b.view().unwrap().origin, "default");
    let saved = std::fs::read(f.config.join(NAME)).unwrap();
    b.initialize(&s).unwrap();
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), saved);
    assert_eq!(
        std::fs::metadata(f.config.join(NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(b.root()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(s);
    drop(b);
    let (b, s) = f.start();
    assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
    assert_eq!(b.view().unwrap().revision, 0);
    assert_eq!(
        std::fs::read(f.legacy.join("models/model.bin")).unwrap(),
        b"model remains"
    );
}
#[test]
fn bootstrap_unregistered_old_layout_is_not_adopted_or_converted() {
    let f = Fixture::new();
    let old = Storage::open(f.legacy.clone()).unwrap();
    old.create_meeting("unsupported fixture").unwrap();
    tree_file(
        &f.legacy.join("meetings/audio"),
        "record.flac",
        b"audio remains",
    );
    tree_file(&f.legacy.join("models"), "model.bin", b"model remains");
    drop(old);
    let before = std::fs::read(f.legacy.join("meetings.sqlite3")).unwrap();
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    assert_eq!(
        std::fs::read(f.legacy.join("meetings.sqlite3")).unwrap(),
        before
    );
    assert!(!f.legacy.join("data").exists());
    assert_eq!(
        std::fs::read(f.legacy.join("meetings/audio/record.flac")).unwrap(),
        b"audio remains"
    );
    assert_eq!(
        std::fs::read(f.legacy.join("models/model.bin")).unwrap(),
        b"model remains"
    );
}

#[test]
fn bootstrap_shared_os_config_and_app_data_parent_uses_current_data_directory() {
    let mut f = Fixture::new();
    f.config = f.legacy.clone();
    tree_file(&f.legacy.join("models"), "model.bin", b"retained");
    let (b, s) = f.start();
    let meeting = s.create_meeting("current shared parent").unwrap();
    let backup = f._directory.path().join("shared-parent.monbackup");
    s.create_backup_archive(&backup, &|| false).unwrap();
    assert_eq!(b.view().unwrap().meeting_root, f.legacy.join("data"));
    assert_eq!(
        std::fs::read(f.legacy.join("models/model.bin")).unwrap(),
        b"retained"
    );
    drop(s);
    drop(b);
    assert_eq!(f.start().1.list_meetings().unwrap(), vec![meeting]);
}
#[test]
fn bootstrap_corrupt_missing_oversized_or_public_configuration_never_falls_back_to_empty_database()
{
    for case in ["json", "missing", "size", "permissions", "hash"] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let meeting = s.create_meeting("must remain").unwrap();
        let root = b.root().to_path_buf();
        drop(s);
        drop(b);
        match case {
            "json" => std::fs::write(f.config.join(NAME), b"{} broken").unwrap(),
            "missing" => {
                std::fs::rename(f.config.join(NAME), f.config.join("retained.json")).unwrap()
            }
            "size" => std::fs::write(f.config.join(NAME), vec![b' '; LIMIT as usize + 1]).unwrap(),
            "permissions" => std::fs::set_permissions(
                f.config.join(NAME),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap(),
            "hash" => {
                let mut d = f.document();
                d.sha256 = "0".repeat(64);
                f.replace_document(&d);
            }
            _ => unreachable!(),
        }
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err(), "{case}");
        assert!(!f.legacy.join("meetings.sqlite3").exists());
        let source = Storage::open(root).unwrap();
        assert_eq!(source.list_meetings().unwrap(), vec![meeting]);
    }
}
#[test]
fn bootstrap_rehashed_unknown_version_fields_paths_and_revision_are_rejected() {
    for case in ["version", "field", "path", "revision", "intent"] {
        let f = Fixture::new();
        let (b, s) = f.start();
        drop(s);
        drop(b);
        let mut d = f.document();
        match case {
            "version" => d.schema_version = 99,
            "path" => d.record.current.path = PathBuf::from("../external"),
            "revision" => d.record.revision = u64::MAX,
            "intent" => {
                d.record.pending_move = Some(PendingMove {
                    operation_id: "wrong".into(),
                    confirmation_id: Uuid::new_v4().to_string(),
                    source: d.record.current.clone(),
                    target: f.target.clone(),
                    target_directory: d.record.current.directory.clone(),
                    manifest_hash: "0".repeat(64),
                })
            }
            "field" => {}
            _ => unreachable!(),
        }
        resign(&mut d);
        if case == "field" {
            let mut v = serde_json::to_value(&d).unwrap();
            v["record"]["adopt"] = serde_json::json!(true);
            std::fs::write(f.config.join(NAME), serde_json::to_vec(&v).unwrap()).unwrap();
        } else {
            f.replace_document(&d);
        }
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err(), "{case}");
        assert!(!f.legacy.join("meetings.sqlite3").exists());
    }
}
#[test]
fn bootstrap_missing_replaced_or_linked_root_and_database_are_not_recreated_or_adopted() {
    for case in [
        "missing_root",
        "replaced_root",
        "root_link",
        "missing_db",
        "replaced_db",
        "db_link",
        "db_hardlink",
        "truncated_db",
    ] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let root = b.root().to_path_buf();
        s.create_meeting("preserve").unwrap();
        drop(s);
        drop(b);
        if case.contains("root") {
            std::fs::rename(&root, &f.target).unwrap();
            if case == "replaced_root" {
                tree_file(&root, "foreign", b"foreign");
            }
            if case == "root_link" {
                symlink(&f.target, &root).unwrap();
            }
        } else {
            let db = root.join("meetings.sqlite3");
            if case == "truncated_db" {
                std::fs::write(&db, b"").unwrap();
            } else if case == "db_hardlink" {
                std::fs::hard_link(&db, f.target.clone()).unwrap();
            } else {
                std::fs::rename(&db, &f.target).unwrap();
                if case == "replaced_db" {
                    std::fs::copy(&f.target, &db).unwrap();
                }
                if case == "db_link" {
                    symlink(&f.target, &db).unwrap();
                }
            }
        }
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err(), "{case}");
        if case == "missing_root" {
            assert!(!root.exists());
        }
        if case == "missing_db" {
            assert!(!root.join("meetings.sqlite3").exists());
        }
        if case == "truncated_db" {
            assert_eq!(
                std::fs::metadata(root.join("meetings.sqlite3"))
                    .unwrap()
                    .len(),
                0
            );
        }
        if case == "replaced_root" {
            assert_eq!(std::fs::read(root.join("foreign")).unwrap(), b"foreign");
        }
        assert!(!f.legacy.join("meetings.sqlite3").exists());
    }
}
#[test]
fn bootstrap_exclusive_control_lock_survives_initialization_and_releases_on_drop() {
    let f = Fixture::new();
    let (b, s) = f.start();
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    assert_eq!(b.view().unwrap().revision, 0);
    drop(b);
    let reopened = Bootstrap::open(&f.config, &f.legacy).unwrap();
    assert_eq!(reopened.root(), s.root());
    drop(reopened);
    drop(s);
    assert!(Bootstrap::open(&f.config, &f.legacy).is_ok());
}

#[test]
fn bootstrap_owner_drop_releases_its_lock_despite_a_retained_duplicate_descriptor() {
    let f = Fixture::new();
    let (b, s) = f.start();
    // Negative control: closing a bare locked File does not release the
    // shared flock while a duplicate survives. This reproduces the old
    // lifetime bug without depending on concurrent-spawn scheduling.
    let control_path = f.config.join("synthetic-negative-control.lock");
    let bare = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&control_path)
        .unwrap();
    bare.try_lock().unwrap();
    let retained = bare.try_clone().unwrap();
    drop(bare);
    let contender = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&control_path)
        .unwrap();
    assert!(contender.try_lock().is_err());
    retained.unlock().unwrap();
    contender.try_lock().unwrap();
    drop(retained);
    drop(contender);

    // Models the shared open file description inherited before another
    // thread's spawn reaches exec. CLOEXEC does not close it before exec.
    let inherited = b.lock.try_clone().unwrap();
    let saved = std::fs::read(f.config.join(NAME)).unwrap();
    // Failed acquisition must not create an unlocking guard or release b.
    for _ in 0..2 {
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
        assert!(b.view().is_ok());
    }
    drop(s);
    drop(b);
    let next = Bootstrap::open(&f.config, &f.legacy).unwrap();
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), saved);
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    // The stale raw descriptor's eventual close cannot unlock next's
    // separately acquired file description.
    drop(inherited);
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    assert!(next.view().is_ok());
    drop(next);
    assert!(Bootstrap::open(&f.config, &f.legacy).is_ok());
}
#[test]
fn bootstrap_pending_intent_replays_on_old_root_even_when_target_disappears_and_cancel_deletes_nothing(
) {
    let f = Fixture::new();
    let (b, s) = f.start();
    let meeting = s.create_meeting("source remains").unwrap();
    f.create_target();
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    let manifest = "a".repeat(64);
    let before = b.view().unwrap();
    let intent = b
        .begin_move(
            &operation,
            &confirmation,
            &f.target,
            &manifest,
            before.revision,
        )
        .unwrap();
    std::fs::write(f.target.join("copy.part"), b"unverified copy").unwrap();
    assert_eq!(intent.meeting_root, before.meeting_root);
    assert_eq!(intent.revision, 1);
    let encoded = std::fs::read(f.config.join(NAME)).unwrap();
    assert_eq!(
        b.begin_move(
            &operation,
            &confirmation,
            &f.target,
            &manifest,
            before.revision
        )
        .unwrap()
        .revision,
        1
    );
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), encoded);
    drop(s);
    drop(b);
    // Retain the partial copy but remove its old pathname. Startup must not
    // open/recreate/adopt the destination of an unverified move.
    let retained = f.target.with_extension("retained");
    std::fs::rename(&f.target, &retained).unwrap();
    let (b, s) = f.start();
    assert_eq!(s.list_meetings().unwrap(), vec![meeting]);
    assert_eq!(b.view().unwrap().pending_target, Some(f.target.clone()));
    assert!(!f.target.exists());
    assert!(b.cancel_move("wrong").is_err());
    let cancelled = b.cancel_move(&operation).unwrap();
    assert_eq!(cancelled.revision, 2);
    assert!(cancelled.pending_target.is_none());
    assert_eq!(
        std::fs::read(retained.join("copy.part")).unwrap(),
        b"unverified copy"
    );
}
#[test]
fn bootstrap_old_confirmation_wrong_id_or_nested_target_cannot_change_current_or_pending_root() {
    let f = Fixture::new();
    let (b, s) = f.start();
    f.create_target();
    let original = std::fs::read(f.config.join(NAME)).unwrap();
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    for (id, confirm, target, manifest, revision) in [
        (
            "wrong",
            confirmation.as_str(),
            f.target.as_path(),
            "a".repeat(64),
            0,
        ),
        (
            operation.as_str(),
            "wrong",
            f.target.as_path(),
            "a".repeat(64),
            0,
        ),
        (
            operation.as_str(),
            confirmation.as_str(),
            f.target.as_path(),
            "wrong".into(),
            0,
        ),
        (
            operation.as_str(),
            confirmation.as_str(),
            f.target.as_path(),
            "a".repeat(64),
            99,
        ),
        (
            operation.as_str(),
            confirmation.as_str(),
            b.root(),
            "a".repeat(64),
            0,
        ),
        (
            operation.as_str(),
            confirmation.as_str(),
            f.config.as_path(),
            "a".repeat(64),
            0,
        ),
    ] {
        assert!(b
            .begin_move(id, confirm, target, &manifest, revision)
            .is_err());
    }
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), original);
    assert_eq!(s.root(), b.root());
}
#[test]
fn bootstrap_external_configuration_change_is_not_overwritten_by_existing_manager() {
    let f = Fixture::new();
    let (b, s) = f.start();
    let mut d = f.document();
    d.record.revision += 1;
    resign(&mut d);
    f.replace_document(&d);
    let replaced = std::fs::read(f.config.join(NAME)).unwrap();
    assert!(b.view().is_err());
    assert!(b.initialize(&s).is_err());
    f.create_target();
    assert!(b
        .begin_move(
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
            &f.target,
            &"a".repeat(64),
            0
        )
        .is_err());
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), replaced);
}
#[test]
fn bootstrap_initialization_gap_marker_stops_automatic_default_selection_without_database_creation()
{
    let f = Fixture::new();
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
    let planned = b.root().to_path_buf();
    std::fs::write(f.config.join(LOCK), STAMP).unwrap();
    drop(b);
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    assert!(!planned.join("meetings.sqlite3").exists());
    assert!(!f.legacy.join("meetings.sqlite3").exists());
}
#[test]
fn bootstrap_linked_control_files_parent_replacement_and_ambiguous_legacy_are_rejected() {
    for case in [
        "config_link",
        "config_hardlink",
        "lock_link",
        "lock_replacement",
        "parent_replacement",
        "ambiguous",
    ] {
        let f = Fixture::new();
        let (b, s) = f.start();
        drop(s);
        if case == "parent_replacement" {
            std::fs::rename(&f.config, &f.target).unwrap();
            tree_file(&f.config, "foreign", b"protected");
            assert!(b.view().is_err());
            assert_eq!(
                std::fs::read(f.config.join("foreign")).unwrap(),
                b"protected"
            );
            continue;
        }
        if case == "lock_replacement" {
            std::fs::rename(f.config.join(LOCK), &f.target).unwrap();
            std::fs::write(f.config.join(LOCK), STAMP).unwrap();
            assert!(b.view().is_err());
            continue;
        }
        drop(b);
        match case {
            "config_link" => {
                std::fs::rename(f.config.join(NAME), &f.target).unwrap();
                symlink(&f.target, f.config.join(NAME)).unwrap();
            }
            "config_hardlink" => std::fs::hard_link(f.config.join(NAME), &f.target).unwrap(),
            "lock_link" => {
                std::fs::rename(f.config.join(LOCK), &f.target).unwrap();
                symlink(&f.target, f.config.join(LOCK)).unwrap();
            }
            "ambiguous" => {
                // Before bootstrap introduction, neither of two candidate DBs
                // is privileged. Keep both and require an explicit resolution.
                std::fs::rename(&f.config, &f.target).unwrap();
                let old = Storage::open(f.legacy.clone()).unwrap();
                drop(old);
            }
            _ => unreachable!(),
        }
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err(), "{case}");
    }
}
#[test]
fn bootstrap_mount_policy_rejects_remote_and_read_only_flags_without_claiming_performance() {
    assert!(mount_supported(libc::MNT_LOCAL as u32, b"apfs"));
    assert!(!mount_supported(0, b"apfs"));
    assert!(!mount_supported(
        libc::MNT_LOCAL as u32 | libc::MNT_RDONLY as u32,
        b"apfs"
    ));
    for kind in [b"smbfs".as_slice(), b"nfs", b"exfat", b"hfs", b"unknown"] {
        assert!(!mount_supported(libc::MNT_LOCAL as u32, kind));
    }
}

#[test]
fn bootstrap_uninitialized_legacy_empty_database_is_not_silently_populated_with_new_schema() {
    let f = Fixture::new();
    tree_file(&f.legacy, "meetings.sqlite3", b"");
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    assert_eq!(
        std::fs::metadata(f.legacy.join("meetings.sqlite3"))
            .unwrap()
            .len(),
        0
    );
    assert!(!f.legacy.join("data/meetings.sqlite3").exists());
}

#[test]
fn bootstrap_old_schema_and_missing_current_relocation_capability_are_rejected_before_open() {
    for (version, origin) in [
        (1, Origin::Default),
        (1, Origin::Relocated),
        (2, Origin::Relocated),
    ] {
        let f = Fixture::new();
        let (b, s) = f.start();
        let original = s.create_meeting("current data remains").unwrap();
        let root = b.root().to_owned();
        drop(s);
        drop(b);
        let mut d = f.document();
        d.schema_version = version;
        d.record.origin = origin;
        resign(&mut d);
        f.replace_document(&d);
        let config = std::fs::read(f.config.join(NAME)).unwrap();
        let database = std::fs::read(root.join("meetings.sqlite3")).unwrap();
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
        assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), config);
        assert_eq!(
            std::fs::read(root.join("meetings.sqlite3")).unwrap(),
            database
        );
        assert!(!f.target.exists());
        assert_eq!(
            Storage::open(root).unwrap().list_meetings().unwrap(),
            vec![original]
        );
    }
}

#[test]
fn bootstrap_move_intent_refuses_nonempty_or_public_destination_and_preserves_existing_objects() {
    let f = Fixture::new();
    let (b, _s) = f.start();
    f.create_target();
    tree_file(&f.target, "foreign", b"must stay");
    let old = std::fs::read(f.config.join(NAME)).unwrap();
    let id = Uuid::new_v4().to_string();
    let consent = Uuid::new_v4().to_string();
    assert!(b
        .begin_move(&id, &consent, &f.target, &"a".repeat(64), 0)
        .is_err());
    assert_eq!(
        std::fs::read(f.target.join("foreign")).unwrap(),
        b"must stay"
    );
    let public = f.target.with_extension("public");
    std::fs::create_dir(&public).unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(b
        .begin_move(&id, &consent, &public, &"a".repeat(64), 0)
        .is_err());
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), old);
}

#[test]
fn bootstrap_control_lock_child() {
    let Ok(raw) = std::env::var("MON_BOOTSTRAP_LOCK_CHILD") else {
        return;
    };
    let (config, legacy, mode, marker): (PathBuf, PathBuf, String, PathBuf) =
        serde_json::from_str(&raw).unwrap();
    if mode == "denied" {
        let error = Bootstrap::open(&config, &legacy)
            .err()
            .expect("second process must be blocked");
        assert!(error.contains("実行中"), "{error}");
        return;
    }
    let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
    assert!(bootstrap.view().is_ok());
    if mode == "hold" {
        std::fs::write(marker, b"locked").unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[test]
fn bootstrap_actual_subprocess_lock_blocks_other_instances_and_is_released_after_sigkill() {
    use std::process::{Command, Stdio};
    let f = Fixture::new();
    let (b, s) = f.start();
    let encoded = std::fs::read(f.config.join(NAME)).unwrap();
    let marker = f.config.join("child.ready");
    let command = |mode: &str| {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--exact",
            "bootstrap::tests::bootstrap_control_lock_child",
            "--nocapture",
        ])
        .env(
            "MON_BOOTSTRAP_LOCK_CHILD",
            serde_json::to_string(&(&f.config, &f.legacy, mode, &marker)).unwrap(),
        );
        c
    };
    let denied = command("denied").output().unwrap();
    assert!(
        denied.status.success(),
        "{:?}",
        String::from_utf8_lossy(&denied.stdout)
    );
    drop(s);
    drop(b);
    assert!(command("allowed").output().unwrap().status.success());
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        command("hold")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child failed before locking"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "child did not become ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(Bootstrap::open(&f.config, &f.legacy)
        .unwrap()
        .view()
        .is_ok());
    assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), encoded);
}
