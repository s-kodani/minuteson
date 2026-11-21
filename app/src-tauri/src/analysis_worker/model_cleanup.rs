use super::{data_maintenance::DataMaintenanceLease, model_preparation::ModelPreparationLease, *};
use crate::storage::deletion::models::cleanup;
use crate::{import_controller::ImportController, job_history, session::SessionStore};
use serde::Serialize;
use tauri::WebviewWindow;

pub(crate) use cleanup::Job;

const PENDING: &str = "モデル設定は反映済み。重複モデルの整理は未完了";

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub(super) revision: u64,
    pub(super) active: Option<Job>,
}

struct Running<'a> {
    app: &'a AppHandle,
    lease: Option<DataMaintenanceLease>,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        drop(self.lease.take());
        publish(self.app);
        let _ = self.app.emit("model-inventory-changed", ());
    }
}

fn publish(app: &AppHandle) {
    if let Ok(snapshot) = app.state::<AnalysisController>().model_cleanup_snapshot() {
        let _ = app.emit_to("main", "model-cleanup-changed", snapshot);
    }
}

fn history(job: &Job, state: &str, detail: Option<&str>) {
    let mut entry = job_history::Entry::new("model", &job.id, &job.target, None, state);
    entry.detail = detail.map(str::to_owned);
    if state == "attention" {
        entry.finished_at = Some(entry.observed_at);
    }
    job_history::record(entry);
}

fn job(id: Option<&str>, target: &str, applied_package: Option<&str>) -> Job {
    Job {
        id: id.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned),
        target: if target.is_empty() {
            "ローカルAI"
        } else {
            target
        }
        .to_owned(),
        started_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        applied_package: applied_package.map(str::to_owned),
    }
}

impl AnalysisController {
    fn model_cleanup_snapshot(&self) -> Result<Snapshot, String> {
        self.state
            .lock()
            .map(|state| state.model_cleanup.clone())
            .map_err(|_| "モデル整理の状態を確認できません".into())
    }

    pub(crate) fn model_cleanup_active(&self) -> bool {
        self.model_cleanup_snapshot()
            .is_ok_and(|state| state.active.is_some())
    }

    fn start_model_cleanup<'a>(
        app: &'a AppHandle,
        job: &Job,
        preparation: Option<&ModelPreparationLease<'_>>,
    ) -> Result<Option<Running<'a>>, String> {
        let importing = app.state::<ImportController>().snapshot()?;
        let recording = app.state::<SessionStore>().snapshot().is_active();
        let lease = app.state::<Self>().begin_model_cleanup(
            job.clone(),
            recording,
            importing.busy || importing.recording_blocked,
            preparation.map(|lease| &lease.cancel),
        )?;
        Ok(lease.map(|lease| {
            let running = Running {
                app,
                lease: Some(lease),
            };
            publish(app);
            history(job, "running", None);
            running
        }))
    }

    fn execute_model_cleanup(
        app: &AppHandle,
        job: &Job,
        running: &Running<'_>,
        targets: Vec<cleanup::Target>,
    ) -> Result<(), String> {
        let lease = running.lease.as_ref().unwrap();
        let root = app.state::<Storage>().native_models_root()?.to_owned();
        let storage = app.state::<Storage>();
        let _measure = crate::operation_observation::Scope::enter("model_cleanup", |_| {});
        cleanup::execute(&storage, &root, targets, job, lease)
    }

    pub(crate) fn finish_model_application(
        app: &AppHandle,
        preparation: &ModelPreparationLease<'_>,
        history_id: Option<&str>,
        target: &str,
        package: &str,
    ) {
        let controller = app.state::<Self>();
        controller.model_application_finished();
        let job = job(history_id, target, Some(package));
        let mut started = false;
        let result = (|| -> Result<bool, String> {
            let storage = app.state::<Storage>();
            if cleanup::plan(&storage, Some(package))?.is_empty() {
                return Ok(true);
            }
            let Some(running) = Self::start_model_cleanup(app, &job, Some(preparation))? else {
                return Ok(false);
            };
            started = true;
            let targets = cleanup::plan(&storage, Some(package))?;
            Self::execute_model_cleanup(app, &job, &running, targets)?;
            Ok(true)
        })();
        match result {
            Ok(true) if history_id.is_some() || started => history(&job, "succeeded", None),
            Ok(true) => {}
            Ok(false) => history(&job, "attention", Some(PENDING)),
            Err(error) => {
                controller
                    .model_cleanup_disabled
                    .store(true, Ordering::Release);
                crate::internal_logs::report_error("model_cleanup", &error);
                history(&job, "attention", Some(PENDING));
            }
        }
    }

    pub(super) fn cleanup_models(app: &AppHandle) {
        let controller = app.state::<Self>();
        let storage = app.state::<Storage>();
        while !storage.data_write_blocked()
            && !controller.shutdown.load(Ordering::Acquire)
            && !controller.model_cleanup_disabled.load(Ordering::Acquire)
            && !controller.model_cleanup_suspended.load(Ordering::Acquire)
        {
            let result = (|| -> Result<bool, String> {
                let Some(target) = cleanup::plan(&storage, None)?.into_iter().next() else {
                    return Ok(false);
                };
                let job = target
                    .job()
                    .cloned()
                    .unwrap_or_else(|| job(None, "重複モデルの整理", None));
                let Some(running) = Self::start_model_cleanup(app, &job, None)? else {
                    return Ok(false);
                };
                let result = (|| {
                    let mut targets: Vec<_> = cleanup::plan(&storage, None)?
                        .into_iter()
                        .filter(|updated| updated.package() == target.package())
                        .take(1)
                        .collect();
                    if let Some(package) = job.applied_package.as_deref() {
                        targets.extend(cleanup::plan(&storage, Some(package))?);
                    }
                    Self::execute_model_cleanup(app, &job, &running, targets)
                })();
                let cancelled = running
                    .lease
                    .as_ref()
                    .is_some_and(DataMaintenanceLease::cancelled);
                drop(running);
                match &result {
                    Ok(()) => history(&job, "succeeded", None),
                    Err(_) if cancelled => {
                        history(&job, "interrupted", Some("重複モデルの整理を中断しました"))
                    }
                    Err(_) if job.applied_package.is_some() => {
                        history(&job, "attention", Some(PENDING))
                    }
                    Err(_) => history(
                        &job,
                        "failed",
                        Some("重複モデルの整理を完了できませんでした"),
                    ),
                }
                if cancelled {
                    Ok(false)
                } else {
                    result.map(|_| true)
                }
            })();
            match result {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    controller
                        .model_cleanup_disabled
                        .store(true, Ordering::Release);
                    crate::internal_logs::report_error("model_cleanup", &error);
                    break;
                }
            }
        }
    }
}

#[tauri::command]
pub(crate) fn get_model_cleanup(
    window: WebviewWindow,
) -> Result<Snapshot, mon_contracts::AppError> {
    let app = crate::data_app(&window)
        .map_err(|error| mon_contracts::AppError::from_command("get_model_cleanup", error))?;
    app.state::<AnalysisController>()
        .model_cleanup_snapshot()
        .map_err(|error| mon_contracts::AppError::from_command("get_model_cleanup", error))
}
