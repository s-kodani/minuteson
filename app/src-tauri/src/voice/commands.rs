use crate::analysis_worker::AnalysisController;
use crate::generation::commands::minutes_app;
use crate::internal_logs;
use crate::playback::{PlaybackInfo, PlaybackPreparations, PlaybackStore};
use crate::speaker_completion;
use crate::storage::{self, Storage};
use crate::voice;
use tauri::{Emitter, Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_voice_people(
    window: WebviewWindow,
) -> Result<Vec<storage::voice::Person>, mon_contracts::AppError> {
    let result: Result<Vec<storage::voice::Person>, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().voice_people())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_voice_people", message))
}

#[tauri::command]
pub(crate) async fn prepare_voice_sample_playback(
    window: WebviewWindow,
    sample_id: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        let app = minutes_app(&window)?;
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            guard.check()?;
            let audio = app
                .state::<Storage>()
                .prepare_voice_sample_audio_cancellable(&sample_id, &|| guard.cancelled())?;
            guard.publish(|| app.state::<PlaybackStore>().insert(audio))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_voice_sample_playback", message)
    })
}

#[tauri::command]
pub(crate) async fn get_voice_context(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::voice::Context, mon_contracts::AppError> {
    let result: Result<storage::voice::Context, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().voice_context(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_voice_context", message))
}

#[derive(serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum VoiceStartReply {
    Accepted { job: storage::JobSnapshot },
    Rejected { message: String },
}

#[tauri::command]
pub(crate) async fn start_voice_job(
    window: WebviewWindow,
    request: storage::voice::Start,
) -> Result<VoiceStartReply, mon_contracts::AppError> {
    let result: Result<VoiceStartReply, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            match app.state::<Storage>().start_voice(request) {
                Ok(job) => {
                    if let Err(error) = AnalysisController::kick(&app) {
                        internal_logs::report_error("voice_matching", &error);
                    }
                    VoiceStartReply::Accepted { job }
                }
                Err(message) => VoiceStartReply::Rejected { message },
            }
        })
        .await
        .map_err(|e| e.to_string())
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("start_voice_job", message))
}

#[tauri::command]
pub(crate) async fn manage_voice_person(
    window: WebviewWindow,
    person_id: String,
    expected_version: i64,
    name: Option<String>,
    sample_id: Option<String>,
) -> Result<storage::voice::PersonMutation, mon_contracts::AppError> {
    let result: Result<storage::voice::PersonMutation, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().manage_voice_person(
                &person_id,
                expected_version,
                name,
                sample_id,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("manage_voice_person", message))
}

#[tauri::command]
pub(crate) async fn retry_voice_audio_storage(
    window: WebviewWindow,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let result = app.state::<Storage>().retry_voice_audio_cleanup();
            let _ = app.emit("voice-identity-changed", ());
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("retry_voice_audio_storage", message)
    })
}

#[tauri::command]
pub(crate) fn get_voice_audio_storage_state(
    window: WebviewWindow,
) -> Result<storage::voice::AudioStorageState, mon_contracts::AppError> {
    let result: Result<storage::voice::AudioStorageState, String> = (|| {
        let app = minutes_app(&window)?;
        app.state::<Storage>().voice_audio_storage_state()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_voice_audio_storage_state", message)
    })
}

#[tauri::command]
pub(crate) async fn decide_voice_match(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
    speaker_id: String,
    person_id: Option<String>,
    expected_working_hash: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().decide_voice_match(
                &meeting_id,
                &analysis_id,
                &speaker_id,
                person_id,
                &expected_working_hash,
            )?;
            let _ = app.emit("voice-identity-changed", &meeting_id);
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("decide_voice_match", message))
}

#[tauri::command]
pub(crate) async fn retry_voice_job(
    window: WebviewWindow,
    job_id: String,
    operation_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let job = if app.state::<Storage>().get_job(&job_id)?.kind == "speaker_completion" {
                speaker_completion::retry(&app.state::<Storage>(), &job_id, &operation_id)?
            } else {
                voice::retry(&app.state::<Storage>(), &job_id, &operation_id)?
            };
            AnalysisController::kick(&app)?;
            Ok(job)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("retry_voice_job", message))
}

#[tauri::command]
pub(crate) fn get_voice_matching_preferences(
    window: WebviewWindow,
) -> Result<storage::voice_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::voice_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window.state::<Storage>().voice_matching_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_voice_matching_preferences", message)
    })
}

#[tauri::command]
pub(crate) fn save_voice_matching_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::voice_preferences::VoiceMatchingPreferences,
) -> Result<storage::voice_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::voice_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window
            .state::<Storage>()
            .save_voice_matching_preferences(expected_lock_version, payload)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_voice_matching_preferences", message)
    })
}
