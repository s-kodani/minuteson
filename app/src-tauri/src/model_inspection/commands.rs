use crate::model_inspection;
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn cancel_model_inspection(
    window: WebviewWindow,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        require_main_window(&window, "Only the main window can cancel model inspection")?;
        window.state::<model_inspection::Inspector>().cancel(&token)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("cancel_model_inspection", message)
    })
}
