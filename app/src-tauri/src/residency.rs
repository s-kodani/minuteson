//! Native ownership of the user's entry points, separate from application exit.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, WebviewWindow};
#[cfg(target_os = "macos")]
mod macos;
pub(crate) mod policy;
#[cfg(any(windows, target_os = "macos"))]
mod tray;
#[cfg(windows)]
pub(crate) mod windows;

#[derive(Default)]
pub(crate) struct Residency {
    guid: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
    drain: AtomicBool,
    stopping: AtomicBool,
    menu_installed: AtomicBool,
    cached: Mutex<Probe>,
    refreshed: Mutex<Option<std::time::Instant>>,
    status: Mutex<String>,
    refresh_pending: AtomicBool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Probe {
    pub main: u64,
    pub mini: u64,
    pub tray: Option<String>,
    pub dock: bool,
    pub tray_present: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    supported: bool,
    can_background: bool,
    tray_available: bool,
    dock_available: bool,
    error: Option<String>,
}

pub(crate) fn initialize(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        macos::observe_session();
    }
    #[cfg(any(windows, target_os = "macos"))]
    if let Err(error) = tray::create(app) {
        set_error(app, error);
    }
    // Publish native identities before storage admission can take time. The
    // event loop need not have started for the external guard to see the icon.
    *app.state::<Residency>()
        .cached
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = probe(app);
    *app.state::<Residency>()
        .refreshed
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
    #[cfg(any(windows, target_os = "macos"))]
    {
        let handle = app.clone();
        std::thread::spawn(move || loop {
            if handle.state::<Residency>().stopping.load(Ordering::SeqCst) {
                return;
            }
            let status = tray::status(&handle);
            *handle
                .state::<Residency>()
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = status;
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut backstop = policy::Watch::default();
        let mut monitor_error = false;
        loop {
            if handle.state::<Residency>().stopping.load(Ordering::SeqCst) {
                return;
            }
            let app = handle.clone();
            // Never wait here on a stalled AppKit/WebView event loop. The
            // external watcher continues using the registered native identities.
            if !handle
                .state::<Residency>()
                .refresh_pending
                .swap(true, Ordering::SeqCst)
            {
                let queued = handle.run_on_main_thread(move || {
                    #[cfg(any(windows, target_os = "macos"))]
                    {
                        tray::refresh(&app);
                    }
                    *app.state::<Residency>()
                        .cached
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = probe(&app);
                    *app.state::<Residency>()
                        .refreshed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                    app.state::<Residency>()
                        .refresh_pending
                        .store(false, Ordering::SeqCst);
                });
                if queued.is_err() {
                    handle
                        .state::<Residency>()
                        .refresh_pending
                        .store(false, Ordering::SeqCst);
                }
            }
            {
                let app = &handle;
                let mut probe = app
                    .state::<Residency>()
                    .cached
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if !app.state::<Residency>().refreshed.lock().is_ok_and(|time| {
                    time.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
                }) {
                    probe.tray_present = false;
                    probe.dock = false;
                }
                #[cfg(windows)]
                let result = crate::startup_instance::residency_pulse(&probe);
                #[cfg(target_os = "macos")]
                let result = macos::pulse(&probe);
                #[cfg(not(any(windows, target_os = "macos")))]
                let result: Result<bool, String> = Ok(false);
                match result {
                    Ok(true) => {
                        monitor_error = false;
                        backstop.observe(
                            started.elapsed().as_millis() as u64,
                            policy::Observation::Present,
                            false,
                        );
                        request_drain(app);
                    }
                    Err(error) => {
                        if started.elapsed() >= std::time::Duration::from_secs(3) {
                            set_error(app, error);
                        }
                        if !monitor_error {
                            let handle = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                let _ = restore(&handle);
                            });
                            monitor_error = true;
                        }
                        // A channel error is NOT evidence of missing UI. Only
                        // fresh OS observations can advance this independent
                        // 5s restore / 20s drain / 5s termination backstop.
                        #[cfg(windows)]
                        let (observed, paused) = (
                            windows::observe(std::process::id(), &probe),
                            windows::paused(),
                        );
                        #[cfg(target_os = "macos")]
                        let (observed, paused) =
                            (macos::observation(std::process::id()), macos::paused());
                        #[cfg(not(any(windows, target_os = "macos")))]
                        let (observed, paused) = (policy::Observation::Present, false);
                        let action = backstop.observe(
                            started.elapsed().as_millis() as u64,
                            observed,
                            paused,
                        );
                        match action {
                            policy::Action::Restore => {
                                let handle = app.clone();
                                let _ = app.run_on_main_thread(move || {
                                    let _ = restore(&handle);
                                });
                            }
                            policy::Action::Drain => request_drain(app),
                            policy::Action::Terminate => {
                                // Watch::observe requires >=25 uninterrupted
                                // seconds and >=20 confirmed absent samples.
                                // Unknown results, restored entries, suspend
                                // and screen lock reset this authorization.
                                std::process::exit(1);
                            }
                            _ => {}
                        }
                    }
                    Ok(false) => {
                        monitor_error = false;
                        backstop.observe(
                            started.elapsed().as_millis() as u64,
                            policy::Observation::Present,
                            false,
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}
fn set_error(app: &tauri::AppHandle, error: String) {
    *app.state::<Residency>()
        .error
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(error.clone());
    let _ = app.emit_to("main", "residency-error", error);
}
fn monitor_available() -> bool {
    #[cfg(windows)]
    {
        return crate::startup_instance::residency_available();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::available();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    false
}
fn report(app: &tauri::AppHandle) -> Report {
    #[cfg(any(windows, target_os = "macos"))]
    let tray_available = tray::present();
    #[cfg(not(any(windows, target_os = "macos")))]
    let tray_available = false;
    #[cfg(target_os = "macos")]
    let dock_available = macos::dock_present(std::process::id());
    #[cfg(not(target_os = "macos"))]
    let dock_available = false;
    let state = app.state::<Residency>();
    let ready = monitor_available();
    Report {
        supported: cfg!(any(windows, target_os = "macos")),
        can_background: ready && (tray_available || dock_available),
        tray_available,
        dock_available,
        error: if !ready {
            Some("常駐の監視処理を確認できないため、画面を残します。".into())
        } else if !tray_available && !dock_available {
            Some("常駐アイコンを確認できないため、画面を残します。".into())
        } else {
            state
                .error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        },
    }
}
#[tauri::command]
pub(crate) fn get_residency_state(window: WebviewWindow) -> Result<Report, String> {
    if window.label() != "main" {
        return Err("常駐状態はメイン画面だけで確認できます".into());
    }
    Ok(report(window.app_handle()))
}
#[tauri::command]
pub(crate) fn hide_main_to_background(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("メイン画面だけを収納できます".into());
    }
    let current = report(window.app_handle());
    if !current.can_background {
        let error = current.error.unwrap_or_else(|| {
            "MONの画面を開き直せることを確認できません。画面を表示したままにします。".into()
        });
        set_error(window.app_handle(), error.clone());
        return Err(error);
    }
    window.hide().map_err(|e| e.to_string())?;
    let _ = crate::recorder_panel::synchronize(window.app_handle());
    Ok(())
}
pub(crate) fn restore(app: &tauri::AppHandle) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or("メイン画面がありません")?;
    crate::recorder_panel::with_main_restore(app, || {
        #[cfg(target_os = "macos")]
        {
            let _ = app.show();
        }
        main.show()
            .and_then(|_| main.unminimize())
            .and_then(|_| main.set_focus())
            .map_err(|e| e.to_string())
    })
}
pub(crate) fn quit(app: &tauri::AppHandle) {
    if app.state::<crate::startup_attention::Startup>().blocked() {
        app.exit(0);
        return;
    }
    let _ = restore(app);
    if app
        .state::<crate::recording_confirmation::StopDecisionStore>()
        .snapshot()
        .visible()
    {
        crate::recording_confirmation_window::focus(app);
        return;
    }
    let _ = app.emit_to(
        "main",
        "main-close-requested",
        serde_json::json!({"quit":true}),
    );
}
fn probe(app: &tauri::AppHandle) -> Probe {
    let mut probe = Probe {
        tray: app
            .state::<Residency>()
            .guid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
        ..Probe::default()
    };
    #[cfg(any(windows, target_os = "macos"))]
    {
        probe.tray_present = tray::present();
    }
    #[cfg(windows)]
    {
        probe.main = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map_or(0, |h| h.0 as usize as u64);
        probe.mini = app
            .get_webview_window("recorder")
            .and_then(|w| w.hwnd().ok())
            .map_or(0, |h| h.0 as usize as u64);
    }
    #[cfg(target_os = "macos")]
    {
        probe.dock = macos::dock_present(std::process::id());
    }
    probe
}
pub(crate) fn request_drain(app: &tauri::AppHandle) {
    if !confirmed_absence(app) {
        return;
    }
    if app.state::<Residency>().drain.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        struct Drain {
            app: tauri::AppHandle,
            committed: bool,
        }
        impl Drop for Drain {
            fn drop(&mut self) {
                if !self.committed {
                    self.app
                        .state::<crate::analysis_worker::AnalysisController>()
                        .abort_shutdown();
                    crate::import_controller::ImportController::abort_shutdown(&self.app);
                    crate::data_controller::DataController::abort_shutdown(&self.app);
                    self.app
                        .state::<Residency>()
                        .drain
                        .store(false, Ordering::SeqCst);
                }
            }
        }
        let mut drain = Drain {
            app: handle.clone(),
            committed: false,
        };
        crate::internal_logs::phase("residency", None, None, "draining", None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let snapshot = handle.state::<crate::session::SessionStore>().snapshot();
        if matches!(
            snapshot.phase,
            crate::session::RecordingPhase::Preparing | crate::session::RecordingPhase::Recording
        ) {
            if let Some(id) = snapshot.session_id {
                let _ = crate::recorder::RecorderController::stop_session(&handle, &id);
            }
        }
        while handle
            .state::<crate::session::SessionStore>()
            .snapshot()
            .is_active()
        {
            if std::time::Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if handle.try_state::<crate::storage::Storage>().is_some() {
            if crate::data_controller::DataController::shutdown(&handle).is_err() {
                return;
            }
            if crate::import_controller::ImportController::shutdown(&handle).is_err() {
                return;
            }
            if crate::analysis_worker::AnalysisController::shutdown(&handle).is_err() {
                return;
            }
        }
        if !confirmed_absence(&handle) {
            // Entry restoration cancels automatic exit, even when recording
            // has already been safely stopped. Never discard new edits made
            // after the user regained a window.
            set_error(&handle,"画面を開き直せるようになったか、アプリの状態が変わったため、自動終了を中止しました。停止した処理の状態を確認してください。".into());
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let _ = restore(&app);
            });
            return;
        }
        handle
            .state::<crate::ExitGuard>()
            .0
            .store(true, Ordering::SeqCst);
        drain.committed = true;
        handle.exit(0);
    });
}
fn confirmed_absence(app: &tauri::AppHandle) -> bool {
    #[cfg(windows)]
    {
        let probe = app
            .state::<Residency>()
            .cached
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        return windows::observe(std::process::id(), &probe) == policy::Observation::Absent
            && !windows::paused();
    }
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<Residency>();
        if state
            .refreshed
            .lock()
            .is_ok_and(|t| t.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)))
            && state.cached.lock().is_ok_and(|p| p.tray_present)
        {
            return false;
        }
        return macos::observation(std::process::id()) == policy::Observation::Absent
            && !macos::paused();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    false
}
pub(crate) fn stopping(app: &tauri::AppHandle) {
    app.state::<Residency>()
        .stopping
        .store(true, Ordering::SeqCst);
    #[cfg(any(windows, target_os = "macos"))]
    tray::remove();
}
#[cfg(target_os = "macos")]
pub(crate) fn supervisor_entry() -> Option<i32> {
    macos::entry()
}
#[cfg(target_os = "macos")]
pub(crate) fn prepare_supervisor() {
    macos::prepare();
}
