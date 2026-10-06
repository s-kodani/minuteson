//! Shared controller with real maintenance leases and synthetic, private fixtures.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::audio::tests::Fixture as Audio;

struct Fixture {
    audio: Audio,
    controller: DataController,
    analysis: AnalysisController,
}
impl Fixture {
    fn new() -> Self {
        Self {
            audio: Audio::new(),
            controller: DataController::default(),
            analysis: AnalysisController::default(),
        }
    }
    fn lease(&self, snapshot: Option<&Snapshot>) -> DataMaintenanceLease {
        let operation = snapshot
            .and_then(|s| s.audio_deletion_preview.as_ref())
            .and_then(|p| {
                if p.phase == "reconfirmation" {
                    p.source_operation_id.as_deref()
                } else if p.pending {
                    Some(p.operation_id.as_str())
                } else {
                    None
                }
            });
        match operation {
            Some(id) => self
                .analysis
                .begin_audio_deletion_maintenance(&self.audio.storage, id, false, false, &|| false)
                .unwrap(),
            None => self
                .analysis
                .begin_data_maintenance(false, false, &|| false)
                .unwrap(),
        }
    }
    fn ready(&self) -> Snapshot {
        let lease = self.lease(None);
        let (s, _, previous) = self.controller.begin(Kind::AudioDeletion).unwrap();
        self.finish(
            Work::AudioDeletion(AudioTarget::Fresh(self.audio.meeting.clone()), previous),
            &s,
            &lease,
            &|| false,
            &|_| {},
        )
    }
    fn pending(&self, operation: &str) -> Snapshot {
        let lease = self
            .analysis
            .begin_audio_deletion_maintenance(&self.audio.storage, operation, false, false, &|| {
                false
            })
            .unwrap();
        let (s, _, previous) = self.controller.begin(Kind::AudioDeletion).unwrap();
        self.finish(
            Work::AudioDeletion(AudioTarget::Pending(operation.into()), previous),
            &s,
            &lease,
            &|| false,
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
                work.run_admitted(&self.audio.storage, check, progress, Some(lease)),
            )
            .unwrap()
    }
    fn request(&self, s: &Snapshot, action: Action) -> Request {
        Request {
            token: s.token.clone().unwrap(),
            confirmation_id: s
                .audio_deletion_preview
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
    fn count(&self) -> i64 {
        self.audio.operation_count()
    }
    fn original(&self, operation: &str) -> (String, String, i64, i64, i64) {
        self.audio.original_deletion_record(operation)
    }
}

#[test]
fn audio_controller_review_is_readonly_paged_and_has_no_serialized_permit() {
    let f = Fixture::new();
    for i in 0..55 {
        f.audio.file(&format!("unknown-{i:02}"), None);
    }
    let s = f.ready();
    let p = s.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(s.phase, "ready");
    assert!(p.execute_allowed && !p.pending && !p.keep_allowed);
    assert_eq!(
        (p.path_count, p.audio_files, p.unclassified_files),
        (56, 1, 55)
    );
    let a = f
        .controller
        .audio_deletion_page(&p.confirmation_id, 0)
        .unwrap();
    let b = f
        .controller
        .audio_deletion_page(&p.confirmation_id, 50)
        .unwrap();
    assert_eq!(
        (
            a.entries.len(),
            a.next_offset,
            b.entries.len(),
            b.next_offset
        ),
        (50, Some(50), 6, None)
    );
    assert!(f
        .controller
        .audio_deletion_page(&p.confirmation_id, 1)
        .is_err());
    assert!(f
        .controller
        .audio_deletion_page(&Uuid::new_v4().to_string(), 0)
        .is_err());
    assert_eq!(f.count(), 0);
    assert!(f.audio.path.exists());
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("PRIVATE AUDIO")
            && !json.contains("PRIVATE MEETING")
            && !json.contains("PRIVATE FILE")
    );
    assert!(!json.contains("manifest") && !json.contains("censusHash"));
}
#[test]
fn audio_controller_requires_live_shared_lease_and_rejects_busy_recording() {
    let f = Fixture::new();
    let result = Work::AudioDeletion(AudioTarget::Fresh(f.audio.meeting.clone()), None)
        .run_admitted(&f.audio.storage, &|| false, &|_| {}, None);
    assert!(result.error.is_some() && result.selection.is_none());
    assert!(f
        .analysis
        .begin_data_maintenance(true, false, &|| false)
        .is_err());
    let lease = f.lease(None);
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    drop(lease);
    assert_eq!(f.count(), 0);
}
#[test]
fn audio_controller_stale_tokens_and_confirmation_cannot_delete() {
    let f = Fixture::new();
    let s = f.ready();
    let mut r = f.request(&s, Action::Publish);
    r.confirmation_id = Uuid::new_v4().to_string();
    assert!(f.controller.begin_request(r).is_err());
    let mut r = f.request(&s, Action::Publish);
    r.token = Uuid::new_v4().to_string();
    assert!(f.controller.begin_request(r).is_err());
    let refreshed = f.run(&s, Action::Refresh);
    assert_ne!(
        s.audio_deletion_preview.unwrap().confirmation_id,
        refreshed.audio_deletion_preview.unwrap().confirmation_id
    );
    assert_eq!(f.count(), 0);
    assert!(f.audio.path.exists());
}
#[test]
fn audio_controller_deletes_only_audio_and_replays_result_without_second_operation() {
    let f = Fixture::new();
    f.audio.documents();
    let document = f.audio.file("notes.json", Some("application/json"));
    let unknown = f.audio.file("private-unknown", None);
    let s = f.ready();
    let request = f.request(&s, Action::Publish);
    let done = f.run(&s, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    let r = done.audio_deletion_receipt.as_ref().unwrap();
    assert!(r.audio_deleted && !r.retained && r.history_retained);
    assert!(!f.audio.path.exists() && document.exists() && unknown.exists());
    assert_eq!(f.audio.storage.list_meetings().unwrap().len(), 1);
    assert!(
        done.audio_deletion_preview
            .as_ref()
            .unwrap()
            .retained
            .utterances
            > 0
    );
    assert_eq!(f.count(), 1);
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert_eq!(f.count(), 1);
}
#[test]
fn audio_controller_unregistered_audio_requires_separate_choice_unknown_is_always_retained() {
    let f = Fixture::new();
    let leftover = f.audio.unregistered_audio();
    let unknown = f.audio.file("unknown", None);
    let s = f.ready();
    assert_eq!(
        s.audio_deletion_preview
            .as_ref()
            .unwrap()
            .unregistered_audio_files,
        1
    );
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
    let done = f.run(&s, Action::DeleteIncludingUnregistered);
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted);
    assert!(!leftover.exists() && unknown.exists());
}
#[test]
fn audio_controller_saved_unstarted_policy_continues_without_new_choice() {
    let f = Fixture::new();
    let leftover = f.audio.unregistered_audio();
    let stopped = f.stop_at(
        &f.ready(),
        Action::DeleteIncludingUnregistered,
        "audio_delete_scope_fixed",
    );
    let p = stopped.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(
        (stopped.phase.as_str(), p.phase.as_str()),
        ("attention", "not_started")
    );
    assert!(p.pending && p.keep_allowed && !p.execute_allowed);
    assert_eq!(p.include_unregistered_recorded, Some(true));
    assert!(f.audio.path.exists() && leftover.exists());
    assert!(
        audio_deletion::listing(&f.audio.storage)
            .unwrap()
            .write_blocked
    );
    let model_listing =
        model_deletion::listing(&f.audio.storage, &f.audio.storage.root().join("models")).unwrap();
    assert!(model_listing.items.is_empty() && model_listing.write_blocked);
    let s = f.run(&stopped, Action::Refresh);
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::DeleteIncludingUnregistered))
        .is_err());
    assert!(
        f.run(&s, Action::Publish)
            .audio_deletion_receipt
            .unwrap()
            .audio_deleted
    );
    assert!(!leftover.exists());
    assert_eq!(f.count(), 1);
}
#[test]
fn audio_controller_keep_unstarted_preserves_audio_and_clears_fence() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "audio_delete_scope_fixed");
    let request = f.request(&stopped, Action::Keep);
    let done = f.run(&stopped, Action::Keep);
    assert_eq!(done.phase, "completed");
    let r = done.audio_deletion_receipt.unwrap();
    assert!(r.retained && !r.audio_deleted && r.history_retained);
    assert!(f.audio.path.exists());
    assert!(!f.audio.storage.data_write_blocked());
    assert!(audio_deletion::listing(&f.audio.storage)
        .unwrap()
        .items
        .is_empty());
    assert!(f.controller.begin_request(request).unwrap().is_none());
}
#[test]
fn audio_controller_started_cancel_cannot_keep_and_requires_refresh() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_audio_files");
    let p = stopped.audio_deletion_preview.as_ref().unwrap();
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
            .audio_deletion_receipt
            .unwrap()
            .audio_deleted
    );
}
#[test]
fn audio_controller_availability_commit_waits_and_resume_keeps_same_operation() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "fixing_audio_unavailable");
    assert!(!f.audio.path.exists());
    let p = stopped.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "availability");
    let pending = f.pending(&p.operation_id);
    assert!(
        pending
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .execute_allowed
    );
    assert!(
        f.run(&pending, Action::Publish)
            .audio_deletion_receipt
            .unwrap()
            .audio_deleted
    );
    assert_eq!(f.count(), 1);
}
#[test]
fn audio_controller_changed_fresh_scope_is_not_implicitly_adopted() {
    let f = Fixture::new();
    let s = f.ready();
    std::fs::write(&f.audio.path, b"CHANGED SYNTHETIC AUDIO").unwrap();
    let failed = f.run(&s, Action::Publish);
    assert!(failed.audio_deletion_receipt.is_none());
    assert_eq!(f.count(), 0);
    assert!(f.audio.path.exists());
    let s = f.run(&failed, Action::Refresh);
    assert_eq!(
        s.audio_deletion_preview
            .as_ref()
            .unwrap()
            .changed_audio_files,
        1
    );
    assert!(
        f.run(&s, Action::Publish)
            .audio_deletion_receipt
            .unwrap()
            .audio_deleted
    );
}
#[test]
fn audio_controller_completed_reappearance_uses_source_admission_then_new_saved_operation() {
    let f = Fixture::new();
    let first = f.run(&f.ready(), Action::Publish);
    let old = first.audio_deletion_receipt.unwrap().operation_id;
    std::fs::write(&f.audio.path, b"RETURNED SYNTHETIC AUDIO").unwrap();
    let reserved = f
        .audio
        .path
        .with_file_name(format!(".mon-delete-audio-{old}-0"));
    std::fs::write(&reserved, b"RETURNED RESERVED AUDIO").unwrap();
    let pending = f.pending(&old);
    let p = pending.audio_deletion_preview.as_ref().unwrap();
    assert!(!p.execute_allowed && p.reconfirmation_allowed);
    let reviewed = f.run(&pending, Action::ReviewRemaining);
    let p = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "reconfirmation");
    assert_eq!(p.unregistered_audio_files, 1);
    assert_eq!(p.source_operation_id.as_deref(), Some(old.as_str()));
    assert_ne!(p.operation_id, old);
    {
        let state = f.controller.state.lock().unwrap();
        let Some(Selection::AudioDeletion(prepared)) = &state.selection else {
            panic!()
        };
        assert_eq!(prepared.admission_operation(), Some(old.as_str()));
    }
    assert_eq!(f.count(), 1);
    let fixed = f.stop_at(
        &reviewed,
        Action::DeleteIncludingUnregistered,
        "audio_delete_scope_fixed",
    );
    assert_eq!(f.count(), 2);
    let p = fixed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "not_started");
    {
        let state = f.controller.state.lock().unwrap();
        let Some(Selection::AudioDeletion(prepared)) = &state.selection else {
            panic!()
        };
        assert_eq!(
            prepared.admission_operation(),
            Some(p.operation_id.as_str())
        );
    }
    let resumed = f.run(&fixed, Action::Refresh);
    let done = f.run(&resumed, Action::Publish);
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted);
    assert_eq!(f.count(), 2);
    assert!(!f.audio.path.exists());
    assert!(!reserved.exists());
}
#[test]
fn audio_controller_cancel_after_completion_does_not_erase_committed_receipt() {
    let f = Fixture::new();
    let done = f.stop_at(&f.ready(), Action::Publish, "audio_delete_completed");
    assert_eq!(done.phase, "completed");
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted);
    assert_eq!(f.count(), 1);
}

#[test]
fn audio_controller_ancestral_remaining_path_is_paged_and_requires_new_explicit_choice() {
    let mut f = Fixture::new();
    let parent = f
        .run(&f.ready(), Action::Publish)
        .audio_deletion_receipt
        .unwrap()
        .operation_id;
    let parent_original = f.original(&parent);
    std::fs::write(&f.audio.path, b"CHILD ORIGINAL SCOPE").unwrap();
    let reviewed = f.run(&f.pending(&parent), Action::ReviewRemaining);
    let stopped = f.stop_at(&reviewed, Action::Publish, "deleting_audio_files");
    let op = stopped
        .audio_deletion_preview
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    assert_ne!(op, parent);
    let original = f.original(&op);
    let returned = f
        .audio
        .path
        .with_file_name(format!(".mon-delete-audio-{parent}-0"));
    let unrelated = f
        .audio
        .path
        .with_file_name(format!(".mon-delete-audio-{parent}-999"));
    std::fs::write(
        &returned,
        b"ANCESTRAL PATH ABSENT FROM CHILD ORIGINAL CONSENT",
    )
    .unwrap();
    std::fs::write(&unrelated, b"LOOKALIKE RETAIN").unwrap();
    let pending = f.run(&stopped, Action::Refresh);
    let reviewed = f.run(&pending, Action::ReviewRemaining);
    let preview = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(
        preview.phase, "reconfirmation",
        "{:?}",
        reviewed.error_message
    );
    assert_eq!(preview.source_operation_id.as_deref(), Some(op.as_str()));
    assert_eq!(preview.operation_id, op);
    assert_eq!(
        preview.reconfirmation_source_kind.as_deref(),
        Some("native_audio_pending")
    );
    assert_eq!(preview.unregistered_audio_files, 1);
    let page = f
        .controller
        .audio_deletion_page(&preview.confirmation_id, 0)
        .unwrap();
    let relative = returned
        .strip_prefix(f.audio.storage.root())
        .unwrap()
        .to_str()
        .unwrap();
    assert!(page.entries.iter().any(|e| e.relative_path == relative
        && matches!(
            e.disposition,
            crate::storage::deletion::audio::Disposition::HistoricalAudio
        )));
    assert!(f
        .controller
        .begin_request(f.request(&reviewed, Action::Publish))
        .is_err());
    let saved = f.stop_at(
        &reviewed,
        Action::DeleteIncludingUnregistered,
        "audio_delete_scope_fixed",
    );
    assert_eq!(f.count(), 3);
    assert!(returned.exists() && unrelated.exists());
    assert_eq!(f.original(&parent), parent_original);
    assert_eq!(f.original(&op), original);
    f.audio = f.audio.reopen();
    f.controller = DataController::default();
    f.analysis.require_storage_restart();
    let pending = f.pending(&op);
    assert_eq!(
        pending
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(true)
    );
    let done = f.run(&pending, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert_eq!(done.audio_deletion_receipt.unwrap().operation_id, op);
    assert!(!returned.exists() && !f.audio.path.exists() && unrelated.exists());
    assert_eq!(f.original(&parent), parent_original);
    assert_eq!(f.original(&op), original);
    assert_eq!(f.count(), 3);
    assert_eq!(saved.audio_deletion_preview.unwrap().operation_id, op);
    f.audio = f.audio.reopen();
    assert!(!f.audio.storage.data_write_blocked());
}
#[test]
fn audio_controller_modified_unfinished_scope_requires_readonly_review_and_separate_consent() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_audio_files");
    std::fs::write(&f.audio.path, b"CHANGED AFTER CONSENT").unwrap();
    let s = f.run(&stopped, Action::Refresh);
    let p = s.audio_deletion_preview.as_ref().unwrap();
    assert!(!p.execute_allowed && p.reconfirmation_allowed && !p.keep_allowed);
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Publish))
        .is_err());
    assert!(f.audio.path.exists() && f.audio.storage.data_write_blocked());
    let operation = p.operation_id.clone();
    let original = f.original(&operation);
    let request = f.request(&s, Action::ReviewRemaining);
    let reviewed = f.run(&s, Action::ReviewRemaining);
    let p = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(reviewed.phase, "ready", "{:?}", reviewed.error_message);
    assert_eq!(p.phase, "reconfirmation");
    assert_eq!(
        p.reconfirmation_source_kind.as_deref(),
        Some("native_audio_pending")
    );
    assert_eq!(p.source_operation_id.as_deref(), Some(operation.as_str()));
    assert_eq!(p.operation_id, operation);
    assert_ne!(
        p.confirmation_id,
        s.audio_deletion_preview.as_ref().unwrap().confirmation_id
    );
    assert!(
        p.execute_allowed
            && !p.keep_allowed
            && !p.reconfirmation_allowed
            && !p.prior_deletion_completed
    );
    assert_eq!(p.include_unregistered_recorded, None);
    assert_eq!(p.changed_audio_files, 1);
    assert_eq!(f.count(), 1);
    assert_eq!(f.original(&operation), original);
    assert!(f.controller.begin_request(request).unwrap().is_none());
    let done = f.run(&reviewed, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert_eq!(done.audio_deletion_receipt.unwrap().operation_id, operation);
    assert_eq!(f.count(), 2);
    assert_eq!(f.original(&operation), original);
    assert!(!f.audio.path.exists());
}

#[test]
fn audio_controller_remaining_consent_uses_new_choice_and_cold_resumes_same_operation() {
    let mut f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_audio_files");
    let operation = stopped
        .audio_deletion_preview
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    let original = f.original(&operation);
    let leftover = f.audio.unregistered_audio();
    let pending = f.run(&stopped, Action::Refresh);
    let reviewed = f.run(&pending, Action::ReviewRemaining);
    let p = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.unregistered_audio_files, 1);
    assert!(p.execute_allowed && !p.keep_allowed);
    assert!(f
        .controller
        .begin_request(f.request(&reviewed, Action::Publish))
        .is_err());
    assert!(f
        .controller
        .begin_request(f.request(&reviewed, Action::Keep))
        .is_err());
    assert_eq!(f.count(), 1);
    let request = f.request(&reviewed, Action::DeleteIncludingUnregistered);
    let fixed = f.stop_at(
        &reviewed,
        Action::DeleteIncludingUnregistered,
        "audio_delete_scope_fixed",
    );
    let p = fixed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "files");
    assert_eq!(p.operation_id, operation);
    assert_eq!(p.include_unregistered_recorded, Some(true));
    assert!(!p.keep_allowed && !p.reconfirmation_allowed);
    assert!(f.audio.path.exists() && leftover.exists());
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert_eq!(f.count(), 2);
    assert_eq!(f.original(&operation), original);
    f.audio = f.audio.reopen();
    f.controller = DataController::default();
    f.analysis.require_storage_restart();
    let pending = f.pending(&operation);
    let p = pending.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.include_unregistered_recorded, Some(true));
    assert!(p.execute_allowed && !p.reconfirmation_allowed && !p.keep_allowed);
    assert!(f
        .controller
        .begin_request(f.request(&pending, Action::DeleteIncludingUnregistered))
        .is_err());
    let done = f.run(&pending, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(done.restart_required);
    assert_eq!(done.audio_deletion_receipt.unwrap().operation_id, operation);
    assert_eq!(f.count(), 2);
    assert_eq!(f.original(&operation), original);
    assert!(!f.audio.path.exists() && !leftover.exists());
    f.audio = f.audio.reopen();
    assert!(!f.audio.storage.data_write_blocked());
}

#[test]
fn audio_controller_remaining_stale_page_refresh_and_cancel_never_accept_new_bytes() {
    let f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "deleting_audio_files");
    std::fs::write(&f.audio.path, b"FIRST CHANGE").unwrap();
    let pending = f.run(&stopped, Action::Refresh);
    let cancelled = f.stop_at(
        &pending,
        Action::ReviewRemaining,
        "verifying_audio_delete_scope",
    );
    assert_eq!(f.count(), 1);
    assert!(f.audio.path.exists());
    let pending = f.run(&cancelled, Action::Refresh);
    let reviewed = f.run(&pending, Action::ReviewRemaining);
    let old = reviewed
        .audio_deletion_preview
        .as_ref()
        .unwrap()
        .confirmation_id
        .clone();
    let old_request = f.request(&reviewed, Action::Publish);
    let refreshed = f.run(&reviewed, Action::Refresh);
    assert_eq!(
        refreshed.audio_deletion_preview.as_ref().unwrap().phase,
        "reconfirmation"
    );
    assert_ne!(
        refreshed
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .confirmation_id,
        old
    );
    assert!(f.controller.audio_deletion_page(&old, 0).is_err());
    assert!(f.controller.begin_request(old_request).is_err());
    std::fs::write(&f.audio.path, b"SECOND UNCONFIRMED CHANGE").unwrap();
    let rejected = f.run(&refreshed, Action::Publish);
    assert!(rejected.error_message.is_some() && rejected.audio_deletion_receipt.is_none());
    assert_eq!(f.count(), 1);
    assert!(f.audio.path.exists() && f.audio.storage.data_write_blocked());
    let pending = f.run(&rejected, Action::Refresh);
    let reviewed = f.run(&pending, Action::ReviewRemaining);
    let done = f.run(&reviewed, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert_eq!(f.count(), 2);
}

#[test]
fn audio_controller_remaining_availability_reappearance_and_unsafe_links() {
    for unsafe_link in [false, true] {
        let f = Fixture::new();
        let stopped = f.stop_at(&f.ready(), Action::Publish, "fixing_audio_unavailable");
        assert!(!f.audio.path.exists());
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"OUTSIDE AUDIO MUST SURVIVE").unwrap();
        if unsafe_link {
            std::os::unix::fs::symlink(outside.path(), &f.audio.path).unwrap();
        } else {
            std::fs::write(&f.audio.path, b"RETURNED BEFORE AVAILABILITY").unwrap();
        }
        let pending = f.run(&stopped, Action::Refresh);
        let p = pending.audio_deletion_preview.as_ref().unwrap();
        assert_eq!(p.phase, "availability");
        assert!(!p.execute_allowed && !p.keep_allowed);
        assert_eq!(p.reconfirmation_allowed, !unsafe_link);
        if unsafe_link {
            assert!(f
                .controller
                .begin_request(f.request(&pending, Action::ReviewRemaining))
                .is_err());
            assert_eq!(f.count(), 1);
        } else {
            let reviewed = f.run(&pending, Action::ReviewRemaining);
            let done = f.run(&reviewed, Action::Publish);
            assert_eq!(done.phase, "completed", "{:?}", done.error_message);
            assert_eq!(f.count(), 2);
            assert!(!f.audio.path.exists());
        }
        assert_eq!(
            std::fs::read(outside.path()).unwrap(),
            b"OUTSIDE AUDIO MUST SURVIVE"
        );
    }
}

#[test]
fn audio_controller_cold_restart_keep_requires_native_admission_and_another_restart() {
    let mut f = Fixture::new();
    let stopped = f.stop_at(&f.ready(), Action::Publish, "audio_delete_scope_fixed");
    let operation = stopped.audio_deletion_preview.unwrap().operation_id;
    f.audio = f.audio.reopen();
    f.controller = DataController::default();
    f.analysis.require_storage_restart();
    assert!(f.audio.storage.data_write_blocked());
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    let pending = f.pending(&operation);
    assert!(pending.audio_deletion_preview.as_ref().unwrap().pending);
    let done = f.run(&pending, Action::Keep);
    assert!(done.restart_required && done.audio_deletion_receipt.unwrap().retained);
    assert!(f.audio.path.exists() && f.audio.storage.restart_required());
    f.audio = f.audio.reopen();
    assert!(!f.audio.storage.restart_required() && !f.audio.storage.data_write_blocked());
    assert!(f.audio.path.exists());
}
#[test]
fn audio_controller_cold_restart_continues_saved_audio_policy_without_reaccepting_scope() {
    let mut f = Fixture::new();
    let leftover = f.audio.unregistered_audio();
    let stopped = f.stop_at(
        &f.ready(),
        Action::DeleteIncludingUnregistered,
        "audio_delete_scope_fixed",
    );
    let operation = stopped.audio_deletion_preview.unwrap().operation_id;
    f.audio = f.audio.reopen();
    f.controller = DataController::default();
    f.analysis.require_storage_restart();
    let pending = f.pending(&operation);
    assert_eq!(
        pending
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(true)
    );
    assert!(f
        .controller
        .begin_request(f.request(&pending, Action::DeleteIncludingUnregistered))
        .is_err());
    let done = f.run(&pending, Action::Publish);
    assert!(done.restart_required && done.audio_deletion_receipt.unwrap().audio_deleted);
    assert!(!f.audio.path.exists() && !leftover.exists());
    assert_eq!(f.count(), 1);
    f.audio = f.audio.reopen();
    assert!(!f.audio.storage.restart_required() && !f.audio.storage.data_write_blocked());
}
#[test]
fn audio_controller_completed_receipt_can_be_acknowledged_without_another_consent() {
    let mut f = Fixture::new();
    let first = f.run(&f.ready(), Action::Publish);
    let operation = first.audio_deletion_receipt.unwrap().operation_id;
    f.audio = f.audio.reopen();
    f.controller = DataController::default();
    let pending = f.pending(&operation);
    let p = pending.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "awaiting_ack");
    assert!(p.execute_allowed && !p.reconfirmation_allowed);
    let done = f.run(&pending, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted);
    assert_eq!(f.count(), 1);
}
#[test]
fn audio_controller_cancelled_lease_is_not_reused_after_exit_is_aborted() {
    let f = Fixture::new();
    let lease = f.lease(None);
    let mut prepared =
        audio_deletion::Prepared::fresh(&f.audio.storage, &f.audio.meeting, &lease, &|| false)
            .unwrap();
    let confirmation = prepared.preview().confirmation_id;
    f.analysis.begin_exit().unwrap();
    f.analysis.abort_shutdown();
    assert!(lease.cancelled());
    assert!(prepared
        .refresh(&f.audio.storage, &lease, &|| false)
        .is_err());
    assert!(prepared
        .execute(
            &f.audio.storage,
            &lease,
            &confirmation,
            false,
            &|| false,
            &|_| {}
        )
        .is_err());
    assert!(f.audio.path.exists());
    assert_eq!(f.count(), 0);
}

#[test]
fn audio_controller_malformed_and_oversized_records_remain_visible_without_authority() {
    for header in [
        "{".into(),
        "x".repeat(65537),
        "{\"subtype\":\"audioOnlyDeletion\"}".into(),
    ] {
        let f = Fixture::new();
        let stopped = f.stop_at(&f.ready(), Action::Publish, "audio_delete_scope_fixed");
        let operation = stopped.audio_deletion_preview.unwrap().operation_id;
        f.audio.corrupt_operation_header(&operation, &header);
        let audio_list = audio_deletion::listing(&f.audio.storage).unwrap();
        assert_eq!(audio_list.items.len(), 1);
        assert!(audio_list.write_blocked && !audio_list.items[0].review_allowed);
        assert!(
            audio_list.items[0].meeting_id.is_none() && audio_list.items[0].error_message.is_some()
        );
        let model_list =
            model_deletion::listing(&f.audio.storage, &f.audio.storage.root().join("models"))
                .unwrap();
        assert!(model_list.write_blocked);
        if header.contains("audioOnlyDeletion") {
            assert!(model_list.items.is_empty());
        } else {
            assert_eq!(model_list.items.len(), 1);
            assert!(!model_list.items[0].review_allowed);
        }
        assert!(f.audio.path.exists() && f.audio.storage.data_write_blocked());
        assert!(f.audio.storage.create_meeting("MUST NOT WRITE").is_err());
        assert!(f
            .analysis
            .begin_audio_deletion_maintenance(&f.audio.storage, &operation, false, false, &|| false)
            .is_err());
    }
}
