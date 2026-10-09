//! Real private PCM, Storage transactions and live shared maintenance ownership.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::recordings::{partial_test_fixture, PartialTestFixture};

struct Fixture {
    audio: PartialTestFixture,
    controller: DataController,
    analysis: AnalysisController,
}
impl Fixture {
    fn new() -> Self {
        Self {
            audio: partial_test_fixture(),
            controller: DataController::default(),
            analysis: AnalysisController::default(),
        }
    }
    fn lease(&self) -> DataMaintenanceLease {
        self.analysis
            .begin_data_maintenance(false, false, &|| false)
            .unwrap()
    }
    fn ready(&self) -> Snapshot {
        let lease = self.lease();
        let (s, _, previous) = self.controller.begin(Kind::PartialRecording).unwrap();
        self.controller
            .finish(
                s.token.as_ref().unwrap(),
                Work::PartialRecording(self.audio.session.id.clone(), previous).run_admitted(
                    &self.audio.storage,
                    &|| false,
                    &|_| {},
                    Some(&lease),
                ),
            )
            .unwrap()
    }
    fn request(&self, s: &Snapshot, action: Action) -> Request {
        Request {
            token: s.token.clone().unwrap(),
            confirmation_id: s
                .partial_recording_preview
                .as_ref()
                .unwrap()
                .confirmation_id
                .clone(),
            action,
        }
    }
    fn run(&self, s: &Snapshot, action: Action) -> Snapshot {
        let lease = self.lease();
        let (work, _, start) = self
            .controller
            .begin_request(self.request(s, action))
            .unwrap()
            .unwrap();
        self.controller
            .finish(
                start.token.as_ref().unwrap(),
                work.run_admitted(&self.audio.storage, &|| false, &|_| {}, Some(&lease)),
            )
            .unwrap()
    }
    fn registered(&self) -> i64 {
        self.audio.registered_tracks()
    }
}

#[test]
fn partial_controller_review_is_read_only_server_owned_and_discoverable() {
    let f = Fixture::new();
    let before = std::fs::read_dir(&f.audio.base).unwrap().count();
    let listing = f.audio.storage.partial_recording_listing(None).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].session_id, f.audio.session.id);
    assert!(listing.next.is_none());
    assert!(f
        .audio
        .storage
        .partial_recording_listing(Some("../bad"))
        .is_err());
    let s = f.ready();
    assert_eq!(s.phase, "ready");
    assert!(!s.busy && s.retry_allowed);
    let p = s.partial_recording_preview.unwrap();
    assert_eq!(p.summary.duration_us, 5_000_000);
    assert!(!p.summary.acquisition_end_known && p.summary.tail_loss_us.is_none());
    assert_eq!(f.registered(), 0);
    assert_eq!(std::fs::read_dir(&f.audio.base).unwrap().count(), before);
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("raw-microphone") && !json.contains("configText"));
}

#[test]
fn partial_controller_requires_actual_shared_admission_and_blocks_capture_import_and_work() {
    let f = Fixture::new();
    let result = Work::PartialRecording(f.audio.session.id.clone(), None).run_admitted(
        &f.audio.storage,
        &|| false,
        &|_| {},
        None,
    );
    assert!(result.error.is_some() && result.selection.is_none());
    assert!(f
        .analysis
        .begin_data_maintenance(true, false, &|| false)
        .is_err());
    assert!(f
        .analysis
        .begin_data_maintenance(false, true, &|| false)
        .is_err());
    let lease = f.lease();
    assert!(f
        .analysis
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    drop(lease);
    assert_eq!(f.registered(), 0);
}

#[test]
fn partial_controller_rejects_stale_tokens_confirmations_and_non_recovery_actions() {
    let f = Fixture::new();
    let s = f.ready();
    for action in [
        Action::Publish,
        Action::DeleteIncludingUnregistered,
        Action::ReviewRemaining,
    ] {
        assert!(f.controller.begin_request(f.request(&s, action)).is_err());
    }
    let mut r = f.request(&s, Action::Recover);
    r.token = Uuid::new_v4().to_string();
    assert!(f.controller.begin_request(r).is_err());
    let mut r = f.request(&s, Action::Recover);
    r.confirmation_id = Uuid::new_v4().to_string();
    assert!(f.controller.begin_request(r).is_err());
    assert_eq!(f.registered(), 0);
}

#[test]
fn partial_controller_lost_refresh_is_replayable_but_old_consent_cannot_publish_new_review() {
    let f = Fixture::new();
    let s = f.ready();
    let request = f.request(&s, Action::Refresh);
    let next = f.run(&s, Action::Refresh);
    assert_eq!(next.phase, "ready");
    assert_ne!(
        s.partial_recording_preview
            .as_ref()
            .unwrap()
            .confirmation_id,
        next.partial_recording_preview
            .as_ref()
            .unwrap()
            .confirmation_id
    );
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert!(f
        .controller
        .begin_request(f.request(&s, Action::Recover))
        .is_err());
    assert_eq!(f.registered(), 0);
}

#[test]
fn partial_controller_changed_source_fails_and_requires_new_review_not_automatic_retry() {
    let f = Fixture::new();
    let s = f.ready();
    let meeting = f
        .audio
        .storage
        .get_meeting(&f.audio.session.meeting_id)
        .unwrap()
        .unwrap();
    f.audio
        .storage
        .rename_meeting(&meeting.id, "確認後の変更", meeting.lock_version)
        .unwrap();
    let failed = f.run(&s, Action::Recover);
    assert_eq!(failed.phase, "failed");
    assert_eq!(f.registered(), 0);
    assert!(f
        .controller
        .begin_request(f.request(&failed, Action::Recover))
        .is_err());
    let fresh = f.run(&failed, Action::Refresh);
    assert_eq!(fresh.phase, "ready");
    assert_ne!(
        s.partial_recording_preview.unwrap().summary.review_hash,
        fresh
            .partial_recording_preview
            .as_ref()
            .unwrap()
            .summary
            .review_hash
    );
    assert_eq!(f.run(&fresh, Action::Recover).phase, "completed");
}

#[test]
fn partial_controller_commit_wins_late_cancel_and_same_request_replays_without_new_files() {
    let f = Fixture::new();
    let s = f.ready();
    let request = f.request(&s, Action::Recover);
    let lease = f.lease();
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
    let result = work.run_admitted(&f.audio.storage, &|| false, &|_| {}, Some(&lease));
    cancel.store(true, Ordering::Release);
    let done = f
        .controller
        .finish(started.token.as_ref().unwrap(), result)
        .unwrap();
    assert_eq!(done.phase, "completed");
    assert!(!done.busy && !done.retry_allowed);
    let receipt = done.partial_recording_receipt.as_ref().unwrap();
    assert!(receipt.recovered);
    assert_eq!(f.registered(), 2);
    assert!(f.controller.begin_request(request).unwrap().is_none());
    assert_eq!(f.registered(), 2);
    assert_eq!(
        f.audio
            .storage
            .partial_recovery_result(&receipt.summary.session_id, &receipt.summary.review_hash)
            .unwrap(),
        Some(receipt.summary.clone())
    );
    assert!(f
        .audio
        .storage
        .partial_recording_listing(None)
        .unwrap()
        .items
        .is_empty());
}

#[test]
fn partial_controller_panic_after_real_commit_reads_durable_result_not_failed_or_second_publication(
) {
    let f = Fixture::new();
    let s = f.ready();
    partial_recording::PANIC_AFTER_COMMIT.with(|v| v.set(true));
    let done = f.run(&s, Action::Recover);
    assert_eq!(done.phase, "completed");
    assert!(done.error_message.is_none() && !done.attention_required);
    assert_eq!(f.registered(), 2);
    assert!(done.partial_recording_receipt.unwrap().recovered);
}

#[test]
fn partial_controller_keep_and_cancel_never_adopt_or_remove_audio() {
    let f = Fixture::new();
    let before = std::fs::read_dir(&f.audio.base).unwrap().count();
    let s = f.ready();
    let done = f.run(&s, Action::Keep);
    assert_eq!(done.phase, "completed");
    assert!(!done.partial_recording_receipt.unwrap().recovered);
    assert_eq!(f.registered(), 0);
    assert_eq!(std::fs::read_dir(&f.audio.base).unwrap().count(), before);
    let s = f.ready();
    let lease = f.lease();
    let (work, cancel, start) = f
        .controller
        .begin_request(f.request(&s, Action::Recover))
        .unwrap()
        .unwrap();
    cancel.store(true, Ordering::Release);
    let completion = work.run_admitted(
        &f.audio.storage,
        &|| cancel.load(Ordering::Acquire),
        &|_| {},
        Some(&lease),
    );
    let done = f
        .controller
        .finish(start.token.as_ref().unwrap(), completion)
        .unwrap();
    assert_eq!(done.phase, "cancelled");
    assert_eq!(f.registered(), 0);
    assert!(f.audio.base.join("raw-system-00000003.caf").is_file());
}

#[test]
fn partial_controller_db_failure_preserves_checkpoint_and_new_consent_resumes_same_ids() {
    let f = Fixture::new();
    let s = f.ready();
    f.audio.reject_partial_commit(true);
    let failed = f.run(&s, Action::Recover);
    assert_eq!(failed.phase, "failed");
    assert_eq!(f.registered(), 0);
    let plan = std::fs::read(f.audio.base.join("partial-flac.json")).unwrap();
    assert!(f
        .controller
        .begin_request(f.request(&failed, Action::Recover))
        .is_err());
    f.audio.reject_partial_commit(false);
    let fresh = f.run(&failed, Action::Refresh);
    assert_eq!(f.run(&fresh, Action::Recover).phase, "completed");
    assert_eq!(
        std::fs::read(f.audio.base.join("partial-flac.json")).unwrap(),
        plan
    );
}

#[test]
fn partial_controller_cancel_after_flac_publication_keeps_audio_no_db_and_can_resume() {
    let f = Fixture::new();
    let s = f.ready();
    let lease = f.lease();
    let (work, _, start) = f
        .controller
        .begin_request(f.request(&s, Action::Recover))
        .unwrap()
        .unwrap();
    let check = || {
        let parent = f.audio.base.parent().unwrap();
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .any(|dir| {
                dir.path() != f.audio.base
                    && std::fs::read_dir(dir.path()).is_ok_and(|mut files| {
                        files.any(|file| {
                            file.is_ok_and(|f| f.path().extension().is_some_and(|e| e == "flac"))
                        })
                    })
            })
    };
    let done = f
        .controller
        .finish(
            start.token.as_ref().unwrap(),
            work.run_admitted(&f.audio.storage, &check, &|_| {}, Some(&lease)),
        )
        .unwrap();
    assert_eq!(done.phase, "cancelled");
    assert_eq!(f.registered(), 0);
    assert!(f.audio.base.join("partial-flac.json").is_file());
    drop(lease);
    let fresh = f.run(&done, Action::Refresh);
    assert_eq!(f.run(&fresh, Action::Recover).phase, "completed");
    assert!(f.audio.base.join("raw-microphone-00000001.caf").is_file());
}

#[test]
fn partial_controller_durable_result_reopens_without_minting_original_recovery_authority() {
    let f = Fixture::new();
    let s = f.ready();
    let done = f.run(&s, Action::Recover);
    let summary = done.partial_recording_receipt.unwrap().summary;
    let Fixture {
        audio,
        controller: _,
        analysis: _,
    } = f;
    let PartialTestFixture {
        temp,
        storage,
        session: _,
        base: _,
    } = audio;
    drop(storage);
    let reopened = Storage::open(temp.path().to_owned()).unwrap();
    assert_eq!(
        reopened
            .partial_recovery_result(&summary.session_id, &summary.review_hash)
            .unwrap(),
        Some(summary.clone())
    );
    assert!(reopened
        .review_partial_recording(&summary.session_id)
        .is_err());
    assert!(reopened
        .partial_recovery_result(&summary.session_id, &"f".repeat(64))
        .unwrap()
        .is_none());
    assert!(reopened
        .partial_recovery_result(&summary.session_id, "bad")
        .is_err());
    assert!(reopened
        .partial_recording_listing(None)
        .unwrap()
        .items
        .is_empty());
}
