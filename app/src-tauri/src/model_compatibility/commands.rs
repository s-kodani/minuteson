use crate::analysis_worker::AnalysisController;
use crate::job_history;
use crate::model_compatibility;
use crate::model_inspection;
use crate::storage::Storage;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn check_model_compatibility(
    window: WebviewWindow,
    token: String,
    history_operation_id: Option<String>,
    package_id: String,
    expected_review_hash: String,
) -> Result<model_compatibility::CheckSummary, mon_contracts::AppError> {
    let result: Result<model_compatibility::CheckSummary, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can check model compatibility".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
        let models_root = app.state::<Storage>().native_models_root()?.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let target = app
                .state::<Storage>()
                .history_model_name(&package_id)
                .unwrap_or_default();
            job_history::model_stage(history_operation_id.as_deref(), &target, false, || {
                AnalysisController::with_model_preparation(
                    &app,
                    Some(&token),
                    &|| guard.cancelled(),
                    |lease| {
                        model_compatibility::check(
                            &app.state::<Storage>(),
                            &models_root,
                            &token,
                            &package_id,
                            &expected_review_hash,
                            &|| guard.cancelled() || lease.cancelled(),
                        )
                    },
                )
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("check_model_compatibility", message)
    })
}
