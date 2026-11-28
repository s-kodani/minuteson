use crate::model_inspection;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn cancel_model_inspection(
    window: WebviewWindow,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can cancel model inspection".into());
        }
        window.state::<model_inspection::Inspector>().cancel(&token)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("cancel_model_inspection", message)
    })
}
