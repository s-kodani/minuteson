use crate::analysis_worker::AnalysisController;
use crate::input_status;
use crate::recorder::RecorderController;
use crate::storage::Storage;
use tauri::{Manager, WebviewWindow};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoticeReply<T> {
    paused: bool,
    value: Option<T>,
}

impl<T> NoticeReply<T> {
    fn ready(value: T) -> Self {
        Self {
            paused: false,
            value: Some(value),
        }
    }
    fn paused() -> Self {
        Self {
            paused: true,
            value: None,
        }
    }
}

#[tauri::command]
pub(crate) async fn get_archived_recording_input_notices(
    window: WebviewWindow,
    after_session_id: Option<String>,
) -> Result<NoticeReply<input_status::archived::Page>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::archived::Page>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        // The blocking worker owns admission until its actual I/O ends, even if the
        // WebView/awaiting caller disappears. Do not hold any recorder/child lock.
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            if analysis.notice_storage_paused(None) {
                return Ok(NoticeReply::paused());
            }
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .page_scoped(
                    &app.state::<Storage>(),
                    &analysis,
                    after_session_id.as_deref(),
                    &visible,
                )
                .map(NoticeReply::ready)
                .or_else(|error| {
                    if error == input_status::archived::PAUSED {
                        Ok(NoticeReply::paused())
                    } else {
                        Err(error)
                    }
                })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_archived_recording_input_notices", message)
    })
}

#[tauri::command]
pub(crate) async fn get_archived_recording_input_notice_rows(
    window: WebviewWindow,
    targets: Vec<input_status::archived::Target>,
) -> Result<NoticeReply<input_status::archived::SelectedRows>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::archived::SelectedRows>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .selected(&app.state::<Storage>(), &analysis, targets, &visible)
                .map(NoticeReply::ready)
                .or_else(|error| {
                    if error == input_status::archived::PAUSED {
                        Ok(NoticeReply::paused())
                    } else {
                        Err(error)
                    }
                })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_archived_recording_input_notice_rows", message)
    })
}

#[tauri::command]
pub(crate) async fn acknowledge_archived_recording_input_notice(
    window: WebviewWindow,
    request: input_status::archived::Acknowledge,
) -> Result<NoticeReply<input_status::Receipt>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::Receipt>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知の受付・表示・終了はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            let lease = match analysis.try_begin_scoped_notice_storage(Some(&request.meeting_id))? {
                Some(lease) => lease,
                None => return Ok(NoticeReply::paused()),
            };
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .acknowledge(&app.state::<Storage>(), &lease, request, &visible)
                .map(NoticeReply::ready)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command(
            "acknowledge_archived_recording_input_notice",
            message,
        )
    })
}

#[tauri::command]
pub(crate) async fn receive_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: Option<u64>,
    receive_id: Option<String>,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力通知は録音画面から受付してください".into());
        }
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                generation,
                input_status::queue::Action::Receive,
                receive_id.as_deref(),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("receive_recording_input_notice", message)
    })
}

#[tauri::command]
pub(crate) async fn render_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: u64,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                Some(generation),
                input_status::queue::Action::Render,
                None,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("render_recording_input_notice", message)
    })
}

#[tauri::command]
pub(crate) async fn close_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: u64,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                Some(generation),
                input_status::queue::Action::Close,
                None,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("close_recording_input_notice", message)
    })
}
