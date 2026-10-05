//! Synthetic filesystem/SQLite only, with actual shared admission and fences.
use super::*;
use crate::analysis_worker::{data_maintenance::DataMaintenanceLease, AnalysisController};
use crate::storage::deletion::{fence, operations, renewal};

fn lease() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
#[test]
fn database_renewal_retains_old_generation_root_reservation_created_after_plan_switch() {
    let f = Fixture::new();
    let original = at_database_phase(&f, Phase::Finished);
    let progress = root_json(&f);
    let index = original
        .manifest
        .entries
        .iter()
        .rev()
        .position(|e| e.path == format!("meetings/{}", f.f.meeting))
        .unwrap();
    let reserved =
        f.f.storage
            .root()
            .join(format!("meetings/.mon-delete-{}-{index}", f.operation));
    reappear(&f, b"KEEP NEW RANGE");
    let review = renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
    assert!(renewal::accept_database_observed(
        &f.f.storage,
        &original,
        &review,
        true,
        &lease(),
        &|| false,
        &|event| {
            if event == "beforeReviewCommit" {
                std::fs::create_dir(&reserved).unwrap();
                std::fs::write(reserved.join("unknown"), b"KEEP OLD GENERATION").unwrap();
            }
        }
    )
    .is_err());
    assert_eq!(root_json(&f), progress);
    assert_eq!(audits(&f), 0);
    assert_eq!(
        std::fs::read(reserved.join("unknown")).unwrap(),
        b"KEEP OLD GENERATION"
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP NEW RANGE");
    f.unaffected();
}
fn reappear(f: &Fixture, bytes: &[u8]) {
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, bytes).unwrap();
}

#[test]
fn database_renewal_after_cold_completion_retains_actual_restart_fenced_admission_for_new_pending_plan(
) {
    let f = Fixture::new();
    at_database_phase(&f, Phase::RowsDeleted);
    let path = f.f.storage.root().to_path_buf();
    let operation = f.operation.clone();
    drop(f.f.storage);
    let storage = Storage::open(path).unwrap();
    let mut original = read(&storage, &operation, &|| false).unwrap();
    run(&storage, &mut original, &|| false).unwrap();
    assert!(storage.restart_required());
    let analysis = AnalysisController::default();
    analysis.require_storage_restart();
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"NEW AFTER COLD COMPLETION").unwrap();
    let review = renewal::Review::capture_database(&storage, &original, &|| false).unwrap();
    let admitted = analysis
        .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    renewal::accept_database(&storage, &original, &review, true, &admitted, &|| false).unwrap();
    drop(admitted);
    assert!(!storage.restart_required());
    assert!(storage.data_write_blocked());
    assert!(analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    assert!(analysis.begin_export().is_err());
    let admitted = analysis
        .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    let mut pending = operations::Prepared::pending(&storage, &operation, &|| false).unwrap();
    assert_eq!(pending.preview().phase, "database_files");
    assert!(pending.preview().row_phase_committed);
    let receipt = pending
        .execute(
            &storage,
            &admitted,
            &pending.preview().confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted && receipt.restart_required);
    drop(admitted);
    assert!(analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    assert!(storage.require_data_writable().is_err());
    assert!(!f.file.exists());
    assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
}

#[test]
fn database_renewal_residual_file_boundaries_cold_resume_preserve_staging_and_completed_sql_state()
{
    for phase in [Phase::RowsDeleted, Phase::Finished] {
        for point in ["staged", "unlinked", "rootRenamed", "filesRemoved"] {
            let f = Fixture::new();
            let original = at_database_phase(&f, phase);
            reappear(&f, b"RESIDUAL FILE TO DELETE");
            let mut files = accept(&f, &original);
            let stop = Cell::new(false);
            let meeting_root = f.f.storage.root().join(format!("meetings/{}", f.f.meeting));
            let result =
                removal::run_observed(&f.f.storage, &mut files, &|| stop.get(), &|event| {
                    if event == point
                        || (point == "rootRenamed" && event == "renamed" && !meeting_root.exists())
                    {
                        stop.set(true);
                    }
                });
            assert!(stop.get());
            assert_eq!(result.is_ok(), point == "filesRemoved");
            let json = files.progress_json().unwrap();
            let path = f.f.storage.root().to_path_buf();
            let operation = f.operation.clone();
            let meeting = f.f.meeting.clone();
            drop(f.f.storage);
            let storage = Storage::open(path).unwrap();
            assert!(storage.data_write_blocked());
            let mut resumed = removal::read(&storage, &operation, &|| false).unwrap();
            assert_eq!(resumed.progress_json().unwrap(), json);
            assert_eq!(resumed.progress.database_phase, Some(phase));
            removal::run(&storage, &mut resumed, &|| false).unwrap();
            let mut database = start(&storage, &resumed, &|| false).unwrap();
            assert_eq!(database.database_phase(), phase);
            run(&storage, &mut database, &|| false).unwrap();
            assert!(database.completed());
            assert!(storage.restart_required());
            assert!(storage.get_meeting(&meeting).unwrap().is_none());
            assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
            assert_eq!(
                std::fs::read(f.outside.path().join("export.md")).unwrap(),
                b"KEEP EXTERNAL EXPORT"
            );
            assert!(!f.file.exists());
        }
    }
}
fn root_json(f: &Fixture) -> String {
    f.f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT progress_json FROM maintenance_operations WHERE id=?1",
            params![f.operation],
            |r| r.get(0),
        )
        .unwrap()
}
fn evidence(f: &Fixture) -> (String, String, i64, Vec<(String, String)>) {
    let db = f.f.storage.connection.lock().unwrap();
    let (json, hash, consent) = db.query_row(
        "SELECT target_manifest_json,target_hash,consent_at FROM maintenance_operations WHERE id=?1",
        params![f.operation], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let mut q = db.prepare("SELECT relative_path,identity_json FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").unwrap();
    let paths = q
        .query_map(params![f.operation], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (json, hash, consent, paths)
}
fn audits(f: &Fixture) -> i64 {
    f.f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM maintenance_operations WHERE id!=?1 AND kind='delete_meeting'",
            params![f.operation],
            |r| r.get(0),
        )
        .unwrap()
}
fn accept(f: &Fixture, receipt: &Receipt) -> removal::Receipt {
    let review = renewal::Review::capture_database(&f.f.storage, receipt, &|| false).unwrap();
    renewal::accept_database(&f.f.storage, receipt, &review, true, &lease(), &|| false).unwrap()
}

#[test]
fn database_renewal_all_three_stages_preserve_original_evidence_and_resume_without_row_recreation()
{
    for phase in [Phase::RowsPending, Phase::RowsDeleted, Phase::Finished] {
        let f = Fixture::new();
        let original = at_database_phase(&f, phase);
        let fixed = evidence(&f);
        let old_progress = root_json(&f);
        reappear(&f, b"NEW CURRENT FILE");
        let review = renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
        assert_eq!(root_json(&f), old_progress);
        assert_eq!(audits(&f), 0);
        if phase.rows_committed() {
            assert!(renewal::accept_database(
                &f.f.storage,
                &original,
                &review,
                false,
                &lease(),
                &|| false
            )
            .is_err());
            assert_eq!(root_json(&f), old_progress);
        }
        let mut files =
            renewal::accept_database(&f.f.storage, &original, &review, true, &lease(), &|| false)
                .unwrap();
        assert_eq!(files.progress.database_phase, Some(phase));
        assert_eq!(files.processed_paths(), 0);
        assert_eq!(evidence(&f), fixed);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"NEW CURRENT FILE");
        assert_eq!(
            f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some(),
            !phase.rows_committed()
        );
        let db = f.f.storage.connection.lock().unwrap();
        assert!(fence::has_pending(&db).unwrap());
        assert!(f.f.storage.data_write_blocked());
        assert!(original.verify_saved(&db, &|| false).is_err());
        let reference = files.progress.confirmation.as_ref().unwrap();
        let json: String = db
            .query_row(
                "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                params![reference.operation_id],
                |r| r.get(0),
            )
            .unwrap();
        let record: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            record["previousProgress"],
            serde_json::to_value(original.file_progress()).unwrap()
        );
        assert_eq!(
            record["previousDatabasePhase"],
            serde_json::to_value(phase).unwrap()
        );
        drop(db);
        assert_eq!(operations::listing(&f.f.storage).unwrap().items.len(), 1);
        removal::run(&f.f.storage, &mut files, &|| false).unwrap();
        let mut restored = start(&f.f.storage, &files, &|| false).unwrap();
        assert_eq!(restored.database_phase(), phase);
        let replay = start(&f.f.storage, &files, &|| false).unwrap();
        assert_eq!(
            replay.progress_json().unwrap(),
            restored.progress_json().unwrap()
        );
        run(&f.f.storage, &mut restored, &|| false).unwrap();
        assert!(restored.completed());
        assert!(!f.file.exists());
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
        assert_eq!(audits(&f), 1);
        assert_eq!(evidence(&f), fixed);
        assert!(operations::listing(&f.f.storage).unwrap().items.is_empty());
        f.unaffected();
    }
}

#[test]
fn database_renewal_cancellation_rolls_back_audit_and_plan_at_all_precommit_boundaries() {
    for phase in [Phase::RowsDeleted, Phase::Finished] {
        for point in [
            "reviewIntentStored",
            "reviewPathStored",
            "reviewRecorded",
            "beforeReviewCommit",
        ] {
            let f = Fixture::new();
            let original = at_database_phase(&f, phase);
            let fixed = evidence(&f);
            let progress = root_json(&f);
            reappear(&f, b"KEEP CANCELLED CURRENT FILE");
            let review =
                renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
            let stop = Cell::new(false);
            assert!(renewal::accept_database_observed(
                &f.f.storage,
                &original,
                &review,
                true,
                &lease(),
                &|| stop.get(),
                &|event| {
                    if event == point {
                        stop.set(true);
                    }
                }
            )
            .is_err());
            assert!(stop.get());
            assert_eq!(audits(&f), 0);
            assert_eq!(root_json(&f), progress);
            assert_eq!(evidence(&f), fixed);
            assert_eq!(
                std::fs::read(&f.file).unwrap(),
                b"KEEP CANCELLED CURRENT FILE"
            );
            assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
            f.unaffected();
        }
    }
}

#[test]
fn database_renewal_committed_cancel_cold_reopen_retains_stage_and_fence_until_resolution_and_restart(
) {
    for phase in [Phase::RowsPending, Phase::RowsDeleted, Phase::Finished] {
        let f = Fixture::new();
        let original = at_database_phase(&f, phase);
        reappear(&f, b"KEEP COMMITTED CURRENT FILE");
        let review = renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
        let stop = Cell::new(false);
        let mut files = renewal::accept_database_observed(
            &f.f.storage,
            &original,
            &review,
            true,
            &lease(),
            &|| stop.get(),
            &|event| {
                if event == "reviewCommitted" {
                    stop.set(true);
                }
            },
        )
        .unwrap();
        assert!(removal::run(&f.f.storage, &mut files, &|| stop.get()).is_err());
        let path = f.f.storage.root().to_path_buf();
        let operation = f.operation.clone();
        let meeting = f.f.meeting.clone();
        let other = f.f.other.clone();
        drop(f.f.storage);
        let storage = Storage::open(path.clone()).unwrap();
        assert!(storage.data_write_blocked());
        assert_eq!(operations::listing(&storage).unwrap().items.len(), 1);
        operations::authorize_review(&storage, Some(&operation)).unwrap();
        let mut saved = removal::read(&storage, &operation, &|| false).unwrap();
        assert_eq!(saved.progress.database_phase, Some(phase));
        assert_eq!(saved.progress.confirmation.as_ref().unwrap().generation, 1);
        assert!(storage.require_data_writable().is_err());
        removal::run(&storage, &mut saved, &|| false).unwrap();
        let mut restored = start(&storage, &saved, &|| false).unwrap();
        assert_eq!(restored.database_phase(), phase);
        run(&storage, &mut restored, &|| false).unwrap();
        assert!(storage.require_data_writable().is_err());
        assert!(storage.restart_required());
        assert!(operations::listing(&storage).unwrap().items.is_empty());
        assert!(storage.get_meeting(&meeting).unwrap().is_none());
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
        drop(storage);
        let reopened = Storage::open(path).unwrap();
        reopened.require_data_writable().unwrap();
        assert!(!f.file.exists());
    }
}

#[test]
fn database_renewal_chained_reviews_preserve_monotonic_row_and_purge_stages() {
    let f = Fixture::new();
    let first = at_database_phase(&f, Phase::RowsPending);
    let fixed = evidence(&f);
    reappear(&f, b"FIRST GENERATION");
    let mut files = accept(&f, &first);
    removal::run(&f.f.storage, &mut files, &|| false).unwrap();
    let mut rows = start(&f.f.storage, &files, &|| false).unwrap();
    let stop = Cell::new(false);
    assert!(
        run_inner(&f.f.storage, &mut rows, &|| stop.get(), &|event| {
            if event == "rowsCommitted" {
                stop.set(true);
            }
        })
        .is_err()
    );
    assert_eq!(rows.database_phase(), Phase::RowsDeleted);
    reappear(&f, b"SECOND GENERATION");
    let mut files = accept(&f, &rows);
    assert_eq!(files.progress.database_phase, Some(Phase::RowsDeleted));
    assert_eq!(files.progress.confirmation.as_ref().unwrap().generation, 2);
    removal::run(&f.f.storage, &mut files, &|| false).unwrap();
    let mut purged = start(&f.f.storage, &files, &|| false).unwrap();
    run(&f.f.storage, &mut purged, &|| false).unwrap();
    reappear(&f, b"THIRD GENERATION");
    let mut files = accept(&f, &purged);
    assert_eq!(files.progress.database_phase, Some(Phase::Finished));
    assert_eq!(files.progress.confirmation.as_ref().unwrap().generation, 3);
    // A change during the residual file phase requires another explicit review,
    // but may not drop the previously committed DB phase.
    std::fs::write(&f.file, b"FOURTH GENERATION").unwrap();
    let review = renewal::Review::capture(&f.f.storage, &files, &|| false).unwrap();
    renewal::accept(&f.f.storage, &mut files, &review, true, &lease(), &|| false).unwrap();
    assert_eq!(files.progress.database_phase, Some(Phase::Finished));
    assert_eq!(files.progress.confirmation.as_ref().unwrap().generation, 4);
    removal::run(&f.f.storage, &mut files, &|| false).unwrap();
    let mut final_receipt = start(&f.f.storage, &files, &|| false).unwrap();
    assert_eq!(final_receipt.database_phase(), Phase::Finished);
    run(&f.f.storage, &mut final_receipt, &|| false).unwrap();
    assert_eq!(evidence(&f), fixed);
    assert_eq!(audits(&f), 4);
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    f.unaffected();
}

#[test]
fn database_renewal_rejects_downgraded_or_forgotten_phase_and_does_not_revert_completed_sql_state()
{
    let f = Fixture::new();
    let original = at_database_phase(&f, Phase::Finished);
    reappear(&f, b"KEEP INVALID PLAN FILE");
    let files = accept(&f, &original);
    let json = root_json(&f);
    let db = f.f.storage.connection.lock().unwrap();
    let _writer = fence::write_removal(&f.f.storage, &db, &files, &|| false).unwrap();
    for (version, phase) in [
        (2, None),
        (3, Some("rowsPending")),
        (3, Some("rowsDeleted")),
        (1, Some("finished")),
    ] {
        let mut changed: serde_json::Value = serde_json::from_str(&json).unwrap();
        changed["version"] = version.into();
        if let Some(phase) = phase {
            changed["databasePhase"] = phase.into();
        } else {
            changed.as_object_mut().unwrap().remove("databasePhase");
        }
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![f.operation, changed.to_string()],
        )
        .unwrap();
        assert!(removal::Receipt::load(&db, &f.operation, &|| false).is_err());
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![f.operation, json],
        )
        .unwrap();
    }
    assert!(db
        .execute(
            "UPDATE maintenance_operations SET state='cleanup' WHERE id=?1",
            params![f.operation]
        )
        .is_err());
    files.verify_saved(&db, &|| false).unwrap();
    drop(_writer);
    drop(db);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP INVALID PLAN FILE");
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    f.unaffected();
}

#[test]
fn database_renewal_native_preview_requires_new_id_and_unknown_decision_then_finishes_each_db_stage(
) {
    for phase in [Phase::RowsPending, Phase::RowsDeleted, Phase::Finished] {
        let f = Fixture::new();
        at_database_phase(&f, phase);
        reappear(&f, b"CURRENT API TARGET");
        let mut prepared =
            operations::Prepared::pending(&f.f.storage, &f.operation, &|| false).unwrap();
        let old = prepared.preview();
        assert!(!old.execute_allowed && old.reconfirmation_allowed);
        let admitted = lease();
        prepared
            .review_remaining(&f.f.storage, &admitted, &old.confirmation_id, &|| false)
            .unwrap();
        let next = prepared.preview();
        assert_eq!(next.phase, "database_reconfirmation");
        assert_eq!(next.row_phase_committed, phase.rows_committed());
        assert_ne!(next.confirmation_id, old.confirmation_id);
        assert_eq!(next.processed_paths, 0);
        assert_eq!(next.previous_processed_paths, next.previous_path_count);
        assert!(!next.keep_allowed && !next.reconfirmation_allowed && next.execute_allowed);
        assert_eq!(audits(&f), 0);
        assert!(prepared
            .execute(
                &f.f.storage,
                &admitted,
                &old.confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .is_err());
        if phase.rows_committed() {
            assert!(prepared
                .execute(
                    &f.f.storage,
                    &admitted,
                    &next.confirmation_id,
                    false,
                    &|| false,
                    &|_| {}
                )
                .is_err());
            assert_eq!(audits(&f), 0);
            prepared
                .review_remaining(
                    &f.f.storage,
                    &admitted,
                    &prepared.preview().confirmation_id,
                    &|| false,
                )
                .unwrap();
        }
        let current = prepared.preview();
        let outcome = prepared
            .execute(
                &f.f.storage,
                &admitted,
                &current.confirmation_id,
                true,
                &|| false,
                &|_| {},
            )
            .unwrap();
        assert!(outcome.deleted && !outcome.retained);
        assert_eq!(prepared.preview().phase, "completed");
        assert_eq!(audits(&f), 1);
        f.unaffected();
    }
}

#[test]
fn database_renewal_native_cancel_after_publication_reloads_fixed_stage_and_does_not_reconsent_on_retry(
) {
    let f = Fixture::new();
    at_database_phase(&f, Phase::Finished);
    reappear(&f, b"KEEP PUBLISHED API TARGET");
    let mut prepared =
        operations::Prepared::pending(&f.f.storage, &f.operation, &|| false).unwrap();
    let admitted = lease();
    prepared
        .review_remaining(
            &f.f.storage,
            &admitted,
            &prepared.preview().confirmation_id,
            &|| false,
        )
        .unwrap();
    let stop = Cell::new(false);
    assert!(prepared
        .execute(
            &f.f.storage,
            &admitted,
            &prepared.preview().confirmation_id,
            true,
            &|| stop.get(),
            &|event| {
                if event == "delete_scope_fixed" {
                    stop.set(true);
                }
            }
        )
        .is_err());
    let stopped = prepared.preview();
    assert_eq!(stopped.phase, "database_files");
    assert!(stopped.row_phase_committed);
    assert_eq!(audits(&f), 1);
    assert_eq!(
        std::fs::read(&f.file).unwrap(),
        b"KEEP PUBLISHED API TARGET"
    );
    prepared.refresh(&f.f.storage, &|| false).unwrap();
    let current = prepared.preview();
    assert!(current.execute_allowed && !current.reconfirmation_allowed);
    let result = prepared
        .execute(
            &f.f.storage,
            &admitted,
            &current.confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(result.deleted);
    assert_eq!(audits(&f), 1);
    f.unaffected();
}

#[test]
fn database_renewal_refuses_changed_review_and_cancelled_actual_lease_without_new_audit() {
    let f = Fixture::new();
    let original = at_database_phase(&f, Phase::RowsDeleted);
    reappear(&f, b"FIRST REVIEW");
    let progress = root_json(&f);
    let review = renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
    std::fs::write(&f.file, b"CHANGED AFTER REVIEW").unwrap();
    assert!(
        renewal::accept_database(&f.f.storage, &original, &review, true, &lease(), &|| false)
            .is_err()
    );
    let review = renewal::Review::capture_database(&f.f.storage, &original, &|| false).unwrap();
    let controller = AnalysisController::default();
    let admitted = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    controller.begin_exit().unwrap();
    assert!(
        renewal::accept_database(&f.f.storage, &original, &review, true, &admitted, &|| false)
            .is_err()
    );
    assert_eq!(root_json(&f), progress);
    assert_eq!(audits(&f), 0);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"CHANGED AFTER REVIEW");
    f.unaffected();
}
