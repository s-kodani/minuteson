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
}

impl Default for RecordingSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: RecordingPhase::Idle,
            elapsed_ms: 0,
            microphone_connected: false,
            system_audio_connected: false,
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

    // Future recording adapters must update this store before emitting session-changed.
    #[allow(dead_code)]
    pub fn update(&self, change: impl FnOnce(&mut RecordingSnapshot)) -> RecordingSnapshot {
        let mut snapshot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(&mut snapshot);
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.clone()
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
}
