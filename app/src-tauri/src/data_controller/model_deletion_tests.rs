//! Shared controller with real maintenance leases and synthetic, private fixtures.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::models::tests::Fixture as Model;

struct Fixture {
    model: Model,
    controller: DataController,
    analysis: AnalysisController,
}
impl Fixture {
    fn new() -> Self {
        Self {
            model: Model::new(),
            controller: DataController::default(),
            analysis: AnalysisController::default(),
        }
    }
    fn lease(&self, snapshot: Option<&Snapshot>) -> DataMaintenanceLease {
        let operation = snapshot
            .and_then(|s| s.model_deletion_preview.as_ref())
            .and_then(|p| p.pending.then_some(p.operation_id.as_str()));
        match operation {
            Some(id) => self
                .analysis
                .begin_model_deletion_maintenance(
                    &self.model.storage,
                    id,
                    false,
                    false,
                    &crate::test_support::never_cancelled,
                )
                .unwrap(),
            None => self
                .analysis
                .begin_model_deletion(false, false, &crate::test_support::never_cancelled)
                .unwrap(),
        }
    }
    fn ready(&self) -> Snapshot {
        let lease = self.lease(None);
        let (s, _, previous) = self.controller.begin(Kind::ModelDeletion).unwrap();
        self.finish(
            Work::ModelDeletion(
                ModelTarget::Fresh {
                    package: self.model.package.clone(),
                    review: self.model.review.clone(),
                },
                self.model.root.clone(),
                previous,
            ),
            &s,
            &lease,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
    }
    fn pending(&self, operation: &str) -> Snapshot {
        let lease = self
            .analysis
            .begin_model_deletion_maintenance(&self.model.storage, operation, false, false, &|| {
                false
            })
            .unwrap();
        let (s, _, previous) = self.controller.begin(Kind::ModelDeletion).unwrap();
        self.finish(
            Work::ModelDeletion(
                ModelTarget::Pending(operation.into()),
                self.model.root.clone(),
                previous,
            ),
            &s,
            &lease,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
    }
    fn finish(
        &self,
        work: Work,
        s: &Snapshot,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Snapshot {
        self.controller
            .finish(
                s.token.as_ref().unwrap(),
                work.run_admitted(&self.model.storage, check, progress, Some(lease)),
            )
            .unwrap()
    }
    fn request(&self, s: &Snapshot, action: Action) -> Request {
        Request {
            token: s.token.clone().unwrap(),
            confirmation_id: s
                .model_deletion_preview
                .as_ref()
                .unwrap()
                .confirmation_id
                .clone(),
            action,
        }
    }
    fn run(&self, s: &Snapshot, action: Action) -> Snapshot {
        self.stop_at(s, action, "NEVER")
    }
    fn stop_at(&self, s: &Snapshot, action: Action, phase: &str) -> Snapshot {
        let lease = self.lease(Some(s));
        let (work, _, started) = self
            .controller
            .begin_request(self.request(s, action))
            .unwrap()
            .unwrap();
        let cancelled = AtomicBool::new(false);
        self.finish(
            work,
            &started,
            &lease,
            &|| cancelled.load(Ordering::Acquire),
            &|p| {
                if p == phase {
                    cancelled.store(true, Ordering::Release);
                }
            },
        )
    }
}

#[test]
fn selected_model_body_controller_deletes_exact_copy_and_replays_receipt() {
    let f = Fixture::new();
    let other = f.model.root.join(Uuid::new_v4().to_string());
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("KEEP"), b"other copy").unwrap();
    let before = f.model.storage.list_meetings().unwrap();
    let ready = f.ready();
    assert_eq!(ready.phase, "ready");
    assert!(f.model.path("cli").exists());
    assert_eq!(f.model.operation_count(), 0);
    let request = f.request(&ready, Action::Publish);
    let done = f.run(&ready, Action::Publish);
    assert!(done.model_deletion_receipt.as_ref().unwrap().body_deleted);
    assert_eq!(done.phase, "completed");
    assert!(!f.model.path("").exists());
    assert_eq!(std::fs::read(other.join("KEEP")).unwrap(), b"other copy");
    assert_eq!(f.model.storage.list_meetings().unwrap().len(), before.len());
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert_eq!(f.model.operation_count(), 1);
}
#[test]
fn directory_model_controller_includes_changed_and_unregistered_contents() {
    let f = Fixture::new();
    let ready = f.ready();
    assert_eq!(
        ready.model_deletion_preview.as_ref().unwrap().scope_mode,
        "directory"
    );
    std::fs::write(f.model.path("extra"), b"unknown").unwrap();
    std::fs::write(f.model.path("weights/model.bin"), b"changed model").unwrap();
    let done = f.run(&ready, Action::Publish);
    assert!(done.model_deletion_receipt.unwrap().body_deleted);
    assert!(!f.model.path("").exists());
    assert_eq!(f.model.storage.list_meetings().unwrap().len(), 1);
}
#[test]
fn model_body_controller_stop_before_removal_can_keep_and_partial_stop_can_resume() {
    let f = Fixture::new();
    let ready = f.ready();
    let stopped = f.stop_at(&ready, Action::Publish, "model_delete_scope_fixed");
    assert!(stopped.attention_required);
    assert!(f.model.path("cli").exists());
    let kept = f.run(&stopped, Action::Keep);
    assert!(kept.model_deletion_receipt.unwrap().retained);
    assert!(f.model.path("cli").exists());
    assert!(!f.model.storage.data_write_blocked());
    let ready = f.ready();
    let stopped = f.stop_at(&ready, Action::Publish, "deleting_model_files");
    assert!(stopped.attention_required);
    assert!(
        !stopped
            .model_deletion_preview
            .as_ref()
            .unwrap()
            .keep_allowed
    );
    let operation = stopped
        .model_deletion_preview
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    let pending = f.pending(&operation);
    let done = f.run(&pending, Action::Publish);
    assert!(done.model_deletion_receipt.unwrap().body_deleted);
    assert!(!f.model.storage.data_write_blocked());
}
#[test]
fn model_body_controller_without_shared_lease_never_deletes() {
    let f = Fixture::new();
    let (s, _, previous) = f.controller.begin(Kind::ModelDeletion).unwrap();
    let result = Work::ModelDeletion(
        ModelTarget::Fresh {
            package: f.model.package.clone(),
            review: f.model.review.clone(),
        },
        f.model.root.clone(),
        previous,
    )
    .run_admitted(&f.model.storage, &|| false, &|_| {}, None);
    let failed = f
        .controller
        .finish(s.token.as_deref().unwrap(), result)
        .unwrap();
    assert_eq!(failed.phase, "failed");
    assert!(f.model.path("cli").exists());
    assert_eq!(f.model.operation_count(), 0);
    assert!(f
        .analysis
        .begin_model_deletion(true, false, &|| false)
        .is_err());
}

#[test]
fn model_review_progress_is_model_specific_and_cancellable_before_any_file_removal() {
    let f = Fixture::new();
    let lease = f.lease(None);
    let (started, _, previous) = f.controller.begin(Kind::ModelDeletion).unwrap();
    let token = started.token.as_deref().unwrap();
    let stopped = AtomicBool::new(false);
    let mut_phase = std::sync::Mutex::new(Vec::new());
    let completion = Work::ModelDeletion(
        ModelTarget::Fresh {
            package: f.model.package.clone(),
            review: f.model.review.clone(),
        },
        f.model.root.clone(),
        previous,
    )
    .run_admitted(
        &f.model.storage,
        &|| stopped.load(Ordering::Acquire),
        &|phase| {
            let event = f.controller.progress(token, phase).unwrap();
            assert_eq!(event.kind, Some(Kind::ModelDeletion));
            mut_phase.lock().unwrap().push(event.phase);
            stopped.store(true, Ordering::Release);
        },
        Some(&lease),
    );
    let terminal = f.controller.finish(token, completion).unwrap();
    assert_eq!(
        *mut_phase.lock().unwrap(),
        vec!["verifying_model_delete_scope"]
    );
    assert_eq!(terminal.phase, "cancelled");
    assert!(f.model.path("weights/model.bin").exists());
    assert_eq!(f.model.operation_count(), 0);
}

#[test]
fn model_deletion_native_snapshots_keep_and_retry_current_operation() {
    let f = Fixture::new();
    let ready = f.ready();
    let stopped = f.stop_at(&ready, Action::Publish, "model_delete_scope_fixed");
    assert!(
        f.run(&stopped, Action::Keep)
            .model_deletion_receipt
            .unwrap()
            .retained
    );
    let ready = f.ready();
    let stopped = f.stop_at(&ready, Action::Publish, "deleting_model_files");
    let operation = stopped
        .model_deletion_preview
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    let pending = f.pending(&operation);
    std::fs::write(f.model.path("extra"), b"changed scope").unwrap();
    let done = f.run(&pending, Action::Publish);
    assert_eq!(done.phase, "completed");
    let pending = f.pending(&operation);
    assert_eq!(
        pending.model_deletion_preview.as_ref().unwrap().phase,
        "awaiting_ack"
    );
    let replayed = f.run(&pending, Action::Publish);
    assert_eq!(replayed.phase, "completed");
}
