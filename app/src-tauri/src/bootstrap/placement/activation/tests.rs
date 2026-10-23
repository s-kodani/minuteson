use super::super::tests::Fixture;
use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

mod native_history;

fn select(f: &Fixture) {
    let s = f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    let confirmation = p.pending.confirmation_id.clone();
    p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
        .unwrap();
}

#[test]
fn activation_saved_manifest_requires_bounded_owned_paths_and_strict_native_fields() {
    let f = Fixture::new();
    let s = f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    validate_manifest(s.manifest()).unwrap();
    for failure in [
        "version",
        "database-size",
        "escape",
        "duplicate",
        "asset-meeting",
        "table-sha",
    ] {
        let mut m = s.manifest().clone();
        match failure {
            "version" => m.version = 99,
            "database-size" => m.database_size = 0,
            "escape" => m.entries[0].relative_path = "../escape".into(),
            "duplicate" => m.entries.push(m.entries[0].clone()),
            "asset-meeting" => m.assets[0].meeting_id = "other-meeting".into(),
            "table-sha" => m.tables[0].hash = "A".repeat(64),
            _ => unreachable!(),
        }
        assert!(validate_manifest(&m).is_err(), "{failure}");
    }
    let mut json = serde_json::to_value(s.manifest()).unwrap();
    json["verified"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Manifest>(json).is_err());
}

#[test]
fn activation_modified_source_or_target_refuses_before_selection_and_keeps_old_storage() {
    for failure in [
        "source-audio",
        "target-audio",
        "unknown",
        "registry",
        "database",
    ] {
        let f = Fixture::new();
        let s = f
            .storage
            .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
            .unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &crate::test_support::never_cancelled).unwrap();
        let h = s
            .prepare_control_handoff(&crate::test_support::never_cancelled)
            .unwrap();
        p.install_history(&h, &crate::test_support::never_cancelled)
            .unwrap();
        match failure {
            "source-audio" => {
                std::fs::write(f.storage.root().join(&f.relative), b"changed source").unwrap()
            }
            "target-audio" => {
                std::fs::write(f.target.join(&f.relative), b"changed target").unwrap()
            }
            "unknown" => std::fs::write(f.target.join("unknown"), b"do not adopt").unwrap(),
            "registry" => {
                let db = Connection::open(f.target.join(".mon-scratch.sqlite3")).unwrap();
                db.execute_batch("CREATE TABLE unknown(value TEXT)")
                    .unwrap();
            }
            "database" => {
                let db = Connection::open(f.target.join("meetings.sqlite3")).unwrap();
                db.execute("UPDATE meetings SET title='changed target'", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
        let confirmation = p.pending.confirmation_id.clone();
        assert!(
            p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
                .is_err(),
            "{failure}"
        );
        assert_eq!(
            std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
            before
        );
        assert!(!f.bootstrap.view().unwrap().restart_required);
        drop(p);
        drop(h);
        drop(s);
        f.storage.create_meeting("旧保存先を維持").unwrap();
    }
}

#[test]
fn activation_missing_owner_lock_history_or_invalid_target_refuses_cold_start_without_fallback() {
    for failure in [
        "owner",
        "lock",
        "handoff",
        "registry",
        "zero-db",
        "directory",
    ] {
        let f = Fixture::new();
        select(&f);
        let config = f.bootstrap.config.clone();
        let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
        let source = f.storage.root().to_owned();
        drop(f.storage);
        drop(f.bootstrap);
        let damaged = match failure {
            "owner" => config.join(format!(".relocation-{}.sqlite3", f.operation)),
            "lock" => config.join(format!(".relocation-{}.lock", f.operation)),
            "handoff" => f.target.join(".mon-handoff.sqlite3"),
            "registry" => f.target.join(".mon-scratch.sqlite3"),
            "zero-db" => f.target.join("meetings.sqlite3"),
            "directory" => f.target.clone(),
            _ => unreachable!(),
        };
        if failure == "zero-db" {
            std::fs::write(&damaged, []).unwrap();
        } else {
            std::fs::rename(&damaged, damaged.with_extension("retained")).unwrap();
        }
        let before = std::fs::read(config.join(NAME)).unwrap();
        assert!(Bootstrap::open(&config, &legacy).is_err(), "{failure}");
        assert_eq!(std::fs::read(config.join(NAME)).unwrap(), before);
        if failure != "zero-db" {
            assert!(
                !damaged.exists(),
                "missing targets are not recreated: {failure}"
            );
        } else {
            assert_eq!(damaged.metadata().unwrap().len(), 0);
        }
        assert_eq!(
            std::fs::read(source.join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}

#[test]
fn activation_database_copy_rebinds_but_changed_ownership_ledgers_are_rejected() {
    for failure in ["db", "registry", "table", "admission"] {
        let f = Fixture::new();
        select(&f);
        let config = f.bootstrap.config.clone();
        let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
        drop(f.storage);
        drop(f.bootstrap);
        drop(Bootstrap::open(&config, &legacy).unwrap());
        let owner = config.join(format!(".relocation-{}.sqlite3", f.operation));
        if matches!(failure, "db" | "registry") {
            let path = f.target.join(if failure == "db" {
                "meetings.sqlite3"
            } else {
                ".mon-scratch.sqlite3"
            });
            let retained = path.with_extension("retained");
            std::fs::rename(&path, &retained).unwrap();
            std::fs::copy(&retained, &path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        } else {
            let db = Connection::open(&owner).unwrap();
            db.execute_batch("DROP TRIGGER admission_fixed; DROP TRIGGER admission_retained;")
                .unwrap();
            if failure == "table" {
                db.execute_batch("DROP TABLE admission;").unwrap();
            } else {
                db.execute("UPDATE admission SET control_hash=?1", ["0".repeat(64)])
                    .unwrap();
                let expected = Connection::open_in_memory().unwrap();
                expected.execute_batch(ADMISSION_SCHEMA).unwrap();
                for (kind, _, sql) in schema(&expected).unwrap() {
                    if kind == "trigger" {
                        db.execute_batch(&sql).unwrap();
                    }
                }
            }
        }
        let before = std::fs::read(owner).unwrap();
        if failure == "db" {
            let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
            assert_eq!(bootstrap.root(), f.target);
            assert_eq!(
                bootstrap.recovery_view().unwrap().unwrap().kind,
                "copy_restored"
            );
            assert!(!bootstrap.open_storage().unwrap().data_write_blocked());
        } else {
            assert!(Bootstrap::open(&config, &legacy).is_err(), "{failure}");
        }
        assert_eq!(
            std::fs::read(config.join(format!(".relocation-{}.sqlite3", f.operation))).unwrap(),
            before
        );
    }
}

#[test]
fn activation_old_placement_ledger_is_rejected_without_rewriting_any_copy_proofs() {
    let f = Fixture::new();
    let s = f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    let owner = p.path.clone();
    drop(p);
    let db = Connection::open(&owner).unwrap();
    db.execute_batch("DROP TRIGGER admission_fixed; DROP TRIGGER admission_retained; DROP TABLE admission; PRAGMA user_version=1;").unwrap();
    drop(db);
    let ledger = std::fs::read(&owner).unwrap();
    let bootstrap = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
    let source = std::fs::read(f.storage.root().join(&f.relative)).unwrap();
    let target = std::fs::read(f.target.join(&f.relative)).unwrap();
    assert!(f.bootstrap.pending_placement(&f.operation, &s).is_err());
    assert_eq!(std::fs::read(&owner).unwrap(), ledger);
    assert_eq!(
        std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
        bootstrap
    );
    assert_eq!(
        std::fs::read(f.storage.root().join(&f.relative)).unwrap(),
        source
    );
    assert_eq!(std::fs::read(f.target.join(&f.relative)).unwrap(), target);
    let db = Connection::open(&owner).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='admission')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn activation_current_shared_app_parent_preserves_models_and_lost_response_replay() {
    let f = Fixture::new();
    let parent = f._base.path().canonicalize().unwrap();
    drop(f.storage);
    drop(f.bootstrap);
    let root = parent.join("current-shared");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let b = Bootstrap::open(&root, &root).unwrap();
    let storage = b.open_storage().unwrap();
    let meeting = storage.create_meeting("shared source").unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(root.join("models/model.bin"), b"kept model").unwrap();
    let target = parent.join("shared-target");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let s = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    b.begin_move(&operation, &confirmation, &target, s.manifest_hash(), 0)
        .unwrap();
    let mut p = b.pending_placement(&operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
            .unwrap()
            .revision,
        2
    );
    drop(p);
    drop(h);
    drop(s);
    drop(storage);
    drop(b);
    let b = Bootstrap::open(&root, &root).unwrap();
    let storage = b.open_storage().unwrap();
    assert!(storage.get_meeting(&meeting.id).unwrap().is_some());
    assert_eq!(
        std::fs::read(root.join("models/model.bin")).unwrap(),
        b"kept model"
    );
    assert!(!target.join("models").exists());
}

#[test]
fn activation_second_move_uses_current_admitted_root_without_restoring_stale_source() {
    let f = Fixture::new();
    select(&f);
    let config = f.bootstrap.config.clone();
    let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
    let old = f.storage.root().to_owned();
    drop(f.storage);
    drop(f.bootstrap);
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let added = storage.create_meeting("first target edit").unwrap();
    let target = f._base.path().canonicalize().unwrap().join("second-target");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let s = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    b.begin_move(&operation, &confirmation, &target, s.manifest_hash(), 2)
        .unwrap();
    let mut p = b.pending_placement(&operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    p.activate_for_restart(&h, &confirmation, 3, &crate::test_support::never_cancelled)
        .unwrap();
    drop(p);
    drop(h);
    drop(s);
    drop(storage);
    drop(b);
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    assert_eq!(b.root(), target);
    assert!(storage.get_meeting(&added.id).unwrap().is_some());
    assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(storage.previous_storage_history().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(old.join(&f.relative)).unwrap(),
        b"original audio"
    );
}

#[test]
fn activation_bootstrap_does_not_accept_a_schema_two_relocated_success_without_owned_activation() {
    let f = Fixture::new();
    let config = f.bootstrap.config.clone();
    let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
    drop(f.storage);
    drop(f.bootstrap);
    let mut d: Document =
        serde_json::from_slice(&std::fs::read(config.join(NAME)).unwrap()).unwrap();
    d.record.origin = Origin::Relocated;
    d.sha256 = hash(&serde_json::to_vec(&d.record).unwrap());
    std::fs::write(config.join(NAME), serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(Bootstrap::open(&config, &legacy).is_err());
}

#[test]
fn activation_publication_failure_before_or_after_rename_never_releases_old_writes() {
    for fault in ["before_selection", "after_rename"] {
        let f = Fixture::new();
        let config = f.bootstrap.config.clone();
        let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
        let source = f.storage.root().to_owned();
        let s = f
            .storage
            .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
            .unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &crate::test_support::never_cancelled).unwrap();
        let h = s
            .prepare_control_handoff(&crate::test_support::never_cancelled)
            .unwrap();
        p.install_history(&h, &crate::test_support::never_cancelled)
            .unwrap();
        let confirmation = p.pending.confirmation_id.clone();
        FAULT.with(|f| f.set(Some(fault)));
        let result =
            p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled);
        FAULT.with(|f| f.set(None));
        assert!(result
            .err()
            .unwrap()
            .contains("injected activation failure"));
        assert!(f.bootstrap.restart_required.load(Ordering::Acquire));
        if fault == "before_selection" {
            assert!(f.bootstrap.view().unwrap().restart_required);
        } else {
            assert!(f.bootstrap.view().is_err());
        }
        drop(p);
        drop(h);
        drop(s);
        assert!(f
            .storage
            .create_meeting("失敗でも旧Storageは再開しない")
            .is_err());
        drop(f.storage);
        drop(f.bootstrap);
        let b = Bootstrap::open(&config, &legacy).unwrap();
        assert_eq!(
            b.root(),
            if fault == "before_selection" {
                &source
            } else {
                &f.target
            }
        );
        let storage = b.open_storage().unwrap();
        assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
        assert_eq!(
            std::fs::read(source.join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}

#[test]
#[ignore = "subprocess fixture; invoked by the SIGKILL boundary test"]
fn activation_cutover_child() {
    let (config, legacy, target, operation, confirmation): (
        PathBuf,
        PathBuf,
        PathBuf,
        String,
        String,
    ) = serde_json::from_str(&std::env::var("MON_ACTIVATION_TEST_PATHS").unwrap()).unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let s = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    b.begin_move(&operation, &confirmation, &target, s.manifest_hash(), 0)
        .unwrap();
    let mut p = b.pending_placement(&operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
        .unwrap();
    drop(p);
    drop(h);
    drop(s);
    drop(storage);
    drop(b);
    let _b = Bootstrap::open(&config, &legacy).unwrap();
    panic!("configured activation checkpoint was not reached");
}

struct ChildGuard {
    child: std::process::Child,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn activation_actual_sigkill_before_selection_after_selection_and_after_admission_restarts_safely()
{
    use std::io::BufRead;
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "before_selection",
        "after_selection",
        "after_admission",
        "after_selection_tamper",
    ] {
        let f = Fixture::new();
        let config = f.bootstrap.config.clone();
        let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
        let source = f.storage.root().to_owned();
        drop(f.storage);
        drop(f.bootstrap);
        let checkpoint = if phase == "after_selection_tamper" {
            "after_selection"
        } else {
            phase
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bootstrap::placement::activation::tests::activation_cutover_child",
                "--ignored",
                "--nocapture",
            ])
            .env(
                "MON_ACTIVATION_TEST_PATHS",
                serde_json::to_string(&(
                    &config,
                    &legacy,
                    &f.target,
                    &f.operation,
                    Uuid::new_v4().to_string(),
                ))
                .unwrap(),
            )
            .env("MON_ACTIVATION_TEST_CHECKPOINT", checkpoint)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                match line {
                    Ok(line) if line.starts_with("MON_ACTIVATION_READY=") => {
                        let _ = tx.send(Ok(line));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                    _ => {}
                }
            }
            let _ = tx.send(Err("child exited before checkpoint".into()));
        });
        let mut child = ChildGuard {
            child,
            reader: Some(reader),
        };
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(30))
                .unwrap()
                .unwrap(),
            format!("MON_ACTIVATION_READY={checkpoint}")
        );
        child.child.kill().unwrap();
        assert_eq!(child.child.wait().unwrap().signal(), Some(9));
        if let Some(reader) = child.reader.take() {
            reader.join().unwrap();
        }
        drop(child);
        let document: Document =
            serde_json::from_slice(&std::fs::read(config.join(NAME)).unwrap()).unwrap();
        assert_eq!(
            document.record.current.path,
            if phase == "before_selection" {
                source.clone()
            } else {
                f.target.clone()
            }
        );
        let owner =
            Connection::open(config.join(format!(".relocation-{}.sqlite3", f.operation))).unwrap();
        assert_eq!(
            owner
                .query_row("SELECT count(*) FROM admission", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            i64::from(phase == "after_admission")
        );
        drop(owner);
        if phase == "after_selection_tamper" {
            let before = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
            std::fs::write(f.target.join(&f.relative), b"altered after kill").unwrap();
            assert!(Bootstrap::open(&config, &legacy).is_err());
            assert_eq!(
                std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
                before
            );
            assert_eq!(
                std::fs::read(source.join(&f.relative)).unwrap(),
                b"original audio"
            );
            continue;
        }
        let b = Bootstrap::open(&config, &legacy).unwrap();
        let storage = b.open_storage().unwrap();
        if phase == "before_selection" {
            // Resume the interrupted, still-pending operation through new source
            // staging and the installed compatible history, not a fake receipt.
            let pending = document.record.pending_move.as_ref().unwrap();
            let s = storage
                .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
                .unwrap();
            let mut p = b.pending_placement(&f.operation, &s).unwrap();
            p.copy(&s, &crate::test_support::never_cancelled).unwrap();
            let h = s
                .prepare_control_handoff(&crate::test_support::never_cancelled)
                .unwrap();
            p.install_history(&h, &crate::test_support::never_cancelled)
                .unwrap();
            p.activate_for_restart(
                &h,
                &pending.confirmation_id,
                1,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            drop(p);
            drop(h);
            drop(s);
            drop(storage);
            drop(b);
            let b = Bootstrap::open(&config, &legacy).unwrap();
            let storage = b.open_storage().unwrap();
            assert_eq!(b.root(), f.target);
            assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
        } else {
            assert_eq!(b.root(), f.target);
            assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
            storage.create_meeting("承認後の通常書込み").unwrap();
        }
        assert_eq!(
            std::fs::read(source.join(&f.relative)).unwrap(),
            b"original audio"
        );
    }
}

#[test]
fn activation_selection_freezes_old_storage_then_cold_start_admits_same_owned_meetings() {
    let f = Fixture::new();
    let source = f.storage.root().to_owned();
    let config = f.bootstrap.config.clone();
    let models = f.bootstrap.models_root.clone();
    let legacy = models.parent().unwrap().to_owned();
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(models.join("model.bin"), b"keep model").unwrap();
    let s = f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    let confirmation = p.pending.confirmation_id.clone();
    let view = p
        .activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(view.meeting_root, f.target);
    assert_eq!(view.active_meeting_root, source);
    assert!(view.restart_required);
    assert_eq!(view.revision, 2);
    assert_eq!(view.origin, "relocated");
    let bytes = std::fs::read(config.join(NAME)).unwrap();
    assert_eq!(
        p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
            .unwrap()
            .revision,
        2
    );
    assert_eq!(std::fs::read(config.join(NAME)).unwrap(), bytes);
    assert!(p.copy(&s, &crate::test_support::never_cancelled).is_err());
    assert!(p
        .install_history(&h, &crate::test_support::never_cancelled)
        .is_err());
    assert!(f.bootstrap.initialize(&f.storage).is_err());
    drop(p);
    drop(h);
    drop(s);
    assert!(f.storage.create_meeting("旧Storageへ書かない").is_err());
    f.storage.pause_for_exit().unwrap();
    let target = f.target.clone();
    let meeting = f.meeting.clone();
    let relative = f.relative.clone();
    drop(f.storage);
    drop(f.bootstrap);
    // Cold startup must not need any live source snapshot/root/SQLite handle.
    let retained = source.with_extension("retained");
    std::fs::rename(&source, &retained).unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    assert_eq!(b.root(), target);
    let db = Connection::open(config.join(format!(".relocation-{}.sqlite3", f.operation))).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM admission", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    let storage = b.open_storage().unwrap();
    assert!(storage.get_meeting(&meeting).unwrap().is_some());
    assert!(!b.view().unwrap().restart_required);
    assert_eq!(b.view().unwrap().active_meeting_root, target);
    storage.create_meeting("切替後の通常編集").unwrap();
    assert_eq!(storage.previous_storage_history().unwrap().len(), 1);
    drop(storage);
    drop(b);
    // An admitted, normally modified DB/registry is not rechecked against the
    // old payload SHA; its ownership and original admission remain required.
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
    assert_eq!(
        std::fs::read(retained.join(&relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(
        std::fs::read(models.join("model.bin")).unwrap(),
        b"keep model"
    );
    assert!(!source.exists());
}

#[test]
fn activation_wrong_confirmation_revision_uninstalled_history_or_cancellation_preserves_old_selection(
) {
    for failure in ["confirmation", "revision", "history", "cancel"] {
        let f = Fixture::new();
        let s = f
            .storage
            .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
            .unwrap();
        f.intent(&s);
        let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
        p.copy(&s, &crate::test_support::never_cancelled).unwrap();
        let h = s
            .prepare_control_handoff(&crate::test_support::never_cancelled)
            .unwrap();
        if failure != "history" {
            p.install_history(&h, &crate::test_support::never_cancelled)
                .unwrap();
        }
        let before = std::fs::read(f.bootstrap.config.join(NAME)).unwrap();
        let confirmation = if failure == "confirmation" {
            Uuid::new_v4().to_string()
        } else {
            p.pending.confirmation_id.clone()
        };
        assert!(
            p.activate_for_restart(
                &h,
                &confirmation,
                if failure == "revision" { 0 } else { 1 },
                &|| failure == "cancel"
            )
            .is_err(),
            "{failure}"
        );
        assert_eq!(
            std::fs::read(f.bootstrap.config.join(NAME)).unwrap(),
            before
        );
        assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
        assert!(!f.bootstrap.view().unwrap().restart_required);
        drop(p);
        drop(h);
        drop(s);
        f.storage.create_meeting("拒否後の旧保存先").unwrap();
    }
}

#[test]
fn activation_first_launch_rejects_audio_tamper_without_opening_or_mutating_target_database() {
    let f = Fixture::new();
    let config = f.bootstrap.config.clone();
    let legacy = f.bootstrap.models_root.parent().unwrap().to_owned();
    let s = f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    f.intent(&s);
    let mut p = f.bootstrap.pending_placement(&f.operation, &s).unwrap();
    p.copy(&s, &crate::test_support::never_cancelled).unwrap();
    let h = s
        .prepare_control_handoff(&crate::test_support::never_cancelled)
        .unwrap();
    p.install_history(&h, &crate::test_support::never_cancelled)
        .unwrap();
    let confirmation = p.pending.confirmation_id.clone();
    p.activate_for_restart(&h, &confirmation, 1, &crate::test_support::never_cancelled)
        .unwrap();
    drop(p);
    drop(h);
    drop(s);
    drop(f.storage);
    drop(f.bootstrap);
    let database = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    let bootstrap = std::fs::read(config.join(NAME)).unwrap();
    std::fs::write(f.target.join(&f.relative), b"altered audio").unwrap();
    assert!(Bootstrap::open(&config, &legacy).is_err());
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        database
    );
    assert_eq!(std::fs::read(config.join(NAME)).unwrap(), bootstrap);
    let owner =
        Connection::open(config.join(format!(".relocation-{}.sqlite3", f.operation))).unwrap();
    assert_eq!(
        owner
            .query_row("SELECT count(*) FROM admission", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
