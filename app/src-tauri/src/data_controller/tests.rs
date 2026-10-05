use super::*;

fn finish_work(
    controller: &DataController,
    snapshot: &Snapshot,
    storage: &Storage,
    work: Work,
) -> Snapshot {
    let token = snapshot.token.as_deref().unwrap();
    let result = work.run(storage, &|| false, &|phase| {
        controller.progress(token, phase);
    });
    controller.finish(token, result).unwrap()
}

fn select(controller: &DataController, kind: Kind, path: PathBuf, storage: &Storage) -> Snapshot {
    let (snapshot, _, previous) = controller.begin(kind).unwrap();
    finish_work(
        controller,
        &snapshot,
        storage,
        Work::Select(kind, Ok(Some(path)), previous),
    )
}

fn request(snapshot: &Snapshot, action: Action) -> Request {
    Request {
        token: snapshot.token.clone().unwrap(),
        confirmation_id: snapshot
            .backup_preview
            .as_ref()
            .map(|p| p.confirmation_id.clone())
            .or_else(|| {
                snapshot
                    .restore_preview
                    .as_ref()
                    .map(|p| p.confirmation_id.clone())
            })
            .unwrap(),
        action,
    }
}

#[test]
fn backup_controller_replays_the_accepted_request_without_reopening_changed_output() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_path_buf()).unwrap();
    storage.create_meeting("confirmed").unwrap();
    let path = output.path().join("backup.monbackup");
    let controller = DataController::default();
    let selected = select(&controller, Kind::Backup, path.clone(), &storage);
    assert_eq!(selected.phase, "ready");
    let publish = request(&selected, Action::Publish);
    let mut wrong = publish.clone();
    wrong.confirmation_id = Uuid::new_v4().to_string();
    assert!(controller.begin_request(wrong).is_err());
    let (work, _, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    assert!(controller.begin_request(publish.clone()).unwrap().is_none());
    let done = finish_work(&controller, &started, &storage, work);
    assert_eq!(done.phase, "completed");
    assert!(done.backup_receipt.is_some());
    std::fs::write(&path, b"changed after publication").unwrap();
    assert!(controller.begin_request(publish.clone()).unwrap().is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"changed after publication");
    let old_token = selected.token.unwrap();
    let (new, _, previous) = controller.begin(Kind::Backup).unwrap();
    drop(previous);
    assert_ne!(new.token.as_deref(), Some(old_token.as_str()));
    assert!(controller.begin_request(publish).is_err());
}

#[test]
fn restore_controller_refresh_response_loss_and_publication_do_not_duplicate_meetings() {
    let source_root = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = Storage::open(source_root.path().to_path_buf()).unwrap();
    source.create_meeting("restored").unwrap();
    let path = output.path().join("backup.monbackup");
    source.create_backup_archive(&path, &|| false).unwrap();
    let target = Storage::open(target_root.path().to_path_buf()).unwrap();
    let existing = target.create_meeting("existing").unwrap();
    assert!(target.restored_meeting_ids().unwrap().is_empty());
    let controller = DataController::default();
    let selected = select(&controller, Kind::Restore, path, &target);
    assert_eq!(selected.phase, "ready", "{:?}", selected.error_message);
    assert_eq!(target.list_meetings().unwrap().len(), 1);
    let refresh = request(&selected, Action::Refresh);
    let (work, _, started) = controller.begin_request(refresh.clone()).unwrap().unwrap();
    let refreshed = finish_work(&controller, &started, &target, work);
    assert_ne!(
        refreshed.restore_preview.as_ref().unwrap().confirmation_id,
        refresh.confirmation_id
    );
    assert_eq!(
        refreshed.restore_preview.as_ref().unwrap().batch_id,
        selected.restore_preview.as_ref().unwrap().batch_id
    );
    assert!(controller.begin_request(refresh).unwrap().is_none());
    let publish = request(&refreshed, Action::Publish);
    let (work, _, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    let done = finish_work(&controller, &started, &target, work);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert_eq!(done.restore_receipt.as_ref().unwrap().meeting_ids.len(), 1);
    assert!(controller.begin_request(publish).unwrap().is_none());
    assert_eq!(target.list_meetings().unwrap().len(), 2);
    assert_eq!(
        target.restored_meeting_ids().unwrap(),
        done.restore_receipt.unwrap().meeting_ids
    );
    assert_eq!(
        target.get_meeting(&existing.id).unwrap().unwrap().title,
        "existing"
    );
}

#[test]
fn controller_waits_for_actual_work_after_cancellation_and_ignores_stale_progress() {
    let controller = Arc::new(DataController::default());
    let (snapshot, cancel, _) = controller.begin(Kind::Restore).unwrap();
    let token = snapshot.token.unwrap();
    assert!(controller.begin(Kind::Backup).is_err());
    assert!(controller.progress("other token", "reading").is_none());
    assert!(controller.stop(Duration::from_millis(10)).is_err());
    assert!(cancel.load(Ordering::Acquire));
    assert!(controller.snapshot().unwrap().busy);
    assert!(controller.progress(&token, "reading").is_none());
    let worker = controller.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        worker
            .finish(
                &token,
                Completion {
                    cancelled: true,
                    ..Completion::default()
                },
            )
            .unwrap();
    });
    assert!(controller.stop(Duration::from_secs(1)).unwrap().is_none());
    thread.join().unwrap();
    assert!(!controller.snapshot().unwrap().busy);
    assert_eq!(controller.snapshot().unwrap().phase, "cancelled");
    assert!(controller.begin(Kind::Backup).is_err());
}

#[test]
fn terminal_completion_wins_over_late_cancel_and_uncertain_backup_never_retries() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_path_buf()).unwrap();
    let controller = DataController::default();
    let selected = select(
        &controller,
        Kind::Backup,
        output.path().join("backup.monbackup"),
        &storage,
    );
    let publish = request(&selected, Action::Publish);
    let (work, cancel, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    let mut completion = work.run(&storage, &|| false, &|_| {});
    cancel.store(true, Ordering::Release);
    completion.cancelled = true;
    let done = controller
        .finish(started.token.as_deref().unwrap(), completion)
        .unwrap();
    assert_eq!(done.phase, "completed");
    assert!(done.backup_receipt.is_some());
    let selected = select(
        &controller,
        Kind::Backup,
        output.path().join("uncertain.monbackup"),
        &storage,
    );
    let publish = request(&selected, Action::Publish);
    let (work, _, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    let Work::Request(selection, _) = work else {
        panic!("request")
    };
    // Synthetic post-publication uncertainty, not a real fsync failure.
    let attention = controller
        .finish(
            started.token.as_deref().unwrap(),
            Completion {
                selection: Some(selection),
                attention: true,
                error: Some("uncertain".into()),
                ..Completion::default()
            },
        )
        .unwrap();
    assert_eq!(attention.phase, "attention");
    assert!(!attention.retry_allowed);
    assert!(controller.begin_request(publish).unwrap().is_none());
    assert!(!output.path().join("uncertain.monbackup").exists());
}

#[test]
fn failed_validation_and_cancellation_release_busy_without_publishing_or_changing_live_data() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_path_buf()).unwrap();
    storage.create_meeting("existing").unwrap();
    let path = output.path().join("bad.monbackup");
    std::fs::write(&path, b"invalid").unwrap();
    let controller = DataController::default();
    let failed = select(&controller, Kind::Restore, path.clone(), &storage);
    assert_eq!(failed.phase, "failed");
    assert!(!failed.busy);
    assert!(failed.restore_preview.is_none());
    assert!(!failed.retry_allowed);
    let (snapshot, cancel, previous) = controller.begin(Kind::Restore).unwrap();
    cancel.store(true, Ordering::Release);
    let completion = Work::Select(Kind::Restore, Ok(Some(path)), previous).run(
        &storage,
        &|| cancel.load(Ordering::Acquire),
        &|_| panic!("must not read cancelled input"),
    );
    let done = controller
        .finish(snapshot.token.as_deref().unwrap(), completion)
        .unwrap();
    assert_eq!(done.phase, "cancelled");
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}

#[test]
fn restore_panic_retains_same_batch_for_recovery_instead_of_selecting_another_archive() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = Storage::open(root.path().to_path_buf()).unwrap();
    source.create_meeting("restored").unwrap();
    let path = output.path().join("backup.monbackup");
    source.create_backup_archive(&path, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().to_path_buf()).unwrap();
    let controller = DataController::default();
    let selected = select(&controller, Kind::Restore, path, &target);
    let publish = request(&selected, Action::Publish);
    let (work, _, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    let failed = controller
        .finish(
            started.token.as_deref().unwrap(),
            work.run(&target, &|| false, &|_| {
                panic!("synthetic reconstruction panic")
            }),
        )
        .unwrap();
    assert_eq!(failed.phase, "failed");
    assert!(failed.retry_allowed);
    assert_eq!(
        failed.restore_preview.as_ref().unwrap().batch_id,
        selected.restore_preview.as_ref().unwrap().batch_id
    );
    assert!(target.list_meetings().unwrap().is_empty());
    let (work, _, started) = controller.begin_request(publish).unwrap().unwrap();
    assert_eq!(
        finish_work(&controller, &started, &target, work).phase,
        "completed"
    );
    assert_eq!(target.list_meetings().unwrap().len(), 1);
}

#[test]
fn controller_restore_recovers_a_committed_batch_even_without_an_in_memory_receipt() {
    let source_root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = Storage::open(source_root.path().to_path_buf()).unwrap();
    source.create_meeting("restored").unwrap();
    let path = output.path().join("backup.monbackup");
    source.create_backup_archive(&path, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().to_path_buf()).unwrap();
    let controller = DataController::default();
    let selected = select(&controller, Kind::Restore, path, &target);
    let publish = request(&selected, Action::Publish);
    let (work, _, started) = controller.begin_request(publish.clone()).unwrap().unwrap();
    let Work::Request(selection, _) = work else {
        panic!("request")
    };
    let Selection::Restore(prepared) = &selection else {
        panic!("restore")
    };
    let receipt = prepared
        .publish(&target, &publish.confirmation_id, &|| false, &|_| {})
        .unwrap();
    // Fault-inject the gap after durable commit but before controller receipt
    // delivery. This is not a real process-kill/crash recovery measurement.
    let failed = controller
        .finish(
            started.token.as_deref().unwrap(),
            Completion {
                selection: Some(selection),
                error: Some("lost receipt after commit".into()),
                ..Completion::default()
            },
        )
        .unwrap();
    assert!(failed.retry_allowed);
    let (work, _, started) = controller.begin_request(publish).unwrap().unwrap();
    let done = finish_work(&controller, &started, &target, work);
    let replay = done.restore_receipt.unwrap();
    assert!(replay.already_published);
    assert_eq!(replay.batch_id, receipt.batch_id);
    assert_eq!(replay.meeting_ids, receipt.meeting_ids);
    assert!(replay.vocabulary_added.is_none());
    assert_eq!(target.list_meetings().unwrap().len(), 1);
}
