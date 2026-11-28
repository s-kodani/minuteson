use crate::analysis_preparation;
use crate::analysis_worker::AnalysisController;
use crate::capture_test;
use crate::generation::commands::minutes_app;
use crate::internal_logs;
use crate::speaker_completion;
use crate::storage::{self, Storage, TranscriptDraft, TranscriptDraftPayload};
use crate::transcription;
use crate::window_environment::require_main_window;
use tauri::{Emitter, Manager, State, WebviewWindow};

#[tauri::command]
pub(crate) fn get_asr_preferences(
    window: WebviewWindow,
) -> Result<storage::PreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::PreferencesSnapshot, String> = (|| {
        require_main_window(&window, "メイン画面で操作してください")?;
        window.state::<Storage>().analysis_preferences()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_asr_preferences", message))
}

#[tauri::command]
pub(crate) fn get_asr_backends(
    window: WebviewWindow,
) -> Result<Vec<transcription::asr::Descriptor>, mon_contracts::AppError> {
    let result: Result<Vec<transcription::asr::Descriptor>, String> = (|| {
        require_main_window(&window, "メイン画面で操作してください")?;
        Ok(transcription::asr::registry()
            .iter()
            .map(|a| a.descriptor())
            .collect())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_asr_backends", message))
}

#[tauri::command]
pub(crate) async fn get_transcription_regeneration_target(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::analysis_flows::regeneration::Admission, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::regeneration::Admission, String> = async move {
        require_main_window(
            &window,
            "文字起こしの再生成はメイン画面から操作してください",
        )?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().regeneration_admission(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_transcription_regeneration_target", message)
    })
}

#[tauri::command]
pub(crate) async fn regenerate_transcription(
    window: WebviewWindow,
    request: storage::analysis_flows::regeneration::Request,
) -> Result<storage::analysis_flows::AnalysisFlow, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::AnalysisFlow, String> = async move {
        require_main_window(
            &window,
            "文字起こしの再生成はメイン画面から操作してください",
        )?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _preparing =
                analysis_preparation::begin(&request.meeting_id, &request.operation_id);
            if let Some(flow) = app.state::<Storage>().regeneration_replay(&request)? {
                return Ok(flow);
            }
            let flow =
                AnalysisController::with_model_preparation(&app, None, &|| false, |lease| {
                    app.state::<capture_test::CaptureTests>().require_empty()?;
                    app.state::<Storage>()
                        .regenerate_analysis(&request, &|| lease.cancelled())
                })?;
            let _ = app.emit("analysis-flow-changed", flow.clone());
            Ok(flow)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("regenerate_transcription", message)
    })
}

#[tauri::command]
pub(crate) async fn publish_regenerated_transcription(
    window: WebviewWindow,
    meeting_id: String,
    flow_id: String,
    expected_lock_version: i64,
    confirmed_target: Option<storage::analysis_flows::regeneration::Target>,
) -> Result<storage::analysis_flows::regeneration::Publication, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::regeneration::Publication, String> = async move {
        require_main_window(&window, "文字起こしの保存はメイン画面から操作してください")?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = app.state::<Storage>().publish_regenerated_flow(
                &meeting_id,
                &flow_id,
                expected_lock_version,
                confirmed_target.as_ref(),
            );
            if let Ok(Some(flow)) = app.state::<Storage>().analysis_flow(&meeting_id) {
                let _ = app.emit("analysis-flow-changed", flow);
            }
            if matches!(
                &result,
                Ok(storage::analysis_flows::regeneration::Publication::Published { .. })
            ) {
                if let Err(error) =
                    speaker_completion::after_analysis(&app.state::<Storage>(), &meeting_id)
                {
                    internal_logs::report_error("voice_matching", &error);
                }
                let _ = AnalysisController::kick(&app);
            }
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("publish_regenerated_transcription", message)
    })
}

#[tauri::command]
pub(crate) async fn consolidate_transcript(
    window: WebviewWindow,
    request: storage::transcript_consolidation::Request,
) -> Result<storage::transcript_consolidation::Reply, mon_contracts::AppError> {
    let result: Result<storage::transcript_consolidation::Reply, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().consolidate_transcript(&request)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("consolidate_transcript", message))
}

#[tauri::command]
pub(crate) async fn get_transcript_draft(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<TranscriptDraft>, mon_contracts::AppError> {
    let result: Result<Option<TranscriptDraft>, String> = async move {
        require_main_window(&window, "Only the main window can read drafts")?;

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().load_transcript_draft(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_transcript_draft", message))
}

#[tauri::command]
pub(crate) fn begin_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
    analysis_id: String,
) -> Result<TranscriptDraft, mon_contracts::AppError> {
    let result: Result<TranscriptDraft, String> = (|| {
        require_main_window(&window, "Only the main window can edit transcripts")?;
        storage.create_transcript_draft(&meeting_id, &analysis_id)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("begin_transcript_draft", message))
}

#[tauri::command]
pub(crate) fn save_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    draft_id: String,
    expected_lock_version: i64,
    payload: TranscriptDraftPayload,
) -> Result<TranscriptDraft, mon_contracts::AppError> {
    let result: Result<TranscriptDraft, String> = (|| {
        require_main_window(&window, "Only the main window can edit transcripts")?;
        storage.save_transcript_draft(&draft_id, expected_lock_version, payload)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("save_transcript_draft", message))
}
