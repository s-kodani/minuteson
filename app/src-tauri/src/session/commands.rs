use super::{RecordingSnapshot, SessionStore};
use tauri::State;

#[tauri::command]
pub(crate) fn get_recording_snapshot(store: State<'_, SessionStore>) -> RecordingSnapshot {
    store.snapshot()
}
