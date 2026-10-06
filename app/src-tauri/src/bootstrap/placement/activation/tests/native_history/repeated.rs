//! Repeated moves through held snapshots, real placement ownership and cold
//! bootstrap admission. All data is synthetic; the public cutover stays gated.
use super::*;
use crate::storage::deletion::relocation::receiver::{
    synthetic_consent_record, synthetic_keep_meeting, synthetic_restored_audio_operation,
};

fn target(f: &NativeFixture) -> PathBuf {
    let path = f._base.path().join(format!("move-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    path.canonicalize().unwrap()
}

fn select_current(b: &Bootstrap, storage: &Storage, target: &Path) -> serde_json::Value {
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let operation = Uuid::new_v4().to_string();
    b.begin_move(
        &operation,
        &Uuid::new_v4().to_string(),
        target,
        snapshot.manifest_hash(),
        b.view().unwrap().revision,
    )
    .unwrap();
    let mut p = b.pending_placement(&operation, &snapshot).unwrap();
    p.copy(&snapshot, &|| false).unwrap();
    let h = snapshot.prepare_control_handoff(&|| false).unwrap();
    let control: serde_json::Value = serde_json::from_slice(h.bytes()).unwrap();
    p.install_history(&h, &|| false).unwrap();
    p.inspect_deletion_handoff(&h, &|| false)
        .unwrap()
        .require_clean()
        .unwrap();
    assert!(h.require_supported_cutover().is_err());
    let (_, control_hash, _) = history::installed_control(&p).unwrap();
    let mut record = b
        .state
        .lock()
        .unwrap()
        .loaded
        .as_ref()
        .unwrap()
        .record
        .clone();
    record.current = Root::inspect(target).unwrap();
    record.origin = Origin::Relocated;
    record.pending_move = None;
    record.activation = Some(Activation {
        pending: p.pending.clone(),
        control_hash,
    });
    record.revision += 1;
    snapshot.retire_source_for_restart(&|| false).unwrap();
    b.write(&mut b.state.lock().unwrap(), record).unwrap();
    control
}

fn record<'a>(control: &'a serde_json::Value, operation: &str) -> &'a serde_json::Value {
    control["deletion_history"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["operationId"] == operation)
        .unwrap()
}

fn flat(control: &serde_json::Value, original_operation: &str, prior_moves: usize) {
    assert_eq!(control["version"], 4);
    assert_eq!(control["deletion_history"]["version"], 3);
    assert_eq!(
        record(control, original_operation)["historicalRoots"]
            .as_array()
            .unwrap()
            .len(),
        prior_moves
    );
    let origins = control["history"]["origins"].as_array().unwrap();
    assert_eq!(origins.len(), prior_moves);
    for origin in origins {
        let local: serde_json::Value =
            serde_json::from_str(origin["local_json"].as_str().unwrap()).unwrap();
        assert!(local.get("origins").is_none());
    }
}

fn delete_current(storage: &Storage) -> String {
    let meeting = storage.create_meeting("CURRENT ROOT ONLY").unwrap();
    let mut p = operations::Prepared::fresh(storage, &meeting.id, &|| false).unwrap();
    let operation = p.preview().operation_id;
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(
        p.execute(
            storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &|| false,
            &|_| {}
        )
        .unwrap()
        .deleted
    );
    operation
}

#[test]
fn deletion_repeat_moves_completed_audio_mixed_current_receipts_and_flat_origins() {
    let mut f = NativeFixture::new(false, false);
    let original = f.live_record(f.storage.as_ref().unwrap());
    f.select();
    let mut current = f.target.clone();
    let mut additional = Vec::new();
    for prior_moves in 1..=2 {
        let (b, storage) = f.open();
        assert_eq!(storage.root(), current);
        assert!(!storage.data_write_blocked());
        assert_eq!(f.live_record(&storage), original);
        assert!(storage.recorded_audio_plan(&f.meeting).is_err());
        let new = delete_current(&storage);
        additional.push((new.clone(), synthetic_consent_record(&storage, &new)));
        let next = target(&f);
        let control = select_current(&b, &storage, &next);
        flat(&control, &f.deletion, prior_moves);
        // A new receipt from this actual root does not inherit any older root.
        assert!(record(&control, &new).get("historicalRoots").is_none());
        assert!(control["deletion_history"]["historicalDirectories"]
            .as_array()
            .is_some_and(|dirs| !dirs.is_empty()));
        drop(storage);
        drop(b);
        current = next;
    }
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert_eq!(storage.root(), current);
        assert!(!storage.data_write_blocked());
        assert_eq!(f.live_record(&storage), original);
        for (operation, before) in &additional {
            assert_eq!(synthetic_consent_record(&storage, operation), *before);
        }
        assert!(!current.join(&f.audio).exists());
        assert_eq!(
            std::fs::read(
                current
                    .join(&f.audio)
                    .parent()
                    .unwrap()
                    .join("retained.txt")
            )
            .unwrap(),
            b"SYNTHETIC TEXT"
        );
        let backup = f._base.path().join(format!("{}.monbackup", Uuid::new_v4()));
        assert_eq!(
            storage
                .create_backup_archive(&backup, &|| false)
                .unwrap()
                .deleted_audio_meeting_count,
            1
        );
        drop(storage);
        drop(b);
    }
}

#[test]
fn deletion_repeat_moves_kept_audio_and_whole_meeting_never_become_current_consent() {
    for whole in [false, true] {
        let mut f = NativeFixture::new(true, false);
        let mut former_roots = vec![f.storage.as_ref().unwrap().root().to_path_buf()];
        let kept = whole.then(|| synthetic_keep_meeting(f.storage.as_ref().unwrap(), &f.meeting));
        let old = f.live_record(f.storage.as_ref().unwrap());
        let old_whole = kept
            .as_ref()
            .map(|op| synthetic_consent_record(f.storage.as_ref().unwrap(), op));
        f.select();
        let mut current = f.target.clone();
        for prior_moves in 1..=2 {
            let (b, storage) = f.open();
            assert_eq!(storage.root(), current);
            assert!(!storage.data_write_blocked());
            let next = target(&f);
            flat(
                &select_current(&b, &storage, &next),
                &f.deletion,
                prior_moves,
            );
            former_roots.push(current);
            drop(storage);
            drop(b);
            current = next;
        }
        let (_b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(f.live_record(&storage), old);
        assert_eq!(
            std::fs::read(current.join(&f.audio)).unwrap(),
            b"SYNTHETIC AUDIO"
        );
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        assert!(audio::intent::removal::start(&storage, &f.deletion, &lease, &|| false).is_err());
        if let Some(op) = kept {
            let mut p = operations::Prepared::pending(&storage, &op, &|| false).unwrap();
            assert!(!p.preview().execute_allowed);
            assert!(p
                .execute(
                    &storage,
                    &lease,
                    &p.preview().confirmation_id,
                    true,
                    &|| false,
                    &|_| {}
                )
                .is_err());
            assert_eq!(synthetic_consent_record(&storage, &op), old_whole.unwrap());
        }
        assert!(storage.create_meeting("UNCHANGED KEEP").is_ok());
        // Every former root's retained body remains untouched as well.
        for root in former_roots {
            assert_eq!(
                std::fs::read(root.join(&f.audio)).unwrap(),
                b"SYNTHETIC AUDIO"
            );
        }
    }
}

#[test]
fn deletion_repeat_moves_full_supersession_and_new_current_consent_preserve_all_generations() {
    use crate::storage::deletion::relocation::receiver::synthetic_meeting_reconfirmation_records;
    let mut f = NativeFixture::new(false, true);
    let mut former_roots = vec![f.storage.as_ref().unwrap().root().to_path_buf()];
    let op = f.full_deletion.as_ref().unwrap().clone();
    let old = synthetic_consent_record(f.storage.as_ref().unwrap(), &op);
    let old_audio = f.live_record(f.storage.as_ref().unwrap());
    f.select();
    let mut current = f.target.clone();
    let mut retained = Vec::new();
    let mut recorded = Vec::new();
    for prior_moves in 1..=2 {
        // Admit the immutable placement first, then reappear at this root.
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        drop(storage);
        drop(b);
        let relative = format!("meetings/{}/reappeared.bin", f.meeting);
        let body = current.join(&relative);
        std::fs::create_dir_all(body.parent().unwrap()).unwrap();
        std::fs::write(&body, b"CURRENT UNKNOWN BODY").unwrap();
        for root in &former_roots {
            if root == &current {
                continue;
            }
            let original = root.join(&relative);
            std::fs::create_dir_all(original.parent().unwrap()).unwrap();
            std::fs::write(&original, b"FORMER ROOT DO NOT DELETE").unwrap();
            if !retained.contains(&original) {
                retained.push(original);
            }
        }
        let (b, storage) = f.open();
        assert!(storage.data_write_blocked());
        assert_eq!(synthetic_meeting_reconfirmation_records(&storage), recorded);
        let controller = AnalysisController::default();
        let mut p = operations::Prepared::pending(&storage, &op, &|| false).unwrap();
        let lease = controller
            .begin_deletion_maintenance(&storage, &op, false, false, &|| false)
            .unwrap();
        assert!(!p.preview().execute_allowed && p.preview().reconfirmation_allowed);
        assert!(p
            .execute(
                &storage,
                &lease,
                &p.preview().confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .is_err());
        assert!(body.exists());
        p.review_remaining(&storage, &lease, &p.preview().confirmation_id, &|| false)
            .unwrap();
        assert!(
            p.execute(
                &storage,
                &lease,
                &p.preview().confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
        );
        assert!(!body.exists());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        assert_eq!(f.live_record(&storage), old_audio);
        recorded = synthetic_meeting_reconfirmation_records(&storage);
        assert_eq!(recorded.len(), prior_moves);
        for path in &retained {
            assert_eq!(std::fs::read(path).unwrap(), b"FORMER ROOT DO NOT DELETE");
        }
        drop(lease);
        drop(storage);
        drop(b);
        // The appended v4 audit uses the preceding current root; a new move
        // must retain both that root and the original root without rewriting.
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        let next = target(&f);
        flat(&select_current(&b, &storage, &next), &op, prior_moves);
        former_roots.push(current);
        drop(storage);
        drop(b);
        current = next;
    }
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert_eq!(f.live_record(&storage), old_audio);
    assert_eq!(synthetic_meeting_reconfirmation_records(&storage), recorded);
    assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(storage.recorded_audio_plan(&f.meeting).is_err());
    for path in retained {
        assert_eq!(std::fs::read(path).unwrap(), b"FORMER ROOT DO NOT DELETE");
    }
}

#[test]
fn deletion_repeat_moves_restored_unavailable_audio_and_supersession_survive_rebackup() {
    for full in [false, true] {
        let mut f = NativeFixture::new(false, false);
        let storage = f.storage.as_ref().unwrap();
        let archive = f._base.path().join("restore-input.monbackup");
        storage.create_backup_archive(&archive, &|| false).unwrap();
        let selected = storage
            .prepare_additive_restore(&archive, &|| false, &|_| {})
            .unwrap();
        let restored = selected
            .publish(
                storage,
                &selected.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap()
            .meeting_ids[0]
            .clone();
        drop(selected);
        let op = synthetic_restored_audio_operation(storage, &restored);
        let old = synthetic_consent_record(storage, &op);
        if full {
            let mut p = operations::Prepared::fresh(storage, &restored, &|| false).unwrap();
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                p.execute(
                    storage,
                    &lease,
                    &p.preview().confirmation_id,
                    true,
                    &|| false,
                    &|_| {}
                )
                .unwrap()
                .deleted
            );
        }
        f.select();
        for prior_moves in 1..=2 {
            let (b, storage) = f.open();
            assert!(!storage.data_write_blocked());
            assert_eq!(synthetic_consent_record(&storage, &op), old);
            let next = target(&f);
            flat(&select_current(&b, &storage, &next), &op, prior_moves);
            drop(storage);
            drop(b);
        }
        let (_b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        assert!(storage.recorded_audio_plan(&restored).is_err());
        let archive = f._base.path().join("rebackup.monbackup");
        assert_eq!(
            storage
                .create_backup_archive(&archive, &|| false)
                .unwrap()
                .deleted_audio_meeting_count,
            if full { 1 } else { 2 }
        );
    }
}

#[test]
fn deletion_repeat_moves_models_keep_native_root_and_never_reuse_moved_consent() {
    use crate::storage::deletion::models;
    for completed in [false, true] {
        let mut f = NativeFixture::new(false, false);
        let (op, file) = synthetic_model(&f, completed);
        let old = synthetic_consent_record(f.storage.as_ref().unwrap(), &op);
        let models_root = f.bootstrap.as_ref().unwrap().models_root.clone();
        f.select();
        for prior_moves in 1..=2 {
            let (b, storage) = f.open();
            assert_eq!(b.models_root, models_root);
            assert!(!storage.data_write_blocked());
            let next = target(&f);
            flat(&select_current(&b, &storage, &next), &op, prior_moves);
            drop(storage);
            drop(b);
        }
        let (b, storage) = f.open();
        assert_eq!(b.models_root, models_root);
        assert!(!storage.data_write_blocked());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        assert_eq!(file.exists(), !completed);
        if !completed {
            assert_eq!(std::fs::read(&file).unwrap(), b"SYNTHETIC MODEL");
        }
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        assert!(models::intent::removal::start(&storage, &op, &lease, &|| false).is_err());
        assert!(storage
            .create_meeting("CURRENT DB, ORIGINAL MODEL ROOT")
            .is_ok());
    }
}

#[test]
fn deletion_repeat_moves_real_storage_crash_reads_wal_and_preserves_latest_commit() {
    let mut f = NativeFixture::new(false, false);
    let old = f.live_record(f.storage.as_ref().unwrap());
    f.select();
    let (b, storage) = f.open();
    let next = target(&f);
    flat(&select_current(&b, &storage, &next), &f.deletion, 1);
    drop(storage);
    drop(b);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bootstrap::placement::activation::tests::native_history::deletion_startup_admission_storage_crash_child", "--ignored", "--nocapture"])
        .env("MON_SYNTHETIC_STARTUP_BOOTSTRAP", serde_json::to_string(&(&f.config, &f.legacy)).unwrap()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(next.join("meetings.sqlite3-wal").metadata().unwrap().len() > 32);
    assert!(!next.join("meetings.sqlite3-shm").exists());
    let (_b, storage) = f.open();
    assert_eq!(storage.root(), next);
    assert!(!storage.data_write_blocked());
    assert_eq!(f.live_record(&storage), old);
    assert!(storage
        .list_meetings()
        .unwrap()
        .iter()
        .any(|m| m.title == "LATEST COMMITTED IN WAL"));
    assert!(!next.join(&f.audio).exists());
}

#[test]
fn deletion_repeat_moves_cancel_never_retire_source_or_publish_selection() {
    let mut f = NativeFixture::new(true, false);
    let old = f.live_record(f.storage.as_ref().unwrap());
    f.select();
    let (b, storage) = f.open();
    let next = target(&f);
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let operation = Uuid::new_v4().to_string();
    b.begin_move(
        &operation,
        &Uuid::new_v4().to_string(),
        &next,
        snapshot.manifest_hash(),
        b.view().unwrap().revision,
    )
    .unwrap();
    let mut p = b.pending_placement(&operation, &snapshot).unwrap();
    p.copy(&snapshot, &|| false).unwrap();
    assert!(snapshot.prepare_control_handoff(&|| true).is_err());
    let h = snapshot.prepare_control_handoff(&|| false).unwrap();
    flat(&serde_json::from_slice(h.bytes()).unwrap(), &f.deletion, 1);
    assert!(p.install_history(&h, &|| true).is_err());
    p.install_history(&h, &|| false).unwrap();
    let registry = next.join(".mon-scratch.sqlite3");
    let before = std::fs::read(&registry).unwrap();
    assert!(p.inspect_deletion_handoff(&h, &|| true).is_err());
    assert_eq!(std::fs::read(registry).unwrap(), before);
    assert!(h.require_supported_cutover().is_err());
    assert_eq!(
        b.state
            .lock()
            .unwrap()
            .loaded
            .as_ref()
            .unwrap()
            .record
            .current
            .path,
        f.target
    );
    assert_eq!(f.live_record(&storage), old);
    assert_eq!(
        std::fs::read(f.target.join(&f.audio)).unwrap(),
        b"SYNTHETIC AUDIO"
    );
    drop(h);
    drop(p);
    drop(snapshot);
    assert!(!storage.data_write_blocked());
    assert!(storage.create_meeting("SOURCE NOT RETIRED").is_ok());
}

#[test]
fn deletion_repeat_moves_forged_historical_binding_or_roots_refuse_before_mutation() {
    for changed_roots in [false, true] {
        let mut f = NativeFixture::new(true, false);
        f.select();
        let (b, storage) = f.open();
        let next = target(&f);
        select_current(&b, &storage, &next);
        drop(storage);
        drop(b);
        let (b, storage) = f.open(); // admit the immutable image before tampering
        assert!(!storage.data_write_blocked());
        let operation = b
            .state
            .lock()
            .unwrap()
            .loaded
            .as_ref()
            .unwrap()
            .record
            .activation
            .as_ref()
            .unwrap()
            .pending
            .operation_id
            .clone();
        drop(storage);
        drop(b);
        let registry = next.join(".mon-scratch.sqlite3");
        let db = Connection::open(&registry).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        let json: String = db
            .query_row(
                "SELECT local_json FROM relocation_history WHERE operation_id=?1",
                [&operation],
                |r| r.get(0),
            )
            .unwrap();
        let mut local: serde_json::Value = serde_json::from_str(&json).unwrap();
        if changed_roots {
            local["deletion_history"]["records"][0]["historicalRoots"][0]["root"]["inode"] =
                0.into();
        } else {
            local["deletion_binding"]["historicalDirectories"][0]["copied"]["inode"] = 0.into();
        }
        let json = serde_json::to_string(&local).unwrap();
        db.execute(
            "UPDATE relocation_history SET local_json=?1,local_hash=?2 WHERE operation_id=?3",
            params![json, hash(json.as_bytes()), operation],
        )
        .unwrap();
        drop(db);
        let native = std::fs::read(next.join("meetings.sqlite3")).unwrap();
        let registry_before = std::fs::read(&registry).unwrap();
        let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
        assert!(b.open_storage().is_err());
        assert_eq!(
            std::fs::read(next.join("meetings.sqlite3")).unwrap(),
            native
        );
        assert_eq!(std::fs::read(&registry).unwrap(), registry_before);
        assert!(!next.join("meetings.sqlite3-wal").exists());
        assert_eq!(
            std::fs::read(next.join(&f.audio)).unwrap(),
            b"SYNTHETIC AUDIO"
        );
    }
}

#[test]
fn deletion_repeat_moves_missing_all_predecessor_records_never_become_no_history() {
    let mut f = NativeFixture::new(true, false);
    f.select();
    let (b, storage) = f.open();
    crate::storage::deletion::relocation::receiver::synthetic_remove_kept_audio_record(
        &storage,
        &f.deletion,
    );
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let result = snapshot.prepare_control_handoff(&|| false);
    assert!(matches!(result, Err(ref error) if error.contains("元の操作が欠落")));
    assert_eq!(b.view().unwrap().meeting_root, f.target);
    assert_eq!(
        std::fs::read(f.target.join(&f.audio)).unwrap(),
        b"SYNTHETIC AUDIO"
    );
    assert!(!snapshot
        .image()
        .parent()
        .unwrap()
        .join("relocation-control.json")
        .exists());
}
