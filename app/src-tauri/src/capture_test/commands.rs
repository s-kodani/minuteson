use crate::capture_test;
use crate::input_levels;
use crate::playback::PlaybackInfo;
use crate::recorder::RecorderController;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_capture_test_snapshot(
    window: WebviewWindow,
    run_id: Option<String>,
) -> Result<Option<capture_test::Snapshot>, mon_contracts::AppError> {
    let result: Result<Option<capture_test::Snapshot>, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<capture_test::CaptureTests>()
                .snapshot(run_id.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_test_snapshot", message)
    })
}

#[tauri::command]
pub(crate) async fn reconcile_capture_test_start(
    window: WebviewWindow,
    run_id: String,
) -> Result<Option<capture_test::Snapshot>, mon_contracts::AppError> {
    let result: Result<Option<capture_test::Snapshot>, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                app.state::<capture_test::CaptureTests>()
                    .reconcile_start(&run_id)
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("reconcile_capture_test_start", message)
    })
}

#[tauri::command]
pub(crate) async fn start_capture_test(
    window: WebviewWindow,
    request: capture_test::Request,
) -> Result<capture_test::Snapshot, mon_contracts::AppError> {
    let result: Result<capture_test::Snapshot, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から開始してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::start(&app, request)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("start_capture_test", message))
}

#[tauri::command]
pub(crate) async fn cancel_capture_test(
    window: WebviewWindow,
    run_id: String,
) -> Result<capture_test::Snapshot, mon_contracts::AppError> {
    let result: Result<capture_test::Snapshot, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から停止してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::cancel(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_capture_test", message))
}

#[tauri::command]
pub(crate) async fn discard_capture_test(
    window: WebviewWindow,
    run_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から閉じてください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::discard(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("discard_capture_test", message))
}

#[tauri::command]
pub(crate) async fn get_capture_test_levels(
    window: WebviewWindow,
    run_id: String,
) -> Result<input_levels::InputLevels, mon_contracts::AppError> {
    let result: Result<input_levels::InputLevels, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::levels(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_test_levels", message)
    })
}

#[tauri::command]
pub(crate) async fn prepare_capture_test_playback(
    window: WebviewWindow,
    run_id: String,
    track_kind: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から再生してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::prepare_playback(
                &app,
                &run_id,
                &track_kind,
                &preparation_id,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_capture_test_playback", message)
    })
}
