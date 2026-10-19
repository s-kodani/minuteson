//! Actual bounded native restore ledger and synthetic reappearing files only.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::audio::intent::{self, reconfirmation::Review};

fn policy() -> intent::Policy {
    intent::Policy {
        include_unregistered_audio: true,
        retain_unclassified: true,
    }
}
fn owner(storage: &Storage, operation: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(
            storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
}
fn fixed(storage: &Storage, op: &str) -> String {
    storage.connection.lock().unwrap().query_row("SELECT json_array(rowid,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1", [op], |r| r.get(0)).unwrap()
}
fn consent(storage: &Storage, source: &str, lease: &DataMaintenanceLease) -> String {
    let review = Review::capture(
        storage,
        source,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let operation = Uuid::new_v4().to_string();
    intent::reconfirmation::accept(
        storage,
        &operation,
        &review,
        policy(),
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    operation
}
fn execute(storage: &Storage, operation: &str, lease: &DataMaintenanceLease) {
    intent::removal::start(
        storage,
        operation,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::removal::run(
        storage,
        operation,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(intent::removal::completion::finish(
        storage,
        operation,
        lease,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
}
fn attention(storage: &Storage, op: &str) -> bool {
    storage.connection.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM temp.mon_deletion_startup_attention WHERE operation_id=?1)", [op], |r|r.get(0)).unwrap()
}

#[test]
fn restored_audio_reconfirmation_new_consent_preserves_no_consent_baseline_and_text() {
    let (_root, target, meeting, original) = native_fixture();
    let before = fixed(&target, &original);
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"REAPPEARED DIFFERENT AUDIO").unwrap();
    let unknown = path.with_file_name("unknown.bin");
    std::fs::write(&unknown, b"RETAIN").unwrap();
    let draft = target.load_transcript_draft(&meeting).unwrap();
    let analysis = target.load_current_analysis(&meeting).unwrap();
    let lease = owner(&target, &original);
    // The baseline is never a permit even with real shared admission.
    assert!(intent::removal::start(
        &target,
        &original,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let review = Review::capture(
        &target,
        &original,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = review.preview(&Uuid::new_v4().to_string()).unwrap();
    assert!(!preview.execute_allowed && preview.changed_audio_files > 0);
    assert!(preview.unclassified_files > 0 && preview.retained.analysis_revisions > 0);
    let operation = consent(&target, &original, &lease);
    let json: Value = target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&operation],
            |r| r.get::<_, String>(0),
        )
        .map(|s| serde_json::from_str(&s).unwrap())
        .unwrap();
    assert_eq!(json["version"], 3);
    assert_eq!(json["origin"]["operationId"], original);
    execute(&target, &operation, &lease);
    assert!(!path.exists() && unknown.exists());
    assert_eq!(fixed(&target, &original), before);
    assert_eq!(target.load_transcript_draft(&meeting).unwrap(), draft);
    assert_eq!(target.load_current_analysis(&meeting).unwrap(), analysis);
    assert!(target.recorded_audio_plan(&meeting).is_err());
    let db = target.connection.lock().unwrap();
    assert!(restored::deleted(&db, &original, &meeting).is_ok());
    assert_eq!(
        audio::backup_deleted(&target, &db, &crate::test_support::never_cancelled).unwrap(),
        std::collections::BTreeSet::from([meeting])
    );
    assert!(!target.data_write_blocked());
}

#[test]
fn restored_audio_reconfirmation_cold_saved_scope_resume_clears_attention_but_requires_restart() {
    let (_root, target, meeting, original) = native_fixture();
    let before = fixed(&target, &original);
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let root = target.root.clone();
    drop(target);
    let target = Storage::open(root.clone()).unwrap();
    assert!(target.data_write_blocked() && attention(&target, &original));
    let lease = owner(&target, &original);
    let operation = consent(&target, &original, &lease);
    drop(lease);
    drop(target);
    let target = Storage::open(root.clone()).unwrap();
    let lease = owner(&target, &operation);
    execute(&target, &operation, &lease);
    assert!(!path.exists() && !attention(&target, &original));
    assert_eq!(fixed(&target, &original), before);
    assert!(target.data_write_blocked() && target.deletion_resume_restart_required());
    assert!(target.create_meeting("DENIED BEFORE RESTART").is_err());
    drop(lease);
    drop(target);
    let target = Storage::open(root).unwrap();
    assert!(!target.data_write_blocked() && target.create_meeting("NORMAL").is_ok());
}

#[test]
fn restored_audio_reconfirmation_native_descendants_derive_only_exact_reserved_names() {
    let (_root, target, meeting, original) = native_fixture();
    let before = fixed(&target, &original);
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"FIRST RETURN").unwrap();
    let lease = owner(&target, &original);
    let operation = consent(&target, &original, &lease);
    execute(&target, &operation, &lease);
    drop(lease);
    let reserved = path.with_file_name(format!(".mon-delete-audio-{operation}-0"));
    // The restore itself issued no reservations. A similar baseline name
    // therefore stays unclassified even after a later native deletion.
    let unknown = path.with_file_name(format!(".mon-delete-audio-{original}-0"));
    std::fs::write(&reserved, b"NATIVE RESERVED RETURN").unwrap();
    std::fs::write(&unknown, b"NEVER ISSUED BY RESTORE").unwrap();
    let lease = owner(&target, &operation);
    let review = Review::capture(
        &target,
        &operation,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = review.preview(&Uuid::new_v4().to_string()).unwrap();
    assert_eq!(preview.unregistered_audio_files, 1);
    assert!(preview.entries.iter().any(|e| e.relative_path
        == reserved
            .strip_prefix(&target.root)
            .unwrap()
            .to_str()
            .unwrap()
        && e.disposition == crate::storage::deletion::audio::Disposition::HistoricalAudio));
    let next = Uuid::new_v4().to_string();
    assert!(intent::reconfirmation::accept(
        &target,
        &next,
        &review,
        intent::Policy {
            include_unregistered_audio: false,
            retain_unclassified: true
        },
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    intent::reconfirmation::accept(
        &target,
        &next,
        &review,
        policy(),
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    execute(&target, &next, &lease);
    assert!(!reserved.exists() && unknown.exists());
    assert_eq!(fixed(&target, &original), before);
}

#[test]
fn restored_audio_reconfirmation_ancestral_renewal_full_purge_preserves_metadata_only_baseline() {
    use crate::storage::deletion::operations;
    let (_root, target, meeting, original) = native_fixture();
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"FIRST NATIVE RETURN").unwrap();
    let lease = owner(&target, &original);
    let parent = consent(&target, &original, &lease);
    execute(&target, &parent, &lease);
    std::fs::write(&path, b"NEXT NATIVE RETURN").unwrap();
    let child = consent(&target, &parent, &lease);
    intent::removal::start(
        &target,
        &child,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let returned = path.with_file_name(format!(".mon-delete-audio-{parent}-0"));
    std::fs::write(&returned, b"ANCESTRAL RESERVED RETURNED LATE").unwrap();
    let review = intent::removal::renewal::Review::capture(
        &target,
        &child,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::removal::renewal::accept(
        &target,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::removal::run(
        &target,
        &child,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::removal::completion::finish(
        &target,
        &child,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!returned.exists());
    let record = |id: &str| {
        target.connection.lock().unwrap().query_row("SELECT json_array(rowid,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) FROM maintenance_operations WHERE id=?1", [id], |r|r.get::<_,String>(0)).unwrap()
    };
    let before: Vec<_> = [&original, &parent, &child].map(|id| record(id)).into();
    let mut prepared =
        operations::Prepared::fresh(&target, &meeting, &crate::test_support::never_cancelled)
            .unwrap();
    let preview = prepared.preview();
    assert!(
        prepared
            .execute(
                &target,
                &lease,
                &preview.confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    for (id, expected) in [&original, &parent, &child].iter().zip(&before) {
        assert_eq!(&record(id), expected);
        let db = target.connection.lock().unwrap();
        assert!(
            audio::backup_deleted(&target, &db, &crate::test_support::never_cancelled)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.query_row(
                "SELECT consent_at FROM maintenance_operations WHERE id=?1",
                [&original],
                |r| r.get::<_, Option<i64>>(0)
            )
            .unwrap(),
            None
        );
        drop(db);
        assert!(
            intent::removal::start(&target, id, &lease, &crate::test_support::never_cancelled)
                .is_err()
        );
        assert!(
            Review::capture(&target, id, &lease, &crate::test_support::never_cancelled).is_err()
        );
    }
    drop(lease);
    let storage_root = target.root.clone();
    drop(target);
    let target = Storage::open(storage_root).unwrap();
    assert!(!target.data_write_blocked());
    assert!(target.get_meeting(&meeting).unwrap().is_none());
}

#[test]
fn restored_audio_reconfirmation_kept_new_consent_never_resolves_old_reappearance() {
    let (_root, target, meeting, original) = native_fixture();
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"KEEP UNTIL EXPLICIT NEW CONSENT").unwrap();
    let root = target.root.clone();
    drop(target);
    let target = Storage::open(root).unwrap();
    let before = fixed(&target, &original);
    let lease = owner(&target, &original);
    let operation = consent(&target, &original, &lease);
    intent::keep_unstarted(
        &target,
        &operation,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(path.exists() && attention(&target, &original));
    assert!(intent::removal::start(
        &target,
        &operation,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let next = consent(&target, &original, &lease);
    execute(&target, &next, &lease);
    assert_eq!(fixed(&target, &original), before);
    assert!(!path.exists() && !attention(&target, &original));
}

#[test]
fn restored_audio_reconfirmation_changed_inventory_cancel_dead_lease_do_not_publish() {
    for variant in 0..3 {
        let (_root, target, meeting, original) = native_fixture();
        let path = audio_path(&target, &meeting);
        std::fs::write(&path, b"RETURNED").unwrap();
        let controller = AnalysisController::default();
        let lease = controller
            .begin_audio_deletion_maintenance(
                &target,
                &original,
                false,
                false,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        let review = Review::capture(
            &target,
            &original,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        if variant == 0 {
            std::fs::write(&path, b"CHANGED AFTER REVIEW").unwrap();
        }
        if variant == 2 {
            controller.begin_exit().unwrap();
            controller.abort_shutdown();
        }
        let operation = Uuid::new_v4().to_string();
        assert!(intent::reconfirmation::accept(
            &target,
            &operation,
            &review,
            policy(),
            &lease,
            &|| variant == 1
        )
        .is_err());
        assert!(path.exists());
        let db = target.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM maintenance_operations WHERE id=?1",
                [&operation],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(restored::deleted(&db, &original, &meeting).is_ok());
    }
}

#[test]
fn restored_audio_reconfirmation_corrupt_ledger_or_receipt_never_grants_admission() {
    let (_root, target, meeting, original) = native_fixture();
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let lease = owner(&target, &original);
    let review = Review::capture(
        &target,
        &original,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    for sql in [
        "UPDATE maintenance_operations SET consent_at=1 WHERE id=?1",
        "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
        "UPDATE restore_identity_maps SET target_key='forged' WHERE target_key=?1",
    ] {
        let db = target.connection.lock().unwrap();
        let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate).unwrap();
        tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        tx.execute(sql, [&original]).unwrap();
        tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        assert!(
            restored::deleted(&tx, &original, &meeting).is_err(),
            "{sql}"
        );
        tx.rollback().unwrap();
    }
    // A previously captured review also detects a changed origin at accept.
    target
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
            [&original],
        )
        .unwrap();
    assert!(intent::reconfirmation::accept(
        &target,
        &Uuid::new_v4().to_string(),
        &review,
        policy(),
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    drop(lease);
    assert!(AnalysisController::default()
        .begin_audio_deletion_maintenance(
            &target,
            &original,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(path.exists());
}

#[test]
fn restored_audio_reconfirmation_symlink_and_unowned_database_copy_remain_fenced() {
    let (root, target, meeting, original) = native_fixture();
    let path = audio_path(&target, &meeting);
    let outside = root.path().join("unrelated-audio");
    std::fs::write(&outside, b"NEVER DELETE OUTSIDE").unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let lease = owner(&target, &original);
    assert!(Review::capture(
        &target,
        &original,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"NEVER DELETE OUTSIDE");
    drop(lease);
    let copy = tempfile::tempdir().unwrap();
    target
        .connection
        .lock()
        .unwrap()
        .execute(
            "VACUUM INTO ?1",
            [copy.path().join("meetings.sqlite3").to_str().unwrap()],
        )
        .unwrap();
    let copied = Storage::open(copy.path().to_owned()).unwrap();
    assert!(AnalysisController::default()
        .begin_audio_deletion_maintenance(
            &copied,
            &original,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
}

#[test]
fn restored_audio_reconfirmation_new_consent_cannot_predate_verified_publication() {
    let (_root, target, meeting, original) = native_fixture();
    let future = crate::storage::now_ms().unwrap() + 86_400_000;
    {
        // Owned synthetic DB only; restore real triggers before any product
        // call. The canonical baseline and published ledger agree on time.
        let db = target.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute("UPDATE restore_batches SET published_at=?1", [future])
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET created_at=?2,updated_at=?2 WHERE id=?1",
            params![original, future],
        )
        .unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        assert!(restored::deleted(&db, &original, &meeting).is_ok());
    }
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let lease = owner(&target, &original);
    let review = Review::capture(
        &target,
        &original,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let operation = Uuid::new_v4().to_string();
    assert!(intent::reconfirmation::accept(
        &target,
        &operation,
        &review,
        policy(),
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(path.exists());
    assert_eq!(
        target
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM maintenance_operations WHERE id=?1",
                [&operation],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn restored_audio_reconfirmation_real_publication_sigkill_resume_and_full_deletion_keep_origin() {
    use std::os::unix::process::ExitStatusExt;
    let (base, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let archive = output.path().join("restore-reconfirmation.monbackup");
    source
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let target = Storage::open(base.path().join("crash-target")).unwrap();
    let selected = target
        .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let preview = selected.preview();
    let meeting = selected
        .publish(
            &target,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .meeting_ids[0]
        .clone();
    drop(selected);
    let original: String = target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM maintenance_operations WHERE meeting_id=?1 AND kind='delete_assets'",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let before = fixed(&target, &original);
    let path = audio_path(&target, &meeting);
    std::fs::write(&path, b"RETURNED AFTER REAL PUBLICATION").unwrap();
    let root = target.root.clone();
    let operation = Uuid::new_v4().to_string();
    let marker = (
        root.to_str().unwrap().to_owned(),
        original.clone(),
        operation.clone(),
    );
    std::fs::write(
        root.parent()
            .unwrap()
            .join("restored-audio-reconfirmation-owner.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    drop(target);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "storage::backup::archive::deleted_audio_tests::restored::reconfirmation::restored_audio_reconfirmation_crash_child", "--ignored", "--nocapture"])
        .env("MON_RESTORED_AUDIO_RECONFIRMATION_ROOT", &root)
        .env("MON_RESTORED_AUDIO_RECONFIRMATION_SOURCE", &original)
        .env("MON_RESTORED_AUDIO_RECONFIRMATION_OPERATION", &operation)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    let target = Storage::open(root.clone()).unwrap();
    assert!(target.data_write_blocked() && path.exists());
    assert_eq!(fixed(&target, &original), before);
    let lease = owner(&target, &operation);
    execute(&target, &operation, &lease);
    assert!(!path.exists() && !attention(&target, &original));
    assert_eq!(fixed(&target, &original), before);
    drop(lease);
    drop(target);
    let target = Storage::open(root).unwrap();
    assert!(!target.data_write_blocked());
    let (_, receipt) = write(&target, output.path());
    assert_eq!(receipt.deleted_audio_meeting_count, 1);
    // Full deletion must retain both baseline and subsequent native history,
    // even after the nullable meeting FK is detached by the schema.
    let record = |storage: &Storage, id: &str| -> String {
        storage.connection.lock().unwrap().query_row("SELECT json_array(target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) FROM maintenance_operations WHERE id=?1", [id], |r|r.get(0)).unwrap()
    };
    let originals = (record(&target, &original), record(&target, &operation));
    let mut deletion = crate::storage::deletion::operations::Prepared::fresh(
        &target,
        &meeting,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let confirmation = deletion.preview().confirmation_id;
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(
        deletion
            .execute(
                &target,
                &lease,
                &confirmation,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    assert_eq!(
        (record(&target, &original), record(&target, &operation)),
        originals
    );
    drop(lease);
    let (_, receipt) = write(&target, output.path());
    assert_eq!(
        (receipt.meeting_count, receipt.deleted_audio_meeting_count),
        (0, 0)
    );
}

#[test]
#[ignore = "subprocess-only parent-owned synthetic restored-audio reconfirmation fixture"]
fn restored_audio_reconfirmation_crash_child() {
    let root = PathBuf::from(std::env::var("MON_RESTORED_AUDIO_RECONFIRMATION_ROOT").unwrap());
    let source = std::env::var("MON_RESTORED_AUDIO_RECONFIRMATION_SOURCE").unwrap();
    let operation = std::env::var("MON_RESTORED_AUDIO_RECONFIRMATION_OPERATION").unwrap();
    assert_eq!(root.file_name().unwrap(), "crash-target");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(
            root.parent()
                .unwrap()
                .join("restored-audio-reconfirmation-owner.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker,
        (
            root.to_str().unwrap().into(),
            source.clone(),
            operation.clone()
        )
    );
    let storage = Storage::open(root).unwrap();
    let lease = owner(&storage, &source);
    let review = Review::capture(
        &storage,
        &source,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::reconfirmation::accept(
        &storage,
        &operation,
        &review,
        policy(),
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
    panic!("SIGKILL failed");
}
