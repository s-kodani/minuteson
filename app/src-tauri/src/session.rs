use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingPhase {
    Idle,
    Preparing,
    Recording,
    Stopping,
    Saving,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSnapshot {
    pub revision: u64,
    pub phase: RecordingPhase,
    pub elapsed_ms: u64,
    pub microphone_connected: bool,
    pub system_audio_connected: bool,
    pub meeting_id: Option<String>,
    pub session_id: Option<String>,
    pub error_message: Option<String>,
}

impl Default for RecordingSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: RecordingPhase::Idle,
            elapsed_ms: 0,
            microphone_connected: false,
            system_audio_connected: false,
            meeting_id: None,
            session_id: None,
            error_message: None,
        }
    }
}

impl RecordingSnapshot {
    pub fn is_active(&self) -> bool {
        !matches!(self.phase, RecordingPhase::Idle | RecordingPhase::Error)
    }
}

#[derive(Default)]
pub struct SessionStore(Mutex<RecordingSnapshot>);

impl SessionStore {
    pub fn snapshot(&self) -> RecordingSnapshot {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn update(&self, change: impl FnOnce(&mut RecordingSnapshot)) -> RecordingSnapshot {
        let mut snapshot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.clone()
    }
    pub(crate) fn update_if(
        &self,
        change: impl FnOnce(&mut RecordingSnapshot) -> bool,
    ) -> Option<RecordingSnapshot> {
        let mut snapshot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !change(&mut snapshot) {
            return None;
        }
        snapshot.revision = snapshot.revision.saturating_add(1);
        Some(snapshot.clone())
    }
    pub(crate) fn update_input_connections(
        &self,
        owner: &str,
        microphone: bool,
        system: bool,
    ) -> Option<RecordingSnapshot> {
        self.update_if(|snapshot| {
            if snapshot.session_id.as_deref() != Some(owner)
                || !matches!(
                    snapshot.phase,
                    RecordingPhase::Preparing | RecordingPhase::Recording
                )
                || (snapshot.microphone_connected == microphone
                    && snapshot.system_audio_connected == system)
            {
                return false;
            }
            snapshot.microphone_connected = microphone;
            snapshot.system_audio_connected = system;
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_real_initial_state() {
        let snapshot = SessionStore::default().snapshot();
        assert_eq!(snapshot.phase, RecordingPhase::Idle);
        assert!(!snapshot.is_active());
        assert!(!snapshot.microphone_connected);
        assert!(!snapshot.system_audio_connected);
    }

    #[test]
    fn shared_store_returns_versioned_snapshots() {
        let store = SessionStore::default();
        let first_window = store.snapshot();
        let changed = store.update(|snapshot| snapshot.phase = RecordingPhase::Preparing);
        let second_window = store.snapshot();
        assert_eq!(first_window.revision, 0);
        assert_eq!(changed, second_window);
        assert_eq!(second_window.revision, 1);
        assert!(second_window.is_active());
    }
    #[test]
    fn late_input_telemetry_cannot_change_a_stopped_or_different_owner() {
        let store = SessionStore::default();
        store.update(|s| {
            s.phase = RecordingPhase::Recording;
            s.session_id = Some("current".into());
        });
        let connected = store
            .update_input_connections("current", true, true)
            .unwrap();
        assert!(connected.microphone_connected && connected.system_audio_connected);
        store.update(|s| {
            s.phase = RecordingPhase::Stopping;
            s.microphone_connected = false;
            s.system_audio_connected = false;
        });
        let before = store.snapshot();
        assert!(store
            .update_input_connections("current", true, true)
            .is_none());
        assert_eq!(store.snapshot(), before);
        store.update(|s| {
            s.phase = RecordingPhase::Recording;
            s.session_id = Some("next".into());
        });
        let before = store.snapshot();
        assert!(store
            .update_input_connections("current", true, true)
            .is_none());
        assert_eq!(store.snapshot(), before);
    }
    #[test]
    fn paired_input_flags_update_once_without_overwriting_the_other_lane() {
        let store = SessionStore::default();
        store.update(|s| {
            s.phase = RecordingPhase::Preparing;
            s.session_id = Some("current".into());
        });
        let mic_only = store
            .update_input_connections("current", true, false)
            .unwrap();
        assert!(mic_only.microphone_connected);
        assert!(!mic_only.system_audio_connected);
        assert!(store
            .update_input_connections("current", true, false)
            .is_none());
        assert_eq!(store.snapshot(), mic_only);
        let pc_only = store
            .update_input_connections("current", false, true)
            .unwrap();
        assert!(!pc_only.microphone_connected);
        assert!(pc_only.system_audio_connected);
        assert_eq!(pc_only.revision, mic_only.revision + 1);
        assert_eq!(store.snapshot(), pc_only);
        let unknown = store
            .update_input_connections("current", false, false)
            .unwrap();
        assert!(!unknown.microphone_connected && !unknown.system_audio_connected);
    }
    #[test]
    fn no_terminal_phase_can_become_connected_from_late_input_status() {
        let store = SessionStore::default();
        for phase in [
            RecordingPhase::Idle,
            RecordingPhase::Stopping,
            RecordingPhase::Saving,
            RecordingPhase::Error,
        ] {
            store.update(|s| {
                s.phase = phase;
                s.session_id = Some("current".into());
            });
            let before = store.snapshot();
            assert!(store
                .update_input_connections("current", true, true)
                .is_none());
            assert_eq!(store.snapshot(), before);
        }
    }
}
