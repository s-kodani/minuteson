use crate::analysis_worker::AnalysisController;
use crate::capture_scope;
use crate::capture_test;
use crate::import_controller::ImportController;
use crate::input_levels;
use crate::input_status;
use crate::internal_logs;
use crate::recorder::{self, RecorderController};
use crate::recording_confirmation::StopDecisionStore;
use crate::recording_windows;
use crate::recording_writes;
use crate::session::{RecordingSnapshot, SessionStore};
use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_recording_levels(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_levels::InputLevels, mon_contracts::AppError> {
    let result: Result<input_levels::InputLevels, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力レベルは録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::levels(&app, &session_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_recording_levels", message))
}

#[tauri::command]
pub(crate) async fn get_recording_writes(
    window: WebviewWindow,
    session_id: String,
) -> Result<recording_writes::Snapshot, mon_contracts::AppError> {
    let result: Result<recording_writes::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("書込み状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::writes(&app, &session_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_recording_writes", message))
}

#[tauri::command]
pub(crate) async fn get_recording_input_status(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_status::Snapshot, mon_contracts::AppError> {
    let result: Result<input_status::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_status(&app, &session_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_input_status", message)
    })
}

#[tauri::command]
pub(crate) async fn get_recording_input_history_status(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_status::persistence::Snapshot, mon_contracts::AppError> {
    let result: Result<input_status::persistence::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力履歴の保存状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = RecorderController::input_history_status(&app, &session_id);
            input_status::diagnostics::read_result(
                "live-journal/input-journal-status.json",
                Some(&session_id),
                None,
                result.as_ref().err().map(String::as_str),
            );
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_input_history_status", message)
    })
}

#[tauri::command]
pub(crate) fn capture_available(window: WebviewWindow) -> Result<bool, mon_contracts::AppError> {
    let result: Result<bool, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can inspect capture availability".into());
        }
        Ok(RecorderController::available(window.app_handle()))
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("capture_available", message))
}

#[tauri::command]
pub(crate) async fn start_recording(
    window: WebviewWindow,
    meeting_id: String,
    microphone_uid: Option<String>,
    capture_scope: capture_scope::CaptureScope,
    new_title: Option<String>,
    num_speakers: Option<u32>,
    meeting_vocabulary: Option<Vec<storage::VocabularyEntry>>,
) -> Result<RecordingSnapshot, mon_contracts::AppError> {
    async move {
        if window.label() != "main" {
            return Err("Only the main window can start recording".into());
        }
        let app = window.app_handle().clone();
        let speaker_counts = storage::recordings::SpeakerCounts { num_speakers };
        speaker_counts.validate()?;
        if new_title.is_none() && meeting_vocabulary.is_some() {
            return Err("既存会議の用語は会議の用語画面で保存してください".into());
        }
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                Ok(
                    (|| -> Result<RecordingSnapshot, mon_contracts::AppError> {
                        app.state::<capture_test::CaptureTests>().require_empty()?;
                        if app.state::<StopDecisionStore>().snapshot().visible() {
                            return Err("録音確認を完了してから開始してください".into());
                        }
                        if app.state::<SessionStore>().snapshot().is_active() {
                            return Err("別の録音が進行中です".into());
                        }
                        app.state::<Storage>().require_data_writable()?;
                        let inputs = RecorderController::inputs(&app)?;
                        if let Some(uid) = microphone_uid.as_deref() {
                            inputs.require_microphone(uid)?;
                        } else {
                            inputs.require_automatic_microphone()?;
                        }
                        RecorderController::require_scope(&app, &capture_scope)?;
                        ImportController::prepare_recording(&app)?;
                        AnalysisController::prepare_recording(&app)?;
                        let result = (|| {
                            recording_windows::prepare(&app)?;
                            if let Some(title) = new_title {
                                app.state::<Storage>()
                                    .create_recording_draft_with_vocabulary(
                                        &meeting_id,
                                        &title,
                                        &meeting_vocabulary.unwrap_or_default(),
                                    )?;
                            }
                            RecorderController::start(
                                &app,
                                &meeting_id,
                                microphone_uid.as_deref(),
                                &capture_scope,
                                &speaker_counts,
                            )
                        })();
                        if result.is_err() {
                            AnalysisController::recording_finished(&app);
                            if let Err(error) = recording_windows::release_if_current(
                                &app,
                                &app.state::<SessionStore>().snapshot(),
                            ) {
                                internal_logs::report_error("recording_windows", &error);
                            }
                        }
                        result.map_err(|error| {
                            mon_contracts::AppError::from_command("start_recording", error)
                        })
                    })(),
                )
            })
        })
        .await
        .map_err(|e| e.to_string())??
    }
    .await
}

#[tauri::command]
pub(crate) async fn select_recording_microphone(
    window: WebviewWindow,
    session_id: String,
    microphone_uid: Option<String>,
) -> Result<u64, mon_contracts::AppError> {
    let result: Result<u64, String> = async move {
        if window.label() != "main" {
            return Err("マイク切替はメイン画面で行ってください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                RecorderController::select_input(&app, &session_id, microphone_uid.as_deref())
            })
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("select_recording_microphone", message)
    })
}

#[tauri::command]
pub(crate) async fn get_recording_microphone_selection(
    window: WebviewWindow,
    session_id: String,
) -> Result<recorder::MicrophoneSelectionStatus, mon_contracts::AppError> {
    let result: Result<recorder::MicrophoneSelectionStatus, String> = async move {
        if window.label() != "main" {
            return Err("マイク選択状態はメイン画面で確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::selection_status(&app, &session_id)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_microphone_selection", message)
    })
}
