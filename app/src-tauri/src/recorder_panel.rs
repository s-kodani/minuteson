use crate::recording_confirmation::StopDecisionStore;
use crate::session::{RecordingPhase, RecordingSnapshot, SessionStore};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tauri::{AppHandle, Manager, WebviewWindow};

#[cfg(all(test, windows))]
mod native_tests;
#[cfg(windows)]
mod windows;

/// Closing the mini panel is a per-focus-cycle choice, not a request to stop
/// capture. Returning to the main window resets it for the next deactivation.
#[derive(Default)]
pub(crate) struct PanelVisibility {
    dismissed: AtomicBool,
    restoring: AtomicUsize,
    queued: AtomicBool,
    synchronizing: AtomicBool,
}

struct RestoreGuard<'a>(&'a AtomicUsize);
impl Drop for RestoreGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct SynchronizeGuard<'a>(&'a AtomicBool);
impl Drop for SynchronizeGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl PanelVisibility {
    fn begin_synchronize(&self) -> Option<SynchronizeGuard<'_>> {
        if self.synchronizing.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(SynchronizeGuard(&self.synchronizing))
        }
    }

    fn begin_restore(&self) -> RestoreGuard<'_> {
        self.restoring.fetch_add(1, Ordering::SeqCst);
        RestoreGuard(&self.restoring)
    }

    fn wants_panel(&self, snapshot: &RecordingSnapshot, focused: bool, confirming: bool) -> bool {
        self.restoring.load(Ordering::SeqCst) == 0
            && wants_panel(
                snapshot,
                focused,
                self.dismissed.load(Ordering::SeqCst),
                confirming,
            )
    }

    fn focus_changed(&self, focused: bool) {
        if focused {
            self.dismissed.store(false, Ordering::SeqCst);
        }
    }
}

fn wants_panel(
    snapshot: &RecordingSnapshot,
    main_focused: bool,
    dismissed: bool,
    confirming: bool,
) -> bool {
    matches!(snapshot.phase, RecordingPhase::Recording)
        && !main_focused
        && !dismissed
        && !confirming
}

fn confirmation_hides_panel(visible: bool, origin: Option<&str>) -> bool {
    visible && origin != Some("recorder")
}

fn main_focused(app: &AppHandle) -> bool {
    // A failed focus read must never make an always-on-top window appear.
    app.get_webview_window("main")
        .and_then(|main| {
            Some(main.is_visible().ok()? && !main.is_minimized().ok()? && main.is_focused().ok()?)
        })
        .unwrap_or(true)
}

pub(crate) fn may_present(app: &AppHandle) -> bool {
    app.state::<PanelVisibility>().wants_panel(
        &app.state::<SessionStore>().snapshot(),
        main_focused(app),
        app.state::<StopDecisionStore>().snapshot().visible(),
    )
}

pub(crate) fn dismiss(app: &AppHandle) {
    app.state::<PanelVisibility>()
        .dismissed
        .store(true, Ordering::SeqCst);
}

pub(crate) fn main_focus_changed(app: &AppHandle, focused: bool) {
    app.state::<PanelVisibility>().focus_changed(focused);
    request_synchronize(app);
}

fn report_error(error: &str) {
    crate::internal_logs::phase("recorder_panel", None, None, "failed", Some(error));
}

fn request_synchronize(app: &AppHandle) {
    if app
        .state::<PanelVisibility>()
        .queued
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    let handle = app.clone();
    // run_on_main_thread executes inline when called on the UI thread. Cross
    // the runtime first so focus callbacks unwind before any show/hide call.
    tauri::async_runtime::spawn(async move {
        let ui = handle.clone();
        if let Err(error) = handle.run_on_main_thread(move || {
            ui.state::<PanelVisibility>()
                .queued
                .store(false, Ordering::SeqCst);
            if let Err(error) = synchronize(&ui) {
                report_error(&error);
            }
        }) {
            handle
                .state::<PanelVisibility>()
                .queued
                .store(false, Ordering::SeqCst);
            report_error(&error.to_string());
        }
    });
}

/// Suppress reentrant focus events throughout show/unminimize/set_focus. The
/// counter also covers nested restores and is released on every error path.
pub(crate) fn with_main_restore(
    app: &AppHandle,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let state = app.state::<PanelVisibility>();
    let restoring = state.begin_restore();
    if let Err(error) = synchronize(app) {
        report_error(&error);
    }
    let result = restore();
    drop(restoring);
    request_synchronize(app);
    result
}

pub(crate) fn hide(panel: &WebviewWindow) -> Result<(), String> {
    // Explicit confirmation cancellation may have used Tauri's activating
    // show(). Clear its cached visibility as well as the actual HWND state.
    let result = panel.hide().map_err(|error| error.to_string());
    #[cfg(windows)]
    windows::set_visible(panel.hwnd().map_err(|error| error.to_string())?.0, false)?;
    result
}

fn show(panel: &WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::set_visible(panel.hwnd().map_err(|error| error.to_string())?.0, true)
    }
    #[cfg(not(windows))]
    panel.show().map_err(|error| error.to_string())
}

/// Called on the native UI thread. Always read the latest phase and focus,
/// never the values captured by an earlier window event. No recorder lock is
/// held through a window operation.
pub(crate) fn synchronize(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<PanelVisibility>();
    let Some(_synchronizing) = state.begin_synchronize() else {
        request_synchronize(app);
        return Ok(());
    };
    let snapshot = app.state::<SessionStore>().snapshot();
    let active = matches!(snapshot.phase, RecordingPhase::Recording);
    if !active {
        app.state::<PanelVisibility>()
            .dismissed
            .store(false, Ordering::SeqCst);
    }
    let focused = main_focused(app);
    let panel = app
        .get_webview_window("recorder")
        .ok_or("録音パネルがありません")?;
    let decision = app.state::<StopDecisionStore>().snapshot();
    // Keep the panel behind a stop popup opened from its own stop button.
    // It still closes as soon as the recording enters stopping/saving.
    let confirming_elsewhere =
        confirmation_hides_panel(decision.visible(), decision.origin.as_deref());
    if !state.wants_panel(&snapshot, focused, confirming_elsewhere) {
        // If visibility cannot be read, still attempt to close: a stopped
        // recording must not leave an always-on-top panel behind.
        if panel.is_visible().unwrap_or(true) {
            hide(&panel)?;
            let _ = crate::recorder_info::hide(app, false);
        }
    } else if !panel.is_visible().map_err(|error| error.to_string())? {
        show(&panel)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> RecordingSnapshot {
        RecordingSnapshot {
            phase: RecordingPhase::Recording,
            ..RecordingSnapshot::default()
        }
    }

    #[test]
    fn restore_suppresses_alternating_focus_until_the_entire_restore_finishes() {
        let state = PanelVisibility::default();
        let recording = recording();
        assert!(state.wants_panel(&recording, false, false));
        let outer = state.begin_restore();
        for focused in [false, true, false, true, false] {
            state.focus_changed(focused);
            assert!(!state.wants_panel(&recording, focused, false));
        }
        let inner = state.begin_restore();
        drop(inner);
        assert!(!state.wants_panel(&recording, false, false));
        drop(outer);
        assert!(!state.wants_panel(&recording, true, false));
        // A later, deliberate app switch must still display the panel.
        assert!(state.wants_panel(&recording, false, false));
    }

    #[test]
    fn failed_restore_releases_suppression_and_allows_another_attempt() {
        let state = PanelVisibility::default();
        let attempt = || -> Result<(), &str> {
            let _restoring = state.begin_restore();
            state.focus_changed(true);
            state.focus_changed(false);
            assert!(!state.wants_panel(&recording(), false, false));
            Err("native restore failed")
        };
        for _ in 0..2 {
            assert!(attempt().is_err());
            assert!(state.wants_panel(&recording(), false, false));
        }
    }

    #[test]
    fn delayed_focus_work_respects_stop_and_explicit_dismissal() {
        let state = PanelVisibility::default();
        let mut latest = recording();
        state.focus_changed(false);
        latest.phase = RecordingPhase::Stopping;
        assert!(!state.wants_panel(&latest, false, false));
        state.dismissed.store(true, Ordering::SeqCst);
        assert!(!state.wants_panel(&recording(), false, false));
        state.focus_changed(true);
        assert!(!state.wants_panel(&recording(), true, false));
        state.focus_changed(false);
        assert!(state.wants_panel(&recording(), false, false));
    }

    #[test]
    fn nested_synchronization_is_rejected_without_releasing_the_outer_guard() {
        let state = PanelVisibility::default();
        let outer = state.begin_synchronize().unwrap();
        assert!(state.begin_synchronize().is_none());
        assert!(state.begin_synchronize().is_none());
        drop(outer);
        assert!(state.begin_synchronize().is_some());
    }

    #[test]
    fn only_an_unfocused_active_recording_can_show_the_panel() {
        let mut snapshot = RecordingSnapshot::default();
        snapshot.phase = RecordingPhase::Recording;
        assert!(wants_panel(&snapshot, false, false, false));
        assert!(!wants_panel(&snapshot, true, false, false));
        assert!(!wants_panel(&snapshot, false, true, false));
        assert!(!wants_panel(&snapshot, false, false, true));
        assert!(!confirmation_hides_panel(true, Some("recorder")));
        assert!(confirmation_hides_panel(true, Some("main")));
        assert!(!confirmation_hides_panel(false, Some("main")));
        for phase in [
            RecordingPhase::Preparing,
            RecordingPhase::Stopping,
            RecordingPhase::Saving,
            RecordingPhase::Idle,
            RecordingPhase::Error,
        ] {
            snapshot.phase = phase;
            assert!(!wants_panel(&snapshot, false, false, false));
        }
    }
}
