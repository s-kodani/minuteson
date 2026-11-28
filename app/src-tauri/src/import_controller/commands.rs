use crate::analysis_worker::AnalysisController;
use crate::import_controller::{self, ImportController};
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_audio_import(
    window: WebviewWindow,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        require_main_window(&window, "Only the main window can read imports")?;
        window.state::<ImportController>().snapshot()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_audio_import", message))
}

#[tauri::command]
pub(crate) async fn select_audio_import(
    window: WebviewWindow,
    path: Option<std::path::PathBuf>,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = async move {
        require_main_window(&window, "Only the main window can select audio")?;
        if window.state::<AnalysisController>().model_cleanup_active() {
            return Err("重複モデルを整理中です。完了後に音声取り込みを開始できます。".into());
        }
        ImportController::select(&window, path).await
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("select_audio_import", message))
}

#[tauri::command]
pub(crate) fn start_audio_import(
    window: WebviewWindow,
    request: import_controller::StartRequest,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        require_main_window(&window, "Only the main window can import audio")?;
        if window.state::<AnalysisController>().model_cleanup_active() {
            return Err("重複モデルを整理中です。完了後に音声取り込みを開始できます。".into());
        }
        ImportController::start(window.app_handle(), request)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("start_audio_import", message))
}

#[tauri::command]
pub(crate) fn cancel_audio_import(
    window: WebviewWindow,
    token: String,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        require_main_window(&window, "Only the main window can cancel imports")?;
        ImportController::cancel_from_app(window.app_handle(), &token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_audio_import", message))
}
