//! Real owned moves and current-root consent; no model engine is executed.
use super::*;
use crate::storage::deletion::{
    models,
    relocation::receiver::{synthetic_consent_record, synthetic_model_reconfirmation_records},
};
use models::intent::removal::{self, renewal};
use std::{
    cell::Cell,
    os::unix::{fs::MetadataExt, process::ExitStatusExt},
};

fn fixture() -> (NativeFixture, String, PathBuf) {
    let mut f = NativeFixture::new(false, false);
    let (op, file) = synthetic_model(&f, true);
    f.select();
    // Complete the first owned activation before later legitimate changes.
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    drop(storage);
    drop(b);
    (f, op, file)
}
fn reappear(file: &Path) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, b"SYNTHETIC REAPPEARED MODEL").unwrap();
}
fn owner(
    storage: &Storage,
    op: &str,
) -> crate::analysis_worker::data_maintenance::DataMaintenanceLease {
    AnalysisController::default()
        .begin_model_deletion_maintenance(
            storage,
            op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
}
fn review(
    storage: &Storage,
    root: &Path,
    op: &str,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
) -> models::operations::Prepared {
    let mut p = models::operations::Prepared::pending(
        storage,
        root,
        op,
        lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let v = p.preview();
    assert!(!v.execute_allowed && v.reconfirmation_allowed);
    p.review_remaining(
        storage,
        lease,
        &v.confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(p.preview().execute_allowed);
    p
}

#[test]
fn model_relocated_consent_requires_current_review_and_unknown_choice_without_rewriting_original() {
    let (f, op, file) = fixture();
    reappear(&file);
    let unknown = file.with_file_name("unknown.bin");
    std::fs::write(&unknown, b"NEW UNKNOWN MODEL BODY").unwrap();
    let neighbor = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("unrelated-model");
    std::fs::create_dir(&neighbor).unwrap();
    std::fs::write(neighbor.join("KEEP"), b"NEIGHBOR KEEP").unwrap();
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let old = synthetic_consent_record(&storage, &op);
    let audio = f.live_record(&storage);
    let lease = owner(&storage, &op);
    assert!(removal::run(&storage, &op, &lease, &crate::test_support::never_cancelled).is_err());
    assert!(file.exists());
    let mut p = review(&storage, &b.models_root, &op, &lease);
    assert!(p.preview().unregistered_count > 0);
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
    assert!(file.exists() && unknown.exists());
    p.refresh(&storage, &lease, &crate::test_support::never_cancelled)
        .unwrap();
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
    assert!(done.body_deleted && done.history_retained && done.restart_required);
    assert!(!file.parent().unwrap().exists());
    assert_eq!(
        std::fs::read(neighbor.join("KEEP")).unwrap(),
        b"NEIGHBOR KEEP"
    );
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert_eq!(f.live_record(&storage), audio);
    let audits = synthetic_model_reconfirmation_records(&storage);
    assert_eq!(audits.len(), 1);
    let record: serde_json::Value = serde_json::from_str(&audits[0].1).unwrap();
    let plan: serde_json::Value = serde_json::from_str(&audits[0].2).unwrap();
    assert_eq!(record["version"], 2);
    assert_eq!(record["sourceOperation"], op);
    assert_eq!(record["previousProgress"]["phase"], "completed");
    for (field, path) in [
        ("storageRoot", f.target.clone()),
        ("database", f.target.join("meetings.sqlite3")),
    ] {
        let m = std::fs::metadata(path).unwrap();
        assert_eq!(plan["currentRoot"][field]["device"].as_u64(), Some(m.dev()));
        assert_eq!(plan["currentRoot"][field]["inode"].as_u64(), Some(m.ino()));
    }
    assert!(storage.create_meeting("RESTART REQUIRED").is_err());
    drop(lease);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        assert_eq!(synthetic_model_reconfirmation_records(&storage), audits);
        assert!(storage
            .create_meeting("NORMAL AFTER CURRENT CONSENT")
            .is_ok());
        assert_eq!(f.live_record(&storage), audio);
        drop(storage);
        drop(b);
    }
}

#[test]
fn model_relocated_consent_saved_policy_resumes_after_cold_cancel_and_supports_new_current_scope() {
    let (f, op, file) = fixture();
    reappear(&file);
    let (b, storage) = f.open();
    let old = synthetic_consent_record(&storage, &op);
    let lease = owner(&storage, &op);
    let mut p = review(&storage, &b.models_root, &op, &lease);
    let stop = Cell::new(false);
    assert!(p
        .execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &|| stop.get(),
            &|phase| {
                if phase == "model_delete_scope_fixed" {
                    stop.set(true);
                }
            }
        )
        .is_err());
    assert!(file.exists());
    let audits = synthetic_model_reconfirmation_records(&storage);
    assert_eq!(audits.len(), 1);
    drop(lease);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let lease = owner(&storage, &op);
    let mut p = models::operations::Prepared::pending(
        &storage,
        &b.models_root,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(p.preview().execute_allowed);
    assert_eq!(p.preview().include_unregistered_recorded, Some(true));
    assert!(
        p.execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap()
        .body_deleted
    );
    assert_eq!(synthetic_model_reconfirmation_records(&storage), audits);
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    drop(lease);
    drop(storage);
    drop(b);
    reappear(&file);
    let (b, storage) = f.open();
    let lease = owner(&storage, &op);
    let mut p = review(&storage, &b.models_root, &op, &lease);
    assert!(
        p.execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap()
        .body_deleted
    );
    assert_eq!(synthetic_model_reconfirmation_records(&storage).len(), 2);
    assert_eq!(synthetic_consent_record(&storage, &op), old);
}

#[test]
fn model_relocated_consent_all_precommit_boundaries_roll_back_both_audit_and_progress() {
    for boundary in [
        "auditStored",
        "proofStored",
        "auditCompleted",
        "beforeCommit",
    ] {
        let (f, op, file) = fixture();
        reappear(&file);
        let (b, storage) = f.open();
        let old = synthetic_consent_record(&storage, &op);
        let lease = owner(&storage, &op);
        let v =
            renewal::review(&storage, &op, &lease, &crate::test_support::never_cancelled).unwrap();
        let stop = Cell::new(false);
        assert!(
            renewal::accept_observed(&storage, &v, true, &lease, &|| stop.get(), &|event| {
                if event == boundary {
                    stop.set(true);
                }
            })
            .is_err()
        );
        assert!(stop.get());
        assert!(file.exists());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
        // The old-root execution remains refused after every rollback.
        assert!(
            removal::run(&storage, &op, &lease, &crate::test_support::never_cancelled).is_err()
        );
        drop(lease);
        drop(storage);
        drop(b);
        let (_b, storage) = f.open();
        assert!(storage.data_write_blocked());
        assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
        assert_eq!(synthetic_consent_record(&storage, &op), old);
    }
}

#[test]
fn model_relocated_consent_cancel_or_inventory_change_does_not_publish_audit() {
    let (f, op, file) = fixture();
    reappear(&file);
    let (_b, storage) = f.open();
    let old = synthetic_consent_record(&storage, &op);
    let lease = owner(&storage, &op);
    assert!(renewal::review(&storage, &op, &lease, &|| true).is_err());
    let v = renewal::review(&storage, &op, &lease, &crate::test_support::never_cancelled).unwrap();
    std::fs::write(&file, b"CHANGED AFTER CURRENT ROOT REVIEW").unwrap();
    assert!(renewal::accept(
        &storage,
        &v,
        v.census_hash(),
        true,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"CHANGED AFTER CURRENT ROOT REVIEW"
    );
}

#[test]
fn model_relocated_consent_unowned_database_copy_never_gets_review_or_execution() {
    let (f, op, file) = fixture();
    reappear(&file);
    let root = f
        ._base
        .path()
        .canonicalize()
        .unwrap()
        .join("unowned-model-copy");
    std::fs::create_dir(&root).unwrap();
    std::fs::copy(
        f.target.join("meetings.sqlite3"),
        root.join("meetings.sqlite3"),
    )
    .unwrap();
    let storage = Storage::open(root).unwrap();
    assert!(storage.data_write_blocked());
    let old = synthetic_consent_record(&storage, &op);
    assert!(AnalysisController::default()
        .begin_model_deletion_maintenance(
            &storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(renewal::review(&storage, &op, &lease, &crate::test_support::never_cancelled).is_err());
    assert!(removal::run(&storage, &op, &lease, &crate::test_support::never_cancelled).is_err());
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
    assert!(file.exists());
}

#[test]
fn model_relocated_consent_linked_model_never_adopts_external_body() {
    let (f, op, file) = fixture();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let external = f
        ._base
        .path()
        .canonicalize()
        .unwrap()
        .join("external-model.bin");
    std::fs::write(&external, b"EXTERNAL KEEP").unwrap();
    symlink(&external, &file).unwrap();
    let (b, storage) = f.open();
    let old = synthetic_consent_record(&storage, &op);
    let lease = owner(&storage, &op);
    let p = models::operations::Prepared::pending(
        &storage,
        &b.models_root,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!p.preview().execute_allowed && !p.preview().reconfirmation_allowed);
    assert!(renewal::review(&storage, &op, &lease, &crate::test_support::never_cancelled).is_err());
    assert!(synthetic_model_reconfirmation_records(&storage).is_empty());
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert_eq!(std::fs::read(&external).unwrap(), b"EXTERNAL KEEP");
}

#[test]
#[ignore = "child process only; owned current-root model fixture required"]
fn model_relocated_consent_crash_child() {
    let config = PathBuf::from(std::env::var("MON_MODEL_RELOCATED_CONFIG").unwrap());
    let legacy = PathBuf::from(std::env::var("MON_MODEL_RELOCATED_LEGACY").unwrap());
    let op = std::env::var("MON_MODEL_RELOCATED_OPERATION").unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let lease = owner(&storage, &op);
    let v = renewal::review(&storage, &op, &lease, &crate::test_support::never_cancelled).unwrap();
    renewal::accept_observed(
        &storage,
        &v,
        true,
        &lease,
        &crate::test_support::never_cancelled,
        &|event| {
            if event == "committed" {
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGKILL);
                }
                panic!("synthetic child kill failed");
            }
        },
    )
    .unwrap();
    panic!("synthetic child did not reach committed boundary");
}

#[test]
fn model_relocated_consent_sigkill_after_commit_restores_wal_and_keeps_saved_policy() {
    let (f, op, file) = fixture();
    reappear(&file);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "bootstrap::placement::activation::tests::native_history::model_reconfirmation::model_relocated_consent_crash_child"])
        .env("MON_MODEL_RELOCATED_CONFIG", &f.config)
        .env("MON_MODEL_RELOCATED_LEGACY", &f.legacy)
        .env("MON_MODEL_RELOCATED_OPERATION", &op)
        .status().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(file.exists());
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let old = synthetic_consent_record(&storage, &op);
    let audits = synthetic_model_reconfirmation_records(&storage);
    assert_eq!(audits.len(), 1);
    let lease = owner(&storage, &op);
    let mut p = models::operations::Prepared::pending(
        &storage,
        &b.models_root,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(p.preview().execute_allowed);
    assert_eq!(p.preview().include_unregistered_recorded, Some(true));
    assert!(
        p.execute(
            &storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap()
        .body_deleted
    );
    assert!(!file.exists());
    assert_eq!(synthetic_consent_record(&storage, &op), old);
    assert_eq!(synthetic_model_reconfirmation_records(&storage), audits);
    drop(lease);
    drop(storage);
    drop(b);
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(synthetic_model_reconfirmation_records(&storage), audits);
}

#[test]
fn model_relocated_consent_repeat_moves_retain_all_native_anchors_and_original_model_history() {
    let (f, op, file) = fixture();
    reappear(&file);
    let (b, storage) = f.open();
    let old = synthetic_consent_record(&storage, &op);
    let native_models = b.models_root.clone();
    let lease = owner(&storage, &op);
    let mut p = review(&storage, &b.models_root, &op, &lease);
    p.execute(
        &storage,
        &lease,
        &p.preview().confirmation_id,
        true,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let first_audit = synthetic_model_reconfirmation_records(&storage);
    drop(lease);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(b.models_root, native_models);
        let next = f
            ._base
            .path()
            .canonicalize()
            .unwrap()
            .join(Uuid::new_v4().to_string());
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&next)
            .unwrap();
        super::repeated::select_current(&b, &storage, &next);
        drop(storage);
        drop(b);
        reappear(&file);
        let (b, storage) = f.open();
        assert!(storage.data_write_blocked());
        let lease = owner(&storage, &op);
        let mut p = review(&storage, &b.models_root, &op, &lease);
        assert!(
            p.execute(
                &storage,
                &lease,
                &p.preview().confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .body_deleted
        );
        assert_eq!(synthetic_consent_record(&storage, &op), old);
        let audits = synthetic_model_reconfirmation_records(&storage);
        assert_eq!(audits[0], first_audit[0]);
        let plan: serde_json::Value = serde_json::from_str(&audits.last().unwrap().2).unwrap();
        let m = std::fs::metadata(storage.root().join("meetings.sqlite3")).unwrap();
        assert_eq!(
            plan["currentRoot"]["database"]["inode"].as_u64(),
            Some(m.ino())
        );
        drop(lease);
        drop(storage);
        drop(b);
    }
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(synthetic_model_reconfirmation_records(&storage).len(), 3);
    assert_eq!(synthetic_consent_record(&storage, &op), old);
}
