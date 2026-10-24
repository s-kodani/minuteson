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
        matches!(self.phase, DecisionPhase::Confirming)
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
        if matches!(
            value.phase,
            DecisionPhase::Confirming | DecisionPhase::Stopping
        ) {
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

// Convert the clicked button's CSS-pixel rectangle to physical desktop
// coordinates before positioning the separate native popup. Invalid or stale
// rectangles fall back to the source window's edge instead of moving offscreen.
pub fn css_anchor_to_physical(
    inner_origin: (i32, i32),
    inner_size: (u32, u32),
    rect: (i32, i32, u32, u32),
    scale: f64,
) -> Option<(i32, i32, u32, u32)> {
    if rect.0 < 0 || rect.1 < 0 || rect.2 == 0 || rect.3 == 0 || !scale.is_finite() || scale <= 0.0
    {
        return None;
    }
    let scaled = |value: u32| -> Option<u32> {
        let physical = (f64::from(value) * scale).round();
        if physical < 0.0 || physical > f64::from(u32::MAX) {
            None
        } else {
            u32::try_from(physical as u64).ok()
        }
    };
    let (x, y, width, height) = (
        scaled(rect.0 as u32)?,
        scaled(rect.1 as u32)?,
        scaled(rect.2)?,
        scaled(rect.3)?,
    );
    if width == 0
        || height == 0
        || u64::from(x) + u64::from(width) > u64::from(inner_size.0) + 2
        || u64::from(y) + u64::from(height) > u64::from(inner_size.1) + 2
    {
        return None;
    }
    Some((
        i32::try_from(i64::from(inner_origin.0) + i64::from(x)).ok()?,
        i32::try_from(i64::from(inner_origin.1) + i64::from(y)).ok()?,
        width,
        height,
    ))
}

// A small, non-modal window follows the clicked stop button. OS-initiated
// requests without a button fall back to the source window's edge.
pub fn popup_position(
    origin: (i32, i32),
    area: (u32, u32),
    source: (i32, i32),
    source_size: (u32, u32),
    anchor: Option<(i32, i32, u32, u32)>,
    popup: (u32, u32),
    mini: bool,
    scale: f64,
) -> (i32, i32) {
    let margin = (12.0 * scale).round() as i64;
    let gap = (8.0 * scale).round() as i64;
    let clamp = |candidate: i64, start: i32, extent: u32, size: u32| {
        let low = i64::from(start) + margin;
        let high = i64::from(start) + i64::from(extent) - i64::from(size) - margin;
        if high < low {
            start
        } else {
            candidate.clamp(low, high) as i32
        }
    };
    let (x, below, above) = if let Some((ax, ay, aw, ah)) = anchor {
        (
            i64::from(ax) + i64::from(aw) - i64::from(popup.0),
            i64::from(ay) + i64::from(ah) + gap,
            i64::from(ay) - i64::from(popup.1) - gap,
        )
    } else {
        (
            i64::from(source.0) + i64::from(source_size.0) - i64::from(popup.0) - margin,
            i64::from(source.1) + i64::from(source_size.1) + gap,
            i64::from(source.1) - i64::from(popup.1) - gap,
        )
    };
    let y = if mini || anchor.is_some() {
        if below + i64::from(popup.1) <= i64::from(origin.1) + i64::from(area.1) - margin {
            below
        } else {
            above
        }
    } else {
        i64::from(source.1) + (68.0 * scale).round() as i64
    };
    (
        clamp(x, origin.0, area.0, popup.0),
        clamp(y, origin.1, area.1, popup.1),
    )
}

pub fn fitted_size(area: (u32, u32), scale: f64) -> Result<(u32, u32), String> {
    if area.0 == 0 || area.1 == 0 || !scale.is_finite() || scale <= 0.0 {
        return Err("確認用の画面領域を検証できません".into());
    }
    Ok((
        ((420.0 * scale).round() as u32).max(1).min(area.0),
        ((280.0 * scale).round() as u32).max(1).min(area.1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(fitted_size((1920, 1080), 2.0).unwrap(), (840, 560));
        assert_eq!(fitted_size((800, 600), 2.0).unwrap(), (800, 560));
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
    fn popup_follows_origin_and_fits_negative_monitor_coordinates() {
        assert_eq!(
            popup_position(
                (-1920, -1080),
                (1920, 1040),
                (-1700, -900),
                (1000, 700),
                None,
                (420, 280),
                false,
                1.0,
            ),
            (-1132, -832)
        );
        assert_eq!(
            popup_position(
                (-1920, -1080),
                (1920, 1040),
                (-1800, -200),
                (360, 116),
                None,
                (420, 280),
                true,
                1.0,
            ),
            (-1872, -488)
        );
        assert_eq!(
            popup_position(
                (10, 20),
                (200, 200),
                (10, 20),
                (200, 200),
                None,
                (200, 200),
                true,
                1.0
            ),
            (10, 20)
        );
        assert_eq!(
            popup_position(
                (-1920, -1080),
                (1920, 1040),
                (-1700, -900),
                (1000, 700),
                Some((-900, -840, 88, 40)),
                (420, 280),
                false,
                1.0,
            ),
            (-1232, -792)
        );
        assert_eq!(
            popup_position(
                (-1920, -1080),
                (1920, 1040),
                (-1600, -200),
                (360, 116),
                Some((-1500, -100, 36, 32)),
                (420, 280),
                true,
                1.0,
            ),
            (-1884, -388)
        );
    }

    #[test]
    fn css_button_anchor_uses_webview_origin_and_retina_scale() {
        assert_eq!(
            css_anchor_to_physical((-1800, -900), (1200, 800), (500, 32, 36, 36), 2.0),
            Some((-800, -836, 72, 72))
        );
        assert_eq!(
            css_anchor_to_physical((-100, 200), (200, 200), (10, 11, 35, 35), 1.5),
            Some((-85, 217, 53, 53))
        );
    }

    #[test]
    fn stale_or_impossible_css_button_anchor_falls_back() {
        let origin = (0, 0);
        let size = (360, 116);
        assert_eq!(
            css_anchor_to_physical(origin, size, (-1, 10, 36, 36), 1.0),
            None
        );
        assert_eq!(
            css_anchor_to_physical(origin, size, (340, 10, 36, 36), 1.0),
            None
        );
        assert_eq!(
            css_anchor_to_physical(origin, size, (10, 10, 0, 36), 1.0),
            None
        );
        assert_eq!(
            css_anchor_to_physical(origin, size, (10, 10, 36, 36), f64::NAN),
            None
        );
        assert_eq!(
            css_anchor_to_physical(origin, size, (10, 10, 36, 36), f64::INFINITY),
            None
        );
        assert_eq!(
            css_anchor_to_physical(origin, size, (10, 10, 36, 36), 0.01),
            None
        );
        assert_eq!(
            css_anchor_to_physical((i32::MAX - 10, 0), (100, 100), (20, 0, 36, 36), 1.0),
            None
        );
    }
}
