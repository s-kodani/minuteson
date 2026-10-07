use crate::session::{RecordingPhase, RecordingSnapshot};
use serde::Serialize;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPhase {
    Idle,
    Confirming,
    Stopping,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopDecision {
    pub revision: u64,
    pub id: Option<String>,
    pub phase: DecisionPhase,
    pub session_id: Option<String>,
    pub meeting_id: Option<String>,
    pub meeting_title: Option<String>,
    pub origin: Option<String>,
    pub preparing: bool,
    pub quit_after_save: bool,
    pub error_message: Option<String>,
}
impl Default for StopDecision {
    fn default() -> Self {
        Self {
            revision: 0,
            id: None,
            phase: DecisionPhase::Idle,
            session_id: None,
            meeting_id: None,
            meeting_title: None,
            origin: None,
            preparing: false,
            quit_after_save: false,
            error_message: None,
        }
    }
}
impl StopDecision {
    pub fn visible(&self) -> bool {
        matches!(
            self.phase,
            DecisionPhase::Confirming | DecisionPhase::Stopping | DecisionPhase::Failed
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderCloseAction {
    Hide,
    Keep,
    FocusConfirmation,
    RequestConfirmation,
}

// Run inside the recorder lifecycle lock. An OS event can wait behind a
// transition, but it must not hide or stop the next recording it did not name.
pub fn recorder_close_action(
    expected: &RecordingSnapshot,
    current: &RecordingSnapshot,
    decision: &StopDecision,
) -> Result<RecorderCloseAction, String> {
    if current.revision < expected.revision
        || current.phase != RecordingPhase::Idle
            && (current.session_id != expected.session_id
                || current.meeting_id != expected.meeting_id)
        || current.is_active() && (current.session_id.is_none() || current.meeting_id.is_none())
    {
        return Err("録音状態が変更されています。別の録音は閉じたり停止したりしていません".into());
    }
    if decision.visible() {
        return Ok(RecorderCloseAction::FocusConfirmation);
    }
    match current.phase {
        RecordingPhase::Idle | RecordingPhase::Error => Ok(RecorderCloseAction::Hide),
        RecordingPhase::Stopping | RecordingPhase::Saving => Ok(RecorderCloseAction::Keep),
        RecordingPhase::Preparing | RecordingPhase::Recording => {
            if !matches!(
                expected.phase,
                RecordingPhase::Preparing | RecordingPhase::Recording
            ) {
                return Err(
                    "録音の状態が変更されています。最新の画面から再確認してください".into(),
                );
            }
            Ok(RecorderCloseAction::RequestConfirmation)
        }
    }
}

#[derive(Default)]
struct DecisionState {
    decision: StopDecision,
    recording_revision: u64,
    quit_notified: bool,
}
#[derive(Default)]
pub struct StopDecisionStore(Mutex<DecisionState>);
impl StopDecisionStore {
    pub fn snapshot(&self) -> StopDecision {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decision
            .clone()
    }
    pub fn request(
        &self,
        recording: &RecordingSnapshot,
        origin: &str,
        title: String,
    ) -> Result<(StopDecision, bool), String> {
        self.request_for_exit(recording, origin, title, false)
    }
    pub fn request_for_exit(
        &self,
        recording: &RecordingSnapshot,
        origin: &str,
        title: String,
        quit_after_save: bool,
    ) -> Result<(StopDecision, bool), String> {
        if !matches!(origin, "main" | "recorder") {
            return Err("録音画面から確認してください".into());
        }
        if quit_after_save && origin != "main" {
            return Err("アプリ終了はメイン画面から確認してください".into());
        }
        let mut state = self
            .0
            .lock()
            .map_err(|_| "録音確認のロックに失敗しました")?;
        if !state.decision.visible() && recording.revision < state.recording_revision {
            return Err("録音状態が更新されています。最新の状態で再確認してください".into());
        }
        state.recording_revision = state.recording_revision.max(recording.revision);
        let value = &mut state.decision;
        if value.visible() {
            return Ok((value.clone(), false));
        }
        if !matches!(
            recording.phase,
            RecordingPhase::Preparing | RecordingPhase::Recording
        ) || recording.session_id.is_none()
            || recording.meeting_id.is_none()
        {
            return Err("停止できる録音を再確認してください".into());
        }
        *value = StopDecision {
            revision: value.revision.saturating_add(1),
            id: Some(Uuid::new_v4().to_string()),
            phase: DecisionPhase::Confirming,
            session_id: recording.session_id.clone(),
            meeting_id: recording.meeting_id.clone(),
            meeting_title: Some(title),
            origin: Some(origin.into()),
            preparing: recording.phase == RecordingPhase::Preparing,
            quit_after_save,
            error_message: None,
        };
        let result = value.clone();
        state.quit_notified = false;
        Ok((result, true))
    }
    // A completion notification continues closing, not stopping or saving.
    // Repeated observations never replay that continuation or upgrade consent.
    pub fn take_completed_quit(&self, id: &str) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.decision.id.as_deref() != Some(id)
            || state.decision.phase != DecisionPhase::Completed
            || !state.decision.quit_after_save
            || state.quit_notified
        {
            return false;
        }
        state.quit_notified = true;
        true
    }
    pub fn claim(
        &self,
        id: &str,
        preparing: bool,
        recording: &RecordingSnapshot,
    ) -> Result<String, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "録音確認のロックに失敗しました")?;
        if recording.revision < state.recording_revision {
            return Err("録音状態が更新されています。停止は開始していません".into());
        }
        state.recording_revision = recording.revision;
        let value = &mut state.decision;
        if value.id.as_deref() != Some(id) || value.phase != DecisionPhase::Confirming {
            return Err(
                "この録音確認は既に確定または変更されています。停止は再送していません".into(),
            );
        }
        if value.session_id != recording.session_id
            || value.meeting_id != recording.meeting_id
            || !matches!(
                recording.phase,
                RecordingPhase::Preparing | RecordingPhase::Recording
            )
        {
            return Err("録音対象が変更されました。停止は開始していません".into());
        }
        if preparing != (recording.phase == RecordingPhase::Preparing) {
            return Err(
                "録音準備の状態が変わりました。更新された本文を確認してから決定してください".into(),
            );
        }
        value.phase = DecisionPhase::Stopping;
        value.revision = value.revision.saturating_add(1);
        Ok(value
            .session_id
            .clone()
            .expect("confirmed recording identity"))
    }
    pub fn dismiss(&self, id: &str) -> Result<StopDecision, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "録音確認のロックに失敗しました")?;
        let value = &mut state.decision;
        if value.id.as_deref() != Some(id)
            || !matches!(
                value.phase,
                DecisionPhase::Confirming | DecisionPhase::Failed
            )
        {
            return Err("停止・保存中は確認を閉じられません".into());
        }
        value.phase = DecisionPhase::Cancelled;
        value.revision = value.revision.saturating_add(1);
        Ok(value.clone())
    }
    pub fn fail(&self, id: &str, error: String) -> StopDecision {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let value = &mut state.decision;
        if value.id.as_deref() == Some(id) && value.phase == DecisionPhase::Stopping {
            value.phase = DecisionPhase::Failed;
            value.error_message = Some(error);
            value.revision = value.revision.saturating_add(1);
        }
        value.clone()
    }
    pub fn observe(&self, recording: &RecordingSnapshot) -> StopDecision {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // A read that started before a newer session event must not undo that
        // event or complete a decision for a subsequent recording.
        if recording.revision < state.recording_revision {
            return state.decision.clone();
        }
        state.recording_revision = recording.revision;
        let value = &mut state.decision;
        if !matches!(
            value.phase,
            DecisionPhase::Confirming | DecisionPhase::Stopping
        ) {
            return value.clone();
        }
        let previous = value.clone();
        if recording.phase == RecordingPhase::Idle && recording.session_id.is_none() {
            value.phase = DecisionPhase::Completed;
        } else if value.session_id != recording.session_id
            || value.meeting_id != recording.meeting_id
        {
            value.phase = DecisionPhase::Failed;
            value.error_message =
                Some("録音対象が変更されました。別の録音には停止を送っていません".into());
        } else if recording.phase == RecordingPhase::Error {
            value.phase = DecisionPhase::Failed;
            value.error_message = recording
                .error_message
                .clone()
                .or(Some("録音の保存状態を確認してください".into()));
        } else {
            value.preparing = recording.phase == RecordingPhase::Preparing;
            if matches!(
                recording.phase,
                RecordingPhase::Stopping | RecordingPhase::Saving
            ) {
                value.phase = DecisionPhase::Stopping;
            }
        }
        if *value != previous {
            value.revision = value.revision.saturating_add(1);
        }
        value.clone()
    }
}

// Physical work-area coordinates include monitors left/above the primary one.
pub fn centered_position(origin: (i32, i32), area: (u32, u32), window: (u32, u32)) -> (i32, i32) {
    let coordinate = |start: i32, extent: u32, size: u32| {
        (i64::from(start) + i64::from(extent.saturating_sub(size)) / 2)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    (
        coordinate(origin.0, area.0, window.0),
        coordinate(origin.1, area.1, window.1),
    )
}

pub fn fitted_size(area: (u32, u32), scale: f64) -> Result<(u32, u32), String> {
    if area.0 == 0 || area.1 == 0 || !scale.is_finite() || scale <= 0.0 {
        return Err("確認用の画面領域を検証できません".into());
    }
    Ok((
        ((560.0 * scale).round() as u32).max(1).min(area.0),
        ((460.0 * scale).round() as u32).max(1).min(area.1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorder_os_close_respects_every_recording_phase_without_stopping() {
        for (phase, action) in [
            (RecordingPhase::Idle, RecorderCloseAction::Hide),
            (RecordingPhase::Error, RecorderCloseAction::Hide),
            (
                RecordingPhase::Preparing,
                RecorderCloseAction::RequestConfirmation,
            ),
            (
                RecordingPhase::Recording,
                RecorderCloseAction::RequestConfirmation,
            ),
            (RecordingPhase::Stopping, RecorderCloseAction::Keep),
            (RecordingPhase::Saving, RecorderCloseAction::Keep),
        ] {
            let state = if phase == RecordingPhase::Idle {
                RecordingSnapshot::default()
            } else {
                RecordingSnapshot {
                    phase,
                    ..recording()
                }
            };
            assert_eq!(
                recorder_close_action(&state, &state, &StopDecision::default()).unwrap(),
                action
            );
        }
    }
    #[test]
    fn recorder_os_close_reuses_pending_confirmation_and_never_changes_quit_intent() {
        let store = StopDecisionStore::default();
        let (value, _) = store
            .request_for_exit(&recording(), "main", "A".into(), true)
            .unwrap();
        for phase in [
            DecisionPhase::Confirming,
            DecisionPhase::Stopping,
            DecisionPhase::Failed,
        ] {
            let value = StopDecision {
                phase,
                ..value.clone()
            };
            assert_eq!(
                recorder_close_action(&recording(), &recording(), &value).unwrap(),
                RecorderCloseAction::FocusConfirmation
            );
        }
        assert_eq!(store.snapshot(), value);
    }
    #[test]
    fn queued_recorder_os_close_rejects_new_sessions_and_stale_reads() {
        let expected = recording();
        let next = RecordingSnapshot {
            revision: expected.revision + 1,
            session_id: Some("new-session".into()),
            ..expected.clone()
        };
        assert!(recorder_close_action(&expected, &next, &StopDecision::default()).is_err());
        let next = RecordingSnapshot {
            meeting_id: Some("other-meeting".into()),
            ..expected.clone()
        };
        assert!(recorder_close_action(&expected, &next, &StopDecision::default()).is_err());
        let expected = RecordingSnapshot {
            revision: 2,
            ..expected
        };
        assert!(recorder_close_action(&expected, &recording(), &StopDecision::default()).is_err());
        assert!(recorder_close_action(
            &RecordingSnapshot::default(),
            &recording(),
            &StopDecision::default()
        )
        .is_err());
    }
    #[test]
    fn queued_recorder_os_close_follows_same_session_preparation_and_actual_completion() {
        let preparing = RecordingSnapshot {
            phase: RecordingPhase::Preparing,
            ..recording()
        };
        assert_eq!(
            recorder_close_action(&preparing, &recording(), &StopDecision::default()).unwrap(),
            RecorderCloseAction::RequestConfirmation
        );
        let saving = RecordingSnapshot {
            phase: RecordingPhase::Saving,
            ..recording()
        };
        assert_eq!(
            recorder_close_action(&preparing, &saving, &StopDecision::default()).unwrap(),
            RecorderCloseAction::Keep
        );
        let idle = RecordingSnapshot {
            revision: saving.revision + 1,
            ..RecordingSnapshot::default()
        };
        assert_eq!(
            recorder_close_action(&saving, &idle, &StopDecision::default()).unwrap(),
            RecorderCloseAction::Hide
        );
        let broken = RecordingSnapshot {
            session_id: None,
            ..recording()
        };
        assert!(recorder_close_action(&broken, &broken, &StopDecision::default()).is_err());
    }
    #[test]
    fn quit_requires_explicit_consent_and_only_continues_once_after_actual_save() {
        let store = StopDecisionStore::default();
        assert!(store
            .request_for_exit(&recording(), "recorder", "A".into(), true)
            .is_err());
        let (stop, _) = store.request(&recording(), "recorder", "A".into()).unwrap();
        let (unchanged, created) = store
            .request_for_exit(&recording(), "main", "A".into(), true)
            .unwrap();
        assert!(!created && !unchanged.quit_after_save);
        assert_eq!(stop.id, unchanged.id);
        store.dismiss(stop.id.as_ref().unwrap()).unwrap();
        let (quit, _) = store
            .request_for_exit(&recording(), "main", "A".into(), true)
            .unwrap();
        let id = quit.id.as_ref().unwrap();
        assert!(!store.take_completed_quit(id));
        store.claim(id, false, &recording()).unwrap();
        assert!(!store.take_completed_quit(id));
        store.observe(&RecordingSnapshot {
            phase: RecordingPhase::Saving,
            ..recording()
        });
        assert!(!store.take_completed_quit(id));
        store.observe(&RecordingSnapshot::default());
        assert!(store.take_completed_quit(id));
        assert!(!store.take_completed_quit(id));
        assert!(!store.take_completed_quit(stop.id.as_ref().unwrap()));
    }
    #[test]
    fn cancelled_and_failed_recording_quit_never_continue_closing() {
        let store = StopDecisionStore::default();
        let (quit, _) = store
            .request_for_exit(&recording(), "main", "A".into(), true)
            .unwrap();
        let id = quit.id.as_ref().unwrap();
        store.dismiss(id).unwrap();
        assert!(!store.take_completed_quit(id));
        let (quit, _) = store
            .request_for_exit(&recording(), "main", "A".into(), true)
            .unwrap();
        let id = quit.id.as_ref().unwrap();
        store.claim(id, false, &recording()).unwrap();
        store.fail(id, "保存失敗".into());
        assert!(!store.take_completed_quit(id));
    }
    #[test]
    fn stale_recording_reads_cannot_revert_consent_or_complete_next_recording() {
        let store = StopDecisionStore::default();
        let mut preparing = recording();
        preparing.phase = RecordingPhase::Preparing;
        preparing.revision = 10;
        let (decision, _) = store.request(&preparing, "recorder", "A".into()).unwrap();
        let mut started = preparing.clone();
        started.phase = RecordingPhase::Recording;
        started.revision = 11;
        let current = store.observe(&started);
        assert!(!current.preparing);
        assert_eq!(store.observe(&preparing), current);
        assert!(store
            .claim(decision.id.as_ref().unwrap(), true, &preparing)
            .is_err());
        store
            .claim(decision.id.as_ref().unwrap(), false, &started)
            .unwrap();
        let idle = RecordingSnapshot {
            revision: 12,
            ..Default::default()
        };
        assert_eq!(store.observe(&idle).phase, DecisionPhase::Completed);
        assert!(store.request(&started, "main", "A".into()).is_err());
        let next = RecordingSnapshot {
            revision: 13,
            session_id: Some("session-b".into()),
            ..started
        };
        let (decision, _) = store.request(&next, "main", "B".into()).unwrap();
        assert_eq!(store.observe(&idle), decision);
        assert_eq!(store.snapshot().phase, DecisionPhase::Confirming);
    }
    #[test]
    fn concurrent_windows_create_and_accept_only_once() {
        let store = std::sync::Arc::new(StopDecisionStore::default());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles = (0..16)
            .map(|i| {
                let s = store.clone();
                let b = barrier.clone();
                std::thread::spawn(move || {
                    b.wait();
                    s.request(
                        &recording(),
                        if i % 2 == 0 { "main" } else { "recorder" },
                        "A".into(),
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let values = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.iter().filter(|v| v.1).count(), 1);
        assert!(values.iter().all(|v| v.0.id == values[0].0.id));
        let id = values[0].0.id.clone().unwrap();
        let handles = (0..16)
            .map(|_| {
                let s = store.clone();
                let b = barrier.clone();
                let id = id.clone();
                std::thread::spawn(move || {
                    b.wait();
                    s.claim(&id, false, &recording()).is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|won| *won)
                .count(),
            1
        );
    }
    #[test]
    fn window_size_uses_target_dpi_and_fits_small_work_area() {
        assert_eq!(fitted_size((1920, 1080), 2.0).unwrap(), (1120, 920));
        assert_eq!(fitted_size((800, 600), 2.0).unwrap(), (800, 600));
        assert!(fitted_size((0, 600), 1.0).is_err());
        assert!(fitted_size((800, 600), f64::NAN).is_err());
    }
    fn recording() -> RecordingSnapshot {
        RecordingSnapshot {
            phase: RecordingPhase::Recording,
            session_id: Some("session-a".into()),
            meeting_id: Some("meeting-a".into()),
            ..Default::default()
        }
    }
    #[test]
    fn simultaneous_windows_share_one_decision_and_first_origin() {
        let s = StopDecisionStore::default();
        let (a, created) = s.request(&recording(), "recorder", "会議A".into()).unwrap();
        let (b, again) = s.request(&recording(), "main", "会議A".into()).unwrap();
        assert!(created);
        assert!(!again);
        assert_eq!(a, b);
        assert_eq!(b.origin.as_deref(), Some("recorder"));
    }
    #[test]
    fn claim_is_exactly_once_and_cannot_be_cancelled_while_saving() {
        let s = StopDecisionStore::default();
        let (a, _) = s.request(&recording(), "main", "A".into()).unwrap();
        let id = a.id.unwrap();
        assert_eq!(s.claim(&id, false, &recording()).unwrap(), "session-a");
        assert!(s.claim(&id, false, &recording()).is_err());
        assert!(s.dismiss(&id).is_err());
    }
    #[test]
    fn decline_does_not_stop_and_old_identity_cannot_claim_next_decision() {
        let s = StopDecisionStore::default();
        let (a, _) = s.request(&recording(), "main", "A".into()).unwrap();
        let id = a.id.unwrap();
        assert_eq!(s.dismiss(&id).unwrap().phase, DecisionPhase::Cancelled);
        let (b, _) = s.request(&recording(), "main", "A".into()).unwrap();
        assert_ne!(b.id.as_deref(), Some(id.as_str()));
        assert!(s.claim(&id, false, &recording()).is_err());
    }
    #[test]
    fn elapsed_updates_do_not_invalidate_but_new_session_does() {
        let s = StopDecisionStore::default();
        let (a, _) = s.request(&recording(), "main", "A".into()).unwrap();
        let mut r = recording();
        r.revision = 22;
        r.elapsed_ms = 5000;
        assert_eq!(s.observe(&r), a);
        r.session_id = Some("session-b".into());
        assert!(s.claim(a.id.as_ref().unwrap(), false, &r).is_err());
        assert_eq!(s.observe(&r).phase, DecisionPhase::Failed);
    }
    #[test]
    fn preparation_transition_requires_updated_consent() {
        let s = StopDecisionStore::default();
        let mut r = recording();
        r.phase = RecordingPhase::Preparing;
        let (a, _) = s.request(&r, "main", "A".into()).unwrap();
        r.phase = RecordingPhase::Recording;
        assert!(!s.observe(&r).preparing);
        assert!(s.claim(a.id.as_ref().unwrap(), true, &r).is_err());
        assert!(s.claim(a.id.as_ref().unwrap(), false, &r).is_ok());
    }
    #[test]
    fn stop_acceptance_and_saving_do_not_mean_completed() {
        let s = StopDecisionStore::default();
        let (a, _) = s.request(&recording(), "main", "A".into()).unwrap();
        s.claim(a.id.as_ref().unwrap(), false, &recording())
            .unwrap();
        let mut r = recording();
        r.phase = RecordingPhase::Saving;
        assert_eq!(s.observe(&r).phase, DecisionPhase::Stopping);
        assert_eq!(
            s.observe(&RecordingSnapshot::default()).phase,
            DecisionPhase::Completed
        );
    }
    #[test]
    fn failed_save_remains_failed_and_late_errors_cannot_touch_next_decision() {
        let s = StopDecisionStore::default();
        let (a, _) = s.request(&recording(), "main", "A".into()).unwrap();
        let id = a.id.unwrap();
        s.claim(&id, false, &recording()).unwrap();
        let mut r = recording();
        r.phase = RecordingPhase::Error;
        r.error_message = Some("保存失敗".into());
        assert_eq!(s.observe(&r).error_message.as_deref(), Some("保存失敗"));
        assert!(s.claim(&id, false, &recording()).is_err());
        s.dismiss(&id).unwrap();
        let (next, _) = s.request(&recording(), "main", "A".into()).unwrap();
        assert_eq!(s.fail(&id, "遅れた応答".into()), next);
    }
    #[test]
    fn idle_missing_identity_and_foreign_origin_are_rejected() {
        let s = StopDecisionStore::default();
        assert!(s
            .request(&RecordingSnapshot::default(), "main", "A".into())
            .is_err());
        assert!(s.request(&recording(), "settings", "A".into()).is_err());
        let mut r = recording();
        r.session_id = None;
        assert!(s.request(&r, "main", "A".into()).is_err());
    }
    #[test]
    fn position_uses_work_area_and_negative_coordinates() {
        assert_eq!(
            centered_position((-1920, -1080), (1920, 1040), (560, 460)),
            (-1240, -790)
        );
        assert_eq!(
            centered_position((10, 20), (200, 200), (560, 460)),
            (10, 20)
        );
    }
}
