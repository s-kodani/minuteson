use super::*;

#[test]
fn cold_deletion_native_resume_uses_actual_restart_fenced_admission_and_keeps_ordinary_work_blocked(
) {
    for keep in [false, true] {
        let f = Fixture::new();
        let ready = f.ready();
        let (work, _, started) = f
            .controller
            .begin_request(f.request(&ready, Action::Publish))
            .unwrap()
            .unwrap();
        let stop = AtomicBool::new(false);
        let partial = f.finish(work, &started, &|| stop.load(Ordering::Acquire), &|phase| {
            if phase
                == if keep {
                    "delete_scope_fixed"
                } else {
                    "deleting_database"
                }
            {
                stop.store(true, Ordering::Release);
            }
        });
        let operation = partial.deletion_preview.unwrap().operation_id;
        drop(f.storage);
        let storage = Storage::open(f._root.path().into()).unwrap();
        assert!(storage.data_write_blocked());
        let analysis = AnalysisController::default();
        assert!(analysis
            .begin_deletion_maintenance(&storage, "wrong", false, false, &|| false)
            .is_err());
        assert!(analysis
            .begin_deletion_maintenance(&storage, &operation, true, false, &|| false)
            .is_err());
        assert!(analysis
            .begin_deletion_maintenance(&storage, &operation, false, true, &|| false)
            .is_err());
        let controller = DataController::default();
        let (started, _, previous) = controller.begin(Kind::Deletion).unwrap();
        let lease = analysis
            .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
            .unwrap();
        let reviewed = Work::Deletion(operation.clone(), true, previous).run_admitted(
            &storage,
            &|| false,
            &|_| {},
            Some(&lease),
        );
        let mut snapshot = controller
            .finish(started.token.as_deref().unwrap(), reviewed)
            .unwrap();
        assert!(!snapshot.restart_required && snapshot.retry_allowed);
        drop(lease);
        for action in [
            Action::Refresh,
            if keep { Action::Keep } else { Action::Publish },
        ] {
            let request = Request {
                token: snapshot.token.clone().unwrap(),
                confirmation_id: snapshot
                    .deletion_preview
                    .as_ref()
                    .unwrap()
                    .confirmation_id
                    .clone(),
                action,
            };
            let (work, _, started) = controller.begin_request(request.clone()).unwrap().unwrap();
            let lease = analysis
                .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
                .unwrap();
            let done = work.run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
            snapshot = controller
                .finish(started.token.as_deref().unwrap(), done)
                .unwrap();
            if storage.restart_required() {
                analysis.require_storage_restart();
            }
            assert!(analysis.begin_export().is_err());
            drop(lease);
            if storage.restart_required() {
                assert!(analysis
                    .begin_data_maintenance(false, false, &|| false)
                    .is_err());
            }
            assert!(storage.require_data_writable().is_err());
            assert!(controller.begin_request(request).unwrap().is_none());
        }
        assert_eq!(snapshot.phase, "completed");
        let receipt = snapshot.deletion_receipt.unwrap();
        assert!(receipt.restart_required);
        assert_eq!(receipt.retained, keep);
        assert_eq!(receipt.deleted, !keep);
        assert_eq!(storage.get_meeting(&f.id).unwrap().is_some(), keep);
        assert!(storage.get_meeting(&f.other).unwrap().is_some());
        if !keep {
            // The final reply can be lost while Storage remains open. Rebuild
            // the server-owned review and replay acknowledgement, not row I/O.
            let next = DataController::default();
            let (started, _, previous) = next.begin(Kind::Deletion).unwrap();
            let lease = analysis
                .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
                .unwrap();
            let reviewed = Work::Deletion(operation.clone(), true, previous).run_admitted(
                &storage,
                &|| false,
                &|_| {},
                Some(&lease),
            );
            let reviewed = next
                .finish(started.token.as_deref().unwrap(), reviewed)
                .unwrap();
            assert!(reviewed.restart_required && reviewed.retry_allowed);
            assert_eq!(
                reviewed.deletion_preview.as_ref().unwrap().phase,
                "awaiting_ack"
            );
            drop(lease);
            let req = Request {
                token: reviewed.token.unwrap(),
                confirmation_id: reviewed.deletion_preview.unwrap().confirmation_id,
                action: Action::Publish,
            };
            let (work, _, started) = next.begin_request(req).unwrap().unwrap();
            let lease = analysis
                .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
                .unwrap();
            let done = work.run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
            assert!(
                next.finish(started.token.as_deref().unwrap(), done)
                    .unwrap()
                    .deletion_receipt
                    .unwrap()
                    .deleted
            );
            drop(lease);
            assert!(analysis
                .begin_data_maintenance(false, false, &|| false)
                .is_err());
        }
        drop(storage);
        let clean = Storage::open(f._root.path().into()).unwrap();
        assert!(!clean.data_write_blocked());
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    id: String,
    other: String,
    audio: PathBuf,
    controller: DataController,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let id = storage.create_meeting("削除確認").unwrap().id;
        let other = storage.create_meeting("残す会議").unwrap().id;
        let audio = storage
            .root()
            .join(format!("meetings/{id}/audio/input.flac"));
        std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
        std::fs::write(&audio, b"TARGET").unwrap();
        storage
            .register_ready_meeting_asset(
                &id,
                audio.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        Self {
            _root: root,
            storage,
            id,
            other,
            audio,
            controller: DataController::default(),
        }
    }
    fn review(&self, id: String, pending: bool) -> Snapshot {
        let (snapshot, _, previous) = self.controller.begin(Kind::Deletion).unwrap();
        let done = Work::Deletion(id, pending, previous).run(&self.storage, &|| false, &|_| {});
        self.controller
            .finish(snapshot.token.as_deref().unwrap(), done)
            .unwrap()
    }
    fn ready(&self) -> Snapshot {
        self.review(self.id.clone(), false)
    }
    fn finish(
        &self,
        work: Work,
        started: &Snapshot,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Snapshot {
        let analysis = AnalysisController::default();
        let lease = analysis
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let done = work.run_admitted(&self.storage, check, progress, Some(&lease));
        assert!(analysis.begin_export().is_err());
        let snapshot = self
            .controller
            .finish(started.token.as_deref().unwrap(), done)
            .unwrap();
        assert!(analysis.begin_export().is_err());
        drop(lease);
        assert!(analysis.begin_export().is_ok());
        snapshot
    }
    fn request(&self, snapshot: &Snapshot, action: Action) -> Request {
        Request {
            token: snapshot.token.clone().unwrap(),
            confirmation_id: snapshot
                .deletion_preview
                .as_ref()
                .unwrap()
                .confirmation_id
                .clone(),
            action,
        }
    }
    fn run(&self, snapshot: &Snapshot, action: Action) -> Snapshot {
        let (work, _, started) = self
            .controller
            .begin_request(self.request(snapshot, action))
            .unwrap()
            .unwrap();
        self.finish(work, &started, &|| false, &|_| {})
    }
}
#[test]
fn deletion_controller_uses_owned_confirmation_and_replays_success_after_late_cancel() {
    let f = Fixture::new();
    let ready = f.ready();
    assert_eq!(ready.phase, "ready");
    assert_eq!(ready.kind, Some(Kind::Deletion));
    let request = f.request(&ready, Action::Publish);
    let mut bad = request.clone();
    bad.confirmation_id = "wrong".into();
    assert!(f.controller.begin_request(bad).is_err());
    assert!(f.controller.deletion_page("wrong", 0).is_err());
    assert_eq!(
        f.controller
            .deletion_page(&request.confirmation_id, 0)
            .unwrap()
            .entries
            .len(),
        3
    );
    let (work, cancel, started) = f
        .controller
        .begin_request(request.clone())
        .unwrap()
        .unwrap();
    assert!(f
        .controller
        .begin_request(request.clone())
        .unwrap()
        .is_none());
    let analysis = AnalysisController::default();
    let lease = analysis
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let completion = work.run_admitted(&f.storage, &|| false, &|_| {}, Some(&lease));
    cancel.store(true, Ordering::Release);
    let done = f
        .controller
        .finish(started.token.as_deref().unwrap(), completion)
        .unwrap();
    assert_eq!(done.phase, "completed");
    assert!(done.deletion_receipt.as_ref().unwrap().deleted);
    assert!(!done.busy && !done.retry_allowed);
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert!(!f.audio.exists());
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
    drop(lease);
    assert!(f
        .controller
        .begin_request(f.request(&done, Action::Keep))
        .is_err());
}
#[test]
fn deletion_controller_rejects_execution_without_actual_shared_lease_and_requires_reconfirmation() {
    let f = Fixture::new();
    let ready = f.ready();
    let (work, _, started) = f
        .controller
        .begin_request(f.request(&ready, Action::Publish))
        .unwrap()
        .unwrap();
    let completion = work.run(&f.storage, &|| false, &|_| {});
    assert!(completion.error.is_some());
    assert!(completion.deletion.is_none());
    let failed = f
        .controller
        .finish(started.token.as_deref().unwrap(), completion)
        .unwrap();
    assert_eq!(failed.phase, "failed");
    assert!(f.audio.exists());
    assert!(deletion::listing(&f.storage).unwrap().items.is_empty());
    assert!(f
        .controller
        .begin_request(f.request(&failed, Action::Publish))
        .is_err());
    let refreshed = f.run(&failed, Action::Refresh);
    let done = f.run(&refreshed, Action::Publish);
    assert!(done.deletion_receipt.unwrap().deleted);
}
#[test]
fn deletion_controller_unregistered_decision_is_separate_and_cannot_apply_to_backup() {
    let f = Fixture::new();
    let unknown = f.audio.parent().unwrap().join("unregistered");
    std::fs::write(&unknown, b"KEEP UNTIL CONFIRMED").unwrap();
    let ready = f.ready();
    assert_eq!(
        ready.deletion_preview.as_ref().unwrap().unregistered_count,
        1
    );
    let failed = f.run(&ready, Action::Publish);
    assert_eq!(failed.phase, "failed");
    assert!(unknown.exists());
    assert!(deletion::listing(&f.storage).unwrap().items.is_empty());
    let refreshed = f.run(&failed, Action::Refresh);
    assert_ne!(
        ready.deletion_preview.unwrap().confirmation_id,
        refreshed.deletion_preview.as_ref().unwrap().confirmation_id
    );
    let done = f.run(&refreshed, Action::DeleteIncludingUnregistered);
    assert!(done.deletion_receipt.unwrap().deleted);
    assert!(!unknown.exists());
    let out = tempfile::tempdir().unwrap();
    let controller = DataController::default();
    let (s, _, previous) = controller.begin(Kind::Backup).unwrap();
    let completion = Work::Select(
        Kind::Backup,
        Ok(Some(out.path().join("test.monbackup"))),
        previous,
    )
    .run(&f.storage, &|| false, &|_| {});
    let ready = controller
        .finish(s.token.as_deref().unwrap(), completion)
        .unwrap();
    let bad = Request {
        token: ready.token.unwrap(),
        confirmation_id: ready.backup_preview.unwrap().confirmation_id,
        action: Action::DeleteIncludingUnregistered,
    };
    assert!(controller.begin_request(bad).is_err());
}
#[test]
fn deletion_controller_cancelled_review_requires_refresh_before_any_destructive_request() {
    let f = Fixture::new();
    let (s, cancel, previous) = f.controller.begin(Kind::Deletion).unwrap();
    let completion =
        Work::Deletion(f.id.clone(), false, previous).run(&f.storage, &|| false, &|_| {});
    cancel.store(true, Ordering::Release);
    let cancelled = f
        .controller
        .finish(s.token.as_deref().unwrap(), completion)
        .unwrap();
    assert_eq!(cancelled.phase, "cancelled");
    assert!(f
        .controller
        .begin_request(f.request(&cancelled, Action::Publish))
        .is_err());
    let ready = f.run(&cancelled, Action::Refresh);
    assert_eq!(ready.phase, "ready");
    assert!(f.audio.exists());
}
#[test]
fn deletion_controller_partial_cancel_replays_attention_and_only_refresh_enables_resume() {
    let f = Fixture::new();
    let ready = f.ready();
    let req = f.request(&ready, Action::Publish);
    let (work, cancel, started) = f.controller.begin_request(req.clone()).unwrap().unwrap();
    let partial = f.finish(
        work,
        &started,
        &|| cancel.load(Ordering::Acquire),
        &|phase| {
            if phase == "deleting_database" {
                cancel.store(true, Ordering::Release)
            }
        },
    );
    assert_eq!(partial.phase, "attention");
    assert!(partial.attention_required);
    assert!(partial.deletion_receipt.is_none());
    assert!(!f.audio.exists());
    assert!(f.controller.begin_request(req).unwrap().is_none());
    assert!(f
        .controller
        .begin_request(f.request(&partial, Action::Keep))
        .is_err());
    let ready = f.run(&partial, Action::Refresh);
    assert_eq!(ready.phase, "ready");
    let done = f.run(&ready, Action::Publish);
    assert!(done.deletion_receipt.unwrap().deleted);
}
#[test]
fn deletion_controller_unstarted_keep_resolves_attention_without_deleting_or_claiming_restore() {
    let f = Fixture::new();
    let ready = f.ready();
    let (work, cancel, started) = f
        .controller
        .begin_request(f.request(&ready, Action::Publish))
        .unwrap()
        .unwrap();
    let partial = f.finish(
        work,
        &started,
        &|| cancel.load(Ordering::Acquire),
        &|phase| {
            if phase == "delete_scope_fixed" {
                cancel.store(true, Ordering::Release)
            }
        },
    );
    assert_eq!(partial.phase, "attention");
    assert_eq!(
        partial.deletion_preview.as_ref().unwrap().phase,
        "not_started"
    );
    assert!(partial.deletion_preview.as_ref().unwrap().keep_allowed);
    std::fs::write(&f.audio, b"KEEP CHANGED").unwrap();
    let done = f.run(&partial, Action::Keep);
    assert_eq!(done.phase, "completed");
    let receipt = done.deletion_receipt.unwrap();
    assert!(receipt.retained && !receipt.deleted);
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"KEEP CHANGED");
    assert!(f.storage.get_meeting(&f.id).unwrap().is_some());
    assert!(!f.storage.data_write_blocked());
}
#[test]
fn deletion_controller_panic_after_file_phase_retains_progress_and_never_fabricates_success() {
    let f = Fixture::new();
    let ready = f.ready();
    let (work, _, started) = f
        .controller
        .begin_request(f.request(&ready, Action::Publish))
        .unwrap()
        .unwrap();
    let partial = f.finish(work, &started, &|| false, &|phase| {
        if phase == "deleting_database" {
            panic!("simulated worker panic after file handoff")
        }
    });
    assert_eq!(partial.phase, "attention");
    assert!(partial.deletion_receipt.is_none());
    assert!(!f.audio.exists());
    assert!(f.storage.get_meeting(&f.id).unwrap().is_some());
    let op = partial.deletion_preview.unwrap().operation_id;
    let ready = f.review(op, true);
    assert_eq!(ready.phase, "ready");
    assert_eq!(ready.deletion_preview.as_ref().unwrap().phase, "database");
    let done = f.run(&ready, Action::Publish);
    assert!(done.deletion_receipt.unwrap().deleted);
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
}
