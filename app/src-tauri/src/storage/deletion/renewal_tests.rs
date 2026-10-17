//! Real temporary Storage + shared admission, never user audio or client scopes.
use super::*;
use crate::analysis_worker::{data_maintenance::DataMaintenanceLease, AnalysisController};
use crate::storage::deletion::{completion, operations, renewal};

#[path = "root_renewal_tests.rs"]
mod root_renewal_tests;

fn lease() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap()
}
fn snapshot(f: &Fixture) -> (String, String, i64, String) {
    let db = f.storage.connection.lock().unwrap();
    let (json, hash, consent): (String, String, i64) = db.query_row(
        "SELECT target_manifest_json,target_hash,consent_at FROM maintenance_operations WHERE id=?1",
        params![f.operation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let mut q = db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").unwrap();
    let paths = q
        .query_map(params![f.operation], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (json, hash, consent, serde_json::to_string(&paths).unwrap())
}
fn audit_count(f: &Fixture) -> i64 {
    f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM maintenance_operations WHERE kind='delete_meeting' AND id!=?1",
            params![f.operation],
            |r| r.get(0),
        )
        .unwrap()
}
fn pause_first(f: &Fixture, r: &mut Receipt, phase: &str) {
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, r, &|| stop.get(), &|p| {
        if p == phase {
            stop.set(true);
        }
    })
    .is_err());
    assert!(stop.get());
}
fn review(f: &Fixture, r: &Receipt) -> renewal::Review {
    renewal::Review::capture(&f.storage, r, &crate::test_support::never_cancelled).unwrap()
}
fn accept(f: &Fixture, r: &mut Receipt, include: bool) {
    let v = review(f, r);
    renewal::accept(
        &f.storage,
        r,
        &v,
        include,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
}
fn finish(f: &Fixture, r: &mut Receipt) {
    run(&f.storage, r, &crate::test_support::never_cancelled).unwrap();
    let mut db_phase =
        completion::start(&f.storage, r, &crate::test_support::never_cancelled).unwrap();
    completion::run(
        &f.storage,
        &mut db_phase,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(db_phase.completed());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
    assert!(!f
        .storage
        .root()
        .join(format!("meetings/{}", f.meeting))
        .exists());
    assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
    assert_eq!(std::fs::read(&f.external).unwrap(), b"KEEP EXTERNAL EXPORT");
    assert_eq!(
        std::fs::read(f.storage.root().parent().unwrap().join("models/model.gguf")).unwrap(),
        b"KEEP MODEL"
    );
}

#[test]
fn rootless_renewal_uses_current_v3_and_rejects_noncurrent_audit_versions_without_conversion() {
    let base = tempfile::tempdir().unwrap();
    let storage = Storage::open(base.path().join("root")).unwrap();
    let meeting = storage.create_meeting("EMPTY CURRENT MEETING").unwrap();
    let keep = storage.create_meeting("KEEP CURRENT MEETING").unwrap();
    let census = crate::storage::deletion::files::Census::capture(
        &storage,
        &meeting.id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(census.manifest().entries.is_empty());
    let operation = uuid::Uuid::new_v4().to_string();
    let prepared = crate::storage::deletion::intent::prepare(
        &storage,
        &operation,
        &census,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let reviewed = Reviewed::confirm(&census, false).unwrap();
    let mut receipt = start(
        &storage,
        &prepared,
        &reviewed,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let view = renewal::Review::capture(&storage, &receipt, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(view.manifest().reservation_roots.is_empty());
    renewal::accept(
        &storage,
        &mut receipt,
        &view,
        false,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let audit = receipt
        .progress
        .confirmation
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    let saved: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&audit],
            |r| r.get(0),
        )
        .unwrap();
    let current: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(current["version"], 3);
    assert_eq!(renewal::audit_source(&saved).unwrap(), operation);
    let tail = saved.strip_prefix("{\"version\":3,").unwrap();
    for version in [0, 1, 2, 5] {
        let unsupported = format!("{{\"version\":{version},{tail}");
        assert!(renewal::audit_source(&unsupported).is_err());
    }
    run(
        &storage,
        &mut receipt,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut completed =
        completion::start(&storage, &receipt, &crate::test_support::never_cancelled).unwrap();
    completion::run(
        &storage,
        &mut completed,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let root = storage.root().to_owned();
    drop(storage);
    let reopened = Storage::open(root).unwrap();
    assert!(!reopened.data_write_blocked());
    assert_eq!(reopened.get_meeting(&keep.id).unwrap().unwrap(), keep);
    assert!(reopened.get_meeting(&meeting.id).unwrap().is_none());
    let after: String = reopened
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&audit],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, saved);
}

#[test]
fn renewal_partial_deletion_requires_separate_unknown_consent_and_preserves_original_evidence() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let original = snapshot(&f);
    pause_first(&f, &mut r, "advanced");
    assert!(!f.file.exists());
    let old_progress = r.progress_json().unwrap();
    let extra = f.file.parent().unwrap().join("extra.txt");
    std::fs::write(&extra, b"NEW UNKNOWN FILE").unwrap();
    assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    let v = review(&f, &r);
    assert_eq!(audit_count(&f), 0); // Inspection has no mutation or consent.
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &v,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(old_progress, r.progress_json().unwrap());
    assert_eq!(audit_count(&f), 0);
    assert!(extra.exists());
    renewal::accept(
        &f.storage,
        &mut r,
        &v,
        true,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(r.progress.next, 0); // This generation's remaining plan, not restored files.
    assert_eq!(r.progress.confirmation.as_ref().unwrap().generation, 1);
    assert_eq!(audit_count(&f), 1);
    assert_eq!(original, snapshot(&f));
    assert!(!f.file.exists());
    assert!(extra.exists()); // Recording renewed consent does not delete anything.
    let db = f.storage.connection.lock().unwrap();
    let audit_id = &r.progress.confirmation.as_ref().unwrap().operation_id;
    let audit: (Option<String>, String, String) = db
        .query_row(
            "SELECT meeting_id,state,target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![audit_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(audit.0.is_none());
    assert_eq!(audit.1, "completed");
    let record: serde_json::Value = serde_json::from_str(&audit.2).unwrap();
    assert_eq!(
        record["previousProgress"],
        serde_json::from_str::<serde_json::Value>(&old_progress).unwrap()
    );
    drop(db);
    let listing = operations::listing(&f.storage).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].operation_id, f.operation);
    assert!(operations::authorize_review(&f.storage, Some(audit_id)).is_err());
    finish(&f, &mut r);
    assert!(!extra.exists());
    assert_eq!(original, snapshot(&f));
    assert_eq!(audit_count(&f), 1);
    // Stored audit completion never substitutes for meeting-deletion completion.
    assert!(operations::listing(&f.storage).unwrap().items.is_empty());
}

#[test]
fn renewal_changed_registered_remaining_file_rechecks_new_bytes_but_keeps_original_database_scope()
{
    let f = Fixture::new();
    let remaining = f.file.parent().unwrap().join("a.flac");
    std::fs::write(&remaining, b"ORIGINAL SECOND AUDIO").unwrap();
    f.storage
        .register_ready_meeting_asset(
            &f.meeting,
            remaining.strip_prefix(f.storage.root()).unwrap(),
            "audio/flac",
        )
        .unwrap();
    let mut r = f.begin(false);
    let original = snapshot(&f);
    pause_first(&f, &mut r, "advanced");
    assert!(!f.file.exists());
    std::fs::write(&remaining, b"CHANGED SECOND AUDIO").unwrap();
    assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    accept(&f, &mut r, false);
    assert!(r.manifest.entries.iter().any(|e| e.registered.is_some()
        && e.proof.is_some()
        && e.sha256.as_ref() != e.registered.as_ref().map(|a| &a.sha256)));
    assert_eq!(original, snapshot(&f));
    assert_eq!(std::fs::read(&remaining).unwrap(), b"CHANGED SECOND AUDIO");
    finish(&f, &mut r);
}

#[test]
fn renewal_multiple_generations_reject_stale_review_and_cold_reload_keeps_linked_plans() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    std::fs::write(&f.file, b"FIRST CHANGED CONTENT").unwrap();
    let stale = review(&f, &r);
    renewal::accept(
        &f.storage,
        &mut r,
        &stale,
        false,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let first = r.progress.confirmation.clone().unwrap();
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &stale,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(audit_count(&f), 1);
    std::fs::write(&f.file, b"SECOND CHANGED CONTENT").unwrap();
    accept(&f, &mut r, false);
    assert_eq!(r.progress.confirmation.as_ref().unwrap().generation, 2);
    assert_ne!(
        r.progress.confirmation.as_ref().unwrap().operation_id,
        first.operation_id
    );
    assert_eq!(audit_count(&f), 2);
    let expected = r.progress_json().unwrap();
    let path = f.storage.root().to_owned();
    let Fixture {
        storage,
        base,
        meeting,
        other,
        other_file,
        external,
        file,
        operation,
    } = f;
    drop(storage);
    let reopened = Storage::open(path).unwrap();
    assert!(reopened.data_write_blocked());
    let mut cold = read(&reopened, &operation, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(cold.progress_json().unwrap(), expected);
    let f = Fixture {
        storage: reopened,
        base,
        meeting,
        other,
        other_file,
        external,
        file,
        operation,
    };
    finish(&f, &mut cold);
    assert!(f.storage.restart_required());
    let mut replay = completion::read(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    completion::run(
        &f.storage,
        &mut replay,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(replay.completed());
}

#[test]
fn renewal_internal_staged_file_requires_unknown_consent_and_new_generation_reservations() {
    for boundary in ["renamed", "staged", "unlinked"] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        pause_first(&f, &mut r, boundary);
        let original_reserved = f.file.parent().unwrap().join(r.reserved());
        let v = review(&f, &r);
        if boundary != "unlinked" {
            assert!(original_reserved.exists());
            assert!(renewal::accept(
                &f.storage,
                &mut r,
                &v,
                false,
                &lease(),
                &crate::test_support::never_cancelled
            )
            .is_err());
        }
        renewal::accept(
            &f.storage,
            &mut r,
            &v,
            boundary != "unlinked",
            &lease(),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(r.reserved().contains("-r1-"));
        finish(&f, &mut r);
        assert!(!original_reserved.exists());
    }
}

#[test]
fn renewal_cancelled_or_panicked_before_commit_rolls_back_audit_and_cursor_together() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    std::fs::write(&f.file, b"CHANGED BUT KEPT").unwrap();
    let v = review(&f, &r);
    let original = snapshot(&f);
    let before = r.progress_json().unwrap();
    // A successful rollback leaves the same input for every fault boundary.
    for point in [
        "reviewIntentStored",
        "reviewPathStored",
        "reviewRecorded",
        "beforeReviewCommit",
    ] {
        for unwind in [false, true] {
            let stop = Cell::new(false);
            let reached = Cell::new(false);
            let admitted = lease();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renewal::accept_observed(
                    &f.storage,
                    &mut r,
                    &v,
                    false,
                    &admitted,
                    &|| stop.get(),
                    &|p| {
                        if p == point {
                            reached.set(true);
                            if unwind {
                                panic!("test interrupted renewal");
                            } else {
                                stop.set(true);
                            }
                        }
                    },
                )
            }));
            assert!(reached.get(), "{point}, unwind={unwind}");
            if unwind {
                assert!(outcome.is_err());
                // A panic poisons the outer mutex, not the on-disk transaction.
                f.storage.connection.clear_poison();
            } else {
                assert!(outcome.unwrap().is_err());
                assert!(stop.get());
            }
            assert_eq!(before, r.progress_json().unwrap());
            assert_eq!(
                before,
                read(
                    &f.storage,
                    &f.operation,
                    &crate::test_support::never_cancelled
                )
                .unwrap()
                .progress_json()
                .unwrap()
            );
            assert_eq!(audit_count(&f), 0);
            assert_eq!(original, snapshot(&f));
            assert_eq!(std::fs::read(&f.file).unwrap(), b"CHANGED BUT KEPT");
            f.unaffected();
        }
    }
}

#[test]
fn renewal_after_commit_unwind_reloads_committed_generation_without_deleting_files() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    std::fs::write(&f.file, b"COMMITTED NEW OBSERVATION").unwrap();
    let v = review(&f, &r);
    let admitted = lease();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| renewal::accept_observed(
            &f.storage,
            &mut r,
            &v,
            false,
            &admitted,
            &crate::test_support::never_cancelled,
            &|p| {
                if p == "reviewCommitted" {
                    panic!("test lost post-commit acknowledgement");
                }
            }
        )))
        .is_err()
    );
    f.storage.connection.clear_poison();
    assert_eq!(audit_count(&f), 1);
    assert_eq!(
        std::fs::read(&f.file).unwrap(),
        b"COMMITTED NEW OBSERVATION"
    );
    let mut resumed = read(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        resumed.progress.confirmation.as_ref().unwrap().generation,
        1
    );
    assert!(renewal::accept(
        &f.storage,
        &mut resumed,
        &v,
        false,
        &admitted,
        &crate::test_support::never_cancelled
    )
    .is_err());
    finish(&f, &mut resumed);
}

#[test]
fn renewal_real_lease_cancellation_overrides_caller_and_retains_old_plan() {
    for point in [
        "before",
        "reviewIntentStored",
        "reviewPathStored",
        "reviewRecorded",
        "beforeReviewCommit",
    ] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        std::fs::write(&f.file, b"KEPT AFTER CANCEL").unwrap();
        let v = review(&f, &r);
        let before = r.progress_json().unwrap();
        let analysis = AnalysisController::default();
        let admitted = analysis
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        if point == "before" {
            analysis.begin_exit().unwrap();
        }
        assert!(renewal::accept_observed(
            &f.storage,
            &mut r,
            &v,
            false,
            &admitted,
            &crate::test_support::never_cancelled,
            &|p| {
                if p == point {
                    analysis.begin_exit().unwrap();
                }
            }
        )
        .is_err());
        assert!(admitted.validate_active().is_err());
        assert_eq!(before, r.progress_json().unwrap());
        assert_eq!(audit_count(&f), 0);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"KEPT AFTER CANCEL");
        assert!(analysis.begin_export().is_err());
        f.unaffected();
    }
}

#[test]
fn renewal_rejects_inventory_changes_since_review_and_just_before_commit() {
    for point in [
        "before",
        "reviewIntentStored",
        "reviewRecorded",
        "beforeReviewCommit",
    ] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        std::fs::write(&f.file, b"REVIEWED CONTENT").unwrap();
        let v = review(&f, &r);
        let before = r.progress_json().unwrap();
        if point == "before" {
            std::fs::write(&f.file, b"NEWER CONTENT").unwrap();
        }
        assert!(renewal::accept_observed(
            &f.storage,
            &mut r,
            &v,
            false,
            &lease(),
            &crate::test_support::never_cancelled,
            &|p| {
                if p == point {
                    std::fs::write(&f.file, b"NEWER CONTENT").unwrap();
                }
            }
        )
        .is_err());
        assert_eq!(before, r.progress_json().unwrap());
        assert_eq!(audit_count(&f), 0);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"NEWER CONTENT");
        f.unaffected();
    }
}

#[test]
fn renewal_rejects_foreign_review_without_adopting_its_scope() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let foreign = Fixture::new();
    let other = foreign.begin(false);
    let v = review(&foreign, &other);
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &v,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(audit_count(&f), 0);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
    foreign.unaffected();
}

#[test]
fn renewal_immutable_audit_targets_and_paths_reject_updates_deletes_or_post_completion_inserts() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    std::fs::write(&f.file, b"NEW OBSERVATION").unwrap();
    accept(&f, &mut r, false);
    let id = &r.progress.confirmation.as_ref().unwrap().operation_id;
    let db = f.storage.connection.lock().unwrap();
    let writer = crate::storage::deletion::fence::write_removal(
        &f.storage,
        &db,
        &r,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    for sql in [
        "UPDATE maintenance_operations SET target_manifest_json='{}' WHERE id=?1",
        "UPDATE maintenance_operations SET consent_at=consent_at+1 WHERE id=?1",
        "UPDATE maintenance_operations SET state='prepared' WHERE id=?1",
        "DELETE FROM maintenance_operations WHERE id=?1",
        "UPDATE maintenance_paths SET relative_path='other' WHERE operation_id=?1",
        "DELETE FROM maintenance_paths WHERE operation_id=?1",
        "INSERT INTO maintenance_paths(operation_id,relative_path,kind,identity_json,identity_hash) VALUES(?1,'late','directory','{}','0000000000000000000000000000000000000000000000000000000000000000')",
    ] {
        assert!(db.execute(sql, params![id]).is_err(), "audit must be immutable: {sql}");
    }
    drop(writer);
    drop(db);
    finish(&f, &mut r);
}

#[test]
fn renewal_invalid_reference_policy_cursor_or_audit_progress_refuses_loading_and_file_io() {
    for corruption in [
        "reference_hash",
        "reference_id",
        "generation",
        "version",
        "cursor",
        "policy",
        "consent",
        "audit_progress",
    ] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        std::fs::write(&f.file, b"REVIEWED AND KEPT").unwrap();
        accept(&f, &mut r, false);
        let id = r
            .progress
            .confirmation
            .as_ref()
            .unwrap()
            .operation_id
            .clone();
        let db = f.storage.connection.lock().unwrap();
        let writer = crate::storage::deletion::fence::write_removal(
            &f.storage,
            &db,
            &r,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        if corruption == "audit_progress" {
            db.execute(
                "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                params![id],
            )
            .unwrap();
        } else {
            let mut json = serde_json::to_value(&r.progress).unwrap();
            match corruption {
                "reference_hash" => json["confirmation"]["intentHash"] = "0".repeat(64).into(),
                "reference_id" => json["confirmation"]["operationId"] = f.operation.clone().into(),
                "generation" => json["confirmation"]["generation"] = 2.into(),
                "version" => json["version"] = 1.into(),
                "cursor" => json["next"] = (r.manifest.entries.len() + 1).into(),
                "policy" => json["policy"] = "explicitUnregistered".into(),
                "consent" => json["consentAt"] = (r.progress.consent_at + 1).into(),
                _ => unreachable!(),
            }
            db.execute(
                "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
                params![f.operation, serde_json::to_string(&json).unwrap()],
            )
            .unwrap();
        }
        drop(writer);
        drop(db);
        assert!(
            read(
                &f.storage,
                &f.operation,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "must reject {corruption}"
        );
        assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
        assert_eq!(std::fs::read(&f.file).unwrap(), b"REVIEWED AND KEPT");
        f.unaffected();
    }
}

#[test]
fn renewal_rejects_symlinks_and_hardlinks_instead_of_adopting_external_files() {
    for hardlink in [false, true] {
        let f = Fixture::new();
        let r = f.begin(false);
        let linked = f.file.parent().unwrap().join("external-link");
        if hardlink {
            std::fs::hard_link(&f.external, &linked).unwrap();
        } else {
            std::os::unix::fs::symlink(&f.external, &linked).unwrap();
        }
        assert!(
            renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled)
                .is_err()
        );
        assert_eq!(audit_count(&f), 0);
        assert_eq!(std::fs::read(&f.external).unwrap(), b"KEEP EXTERNAL EXPORT");
        f.unaffected();
    }
}

#[test]
fn renewal_preserves_reserved_meeting_root_outside_logical_tree_without_silently_abandoning_it() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|p| {
        if p == "renamed"
            && !f
                .storage
                .root()
                .join(format!("meetings/{}", f.meeting))
                .exists()
        {
            stop.set(true);
        }
    })
    .is_err());
    assert!(stop.get());
    let reserved = f.storage.root().join("meetings").join(r.reserved());
    assert!(reserved.exists());
    let inspected =
        renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled).unwrap();
    assert!(inspected.manifest().entries.iter().any(|e| e.path
        == reserved
            .strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()));
    assert!(reserved.exists());
    assert_eq!(audit_count(&f), 0);
    // The unchanged original plan can still resolve its exact reservation.
    finish(&f, &mut r);
}

#[test]
fn renewal_many_remaining_files_store_observations_separately_and_keep_cursor_bounded() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    for n in 0..200 {
        std::fs::write(
            f.file.parent().unwrap().join(format!("new-{n:04}.txt")),
            b"EXPLICIT REMAINING FILE",
        )
        .unwrap();
    }
    let v = review(&f, &r);
    assert!(serde_json::to_vec(v.manifest()).unwrap().len() > 65536);
    renewal::accept(
        &f.storage,
        &mut r,
        &v,
        true,
        &lease(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let id = &r.progress.confirmation.as_ref().unwrap().operation_id;
    let db = f.storage.connection.lock().unwrap();
    let (bytes, paths): (i64,i64) = db.query_row("SELECT length(CAST(target_manifest_json AS BLOB)),(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",params![id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert!(bytes < 65536);
    assert_eq!(paths, 203);
    assert!(r.progress_json().unwrap().len() < 4096);
    drop(db);
    finish(&f, &mut r);
}

#[test]
fn renewal_generation_limit_retains_last_valid_plan_and_does_not_create_an_extra_audit() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    for n in 1..=32 {
        std::fs::write(&f.file, format!("CHANGED GENERATION {n}")).unwrap();
        accept(&f, &mut r, false);
        assert_eq!(r.progress.confirmation.as_ref().unwrap().generation, n);
    }
    let before = r.progress_json().unwrap();
    assert!(
        renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled).is_err()
    );
    assert_eq!(audit_count(&f), 32);
    assert_eq!(
        before,
        read(
            &f.storage,
            &f.operation,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .progress_json()
        .unwrap()
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"CHANGED GENERATION 32");
    finish(&f, &mut r);
}

#[test]
fn renewal_another_pending_kind_blocks_inspection_and_acceptance_without_changing_evidence() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let v = review(&f, &r);
    let before = r.progress_json().unwrap();
    let db = f.storage.connection.lock().unwrap();
    let writer = crate::storage::deletion::fence::write_removal(
        &f.storage,
        &db,
        &r,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','attention','{}',?3,'{}',1,1)",params![uuid::Uuid::new_v4().to_string(), f.other,"0".repeat(64)]).unwrap();
    drop(writer);
    drop(db);
    assert!(
        renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled).is_err()
    );
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &v,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(audit_count(&f), 0);
    assert_eq!(before, r.progress_json().unwrap());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}

#[test]
fn renewal_saved_generation_is_used_by_native_preview_paging_execution_and_final_acknowledgement() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let prior = operations::Prepared::pending(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .preview();
    let unknown = f.file.parent().unwrap().join("new-unknown.txt");
    std::fs::write(&unknown, b"NEW EXPLICIT CONTENT").unwrap();
    accept(&f, &mut r, true);
    let mut native = operations::Prepared::pending(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let current = native.preview();
    assert!(current.execute_allowed && current.pending && !current.keep_allowed);
    assert_eq!(current.phase, "files");
    assert_eq!(current.path_count, r.manifest.entries.len());
    assert_eq!(current.unregistered_count, 1);
    assert_ne!(prior.confirmation_id, current.confirmation_id);
    assert!(native.page(&prior.confirmation_id, 0).is_err());
    let page = native.page(&current.confirmation_id, 0).unwrap();
    assert!(page
        .entries
        .iter()
        .any(|p| p.relative_path.ends_with("/new-unknown.txt") && !p.registered));
    let receipt = native
        .execute(
            &f.storage,
            &lease(),
            &current.confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted && !receipt.retained);
    assert!(!f.file.exists());
    assert!(!unknown.exists());
    let mut replay = operations::Prepared::pending(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = replay.preview();
    assert_eq!(preview.phase, "awaiting_ack");
    assert!(
        replay
            .execute(
                &f.storage,
                &lease(),
                &preview.confirmation_id,
                false,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    assert_eq!(audit_count(&f), 1);
}

#[test]
fn renewal_preexisting_generation_reservation_is_not_overwritten_even_with_unknown_consent() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let reserved = f
        .file
        .parent()
        .unwrap()
        .join(format!(".mon-delete-{}-r1-0", f.operation));
    std::fs::write(&reserved, b"PREEXISTING UNKNOWN RESERVATION").unwrap();
    let v = review(&f, &r);
    let before = r.progress_json().unwrap();
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &v,
        true,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(before, r.progress_json().unwrap());
    assert_eq!(audit_count(&f), 0);
    assert_eq!(
        std::fs::read(&reserved).unwrap(),
        b"PREEXISTING UNKNOWN RESERVATION"
    );
    f.unaffected();
}

#[test]
fn renewal_cannot_replace_original_database_scope_even_after_a_new_file_review() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let v = review(&f, &r);
    let before = r.progress_json().unwrap();
    let db = f.storage.connection.lock().unwrap();
    let writer = crate::storage::deletion::fence::write_removal(
        &f.storage,
        &db,
        &r,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute(
        "UPDATE meetings SET title='UNREVIEWED DATABASE CHANGE' WHERE id=?1",
        params![f.meeting],
    )
    .unwrap();
    drop(writer);
    drop(db);
    assert!(
        renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled).is_err()
    );
    assert!(renewal::accept(
        &f.storage,
        &mut r,
        &v,
        false,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(before, r.progress_json().unwrap());
    assert_eq!(audit_count(&f), 0);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}

#[test]
fn renewal_cached_builtin_schema_never_adopts_a_changed_live_constraint() {
    let f = Fixture::new();
    let r = f.begin(false); // Warm the fixed built-in reference schema.
    assert!(review(&f, &r).manifest().entries.len() > 1);
    let db = f.storage.connection.lock().unwrap();
    let writer = crate::storage::deletion::fence::write_removal(
        &f.storage,
        &db,
        &r,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute_batch("DROP TRIGGER maintenance_operation_fixed")
        .unwrap();
    drop(writer);
    drop(db);
    assert!(read(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(
        renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled).is_err()
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}

#[test]
#[ignore = "subprocess fixture; invoked by renewal SIGKILL parent"]
fn renewal_checkpoint_child_fixture() {
    let base = PathBuf::from(std::env::var("MON_RENEWAL_TEST_BASE").unwrap());
    let phase = std::env::var("MON_RENEWAL_TEST_PHASE").unwrap();
    let f = Fixture::at(tempfile::tempdir_in(&base).unwrap());
    let mut r = f.begin(false);
    let extra = if std::env::var("MON_RENEWAL_TEST_ROOT").as_deref() == Ok("1") {
        let logical = f.storage.root().join(format!("meetings/{}", f.meeting));
        let stop = Cell::new(false);
        assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|point| {
            if point == "renamed" && !logical.exists() {
                stop.set(true);
            }
        })
        .is_err());
        assert!(stop.get());
        f.storage
            .root()
            .join("meetings")
            .join(r.reserved())
            .join("remaining.txt")
    } else {
        pause_first(&f, &mut r, "advanced");
        f.file.parent().unwrap().join("remaining.txt")
    };
    std::fs::write(&extra, b"KEEP UNTIL EXPLICIT RESUME").unwrap();
    let v = review(&f, &r);
    let original = snapshot(&f);
    let old_progress = r.progress_json().unwrap();
    renewal::accept_observed(
        &f.storage,
        &mut r,
        &v,
        true,
        &lease(),
        &crate::test_support::never_cancelled,
        &|point| {
            if point == phase {
                let receipt = serde_json::to_vec(&(
                    f.storage.root(),
                    &f.operation,
                    &f.meeting,
                    &f.other,
                    &f.other_file,
                    &f.external,
                    &f.file,
                    &extra,
                    &original,
                    &old_progress,
                ))
                .unwrap();
                std::fs::write(base.join("ready.tmp"), receipt).unwrap();
                std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        },
    )
    .unwrap();
    panic!("renewal checkpoint not reached");
}

#[test]
fn renewal_sigkill_at_five_atomic_boundaries_cold_reopens_without_losing_old_progress_or_new_consent(
) {
    sigkill_reconfirmation(false);
}

#[test]
fn renewal_root_sigkill_at_five_atomic_boundaries_retains_exact_reserved_scope_and_consent() {
    sigkill_reconfirmation(true);
}

fn sigkill_reconfirmation(reserved_root: bool) {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "reviewIntentStored",
        "reviewPathStored",
        "reviewRecorded",
        "beforeReviewCommit",
        "reviewCommitted",
    ] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::deletion::removal::tests::renewal_tests::renewal_checkpoint_child_fixture", "--ignored", "--nocapture"])
            .env("MON_RENEWAL_TEST_BASE", base.path())
            .env("MON_RENEWAL_TEST_PHASE", phase)
            .env("MON_RENEWAL_TEST_ROOT", if reserved_root { "1" } else { "0" })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn().unwrap();
        let ready = base.path().join("ready.json");
        let start = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || start.elapsed() > std::time::Duration::from_secs(30)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("renewal checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        type Evidence = (
            PathBuf,
            String,
            String,
            String,
            PathBuf,
            PathBuf,
            PathBuf,
            PathBuf,
            (String, String, i64, String),
            String,
        );
        let (
            root,
            operation,
            meeting,
            other,
            other_file,
            external,
            file,
            extra,
            original,
            old_progress,
        ): Evidence = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        let storage = Storage::open(root).unwrap();
        let f = Fixture {
            base,
            storage,
            meeting,
            other,
            file,
            other_file,
            external,
            operation,
        };
        assert!(f.storage.data_write_blocked());
        assert!(!f.file.exists());
        assert_eq!(
            std::fs::read(&extra).unwrap(),
            b"KEEP UNTIL EXPLICIT RESUME"
        );
        assert_eq!(snapshot(&f), original);
        let mut resumed = read(
            &f.storage,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        if phase == "reviewCommitted" {
            assert_eq!(audit_count(&f), 1);
            assert_eq!(
                resumed.progress.confirmation.as_ref().unwrap().generation,
                1
            );
        } else {
            assert_eq!(audit_count(&f), 0);
            assert_eq!(resumed.progress_json().unwrap(), old_progress);
            assert!(run(
                &f.storage,
                &mut resumed,
                &crate::test_support::never_cancelled
            )
            .is_err());
            accept(&f, &mut resumed, true);
        }
        finish(&f, &mut resumed);
        assert!(!extra.exists());
        if reserved_root {
            assert!(!extra.parent().unwrap().exists());
        }
        assert_eq!(snapshot(&f), original);
        assert_eq!(audit_count(&f), 1);
        assert!(f.storage.restart_required());
    }
}
