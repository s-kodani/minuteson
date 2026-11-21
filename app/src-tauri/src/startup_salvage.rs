//! Native entry points for partial salvage. Folder paths and plans never
//! arrive from the webview. In the recovery process normal Storage is absent.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{AppHandle, Manager, WebviewWindow};

#[cfg(any(target_os = "macos", windows))]
use crate::bootstrap::salvage::Session;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    phase: &'static str,
    progress: Option<crate::operation_observation::Progress>,
    error: Option<String>,
    #[cfg(any(target_os = "macos", windows))]
    selection: Option<crate::bootstrap::salvage::View>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "idle",
            progress: None,
            error: None,
            #[cfg(any(target_os = "macos", windows))]
            selection: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct Salvage {
    busy: AtomicBool,
    cancel: AtomicBool,
    engaged: AtomicBool,
    handoff: AtomicBool,
    status: Mutex<Status>,
    #[cfg(any(target_os = "macos", windows))]
    session: Mutex<Option<Session>>,
}

impl Salvage {
    pub(crate) fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub(crate) fn engaged(&self) -> bool {
        self.engaged.load(Ordering::Acquire) || self.busy() || self.handoff.load(Ordering::Acquire)
    }

    pub(crate) fn begin_handoff(&self) -> Result<Handoff<'_>, String> {
        self.handoff
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "データ復元を開く準備をしています")?;
        Ok(Handoff(self))
    }

    fn snapshot(&self) -> Result<Status, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|e| e.to_string())
    }

    fn progress_snapshot(&self) -> Result<Status, String> {
        let status = self.status.lock().map_err(|e| e.to_string())?;
        if status.phase == "idle" {
            return Ok(status.clone());
        }
        Ok(Status {
            phase: status.phase,
            progress: status.progress.clone(),
            ..Status::default()
        })
    }

    #[cfg(any(target_os = "macos", windows))]
    fn begin(&self) -> Result<Busy<'_>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "復元操作は処理中です")?;
        self.cancel.store(false, Ordering::Release);
        Ok(Busy(self))
    }

    #[cfg(any(target_os = "macos", windows))]
    fn with_session<T>(
        &self,
        app: &AppHandle,
        action: impl FnOnce(&mut Session) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self.session.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let config = app.path().app_config_dir().map_err(|e| e.to_string())?;
            let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
            *guard = Some(Session::open(&config, &data)?);
            self.engaged.store(true, Ordering::Release);
        }
        let session = guard.as_mut().ok_or("復元操作の状態を取得できません")?;
        action(session)
    }

    #[cfg(any(target_os = "macos", windows))]
    fn start_phase(&self, phase: &'static str) -> Result<(), String> {
        let mut status = self.status.lock().map_err(|e| e.to_string())?;
        status.phase = phase;
        status.progress = None;
        status.error = None;
        Ok(())
    }

    #[cfg(any(target_os = "macos", windows))]
    fn finish(
        &self,
        outcome: Result<crate::bootstrap::salvage::View, String>,
    ) -> Result<Status, String> {
        let mut status = self.status.lock().map_err(|e| e.to_string())?;
        status.phase = "idle";
        status.progress = None;
        match outcome {
            Ok(view) => {
                status.selection = Some(view);
                status.error = None;
            }
            Err(error) => {
                status.error = Some(error);
            }
        }
        Ok(status.clone())
    }
}

pub(crate) struct Handoff<'a>(&'a Salvage);

impl Drop for Handoff<'_> {
    fn drop(&mut self) {
        self.0.handoff.store(false, Ordering::Release);
    }
}

#[cfg(any(target_os = "macos", windows))]
struct Busy<'a>(&'a Salvage);

#[cfg(any(target_os = "macos", windows))]
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

pub(crate) fn allowed_while_engaged(command: &str) -> bool {
    matches!(
        command,
        "get_window_environment"
            | "get_startup_report"
            | "get_residency_state"
            | "record_background_read_failure"
            | "hide_main_to_background"
            | "get_salvage_state"
            | "recheck_salvage_environment"
            | "open_salvage"
            | "choose_salvage_folder"
            | "inspect_salvage"
            | "commit_salvage"
            | "cancel_salvage_work"
            | "leave_salvage"
    )
}

pub(crate) fn require_idle(app: &AppHandle) -> Result<(), String> {
    if app.state::<Salvage>().engaged() {
        Err("復元画面で処理の完了または取消を確認してください".into())
    } else {
        Ok(())
    }
}

fn require_blocked(window: &WebviewWindow, command: &str) -> Result<(), String> {
    window
        .state::<crate::startup_attention::Startup>()
        .admit_command(window.label(), command)?;
    crate::startup_reset::check_idle(window.app_handle())?;
    if window.try_state::<crate::storage::Storage>().is_some() {
        return Err("会議保存先を開いています。アプリを終了して保存先を解放してください".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn recheck_salvage_environment(
    window: WebviewWindow,
) -> Result<Vec<String>, mon_contracts::AppError> {
    let result: Result<Vec<String>, String> = async move {
        let cmd = "recheck_salvage_environment";
        require_blocked(&window, cmd)?;
        let (meeting, config) = window
            .state::<crate::startup_attention::Startup>()
            .failure_paths()?;
        let config = config.or_else(|| window.app_handle().path().app_config_dir().ok());
        tauri::async_runtime::spawn_blocking(move || {
            [("会議保存先", meeting), ("起動設定のフォルダー", config)]
                .into_iter()
                .map(|(label, path)| match path {
                    Some(path) => match std::fs::read_dir(&path) {
                        Ok(_) => format!("{label}を読み取れます。{}", path.display()),
                        Err(error) => {
                            format!("{label}を読み取れません。{} ({error})", path.display())
                        }
                    },
                    None => format!("{label}の場所を確認できません。"),
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| error.to_string())
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("recheck_salvage_environment", message)
    })
}

#[tauri::command]
pub(crate) fn get_salvage_state(window: WebviewWindow) -> Result<Status, mon_contracts::AppError> {
    let result = require_blocked(&window, "get_salvage_state")
        .and_then(|_| window.state::<Salvage>().progress_snapshot());
    result.map_err(|message| mon_contracts::AppError::from_command("get_salvage_state", message))
}

#[tauri::command]
pub(crate) async fn open_salvage(window: WebviewWindow) -> Result<Status, mon_contracts::AppError> {
    run(window, Action::Open, "open_salvage").await
}

#[tauri::command]
pub(crate) async fn export_salvage_edits(
    window: WebviewWindow,
    contents: String,
) -> Result<bool, mon_contracts::AppError> {
    let result: Result<bool, String> = async move {
        window
            .state::<crate::startup_attention::Startup>()
            .admit_command(window.label(), "export_salvage_edits")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            app.state::<crate::bootstrap::Bootstrap>()
                .require_ready_salvage(&app.state::<crate::storage::Storage>())?;
            let lease = app.state::<crate::exports::ExportController>().begin()?;
            tauri::async_runtime::spawn(async move {
                // Keep the export lease until the save dialog and durable write finish.
                let _lease = lease;
                validate_edit_snapshot(&contents)?;
                let selected = crate::exports::save_dialog_with(
                    &window,
                    "MinutesOn-unsaved-edits.json".into(),
                    "未保存の入力をファイルに退避",
                    "json",
                )
                .await?;
                let Some(path) = selected else {
                    return Ok(false);
                };
                tauri::async_runtime::spawn_blocking(move || {
                    let storage = app.state::<crate::storage::Storage>();
                    crate::exports::Destination::selected_edit_snapshot(storage.root(), &path)?
                        .publish(contents.as_bytes())
                        .map_err(|error| error.message)?;
                    Ok(true)
                })
                .await
                .map_err(|error| error.to_string())?
            })
            .await
            .map_err(|error| error.to_string())?
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = contents;
            Err("このOSでは未保存の入力を別のファイルに保存できません".into())
        }
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("export_salvage_edits", message))
}

#[cfg(any(target_os = "macos", windows))]
fn validate_edit_snapshot(contents: &str) -> Result<(), String> {
    if contents.len() > 64 * 1024 * 1024 {
        return Err("入力の量が多すぎるため、別のファイルに保存できませんでした".into());
    }
    let snapshot: serde_json::Value = serde_json::from_str(contents)
        .map_err(|_| "ファイルに保存する入力を確認できませんでした")?;
    if snapshot["schemaVersion"].as_u64() != Some(1)
        || snapshot["edits"].as_array().is_none_or(Vec::is_empty)
    {
        return Err("ファイルに保存する入力を確認できませんでした".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn choose_salvage_folder(
    window: WebviewWindow,
    source: bool,
) -> Result<Status, mon_contracts::AppError> {
    let result: Result<Status, String> = async move {
        require_blocked(&window, "choose_salvage_folder")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let state = window.state::<Salvage>();
            let _busy = state.begin()?;
            let title = if source {
                "元の会議データフォルダーを選択"
            } else {
                "復元先の親フォルダーを選択"
            };
            let Some(path) = crate::startup_reset::pick_directory(&window, title).await? else {
                return state.snapshot();
            };
            let app = window.app_handle().clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<Salvage>();
                state.with_session(&app, |session| {
                    let outcome = session.select(path, source);
                    // Workspace cleanup can fail after the prior confirmation
                    // has been invalidated. Publish only the surviving view.
                    if outcome.is_err() {
                        let selection = session.view().ok();
                        state.status.lock().map_err(|e| e.to_string())?.selection = selection;
                    }
                    outcome
                })
            })
            .await
            .map_err(|error| error.to_string())?;
            state.finish(result)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = source;
            Err("このOSではデータ復元を利用できません".into())
        }
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("choose_salvage_folder", message))
}

#[tauri::command]
pub(crate) async fn inspect_salvage(
    window: WebviewWindow,
) -> Result<Status, mon_contracts::AppError> {
    run(window, Action::Inspect, "inspect_salvage").await
}

#[tauri::command]
pub(crate) async fn commit_salvage(
    window: WebviewWindow,
    operation_id: String,
) -> Result<Status, mon_contracts::AppError> {
    run(window, Action::Commit(operation_id), "commit_salvage").await
}

#[tauri::command]
pub(crate) fn cancel_salvage_work(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    require_blocked(&window, "cancel_salvage_work")
        .map_err(|message| mon_contracts::AppError::from_command("cancel_salvage_work", message))?;
    window
        .state::<Salvage>()
        .cancel
        .store(true, Ordering::Release);
    Ok(())
}

#[tauri::command]
pub(crate) async fn leave_salvage(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        require_blocked(&window, "leave_salvage")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<Salvage>();
                let _busy = state.begin()?;
                let mut guard = state.session.lock().map_err(|e| e.to_string())?;
                if let Some(session) = guard.as_ref() {
                    session.leave()?;
                    *guard = None;
                    state.engaged.store(false, Ordering::Release);
                    *state.status.lock().map_err(|e| e.to_string())? = Status::default();
                }
                Ok(())
            })
            .await
            .map_err(|error| error.to_string())?
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        Err("このOSではデータ復元を利用できません".into())
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("leave_salvage", message))
}

#[tauri::command]
pub(crate) fn get_salvage_result(
    window: WebviewWindow,
) -> Result<serde_json::Value, mon_contracts::AppError> {
    let result: Result<serde_json::Value, String> = (|| {
        window
            .state::<crate::startup_attention::Startup>()
            .admit_command(window.label(), "get_salvage_result")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let value = window.state::<crate::storage::Storage>().salvage_result()?;
            serde_json::to_value(value).map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        Ok(serde_json::Value::Null)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_salvage_result", message))
}

#[tauri::command]
pub(crate) async fn get_salvage_source_cleanup(
    window: WebviewWindow,
    operation_id: String,
) -> Result<Option<mon_contracts::SalvageSourceCleanup>, mon_contracts::AppError> {
    let result: Result<Option<mon_contracts::SalvageSourceCleanup>, String> = async move {
        window
            .state::<crate::startup_attention::Startup>()
            .admit_command(window.label(), "get_salvage_source_cleanup")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                if !app
                    .state::<crate::storage::Storage>()
                    .has_current_salvage_result(&operation_id)?
                {
                    return Ok(None);
                }
                app.state::<crate::bootstrap::Bootstrap>()
                    .salvage_source_cleanup(&operation_id)
            })
            .await
            .map_err(|error| error.to_string())?
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = operation_id;
            Ok(None)
        }
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_salvage_source_cleanup", message)
    })
}

#[tauri::command]
pub(crate) async fn delete_salvage_source(
    window: WebviewWindow,
    operation_id: String,
) -> Result<mon_contracts::SalvageSourceCleanup, mon_contracts::AppError> {
    let result: Result<mon_contracts::SalvageSourceCleanup, String> = async move {
        window
            .state::<crate::startup_attention::Startup>()
            .admit_command(window.label(), "delete_salvage_source")?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::data_controller::DataController::delete_salvage_source(&app, &operation_id)
            })
            .await
            .map_err(|error| error.to_string())?
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = operation_id;
            Err("このOSでは復元元データを削除できません".into())
        }
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("delete_salvage_source", message))
}

enum Action {
    Open,
    Inspect,
    Commit(String),
}

async fn run(
    window: WebviewWindow,
    action: Action,
    command: &'static str,
) -> Result<Status, mon_contracts::AppError> {
    let result: Result<Status, String> = async move {
        require_blocked(&window, command)?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<Salvage>();
                let _busy = state.begin()?;
                let phase = match &action {
                    Action::Open => "opening",
                    Action::Inspect => "inspecting",
                    Action::Commit(_) => "building",
                };
                state.start_phase(phase)?;
                let cancelled = || state.cancel.load(Ordering::Acquire);
                let result = {
                    let progress_app = app.clone();
                    let _observation =
                        crate::operation_observation::Scope::enter("salvage", move |progress| {
                            let state = progress_app.state::<Salvage>();
                            if let Ok(mut status) = state.status.lock() {
                                status.progress = Some(progress);
                            };
                        });
                    state.with_session(&app, |session| match action {
                        Action::Open => session.view(),
                        Action::Inspect => session.inspect(&cancelled),
                        Action::Commit(id) => session.commit(&id, &cancelled),
                    })
                };
                state.finish(result)
            })
            .await
            .map_err(|e| e.to_string())?
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            if let Action::Commit(id) = action {
                drop(id);
            }
            Err("このOSではデータ復元を利用できません".into())
        }
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command(command, message))
}
