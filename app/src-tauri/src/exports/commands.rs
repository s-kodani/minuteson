use crate::analysis_worker::AnalysisController;
use crate::exports;
use crate::generation::commands::minutes_app;
use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportStatus {
    active: bool,
    record: Option<storage::ExportRecord>,
}

#[tauri::command]
pub(crate) async fn export_plain_minutes_markdown(
    window: WebviewWindow,
    request: storage::markdown_exports::MarkdownExportRequest,
) -> Result<Option<storage::ExportRecord>, mon_contracts::AppError> {
    let result: Result<Option<storage::ExportRecord>, String> = async move {
        let app = minutes_app(&window)?;
        app.state::<Storage>().require_data_writable()?;
        let meeting_owner =
            app.state::<Storage>()
                .begin_meeting_work(&request.meeting_id, "export", None)?;
        let admission = app.state::<AnalysisController>().begin_export()?;
        let lease = app.state::<exports::ExportController>().begin()?;
        // A lost receiver does not release admission while the native dialog or
        // durable publication is still alive. Status reads never reopen a dialog.
        tauri::async_runtime::spawn(async move {
            let _admission = admission;
            let _meeting_owner = meeting_owner;
            let _lease = lease;
            let worker = app.clone();
            let query = request.clone();
            if let Some(record) = tauri::async_runtime::spawn_blocking(move || {
                worker.state::<Storage>().existing_markdown_export(&query)
            })
            .await
            .map_err(|e| e.to_string())??
            {
                return Ok(Some(record));
            }
            let worker = app.clone();
            let prepared = tauri::async_runtime::spawn_blocking(move || {
                worker.state::<Storage>().prepare_markdown_export(request)
            })
            .await
            .map_err(|e| e.to_string())??;
            let Some(path) = exports::save_dialog(&window, prepared.filename.clone()).await? else {
                return Ok(None);
            };
            tauri::async_runtime::spawn_blocking(move || {
                app.state::<Storage>()
                    .write_markdown_export(prepared, &path)
            })
            .await
            .map_err(|e| e.to_string())?
            .map(Some)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("export_plain_minutes_markdown", message)
    })
}

#[tauri::command]
pub(crate) async fn get_plain_minutes_export_status(
    window: WebviewWindow,
    request: storage::markdown_exports::MarkdownExportRequest,
) -> Result<ExportStatus, mon_contracts::AppError> {
    let result: Result<ExportStatus, String> = async move {
        let app = minutes_app(&window)?;
        // A publication can finish between the record query and the final active
        // read. If it was alive before querying, require another read rather than
        // misreporting an absent receipt as a safely stopped operation.
        let was_active = app.state::<exports::ExportController>().active();
        let worker = app.clone();
        let record = tauri::async_runtime::spawn_blocking(move || {
            worker.state::<Storage>().existing_markdown_export(&request)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(ExportStatus {
            active: was_active || app.state::<exports::ExportController>().active(),
            record,
        })
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_plain_minutes_export_status", message)
    })
}
