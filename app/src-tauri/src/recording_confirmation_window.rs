use crate::recorder::RecorderController;
use crate::recording_confirmation::{
    centered_position, fitted_size, DecisionPhase, StopDecision, StopDecisionStore,
};
use crate::session::{RecordingSnapshot, SessionStore};
use crate::storage::Storage;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

pub const LABEL: &str = "recording-confirmation";
fn emit(app: &AppHandle, value: &StopDecision) {
    let _ = app.emit("recording-confirmation-changed", value);
}
fn restore(app: &AppHandle, value: &StopDecision, cancelled: bool) {
    let handle = app.clone();
    let expected = value.clone();
    let _ = app.run_on_main_thread(move || restore_current(&handle, &expected, cancelled));
}
fn restore_current(app: &AppHandle, value: &StopDecision, cancelled: bool) {
    // A delayed completion must not hide or unlock the next decision.
    let current = app.state::<StopDecisionStore>().snapshot();
    if current.id != value.id || current.phase != value.phase {
        return;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    for label in ["main", "recorder"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.set_enabled(true);
        }
    }
    // Completion never brings the main window to the foreground.
    if cancelled {
        if let Some(window) = value
            .origin
            .as_deref()
            .and_then(|label| app.get_webview_window(label))
        {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
    if let Some(id) = value.id.as_deref() {
        if app.state::<StopDecisionStore>().take_completed_quit(id) {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.emit(
                    "main-close-requested",
                    serde_json::json!({ "quit": true, "afterRecording": id }),
                );
            }
        }
    }
}
pub fn session_changed(app: &AppHandle, recording: &RecordingSnapshot) -> StopDecision {
    let previous = app.state::<StopDecisionStore>().snapshot();
    let value = app.state::<StopDecisionStore>().observe(recording);
    emit(app, &value);
    if previous.visible()
        && matches!(
            value.phase,
            DecisionPhase::Completed | DecisionPhase::Cancelled
        )
    {
        restore(app, &value, false);
    }
    value
}
pub fn focus(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn get_recording_confirmation(window: WebviewWindow) -> Result<StopDecision, String> {
    if !matches!(window.label(), "main" | "recorder" | LABEL) {
        return Err("録音画面から状態を確認してください".into());
    }
    Ok(session_changed(
        window.app_handle(),
        &window.state::<SessionStore>().snapshot(),
    ))
}
#[tauri::command]
pub fn request_recording_confirmation(
    window: WebviewWindow,
    quit: Option<bool>,
) -> Result<StopDecision, String> {
    if !matches!(window.label(), "main" | "recorder") {
        return Err("録音画面から確認してください".into());
    }
    let app = window.app_handle();
    let recording = app.state::<SessionStore>().snapshot();
    session_changed(app, &recording);
    let title = recording
        .meeting_id
        .as_deref()
        .map(|id| app.state::<Storage>().get_meeting(id))
        .transpose()?
        .flatten()
        .map(|meeting| meeting.title)
        .unwrap_or_else(|| "現在の録音".into());
    let store = app.state::<StopDecisionStore>();
    let (value, created) = if quit.unwrap_or(false) {
        store.request_for_exit(&recording, window.label(), title, true)?
    } else {
        store.request(&recording, window.label(), title)?
    };
    let present = || -> Result<(), String> {
        let modal = app
            .get_webview_window(LABEL)
            .ok_or("録音確認ウィンドウがありません")?;
        if created {
            let monitor = window
                .current_monitor()
                .map_err(|e| e.to_string())?
                .ok_or("録音画面のモニターを確認できません")?;
            let area = monitor.work_area();
            let (width, height) =
                fitted_size((area.size.width, area.size.height), monitor.scale_factor())?;
            let (x, y) = centered_position(
                (area.position.x, area.position.y),
                (area.size.width, area.size.height),
                (width, height),
            );
            modal
                .set_position(PhysicalPosition::new(x, y))
                .map_err(|e| e.to_string())?;
            modal
                .set_size(PhysicalSize::new(width, height))
                .map_err(|e| e.to_string())?;
        }
        modal.set_always_on_top(true).map_err(|e| e.to_string())?;
        for label in ["main", "recorder"] {
            if let Some(background) = app.get_webview_window(label) {
                background.set_enabled(false).map_err(|e| e.to_string())?;
            }
        }
        modal.show().map_err(|e| e.to_string())?;
        modal.set_focus().map_err(|e| e.to_string())
    };
    if let Err(error) = present() {
        if created {
            if let Some(id) = value.id.as_deref() {
                let _ = app.state::<StopDecisionStore>().dismiss(id);
            }
        }
        let current = app.state::<StopDecisionStore>().snapshot();
        emit(app, &current);
        if created {
            restore(app, &current, false);
        }
        return Err(format!(
            "録音確認を表示できません。新しい停止は送っていません。確認ウィンドウの状態を再確認してください: {error}"
        ));
    }
    let current = session_changed(app, &app.state::<SessionStore>().snapshot());
    if !current.visible() {
        restore(app, &current, false);
    }
    emit(app, &current);
    Ok(current)
}
#[tauri::command]
pub fn decide_recording_confirmation(
    window: WebviewWindow,
    id: String,
    stop: bool,
    expected_preparing: bool,
) -> Result<StopDecision, String> {
    if window.label() != LABEL {
        return Err("録音確認ウィンドウから決定してください".into());
    }
    let app = window.app_handle();
    let store = app.state::<StopDecisionStore>();
    if !stop {
        let value = store.dismiss(&id)?;
        emit(app, &value);
        restore(app, &value, true);
        return Ok(value);
    }
    let session = store.claim(
        &id,
        expected_preparing,
        &app.state::<SessionStore>().snapshot(),
    )?;
    emit(app, &store.snapshot());
    if let Err(error) = RecorderController::stop_session(app, &session) {
        let value = store.fail(&id, error);
        emit(app, &value);
        return Ok(value);
    }
    Ok(session_changed(
        app,
        &app.state::<SessionStore>().snapshot(),
    ))
}
pub fn close_requested(app: &AppHandle) {
    let value = app.state::<StopDecisionStore>().snapshot();
    if let Some(id) = value.id.as_deref() {
        if let Ok(cancelled) = app.state::<StopDecisionStore>().dismiss(id) {
            emit(app, &cancelled);
            restore(app, &cancelled, true);
        }
    }
}
