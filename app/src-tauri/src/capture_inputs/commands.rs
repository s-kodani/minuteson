use crate::capture_inputs;
use crate::capture_scope;
use crate::recorder::RecorderController;
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_capture_inputs(
    window: WebviewWindow,
) -> Result<capture_inputs::InputDevices, mon_contracts::AppError> {
    let result: Result<capture_inputs::InputDevices, String> = async move {
        require_main_window(&window, "マイク一覧はメイン画面で確認してください")?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::inputs(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_capture_inputs", message))
}

#[tauri::command]
pub(crate) async fn get_capture_applications(
    window: WebviewWindow,
) -> Result<capture_scope::Applications, mon_contracts::AppError> {
    let result: Result<capture_scope::Applications, String> = async move {
        require_main_window(
            &window,
            "録音対象アプリの一覧はメイン画面で確認してください",
        )?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::applications(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_applications", message)
    })
}
