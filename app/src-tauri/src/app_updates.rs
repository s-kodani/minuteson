//! Native update ownership. Installation is admitted only through the normal saved-edit close path.
#[cfg(target_os = "macos")]
pub(crate) mod macos;
mod policy;
#[cfg(windows)]
mod windows;
use mon_contracts::{AppUpdateChannel as Channel, AppUpdateStatus};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager, WebviewWindow};

pub(crate) struct Updates {
    model: Mutex<policy::Model>,
    #[cfg(target_os = "macos")]
    pending: Mutex<Option<macos::Pending>>,
    started: AtomicBool,
    stopped: AtomicBool,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            pending: Mutex::new(None),
            model: Mutex::new(policy::Model::new(Channel::Unsupported, String::new())),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }
}
impl Updates {
    fn snapshot(&self) -> AppUpdateStatus {
        self.model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }
}
fn publish(app: &tauri::AppHandle) {
    let _ = app.emit_to(
        "main",
        "app-update-status-changed",
        app.state::<Updates>().snapshot(),
    );
}
pub(crate) fn start(app: &tauri::AppHandle) {
    let state = app.state::<Updates>();
    if state.started.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    let packaged = windows::is_store_package();
    #[cfg(not(windows))]
    let packaged = false;
    let channel = policy::channel(
        std::env::consts::OS,
        &app.config().identifier,
        cfg!(debug_assertions),
        packaged,
    );
    *state.model.lock().unwrap_or_else(|e| e.into_inner()) =
        policy::Model::new(channel, app.package_info().version.to_string());
    publish(app);
    if channel == Channel::Unsupported {
        return;
    }
    request(app);
    let app = app.clone();
    // Native process ownership keeps cadence independent of React and tray visibility.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        if app.state::<Updates>().stopped.load(Ordering::SeqCst) {
            return;
        }
        let due = app
            .state::<Updates>()
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .due(Instant::now());
        if due {
            request(&app);
        }
    });
}
pub(crate) fn stop(app: &tauri::AppHandle) {
    app.state::<Updates>().stopped.store(true, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    app.state::<Updates>()
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
}
fn request(app: &tauri::AppHandle) {
    let state = app.state::<Updates>();
    if state.stopped.load(Ordering::SeqCst) {
        return;
    }
    let channel = {
        let mut model = state.model.lock().unwrap_or_else(|e| e.into_inner());
        if !model.begin(Instant::now()) {
            return;
        }
        model.status.channel
    };
    publish(app);
    match channel {
        #[cfg(target_os = "macos")]
        Channel::MacosRelease => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                macos::check_and_download(&app).await;
            });
        }
        #[cfg(windows)]
        Channel::WindowsStore => windows::request(app),
        _ => complete(app, Err("更新確認の配布経路が一致しません".into())),
    }
}
fn complete(app: &tauri::AppHandle, result: Result<(bool, Option<String>), String>) {
    let state = app.state::<Updates>();
    if state.stopped.load(Ordering::SeqCst) {
        return;
    }
    if let Err(error) = &result {
        crate::internal_logs::report_error("app_update", error);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    state
        .model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .finish(result, Instant::now(), now);
    publish(app);
}
fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("更新確認はメイン画面で行ってください".into())
    }
}
#[tauri::command]
pub(crate) fn get_app_update_status(
    window: WebviewWindow,
) -> Result<AppUpdateStatus, mon_contracts::AppError> {
    require_main(&window)
        .map(|_| window.state::<Updates>().snapshot())
        .map_err(|e| mon_contracts::AppError::from_command("get_app_update_status", e))
}
#[tauri::command]
pub(crate) fn check_app_update(
    window: WebviewWindow,
) -> Result<AppUpdateStatus, mon_contracts::AppError> {
    require_main(&window)
        .map(|_| {
            request(window.app_handle());
            window.state::<Updates>().snapshot()
        })
        .map_err(|e| mon_contracts::AppError::from_command("check_app_update", e))
}
#[tauri::command]
pub(crate) async fn open_app_update_page(
    window: WebviewWindow,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async {
        require_main(&window)?;
        let destination = window
            .state::<Updates>()
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .destination()?;
        #[cfg(target_os = "macos")]
        {
            let _ = destination;
            return Err("更新はアプリ内の「再起動して更新」から適用してください".into());
        }
        #[cfg(windows)]
        {
            return windows::open(&window, &destination).await;
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = destination;
            Err("このOSは更新確認の対象外です".into())
        }
    }
    .await;
    if let Err(error) = &result {
        crate::internal_logs::report_error("app_update", error);
    }
    result.map_err(|_| {
        mon_contracts::AppError::from_command(
            "open_app_update_page",
            "配布元を開けませんでした。もう一度お試しください。".into(),
        )
    })
}

/// Ask the main window to save edits; never install directly from this command.
/// The Store package only requests the normal quit path; Microsoft Store installs the
/// deferred update after the app has exited.
#[tauri::command]
pub(crate) fn apply_app_update(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        require_main(&window)?;
        #[cfg(target_os = "macos")]
        {
            macos::require_ready(window.app_handle())?;
            macos::require_idle(window.app_handle())?;
            window
                .emit(
                    "main-close-requested",
                    serde_json::json!({"quit": true, "update": true}),
                )
                .map_err(|e| e.to_string())
        }
        #[cfg(windows)]
        {
            policy::require_store_quit(window.state::<Updates>().snapshot().channel)?;
            crate::residency::quit(window.app_handle());
            Ok(())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        Err("この配布版はアプリ内更新の対象外です".into())
    })();
    result.map_err(|e| mon_contracts::AppError::from_command("apply_app_update", e))
}
