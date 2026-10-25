//! The recovery window never opens storage. Restart always re-enters admission
//! in a new process, rather than resetting a confirmed startup report in place.
use serde::Serialize;
use tauri::{Manager, WebviewWindow};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) fn supervisor_entry() -> Option<i32> {
    windows::supervisor_entry()
}
#[cfg(windows)]
pub(crate) fn prepare_supervisor() {
    windows::prepare_supervisor();
}
#[cfg(windows)]
pub(crate) fn residency_pulse(probe: &crate::residency::Probe) -> Result<bool, String> {
    windows::residency_pulse(probe)
}
#[cfg(windows)]
pub(crate) fn residency_available() -> bool {
    windows::residency_available()
}
#[derive(Default)]
pub(crate) struct Instance {
    activation: std::sync::Mutex<Option<(String, std::sync::mpsc::SyncSender<bool>)>>,
    #[cfg(windows)]
    inner: std::sync::Mutex<Option<windows::Coordinator>>,
}
#[tauri::command]
pub(crate) fn acknowledge_startup_activation(
    window: WebviewWindow,
    request_id: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("メイン画面だけで応答できます".into());
    }
    let state = window.state::<Instance>();
    let mut pending = state
        .activation
        .lock()
        .map_err(|_| "画面表示の確認を読めません")?;
    if pending.as_ref().is_none_or(|(id, _)| *id != request_id) {
        return Err("画面表示の確認は期限切れです".into());
    }
    if let Some((_, sender)) = pending.take() {
        let _ = sender.send(true);
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    schema_version: u8,
    supported: bool,
    target_id: Option<String>,
    can_restart: bool,
    message: String,
}
impl Instance {
    fn report(&self) -> Result<Report, String> {
        #[cfg(windows)]
        {
            let state = self
                .inner
                .try_lock()
                .map_err(|_| "起動復旧の処理中です。操作の応答を待ってください")?;
            if let Some(coordinator) = state.as_ref() {
                let mut report = coordinator.report();
                if report.can_restart {
                    if let Err(error) = windows::restart_available() {
                        report.can_restart = false;
                        report.target_id = None;
                        report.message = error;
                    }
                }
                return Ok(report);
            }
        }
        Ok(Report {
            schema_version: 1,
            supported: cfg!(windows),
            target_id: None,
            can_restart: false,
            message: String::new(),
        })
    }
}
#[cfg(windows)]
pub(crate) fn initialize(app: &tauri::AppHandle, config: &std::path::Path) -> Result<bool, String> {
    let mut coordinator = windows::Coordinator::new(app, config)?;
    let proceed = coordinator.enter(app)?;
    *app.state::<Instance>()
        .inner
        .lock()
        .map_err(|_| "起動連携を保持できません")? = Some(coordinator);
    Ok(proceed)
}
fn require_recovery(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main"
        || !window
            .state::<crate::startup_attention::Startup>()
            .blocked()
    {
        return Err("起動復旧のメイン画面だけで操作できます".into());
    }
    if !cfg!(windows) {
        return Err("この復旧操作はWindows版で利用できます".into());
    }
    Ok(())
}
#[tauri::command]
pub(crate) fn get_startup_instance(window: WebviewWindow) -> Result<Report, String> {
    if window.label() != "main" {
        return Err("起動連携はメイン画面だけで確認できます".into());
    }
    window.state::<Instance>().report()
}
#[tauri::command]
pub(crate) async fn open_existing_instance(window: WebviewWindow) -> Result<bool, String> {
    require_recovery(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            let state = window.state::<Instance>();
            let mut coordinator = state.inner.try_lock().map_err(|_| "起動復旧の処理中です")?;
            let opened = coordinator
                .as_mut()
                .ok_or("起動連携を準備できていません")?
                .open_existing()?;
            if opened {
                exit_secondary(window.app_handle());
            }
            return Ok(opened);
        }
        #[cfg(not(windows))]
        Err("Windows版で利用できます".into())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) async fn recover_startup_instance(
    window: WebviewWindow,
    target_id: String,
) -> Result<(), String> {
    require_recovery(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            let state = window.state::<Instance>();
            let mut coordinator = state.inner.try_lock().map_err(|_| "起動復旧の処理中です")?;
            let coordinator = coordinator.as_mut().ok_or("復旧対象を確認できません")?;
            windows::restart_available()?;
            if coordinator.recover(&target_id)? {
                exit_secondary(window.app_handle());
            } else {
                windows::schedule_restart()?;
                coordinator.release();
                exit_secondary(window.app_handle());
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            let _ = target_id;
            Err("Windows版で利用できます".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn retry_startup(window: WebviewWindow) -> Result<(), String> {
    require_recovery(&window)?;
    #[cfg(windows)]
    {
        let state = window.state::<Instance>();
        let mut owner = state.inner.try_lock().map_err(|_| "起動復旧の処理中です")?;
        windows::schedule_restart()?;
        if let Some(coordinator) = owner.as_mut() {
            coordinator.release();
        }
        exit_secondary(window.app_handle());
        return Ok(());
    }
    #[cfg(not(windows))]
    window.app_handle().restart();
}
#[cfg(windows)]
pub(super) fn exit_secondary(app: &tauri::AppHandle) {
    app.state::<crate::ExitGuard>()
        .0
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
}
