use super::super::tests::Fixture;
use super::*;
use crate::storage::backup::recovery::{Outcome, Source};
use std::cell::Cell;
use std::os::unix::fs::{symlink, DirBuilderExt};

fn delete_audio(f: &Fixture) {
    use crate::{analysis_worker::AnalysisController, storage::deletion::audio};
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let operation = Uuid::new_v4().to_string();
    audio::intent::prepare(
        &f.storage,
        &operation,
        &audio::Census::capture(&f.storage, &f.meeting, &|| false).unwrap(),
        audio::intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &lease,
        &|| false,
    )
    .unwrap();
    audio::intent::removal::start(&f.storage, &operation, &lease, &|| false).unwrap();
    audio::intent::removal::run(&f.storage, &operation, &lease, &|| false).unwrap();
    audio::intent::removal::completion::finish(&f.storage, &operation, &lease, &|| false).unwrap();
}

#[test]
fn deletion_relocation_history_install_binds_only_owned_destination_without_rewriting_native_rows()
{
    for full in [false, true] {
        let f = Fixture::new();
        delete_audio(&f);
        if full {
            use crate::{analysis_worker::AnalysisController, storage::deletion::operations};
            let mut prepared =
                operations::Prepared::fresh(&f.storage, &f.meeting, &|| false).unwrap();
            let preview = prepared.preview();
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                prepared
                    .execute(
                        &f.storage,
                        &lease,
                        &preview.confirmation_id,
                        true,
                        &|| false,
                        &|_| {}
                    )
                    .unwrap()
                    .deleted
            );
        }
        let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &|| false).unwrap();
        let h = s.prepare_control_handoff(&|| false).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(h.bytes()).unwrap();
        let before = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
        let first = p.install_history(&h, &|| false).unwrap();
        assert_eq!(p.install_history(&h, &|| false).unwrap(), first);
        assert_eq!(
            std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
            before
        );
        let db = Connection::open(f.target.join(REGISTRY)).unwrap();
        let local: String = db
            .query_row(
                "SELECT local_json FROM relocation_history WHERE operation_id=?1",
                [&f.operation],
                |r| r.get(0),
            )
            .unwrap();
        let local: serde_json::Value = serde_json::from_str(&local).unwrap();
        assert_eq!(local["deletion_history"], envelope["deletion_history"]);
        let bound = &local["deletion_binding"];
        assert_eq!(bound["operation"], f.operation);
        assert_eq!(bound["targetRoot"], f.target.to_str().unwrap());
        assert_eq!(bound["nativeHash"], s.manifest_hash());
        assert_ne!(bound["root"], local["deletion_history"]["root"]);
        assert_ne!(bound["database"], local["deletion_history"]["database"]);
        let directories = bound["directories"].as_array().unwrap();
        assert_eq!(
            directories.len(),
            envelope["deletion_history"]["directories"]
                .as_array()
                .unwrap()
                .len()
        );
        assert!(directories.iter().any(|d| d["relativePath"] == "meetings"));
        for (copied, original) in directories.iter().zip(
            envelope["deletion_history"]["directories"]
                .as_array()
                .unwrap(),
        ) {
            assert_eq!(copied["relativePath"], original["relativePath"]);
            assert_eq!(copied["original"], original["identity"]);
            assert_ne!(copied["original"], copied["copied"]);
        }
        assert!(local.get("origins").is_none());
        assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
        p.verify(&s, &|| false).unwrap();
    }
}

#[test]
fn deletion_relocation_history_install_rejects_replaced_destination_parent_and_keeps_both_roots() {
    let f = Fixture::new();
    delete_audio(&f);
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    let directory = f.target.join("meetings");
    let original = f.target.join("retained-meetings");
    std::fs::rename(&directory, &original).unwrap();
    symlink(&original, &directory).unwrap();
    assert!(p.install_history(&h, &|| false).is_err());
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert!(original.exists());
    assert!(!f.storage.root().join(&f.relative).exists());
}

#[test]
fn deletion_relocation_cutover_waits_for_typed_receiver_before_retirement_or_selection() {
    let f = Fixture::new();
    delete_audio(&f);
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    p.install_history(&h, &|| false).unwrap();
    let before = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
    let confirmation = p.pending.confirmation_id.clone();
    let error = p
        .activate_for_restart(&h, &confirmation, 1, &|| false)
        .unwrap_err();
    assert!(error.contains("削除履歴を含む保存先の切替"));
    assert_eq!(
        std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
        before
    );
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    assert!(!f.bootstrap.view().unwrap().restart_required);
    assert!(!f.storage.restart_required());
    p.verify(&s, &|| false).unwrap();
    drop(p);
    drop(h);
    drop(s);
    f.storage.create_meeting("旧保存先で作業を継続").unwrap();
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
}

#[test]
fn restore_rebinding_v3_upgrade_preserves_installed_history_and_meeting_data() {
    let f = Fixture::new();
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    p.install_history(&h, &|| false).unwrap();
    drop(p);
    drop(h);
    drop(s);
    let db = Connection::open(f.target.join(REGISTRY)).unwrap();
    let original: String = db
        .query_row("SELECT local_json FROM relocation_history", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute_batch("DROP TRIGGER relocation_restore_binding_fixed; DROP TRIGGER relocation_restore_binding_retained; DROP TABLE relocation_restore_bindings; PRAGMA user_version=3;").unwrap();
    drop(db);
    let target = Storage::open(f.target.clone()).unwrap();
    assert_eq!(target.previous_storage_history().unwrap().len(), 1);
    assert!(target.get_meeting(&f.meeting).unwrap().is_some());
    let db = Connection::open(f.target.join(REGISTRY)).unwrap();
    assert_eq!(
        db.query_row("SELECT local_json FROM relocation_history", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        original
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
}

#[test]
fn history_install_preserves_rows_and_old_root_and_is_idempotent() {
    let f = Fixture::new();
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    let before = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    let receipt = p.install_history(&h, &|| false).unwrap();
    assert_eq!(receipt.state, "installed");
    assert_eq!(p.install_history(&h, &|| false).unwrap(), receipt);
    p.verify(&s, &|| false).unwrap();
    assert_eq!(p.copy(&s, &|| false).unwrap().state, "ready");
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(f.target.join(&f.relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    let db = Connection::open(f.target.join(REGISTRY)).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM relocation_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM areas", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn kept_scratch(f: &Fixture) -> (String, PathBuf) {
    let stage = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    let path = stage.image().parent().unwrap().to_owned();
    std::fs::write(path.join("unknown"), b"kept old-root body").unwrap();
    drop(stage);
    let id = f
        .storage
        .recovery_listing()
        .unwrap()
        .items
        .iter()
        .find(|i| i.source == Source::Scratch)
        .unwrap()
        .operation_id
        .clone();
    let mut confirm = f
        .storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    assert_eq!(
        confirm
            .resolve(
                &f.storage,
                &confirm.preview().confirmation_id,
                true,
                &|| false
            )
            .unwrap()
            .outcome,
        Outcome::Retained
    );
    (id, path)
}

#[test]
fn history_install_keeps_old_scratch_scope_on_target_startup_and_next_move_without_recursive_history(
) {
    let f = Fixture::new();
    let (_id, path) = kept_scratch(&f);
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    p.install_history(&h, &|| false).unwrap();
    drop(p);
    drop(h);
    drop(s);
    let target = Storage::open(f.target.clone()).unwrap();
    let old = target.previous_storage_history().unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].kept_scratch_areas, 1);
    assert_eq!(old[0].unresolved_scratch_areas, 1);
    assert_eq!(old[0].source_root, f.storage.root().to_str().unwrap());
    assert!(target.recovery_listing().unwrap().items.is_empty());
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"kept old-root body"
    );
    // A second real placement using a synthetic bootstrap record only: current
    // app-root cutover remains a separate, not-yet-exposed production operation.
    let bdir = f
        ._base
        .path()
        .canonicalize()
        .unwrap()
        .join("second-bootstrap");
    let b = Bootstrap::open(
        &bdir,
        &f._base.path().canonicalize().unwrap().join("second-data"),
    )
    .unwrap();
    let seed = Storage::open(b.root().into()).unwrap();
    b.initialize(&seed).unwrap();
    drop(seed);
    {
        let mut state = b.state.lock().unwrap();
        let mut record = state.loaded.as_ref().unwrap().record.clone();
        record.current = Root::inspect(&f.target).unwrap();
        b.write(&mut state, record).unwrap();
    }
    drop(b);
    let b = Bootstrap::open(
        &bdir,
        &f._base.path().canonicalize().unwrap().join("second-data"),
    )
    .unwrap();
    let next = f._base.path().canonicalize().unwrap().join("third-root");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&next)
        .unwrap();
    let op = Uuid::new_v4().to_string();
    let native = target.prepare_relocation_snapshot(&|| false).unwrap();
    b.begin_move(
        &op,
        &Uuid::new_v4().to_string(),
        &next,
        native.manifest_hash(),
        0,
    )
    .unwrap();
    let mut second = b.pending_placement(&op, &native).unwrap();
    second.copy(&native, &|| false).unwrap();
    let control = native.prepare_control_handoff(&|| false).unwrap();
    second.install_history(&control, &|| false).unwrap();
    let third = Storage::open(next.clone()).unwrap();
    assert_eq!(third.previous_storage_history().unwrap().len(), 2);
    let db = Connection::open(next.join(REGISTRY)).unwrap();
    let original: String = db
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            params![f.operation],
            |r| r.get(0),
        )
        .unwrap();
    let newest: String = db
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            params![op],
            |r| r.get(0),
        )
        .unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&original)
        .unwrap()
        .get("origins")
        .is_none());
    assert!(serde_json::from_str::<serde_json::Value>(&newest)
        .unwrap()
        .get("origins")
        .is_none());
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"kept old-root body"
    );
}

#[test]
fn history_install_cancel_after_owned_record_reopens_and_resumes_same_inode() {
    let f = Fixture::new();
    let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    let hit = Cell::new(false);
    let cancel = || {
        let exists = f.target.join(REGISTRY).exists();
        if exists {
            hit.set(true);
        }
        exists
    };
    assert!(p.install_history(&h, &cancel).is_err());
    assert!(hit.get());
    let identity =
        Identity::read(&std::fs::symlink_metadata(f.target.join(REGISTRY)).unwrap()).unwrap();
    drop(p);
    let p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    assert_eq!(p.install_history(&h, &|| false).unwrap().state, "installed");
    assert_eq!(
        Identity::read(&std::fs::symlink_metadata(f.target.join(REGISTRY)).unwrap()).unwrap(),
        identity
    );
    p.verify(&s, &|| false).unwrap();
}

#[test]
fn history_install_refuses_copied_registry_changes_or_owned_file_replacement_and_links() {
    for mode in 0..4 {
        let f = Fixture::new();
        let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &|| false).unwrap();
        let h = s.prepare_control_handoff(&|| false).unwrap();
        let file = f.target.join(REGISTRY);
        if mode == 0 {
            p.install_history(&h, &|| false).unwrap();
            std::fs::write(&file, b"edited registry").unwrap();
        } else {
            assert!(p.install_history(&h, &|| file.exists()).is_err());
        }
        let foreign = f._base.path().join("foreign");
        std::fs::write(&foreign, b"foreign registry").unwrap();
        match mode {
            1 => {
                std::fs::remove_file(&file).unwrap();
                std::fs::copy(&foreign, &file).unwrap();
            }
            2 => {
                std::fs::remove_file(&file).unwrap();
                symlink(&foreign, &file).unwrap();
            }
            3 => {
                std::fs::remove_file(&file).unwrap();
                std::fs::hard_link(&foreign, &file).unwrap();
            }
            _ => {}
        }
        assert!(p.install_history(&h, &|| false).is_err());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign registry");
        if mode == 0 {
            assert_eq!(std::fs::read(&file).unwrap(), b"edited registry");
        }
        assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    }
}

#[test]
fn history_install_cold_restart_continues_owned_or_resends_installed_without_rewriting_done_files()
{
    for finished in [false, true] {
        let f = Fixture::new();
        let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &|| false).unwrap();
        let h = s.prepare_control_handoff(&|| false).unwrap();
        if finished {
            p.install_history(&h, &|| false).unwrap();
        } else {
            assert!(p
                .install_history(&h, &|| f.target.join(REGISTRY).exists())
                .is_err());
        }
        let original = std::fs::read(f.target.join(REGISTRY)).unwrap();
        let config = f.bootstrap.config.clone();
        let app_data = f.bootstrap.models_root.parent().unwrap().to_owned();
        let old_root = f.storage.root().to_owned();
        drop(p);
        drop(h);
        drop(s);
        drop(f.storage);
        drop(f.bootstrap);
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let source = Storage::open(b.root().into()).unwrap();
        let native = source.prepare_relocation_snapshot(&|| false).unwrap();
        let control = native.prepare_control_handoff(&|| false).unwrap();
        let p = b.pending_placement(&f.operation, &native).unwrap();
        assert_eq!(
            p.install_history(&control, &|| false).unwrap().state,
            "installed"
        );
        p.verify(&native, &|| false).unwrap();
        if finished {
            assert_eq!(std::fs::read(f.target.join(REGISTRY)).unwrap(), original);
        }
        assert_eq!(b.view().unwrap().meeting_root, old_root);
        assert_eq!(
            std::fs::read(old_root.join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}

#[test]
fn history_install_refuses_unproven_journal_registry_creation_gap_and_control_metadata_tampering() {
    for mode in 0..4 {
        let f = Fixture::new();
        let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &|| false).unwrap();
        let h = s.prepare_control_handoff(&|| false).unwrap();
        let journal = f.target.join(JOURNAL);
        match mode {
            0 => std::fs::write(&journal, b"").unwrap(),
            1 => {
                drop(Local::open(&p, true).unwrap());
                std::fs::write(f.target.join(REGISTRY), b"creation gap, unclaimed").unwrap();
            }
            2 | 3 => {
                p.install_history(&h, &|| false).unwrap();
                let db = Connection::open(&journal).unwrap();
                if mode == 2 {
                    db.execute_batch("CREATE TABLE unexpected(value TEXT)")
                        .unwrap();
                } else {
                    let sql: String = db
                        .query_row(
                            "SELECT sql FROM sqlite_schema WHERE name='handoff_fixed'",
                            [],
                            |r| r.get(0),
                        )
                        .unwrap();
                    db.execute_batch("DROP TRIGGER handoff_fixed").unwrap();
                    db.execute("UPDATE handoff SET image_hash=?1", params!["0".repeat(64)])
                        .unwrap();
                    db.execute_batch(&sql).unwrap();
                }
            }
            _ => unreachable!(),
        }
        let before = std::fs::read(&journal).unwrap();
        assert!(p.install_history(&h, &|| false).is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), before);
        if mode == 1 {
            assert_eq!(
                std::fs::read(f.target.join(REGISTRY)).unwrap(),
                b"creation gap, unclaimed"
            );
        }
        assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    }
}

#[test]
fn history_install_legacy_shared_config_parent_keeps_models_and_control_at_source() {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path().canonicalize().unwrap();
    let shared = parent.join("shared");
    let source = Storage::open(shared.clone()).unwrap();
    source.create_meeting("legacy meeting").unwrap();
    drop(source);
    std::fs::create_dir(shared.join("models")).unwrap();
    std::fs::write(shared.join("models/model.gguf"), b"old model").unwrap();
    let b = Bootstrap::open(&shared, &shared).unwrap();
    let source = Storage::open(b.root().into()).unwrap();
    b.initialize(&source).unwrap();
    let target = parent.join("target");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let s = source.prepare_relocation_snapshot(&|| false).unwrap();
    let op = Uuid::new_v4().to_string();
    b.begin_move(
        &op,
        &Uuid::new_v4().to_string(),
        &target,
        s.manifest_hash(),
        0,
    )
    .unwrap();
    let mut p = b.pending_placement(&op, &s).unwrap();
    p.copy(&s, &|| false).unwrap();
    let h = s.prepare_control_handoff(&|| false).unwrap();
    p.install_history(&h, &|| false).unwrap();
    p.verify(&s, &|| false).unwrap();
    assert_eq!(
        std::fs::read(shared.join("models/model.gguf")).unwrap(),
        b"old model"
    );
    assert!(shared.join(NAME).exists());
    assert!(!target.join(NAME).exists());
    assert!(!target.join("models").exists());
    assert_eq!(b.view().unwrap().meeting_root, shared);
}

#[test]
fn history_install_crash_child() {
    let Ok(config) = std::env::var("MON_HISTORY_INSTALL_CONFIG") else {
        return;
    };
    let app_data = std::env::var("MON_HISTORY_INSTALL_APP_DATA").unwrap();
    let op = std::env::var("MON_HISTORY_INSTALL_OPERATION").unwrap();
    let marker = std::env::var("MON_HISTORY_INSTALL_MARKER").unwrap();
    let mode = std::env::var("MON_HISTORY_INSTALL_MODE").unwrap();
    let b = Bootstrap::open(Path::new(&config), Path::new(&app_data)).unwrap();
    let source = Storage::open(b.root().into()).unwrap();
    let native = source.prepare_relocation_snapshot(&|| false).unwrap();
    let mut p = b.pending_placement(&op, &native).unwrap();
    p.copy(&native, &|| false).unwrap();
    let h = native.prepare_control_handoff(&|| false).unwrap();
    let signal = || -> ! {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .unwrap();
        file.write_all(b"ready").unwrap();
        file.sync_all().unwrap();
        loop {
            std::thread::park();
        }
    };
    if mode == "partial" {
        let _ = p.install_history(&h, &|| {
            if p.pending.target.join(REGISTRY).exists() {
                signal();
            }
            false
        });
        unreachable!();
    }
    p.install_history(&h, &|| false).unwrap();
    signal();
}

#[test]
fn history_install_sigkill_preserves_old_root_and_resumes_owned_or_resends_installed_history() {
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for mode in ["partial", "installed"] {
        let f = Fixture::new();
        let s = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        f.intent(&s);
        drop(s);
        let config = f.bootstrap.config.clone();
        let app_data = f.bootstrap.models_root.parent().unwrap().to_owned();
        let root = f.storage.root().to_owned();
        drop(f.storage);
        drop(f.bootstrap);
        let marker = f._base.path().join("child-ready");
        let mut child = Reap(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "bootstrap::placement::history::tests::history_install_crash_child",
                    "--nocapture",
                ])
                .env("MON_HISTORY_INSTALL_CONFIG", &config)
                .env("MON_HISTORY_INSTALL_APP_DATA", &app_data)
                .env("MON_HISTORY_INSTALL_OPERATION", &f.operation)
                .env("MON_HISTORY_INSTALL_MARKER", &marker)
                .env("MON_HISTORY_INSTALL_MODE", mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while !marker.exists() {
            if let Some(status) = child.0.try_wait().unwrap() {
                panic!("history install child ended before marker: {status}");
            }
            assert!(start.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let before = std::fs::read(f.target.join(REGISTRY)).unwrap();
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let source = Storage::open(b.root().into()).unwrap();
        let native = source.prepare_relocation_snapshot(&|| false).unwrap();
        let h = native.prepare_control_handoff(&|| false).unwrap();
        let p = b.pending_placement(&f.operation, &native).unwrap();
        p.install_history(&h, &|| false).unwrap();
        p.verify(&native, &|| false).unwrap();
        if mode == "installed" {
            assert_eq!(std::fs::read(f.target.join(REGISTRY)).unwrap(), before);
        }
        assert_eq!(b.view().unwrap().meeting_root, root);
        assert_eq!(
            std::fs::read(root.join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}
