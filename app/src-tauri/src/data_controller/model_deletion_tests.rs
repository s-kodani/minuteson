//! Real shared leases and temp synthetic models, never application data or engines.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::models::tests::Fixture as Models;

struct Fixture {
    model: Models,
    controller: DataController,
    analysis: AnalysisController,
}
impl Fixture {
    fn new() -> Self {
        Self {
            model: Models::new(),
            controller: DataController::default(),
            analysis: AnalysisController::default(),
        }
    }
    fn lease(&self, snapshot: Option<&Snapshot>) -> DataMaintenanceLease {
        if let Some(p) = snapshot
            .and_then(|s| s.model_deletion_preview.as_ref())
            .filter(|p| p.pending)
        {
            self.analysis
                .begin_model_deletion_maintenance(
                    &self.model.storage,
                    &p.operation_id,
                    false,
                    false,
                    &|| false,
                )
                .unwrap()
        } else {
            self.analysis
                .begin_data_maintenance(false, false, &|| false)
                .unwrap()
        }
    }
    fn ready(&self) -> Snapshot {
        let lease = self.lease(None);
        let (started, _, previous) = self.controller.begin(Kind::ModelDeletion).unwrap();
        self.finish(
            Work::ModelDeletion(
                self.model.root.clone(),
                ModelTarget::Fresh {
                    package: self.model.package.clone(),
                    review: self.model.review.clone(),
                },
                previous,
            ),
            &started,
            &lease,
            &|| false,
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
        let (started, _, previous) = self.controller.begin(Kind::ModelDeletion).unwrap();
        self.finish(
            Work::ModelDeletion(
                self.model.root.clone(),
                ModelTarget::Pending(operation.into()),
                previous,
            ),
            &started,
            &lease,
            &|| false,
            &|_| {},
        )
    }
    fn finish(
        &self,
        work: Work,
        started: &Snapshot,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Snapshot {
        self.controller
            .finish(
                started.token.as_ref().unwrap(),
                work.run_admitted(&self.model.storage, check, progress, Some(lease)),
            )
            .unwrap()
    }
    fn request(&self, snapshot: &Snapshot, action: Action) -> Request {
        Request {
            token: snapshot.token.clone().unwrap(),
            confirmation_id: snapshot
                .model_deletion_preview
                .as_ref()
                .unwrap()
                .confirmation_id
                .clone(),
            action,
        }
    }
    fn run(&self, snapshot: &Snapshot, action: Action) -> Snapshot {
        let lease = self.lease(Some(snapshot));
        let (work, _, started) = self
            .controller
            .begin_request(self.request(snapshot, action))
            .unwrap()
            .unwrap();
        self.finish(work, &started, &lease, &|| false, &|_| {})
    }
    fn stop_at(&self, snapshot: &Snapshot, action: Action, phase: &str) -> Snapshot {
        let lease = self.lease(Some(snapshot));
        let (work, _, started) = self
            .controller
            .begin_request(self.request(snapshot, action))
            .unwrap()
            .unwrap();
        let stopped = AtomicBool::new(false);
        self.finish(
            work,
            &started,
            &lease,
            &|| stopped.load(Ordering::Acquire),
            &|p| {
                if p == phase {
                    stopped.store(true, Ordering::Release);
                }
            },
        )
    }
    fn operations(&self) -> i64 {
        self.model.operation_count()
    }
}

#[test]
fn model_controller_fresh_review_is_read_only_paged_and_bound_to_model_selection() {
    let f = Fixture::new();
    for n in 0..55 {
        std::fs::write(f.model.path(&format!("unknown-{n:02}")), b"SYNTHETIC").unwrap();
    }
    let before = f.model.evidence();
    let s = f.ready();
    let p = s.model_deletion_preview.as_ref().unwrap();
    assert_eq!(s.phase, "ready");
    assert!(p.execute_allowed && !p.pending && !p.keep_allowed);
    assert_eq!(p.path_count, 60);
    assert_eq!(p.unregistered_count, 55);
    let a = f
        .controller
        .model_deletion_page(&p.confirmation_id, 0)
        .unwrap();
    let b = f
        .controller
        .model_deletion_page(&p.confirmation_id, 50)
        .unwrap();
    assert_eq!(
        (
            a.entries.len(),
            a.next_offset,
            b.entries.len(),
            b.next_offset
        ),
        (50, Some(50), 10, None)
    );
    assert!(a
        .entries
        .iter()
        .chain(b.entries.iter())
        .all(|e| e.relative_path.starts_with(&f.model.package)));
    assert!(f
        .controller
        .model_deletion_page(&Uuid::new_v4().to_string(), 0)
        .is_err());
    assert!(f
        .controller
        .model_deletion_page(&p.confirmation_id, 1)
        .is_err());
    assert!(f
        .controller
        .model_deletion_page(&p.confirmation_id, 100)
        .is_err());
    assert!(f.controller.deletion_page(&p.confirmation_id, 0).is_err());
    assert_eq!(f.model.evidence(), before);
}
#[test]
fn model_controller_refresh_replays_lost_response_and_invalidates_old_confirmation() {
    let f = Fixture::new();
    let s = f.ready();
    let old = f.request(&s, Action::Refresh);
    let next = f.run(&s, Action::Refresh);
    assert_eq!(next.token, s.token);
    assert_ne!(
        next.model_deletion_preview
            .as_ref()
            .unwrap()
            .confirmation_id,
        old.confirmation_id
    );
    assert!(f.controller.begin_request(old).unwrap().is_none());
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .is_err());
    assert_eq!(f.operations(), 0);
}
#[test]
fn model_controller_unknown_choice_is_explicit_and_completed_requests_are_replay_only() {
    let f = Fixture::new();
    std::fs::write(f.model.path("unknown"), b"UNKNOWN").unwrap();
    let s = f.ready();
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .is_err());
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Keep))
        .is_err());
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Recover))
        .is_err());
    let request = f.request(&s, Action::DeleteIncludingUnregistered);
    let done = f.run(&s, Action::DeleteIncludingUnregistered);
    let r = done.model_deletion_receipt.as_ref().unwrap();
    let p = done.model_deletion_preview.as_ref().unwrap();
    assert_eq!(done.phase, "completed");
    assert!(r.body_deleted && !r.retained && r.history_retained);
    assert!(!p.pending && !p.execute_allowed && !p.keep_allowed);
    assert_eq!(p.processed_paths, p.path_count);
    assert!(!f.model.path("").exists());
    assert_eq!(f.model.storage.list_meetings().unwrap().len(), 1);
    let count = f.operations();
    assert_eq!(count, 1);
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert_eq!(f.operations(), count);
    assert!(f
        .controller
        .begin_request(f.request(&done, Action::Refresh))
        .is_err());
}
#[test]
fn model_controller_unstarted_saved_policy_resumes_without_new_unknown_decision() {
    let f = Fixture::new();
    std::fs::write(f.model.path("unknown"), b"UNKNOWN").unwrap();
    let stopped = f.stop_at(
        &f.ready(),
        Action::DeleteIncludingUnregistered,
        "model_delete_scope_fixed",
    );
    let p = stopped.model_deletion_preview.as_ref().unwrap();
    assert_eq!(stopped.phase, "attention");
    assert_eq!(p.phase, "not_started");
    assert!(p.keep_allowed && p.pending && !p.execute_allowed);
    assert_eq!(p.include_unregistered_recorded, Some(true));
    assert!(f.model.path("unknown").exists());
    let s = f.run(&stopped, Action::Refresh);
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::DeleteIncludingUnregistered))
        .is_err());
    assert!(
        f.run(&s, Action::Publish)
            .model_deletion_receipt
            .unwrap()
            .body_deleted
    );
}
#[test]
fn model_controller_unstarted_keep_retains_body_history_and_durable_resolution() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "model_delete_scope_fixed");
    let request = f.request(&stopped, Action::Keep);
    let done = f.run(&stopped, Action::Keep);
    let r = done.model_deletion_receipt.unwrap();
    assert!(r.retained && !r.body_deleted && r.history_retained);
    assert_eq!(done.phase, "completed");
    assert!(f.model.path("cli").exists());
    assert!(!f.model.storage.data_write_blocked());
    assert!(model_deletion::listing(&f.model.storage, &f.model.root)
        .unwrap()
        .items
        .is_empty());
    assert!(f.controller.begin_request(request).unwrap().is_none());
}
#[test]
fn model_controller_started_deletion_cannot_keep_and_refresh_is_required_after_cancel() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_model_files");
    assert_eq!(stopped.phase, "attention");
    let p = stopped.model_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "files");
    assert!(p.pending && !p.keep_allowed);
    assert!(f
        .controller
        .begin_request(f.request(&stopped, Action::Keep))
        .is_err());
    assert!(f
        .controller
        .begin_request(f.request(&stopped, Action::Publish))
        .is_err());
    let s = f.run(&stopped, Action::Refresh);
    assert!(
        f.run(&s, Action::Publish)
            .model_deletion_receipt
            .unwrap()
            .body_deleted
    );
}
#[test]
fn model_controller_changed_after_fresh_view_stops_without_fixing_or_adopting_new_bytes() {
    let f = Fixture::new();
    let s = f.ready();
    std::fs::write(f.model.path("cli"), b"CHANGED").unwrap();
    let failed = f.run(&s, Action::Publish);
    assert!(failed.model_deletion_receipt.is_none());
    assert_eq!(f.operations(), 0);
    assert_eq!(std::fs::read(f.model.path("cli")).unwrap(), b"CHANGED");
    assert!(f
        .controller
        .begin_request(f.request(&failed, Action::Publish))
        .is_err());
    let next = f.run(&failed, Action::Refresh);
    assert_eq!(
        next.model_deletion_preview.as_ref().unwrap().changed_count,
        1
    );
    assert!(
        f.run(&next, Action::Publish)
            .model_deletion_receipt
            .unwrap()
            .body_deleted
    );
}
#[test]
fn model_controller_reconfirmation_is_separate_read_only_consent_and_resumes_recorded_new_policy() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_model_files");
    std::fs::write(f.model.path("cli"), b"CHANGED").unwrap();
    std::fs::write(f.model.path("unknown"), b"UNKNOWN").unwrap();
    let s = f.run(&stopped, Action::Refresh);
    let p = s.model_deletion_preview.as_ref().unwrap();
    assert!(!p.execute_allowed && p.reconfirmation_allowed);
    let old = f.request(&s, Action::ReviewRemaining);
    let reviewed = f.run(&s, Action::ReviewRemaining);
    let p = reviewed.model_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "reconfirmation");
    assert!(p.execute_allowed && !p.keep_allowed && !p.reconfirmation_allowed);
    assert_eq!(f.operations(), 1);
    assert_eq!(p.previous_processed_paths, Some(0));
    assert!(f.controller.begin_request(old).unwrap().is_none());
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .is_err());
    assert!(f
        .controller
        .begin_request(f.request(&reviewed, Action::Publish))
        .is_err());
    let fixed = f.stop_at(
        &reviewed,
        Action::DeleteIncludingUnregistered,
        "model_delete_scope_fixed",
    );
    assert_eq!(f.operations(), 2);
    assert_eq!(
        fixed
            .model_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(true)
    );
    let checked = f.run(&fixed, Action::Refresh);
    assert!(
        f.run(&checked, Action::Publish)
            .model_deletion_receipt
            .unwrap()
            .body_deleted
    );
    assert_eq!(f.operations(), 2);
}
#[test]
fn model_controller_completed_reappearance_keeps_original_receipt_and_requires_new_scope() {
    let f = Fixture::new();
    let done = f.run(&f.ready(), Action::Publish);
    let operation = done.model_deletion_receipt.unwrap().operation_id;
    std::fs::create_dir(f.model.path("")).unwrap();
    std::fs::write(f.model.path("new"), b"NEW").unwrap();
    let s = f.pending(&operation);
    let p = s.model_deletion_preview.as_ref().unwrap();
    assert!(p.prior_deletion_completed && p.reconfirmation_allowed && !p.execute_allowed);
    let reviewed = f.run(&s, Action::ReviewRemaining);
    assert!(
        reviewed
            .model_deletion_preview
            .as_ref()
            .unwrap()
            .prior_deletion_completed
    );
    let fixed = f.stop_at(
        &reviewed,
        Action::DeleteIncludingUnregistered,
        "model_delete_scope_fixed",
    );
    let state = f.model.operation_state(&operation);
    assert_eq!(state, "completed");
    let next = f.run(&fixed, Action::Refresh);
    assert!(
        f.run(&next, Action::Publish)
            .model_deletion_receipt
            .unwrap()
            .body_deleted
    );
}
#[test]
fn model_controller_cold_completed_attention_requires_new_scope_then_restart() {
    let mut f = Fixture::new();
    let done = f.run(&f.ready(), Action::Publish);
    let operation = done.model_deletion_receipt.unwrap().operation_id;
    std::fs::create_dir(f.model.path("")).unwrap();
    std::fs::write(f.model.path("new"), b"COLD NEW CONSENT").unwrap();
    f.model = f.model.cold();
    f.controller = DataController::default();
    f.analysis = AnalysisController::default();
    let listing = model_deletion::listing(&f.model.storage, &f.model.root).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].state, "completed");
    assert_eq!(
        listing.items[0].startup_reason.as_deref(),
        Some("model_completed_scope_reappeared")
    );
    let s = f.pending(&operation);
    assert!(
        s.model_deletion_preview
            .as_ref()
            .unwrap()
            .prior_deletion_completed
    );
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .is_err());
    let reviewed = f.run(&s, Action::ReviewRemaining);
    let accepted = f.request(&reviewed, Action::DeleteIncludingUnregistered);
    let done = f.run(&reviewed, Action::DeleteIncludingUnregistered);
    assert_eq!(done.phase, "completed");
    assert!(done.restart_required && done.model_deletion_receipt.unwrap().body_deleted);
    assert!(f.controller.begin_request(accepted).unwrap().is_none());
    assert_eq!(f.model.operation_state(&operation), "completed");
    assert!(!f.model.path("").exists());
    assert!(f.model.storage.data_write_blocked());
    assert!(model_deletion::listing(&f.model.storage, &f.model.root)
        .unwrap()
        .items
        .is_empty());
    f.model = f.model.cold();
    assert!(!f.model.storage.data_write_blocked());
}
#[test]
fn model_controller_missing_lease_and_cancelled_lease_cannot_create_or_execute_scope() {
    let f = Fixture::new();
    let (s, _, previous) = f.controller.begin(Kind::ModelDeletion).unwrap();
    let result = Work::ModelDeletion(
        f.model.root.clone(),
        ModelTarget::Fresh {
            package: f.model.package.clone(),
            review: f.model.review.clone(),
        },
        previous,
    )
    .run(&f.model.storage, &|| false, &|_| {});
    assert!(result.error.is_some() && result.selection.is_none());
    f.controller
        .finish(s.token.as_ref().unwrap(), result)
        .unwrap();
    let ready = f.ready();
    let (work, _, started) = f
        .controller
        .begin_request(f.request(&ready, Action::Publish))
        .unwrap()
        .unwrap();
    let result = work.run(&f.model.storage, &|| false, &|_| {});
    let failed = f
        .controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert!(failed.model_deletion_receipt.is_none());
    assert_eq!(f.operations(), 0);
    assert!(f.model.path("cli").exists());
    let lease = f.lease(None);
    let p = model_deletion::Prepared::fresh(
        &f.model.storage,
        &f.model.root,
        &f.model.package,
        &f.model.review,
        &lease,
        &|| true,
    );
    assert!(p.is_err());
    assert_eq!(f.operations(), 0);
    let mut p = model_deletion::Prepared::fresh(
        &f.model.storage,
        &f.model.root,
        &f.model.package,
        &f.model.review,
        &lease,
        &|| false,
    )
    .unwrap();
    let confirmation = p.preview().confirmation_id;
    f.analysis.begin_exit().unwrap();
    f.analysis.abort_shutdown();
    assert!(lease.cancelled());
    assert!(p
        .execute(
            &f.model.storage,
            &lease,
            &confirmation,
            false,
            &|| false,
            &|_| {}
        )
        .is_err());
    assert!(p.refresh(&f.model.storage, &lease, &|| false).is_err());
    assert_eq!(f.operations(), 0);
}
#[test]
fn model_controller_completion_wins_late_cancel_and_publishes_exact_path_progress() {
    let f = Fixture::new();
    let s = f.ready();
    let lease = f.lease(Some(&s));
    let (work, cancel, started) = f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .unwrap()
        .unwrap();
    let done = f.finish(
        work,
        &started,
        &lease,
        &|| cancel.load(Ordering::Acquire),
        &|phase| {
            if phase == "model_delete_completed" {
                cancel.store(true, Ordering::Release);
            }
        },
    );
    assert_eq!(done.phase, "completed");
    assert!(done.model_deletion_receipt.is_some());
    assert!(!done.retry_allowed && !done.attention_required);
    let p = done.model_deletion_preview.unwrap();
    assert_eq!(p.processed_paths, p.path_count);
}
#[test]
fn model_controller_pending_listing_and_native_root_refuse_forged_or_foreign_scope() {
    let f = Fixture::new();
    let s = f.stop_at(&f.ready(), Action::Publish, "model_delete_scope_fixed");
    let operation = s.model_deletion_preview.unwrap().operation_id;
    let listed = model_deletion::listing(&f.model.storage, &f.model.root).unwrap();
    assert!(listed.write_blocked);
    assert_eq!(listed.items.len(), 1);
    assert!(listed.items[0].review_allowed);
    assert_eq!(listed.items[0].package_id.as_ref(), Some(&f.model.package));
    let foreign = model_deletion::listing(&f.model.storage, &f.model.root.join("foreign")).unwrap();
    assert!(!foreign.items[0].review_allowed);
    let lease = f
        .analysis
        .begin_model_deletion_maintenance(&f.model.storage, &operation, false, false, &|| false)
        .unwrap();
    assert!(model_deletion::Prepared::pending(
        &f.model.storage,
        &f.model.root.join("foreign"),
        &operation,
        &lease,
        &|| false
    )
    .is_err());
    assert!(model_deletion::Prepared::pending(
        &f.model.storage,
        &f.model.root,
        &Uuid::new_v4().to_string(),
        &lease,
        &|| false
    )
    .is_err());
    assert_eq!(f.operations(), 1);
    assert!(f.model.path("cli").exists());
}

#[test]
fn model_controller_cold_restart_review_and_keep_preserve_restart_fence() {
    let mut f = Fixture::new();
    let s = f.stop_at(&f.ready(), Action::Publish, "model_delete_scope_fixed");
    let operation = s.model_deletion_preview.unwrap().operation_id;
    f.model = f.model.cold();
    let storage = &f.model.storage;
    assert!(storage.data_write_blocked() && storage.deletion_resume_restart_required());
    let controller = DataController::default();
    let analysis = AnalysisController::default();
    analysis.require_storage_restart();
    assert!(analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    let lease = analysis
        .begin_model_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    let (started, _, previous) = controller.begin(Kind::ModelDeletion).unwrap();
    let result = Work::ModelDeletion(
        f.model.root.clone(),
        ModelTarget::Pending(operation),
        previous,
    )
    .run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
    let s = controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert!(!s.restart_required && s.model_deletion_preview.as_ref().unwrap().pending);
    let request = Request {
        token: s.token.clone().unwrap(),
        confirmation_id: s.model_deletion_preview.unwrap().confirmation_id,
        action: Action::Keep,
    };
    let (work, _, started) = controller.begin_request(request).unwrap().unwrap();
    let result = work.run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
    let done = controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert!(done.restart_required && done.model_deletion_receipt.unwrap().retained);
    assert!(storage.restart_required());
    assert!(f.model.path("cli").exists());
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
    drop(lease);
    f.model = f.model.cold();
    assert!(!f.model.storage.restart_required());
}
#[test]
fn model_controller_cold_restart_continues_only_saved_policy_with_shared_admission() {
    let mut f = Fixture::new();
    std::fs::write(f.model.path("unknown"), b"UNKNOWN").unwrap();
    let s = f.stop_at(
        &f.ready(),
        Action::DeleteIncludingUnregistered,
        "model_delete_scope_fixed",
    );
    let operation = s.model_deletion_preview.unwrap().operation_id;
    f.model = f.model.cold();
    let storage = &f.model.storage;
    let controller = DataController::default();
    let analysis = AnalysisController::default();
    analysis.require_storage_restart();
    let lease = analysis
        .begin_model_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    let (started, _, previous) = controller.begin(Kind::ModelDeletion).unwrap();
    let result = Work::ModelDeletion(
        f.model.root.clone(),
        ModelTarget::Pending(operation),
        previous,
    )
    .run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
    let s = controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert_eq!(s.phase, "ready");
    assert_eq!(
        s.model_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(true)
    );
    let request = Request {
        token: s.token.clone().unwrap(),
        confirmation_id: s.model_deletion_preview.unwrap().confirmation_id,
        action: Action::Publish,
    };
    let (work, _, started) = controller.begin_request(request).unwrap().unwrap();
    let result = work.run_admitted(&storage, &|| false, &|_| {}, Some(&lease));
    let done = controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert!(done.restart_required && done.model_deletion_receipt.unwrap().body_deleted);
    assert!(!f.model.path("").exists());
    drop(lease);
    f.model = f.model.cold();
    assert!(!f.model.storage.restart_required());
}

#[test]
fn model_controller_shutdown_cancellation_never_releases_the_running_lease() {
    let f = Fixture::new();
    let s = f.ready();
    let lease = f.lease(Some(&s));
    let (work, cancel, started) = f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .unwrap()
        .unwrap();
    assert!(f.controller.stop(Duration::ZERO).is_err());
    assert!(cancel.load(Ordering::Acquire));
    assert!(f.controller.snapshot().unwrap().busy);
    assert!(lease.validate_active().is_ok());
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    let done = f.finish(
        work,
        &started,
        &lease,
        &|| cancel.load(Ordering::Acquire),
        &|_| {},
    );
    assert_eq!(done.phase, "cancelled");
    assert!(!done.busy);
    assert!(done.model_deletion_receipt.is_none());
    assert_eq!(f.operations(), 0);
    assert!(f.model.path("cli").exists());
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    drop(lease);
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_ok());
}
