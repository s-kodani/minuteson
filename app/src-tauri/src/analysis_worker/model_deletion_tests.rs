//! Shared admission tests run on Windows and macOS without real model engines.
use super::*;
use std::sync::{Arc, Barrier};

#[test]
fn model_deletion_coexists_with_existing_and_new_notification_owners() {
    let c = AnalysisController::default();
    let writer = c.begin_notice_storage().unwrap();
    let lease = c.begin_model_deletion(false, false, &|| false).unwrap();
    writer.validate_active().unwrap();
    assert!(!c.notice_storage_paused(None));
    let next = c.begin_notice_storage().unwrap();
    next.validate_active().unwrap();
    lease.validate_model_deletion().unwrap();
    assert!(c.state.lock().unwrap().data_maintenance.is_none());
    assert!(c.state.lock().unwrap().blocks_worker());
    assert_eq!(
        c.deletion_operation_blocker().unwrap().as_deref(),
        Some("モデルの削除確認・処理中")
    );
    drop(lease);
    writer.validate_active().unwrap();
    next.validate_active().unwrap();
    assert!(!c.state.lock().unwrap().blocks_worker());
}

#[test]
fn model_and_general_or_meeting_proofs_are_not_interchangeable() {
    let c = AnalysisController::default();
    let global = c.begin_data_maintenance(false, false, &|| false).unwrap();
    assert!(global.validate_model_deletion().is_err());
    drop(global);
    let storage_root = tempfile::tempdir().unwrap();
    let storage = Storage::open(storage_root.path().join("data")).unwrap();
    let meeting = storage.create_meeting("synthetic scope").unwrap().id;
    let scoped = c.begin_meeting_deletion(&storage, &meeting).unwrap();
    scoped.validate_meeting(&meeting).unwrap();
    assert!(scoped.validate_model_deletion().is_err());
    storage.release_meeting_deletion(&meeting).unwrap();
    drop(scoped);
    let model = c.begin_model_deletion(false, false, &|| false).unwrap();
    assert!(model.validate_active().is_err());
    assert!(model.validate_meeting(&meeting).is_err());
    model.validate_model_deletion().unwrap();
}

#[test]
fn model_deletion_preserves_all_other_admission_gates() {
    for gate in 0..12 {
        let c = AnalysisController::default();
        {
            let mut s = c.state.lock().unwrap();
            s.busy = gate == 0;
            s.recording_intent = gate == 1;
            s.import_intent = gate == 2;
            s.storage_restart_required = gate == 3;
            if gate == 4 {
                s.model_preparation = Some(Arc::new(AtomicBool::new(false)));
            }
            if gate == 5 {
                s.data_maintenance = Some(Arc::new(AtomicBool::new(false)));
            }
            if gate == 6 {
                s.meeting_deletions
                    .insert("synthetic".into(), Arc::new(AtomicBool::new(false)));
            }
            s.export_intent = gate == 7;
        }
        c.shutdown.store(gate == 8, Ordering::SeqCst);
        assert!(
            c.begin_model_deletion(gate == 9, gate == 10, &|| gate == 11)
                .is_err(),
            "gate {gate}"
        );
        assert!(c.state.lock().unwrap().model_deletion.is_none());
    }
    let c = AnalysisController::default();
    let model = c.begin_model_deletion(false, false, &|| false).unwrap();
    assert!(c.begin_model_deletion(false, false, &|| false).is_err());
    assert!(c.begin_data_maintenance(false, false, &|| false).is_err());
    assert!(c.begin_deletion_wait(false, false, &|| false).is_err());
    assert!(c.begin_export().is_err());
    assert!(c.begin_model_preparation(false, false, &|| false).is_err());
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("data")).unwrap();
    let meeting = storage
        .create_meeting("synthetic model exclusion")
        .unwrap()
        .id;
    assert!(c.begin_meeting_deletion(&storage, &meeting).is_err());
    drop(model);
    assert!(c.begin_model_preparation(false, false, &|| false).is_ok());
}

#[test]
fn model_admission_races_cannot_acquire_two_conflicting_owners() {
    for other in 0..5 {
        let c = Arc::new(AnalysisController::default());
        let start = Arc::new(Barrier::new(3));
        let finish = Arc::new(Barrier::new(3));
        let (tx, rx) = std::sync::mpsc::channel();
        let tasks: Vec<_> = (0..2)
            .map(|index| {
                let (c, start, finish, tx) = (c.clone(), start.clone(), finish.clone(), tx.clone());
                std::thread::spawn(move || {
                    start.wait();
                    // Keep every successful lease alive until both results arrive.
                    let held = if index == 0 || other == 0 {
                        c.begin_model_deletion(false, false, &|| false)
                            .map(|l| (Some(l), None, None))
                    } else if other == 1 {
                        c.begin_data_maintenance(false, false, &|| false)
                            .map(|l| (Some(l), None, None))
                    } else if other == 2 {
                        c.begin_deletion_wait(false, false, &|| false)
                            .map(|l| (Some(l), None, None))
                    } else if other == 3 {
                        c.begin_export().map(|l| (None, Some(l), None))
                    } else {
                        c.begin_model_preparation(false, false, &|| false)
                            .map(|l| (None, None, Some(l)))
                    };
                    tx.send(held.is_ok()).unwrap();
                    finish.wait();
                    drop(held);
                })
            })
            .collect();
        start.wait();
        let winners = (0..2)
            .filter(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .count();
        finish.wait();
        for task in tasks {
            task.join().unwrap();
        }
        assert_eq!(winners, 1, "competitor {other}");
        c.wait_idle(Duration::from_millis(20)).unwrap();
    }
}

#[test]
fn model_shutdown_and_stale_release_never_revive_or_unlock_another_owner() {
    let c = AnalysisController::default();
    let lease = c.begin_model_deletion(false, false, &|| false).unwrap();
    c.begin_exit().unwrap();
    assert!(lease.cancelled());
    assert!(c.wait_idle(Duration::from_millis(10)).is_err());
    c.abort_shutdown();
    assert!(lease.validate_model_deletion().is_err());
    assert!(c.begin_export().is_err());
    drop(lease);
    c.wait_idle(Duration::from_millis(20)).unwrap();
    let stale = c.begin_model_deletion(false, false, &|| false).unwrap();
    let replacement = Arc::new(AtomicBool::new(false));
    c.state.lock().unwrap().model_deletion = Some(replacement.clone());
    assert!(stale.validate_model_deletion().is_err());
    drop(stale);
    assert!(Arc::ptr_eq(
        c.state.lock().unwrap().model_deletion.as_ref().unwrap(),
        &replacement
    ));
}

#[test]
fn model_unwind_releases_only_model_ownership_and_preserves_notifications() {
    let c = AnalysisController::default();
    let writer = c.begin_notice_storage().unwrap();
    c.state.lock().unwrap().wake = true;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = c.begin_model_deletion(false, false, &|| false).unwrap();
        panic!("synthetic model worker failure");
    }))
    .is_err());
    c.wait_idle(Duration::from_millis(20)).unwrap();
    assert!(c.state.lock().unwrap().wake);
    writer.validate_active().unwrap();
    assert!(c.begin_model_deletion(false, false, &|| false).is_ok());
}
