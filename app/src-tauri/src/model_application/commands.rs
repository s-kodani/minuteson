use crate::analysis_worker::AnalysisController;
use crate::job_history;
use crate::model_application;
use crate::model_inspection;
use crate::storage::{self, Storage};
use crate::transcription;
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) fn get_model_profiles(
    window: WebviewWindow,
) -> Result<Vec<storage::model_profiles::ProfileView>, mon_contracts::AppError> {
    let result: Result<Vec<storage::model_profiles::ProfileView>, String> = (|| {
        require_main_window(&window, "Only the main window can read model profiles")?;
        window.state::<Storage>().model_profiles()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_model_profiles", message))
}

#[tauri::command]
pub(crate) async fn preview_model_profile_application(
    window: WebviewWindow,
    token: String,
    history_operation_id: Option<String>,
    profile: storage::model_profiles::ProfileName,
    package_id: String,
    expected_review_hash: String,
    check_operation_id: String,
    diarization_threads: Option<u32>,
    diarization_cluster_threshold_milli: Option<u32>,
    whisper_vad_options: Option<transcription::vad::VadOptions>,
) -> Result<storage::model_profiles::ApplyPreview, mon_contracts::AppError> {
    let result: Result<storage::model_profiles::ApplyPreview, String> = async move {
        require_main_window(&window, "Only the main window can review model application")?;
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
                        model_application::preview_with_vad(
                            &app.state::<Storage>(),
                            &models_root,
                            profile,
                            &package_id,
                            &expected_review_hash,
                            &check_operation_id,
                            diarization_threads,
                            diarization_cluster_threshold_milli,
                            whisper_vad_options,
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
        mon_contracts::AppError::from_command("preview_model_profile_application", message)
    })
}

#[tauri::command]
pub(crate) async fn apply_model_profile_application(
    window: WebviewWindow,
    token: String,
    history_operation_id: Option<String>,
    preview: storage::model_profiles::ApplyPreview,
) -> Result<storage::model_profiles::ProfileView, mon_contracts::AppError> {
    let result: Result<storage::model_profiles::ProfileView, String> = async move {
        require_main_window(&window, "Only the main window can apply model profiles")?;
        let app = window.app_handle().clone();
        let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
        let models_root = app.state::<Storage>().native_models_root()?.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let target = app
                .state::<Storage>()
                .history_model_name(&preview.package_id)
                .unwrap_or_default();
            job_history::model_stage(history_operation_id.as_deref(), &target, false, || {
                AnalysisController::with_model_preparation(
                    &app,
                    Some(&token),
                    &|| guard.cancelled(),
                    |lease| {
                        let applied = model_application::apply(
                            &app.state::<Storage>(),
                            &models_root,
                            &token,
                            &preview,
                            &|| guard.cancelled() || lease.cancelled(),
                        );
                        if applied.is_ok() {
                            AnalysisController::finish_model_application(
                                &app,
                                lease,
                                history_operation_id.as_deref(),
                                &target,
                                &preview.package_id,
                            );
                        }
                        applied
                    },
                )
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("apply_model_profile_application", message)
    })
}
