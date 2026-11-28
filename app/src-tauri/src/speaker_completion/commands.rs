use crate::generation::commands::minutes_app;
use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_speaker_completion_preferences(
    window: WebviewWindow,
) -> Result<storage::completion_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::completion_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window.state::<Storage>().speaker_completion_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_speaker_completion_preferences", message)
    })
}

#[tauri::command]
pub(crate) fn save_speaker_completion_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::completion_preferences::CompletionPreferences,
) -> Result<storage::completion_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::completion_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window
            .state::<Storage>()
            .save_speaker_completion_preferences(expected_lock_version, payload)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_speaker_completion_preferences", message)
    })
}
