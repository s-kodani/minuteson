//! Single-instance activation; restart still re-enters storage admission.
use std::sync::Mutex;
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
// Activation can arrive during plugin initialization, before any windows exist.
// Only the event-loop thread restores windows, once native setup has completed.
#[derive(Default)]
pub(crate) struct Instance(Mutex<Activation>);

#[derive(Default)]
struct Activation {
    ready: bool,
    pending: bool,
}
impl Activation {
    fn request(&mut self) -> bool {
        if self.ready {
            true
        } else {
            self.pending = true;
            false
        }
    }
    fn ready(&mut self) -> bool {
        self.ready = true;
        std::mem::take(&mut self.pending)
    }
}

pub(crate) fn activate(app: &tauri::AppHandle) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if handle
            .state::<Instance>()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .request()
        {
            restore(&handle);
        }
    });
    if let Err(error) = result {
        crate::internal_logs::report_error("startup_activation", &error.to_string());
    }
}
pub(crate) fn ready(app: &tauri::AppHandle) {
    if app
        .state::<Instance>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ready()
    {
        restore(app);
    }
}
fn restore(app: &tauri::AppHandle) {
    if let Err(error) = crate::residency::restore(app) {
        crate::internal_logs::report_error("startup_activation", &error);
    }
}

#[tauri::command]
pub(crate) fn retry_startup(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        crate::startup_reset::check_idle(window.app_handle())?;
        crate::startup_salvage::require_idle(window.app_handle())?;
        window
            .state::<crate::startup_attention::Startup>()
            .admit_command(window.label(), "retry_startup")?;
        prepare_restart()?;
        restart(window.app_handle());
        Ok(())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("retry_startup", message))
}

pub(crate) fn prepare_restart() -> Result<(), String> {
    // On Windows the relay waits for this process to exit before starting its successor.
    #[cfg(windows)]
    windows::schedule_restart()?;
    Ok(())
}

pub(crate) fn restart(app: &tauri::AppHandle) {
    app.state::<crate::ExitGuard>()
        .0
        .store(true, std::sync::atomic::Ordering::SeqCst);
    #[cfg(windows)]
    app.exit(0);
    // Release the rendezvous before requesting the normal exit lifecycle.
    #[cfg(target_os = "macos")]
    {
        tauri_plugin_single_instance::destroy(app);
        app.request_restart();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    app.request_restart();
}

#[cfg(test)]
mod tests {
    use super::Activation;

    #[test]
    fn early_requests_are_coalesced_and_restored_once_after_setup() {
        let mut activation = Activation::default();
        assert!(!activation.request());
        assert!(!activation.request());
        assert!(activation.ready());
        assert!(!activation.ready());
        assert!(activation.request());
    }
    #[test]
    fn ready_without_a_request_does_not_restore_but_later_requests_do() {
        let mut activation = Activation::default();
        assert!(!activation.ready());
        assert!(activation.request());
        assert!(activation.request());
        assert!(!activation.pending);
    }
}
