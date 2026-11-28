use crate::analysis_flow;
use crate::analysis_preparation;
use crate::analysis_worker::AnalysisController;
use crate::storage::{self, AnalysisPresentation, Storage};
use tauri::{Emitter, Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<storage::analysis_flows::AnalysisFlow>, mon_contracts::AppError> {
    let result: Result<Option<storage::analysis_flows::AnalysisFlow>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis flow".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().analysis_flow(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_analysis_flow", message))
}

#[tauri::command]
pub(crate) fn get_analysis_preparations(
    window: WebviewWindow,
) -> Result<Vec<analysis_preparation::Progress>, mon_contracts::AppError> {
    let result: Result<Vec<analysis_preparation::Progress>, String> = (|| {
        if window.label() != "main" {
            return Err("解析準備はメイン画面から確認してください".into());
        }
        let mut entries = window.state::<Storage>().preparation_progress()?;
        for current in analysis_preparation::snapshot() {
            if let Some(saved) = entries
                .iter_mut()
                .find(|p| p.meeting_id == current.meeting_id)
            {
                let started = if saved.operation_id == current.operation_id {
                    saved.started_at_ms.min(current.started_at_ms)
                } else {
                    current.started_at_ms
                };
                *saved = current;
                saved.started_at_ms = started;
            } else {
                entries.push(current);
            }
        }
        for p in &mut entries {
            p.meeting_title = window
                .state::<Storage>()
                .get_meeting(&p.meeting_id)?
                .map(|m| m.title)
                .unwrap_or_else(|| "会議音声".into());
        }
        Ok(entries)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_preparations", message)
    })
}

#[tauri::command]
pub(crate) async fn get_meeting_analysis_audio(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::RecordedAudioPlan, mon_contracts::AppError> {
    let result: Result<storage::RecordedAudioPlan, String> = async move {
        if window.label() != "main" {
            return Err("解析音声はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .recorded_analysis_input(&meeting_id)
                .map(|(source, _)| source)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_analysis_audio", message)
    })
}

#[tauri::command]
pub(crate) async fn get_ready_analysis_flows(
    window: WebviewWindow,
) -> Result<Vec<storage::analysis_flows::AnalysisFlow>, mon_contracts::AppError> {
    let result: Result<Vec<storage::analysis_flows::AnalysisFlow>, String> = async move {
        if window.label() != "main" {
            return Err("文字起こしの保存はメイン画面から操作してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().ready_analysis_flows())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_ready_analysis_flows", message)
    })
}

#[tauri::command]
pub(crate) fn continue_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
    expected_lock_version: i64,
) -> Result<storage::analysis_flows::AnalysisFlow, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::AnalysisFlow, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can continue analysis flow".into());
        }
        let flow = analysis_flow::continue_flow(
            &window.state::<Storage>(),
            &meeting_id,
            expected_lock_version,
        )?;
        window
            .app_handle()
            .emit("analysis-flow-changed", flow.clone())
            .map_err(|e| e.to_string())?;
        AnalysisController::kick(window.app_handle())?;
        Ok(flow)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("continue_analysis_flow", message))
}

#[tauri::command]
pub(crate) async fn get_current_analysis(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<AnalysisPresentation>, mon_contracts::AppError> {
    let result: Result<Option<AnalysisPresentation>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .load_current_analysis_presentation(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_current_analysis", message))
}
