//! Current-root consent through the real owned cold bootstrap and controller.
//! Production cutover preserves every original consent and historical target.
use super::*;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, Ordering};

fn immutable(storage: &Storage, operation: &str) -> String {
    crate::storage::deletion::relocation::receiver::synthetic_consent_record(storage, operation)
}
fn audits(storage: &Storage) -> Vec<(String, String, i64)> {
    crate::storage::deletion::relocation::receiver::synthetic_meeting_reconfirmation_records(
        storage,
    )
}
fn fixture() -> NativeFixture {
    let mut f = NativeFixture::new(false, true);
    f.select();
    // First admission still verifies the actual immutable placement image.
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    drop(storage);
    drop(b);
    f
}
fn reappear(f: &NativeFixture) -> (PathBuf, PathBuf) {
    let relative = format!("meetings/{}/reappeared.bin", f.meeting);
    let target = f.target.join(&relative);
    let original = f.legacy.join(relative);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::create_dir_all(original.parent().unwrap()).unwrap();
    std::fs::write(&target, b"CURRENT UNKNOWN BODY").unwrap();
    std::fs::write(&original, b"ORIGINAL ROOT MUST REMAIN").unwrap();
    (target, original)
}
fn review(storage: &Storage, operation: &str) -> (AnalysisController, operations::Prepared) {
    let controller = AnalysisController::default();
    let p =
        operations::Prepared::pending(storage, operation, &crate::test_support::never_cancelled)
            .unwrap();
    let preview = p.preview();
    assert!(!preview.execute_allowed && preview.reconfirmation_allowed);
    assert!(preview.row_phase_committed && !preview.keep_allowed);
    (controller, p)
}

#[test]
fn deletion_relocated_reconfirmation_uses_new_current_root_consent_and_preserves_original_evidence()
{
    let f = fixture();
    let (target, original) = reappear(&f);
    let (b, storage) = f.open();
    let operation = f.full_deletion.as_deref().unwrap();
    let old = immutable(&storage, operation);
    let old_audio = f.live_record(&storage);
    assert!(storage.data_write_blocked());
    let (controller, mut p) = review(&storage, operation);
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(target.exists());
    assert!(audits(&storage).is_empty());
    p.review_remaining(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = p.preview();
    assert!(preview.execute_allowed && preview.row_phase_committed);
    assert_eq!(preview.unregistered_count, 1);
    // A separate unknown-file decision is required even after the new review.
    assert!(p
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(audits(&storage).is_empty());
    assert_eq!(immutable(&storage, operation), old);
    assert!(target.exists());
    assert!(!p.preview().execute_allowed);
    p.review_remaining(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let done = p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted && done.restart_required);
    assert!(!target.exists());
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"ORIGINAL ROOT MUST REMAIN"
    );
    assert_eq!(immutable(&storage, operation), old);
    assert_eq!(f.live_record(&storage), old_audio);
    let recorded = audits(&storage);
    assert_eq!(recorded.len(), 1);
    let audit: serde_json::Value = serde_json::from_str(&recorded[0].1).unwrap();
    assert_eq!(audit["version"], 4);
    assert_eq!(audit["sourceOperation"], operation);
    assert_eq!(audit["previousDatabasePhase"], "finished");
    let original_record: serde_json::Value = serde_json::from_str(&old).unwrap();
    let original_intent: serde_json::Value =
        serde_json::from_str(original_record[0].as_str().unwrap()).unwrap();
    assert_eq!(audit["observations"]["scope"], original_intent["scope"]);
    for (name, path) in [
        ("root", f.target.clone()),
        ("database", f.target.join("meetings.sqlite3")),
    ] {
        let metadata = std::fs::metadata(path).unwrap();
        let identity = &audit["observations"]["fileHeader"][name];
        assert_eq!(identity["device"].as_u64(), Some(metadata.dev()));
        assert_eq!(identity["inode"].as_u64(), Some(metadata.ino()));
        assert_ne!(identity, &original_intent["fileHeader"][name]);
    }
    assert!(audit["observations"]["fileHeader"]["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(recorded[0].2 >= 0);
    assert!(storage.create_meeting("RESTART STILL REQUIRED").is_err());
    drop(lease);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(immutable(&storage, operation), old);
        assert_eq!(f.live_record(&storage), old_audio);
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        assert!(storage
            .create_meeting("NORMAL CURRENT ROOT AFTER NEW CONSENT")
            .is_ok());
        assert_eq!(audits(&storage), recorded);
        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"ORIGINAL ROOT MUST REMAIN"
        );
        drop(storage);
        drop(b);
    }
}

#[test]
fn deletion_relocated_reconfirmation_saved_consent_survives_cold_stop_and_repeated_current_root_review(
) {
    let f = fixture();
    let (target, original) = reappear(&f);
    let operation = f.full_deletion.as_deref().unwrap();
    let (b, storage) = f.open();
    let old = immutable(&storage, operation);
    let (controller, mut p) = review(&storage, operation);
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    p.review_remaining(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let stop = AtomicBool::new(false);
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &|| stop.load(Ordering::SeqCst),
            &|phase| {
                if phase == "delete_scope_fixed" {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        )
        .is_err());
    assert!(target.exists());
    assert_eq!(audits(&storage).len(), 1);
    assert_eq!(immutable(&storage, operation), old);
    drop(lease);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let controller = AnalysisController::default();
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let mut p =
        operations::Prepared::pending(&storage, operation, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(p.preview().execute_allowed);
    let done = p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted && done.restart_required);
    assert!(!target.exists());
    assert_eq!(immutable(&storage, operation), old);
    drop(lease);
    drop(storage);
    drop(b);
    // A second review at this same current root must extend, not rewrite, v4.
    let (target, _) = reappear(&f);
    let (b, storage) = f.open();
    let (controller, mut p) = review(&storage, operation);
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    p.review_remaining(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    p.execute(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        true,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    assert!(!target.exists());
    let recorded = audits(&storage);
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().any(
        |(_, json, _)| serde_json::from_str::<serde_json::Value>(json).unwrap()["version"] == 4
    ));
    assert_eq!(immutable(&storage, operation), old);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"ORIGINAL ROOT MUST REMAIN"
    );
    drop(lease);
    drop(storage);
    drop(b);
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(audits(&storage), recorded);
}

#[test]
fn deletion_relocated_reconfirmation_cancel_and_inventory_change_never_publish_consent() {
    let f = fixture();
    let (target, original) = reappear(&f);
    let operation = f.full_deletion.as_deref().unwrap();
    let (_b, storage) = f.open();
    let old = immutable(&storage, operation);
    let (controller, mut p) = review(&storage, operation);
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let confirmation = p.preview().confirmation_id;
    assert!(p
        .review_remaining(&storage, &lease, &confirmation, &|| true)
        .is_err());
    assert_eq!(p.preview().confirmation_id, confirmation);
    p.review_remaining(
        &storage,
        &lease,
        &confirmation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &|| true,
            &|_| {}
        )
        .is_err());
    assert!(audits(&storage).is_empty());
    std::fs::write(&target, b"CHANGED AFTER REVIEW").unwrap();
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(audits(&storage).is_empty());
    assert_eq!(immutable(&storage, operation), old);
    assert_eq!(std::fs::read(&target).unwrap(), b"CHANGED AFTER REVIEW");
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"ORIGINAL ROOT MUST REMAIN"
    );
    assert!(storage.create_meeting("MUST STAY FENCED").is_err());
}

#[test]
fn deletion_relocated_reconfirmation_all_precommit_boundaries_roll_back_audit_and_original_progress(
) {
    let f = fixture();
    let (target, original) = reappear(&f);
    let operation = f.full_deletion.as_deref().unwrap();
    let (_b, storage) = f.open();
    let old = immutable(&storage, operation);
    let controller = AnalysisController::default();
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    for event in [
        "reviewIntentStored",
        "reviewPathStored",
        "reviewRecorded",
        "beforeReviewCommit",
    ] {
        crate::storage::deletion::relocation::receiver::synthetic_reconfirmation_cancel_at(
            &storage, operation, &lease, event,
        );
        assert!(audits(&storage).is_empty());
        assert_eq!(immutable(&storage, operation), old);
        assert_eq!(std::fs::read(&target).unwrap(), b"CURRENT UNKNOWN BODY");
        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"ORIGINAL ROOT MUST REMAIN"
        );
        let (_, p) = review(&storage, operation);
        assert!(!p.preview().execute_allowed);
        assert!(storage
            .create_meeting("STILL NEEDS CURRENT CONSENT")
            .is_err());
    }
}

#[test]
fn deletion_relocated_reconfirmation_unowned_database_copy_has_no_review_or_delete_admission() {
    let f = fixture();
    let operation = f.full_deletion.as_deref().unwrap();
    let root = f._base.path().canonicalize().unwrap().join("unowned-copy");
    std::fs::create_dir(&root).unwrap();
    std::fs::copy(
        f.target.join("meetings.sqlite3"),
        root.join("meetings.sqlite3"),
    )
    .unwrap();
    let file = root.join(format!("meetings/{}/unowned.bin", f.meeting));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"UNOWNED COPY MUST REMAIN").unwrap();
    let storage = Storage::open(root).unwrap();
    assert!(storage.data_write_blocked());
    let old = immutable(&storage, operation);
    assert!(operations::Prepared::pending(
        &storage,
        operation,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let controller = AnalysisController::default();
    assert!(controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"UNOWNED COPY MUST REMAIN");
    assert_eq!(immutable(&storage, operation), old);
    assert!(audits(&storage).is_empty());
}

#[test]
fn deletion_relocated_reconfirmation_external_link_is_never_censused_or_adopted() {
    let f = fixture();
    let operation = f.full_deletion.as_deref().unwrap();
    let external = f._base.path().canonicalize().unwrap().join("outside.bin");
    std::fs::write(&external, b"EXTERNAL BODY MUST REMAIN").unwrap();
    let link = f.target.join(format!("meetings/{}/linked.bin", f.meeting));
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&external, &link).unwrap();
    let (_b, storage) = f.open();
    let old = immutable(&storage, operation);
    assert!(storage.data_write_blocked());
    let mut p =
        operations::Prepared::pending(&storage, operation, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(!p.preview().execute_allowed && !p.preview().reconfirmation_allowed);
    let controller = AnalysisController::default();
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(p
        .review_remaining(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert_eq!(immutable(&storage, operation), old);
    assert!(audits(&storage).is_empty());
    assert!(std::fs::symlink_metadata(link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read(external).unwrap(),
        b"EXTERNAL BODY MUST REMAIN"
    );
}

#[test]
#[ignore = "child process only; owned relocated reconfirmation fixture required"]
fn deletion_relocated_reconfirmation_crash_child() {
    let config = PathBuf::from(std::env::var_os("MON_RELOCATION_CONSENT_CONFIG").unwrap());
    let legacy = PathBuf::from(std::env::var_os("MON_RELOCATION_CONSENT_LEGACY").unwrap());
    let operation = std::env::var("MON_RELOCATION_CONSENT_OPERATION").unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let (controller, mut p) = review(&storage, &operation);
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            &operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    p.review_remaining(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let _ = p.execute(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        true,
        &crate::test_support::never_cancelled,
        &|phase| {
            if phase == "delete_scope_fixed" {
                // No Drop/checkpoint, exactly after the durable new consent and
                // before any unlink. The parent owns this synthetic fixture.
                std::process::exit(0);
            }
        },
    );
    panic!("durable reconfirmation boundary was not reached");
}

#[test]
fn deletion_relocated_reconfirmation_crash_after_new_consent_restores_wal_and_requires_saved_policy(
) {
    let f = fixture();
    let operation = f.full_deletion.as_deref().unwrap();
    let (target, original) = reappear(&f);
    let (b, storage) = f.open();
    let old = immutable(&storage, operation);
    drop(storage);
    drop(b);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("bootstrap::placement::activation::tests::native_history::reconfirmation::deletion_relocated_reconfirmation_crash_child")
        .arg("--ignored")
        .env("MON_RELOCATION_CONSENT_CONFIG", &f.config)
        .env("MON_RELOCATION_CONSENT_LEGACY", &f.legacy)
        .env("MON_RELOCATION_CONSENT_OPERATION", operation)
        .output().unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(
        f.target
            .join("meetings.sqlite3-wal")
            .metadata()
            .unwrap()
            .len()
            > 32
    );
    assert!(target.exists());
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    assert_eq!(immutable(&storage, operation), old);
    let recorded = audits(&storage);
    assert_eq!(recorded.len(), 1);
    let controller = AnalysisController::default();
    let lease = controller
        .begin_deletion_maintenance(
            &storage,
            operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let mut p =
        operations::Prepared::pending(&storage, operation, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(p.preview().execute_allowed);
    let done = p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted && done.restart_required);
    assert!(!target.exists());
    assert_eq!(audits(&storage), recorded);
    assert_eq!(immutable(&storage, operation), old);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"ORIGINAL ROOT MUST REMAIN"
    );
    drop(lease);
    drop(storage);
    drop(b);
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(audits(&storage), recorded);
    assert_eq!(immutable(&storage, operation), old);
}
