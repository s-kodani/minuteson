use crate::storage::{self, Storage};
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_diarization_preferences(
    window: WebviewWindow,
) -> Result<storage::DiarizationPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::DiarizationPreferencesSnapshot, String> = (|| {
        require_main_window(
            &window,
            "Only the main window can read diarization settings",
        )?;
        window.state::<Storage>().diarization_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_diarization_preferences", message)
    })
}
