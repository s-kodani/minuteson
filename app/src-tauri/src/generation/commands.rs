use crate::analysis_worker::AnalysisController;
use crate::generation;
use crate::storage::{self, Storage};
use crate::window_environment::require_main_window;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_minutes_source(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::MinutesSource, mon_contracts::AppError> {
    let result: Result<storage::MinutesSource, String> = async move {
        require_main_window(&window, "メイン画面で操作してください")?;

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().minutes_source(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_minutes_source", message))
}

#[tauri::command]
pub(crate) async fn start_markdown_minutes_generation(
    window: WebviewWindow,
    request: generation::StartRequest,
    expected: Option<storage::minutes_markdown::MarkdownStamp>,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let job = request.enqueue_markdown(&app.state::<Storage>(), expected)?;
            AnalysisController::kick(&app)?;
            Ok(job)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("start_markdown_minutes_generation", message)
    })
}

pub(crate) fn minutes_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    require_main_window(window, "Only the main window can read or edit minutes")?;
    Ok(window.app_handle().clone())
}

#[tauri::command]
pub(crate) async fn get_minutes_preferences(
    window: WebviewWindow,
) -> Result<storage::MinutesPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::MinutesPreferencesSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().minutes_preferences())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_minutes_preferences", message)
    })
}

#[tauri::command]
pub(crate) async fn get_minutes_prompt_preferences(
    window: WebviewWindow,
) -> Result<
    storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
    mon_contracts::AppError,
> {
    let result: Result<
        storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
        String,
    > = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().minutes_prompt_preferences()
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_minutes_prompt_preferences", message)
    })
}

#[tauri::command]
pub(crate) async fn save_minutes_prompt_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::minutes_prompt_preferences::MinutesPromptPreferences,
) -> Result<
    storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
    mon_contracts::AppError,
> {
    let result: Result<
        storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
        String,
    > = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .save_minutes_prompt_preferences(expected_lock_version, payload)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_minutes_prompt_preferences", message)
    })
}

#[tauri::command]
pub(crate) async fn get_minutes_markdown(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<storage::minutes_markdown::MarkdownDocument>, mon_contracts::AppError> {
    let result: Result<Option<storage::minutes_markdown::MarkdownDocument>, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().get_minutes_markdown(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_minutes_markdown", message))
}

#[tauri::command]
pub(crate) async fn get_meeting_body_for_review(
    window: WebviewWindow,
    meeting_id: String,
    part: mon_contracts::MeetingBodyPart,
) -> Result<Option<mon_contracts::MeetingBodyReview>, mon_contracts::AppError> {
    let result: Result<Option<mon_contracts::MeetingBodyReview>, String> = async move {
        require_main_window(&window, "会議本文はメイン画面だけで確認できます")?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .meeting_body_for_review(&meeting_id, part)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_body_for_review", message)
    })
}

#[tauri::command]
pub(crate) async fn save_minutes_markdown(
    window: WebviewWindow,
    request: storage::minutes_markdown::SaveMarkdownRequest,
) -> Result<storage::minutes_markdown::MarkdownDocument, mon_contracts::AppError> {
    let result: Result<storage::minutes_markdown::MarkdownDocument, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().save_minutes_markdown(request)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("save_minutes_markdown", message))
}
