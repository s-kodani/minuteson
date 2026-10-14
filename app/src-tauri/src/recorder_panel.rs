use crate::recording_confirmation::StopDecisionStore;
use crate::session::{RecordingPhase, RecordingSnapshot, SessionStore};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

/// Closing the mini panel is a per-focus-cycle choice, not a request to stop
/// capture. Returning to the main window resets it for the next deactivation.
#[derive(Default)]
pub(crate) struct PanelVisibility {
    dismissed: AtomicBool,
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

fn main_focused(app: &AppHandle) -> bool {
    // A failed focus read must never make an always-on-top window appear.
    app.get_webview_window("main")
        .and_then(|main| main.is_focused().ok())
        .unwrap_or(true)
}

pub(crate) fn may_present(app: &AppHandle) -> bool {
    wants_panel(
        &app.state::<SessionStore>().snapshot(),
        main_focused(app),
        app.state::<PanelVisibility>()
            .dismissed
            .load(Ordering::SeqCst),
        app.state::<StopDecisionStore>().snapshot().visible(),
    )
}

pub(crate) fn dismiss(app: &AppHandle) {
    app.state::<PanelVisibility>()
        .dismissed
        .store(true, Ordering::SeqCst);
}

pub(crate) fn main_focus_changed(app: &AppHandle, focused: bool) {
    if focused {
        app.state::<PanelVisibility>()
            .dismissed
            .store(false, Ordering::SeqCst);
    }
    let _ = synchronize(app, Some(focused));
}

/// Called on the native UI thread after a phase publication or main focus
/// change. The recorder lock is never held through a window operation.
pub(crate) fn synchronize(app: &AppHandle, focused: Option<bool>) -> Result<(), String> {
    let snapshot = app.state::<SessionStore>().snapshot();
    let active = matches!(snapshot.phase, RecordingPhase::Recording);
    if !active {
        app.state::<PanelVisibility>()
            .dismissed
            .store(false, Ordering::SeqCst);
    }
    let focused = focused.unwrap_or_else(|| main_focused(app));
    let dismissed = app
        .state::<PanelVisibility>()
        .dismissed
        .load(Ordering::SeqCst);
    let panel = app
        .get_webview_window("recorder")
        .ok_or("録音パネルがありません")?;
    let confirming = app.state::<StopDecisionStore>().snapshot().visible();
    if !wants_panel(&snapshot, focused, dismissed, confirming) {
        // If visibility cannot be read, still attempt to close: a stopped
        // recording must not leave an always-on-top panel behind.
        if panel.is_visible().unwrap_or(true) {
            panel.hide().map_err(|error| error.to_string())?;
            let _ = crate::recorder_info::hide(app, false);
        }
    } else if !panel.is_visible().map_err(|error| error.to_string())? {
        panel.show().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unfocused_active_recording_can_show_the_panel() {
        let mut snapshot = RecordingSnapshot::default();
        snapshot.phase = RecordingPhase::Recording;
        assert!(wants_panel(&snapshot, false, false, false));
        assert!(!wants_panel(&snapshot, true, false, false));
        assert!(!wants_panel(&snapshot, false, true, false));
        assert!(!wants_panel(&snapshot, false, false, true));
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
