//! Synthetic model bodies, temporary SQLite and actual shared leases only.
use super::*;
use crate::{
    analysis_worker::{data_maintenance::DataMaintenanceLease, AnalysisController},
    storage::deletion::models::{self, intent, tests::Fixture},
};
use rusqlite::config::DbConfig;
use std::cell::Cell;

fn lease(f: &Fixture, op: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_model_deletion_maintenance(&f.storage, op, false, false, &|| false)
        .unwrap()
}
fn finish(f: &Fixture) -> String {
    let op = uuid::Uuid::new_v4().to_string();
    let owner = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let view = models::preview(&f.storage, &f.root, &op, &f.package, &f.review, &|| false).unwrap();
    intent::prepare(
        &f.storage,
        &f.root,
        &op,
        &f.package,
        &f.review,
        &view.census_hash,
        false,
        &owner,
        &|| false,
    )
    .unwrap();
    removal::start(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(removal::run(&f.storage, &op, &owner, &|| false)
        .unwrap()
        .completed());
    op
}
fn reason(f: &Fixture, op: &str) -> Option<String> {
    super::super::super::startup::reason(&f.storage.connection.lock().unwrap(), op).unwrap()
}
fn fixed(f: &Fixture, op: &str) -> String {
    f.storage.connection.lock().unwrap().query_row("SELECT json_array(target_manifest_json,target_hash,consent_at,created_at,(SELECT json_group_array(json_array(relative_path,identity_json,identity_hash)) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1", [op], |r| r.get(0)).unwrap()
}
fn reappear(f: &Fixture) {
    std::fs::create_dir(f.path("")).unwrap();
    std::fs::write(f.path("NEW"), b"NEW EXPLICIT CONSENT ONLY").unwrap();
}
fn accept(f: &Fixture, op: &str, owner: &DataMaintenanceLease) {
    let view = renewal::review(&f.storage, op, owner, &|| false).unwrap();
    assert!(
        renewal::accept(&f.storage, &view, view.census_hash(), false, owner, &|| {
            false
        })
        .is_err()
    );
    renewal::accept(&f.storage, &view, view.census_hash(), true, owner, &|| {
        false
    })
    .unwrap();
}
fn stale(f: &Fixture) {
    let db = f.storage.connection.lock().unwrap();
    let meeting: String = db
        .query_row("SELECT id FROM meetings LIMIT 1", [], |r| r.get(0))
        .unwrap();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale',?1,'transcription','running','stale',?2,'{}','{}',?2,1,1)", params![meeting,"a".repeat(64)]).unwrap();
}
fn job_state(f: &Fixture) -> String {
    f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM processing_jobs WHERE id='stale'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn history_v27_completed_model_upgrades_without_rewriting_evidence_or_reviving_bodies() {
    let f = Fixture::new();
    let op = finish(&f);
    {
        let db = f.storage.connection.lock().unwrap();
        intent::historical_header_fixture(&db, &op);
        crate::storage::edit_recovery::remove_schema_for_test(&db);
    }
    let saved = fixed(&f, &op);
    let f = f.cold();
    assert_eq!(fixed(&f, &op), saved);
    assert_eq!(reason(&f, &op), None);
    assert!(!f.storage.data_write_blocked());
    assert!(!f.path("").exists());
    assert_eq!(
        crate::storage::verify_migration_history(&f.storage.connection.lock().unwrap()).unwrap(),
        crate::storage::MIGRATIONS.len()
    );
    f.storage
        .create_meeting("MODEL HISTORY IS READABLE AFTER UPGRADE")
        .unwrap();
    let f = f.cold();
    assert_eq!(fixed(&f, &op), saved);
    assert!(!f.storage.data_write_blocked());
}

#[test]
fn history_old_model_consent_can_be_read_but_never_starts_current_deletion() {
    let f = Fixture::new();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let view = models::preview(&f.storage, &f.root, &op, &f.package, &f.review, &|| false).unwrap();
    intent::prepare(
        &f.storage,
        &f.root,
        &op,
        &f.package,
        &f.review,
        &view.census_hash,
        false,
        &owner,
        &|| false,
    )
    .unwrap();
    {
        let db = f.storage.connection.lock().unwrap();
        // Synthetic old fixed intent on the latest test DB: validating a
        // historical header must not admit it to a new deletion start.
        let _writer = crate::storage::deletion::fence::write_model_prepared(
            &f.storage,
            &db,
            &intent::read(&db, &op).unwrap(),
            &owner,
        )
        .unwrap();
        intent::historical_header_fixture(&db, &op);
        intent::read(&db, &op).unwrap();
    }
    let saved = fixed(&f, &op);
    assert!(removal::start(&f.storage, &op, &owner, &|| false).is_err());
    assert_eq!(fixed(&f, &op), saved);
    assert!(f.path("").exists());
    assert_eq!(f.operation_state(&op), "prepared");
}

#[test]
fn model_startup_clean_completion_allows_ordinary_recovery_and_keeps_original() {
    let f = Fixture::new();
    let op = finish(&f);
    let saved = fixed(&f, &op);
    stale(&f);
    let f = f.cold();
    assert_eq!(reason(&f, &op), None);
    assert!(!f.storage.data_write_blocked());
    assert_eq!(job_state(&f), "interrupted");
    assert_eq!(fixed(&f, &op), saved);
    let owner = lease(&f, &op);
    assert!(removal::run(&f.storage, &op, &owner, &|| false)
        .unwrap()
        .completed());
}

#[test]
fn model_startup_reappearance_is_memory_only_requires_new_consent_and_cold_restart() {
    let f = Fixture::new();
    let op = finish(&f);
    let saved = fixed(&f, &op);
    reappear(&f);
    let keep = f.root.join("UNRELATED");
    std::fs::write(&keep, b"KEEP").unwrap();
    let f = f.cold();
    assert_eq!(reason(&f, &op).as_deref(), Some(REAPPEARED));
    assert_eq!(fixed(&f, &op), saved);
    assert_eq!(f.operation_state(&op), "completed");
    assert!(f.storage.create_meeting("DENIED").is_err());
    assert!(f
        .storage
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM temp.mon_deletion_startup_attention", [])
        .is_err());
    let f = f.cold();
    assert_eq!(reason(&f, &op).as_deref(), Some(REAPPEARED));
    let owner = lease(&f, &op);
    assert!(removal::run(&f.storage, &op, &owner, &|| false).is_err());
    assert!(f.path("NEW").exists());
    accept(&f, &op, &owner);
    assert_eq!(f.operation_state(&op), "completed");
    removal::run(&f.storage, &op, &owner, &|| false).unwrap();
    assert_eq!(reason(&f, &op), None);
    assert_eq!(fixed(&f, &op), saved);
    assert!(f.storage.restart_required());
    assert!(f.storage.create_meeting("STILL DENIED").is_err());
    assert_eq!(std::fs::read(keep).unwrap(), b"KEEP");
    drop(owner);
    let f = f.cold();
    assert!(!f.storage.data_write_blocked());
    assert!(f.storage.create_meeting("AFTER RESTART").is_ok());
}

#[test]
fn model_startup_defers_stale_jobs_but_never_bypasses_actual_worker_admission() {
    let f = Fixture::new();
    let op = finish(&f);
    stale(&f);
    reappear(&f);
    let f = f.cold();
    assert_eq!(job_state(&f), "running");
    let controller = AnalysisController::default();
    for (recording, importing, cancelled) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert!(controller
            .begin_model_deletion_maintenance(&f.storage, &op, recording, importing, &|| cancelled)
            .is_err());
    }
    let owner = controller
        .begin_model_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .unwrap();
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    accept(&f, &op, &owner);
    removal::run(&f.storage, &op, &owner, &|| false).unwrap();
    assert_eq!(job_state(&f), "running");
    drop(owner);
    let f = f.cold();
    assert_eq!(job_state(&f), "interrupted");
}

#[test]
fn model_startup_exact_reserved_root_is_reviewed_without_adopting_neighbors() {
    let f = Fixture::new();
    let op = finish(&f);
    let reserved = f.root.join(format!(".mon-delete-model-{op}-4"));
    let neighbor = f.root.join(format!(".mon-delete-model-{op}-250"));
    for path in [&reserved, &neighbor] {
        std::fs::create_dir(path).unwrap();
        std::fs::write(path.join("NEW"), b"KEEP UNLESS EXACT").unwrap();
    }
    let f = f.cold();
    assert_eq!(reason(&f, &op).as_deref(), Some(REAPPEARED));
    let owner = lease(&f, &op);
    let view = renewal::review(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(view.entries().iter().any(|e| e
        .relative_path
        .starts_with(reserved.file_name().unwrap().to_str().unwrap())));
    assert!(!view
        .entries()
        .iter()
        .any(|e| e.relative_path.contains("-250")));
    renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
        false
    })
    .unwrap();
    removal::run(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(!reserved.exists());
    assert!(neighbor.join("NEW").exists());
}

#[test]
fn model_startup_changed_completed_receipt_never_unlocks_or_rewrites_evidence() {
    for change in ["progress", "proof", "oversized"] {
        let f = Fixture::new();
        let op = finish(&f);
        stale(&f);
        {
            let db = f.storage.connection.lock().unwrap();
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            db.execute_batch("PRAGMA ignore_check_constraints=ON;")
                .unwrap();
            match change {
                "progress" => {
                    db.execute(
                        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                        [&op],
                    )
                    .unwrap();
                }
                "proof" => {
                    db.execute("DELETE FROM maintenance_paths WHERE operation_id=?1 AND relative_path='model-deletion-proof/000000'", [&op]).unwrap();
                }
                _ => {
                    db.execute(
                        "UPDATE maintenance_operations SET target_manifest_json=?2 WHERE id=?1",
                        params![op, "x".repeat(65537)],
                    )
                    .unwrap();
                }
            }
        }
        let f = f.cold();
        assert_eq!(reason(&f, &op).as_deref(), Some(INVALID), "{change}");
        assert!(f.storage.data_write_blocked());
        assert_eq!(job_state(&f), "running");
        let listing = models::operations::listing(&f.storage, &f.root).unwrap();
        assert!(!listing.items[0].review_allowed);
    }
}

#[test]
fn model_startup_regular_file_and_link_are_retained_without_following() {
    for link in [false, true] {
        let f = Fixture::new();
        let op = finish(&f);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("KEEP"), b"OUTSIDE").unwrap();
        if link {
            std::os::unix::fs::symlink(outside.path(), f.path("")).unwrap();
        } else {
            std::fs::write(f.path(""), b"NOT A DIRECTORY").unwrap();
        }
        let f = f.cold();
        assert_eq!(
            reason(&f, &op).as_deref(),
            Some(if link { UNINSPECTABLE } else { REAPPEARED })
        );
        let owner = lease(&f, &op);
        assert!(renewal::review(&f.storage, &op, &owner, &|| false).is_err());
        assert!(f.storage.data_write_blocked());
        assert_eq!(
            std::fs::read(outside.path().join("KEEP")).unwrap(),
            b"OUTSIDE"
        );
    }
}

#[test]
fn model_startup_native_root_identity_replacement_is_not_adopted() {
    let f = Fixture::new();
    let op = finish(&f);
    let moved = f.root.with_file_name("old-models");
    std::fs::rename(&f.root, &moved).unwrap();
    std::fs::create_dir(&f.root).unwrap();
    let f = f.cold();
    assert_eq!(reason(&f, &op).as_deref(), Some(UNINSPECTABLE));
    let owner = lease(&f, &op);
    assert!(removal::run(&f.storage, &op, &owner, &|| false).is_err());
    assert!(f.storage.data_write_blocked());
}

#[test]
fn model_startup_wrong_native_root_never_inspects_saved_foreign_root() {
    let f = Fixture::new();
    let op = finish(&f);
    let foreign = tempfile::tempdir().unwrap();
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(
        inspect(
            &db,
            &f.storage.managed_anchor,
            &foreign.path().join("models")
        )
        .unwrap(),
        vec![(op, INVALID)]
    );
    assert!(!foreign.path().join("models").exists());
}

#[test]
fn model_startup_active_reconfirmation_chain_survives_cold_restart_and_rejects_orphan_marker() {
    let f = Fixture::new();
    let op = finish(&f);
    reappear(&f);
    let f = f.cold();
    let owner = lease(&f, &op);
    accept(&f, &op, &owner);
    drop(owner);
    let f = f.cold();
    assert_eq!(reason(&f, &op), None); // Persistent v3 removing fence, not corrupt completion.
    assert!(f.storage.data_write_blocked());
    let owner = lease(&f, &op);
    removal::run(&f.storage, &op, &owner, &|| false).unwrap();
    drop(owner);
    let f = f.cold();
    assert!(!f.storage.data_write_blocked());
    let orphan = uuid::Uuid::new_v4().to_string();
    f.storage.connection.lock().unwrap().execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) SELECT ?1,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at FROM maintenance_operations WHERE progress_json='{\"modelReconfirmationRecorded\":true}'", [&orphan]).unwrap();
    let f = f.cold();
    assert_eq!(reason(&f, &op), None);
    assert_eq!(reason(&f, &orphan).as_deref(), Some(INVALID));
    assert!(f.storage.data_write_blocked());
}

#[test]
fn model_startup_independent_completed_hints_do_not_unlock_each_other() {
    let f = Fixture::new();
    let first = finish(&f);
    let second = finish(&f);
    reappear(&f);
    let f = f.cold();
    assert_eq!(reason(&f, &first).as_deref(), Some(REAPPEARED));
    assert_eq!(reason(&f, &second).as_deref(), Some(REAPPEARED));
    let owner = lease(&f, &first);
    accept(&f, &first, &owner);
    removal::run(&f.storage, &first, &owner, &|| false).unwrap();
    assert_eq!(reason(&f, &first), None);
    assert!(reason(&f, &second).is_some());
    assert!(f.storage.data_write_blocked());
    drop(owner);
    let owner = lease(&f, &second);
    removal::run(&f.storage, &second, &owner, &|| false).unwrap();
    assert_eq!(reason(&f, &second), None);
    assert!(f.storage.restart_required());
}

#[test]
fn model_startup_final_reappearance_or_cancel_preserves_hint_for_explicit_ack() {
    for cancelled in [false, true] {
        let f = Fixture::new();
        let op = finish(&f);
        reappear(&f);
        let f = f.cold();
        let owner = lease(&f, &op);
        accept(&f, &op, &owner);
        let stop = Cell::new(false);
        assert!(
            removal::run_observed(&f.storage, &op, &owner, &|| stop.get(), &|event| {
                if event == "completed" {
                    if cancelled {
                        stop.set(true);
                    } else {
                        reappear(&f);
                    }
                }
            })
            .is_err()
        );
        assert!(reason(&f, &op).is_some());
        assert!(f.storage.data_write_blocked());
        stop.set(false);
        if !cancelled {
            assert!(f.path("NEW").exists());
            accept(&f, &op, &owner);
        } else {
            assert!(!f.path("").exists());
        }
        removal::run(&f.storage, &op, &owner, &|| false).unwrap();
        assert_eq!(reason(&f, &op), None);
    }
}

#[test]
fn model_startup_database_identity_replacement_defers_recovery_and_retains_receipt() {
    let mut f = Fixture::new();
    let op = finish(&f);
    let saved = fixed(&f, &op);
    stale(&f);
    let root = f.storage.root().to_path_buf();
    let placeholder = tempfile::tempdir().unwrap();
    let previous = std::mem::replace(
        &mut f.storage,
        Storage::open(placeholder.path().into()).unwrap(),
    );
    drop(previous);
    let database = root.join("meetings.sqlite3");
    let old = root.join("synthetic-old.sqlite3");
    std::fs::rename(&database, &old).unwrap();
    std::fs::copy(&old, &database).unwrap();
    f.storage = Storage::open_with_models(root, f.root.clone()).unwrap();
    assert_eq!(reason(&f, &op).as_deref(), Some(INVALID));
    assert_eq!(fixed(&f, &op), saved);
    assert_eq!(job_state(&f), "running");
    assert!(f.storage.create_meeting("DENIED").is_err());
    assert!(
        !models::operations::listing(&f.storage, &f.root)
            .unwrap()
            .items[0]
            .review_allowed
    );
}

#[test]
fn model_startup_completed_scan_caps_history_and_rejects_noncanonical_ids() {
    let f = Fixture::new();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE maintenance_operations(id TEXT,kind TEXT,state TEXT,target_manifest_json TEXT,progress_json TEXT,created_at INTEGER); INSERT INTO maintenance_operations VALUES('invalid','delete_assets','completed','{}','{}',1);").unwrap();
    assert!(inspect(
        &db,
        &f.storage.managed_anchor,
        Path::new("/synthetic/models")
    )
    .is_err());
    db.execute_batch("DELETE FROM maintenance_operations; WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<10001) INSERT INTO maintenance_operations SELECT printf('00000000-0000-0000-0000-%012d',n),'delete_assets','completed','{}','{}',n FROM seq;").unwrap();
    assert!(inspect(
        &db,
        &f.storage.managed_anchor,
        Path::new("/synthetic/models")
    )
    .unwrap_err()
    .contains("上限"));
}

#[test]
fn model_startup_noncanonical_completed_asset_stops_open_before_mutable_recovery() {
    let mut f = Fixture::new();
    stale(&f);
    f.storage.connection.lock().unwrap().execute("INSERT INTO maintenance_operations VALUES('older',NULL,'delete_assets','completed',NULL,'{}',?1,1,'{}',NULL,1,1)", ["a".repeat(64)]).unwrap();
    let root = f.storage.root().to_path_buf();
    let placeholder = tempfile::tempdir().unwrap();
    drop(std::mem::replace(
        &mut f.storage,
        Storage::open(placeholder.path().into()).unwrap(),
    ));
    let error = match Storage::open_with_models(root.clone(), f.root.clone()) {
        Ok(_) => panic!("Malformed completed asset must not be adopted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("操作IDが不正"));
    let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM processing_jobs WHERE id='stale'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "running");
    let retained: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id='older'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, "{}");
}
