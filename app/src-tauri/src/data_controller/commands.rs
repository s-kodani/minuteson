use crate::analysis_worker::AnalysisController;
#[cfg(any(target_os = "macos", windows))]
use crate::bootstrap;
use crate::data_controller::{self, DataController};
use crate::import_controller::ImportController;
use crate::session::SessionStore;
use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

pub(crate) fn data_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    if window.label() != "main" {
        return Err("Only the main window can manage stored data".into());
    }
    Ok(window.app_handle().clone())
}

#[tauri::command]
pub(crate) fn get_data_operation(
    window: WebviewWindow,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> =
        (|| data_app(&window)?.state::<DataController>().snapshot())();
    result.map_err(|message| mon_contracts::AppError::from_command("get_data_operation", message))
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn get_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            storage::deletion::operations::listing(&app.state::<Storage>())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_deletion_items", message))
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn review_meeting_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_deletion(&app, meeting_id, false)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn review_pending_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_deletion(&app, operation_id, true)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[cfg(any(target_os = "macos", windows))]
#[tauri::command]
pub(crate) async fn get_storage_bootstrap(
    window: WebviewWindow,
) -> Result<bootstrap::View, mon_contracts::AppError> {
    let result: Result<bootstrap::View, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let selection = app.state::<bootstrap::Bootstrap>();
            let mut view = selection.view()?;
            view.recovery = selection.recovery_view_for_storage(&app.state::<Storage>())?;
            Ok(view)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_storage_bootstrap", message))
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) fn review_model_body_deletion(
    window: WebviewWindow,
    package_id: String,
    expected_review_hash: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = (|| {
        DataController::review_model_deletion(
            &data_app(&window)?,
            data_controller::ModelTarget::Fresh {
                package: package_id,
                review: expected_review_hash,
            },
        )
    })();
    result
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) fn review_pending_model_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = (|| {
        DataController::review_model_deletion(
            &data_app(&window)?,
            data_controller::ModelTarget::Pending(operation_id),
        )
    })();
    result
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn get_model_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::models::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::models::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().list_model_deletion_items()
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_model_deletion_items", message)
    })
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn get_audio_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::audio::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::audio::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            storage::deletion::audio::operations::listing(&app.state::<Storage>())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_audio_deletion_items", message)
    })
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn review_meeting_audio_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_audio_deletion(
                &app,
                data_controller::AudioTarget::Fresh(meeting_id),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn review_pending_audio_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_audio_deletion(
                &app,
                data_controller::AudioTarget::Pending(operation_id),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) fn get_audio_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::audio::operations::PathPage, mon_contracts::AppError> {
    let result: Result<storage::deletion::audio::operations::PathPage, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .audio_deletion_page(&confirmation_id, offset)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_audio_deletion_paths", message)
    })
}

#[cfg(any(target_os = "macos", windows))]
#[tauri::command]
pub(crate) async fn review_storage_move(
    window: WebviewWindow,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || DataController::review_move(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("review_storage_move", message))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataAvailability {
    available: bool,
    reason: Option<String>,
    reason_code: Option<&'static str>,
    deletion_available: bool,
    deletion_reason: Option<String>,
    deletion_reason_code: Option<&'static str>,
}

#[tauri::command]
pub(crate) async fn get_meeting_deletion_availability(
    window: WebviewWindow,
    meeting_ids: Vec<String>,
) -> Result<Vec<storage::meeting_access::Availability>, mon_contracts::AppError> {
    let result: Result<Vec<storage::meeting_access::Availability>, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let reason = app
                .state::<AnalysisController>()
                .meeting_deletion_blocker()?;
            let mut result = app
                .state::<Storage>()
                .meeting_deletion_availability(&meeting_ids)?;
            let recording = app.state::<SessionStore>().snapshot();
            if recording.is_active() {
                for item in &mut result {
                    if recording.meeting_id.is_none()
                        || recording.meeting_id.as_deref() == Some(item.meeting_id.as_str())
                    {
                        item.available = false;
                        if !item.blockers.iter().any(|b| b.kind == "recording") {
                            item.blockers.push(storage::meeting_access::Blocker {
                                kind: "recording".into(),
                                reason: "この会議の録音・音声保存が完了していません".into(),
                                job_id: None,
                            });
                        }
                    }
                }
            }
            if let Some(reason) = reason {
                for item in &mut result {
                    item.available = false;
                    item.blockers.push(storage::meeting_access::Blocker {
                        kind: "storage".into(),
                        reason: reason.clone(),
                        job_id: None,
                    });
                }
            }
            Ok(result)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_deletion_availability", message)
    })
}

#[tauri::command]
pub(crate) fn get_data_operation_availability(
    window: WebviewWindow,
) -> Result<DataAvailability, mon_contracts::AppError> {
    let result: Result<DataAvailability, String> = (|| {
        let app = data_app(&window)?;
        let importing = app.state::<ImportController>().snapshot()?;
        let (reason, reason_code) = if app.state::<SessionStore>().snapshot().is_active() {
            (Some("録音・音声保存中".into()), Some("recording_active"))
        } else if importing.busy || importing.recording_blocked {
            (
                Some("音声取り込み・録音開始の停止待ち中".into()),
                Some("import_active"),
            )
        } else {
            let controller = app.state::<AnalysisController>();
            let kind = controller.data_operation_blocker_kind()?;
            let reason = kind.map(|kind| {
                if kind == mon_core::operation::Kind::ModelDeletion
                    && controller.model_cleanup_active()
                {
                    "重複モデルを整理中".to_owned()
                } else {
                    AnalysisController::blocker_message(kind).to_owned()
                }
            });
            let code = kind.map(|kind| {
                if kind == mon_core::operation::Kind::Restart {
                    "relocation_restart_required"
                } else {
                    "operation_active"
                }
            });
            (reason, code)
        };
        let (reason, reason_code) =
            if reason.is_none() && app.state::<Storage>().has_scoped_meeting_deletion()? {
                (
                    Some("未完了の会議削除があります。削除の確認画面から解決してください".into()),
                    Some("meeting_deletion_pending"),
                )
            } else {
                (reason, reason_code)
            };
        let deletion_reason = reason.clone();
        Ok(DataAvailability {
            available: reason.is_none(),
            reason,
            reason_code,
            deletion_available: deletion_reason.is_none(),
            deletion_reason,
            deletion_reason_code: reason_code,
        })
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_data_operation_availability", message)
    })
}

#[tauri::command]
pub(crate) async fn list_data_jobs(
    window: WebviewWindow,
) -> Result<Vec<data_controller::DataJob>, mon_contracts::AppError> {
    let result: Result<_, String> = (|| data_app(&window)?.state::<DataController>().list_jobs())();
    result.map_err(|message| mon_contracts::AppError::from_command("list_data_jobs", message))
}

#[tauri::command]
pub(crate) async fn dismiss_data_job(
    window: WebviewWindow,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<_, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .dismiss_job(&token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("dismiss_data_job", message))
}

#[tauri::command]
pub(crate) async fn list_restored_meeting_ids(
    window: WebviewWindow,
) -> Result<Vec<String>, mon_contracts::AppError> {
    let result: Result<Vec<String>, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().restored_meeting_ids())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("list_restored_meeting_ids", message)
    })
}

#[tauri::command]
pub(crate) async fn select_data_operation(
    window: WebviewWindow,
    kind: data_controller::Kind,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> = async move {
        data_app(&window)?;
        DataController::select(&window, kind).await
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("select_data_operation", message))
}

#[tauri::command]
pub(crate) async fn start_data_operation(
    window: WebviewWindow,
    request: data_controller::Request,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || DataController::start(&app, request))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[tauri::command]
pub(crate) fn cancel_data_operation(
    window: WebviewWindow,
    token: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> =
        (|| DataController::cancel(&data_app(&window)?, &token))();
    result
        .map_err(|message| mon_contracts::AppError::from_command("cancel_data_operation", message))
}
