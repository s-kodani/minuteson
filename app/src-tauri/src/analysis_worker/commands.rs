use super::AnalysisController;
use crate::storage::{self, Storage};
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn get_analysis_jobs(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Vec<storage::JobSnapshot>, mon_contracts::AppError> {
    let result: Result<Vec<storage::JobSnapshot>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis jobs".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().list_jobs(&meeting_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_analysis_jobs", message))
}

#[tauri::command]
pub(crate) async fn get_analysis_notification_jobs(
    window: WebviewWindow,
    watched_job_ids: Vec<String>,
) -> Result<Vec<storage::NotificationJob>, mon_contracts::AppError> {
    let result: Result<Vec<storage::NotificationJob>, String> = async move {
        if window.label() != "main" {
            return Err("解析通知の状態はメイン画面から確認してください".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().notification_jobs(&watched_job_ids)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_notification_jobs", message)
    })
}

#[tauri::command]
pub(crate) async fn get_analysis_notification_output(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::NotificationOutput, mon_contracts::AppError> {
    let result: Result<storage::NotificationOutput, String> = async move {
        if window.label() != "main" {
            return Err("解析通知の保存結果はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().notification_output(&job_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_notification_output", message)
    })
}

#[tauri::command]
pub(crate) fn pause_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can pause analysis".into());
        }
        window.state::<Storage>().request_job_pause(&job_id, "user")
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("pause_analysis_job", message))
}

#[tauri::command]
pub(crate) fn resume_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can resume analysis".into());
        }
        let job = window.state::<Storage>().resume_job(&job_id)?;
        AnalysisController::kick(window.app_handle())?;
        Ok(job)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("resume_analysis_job", message))
}

#[tauri::command]
pub(crate) async fn cancel_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can cancel analysis".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let storage = app.state::<Storage>();
            let job = storage.get_job(&job_id)?;
            if job.state == "running" {
                storage.request_job_pause(&job_id, "user")?;
            }
            let started = std::time::Instant::now();
            loop {
                let job = storage.get_job(&job_id)?;
                if job.state != "pausing" {
                    return storage.finish_job(&job_id, job.completed_units, "cancelled", None);
                }
                if started.elapsed() > std::time::Duration::from_secs(15) {
                    return Err(
                        "解析の安全な停止を待っています。完了後に取り消しを再試行してください"
                            .into(),
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_analysis_job", message))
}
