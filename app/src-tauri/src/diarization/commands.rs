use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_diarization_preferences(
    window: WebviewWindow,
) -> Result<storage::DiarizationPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::DiarizationPreferencesSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read diarization settings".into());
        }
        window.state::<Storage>().diarization_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_diarization_preferences", message)
    })
}
