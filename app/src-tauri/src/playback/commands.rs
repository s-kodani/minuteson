use super::{PlaybackInfo, PlaybackPreparations, PlaybackStore};
use crate::capture_test;
use crate::generation::commands::minutes_app;
use crate::storage::{self, Storage};
use tauri::{Manager, State, WebviewWindow};

#[tauri::command]
pub(crate) fn register_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can prepare playback".into());
        }
        window
            .state::<PlaybackPreparations>()
            .reserve(&preparation_id)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("register_playback_preparation", message)
    })
}

#[tauri::command]
pub(crate) fn cancel_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can cancel playback preparation".into());
        }
        window
            .state::<PlaybackPreparations>()
            .cancel(&preparation_id, &window.state::<PlaybackStore>())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("cancel_playback_preparation", message)
    })
}

#[tauri::command]
pub(crate) fn finish_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
    playback_token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can acknowledge playback preparation".into());
        }
        window
            .state::<PlaybackPreparations>()
            .acknowledge(&preparation_id, &playback_token)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("finish_playback_preparation", message)
    })
}

#[tauri::command]
pub(crate) async fn prepare_meeting_playback(
    window: WebviewWindow,
    request: storage::MeetingPlaybackRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let mut audio = app
                        .state::<Storage>()
                        .prepare_meeting_audio_cancellable(&request, &|| guard.cancelled())?;

                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert_meeting(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_meeting_playback", message)
    })
}

#[tauri::command]
pub(crate) async fn get_meeting_audio_summary(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::MeetingAudioSummary, mon_contracts::AppError> {
    let result: Result<storage::MeetingAudioSummary, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let summary = app.state::<Storage>().meeting_audio_summary(&meeting_id)?;
            Ok(summary)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_audio_summary", message)
    })
}

#[tauri::command]
pub(crate) async fn prepare_markdown_interval_playback(
    window: WebviewWindow,
    request: storage::MeetingIntervalRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        let app = minutes_app(&window)?;
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let mut audio = app
                        .state::<Storage>()
                        .prepare_meeting_interval_cancellable(&request, &|| guard.cancelled())?;

                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert_interval(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_markdown_interval_playback", message)
    })
}

#[tauri::command]
pub(crate) async fn prepare_utterance_playback(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
    utterance_id: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner =
                        app.state::<Storage>()
                            .begin_meeting_work(&meeting_id, "playback", None)?;
                    let audio = app.state::<Storage>().prepare_utterance_audio_cancellable(
                        &meeting_id,
                        &analysis_id,
                        &utterance_id,
                        &|| guard.cancelled(),
                    )?;
                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_utterance_playback", message)
    })
}

#[tauri::command]
pub(crate) async fn prepare_draft_utterance_playback(
    window: WebviewWindow,
    request: storage::DraftUtteranceAudioRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let audio = app
                        .state::<Storage>()
                        .prepare_draft_utterance_audio_cancellable(&request, &|| {
                            guard.cancelled()
                        })?;
                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_draft_utterance_playback", message)
    })
}

#[tauri::command]
pub(crate) async fn read_playback_chunk(
    window: WebviewWindow,
    token: String,
    offset_frames: u64,
) -> Result<tauri::ipc::Response, mon_contracts::AppError> {
    let result: Result<tauri::ipc::Response, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let bytes = tauri::async_runtime::spawn_blocking(move || {
            app.state::<PlaybackStore>().read(&token, offset_frames)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(tauri::ipc::Response::new(bytes))
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("read_playback_chunk", message))
}

#[tauri::command]
pub(crate) fn close_playback(
    window: WebviewWindow,
    store: State<'_, PlaybackStore>,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can close playback".into());
        }
        store.close(&token)?;
        window
            .state::<capture_test::CaptureTests>()
            .release_playback(&token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("close_playback", message))
}
