//! Private SQLite and synthetic files only; no native user audio is accessed.
use super::*;
use crate::analysis_worker::AnalysisController;
use crate::storage::deletion::audio::tests::Fixture;
use crate::storage::deletion::fs::NativeProof as _;
use std::cell::Cell;

fn owner() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    super::super::start(
        &f.storage,
        &op,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    op
}
fn fixed(f: &Fixture, op: &str) -> (String, String, i64, i64, i64) {
    f.storage.connection.lock().unwrap().query_row(
        "SELECT target_manifest_json,target_hash,consent_at,created_at,rowid FROM maintenance_operations WHERE id=?1",
        [op], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap()
}
fn finish(f: &Fixture, op: &str, lease: &DataMaintenanceLease) -> Receipt {
    super::super::run(&f.storage, op, lease, &crate::test_support::never_cancelled).unwrap();
    super::super::completion::finish(&f.storage, op, lease, &crate::test_support::never_cancelled)
        .unwrap()
}
fn descendant(f: &Fixture, parent: &str, lease: &DataMaintenanceLease) -> String {
    std::fs::write(&f.path, b"REGISTERED AUDIO RETURNED FOR NEW OPERATION").unwrap();
    let review = intent::reconfirmation::Review::capture(
        &f.storage,
        parent,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    intent::reconfirmation::accept(
        &f.storage,
        &op,
        &review,
        intent::Policy {
            include_unregistered_audio: false,
            retain_unclassified: true,
        },
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    super::super::start(
        &f.storage,
        &op,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    op
}

#[test]
fn ancestral_unobserved_reservation_requires_new_choice_and_cold_resumes_exact_scope() {
    let f = Fixture::new();
    let lease = owner();
    let parent = started(&f, &lease);
    finish(&f, &parent, &lease);
    let old = fixed(&f, &parent);
    let op = descendant(&f, &parent, &lease);
    let original = fixed(&f, &op);
    let returned = f.path.with_file_name(reserved(&parent, 0, 0));
    let unrelated = f.path.with_file_name(reserved(&parent, 0, 999));
    std::fs::write(&returned, b"ANCESTOR RESERVATION RETURNED LATE").unwrap();
    std::fs::write(&unrelated, b"LOOKALIKE MUST SURVIVE").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let relative = returned
        .strip_prefix(f.storage.root())
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(review.snapshot.historical_audio, BTreeSet::from([relative]));
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(f.operation_count(), 2);
    let saved = accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(fixed(&f, &parent), old);
    assert_eq!(saved.progress.version, 4);
    assert!(returned.exists() && unrelated.exists());
    drop(lease);
    let f = f.reopen();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let current = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(current, saved);
    finish(&f, &op, &lease);
    assert!(!returned.exists() && !f.path.exists());
    assert!(unrelated.exists());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(fixed(&f, &parent), old);
    assert_eq!(f.operation_count(), 3);
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}

#[test]
fn grandparent_paths_survive_multiple_completed_sources_and_repeated_remaining_reviews() {
    let f = Fixture::new();
    let lease = owner();
    let parent = started(&f, &lease);
    std::fs::write(&f.path, b"CHANGED BEFORE PARENT RENEWAL").unwrap();
    let review = Review::capture(
        &f.storage,
        &parent,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &parent, &lease);
    let middle = descendant(&f, &parent, &lease);
    finish(&f, &middle, &lease);
    let op = descendant(&f, &middle, &lease);
    let returned = f.path.with_file_name(reserved(&parent, 1, 0));
    std::fs::write(&returned, b"GRANDPARENT ONLY").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(review.snapshot.historical_audio.len(), 1);
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(&returned, b"GRANDPARENT CHANGED AGAIN").unwrap();
    assert!(super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    let source = intent::reconfirmation::Source::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(source.paths().contains(
        returned
            .strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()
    ));
    assert!(!returned.exists());
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}

#[test]
fn ancestral_remaining_history_survives_actual_full_meeting_purge_without_live_permission() {
    ancestral_full_meeting_purge(false);
}

#[test]
fn long_audio_ancestral_remaining_history_survives_full_meeting_purge_without_live_permission() {
    ancestral_full_meeting_purge(true);
}

fn ancestral_full_meeting_purge(long: bool) {
    use crate::storage::deletion::operations;
    let mut f = Fixture::new();
    if long {
        f.use_long_audio_path();
    }
    let lease = owner();
    let parent = started(&f, &lease);
    finish(&f, &parent, &lease);
    let op = descendant(&f, &parent, &lease);
    let returned = f.path.with_file_name(reserved(&parent, 0, 0));
    assert_eq!(
        returned
            .strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()
            .len()
            > 256,
        long
    );
    std::fs::write(&returned, b"ANCESTOR RETURNED BEFORE FULL PURGE").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    let parent_record = fixed(&f, &parent);
    let child_record = fixed(&f, &op);
    let mut prepared = operations::Prepared::fresh(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = prepared.preview();
    assert!(
        prepared
            .execute(
                &f.storage,
                &lease,
                &preview.confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    assert_eq!(fixed(&f, &parent), parent_record);
    assert_eq!(fixed(&f, &op), child_record);
    let db = f.storage.connection.lock().unwrap();
    for id in [&parent, &op] {
        assert_eq!(
            db.query_row(
                "SELECT meeting_id FROM maintenance_operations WHERE id=?1",
                [id],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        assert!(Receipt::load(&db, id, &crate::test_support::never_cancelled).is_err());
        assert!(intent::reconfirmation::Source::load(
            &db,
            id,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(crate::storage::deletion::audio_supersession::superseded(
            &db,
            &f.storage.managed_anchor,
            id,
            &crate::test_support::never_cancelled
        )
        .unwrap());
    }
    assert!(
        audio::backup_deleted(&f.storage, &db, &crate::test_support::never_cancelled)
            .unwrap()
            .is_empty()
    );
    drop(db);
    drop(lease);
    let f = f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    assert_eq!(fixed(&f, &parent), parent_record);
    assert_eq!(fixed(&f, &op), child_record);
}

// Canonical corruption of a private fixture. No product permission is relaxed.
fn rewrite_audit(f: &Fixture, op: &str, mutate: impl FnOnce(&mut Record, &mut audio::Snapshot)) {
    let db = f.storage.connection.lock().unwrap();
    let receipt = Receipt::load(&db, op, &crate::test_support::never_cancelled).unwrap();
    let mut reference = receipt.progress.confirmation.clone().unwrap();
    let mut budget = receipt.saved.header.snapshot_bytes;
    let (mut record, mut snapshot, _) = read_record(
        &db,
        &reference,
        &receipt.saved,
        &mut budget,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    mutate(&mut record, &mut snapshot);
    let bytes = snapshot.encode().unwrap();
    record.snapshot_bytes = bytes.len();
    record.snapshot_hash = intent::hash(&bytes);
    record.chunk_count = bytes.len().div_ceil(intent::CHUNK);
    let json = record.encode().unwrap();
    reference.intent_hash = intent::hash(json.as_bytes());
    let mut progress = receipt.progress.clone();
    progress.confirmation = Some(reference.clone());
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>)
        .unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )
    .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",
        params![reference.operation_id, json, reference.intent_hash],
    )
    .unwrap();
    db.execute(
        "DELETE FROM maintenance_paths WHERE operation_id=?1",
        [&reference.operation_id],
    )
    .unwrap();
    for (index, part) in bytes.chunks(intent::CHUNK).enumerate() {
        let json = intent::chunk_json(part).unwrap();
        db.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,?2,'file',?3,?4,?5,?6)", params![reference.operation_id,intent::slot(index),part.len() as i64,intent::hash(part),json,intent::hash(json.as_bytes())]).unwrap();
    }
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
}

#[test]
fn forged_canonical_ancestral_membership_never_becomes_a_receipt_or_deletion_permit() {
    let f = Fixture::new();
    let lease = owner();
    let parent = started(&f, &lease);
    finish(&f, &parent, &lease);
    let op = descendant(&f, &parent, &lease);
    let unknown = f.path.with_file_name(reserved(&parent, 0, 999));
    std::fs::write(&unknown, b"UNOWNED LOOKALIKE").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let path = unknown
        .strip_prefix(f.storage.root())
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    rewrite_audit(&f, &op, |_, snapshot| {
        snapshot.historical_audio.insert(path);
    });
    // The structural frame and recorded policy are internally valid. Only
    // the oldest-first typed ancestry check can reject this forged membership.
    assert!(Receipt::source_frame(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled
    )
    .is_ok());
    assert!(Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(unknown.exists() && f.path.exists() && f.storage.data_write_blocked());
    drop(lease);
    let f = f.reopen();
    assert!(f.storage.data_write_blocked());
    assert!(unknown.exists() && f.path.exists());
}

#[test]
fn old_remaining_record_is_rejected_without_removing_current_audio() {
    let f = Fixture::new();
    let lease = owner();
    let parent = started(&f, &lease);
    finish(&f, &parent, &lease);
    let op = descendant(&f, &parent, &lease);
    std::fs::write(&f.path, b"LEGACY CHANGED OWN AUDIO").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    rewrite_audit(&f, &op, |record, _| {
        record.version = 1;
    });
    assert!(super::super::run(&f.storage, &op, &lease, &|| false).is_err());
    assert!(f.path.exists());
    drop(lease);
    assert!(f.reopen().storage.data_write_blocked());
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
    let saved = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let old_scope = saved.saved.snapshot.scope.clone();
    std::fs::write(&f.path, b"CHANGED AUDIO").unwrap();
    assert!(super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f.path.exists());
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let current = accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(current.policy.include_unregistered_audio, false);
    assert_eq!(current.progress.version, 4);
    assert_eq!(current.audits.len(), 1);
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(
        accept(
            &f.storage,
            &review,
            false,
            &lease,
            &crate::test_support::never_cancelled
        )
        .unwrap(),
        current
    );
    assert!(accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(f.operation_count(), 2);
    assert!(f.storage.data_write_blocked());
    let completed = finish(&f, &op, &lease);
    assert!(completed.completed());
    assert!(!f.path.exists());
    assert!(document.is_file() && unknown.is_file());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(completed.saved.snapshot.scope, old_scope);
    assert!(Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(f.operation_count(), 1);
    std::fs::write(&extra, b"CHANGED AGAIN").unwrap();
    assert!(accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(f.operation_count(), 1);
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    assert!(!extra.exists() && !f.path.exists());
}
#[test]
fn cancellation_at_each_transaction_boundary_rolls_back_audit_and_cursor() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    // Each attempt must return to this same receipt before the next boundary.
    for point in [
        "audioRenewalAuditStored",
        "audioRenewalProofStored",
        "beforeAudioRenewalCommit",
    ] {
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
        assert!(cancelled.get(), "{point}");
        assert_eq!(f.operation_count(), 1);
        assert_eq!(
            Receipt::load(
                &f.storage.connection.lock().unwrap(),
                &op,
                &crate::test_support::never_cancelled
            )
            .unwrap(),
            review.receipt
        );
        assert!(f.storage.data_write_blocked() && f.path.exists());
    }
    accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
}
#[test]
fn real_lease_and_unchanged_database_metadata_are_required() {
    let f = Fixture::new();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    controller.begin_exit().unwrap();
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
        assert!(Review::capture(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled
        )
        .is_err());
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
    let old = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let staged = f.storage.root().join(old.reserved_path(0));
    assert!(staged.exists() && !f.path.exists());
    std::fs::write(&staged, b"CHANGED RESERVED AUDIO").unwrap();
    let near = staged.with_file_name(format!(".mon-delete-audio-{op}-999"));
    std::fs::write(&near, b"KEEP LOOKALIKE").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        review.snapshot.historical_audio,
        BTreeSet::from([old.reserved_path(0)])
    );
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let renewed = accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut budget = old.saved.header.snapshot_bytes;
    let (record, _, _) = read_record(
        &f.storage.connection.lock().unwrap(),
        renewed.progress.confirmation.as_ref().unwrap(),
        &old.saved,
        &mut budget,
        &crate::test_support::never_cancelled,
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
fn long_audio_unchanged_reserved_rename_cold_resumes_without_another_consent() {
    let mut f = Fixture::new();
    f.use_long_audio_path();
    let lease = owner();
    let op = started(&f, &lease);
    let original = fixed(&f, &op);
    let stop = Cell::new(false);
    assert!(
        super::super::run_inner(&f.storage, &op, &lease, &|| stop.get(), &|e| {
            if e == "audioRenamed" {
                stop.set(true);
            }
        })
        .is_err()
    );
    let old = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let staged = f.storage.root().join(old.reserved_path(0));
    assert!(old.reserved_path(0).len() > 256 && staged.exists() && !f.path.exists());
    assert!(old.progress.pending.as_ref().unwrap().staged.is_none());
    drop(lease);
    let f = f.reopen();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    finish(&f, &op, &lease);
    assert!(!staged.exists() && !f.path.exists());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(f.operation_count(), 1);
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}

#[test]
fn long_audio_reservation_grammar_is_canonical_and_utf8_bounded() {
    use crate::storage::deletion::files::{valid_audio_reservation_path, valid_path};
    let meeting = "11111111-1111-4111-8111-111111111111";
    let operation = "21111111-1111-4111-8111-111111111111";
    let parent = format!("meetings/{meeting}/audio/a-{}", "あ".repeat(65));
    assert!(parent.len() <= 256);
    for (generation, index) in [(0, 0), (1, 199999), (32, 199999)] {
        let path = format!("{parent}/{}", reserved(operation, generation, index));
        assert!(path.len() > 256 && !valid_path(&path, meeting));
        assert!(valid_audio_reservation_path(&path, meeting));
    }
    for leaf in [
        format!(".mon-delete-audio-{operation}-r0-0"),
        format!(".mon-delete-audio-{operation}-r33-0"),
        format!(".mon-delete-audio-{operation}-r01-0"),
        format!(".mon-delete-audio-{operation}-00"),
        format!(".mon-delete-audio-{operation}-200000"),
        format!(".mon-delete-audio-{operation}-0\n"),
        "ordinary.caf".into(),
    ] {
        assert!(!valid_audio_reservation_path(
            &format!("{parent}/{leaf}"),
            meeting
        ));
    }
    let leaf = reserved(operation, 0, 0);
    for parent in [
        format!("meetings/{operation}/audio"),
        format!("meetings/{meeting}/../audio"),
        format!("meetings/{meeting}//audio"),
        format!("meetings/{meeting}/{}", "あ".repeat(80)),
    ] {
        assert!(!valid_audio_reservation_path(
            &format!("{parent}/{leaf}"),
            meeting
        ));
    }
}

#[test]
fn long_audio_changed_staged_path_requires_new_choice_and_keeps_previous_pending_proof() {
    let mut f = Fixture::new();
    f.use_long_audio_path();
    let lease = owner();
    let op = started(&f, &lease);
    let original = fixed(&f, &op);
    let stop = Cell::new(false);
    assert!(
        super::super::run_inner(&f.storage, &op, &lease, &|| stop.get(), &|e| {
            if e == "audioStaged" {
                stop.set(true);
            }
        })
        .is_err()
    );
    let old = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let staged = f.storage.root().join(old.reserved_path(0));
    assert!(old.reserved_path(0).len() > 256);
    std::fs::write(&staged, b"CHANGED LONG RESERVED AUDIO").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        review.snapshot.historical_audio,
        BTreeSet::from([old.reserved_path(0)])
    );
    assert!(accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let saved = accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut budget = saved.saved.header.snapshot_bytes;
    let (record, _, _) = read_record(
        &f.storage.connection.lock().unwrap(),
        saved.progress.confirmation.as_ref().unwrap(),
        &saved.saved,
        &mut budget,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(record.previous_progress, old.progress);
    assert_eq!(fixed(&f, &op), original);
    drop(lease);
    let f = f.reopen();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    finish(&f, &op, &lease);
    assert!(!staged.exists());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(f.operation_count(), 2);
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}

#[test]
fn long_audio_completed_and_ancestral_reservations_need_distinct_and_remaining_consents() {
    let mut f = Fixture::new();
    f.use_long_audio_path();
    let lease = owner();
    let parent = started(&f, &lease);
    let old = finish(&f, &parent, &lease);
    let returned = f.storage.root().join(old.reserved_path(0));
    std::fs::write(&returned, b"COMPLETED LONG RESERVATION RETURNED").unwrap();
    let review = intent::reconfirmation::Review::capture(
        &f.storage,
        &parent,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    intent::reconfirmation::accept(
        &f.storage,
        &op,
        &review,
        intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    super::super::start(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(&returned, b"CHANGED AGAIN WHILE CHILD WAS UNFINISHED").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    assert!(!returned.exists());
    let source = intent::reconfirmation::Source::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(source.paths().contains(&old.reserved_path(0)));
    drop(lease);
    assert!(!f.reopen().storage.data_write_blocked());
}

#[test]
fn long_audio_lookalike_links_and_directories_never_become_observation_or_removal_permission() {
    for variant in 0..3 {
        let mut f = Fixture::new();
        f.use_long_audio_path();
        let lease = owner();
        let op = started(&f, &lease);
        let old = Receipt::load(
            &f.storage.connection.lock().unwrap(),
            &op,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let path = if variant == 0 {
            f.path.with_file_name(reserved(&op, 0, 999))
        } else {
            f.storage.root().join(old.reserved_path(0))
        };
        let external = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(external.path(), b"NEVER DELETE OUTSIDE").unwrap();
        if variant == 0 {
            std::fs::write(&path, b"UNPLANNED LONG NAME").unwrap();
        } else if variant == 1 {
            std::os::unix::fs::symlink(external.path(), &path).unwrap();
        } else {
            std::fs::create_dir(&path).unwrap();
        }
        assert!(Review::capture(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(super::super::run(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(path.symlink_metadata().is_ok() && f.path.exists());
        assert_eq!(
            std::fs::read(external.path()).unwrap(),
            b"NEVER DELETE OUTSIDE"
        );
        assert_eq!(f.operation_count(), 1);
    }
}

#[test]
fn long_audio_forged_canonical_reserved_membership_is_not_a_typed_receipt() {
    let mut f = Fixture::new();
    f.use_long_audio_path();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"OWN AUDIO CHANGED").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        true,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let unknown = f.path.with_file_name(reserved(&op, 0, 999));
    std::fs::write(&unknown, b"UNOWNED LONG RESERVATION").unwrap();
    let path = unknown
        .strip_prefix(f.storage.root())
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let mut forged = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .plan;
    {
        let snapshot = &mut forged;
        let mut entry = snapshot
            .manifest
            .entries
            .iter()
            .find(|e| !e.directory && e.registered.is_some())
            .unwrap()
            .clone();
        entry.path = path.clone();
        entry.registered = None;
        entry.stage_hint = None;
        entry.proof =
            Some(crate::storage::deletion::fs::Proof::read(&unknown.metadata().unwrap()).unwrap());
        entry.sha256 = Some(intent::hash(b"UNOWNED LONG RESERVATION"));
        snapshot.manifest.entries.push(entry);
        snapshot
            .manifest
            .entries
            .sort_by(|a, b| a.path.cmp(&b.path));
        snapshot.historical_audio.insert(path);
        let db = f.storage.connection.lock().unwrap();
        snapshot.validate(&db).unwrap();
        assert!(crate::storage::deletion::intent::validate_observations(
            &db,
            &snapshot.scope,
            &snapshot.manifest
        )
        .is_err());
    }
    rewrite_audit(&f, &op, |_, snapshot| *snapshot = forged);
    assert!(Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(unknown.exists() && f.path.exists());
    drop(lease);
    assert!(f.reopen().storage.data_write_blocked());
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
    let old = Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(old.progress.next, 1);
    assert!(!f.path.exists() && other.exists());
    std::fs::write(&other, b"FIRST CHANGE").unwrap();
    let first = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let first_saved = accept(
        &f.storage,
        &first,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(&other, b"SECOND CHANGE").unwrap();
    let second = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let saved = accept(
        &f.storage,
        &second,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
        &crate::test_support::never_cancelled,
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
    super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(&f.path, b"RETURNED BEFORE COMPLETION").unwrap();
    assert!(super::super::completion::finish(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(review.receipt.progress.phase, Phase::AudioFilesRemoved);
    accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    assert!(!f.path.exists());
}
#[test]
fn committed_consent_wins_late_cancellation_but_cold_resume_requires_saved_policy() {
    let f = Fixture::new();
    let lease = owner();
    let op = started(&f, &lease);
    std::fs::write(&f.path, b"NEW").unwrap();
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
        .begin_audio_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(
        !Receipt::load(
            &f.storage.connection.lock().unwrap(),
            &op,
            &crate::test_support::never_cancelled
        )
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
        let review = Review::capture(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let saved = accept(
            &f.storage,
            &review,
            false,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let audit = saved.audits[0].clone();
        if orphan {
            let db = f.storage.connection.lock().unwrap();
            let _writer = crate::storage::deletion::fence::write_audio_removal(
                &f.storage,
                &db,
                &saved,
                &lease,
                &crate::test_support::never_cancelled,
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
            assert!(super::super::run(
                &f.storage,
                &op,
                &lease,
                &crate::test_support::never_cancelled
            )
            .is_err());
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
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    std::fs::write(&f.path, b"RETURNED AFTER COMPLETION").unwrap();
    let review = intent::reconfirmation::Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    super::super::start(
        &f.storage,
        &new_op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let saved = accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    finish(&f, &op, &lease);
    drop(lease);
    use crate::storage::deletion::{completion, files, intent as meeting_intent, removal};
    let census = files::Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let reviewed = removal::Reviewed::confirm(&census, true).unwrap();
    let original = meeting_intent::prepare(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &census,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut files = removal::start(
        &f.storage,
        &original,
        &reviewed,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    removal::run(
        &f.storage,
        &mut files,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut rows =
        completion::start(&f.storage, &files, &crate::test_support::never_cancelled).unwrap();
    completion::run(&f.storage, &mut rows, &crate::test_support::never_cancelled).unwrap();
    let f = f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert!(Receipt::load(
        &f.storage.connection.lock().unwrap(),
        &op,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
            let saved = Receipt::load(&db, &op, &crate::test_support::never_cancelled).unwrap();
            let _writer = crate::storage::deletion::fence::write_audio_removal(
                &f.storage,
                &db,
                &saved,
                &lease,
                &crate::test_support::never_cancelled,
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
        let review = Review::capture(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(accept(
            &f.storage,
            &review,
            false,
            &lease,
            &crate::test_support::never_cancelled
        )
        .is_err());
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
    let review = Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let saved = accept(
        &f.storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let db = f.storage.connection.lock().unwrap();
    let reference = saved.progress.confirmation.as_ref().unwrap();
    let mut budget = saved.saved.header.snapshot_bytes;
    let (mut record, _, _) = read_record(
        &db,
        reference,
        &saved.saved,
        &mut budget,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
    assert!(
        Receipt::load(&db, &op, &crate::test_support::never_cancelled)
            .unwrap_err()
            .contains("証拠合計が容量上限")
    );
    drop(db);
    assert!(super::super::run(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
        let review = Review::capture(
            &f.storage,
            &op,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let saved = accept(
            &f.storage,
            &review,
            false,
            &lease,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(
            saved.progress.confirmation.as_ref().unwrap().generation,
            generation
        );
        assert_eq!(saved.audits.len(), generation as usize);
        assert_eq!(fixed(&f, &op), original);
    }
    assert!(Review::capture(
        &f.storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
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
        let saved = Receipt::load(
            &f.storage.connection.lock().unwrap(),
            &op,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
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
            .begin_audio_deletion_maintenance(
                &f.storage,
                &op,
                false,
                false,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        if boundary == "beforeAudioRenewalCommit" {
            assert!(super::super::run(
                &f.storage,
                &op,
                &lease,
                &crate::test_support::never_cancelled
            )
            .is_err());
            let review = Review::capture(
                &f.storage,
                &op,
                &lease,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            accept(
                &f.storage,
                &review,
                false,
                &lease,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
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
        .begin_audio_deletion_maintenance(
            &storage,
            &operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let review = Review::capture(
        &storage,
        &operation,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    accept_inner(
        &storage,
        &review,
        false,
        &lease,
        &crate::test_support::never_cancelled,
        &|event| {
            if event == boundary {
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGKILL);
                }
                panic!("SIGKILL failed");
            }
        },
    )
    .unwrap();
    panic!("boundary not reached");
}
