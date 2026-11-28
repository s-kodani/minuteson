use crate::recording_confirmation_window;
use crate::residency;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn hide_recorder_panel(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "recorder" {
            return Err("Only the recorder panel can hide itself".into());
        }
        recording_confirmation_window::hide_recorder(window.app_handle())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("hide_recorder_panel", message))
}

#[tauri::command]
pub(crate) fn open_main_window(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "recorder" {
            return Err("Only the recorder panel can reopen the main window".into());
        }
        residency::restore(window.app_handle())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("open_main_window", message))
}
