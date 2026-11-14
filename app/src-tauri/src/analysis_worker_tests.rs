use super::*;
use std::sync::Arc;

#[test]
fn deletion_never_holds_analysis_lock_while_waiting_for_another_meetings_database() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::open(temp.path().join("data")).unwrap());
    let meeting = storage.create_meeting("削除対象").unwrap().id;
    let other = storage.create_meeting("履歴保存中").unwrap().id;
    let controller = Arc::new(AnalysisController::default());
    let writer = controller
        .begin_scoped_notice_storage(Some(&other))
        .unwrap();
    let mut task = None;
    crate::storage::with_test_database(&storage, |_db| {
        let c = controller.clone();
        let db = storage.clone();
        let target = meeting.clone();
        task = Some(std::thread::spawn(move || {
            c.begin_meeting_deletion(&db, &target)
        }));
        let started = Instant::now();
        let mut reserved = false;
        while started.elapsed() < Duration::from_secs(2) {
            if let Ok(state) = controller.state.try_lock() {
                if state.meeting_deletions.contains_key(&meeting) {
                    reserved = true;
                    break;
                }
            }
            std::thread::yield_now();
        }
        assert!(
            reserved,
            "deletion held the analysis mutex while waiting for SQLite"
        );
        writer.validate_active().unwrap();
    });
    let deletion = task.unwrap().join().unwrap().unwrap();
    writer.validate_active().unwrap();
    storage.release_meeting_deletion(&meeting).unwrap();
    drop(deletion);
}

#[test]
fn shared_history_five_second_timeout_and_unwind_keep_files_protected() {
    let c = AnalysisController::default();
    let writer = c.begin_notice_storage().unwrap();
    let elapsed = std::cell::Cell::new(Duration::ZERO);
    let clock = |_: Instant| {
        let value = elapsed.get();
        elapsed.set(value + Duration::from_secs(5));
        value
    };
    assert!(c
        .begin_data_maintenance_with_elapsed(false, false, &|| false, false, true, &clock)
        .err()
        .unwrap()
        .contains("通知履歴の処理が終わらないため"));
    assert_eq!(elapsed.get(), Duration::from_secs(10));
    assert!(c.state.lock().unwrap().data_maintenance.is_none());
    writer.validate_active().unwrap();
    let calls = std::cell::Cell::new(0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = c.begin_deletion_wait(false, false, &|| {
            calls.set(calls.get() + 1);
            if calls.get() > 1 {
                panic!("injected wait failure")
            }
            false
        });
    }));
    assert!(result.is_err());
    assert!(c
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .data_maintenance
        .is_none());
    drop(writer);
}

#[test]
fn deletion_reservation_drains_history_without_starving_other_meetings() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::open(temp.path().join("data")).unwrap());
    let meeting = storage.create_meeting("対象").unwrap().id;
    let other = storage.create_meeting("他会議").unwrap().id;
    let c = Arc::new(AnalysisController::default());
    let writer = c.begin_scoped_notice_storage(Some(&meeting)).unwrap();
    let worker = c.clone();
    let db = storage.clone();
    let target = meeting.clone();
    let join = std::thread::spawn(move || {
        worker.begin_meeting_deletion_wait(&db, &target, &|| false, Duration::from_secs(5))
    });
    let started = Instant::now();
    while !c.notice_storage_paused(Some(&meeting)) {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert!(c
        .try_begin_scoped_notice_storage(Some(&meeting))
        .unwrap()
        .is_none());
    let unrelated = c.begin_scoped_notice_storage(Some(&other)).unwrap();
    writer.validate_active().unwrap();
    assert!(!join.is_finished());
    drop(writer);
    let deletion = join.join().unwrap().unwrap();
    unrelated.validate_active().unwrap();
    assert!(c
        .try_begin_scoped_notice_storage(Some(&meeting))
        .unwrap()
        .is_none());
    storage.release_meeting_deletion(&meeting).unwrap();
    drop(deletion);
    c.begin_scoped_notice_storage(Some(&meeting)).unwrap();
}

#[test]
fn history_timeout_cancellation_and_failed_reservation_release_admission() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let meeting = storage.create_meeting("対象").unwrap().id;
    let c = AnalysisController::default();
    let writer = c.begin_scoped_notice_storage(Some(&meeting)).unwrap();
    let result =
        c.begin_meeting_deletion_wait(&storage, &meeting, &|| false, Duration::from_millis(20));
    assert!(result
        .err()
        .unwrap()
        .contains("通知履歴の処理が終わらないため"));
    assert!(!c.notice_storage_paused(Some(&meeting)));
    assert!(c
        .begin_meeting_deletion_wait(&storage, &meeting, &|| true, Duration::from_secs(5))
        .is_err());
    assert!(!c.notice_storage_paused(Some(&meeting)));
    writer.validate_active().unwrap();
    drop(writer);
    let work = storage.begin_meeting_work(&meeting, "job", None).unwrap();
    assert!(c.begin_meeting_deletion(&storage, &meeting).is_err());
    assert!(!c.notice_storage_paused(Some(&meeting)));
    drop(work);
    let deletion = c.begin_meeting_deletion(&storage, &meeting).unwrap();
    storage.release_meeting_deletion(&meeting).unwrap();
    drop(deletion);
}

#[test]
fn shared_deletion_waits_for_actual_io_and_releases_on_shutdown() {
    let c = Arc::new(AnalysisController::default());
    let writer = c.begin_notice_storage().unwrap();
    let worker = c.clone();
    let join = std::thread::spawn(move || worker.begin_deletion_wait(false, false, &|| false));
    let started = Instant::now();
    while !c.notice_storage_paused(None) {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert!(c.try_begin_scoped_notice_storage(None).unwrap().is_none());
    writer.validate_active().unwrap();
    assert!(!join.is_finished());
    drop(writer);
    let deletion = join.join().unwrap().unwrap();
    drop(deletion);
    c.begin_notice_storage().unwrap();
    let writer = c.begin_notice_storage().unwrap();
    let worker = c.clone();
    let join = std::thread::spawn(move || worker.begin_deletion_wait(false, false, &|| false));
    while !c.notice_storage_paused(None) {
        std::thread::yield_now();
    }
    c.shutdown.store(true, Ordering::SeqCst);
    assert!(join.join().unwrap().is_err());
    assert!(c.state.lock().unwrap().data_maintenance.is_none());
    drop(writer);
}

#[test]
fn notice_storage_coexists_with_recording_and_inference_but_blocks_data_cutover() {
    let controller = AnalysisController::default();
    let first = controller.begin_notice_storage().unwrap();
    let second = controller.begin_notice_storage().unwrap();
    assert_eq!(controller.state.lock().unwrap().notice_writers.len(), 2);
    assert!(controller.deletion_operation_blocker().unwrap().is_none());
    controller.state.lock().unwrap().busy = true;
    assert_eq!(
        controller.deletion_operation_blocker().unwrap().as_deref(),
        Some("AI処理中")
    );
    controller.state.lock().unwrap().busy = false;
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_err());
    {
        let mut state = controller.state.lock().unwrap();
        state.recording_intent = true;
        state.busy = true;
    }
    assert!(first.validate_active().is_ok());
    assert!(second.validate_active().is_ok());
    controller.state.lock().unwrap().busy = false;
    assert!(controller.wait_idle(Duration::from_millis(10)).is_ok());
    // Recording stop clears its intent without waiting for the companion fsync.
    // Backup/root cutover must still wait for the actual notification owner.
    controller.state.lock().unwrap().recording_intent = false;
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_err());
    assert!(controller.data_operation_blocker().unwrap().is_none());
    drop(first);
    assert_eq!(controller.state.lock().unwrap().notice_writers.len(), 1);
    assert!(second.validate_active().is_ok());
    controller.require_storage_restart();
    assert!(second.validate_active().is_err());
    assert!(controller.begin_notice_storage().is_err());
    drop(second);
    assert!(controller.state.lock().unwrap().notice_writers.is_empty());
}

#[test]
fn backup_admission_stays_available_during_notice_polling_and_drains_actual_io() {
    let controller = Arc::new(AnalysisController::default());
    let writer = controller.begin_notice_storage().unwrap();
    for _ in 0..20 {
        assert!(controller.data_operation_blocker().unwrap().is_none());
    }
    let worker = controller.clone();
    let pending =
        std::thread::spawn(move || worker.begin_data_maintenance_wait(false, false, &|| false));
    let started = Instant::now();
    while !controller.notice_storage_paused(None) {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::yield_now();
    }
    assert!(controller
        .try_begin_scoped_notice_storage(None)
        .unwrap()
        .is_none());
    assert!(!pending.is_finished());
    writer.validate_active().unwrap();
    drop(writer);
    let backup = pending.join().unwrap().unwrap();
    assert!(controller.begin_notice_storage().is_err());
    backup.validate_active().unwrap();
    drop(backup);
    assert!(controller.data_operation_blocker().unwrap().is_none());
    controller.begin_notice_storage().unwrap();
}

#[test]
fn notice_shutdown_waits_for_actual_worker_and_old_lease_never_revives_on_abort() {
    let controller = AnalysisController::default();
    let old = controller.begin_notice_storage().unwrap();
    controller.seal_exit().unwrap();
    assert!(old.validate_active().is_err());
    assert!(controller.begin_notice_storage().is_err());
    assert!(controller
        .wait_idle_inner(
            Duration::from_millis(10),
            &crate::test_support::never_cancelled,
            true
        )
        .is_err());
    controller.abort_shutdown();
    assert!(old.validate_active().is_err());
    let fresh = controller.begin_notice_storage().unwrap();
    assert!(fresh.validate_active().is_ok());
    drop(old);
    assert!(fresh.validate_active().is_ok());
    drop(fresh);
    controller
        .wait_idle_inner(
            Duration::from_millis(10),
            &crate::test_support::never_cancelled,
            true,
        )
        .unwrap();
    let maintenance = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(controller.begin_notice_storage().is_err());
    drop(maintenance);
    let cancelled = controller.begin_notice_storage().unwrap();
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(cancelled.validate_active().is_err());
}

#[test]
fn notice_storage_capacity_is_bounded_and_unwind_releases_just_its_lease() {
    let controller = AnalysisController::default();
    let mut leases = Vec::new();
    for _ in 0..128 {
        leases.push(controller.begin_notice_storage().unwrap());
    }
    assert!(controller.begin_notice_storage().is_err());
    leases.pop();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = controller.begin_notice_storage().unwrap();
        panic!("injected companion worker unwind");
    }))
    .unwrap_err();
    assert_eq!(controller.state.lock().unwrap().notice_writers.len(), 127);
    assert!(leases[0].validate_active().is_ok());
    drop(leases);
    controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
}

#[test]
fn checked_close_seals_admission_without_cancelling_unverified_work() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let model = Arc::new(AtomicBool::new(false));
    controller.state.lock().unwrap().model_preparation = Some(model.clone());
    assert_eq!(controller.close_activity().unwrap(), (false, true));
    controller.seal_exit().unwrap();
    assert!(controller.shutdown.load(Ordering::SeqCst));
    assert!(!model.load(Ordering::SeqCst));
    controller.abort_shutdown();
    assert!(lease.validate_active().is_ok());
    assert!(!model.load(Ordering::SeqCst));
}

#[test]
fn deletion_lease_proof_requires_current_ownership_and_stays_cancelled_after_aborted_exit() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(lease.validate_active().is_ok());
    controller.begin_exit().unwrap();
    assert!(lease.validate_active().is_err());
    controller.abort_shutdown();
    assert!(lease.validate_active().is_err());
    assert!(controller.begin_export().is_err());
    drop(lease);
    let next = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(next.validate_active().is_ok());
}

#[test]
fn deletion_lease_proof_rejects_lost_ownership_without_clearing_another_owner() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let other = Arc::new(AtomicBool::new(false));
    controller.state.lock().unwrap().data_maintenance = Some(other.clone());
    assert!(lease.validate_active().is_err());
    drop(lease);
    assert!(Arc::ptr_eq(
        controller
            .state
            .lock()
            .unwrap()
            .data_maintenance
            .as_ref()
            .unwrap(),
        &other
    ));
}

#[test]
fn storage_restart_fence_survives_temporary_lease_release_and_aborted_exit() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .is_err());
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(controller.state.lock().unwrap().blocks_worker());
    assert!(controller.begin_export().is_err());
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .is_err());
    controller.begin_exit().unwrap();
    assert!(lease.cancelled());
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_err());
    controller.abort_shutdown();
    assert!(lease.cancelled());
    assert!(controller.begin_export().is_err());
    drop(lease);
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.begin_export().is_ok());
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
                .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
                .unwrap()
        });
        let data = (gate == 9).then(|| {
            controller
                .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
                    let lease = controller.begin_data_maintenance(
                        false,
                        false,
                        &crate::test_support::never_cancelled,
                    );
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
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_err());
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .is_err());
    controller.abort_shutdown();
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .is_ok());
}

#[test]
fn data_maintenance_unwind_releases_only_its_lease_and_retains_wake() {
    let controller = AnalysisController::default();
    controller.state.lock().unwrap().wake = true;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = controller
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        panic!("synthetic data work panic");
    }))
    .is_err());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.state.lock().unwrap().wake);
    assert!(controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
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
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(controller.state.lock().unwrap().blocks_worker());
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .is_err());
    controller.state.lock().unwrap().cancel_model_preparation();
    assert!(lease.cancelled());
    assert!(controller.wait_idle(Duration::from_millis(10)).is_err());
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .is_err());
    drop(lease);
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(!controller.state.lock().unwrap().blocks_worker());
    assert!(!controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
        .unwrap()
        .cancelled());
}

#[test]
fn model_preparation_rejects_every_admission_gate_without_changing_worker_state() {
    for gate in 1..8 {
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
                .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
fn model_steps_reserve_the_lane_and_wait_for_a_worker_woken_between_steps() {
    let controller = Arc::new(AnalysisController::default());
    // Download, placement, compatibility, preview and apply each release the
    // lane. Reproduce a queued worker winning admission between every pair.
    for _ in 0..5 {
        controller.state.lock().unwrap().busy = true;
        let worker = controller.clone();
        let (waiting, started) = std::sync::mpsc::channel();
        let (done, finished) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let lease = worker
                .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
                .unwrap();
            lease
                .wait_for_worker(&crate::test_support::never_cancelled, |value| {
                    waiting.send(value).unwrap();
                })
                .unwrap();
            done.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        assert!(started.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(
            finished.try_recv().is_err(),
            "model work must not overlap inference"
        );
        assert!(controller.state.lock().unwrap().blocks_worker());
        assert!(controller
            .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
            .is_err());
        assert!(controller
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .is_err());
        controller.state.lock().unwrap().busy = false;
        controller.idle.notify_all();
        assert!(!started.recv_timeout(Duration::from_secs(2)).unwrap());
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(controller.state.lock().unwrap().blocks_worker());
        release.send(()).unwrap();
        handle.join().unwrap();
        controller.wait_idle(Duration::from_millis(10)).unwrap();
    }
}

#[test]
fn cancelling_a_waiting_model_preparation_releases_only_its_reservation() {
    // IPC cancellation, recording admission and shutdown can all cancel the
    // wait, even if the current inference worker has not returned yet.
    for reason in 0..3 {
        let controller = Arc::new(AnalysisController::default());
        controller.state.lock().unwrap().busy = true;
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = controller.clone();
        let stop = cancelled.clone();
        let (waiting, started) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let lease = worker
                .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
                .unwrap();
            assert!(lease
                .wait_for_worker(&|| stop.load(Ordering::SeqCst), |value| {
                    assert!(value);
                    waiting.send(()).unwrap();
                })
                .is_err());
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        match reason {
            0 => cancelled.store(true, Ordering::SeqCst),
            1 => {
                let mut state = controller.state.lock().unwrap();
                state.recording_intent = true;
                state.cancel_model_preparation();
            }
            _ => controller.shutdown.store(true, Ordering::SeqCst),
        }
        handle.join().unwrap();
        let state = controller.state.lock().unwrap();
        assert!(state.busy);
        assert!(state.model_preparation.is_none());
        assert_eq!(state.recording_intent, reason == 1);
        assert_eq!(controller.shutdown.load(Ordering::SeqCst), reason == 2);
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
                let lease = controller.begin_model_preparation(
                    false,
                    false,
                    &crate::test_support::never_cancelled,
                );
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
            .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        panic!("synthetic model preparation failure");
    }));
    assert!(result.is_err());
    controller.wait_idle(Duration::from_millis(10)).unwrap();
    assert!(controller.state.lock().unwrap().wake);
    assert!(controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
                .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
            .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
            .is_err());
        assert_eq!(controller.state.lock().unwrap().recording_intent, !closing);
        assert_eq!(controller.shutdown.load(Ordering::SeqCst), closing);
    }
}

#[test]
fn cancelled_wait_does_not_release_or_clear_model_preparation() {
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
        backend: None,
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
    let report =
        crate::model_inspection::inspect(source.path(), &crate::test_support::never_cancelled)
            .unwrap();
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
            .begin_model_preparation(false, false, &crate::test_support::never_cancelled)
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
