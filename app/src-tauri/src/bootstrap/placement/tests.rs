use super::*;
use std::cell::Cell;
use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

pub(super) struct Fixture {
    pub(super) _base: tempfile::TempDir,
    pub(super) bootstrap: Bootstrap,
    pub(super) storage: Storage,
    pub(super) target: PathBuf,
    pub(super) operation: String,
    pub(super) relative: String,
    pub(super) meeting: String,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let parent = base.path().canonicalize().unwrap();
        let b = Bootstrap::open(&parent.join("config"), &parent.join("app-data")).unwrap();
        let storage = Storage::open(b.root().into()).unwrap();
        b.initialize(&storage).unwrap();
        let meeting = storage.create_meeting("配置しても同じ会議").unwrap().id;
        let relative = format!("meetings/{meeting}/audio/source.flac");
        let file = storage.root().join(&relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"original audio").unwrap();
        storage
            .register_ready_meeting_asset(&meeting, Path::new(&relative), "audio/flac")
            .unwrap();
        let target = parent.join("target");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .unwrap();
        Self {
            _base: base,
            bootstrap: b,
            storage,
            target,
            operation: Uuid::new_v4().to_string(),
            relative,
            meeting,
        }
    }
    pub(super) fn intent(&self, snapshot: &NativeSnapshot<'_>) {
        self.bootstrap
            .begin_move(
                &self.operation,
                &Uuid::new_v4().to_string(),
                &self.target,
                snapshot.manifest_hash(),
                0,
            )
            .unwrap();
    }
    fn control(&self) -> PathBuf {
        self.bootstrap
            .config
            .join(format!(".relocation-{}.sqlite3", self.operation))
    }
}

#[test]
fn placement_preserves_native_rows_audio_and_current_root_without_cutover_or_deletion() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let config = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert_eq!(p.progress().unwrap().state, "copying");
    let result = p.copy(&snapshot, &|| false).unwrap();
    assert_eq!(result.state, "ready");
    assert_eq!(result.expected_paths, result.verified_paths);
    assert_eq!(result.owned_paths, result.verified_paths);
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(
        std::fs::read(f.storage.root().join(&f.relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(
        std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
        config
    );
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    let copied = Connection::open_with_flags(
        f.target.join("meetings.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        copied
            .query_row("SELECT id FROM meetings", [], |r| r.get::<_, String>(0))
            .unwrap(),
        f.meeting
    );
    assert!(!f.target.join(".mon-scratch.sqlite3").exists());
    assert!(!f.target.join("models").exists());
    assert!(!f.target.join("bootstrap.json").exists());
    assert_eq!(
        f.control().metadata().unwrap().permissions().mode() & 0o077,
        0
    );
    for node in p.nodes.values() {
        assert_eq!(
            f.target
                .join(&node.entry.relative_path)
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    p.verify(&snapshot, &|| false).unwrap();
    assert_eq!(p.copy(&snapshot, &|| false).unwrap(), result);
}

#[test]
fn placement_cancel_after_owned_file_record_can_continue_without_adopting_unknown_data() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    let called = Cell::new(false);
    // Inspect through a separate OS-visible file instead of borrowing p across
    // its mutable copy call. The DB is claimed before the first payload write.
    let cancel = || {
        let exists = f.target.join("meetings.sqlite3").exists();
        if exists {
            called.set(true);
        }
        exists
    };
    assert!(p.copy(&snapshot, &cancel).is_err());
    assert!(called.get());
    assert_eq!(p.progress().unwrap().owned_paths, 1);
    assert_eq!(p.progress().unwrap().verified_paths, 0);
    drop(p);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert_eq!(p.copy(&snapshot, &|| false).unwrap().state, "ready");
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
}

#[test]
fn placement_resumption_refuses_copied_content_change_without_overwrite() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    p.copy(&snapshot, &|| false).unwrap();
    drop(p);
    std::fs::write(f.target.join(&f.relative), b"external edit").unwrap();
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert!(p.copy(&snapshot, &|| false).is_err());
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"external edit"
    );
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
}

#[test]
fn placement_new_journal_refuses_foreign_existing_target_before_claiming_anything() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    std::fs::write(f.target.join("foreign"), b"retain").unwrap();
    assert!(f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .is_err());
    assert!(!f.control().exists());
    assert_eq!(std::fs::read(f.target.join("foreign")).unwrap(), b"retain");
}

#[test]
fn placement_target_and_control_links_or_replacements_are_not_adopted() {
    for kind in ["target", "control-link", "control-empty"] {
        let f = Fixture::new();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&snapshot);
        match kind {
            "target" => {
                std::fs::rename(&f.target, f.target.with_extension("retained")).unwrap();
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&f.target)
                    .unwrap();
            }
            "control-link" => {
                std::fs::write(f.target.with_extension("foreign"), b"retain").unwrap();
                symlink(f.target.with_extension("foreign"), f.control()).unwrap();
            }
            "control-empty" => {
                std::fs::write(f.control(), b"").unwrap();
                std::fs::set_permissions(f.control(), std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            f.bootstrap
                .pending_placement(&f.operation, &snapshot)
                .is_err(),
            "{kind}"
        );
        assert!(!f.target.join("meetings.sqlite3").exists());
    }
}

#[test]
fn placement_creation_gap_and_added_foreign_files_are_retained_not_adopted_or_removed() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    let file = f.target.join("meetings.sqlite3");
    std::fs::write(&file, b"unowned creation gap").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(p.copy(&snapshot, &|| false).is_err());
    assert_eq!(p.progress().unwrap().owned_paths, 0);
    drop(p);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert!(p.copy(&snapshot, &|| false).is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"unowned creation gap");
    assert!(f.bootstrap.cancel_move(&f.operation).is_err());
    assert!(f.bootstrap.view().unwrap().pending_operation_id.is_some());
}

#[test]
fn placement_owned_partial_file_replacement_or_links_refuse_before_payload_write() {
    for kind in ["replacement", "symlink", "hardlink"] {
        let f = Fixture::new();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&snapshot);
        let mut p = f
            .bootstrap
            .pending_placement(&f.operation, &snapshot)
            .unwrap();
        let db = f.target.join("meetings.sqlite3");
        assert!(p.copy(&snapshot, &|| db.exists()).is_err());
        drop(p);
        std::fs::rename(&db, f.target.with_extension("retained-owned")).unwrap();
        let foreign = f.target.with_extension("foreign-file");
        std::fs::write(&foreign, b"foreign payload").unwrap();
        match kind {
            "replacement" => std::fs::write(&db, b"foreign payload").unwrap(),
            "symlink" => symlink(&foreign, &db).unwrap(),
            "hardlink" => std::fs::hard_link(&foreign, &db).unwrap(),
            _ => unreachable!(),
        }
        let mut p = f
            .bootstrap
            .pending_placement(&f.operation, &snapshot)
            .unwrap();
        assert!(p.copy(&snapshot, &|| false).is_err(), "{kind}");
        assert_eq!(std::fs::read(db).unwrap(), b"foreign payload");
        assert_eq!(std::fs::read(foreign).unwrap(), b"foreign payload");
        assert_eq!(
            std::fs::read(f.storage.root().join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}

#[test]
fn placement_runtime_and_reopen_refuse_changed_control_shape_or_identity() {
    for kind in ["shape", "metadata", "file", "lock", "missing-lock"] {
        let f = Fixture::new();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&snapshot);
        let p = f
            .bootstrap
            .pending_placement(&f.operation, &snapshot)
            .unwrap();
        match kind {
            "shape" => p.db.execute_batch("DROP TRIGGER path_fixed;").unwrap(),
            "metadata" => {
                p.db.execute("UPDATE metadata SET state='ready'", [])
                    .map(|_| ())
                    .unwrap()
            }
            "file" => {
                std::fs::rename(f.control(), f.control().with_extension("retained")).unwrap();
                std::fs::write(f.control(), b"keep control replacement").unwrap();
            }
            "lock" => std::fs::write(&p.lock_path, b"keep lock replacement").unwrap(),
            "missing-lock" => {
                std::fs::rename(&p.lock_path, p.lock_path.with_extension("retained-lock")).unwrap()
            }
            _ => unreachable!(),
        }
        assert!(p.progress().is_err(), "{kind}");
        drop(p);
        assert!(
            f.bootstrap
                .pending_placement(&f.operation, &snapshot)
                .is_err(),
            "{kind}"
        );
        assert!(!f.target.join("meetings.sqlite3").exists());
    }
}

#[test]
fn placement_unknown_extra_destination_file_is_kept_and_verification_refuses_it() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    p.copy(&snapshot, &|| false).unwrap();
    let extra = f.target.join("meetings/foreign.part");
    std::fs::write(&extra, b"retain foreign").unwrap();
    assert!(p.verify(&snapshot, &|| false).is_err());
    assert!(p.copy(&snapshot, &|| false).is_err());
    assert_eq!(std::fs::read(extra).unwrap(), b"retain foreign");
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
}

#[test]
fn placement_same_operation_is_locked_and_wrong_operation_or_changed_source_cannot_resume() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert!(f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .is_err());
    assert!(f
        .bootstrap
        .pending_placement(&Uuid::new_v4().to_string(), &snapshot)
        .is_err());
    p.copy(&snapshot, &|| false).unwrap();
    drop(p);
    drop(snapshot);
    f.storage
        .rename_meeting(&f.meeting, "元DBの新しい編集", 0)
        .unwrap();
    let next = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(f.bootstrap.pending_placement(&f.operation, &next).is_err());
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(
        f.storage.get_meeting(&f.meeting).unwrap().unwrap().title,
        "元DBの新しい編集"
    );
}

#[test]
fn placement_owner_drop_releases_only_its_lock_with_a_retained_duplicate_descriptor() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    // A duplicated open file description models the descriptor inherited by a
    // concurrently spawning child before its close-on-exec boundary. It grants
    // no placement owner and performs no protected SQLite work.
    let retained = p.lock.try_clone().unwrap();
    assert!(f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .is_err());
    drop(p);
    let next = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert!(retained.metadata().unwrap().is_file());
    // Closing a stale duplicate must not release a separately acquired owner.
    drop(retained);
    assert!(f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .is_err());
    drop(next);
    assert!(f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .is_ok());
}

#[test]
fn placement_cannot_select_models_root_or_nested_model_directory() {
    for nested in [false, true] {
        let f = Fixture::new();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        let target = if nested {
            f.bootstrap.models_root.join("meeting-data")
        } else {
            f.bootstrap.models_root.clone()
        };
        std::fs::create_dir_all(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let original = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
        assert!(f
            .bootstrap
            .begin_move(
                &f.operation,
                &Uuid::new_v4().to_string(),
                &target,
                snapshot.manifest_hash(),
                0
            )
            .is_err());
        assert_eq!(
            std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
            original
        );
        assert!(!target.join("meetings.sqlite3").exists());
    }
}

#[test]
fn placement_source_database_substitution_keeps_write_stop_after_failed_copy_is_dropped() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    let db = f.storage.root().join("meetings.sqlite3");
    std::fs::rename(&db, db.with_extension("retained")).unwrap();
    std::fs::write(&db, b"unknown replacement").unwrap();
    assert!(p.copy(&snapshot, &|| false).is_err());
    drop(p);
    drop(snapshot);
    assert!(f
        .storage
        .create_meeting("旧接続の書込を再開しない")
        .is_err());
    assert_eq!(std::fs::read(db).unwrap(), b"unknown replacement");
    assert!(!f.target.join("meetings.sqlite3").exists());
}

#[test]
fn placement_restart_reopens_owned_partial_copy_and_preserves_current_old_root() {
    let f = Fixture::new();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&snapshot);
    let mut p = f
        .bootstrap
        .pending_placement(&f.operation, &snapshot)
        .unwrap();
    assert!(p
        .copy(&snapshot, &|| f.target.join("meetings.sqlite3").exists())
        .is_err());
    let config = f.bootstrap.config.clone();
    let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
    let expected = snapshot.manifest_hash().to_owned();
    let old = f.storage.root().to_owned();
    drop(p);
    drop(snapshot);
    drop(f.storage);
    drop(f.bootstrap);
    let b = Bootstrap::open(&config, &legacy).unwrap();
    assert_eq!(b.root(), old);
    let storage = Storage::open(b.root().into()).unwrap();
    b.initialize(&storage).unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert_eq!(
        snapshot.manifest_hash(),
        expected,
        "unchanged native source should retain its confirmed image"
    );
    let mut p = b.pending_placement(&f.operation, &snapshot).unwrap();
    assert_eq!(p.progress().unwrap().owned_paths, 1);
    assert_eq!(p.copy(&snapshot, &|| false).unwrap().state, "ready");
    assert_eq!(b.root(), old);
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
}

#[test]
fn placement_current_shared_app_parent_keeps_models_control_and_meeting() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let old = base.join("old");
    let b = Bootstrap::open(&old, &old).unwrap();
    let seed = b.open_storage().unwrap();
    let meeting = seed.create_meeting("共有親の現行会議").unwrap().id;
    let relative = format!("meetings/{meeting}/audio/source.flac");
    let file = seed.root().join(&relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"current audio").unwrap();
    seed.register_ready_meeting_asset(&meeting, Path::new(&relative), "audio/flac")
        .unwrap();
    std::fs::create_dir_all(old.join("models/package")).unwrap();
    std::fs::write(old.join("models/package/model.bin"), b"retain model").unwrap();
    let storage = seed;
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let target = base.join("target");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let operation = Uuid::new_v4().to_string();
    b.begin_move(
        &operation,
        &Uuid::new_v4().to_string(),
        &target,
        snapshot.manifest_hash(),
        0,
    )
    .unwrap();
    let mut p = b.pending_placement(&operation, &snapshot).unwrap();
    p.copy(&snapshot, &|| false).unwrap();
    assert_eq!(
        std::fs::read(old.join("models/package/model.bin")).unwrap(),
        b"retain model"
    );
    assert_eq!(
        std::fs::read(storage.root().join(&relative)).unwrap(),
        b"current audio"
    );
    assert!(!target.join("models").exists());
    assert!(!target.join(NAME).exists());
    assert_eq!(b.view().unwrap().meeting_root, old.join("data"));
    assert_eq!(b.view().unwrap().models_root, old.join("models"));
}

#[test]
fn placement_crash_child() {
    let Ok(raw) = std::env::var("MON_PLACEMENT_CRASH_CHILD") else {
        return;
    };
    let (config, legacy, operation, marker, mode): (PathBuf, PathBuf, String, PathBuf, String) =
        serde_json::from_str(&raw).unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = Storage::open(b.root().into()).unwrap();
    b.initialize(&storage).unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let mut p = b.pending_placement(&operation, &snapshot).unwrap();
    let target = p.pending.target.clone();
    let ready = || -> ! {
        let temp = marker.with_extension("tmp");
        std::fs::write(&temp, b"copy boundary").unwrap();
        File::open(&temp).unwrap().sync_all().unwrap();
        std::fs::rename(temp, &marker).unwrap();
        loop {
            std::thread::park();
        }
    };
    let inspect = || {
        if mode == "partial" && target.join("meetings.sqlite3").exists() {
            ready();
        }
        false
    };
    p.copy(&snapshot, &inspect).unwrap();
    ready();
}

#[test]
fn placement_sigkill_reopen_preserves_old_data_and_resumes_only_journal_owned_copy() {
    use std::process::{Command, Stdio};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for mode in ["partial", "ready"] {
        let f = Fixture::new();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&snapshot);
        let config = f.bootstrap.config.clone();
        let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
        let old = f.storage.root().to_owned();
        drop(snapshot);
        drop(f.storage);
        drop(f.bootstrap);
        let marker = f._base.path().join("child-ready");
        let mut child = Child(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "bootstrap::placement::tests::placement_crash_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(
                    "MON_PLACEMENT_CRASH_CHILD",
                    serde_json::to_string(&(&config, &legacy, &f.operation, &marker, mode))
                        .unwrap(),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !marker.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before {mode}"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "child never reached {mode}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let b = Bootstrap::open(&config, &legacy).unwrap();
        assert_eq!(b.root(), old);
        let storage = Storage::open(b.root().into()).unwrap();
        b.initialize(&storage).unwrap();
        let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
        let mut p = b.pending_placement(&f.operation, &snapshot).unwrap();
        let before = p.progress().unwrap();
        assert_eq!(
            before.state,
            if mode == "ready" { "ready" } else { "copying" }
        );
        let after = p.copy(&snapshot, &|| false).unwrap();
        assert_eq!(after.state, "ready");
        assert_eq!(after.expected_paths, after.verified_paths);
        assert_eq!(
            storage.get_meeting(&f.meeting).unwrap().unwrap().title,
            "配置しても同じ会議"
        );
        assert_eq!(
            std::fs::read(old.join(&f.relative)).unwrap(),
            b"original audio"
        );
        assert_eq!(
            std::fs::read(f.target.join(&f.relative)).unwrap(),
            b"original audio"
        );
        assert_eq!(b.view().unwrap().meeting_root, old);
    }
}
