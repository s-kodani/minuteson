//! Private SQLite and synthetic files only; no native user audio is accessed.
use super::*;
use crate::analysis_worker::AnalysisController;
use crate::storage::deletion::audio::tests::Fixture;
use std::cell::Cell;

fn owner() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
fn started(f: &Fixture, lease: &DataMaintenanceLease) -> String {
    let op = uuid::Uuid::new_v4().to_string();
    intent::prepare(
        &f.storage,
        &op,
        &f.census(),
        intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        lease,
        &|| false,
    )
    .unwrap();
    super::super::start(&f.storage, &op, lease, &|| false).unwrap();
    op
}
fn fixed(f: &Fixture, op: &str) -> (String, String, i64, i64, i64) {
    f.storage.connection.lock().unwrap().query_row(
        "SELECT target_manifest_json,target_hash,consent_at,created_at,rowid FROM maintenance_operations WHERE id=?1",
        [op], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap()
}
fn finish(f: &Fixture, op: &str, lease: &DataMaintenanceLease) -> Receipt {
    super::super::run(&f.storage, op, lease, &|| false).unwrap();
    super::super::completion::finish(&f.storage, op, lease, &|| false).unwrap()
}
#[test]
fn changed_remaining_requires_new_consent_preserves_original_and_exact_retry() {
    let f = Fixture::new();
    f.documents();
    let document = f.file("documents/retain.json", Some("application/json"));
    let unknown = f.file("unknown.wav", None);
    let lease = owner();
    let op = started(&f, &lease);
    let original = fixed(&f, &op);
    let saved = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
    let old_scope = saved.saved.snapshot.scope.clone();
    std::fs::write(&f.path, b"CHANGED AUDIO").unwrap();
    assert!(super::super::run(&f.storage, &op, &lease, &|| false).is_err());
    assert!(f.path.exists());
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let current = accept(&f.storage, &review, false, &lease, &|| false).unwrap();
    assert_eq!(current.policy.include_unregistered_audio, false);
    assert_eq!(current.progress.version, 3);
    assert_eq!(current.audits.len(), 1);
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(
        accept(&f.storage, &review, false, &lease, &|| false).unwrap(),
        current
    );
    assert!(accept(&f.storage, &review, true, &lease, &|| false).is_err());
    assert_eq!(f.operation_count(), 2);
    assert!(f.storage.data_write_blocked());
    let completed = finish(&f, &op, &lease);
    assert!(completed.completed());
    assert!(!f.path.exists());
    assert!(document.is_file() && unknown.is_file());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(completed.saved.snapshot.scope, old_scope);
    assert!(Review::capture(&f.storage, &op, &lease, &|| false).is_err());
    let f = f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert!(audio::require_source(&f.storage.connection.lock().unwrap(), &f.meeting).is_err());
}
#[test]
fn new_unregistered_audio_and_stale_review_never_borrow_old_consent() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    let extra = f.unregistered_audio();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(accept(&f.storage, &review, false, &lease, &|| false).is_err());
    assert_eq!(f.operation_count(), 1);
    std::fs::write(&extra, b"CHANGED AGAIN").unwrap();
    assert!(accept(&f.storage, &review, true, &lease, &|| false).is_err());
    assert_eq!(f.operation_count(), 1);
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    accept(&f.storage, &review, true, &lease, &|| false).unwrap();
    finish(&f, &op, &lease);
    assert!(!extra.exists() && !f.path.exists());
}
#[test]
fn cancellation_at_each_transaction_boundary_rolls_back_audit_and_cursor() {
    for point in [
        "audioRenewalAuditStored",
        "audioRenewalProofStored",
        "beforeAudioRenewalCommit",
    ] {
        let f = Fixture::new();
        let lease = owner();
        let op = started(&f, &lease);
        std::fs::write(&f.path, b"NEW").unwrap();
        let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
        let cancelled = Cell::new(false);
        assert!(accept_inner(
            &f.storage,
            &review,
            false,
            &lease,
            &|| cancelled.get(),
            &|event| {
                if event == point {
                    cancelled.set(true);
                }
            }
        )
        .is_err());
        assert_eq!(f.operation_count(), 1);
        assert_eq!(
            Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap(),
            review.receipt
        );
        assert!(f.storage.data_write_blocked() && f.path.exists());
        accept(&f.storage, &review, false, &lease, &|| false).unwrap();
        finish(&f, &op, &lease);
    }
}
#[test]
fn real_lease_and_unchanged_database_metadata_are_required() {
    let f = Fixture::new();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    controller.begin_exit().unwrap();
    assert!(accept(&f.storage, &review, false, &lease, &|| false).is_err());
    assert!(f.path.exists());
    let lease = owner();
    let db = f.storage.connection.lock().unwrap();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>)
        .unwrap();
    db.execute(
        "UPDATE meetings SET title='CHANGED' WHERE id=?1",
        [&f.meeting],
    )
    .unwrap();
    f.storage.deletion_fence.arm(&db).unwrap();
    drop(db);
    assert!(accept(&f.storage, &review, false, &lease, &|| false).is_err());
    assert!(Review::capture(&f.storage, &op, &lease, &|| false).is_err());
    assert_eq!(f.operation_count(), 1);
}
#[cfg(unix)]
#[test]
fn symlink_and_hardlink_are_not_adopted_into_a_new_plan() {
    for hardlink in [false, true] {
        let f = Fixture::new();
        let lease = owner();
        let op = started(&f, &lease);
        let external = f.base.path().join("external.caf");
        std::fs::write(&external, b"EXTERNAL").unwrap();
        std::fs::remove_file(&f.path).unwrap();
        if hardlink {
            std::fs::hard_link(&external, &f.path).unwrap();
        } else {
            std::os::unix::fs::symlink(&external, &f.path).unwrap();
        }
        assert!(Review::capture(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(external).unwrap(), b"EXTERNAL");
        assert_eq!(f.operation_count(), 1);
    }
}

#[test]
fn staged_reservation_is_exact_history_and_old_pending_proof_is_preserved() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    let stop = Cell::new(false);
    assert!(
        super::super::run_inner(&f.storage, &op, &lease, &|| stop.get(), &|e| {
            if e == "audioStaged" {
                stop.set(true);
            }
        })
        .is_err()
    );
    let old = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
    let staged = f.storage.root().join(old.reserved_path(0));
    assert!(staged.exists() && !f.path.exists());
    std::fs::write(&staged, b"CHANGED RESERVED AUDIO").unwrap();
    let near = staged.with_file_name(format!(".mon-delete-audio-{op}-999"));
    std::fs::write(&near, b"KEEP LOOKALIKE").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    assert_eq!(
        review.snapshot.historical_audio,
        BTreeSet::from([old.reserved_path(0)])
    );
    assert!(accept(&f.storage, &review, false, &lease, &|| false).is_err());
    let renewed = accept(&f.storage, &review, true, &lease, &|| false).unwrap();
    let mut budget = old.saved.header.snapshot_bytes;
    let (record, _, _) = read_record(
        &f.storage.connection.lock().unwrap(),
        renewed.progress.confirmation.as_ref().unwrap(),
        &old.saved,
        &mut budget,
        &|| false,
    )
    .unwrap();
    assert_eq!(record.previous_progress, old.progress);
    assert!(record
        .previous_progress
        .pending
        .as_ref()
        .unwrap()
        .staged
        .is_some());
    finish(&f, &op, &lease);
    assert!(!staged.exists());
    assert_eq!(std::fs::read(near).unwrap(), b"KEEP LOOKALIKE");
}
#[test]
fn two_reconfirmations_keep_processed_cursor_and_all_historical_reservations() {
    let f = Fixture::new();
    let other = f.file("z-second.flac", Some("audio/flac"));
    let lease = owner();
    let op = started(&f, &lease);
    let stop = Cell::new(false);
    assert!(
        super::super::run_inner(&f.storage, &op, &lease, &|| stop.get(), &|e| {
            if e == "audioAdvanced" {
                stop.set(true);
            }
        })
        .is_err()
    );
    let old = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
    assert_eq!(old.progress.next, 1);
    assert!(!f.path.exists() && other.exists());
    std::fs::write(&other, b"FIRST CHANGE").unwrap();
    let first = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let first_saved = accept(&f.storage, &first, false, &lease, &|| false).unwrap();
    std::fs::write(&other, b"SECOND CHANGE").unwrap();
    let second = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let saved = accept(&f.storage, &second, false, &lease, &|| false).unwrap();
    assert_eq!(saved.audits.len(), 2);
    assert!(saved
        .historical_paths
        .is_superset(&first_saved.historical_paths));
    let mut budget = old.saved.header.snapshot_bytes;
    let (record, _, _) = read_record(
        &f.storage.connection.lock().unwrap(),
        first_saved.progress.confirmation.as_ref().unwrap(),
        &old.saved,
        &mut budget,
        &|| false,
    )
    .unwrap();
    assert_eq!(record.previous_progress.next, 1);
    finish(&f, &op, &lease);
    let f = f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert!(!other.exists());
}
#[test]
fn cleanup_reappearance_can_be_reviewed_before_availability_commit() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    super::super::run(&f.storage, &op, &lease, &|| false).unwrap();
    std::fs::write(&f.path, b"RETURNED BEFORE COMPLETION").unwrap();
    assert!(super::super::completion::finish(&f.storage, &op, &lease, &|| false).is_err());
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    assert_eq!(review.receipt.progress.phase, Phase::AudioFilesRemoved);
    accept(&f.storage, &review, false, &lease, &|| false).unwrap();
    finish(&f, &op, &lease);
    assert!(!f.path.exists());
}
#[test]
fn committed_consent_wins_late_cancellation_but_cold_resume_requires_saved_policy() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let cancelled = Cell::new(false);
    let saved = accept_inner(
        &f.storage,
        &review,
        false,
        &lease,
        &|| cancelled.get(),
        &|e| {
            if e == "audioRenewalCommitted" {
                cancelled.set(true);
            }
        },
    )
    .unwrap();
    assert_eq!(saved.audits.len(), 1);
    assert!(f.storage.data_write_blocked() && f.path.exists());
    drop(lease);
    let f = f.reopen();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .unwrap();
    assert!(
        !Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false)
            .unwrap()
            .policy
            .include_unregistered_audio
    );
    finish(&f, &op, &lease);
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}
#[test]
fn corrupted_audit_and_orphan_audit_cannot_unfence_startup() {
    for orphan in [false, true] {
        let f = Fixture::new();
        let lease = owner();
        let op = started(&f, &lease);
        std::fs::write(&f.path, b"NEW").unwrap();
        let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
        let saved = accept(&f.storage, &review, false, &lease, &|| false).unwrap();
        let audit = saved.audits[0].clone();
        if orphan {
            let db = f.storage.connection.lock().unwrap();
            let _writer = crate::storage::deletion::fence::write_audio_removal(
                &f.storage,
                &db,
                &saved,
                &lease,
                &|| false,
            )
            .unwrap();
            let copied = uuid::Uuid::new_v4().to_string();
            db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) SELECT ?2,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1",params![audit,copied]).unwrap();
            drop(_writer);
            drop(db);
            let f = f.reopen();
            assert!(crate::storage::deletion::startup::contains(
                &f.storage.connection.lock().unwrap(),
                &copied
            )
            .unwrap());
            assert_eq!(
                crate::storage::deletion::startup::reason(
                    &f.storage.connection.lock().unwrap(),
                    &copied
                )
                .unwrap()
                .as_deref(),
                Some("audio_completed_receipt_invalid")
            );
            assert!(f.storage.data_write_blocked() && f.path.exists());
        } else {
            f.corrupt_operation_header(&audit, "{}");
            assert!(super::super::run(&f.storage, &op, &lease, &|| false).is_err());
            let f = f.reopen();
            assert!(crate::storage::deletion::startup::contains(
                &f.storage.connection.lock().unwrap(),
                &audit
            )
            .unwrap());
            assert_eq!(
                crate::storage::deletion::startup::reason(
                    &f.storage.connection.lock().unwrap(),
                    &audit
                )
                .unwrap()
                .as_deref(),
                Some("audio_completed_receipt_invalid")
            );
            assert!(f.storage.data_write_blocked() && f.path.exists());
        }
    }
}
#[test]
fn renewed_completion_is_a_typed_source_for_a_new_distinct_audio_operation() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    accept(&f.storage, &review, false, &lease, &|| false).unwrap();
    finish(&f, &op, &lease);
    std::fs::write(&f.path, b"RETURNED AFTER COMPLETION").unwrap();
    let review =
        intent::reconfirmation::Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let new_op = uuid::Uuid::new_v4().to_string();
    intent::reconfirmation::accept(
        &f.storage,
        &new_op,
        &review,
        intent::Policy {
            include_unregistered_audio: false,
            retain_unclassified: true,
        },
        &lease,
        &|| false,
    )
    .unwrap();
    super::super::start(&f.storage, &new_op, &lease, &|| false).unwrap();
    finish(&f, &new_op, &lease);
    assert_ne!(op, new_op);
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}
#[test]
fn renewed_audit_remains_valid_after_whole_meeting_fk_detachment() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let saved = accept(&f.storage, &review, false, &lease, &|| false).unwrap();
    finish(&f, &op, &lease);
    drop(lease);
    use crate::storage::deletion::{completion, files, intent as meeting_intent, removal};
    let census = files::Census::capture(&f.storage, &f.meeting, &|| false).unwrap();
    let reviewed = removal::Reviewed::confirm(&census, true).unwrap();
    let original = meeting_intent::prepare(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &census,
        &|| false,
    )
    .unwrap();
    let mut files = removal::start(&f.storage, &original, &reviewed, &|| false).unwrap();
    removal::run(&f.storage, &mut files, &|| false).unwrap();
    let mut rows = completion::start(&f.storage, &files, &|| false).unwrap();
    completion::run(&f.storage, &mut rows, &|| false).unwrap();
    let f = f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert!(Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).is_err());
    crate::storage::deletion::audio::intent::removal::inspect_audit_member(
        &f.storage.connection.lock().unwrap(),
        &f.storage.managed_anchor,
        &op,
        &saved.audits[0],
    )
    .unwrap();
}

#[test]
fn clock_rollback_and_new_reservation_collision_do_not_publish_audit() {
    for future in [false, true] {
        let f = Fixture::new();
        let lease = owner();
        let op = started(&f, &lease);
        std::fs::write(&f.path, b"NEW").unwrap();
        if future {
            let db = f.storage.connection.lock().unwrap();
            let saved = Receipt::load(&db, &op, &|| false).unwrap();
            let _writer = crate::storage::deletion::fence::write_audio_removal(
                &f.storage,
                &db,
                &saved,
                &lease,
                &|| false,
            )
            .unwrap();
            db.execute(
                "UPDATE maintenance_operations SET updated_at=?2 WHERE id=?1",
                params![op, crate::storage::now_ms().unwrap() + 86_400_000],
            )
            .unwrap();
        } else {
            let collision = f.path.with_file_name(reserved(&op, 1, 0));
            std::fs::write(collision, b"KEEP COLLISION").unwrap();
        }
        let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
        assert!(accept(&f.storage, &review, false, &lease, &|| false).is_err());
        assert_eq!(f.operation_count(), 1);
        assert!(f.path.is_file() && f.storage.data_write_blocked());
    }
}
#[test]
fn canonical_oversized_chain_is_rejected_before_reading_its_snapshot_chunks() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
    let saved = accept(&f.storage, &review, false, &lease, &|| false).unwrap();
    let db = f.storage.connection.lock().unwrap();
    let reference = saved.progress.confirmation.as_ref().unwrap();
    let mut budget = saved.saved.header.snapshot_bytes;
    let (mut record, _, _) =
        read_record(&db, reference, &saved.saved, &mut budget, &|| false).unwrap();
    record.snapshot_bytes = intent::MAX_BYTES;
    record.chunk_count = intent::MAX_CHUNKS;
    let json = record.encode().unwrap();
    let hash = intent::hash(json.as_bytes());
    let mut progress = saved.progress.clone();
    progress.confirmation.as_mut().unwrap().intent_hash = hash.clone();
    // Fault injection is restricted to this fixture's SQLite connection.
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>)
        .unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )
    .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",
        params![reference.operation_id, json, hash],
    )
    .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
        params![op, progress.encode().unwrap()],
    )
    .unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        true,
    )
    .unwrap();
    f.storage.deletion_fence.arm(&db).unwrap();
    assert!(Receipt::load(&db, &op, &|| false)
        .unwrap_err()
        .contains("証拠合計が容量上限"));
    drop(db);
    assert!(super::super::run(&f.storage, &op, &lease, &|| false).is_err());
    assert!(f.path.is_file() && f.storage.data_write_blocked());
}
#[test]
fn thirty_two_real_generations_are_valid_and_the_next_review_is_not_admitted() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    let original = fixed(&f, &op);
    for generation in 1..=MAX_GENERATIONS {
        std::fs::write(&f.path, format!("CHANGED GENERATION {generation}")).unwrap();
        let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
        let saved = accept(&f.storage, &review, false, &lease, &|| false).unwrap();
        assert_eq!(
            saved.progress.confirmation.as_ref().unwrap().generation,
            generation
        );
        assert_eq!(saved.audits.len(), generation as usize);
        assert_eq!(fixed(&f, &op), original);
    }
    assert!(Review::capture(&f.storage, &op, &lease, &|| false).is_err());
    assert_eq!(f.operation_count(), 33);
    finish(&f, &op, &lease);
    assert!(!f.path.exists());
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}
#[cfg(unix)]
#[test]
fn real_sigkill_before_and_after_commit_cold_reads_only_the_committed_generation() {
    use std::os::unix::process::ExitStatusExt;
    for boundary in ["beforeAudioRenewalCommit", "audioRenewalCommitted"] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = started(&f, &lease);
        let original = fixed(&f, &op);
        std::fs::write(&f.path, b"CHANGED BEFORE CHILD").unwrap();
        drop(lease);
        let root = f.storage.root().to_owned();
        let marker = (
            root.to_str().unwrap().to_owned(),
            op.clone(),
            boundary.to_owned(),
        );
        std::fs::write(
            f.base.path().join("audio-renewal-owner.json"),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();
        let temporary = tempfile::tempdir().unwrap();
        drop(std::mem::replace(
            &mut f.storage,
            Storage::open(temporary.path().to_owned()).unwrap(),
        ));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::deletion::audio::intent::removal::renewal::tests::audio_renewal_crash_child", "--ignored", "--nocapture"])
            .env("MON_AUDIO_RENEWAL_ROOT", &root).env("MON_AUDIO_RENEWAL_OPERATION", &op)
            .env("MON_AUDIO_RENEWAL_BOUNDARY", boundary)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        f.storage = Storage::open(root).unwrap();
        assert!(f.storage.data_write_blocked() && f.path.exists());
        assert_eq!(fixed(&f, &op), original);
        let saved = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
        assert_eq!(
            saved.audits.len(),
            usize::from(boundary == "audioRenewalCommitted")
        );
        assert_eq!(
            f.operation_count(),
            if boundary == "audioRenewalCommitted" {
                2
            } else {
                1
            }
        );
        let lease = AnalysisController::default()
            .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
            .unwrap();
        if boundary == "beforeAudioRenewalCommit" {
            assert!(super::super::run(&f.storage, &op, &lease, &|| false).is_err());
            let review = Review::capture(&f.storage, &op, &lease, &|| false).unwrap();
            accept(&f.storage, &review, false, &lease, &|| false).unwrap();
        }
        finish(&f, &op, &lease);
        assert_eq!(fixed(&f, &op), original);
        drop(lease);
        assert!(!f.reopen().storage.data_write_blocked());
    }
}
#[cfg(unix)]
#[test]
#[ignore = "subprocess-only parent-owned synthetic audio renewal fixture"]
fn audio_renewal_crash_child() {
    let root = std::path::PathBuf::from(std::env::var("MON_AUDIO_RENEWAL_ROOT").unwrap());
    let operation = std::env::var("MON_AUDIO_RENEWAL_OPERATION").unwrap();
    let boundary = std::env::var("MON_AUDIO_RENEWAL_BOUNDARY").unwrap();
    assert_eq!(root.file_name().unwrap(), "root");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(root.parent().unwrap().join("audio-renewal-owner.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker,
        (
            root.to_str().unwrap().into(),
            operation.clone(),
            boundary.clone()
        )
    );
    assert!(matches!(
        boundary.as_str(),
        "beforeAudioRenewalCommit" | "audioRenewalCommitted"
    ));
    let storage = Storage::open(root).unwrap();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    let review = Review::capture(&storage, &operation, &lease, &|| false).unwrap();
    accept_inner(&storage, &review, false, &lease, &|| false, &|event| {
        if event == boundary {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            panic!("SIGKILL failed");
        }
    })
    .unwrap();
    panic!("boundary not reached");
}
