use super::*;
use std::sync::Arc;

#[test]
fn storage_restart_fence_survives_temporary_lease_release_and_aborted_exit() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    controller.require_storage_restart();
    drop(lease);
    controller.abort_shutdown();
    let state = controller.state.lock().unwrap();
    assert!(state.data_maintenance.is_none());
    assert!(state.storage_restart_required);
    assert!(state.blocks_worker());
    assert!(!state.wake);
    drop(state);
    assert!(controller.begin_export().is_err());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_err());
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    assert_eq!(
        controller.data_operation_blocker().unwrap().as_deref(),
        Some("保存先の切替後は終了して再起動が必要です")
    );
    assert!(controller.wait_idle(Duration::from_millis(10)).is_ok());
}

#[test]
fn data_availability_is_informational_and_reports_the_held_start_gate() {
    let controller = AnalysisController::default();
    assert!(controller.data_operation_blocker().unwrap().is_none());
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert_eq!(
        controller.data_operation_blocker().unwrap().as_deref(),
        Some("バックアップ／復元中")
    );
    drop(lease);
    let export = controller.begin_export().unwrap();
    assert_eq!(
        controller.data_operation_blocker().unwrap().as_deref(),
        Some("Markdownの書き出し中")
    );
    drop(export);
    let model = controller
        .begin_model_preparation(false, false, &|| false)
        .unwrap();
    assert_eq!(
        controller.data_operation_blocker().unwrap().as_deref(),
        Some("AIモデルの検査・準備中")
    );
    drop(model);
    for (gate, reason) in [
        (0, "AI処理中"),
        (1, "録音開始・録音処理中"),
        (2, "音声取り込み中"),
    ] {
        let mut state = controller.state.lock().unwrap();
        state.busy = gate == 0;
        state.recording_intent = gate == 1;
        state.import_intent = gate == 2;
        drop(state);
        assert_eq!(
            controller.data_operation_blocker().unwrap().as_deref(),
            Some(reason)
        );
    }
    controller.shutdown.store(true, Ordering::SeqCst);
    assert_eq!(
        controller.data_operation_blocker().unwrap().as_deref(),
        Some("終了処理中")
    );
}

#[test]
fn detached_runtime_task_retains_data_admission_after_its_receiver_is_dropped() {
    let controller = Arc::new(AnalysisController::default());
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, stop) = std::sync::mpsc::channel();
    let worker = controller.clone();
    let receiver = tauri::async_runtime::spawn(async move {
        let _lease = lease;
        tauri::async_runtime::spawn_blocking(move || {
            started.send(()).unwrap();
            stop.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(worker.state.lock().unwrap().data_maintenance.is_some());
        })
        .await
        .unwrap();
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(receiver);
    controller.begin_exit().unwrap();
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    assert!(controller.begin_export().is_err());
    release.send(()).unwrap();
    controller.wait_idle(Duration::from_secs(2)).unwrap();
    assert!(controller.state.lock().unwrap().data_maintenance.is_none());
    controller.abort_shutdown();
    assert!(controller.begin_export().is_ok());
}

#[test]
fn data_maintenance_cancellation_does_not_release_admission_or_allow_export_or_model_work() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(controller.state.lock().unwrap().blocks_worker());
    assert!(controller.begin_export().is_err());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_err());
    controller.begin_exit().unwrap();
    assert!(lease.cancelled());
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    controller.abort_shutdown();
    assert!(lease.cancelled());
    assert!(controller.begin_export().is_err());
    drop(lease);
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.begin_export().is_ok());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_ok());
}

#[test]
fn data_maintenance_rejects_each_admission_gate_without_pausing_existing_work() {
    for gate in 0..10 {
        let controller = AnalysisController::default();
        {
            let mut state = controller.state.lock().unwrap();
            state.busy = gate == 0;
            state.recording_intent = gate == 1;
            state.import_intent = gate == 2;
            state.export_intent = gate == 8;
            state.wake = true;
        }
        controller.shutdown.store(gate == 3, Ordering::SeqCst);
        let model = (gate == 7).then(|| {
            controller
                .begin_model_preparation(false, false, &|| false)
                .unwrap()
        });
        let data = (gate == 9).then(|| {
            controller
                .begin_data_maintenance(false, false, &|| false)
                .unwrap()
        });
        assert!(
            controller
                .begin_data_maintenance(gate == 4, gate == 5, &|| gate == 6)
                .is_err(),
            "gate {gate}"
        );
        let state = controller.state.lock().unwrap();
        assert_eq!(state.busy, gate == 0);
        assert_eq!(state.recording_intent, gate == 1);
        assert_eq!(state.import_intent, gate == 2);
        assert_eq!(state.export_intent, gate == 8);
        assert!(state.wake);
        drop(state);
        drop(model);
        drop(data);
    }
}

#[test]
fn data_and_markdown_admission_are_atomic_and_exit_seals_the_lane() {
    let controller = Arc::new(AnalysisController::default());
    let start = Arc::new(std::sync::Barrier::new(3));
    let finish = Arc::new(std::sync::Barrier::new(3));
    let (sent, received) = std::sync::mpsc::channel();
    let workers = (0..2)
        .map(|index| {
            let controller = controller.clone();
            let start = start.clone();
            let finish = finish.clone();
            let sent = sent.clone();
            std::thread::spawn(move || {
                start.wait();
                if index == 0 {
                    let lease = controller.begin_data_maintenance(false, false, &|| false);
                    sent.send(lease.is_ok()).unwrap();
                    finish.wait();
                    drop(lease);
                } else {
                    let lease = controller.begin_export();
                    sent.send(lease.is_ok()).unwrap();
                    finish.wait();
                    drop(lease);
                }
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    assert_eq!(
        (0..2)
            .filter(|_| received.recv_timeout(Duration::from_secs(2)).unwrap())
            .count(),
        1
    );
    finish.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    let export = controller.begin_export().unwrap();
    assert!(controller.begin_exit().is_err());
    assert!(!controller.shutdown.load(Ordering::SeqCst));
    drop(export);
    controller.begin_exit().unwrap();
    assert!(controller.begin_export().is_err());
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_err());
    controller.abort_shutdown();
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_ok());
}

#[test]
fn data_maintenance_unwind_releases_only_its_lease_and_retains_wake() {
    let controller = AnalysisController::default();
    controller.state.lock().unwrap().wake = true;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = controller
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        panic!("synthetic data work panic");
    }))
    .is_err());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.state.lock().unwrap().wake);
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_ok());
}

#[test]
fn idle_wait_observes_completion_and_timeout_without_holding_worker_lock() {
    let controller = Arc::new(AnalysisController::default());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    controller.state.lock().unwrap().busy = true;
    assert!(controller.wait_idle(Duration::from_millis(20)).is_err());
    let worker = controller.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        worker.state.lock().unwrap().busy = false;
        worker.idle.notify_all();
    });
    controller.wait_idle(Duration::from_secs(1)).unwrap();
    handle.join().unwrap();
}

#[test]
fn model_preparation_owns_admission_until_work_really_finishes_not_until_cancel() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_preparation(false, false, &|| false)
        .unwrap();
    assert!(controller.state.lock().unwrap().blocks_worker());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_err());
    controller.state.lock().unwrap().cancel_model_preparation();
    assert!(lease.cancelled());
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_err());
    drop(lease);
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(!controller.state.lock().unwrap().blocks_worker());
    assert!(!controller
        .begin_model_preparation(false, false, &|| false)
        .unwrap()
        .cancelled());
}

#[test]
fn model_preparation_rejects_every_admission_gate_without_changing_worker_state() {
    for gate in 0..8 {
        let controller = AnalysisController::default();
        {
            let mut state = controller.state.lock().unwrap();
            state.wake = true;
            state.busy = gate == 0;
            state.recording_intent = gate == 1;
            state.import_intent = gate == 2;
        }
        controller.shutdown.store(gate == 3, Ordering::SeqCst);
        let previous = {
            let state = controller.state.lock().unwrap();
            (
                state.busy,
                state.wake,
                state.recording_intent,
                state.import_intent,
            )
        };
        let held = (gate == 7).then(|| {
            controller
                .begin_model_preparation(false, false, &|| false)
                .unwrap()
        });
        assert!(
            controller
                .begin_model_preparation(gate == 4, gate == 5, &|| gate == 6)
                .is_err(),
            "gate {gate}"
        );
        let state = controller.state.lock().unwrap();
        assert_eq!(
            (
                state.busy,
                state.wake,
                state.recording_intent,
                state.import_intent
            ),
            previous
        );
        assert_eq!(state.model_preparation.is_some(), gate == 7);
        drop(state);
        drop(held);
    }
}

#[test]
fn simultaneous_model_preparations_cannot_both_take_the_worker_lane() {
    let controller = Arc::new(AnalysisController::default());
    let start = Arc::new(std::sync::Barrier::new(3));
    let finish = Arc::new(std::sync::Barrier::new(3));
    let (sent, received) = std::sync::mpsc::channel();
    let workers = (0..2)
        .map(|_| {
            let controller = controller.clone();
            let start = start.clone();
            let finish = finish.clone();
            let sent = sent.clone();
            std::thread::spawn(move || {
                start.wait();
                let lease = controller.begin_model_preparation(false, false, &|| false);
                sent.send(lease.is_ok()).unwrap();
                finish.wait();
                drop(lease);
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    let successes = (0..2)
        .filter(|_| received.recv_timeout(Duration::from_secs(2)).unwrap())
        .count();
    assert_eq!(successes, 1);
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    finish.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    controller.wait_idle(Duration::from_secs(1)).unwrap();
}

#[test]
fn model_preparation_unwinding_releases_lease_and_preserves_pending_wake() {
    let controller = AnalysisController::default();
    controller.state.lock().unwrap().wake = true;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = controller
            .begin_model_preparation(false, false, &|| false)
            .unwrap();
        panic!("synthetic model preparation failure");
    }));
    assert!(result.is_err());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.state.lock().unwrap().wake);
    assert!(controller
        .begin_model_preparation(false, false, &|| false)
        .is_ok());
}

#[test]
fn recording_and_exit_intents_wait_for_model_release_and_keep_their_own_gate() {
    for closing in [false, true] {
        let controller = Arc::new(AnalysisController::default());
        let (started, ready) = std::sync::mpsc::channel();
        let (release, stop) = std::sync::mpsc::channel();
        let worker = controller.clone();
        let thread = std::thread::spawn(move || {
            let lease = worker
                .begin_model_preparation(false, false, &|| false)
                .unwrap();
            started.send(()).unwrap();
            stop.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(lease.cancelled());
            // The lease still holds while finalization is pending.
            assert!(worker.state.lock().unwrap().model_preparation.is_some());
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        {
            let mut state = controller.state.lock().unwrap();
            state.recording_intent = !closing;
            controller.shutdown.store(closing, Ordering::SeqCst);
            state.cancel_model_preparation();
        }
        assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
        release.send(()).unwrap();
        controller.wait_idle(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
        assert!(controller
            .begin_model_preparation(false, false, &|| false)
            .is_err());
        assert_eq!(controller.state.lock().unwrap().recording_intent, !closing);
        assert_eq!(controller.shutdown.load(Ordering::SeqCst), closing);
    }
}

#[test]
fn cancelled_wait_does_not_release_or_clear_model_preparation() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_preparation(false, false, &|| false)
        .unwrap();
    assert!(controller
        .wait_idle_cancellable(Duration::from_secs(1), &|| true)
        .is_err());
    assert!(!lease.cancelled());
    assert!(controller.state.lock().unwrap().model_preparation.is_some());
    drop(lease);
    assert!(controller
        .wait_idle_cancellable(Duration::from_secs(1), &|| true)
        .is_err());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
}

#[test]
#[cfg(unix)]
fn recording_cancellation_keeps_lease_through_real_partial_copy_and_db_finalization() {
    use crate::model_inspection::{Engine, PackageFile, PackageManifest, Role, Target};
    let source = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let meetings = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("engine"),
        b"not an executable; never run",
    )
    .unwrap();
    std::fs::write(source.path().join("model.bin"), vec![b'm'; 200_000]).unwrap();
    let manifest = PackageManifest {
        schema_version: 1,
        engine: Engine::Whisper,
        engine_version: "fixture".into(),
        model_name: "fixture".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "fixture only".into(),
        target: Target::current().unwrap(),
        files: vec![
            PackageFile {
                relative_path: "engine".into(),
                role: Role::Executable,
                expected_size: None,
                expected_hash: None,
            },
            PackageFile {
                relative_path: "model.bin".into(),
                role: Role::Asr,
                expected_size: None,
                expected_hash: None,
            },
        ],
    };
    std::fs::write(
        source.path().join("mon-model.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let report = crate::model_inspection::inspect(source.path(), &|| false).unwrap();
    let storage = Arc::new(Storage::open(meetings.path().into()).unwrap());
    let controller = Arc::new(AnalysisController::default());
    let models = data.path().join("models");
    let token = uuid::Uuid::new_v4().to_string();
    let partial = models.join(&token).join("model.bin");
    let (reached, ready) = std::sync::mpsc::channel();
    let (release, stop) = std::sync::mpsc::channel();
    let worker = controller.clone();
    let db = storage.clone();
    let thread = std::thread::spawn(move || {
        let lease = worker
            .begin_model_preparation(false, false, &|| false)
            .unwrap();
        let reached = std::cell::RefCell::new(Some(reached));
        let result = crate::model_placement::place(
            &db,
            &models,
            source.path(),
            &token,
            &report.review_hash,
            &|| {
                if partial.metadata().is_ok_and(|m| m.len() > 0) {
                    if let Some(sent) = reached.borrow_mut().take() {
                        sent.send(()).unwrap();
                        stop.recv_timeout(Duration::from_secs(3)).unwrap();
                    }
                }
                lease.cancelled()
            },
        );
        assert!(result.unwrap_err().contains("中断"));
        assert!(partial.metadata().unwrap().len() > 0);
        assert!(partial.metadata().unwrap().len() < 200_000);
        assert_eq!(
            std::fs::read(source.path().join("model.bin")).unwrap(),
            vec![b'm'; 200_000]
        );
        // Completion (not the cancel request) releases the lane after DB state.
        let record = db.list_model_placements().unwrap().remove(0);
        assert!(!record.placement_recorded && !record.ready);
        assert!(worker.state.lock().unwrap().model_preparation.is_some());
        drop(lease);
    });
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    {
        let state = controller.state.lock().unwrap();
        state.cancel_model_preparation();
    }
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    release.send(()).unwrap();
    controller.wait_idle(Duration::from_secs(3)).unwrap();
    thread.join().unwrap();
    // Storage deliberately uses an exclusive SQLite connection. Inspect the
    // durable state only after that owner has closed, not through a second
    // live writer that production would reject too.
    drop(storage);
    let db = rusqlite::Connection::open(meetings.path().join("meetings.sqlite3")).unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM model_packages WHERE source_type='manual_import'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unavailable");
}
