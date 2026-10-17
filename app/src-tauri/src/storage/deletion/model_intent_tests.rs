//! All files/DBs are synthetic and temporary; no IPC or real engine execution.
use super::*;
use crate::analysis_worker::{data_maintenance::DataMaintenanceLease, AnalysisController};
use rusqlite::{config::DbConfig, Connection};

#[path = "model_removal_tests.rs"]
mod removing;

fn lease() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap()
}
fn consent(
    f: &Fixture,
    operation: &str,
    census: &str,
    include_unknown: bool,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<intent::Prepared, String> {
    intent::prepare(
        &f.storage,
        &f.root,
        operation,
        &f.package,
        &f.review,
        census,
        include_unknown,
        lease,
        check,
    )
}
fn count(f: &Fixture) -> i64 {
    f.storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
            r.get(0)
        })
        .unwrap()
}
fn receipt(f: &Fixture, operation: &str) -> (String, String, i64, i64) {
    f.storage.connection.lock().unwrap().query_row("SELECT target_manifest_json,target_hash,consent_at,created_at FROM maintenance_operations WHERE id=?1",params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
fn reopen(f: &mut Fixture, mutate: impl FnOnce(&Connection)) {
    let root = f.storage.root().to_path_buf();
    // Swap in a different temporary DB so the original exclusive Connection
    // really closes. This is test-only; there is no force-unlock in production.
    let placeholder = tempfile::tempdir().unwrap();
    let old = std::mem::replace(
        &mut f.storage,
        Storage::open(placeholder.path().into()).unwrap(),
    );
    drop(old);
    let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    mutate(&db);
    drop(db);
    f.storage = Storage::open_with_models(root, f.root.clone()).unwrap();
}

#[test]
fn model_deletion_consent_is_atomic_readable_and_preserves_history_and_files() {
    let f = Fixture::new();
    let before: serde_json::Value = serde_json::from_str(&f.evidence()).unwrap();
    let p = f.preview().unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &operation,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(intent::read(&db, &operation).unwrap(), saved);
    let (rows, max_json, max_path): (i64,i64,i64) = db.query_row("SELECT count(*),max(length(CAST(identity_json AS BLOB))),max(length(CAST(relative_path AS BLOB))) FROM maintenance_paths WHERE operation_id=?1",params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(rows > 1 && max_json <= 1024 && max_path <= 256);
    assert!(db
        .execute(
            "UPDATE maintenance_operations SET consent_at=0 WHERE id=?1",
            params![operation]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE maintenance_paths SET identity_json='{}' WHERE operation_id=?1",
            params![operation]
        )
        .is_err());
    drop(db);
    saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .unwrap();
    let after: serde_json::Value = serde_json::from_str(&f.evidence()).unwrap();
    for index in [0, 1, 3, 4, 5] {
        assert_eq!(before[index], after[index]);
    }
    assert_eq!(count(&f), 1);
    assert!(f.storage.data_write_blocked());
    assert!(f.storage.create_meeting("REJECT").is_err());
    assert_eq!(
        std::fs::read(f.path("cli")).unwrap(),
        b"synthetic; do not execute"
    );
    assert_eq!(
        std::fs::read(f.path("weights/model.bin")).unwrap(),
        vec![b'm'; 150000]
    );
}

#[test]
fn model_deletion_consent_retry_keeps_consent_time_and_rejects_changed_policy_or_scope() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let a = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let original = receipt(&f, &op);
    assert_eq!(
        a,
        consent(
            &f,
            &op,
            &p.census_hash,
            false,
            &owner,
            &crate::test_support::never_cancelled
        )
        .unwrap()
    );
    assert_eq!(original, receipt(&f, &op));
    assert!(consent(
        &f,
        &op,
        &p.census_hash,
        true,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(consent(
        &f,
        &op,
        &"0".repeat(64),
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(original, receipt(&f, &op));
    assert_eq!(count(&f), 1);
}

#[test]
fn model_deletion_consent_rejects_stale_preview_and_requires_explicit_unknown_policy() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    std::fs::write(f.path("unknown"), b"KEEP").unwrap();
    let owner = lease();
    assert!(consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        true,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let p = f.preview().unwrap();
    assert!(consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(count(&f), 0);
    let op = uuid::Uuid::new_v4().to_string();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        true,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(f.path("new-after-consent"), b"NEVER ADOPT").unwrap();
    assert!(saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .is_err());
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(f.path("new-after-consent")).unwrap(),
        b"NEVER ADOPT"
    );
    assert_eq!(std::fs::read(f.path("unknown")).unwrap(), b"KEEP");
}

#[test]
fn model_deletion_consent_cancellation_rolls_back_header_and_every_chunk() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let calls = Cell::new(0);
    consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        false,
        &lease(),
        &|| {
            calls.set(calls.get() + 1);
            false
        },
    )
    .unwrap();
    let total = calls.get();
    assert!(total > 20);
    for stop in [1, 3, 10, total / 2, total - 1, total] {
        let f = Fixture::new();
        let p = f.preview().unwrap();
        let calls = Cell::new(0);
        assert!(
            consent(
                &f,
                &uuid::Uuid::new_v4().to_string(),
                &p.census_hash,
                false,
                &lease(),
                &|| {
                    calls.set(calls.get() + 1);
                    calls.get() >= stop
                }
            )
            .is_err(),
            "stop {stop}/{total}"
        );
        assert_eq!(count(&f), 0);
        assert!(!f.storage.data_write_blocked());
        let db = f.storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM maintenance_paths", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        assert_eq!(
            std::fs::read(f.path("cli")).unwrap(),
            b"synthetic; do not execute"
        );
    }
}

#[test]
fn model_deletion_consent_requires_live_lease_even_with_valid_saved_evidence() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let controller = AnalysisController::default();
    let owner = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .is_err());
    assert!(intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f.storage.data_write_blocked());
    drop(owner);
    let owner = controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.storage.data_write_blocked());
}

#[test]
fn model_deletion_consent_keep_is_idempotent_preserves_changed_tree_and_never_revives() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let original = receipt(&f, &op);
    std::fs::write(f.path("cli"), b"CHANGED KEEP").unwrap();
    std::fs::remove_file(f.path("weights/model.bin")).unwrap();
    std::fs::write(f.path("added"), b"ADDED KEEP").unwrap();
    assert!(saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .is_err());
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(original, receipt(&f, &op));
    assert!(!f.storage.data_write_blocked());
    assert!(saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .is_err());
    assert!(consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read(f.path("cli")).unwrap(), b"CHANGED KEEP");
    assert_eq!(std::fs::read(f.path("added")).unwrap(), b"ADDED KEEP");
    assert!(!f.path("weights/model.bin").exists());
    f.storage.create_meeting("ADMITTED AFTER HOT KEEP").unwrap();
}

#[test]
fn model_deletion_consent_keep_cancellation_restores_fence_and_preserves_receipt() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let calls = Cell::new(0);
    assert!(intent::keep_unstarted(&f.storage, &op, &owner, &|| {
        calls.set(calls.get() + 1);
        calls.get() == 3
    })
    .is_err());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(intent::read(&db, &op).unwrap(), saved);
    assert!(db.execute("DELETE FROM meetings", []).is_err());
    drop(db);
    assert!(f.storage.data_write_blocked());
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
}

#[test]
fn model_deletion_consent_cold_keep_requires_restart_before_deferred_recovery() {
    let mut f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    // Simulate another previously interrupted process in the closed DB. Startup
    // must NOT rewrite it while an unresolved deletion receipt exists.
    reopen(&mut f, |db| {
        let meeting: String = db
            .query_row("SELECT id FROM meetings LIMIT 1", [], |r| r.get(0))
            .unwrap();
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale',?1,'transcription','running','stale',?2,'{}','{}',?2,1,1)",params![meeting,"a".repeat(64)]).unwrap();
    });
    assert!(f.storage.data_write_blocked());
    let state = || {
        f.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    };
    assert_eq!(state(), "running");
    let controller = AnalysisController::default();
    controller.require_storage_restart();
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_err());
    let owner = controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(f
        .storage
        .require_data_writable()
        .unwrap_err()
        .contains("再起動"));
    assert_eq!(state(), "running");
    drop(owner);
    reopen(&mut f, |_| {});
    assert!(!f.storage.data_write_blocked());
    assert_eq!(
        f.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "interrupted"
    );
}

#[test]
fn model_deletion_consent_corrupt_chunks_and_unknown_asset_records_remain_fenced() {
    for corruption in ["chunk", "slot", "progress", "subtype"] {
        let mut f = Fixture::new();
        let p = f.preview().unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        consent(
            &f,
            &op,
            &p.census_hash,
            false,
            &lease(),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        reopen(&mut f, |db| {
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            match corruption {
                "chunk" => {
                    db.execute("UPDATE maintenance_paths SET identity_json='{}' WHERE operation_id=?1 AND relative_path='model-deletion-proof/000000'",params![op]).unwrap();
                }
                "slot" => {
                    db.execute("UPDATE maintenance_paths SET relative_path='other-proof/000000' WHERE operation_id=?1 AND relative_path='model-deletion-proof/000000'",params![op]).unwrap();
                }
                "progress" => {
                    db.execute(
                        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                        params![op],
                    )
                    .unwrap();
                }
                _ => {
                    db.execute(
                        "UPDATE maintenance_operations SET target_manifest_json='{}' WHERE id=?1",
                        params![op],
                    )
                    .unwrap();
                }
            }
        });
        assert!(f.storage.data_write_blocked(), "{corruption}");
        assert!(
            intent::authorize_review(&f.storage, &op).is_err(),
            "{corruption}"
        );
        assert!(
            intent::keep_unstarted(
                &f.storage,
                &op,
                &lease(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{corruption}"
        );
        assert_eq!(
            std::fs::read(f.path("cli")).unwrap(),
            b"synthetic; do not execute"
        );
    }
}

#[test]
fn model_deletion_consent_rejects_running_sql_work_but_allows_stale_partial_package() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    let meeting: String = db
        .query_row("SELECT id FROM meetings LIMIT 1", [], |r| r.get(0))
        .unwrap();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('active',?1,'transcription','running','active',?2,'{}','{}',?2,1,1)",params![meeting,"a".repeat(64)]).unwrap();
    drop(db);
    let p = f.preview().unwrap();
    assert!(consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(count(&f), 0);
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE processing_jobs SET state='paused' WHERE id='active'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE model_packages SET state='preparing' WHERE id=?1",
        params![f.package],
    )
    .unwrap();
    db.execute(
        "UPDATE model_files SET state='unavailable' WHERE package_id=?1",
        params![f.package],
    )
    .unwrap();
    drop(db);
    std::fs::remove_file(f.path("cli")).unwrap();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.path("cli").exists());
}

#[test]
fn model_deletion_consent_keeps_long_unicode_paths_without_path_table_truncation() {
    let f = Fixture::new();
    let parent = "日本語".repeat(20);
    let relative = format!("{parent}/{parent}/合成証拠");
    std::fs::create_dir_all(f.path(&format!("{parent}/{parent}"))).unwrap();
    std::fs::write(f.path(&relative), b"KEEP LONG PATH").unwrap();
    assert!(relative.len() > 256);
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        true,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(std::fs::read(f.path(&relative)).unwrap(), b"KEEP LONG PATH");
}

#[test]
fn model_deletion_consent_absent_package_is_not_recreated_and_missing_root_is_rejected() {
    let f = Fixture::new();
    std::fs::rename(f.path(""), f.root.join("held")).unwrap();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .unwrap();
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.path("").exists());
    std::fs::rename(&f.root, f.root.parent().unwrap().join("held-root")).unwrap();
    assert!(consent(
        &f,
        &uuid::Uuid::new_v4().to_string(),
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(!f.root.exists());
    assert_eq!(count(&f), 1);
}

#[test]
fn model_deletion_consent_resume_admission_checks_native_gates_and_exact_record() {
    let f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let controller = AnalysisController::default();
    for (recording, importing, cancel) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert!(controller
            .begin_model_deletion_maintenance(&f.storage, &op, recording, importing, &|| cancel)
            .is_err());
    }
    assert!(controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &uuid::Uuid::new_v4().to_string(),
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    let export = controller.begin_export().unwrap();
    assert!(controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    drop(export);
    let owner = controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
}

#[test]
fn model_deletion_consent_typed_snapshot_rejects_forged_classification_and_structure() {
    let f = Fixture::new();
    let snapshot = capture(
        &f.storage,
        &f.root,
        &f.package,
        &f.review,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let original = serde_json::to_value(&snapshot).unwrap();
    for case in [
        "classification",
        "size",
        "missing-proof",
        "order",
        "unknown-field",
        "root",
        "parent",
        "hash",
    ] {
        let mut json = original.clone();
        match case {
            "classification" => json["observations"][1]["entry"]["registered"] = false.into(),
            "size" => json["observations"][1]["entry"]["bytes"] = 0.into(),
            "missing-proof" => json["observations"][1]["proof"] = serde_json::Value::Null,
            "order" => json["observations"].as_array_mut().unwrap().swap(1, 2),
            "unknown-field" => json["executeAllowed"] = true.into(),
            "root" => json["observations"][0]["entry"]["relativePath"] = "../models".into(),
            "parent" => {
                json["observations"].as_array_mut().unwrap().remove(3);
            }
            _ => json["observations"][1]["entry"]["sha256"] = "G".repeat(64).into(),
        }
        let decoded = serde_json::from_value::<Snapshot>(json);
        assert!(
            decoded.is_err() || decoded.unwrap().validate().is_err(),
            "{case}"
        );
    }
}

#[test]
fn model_deletion_consent_rejects_disabled_reference_guards_and_unknown_schema() {
    for case in ["foreign_keys", "triggers", "schema"] {
        let f = Fixture::new();
        let p = f.preview().unwrap();
        let db = f.storage.connection.lock().unwrap();
        match case {
            "foreign_keys" => db.execute_batch("PRAGMA foreign_keys=OFF").unwrap(),
            "triggers" => {
                db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                    .unwrap();
            }
            _ => db
                .execute_batch("CREATE TABLE untrusted_extra(value TEXT)")
                .unwrap(),
        }
        drop(db);
        assert!(
            consent(
                &f,
                &uuid::Uuid::new_v4().to_string(),
                &p.census_hash,
                false,
                &lease(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{case}"
        );
        assert_eq!(count(&f), 0);
        assert_eq!(
            std::fs::read(f.path("cli")).unwrap(),
            b"synthetic; do not execute"
        );
    }
}

#[test]
fn model_deletion_consent_root_replacement_or_installing_state_never_reuses_unstarted_permission() {
    let mut f = Fixture::new();
    let p = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let held = f.root.parent().unwrap().join("held-models");
    std::fs::rename(&f.root, &held).unwrap();
    std::fs::create_dir(&f.root).unwrap();
    assert!(saved
        .verify_current(&f.storage, &owner, &crate::test_support::never_cancelled)
        .is_err());
    assert!(consent(
        &f,
        &op,
        &p.census_hash,
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    // The safe keep path does not adopt or delete the replacement directory.
    intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(f.root.exists());
    assert!(held.join(&f.package).join("cli").exists());
    reopen(&mut f, |db| {
        db.execute(
            "UPDATE maintenance_operations SET state='installing',progress_json='{}' WHERE id=?1",
            params![op],
        )
        .unwrap();
    });
    assert!(f.storage.data_write_blocked());
    assert!(intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(intent::authorize_review(&f.storage, &op).is_err());
    assert!(held.join(&f.package).join("cli").exists());
}
