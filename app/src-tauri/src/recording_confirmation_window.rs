use crate::recorder::RecorderController;
use crate::recording_confirmation::{
    css_anchor_to_physical, fitted_size, popup_position, StopDecision, StopDecisionStore,
};
use crate::session::{RecordingSnapshot, SessionStore};
use crate::storage::Storage;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

pub const LABEL: &str = "recording-confirmation";
#[derive(serde::Deserialize)]
pub struct PopupAnchor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn physical_anchor(
    window: &WebviewWindow,
    anchor: Option<PopupAnchor>,
    scale: f64,
) -> Option<(i32, i32, u32, u32)> {
    let anchor = anchor?;
    let size = window.inner_size().ok()?;
    let position = window.inner_position().ok()?;
    css_anchor_to_physical(
        (position.x, position.y),
        (size.width, size.height),
        (anchor.x, anchor.y, anchor.width, anchor.height),
        scale,
    )
}
fn emit(app: &AppHandle, value: &StopDecision) {
    let _ = app.emit("recording-confirmation-changed", value);
}
fn restore(app: &AppHandle, value: &StopDecision, cancelled: bool) {
    let handle = app.clone();
    let expected = value.clone();
    let _ = app.run_on_main_thread(move || restore_current(&handle, &expected, cancelled));
}
fn restore_current(app: &AppHandle, value: &StopDecision, cancelled: bool) {
    // A delayed completion must not hide the next decision.
    let current = app.state::<StopDecisionStore>().snapshot();
    if current.id != value.id || current.phase != value.phase {
        return;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    // Completion never brings the main window to the foreground.
    if cancelled {
        if let Some(window) = value
            .origin
            .as_deref()
            .and_then(|label| app.get_webview_window(label))
        {
            // Restore the initiating window's focus only while recording is
            // still active and the panel was not explicitly dismissed.
            if window.label() != "recorder" || crate::recorder_panel::may_present(app) {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
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
    let value = app.state::<StopDecisionStore>().observe(recording);
    emit(app, &value);
    if !value.visible() {
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
pub fn get_recording_confirmation(
    window: WebviewWindow,
) -> Result<StopDecision, mon_contracts::AppError> {
    let result: Result<StopDecision, String> = (|| {
        if !matches!(window.label(), "main" | "recorder" | LABEL) {
            return Err("録音画面から状態を確認してください".into());
        }
        Ok(session_changed(
            window.app_handle(),
            &window.state::<SessionStore>().snapshot(),
        ))
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_confirmation", message)
    })
}
#[tauri::command]
pub fn request_recording_confirmation(
    window: WebviewWindow,
    quit: Option<bool>,
    anchor: Option<PopupAnchor>,
) -> Result<StopDecision, mon_contracts::AppError> {
    let result: Result<StopDecision, String> = (|| {
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
        if !value.visible() {
            emit(app, &value);
            return Ok(value);
        }
        let present = || -> Result<(), String> {
            // Read-only help cannot cover a confirmation or steal its focus back.
            crate::recorder_info::hide(app, false)?;
            let popup = app
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
                let source = window.outer_position().map_err(|e| e.to_string())?;
                let source_size = window.outer_size().map_err(|e| e.to_string())?;
                // CSS coordinates belong to the source WebView. Its active DPI can
                // differ from the selected monitor during a display transition.
                let source_scale = window.scale_factor().unwrap_or(monitor.scale_factor());
                let anchor = physical_anchor(&window, anchor, source_scale);
                let (x, y) = popup_position(
                    (area.position.x, area.position.y),
                    (area.size.width, area.size.height),
                    (source.x, source.y),
                    (source_size.width, source_size.height),
                    anchor,
                    (width, height),
                    window.label() == "recorder",
                    monitor.scale_factor(),
                );
                popup
                    .set_position(PhysicalPosition::new(x, y))
                    .map_err(|e| e.to_string())?;
                popup
                    .set_size(PhysicalSize::new(width, height))
                    .map_err(|e| e.to_string())?;
            }
            popup.set_always_on_top(true).map_err(|e| e.to_string())?;
            popup.show().map_err(|e| e.to_string())?;
            popup.set_focus().map_err(|e| e.to_string())?;
            // A panel failure must not block the stop confirmation itself.
            let _ = crate::recorder_panel::synchronize(app);
            Ok(())
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
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("request_recording_confirmation", message)
    })
}
#[tauri::command]
pub async fn decide_recording_confirmation(
    window: WebviewWindow,
    id: String,
    stop: bool,
    expected_preparing: bool,
) -> Result<StopDecision, mon_contracts::AppError> {
    let result: Result<StopDecision, String> = async move {
        if window.label() != LABEL {
            return Err("録音確認ウィンドウから決定してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            decide_recording_confirmation_blocking(&app, &id, stop, expected_preparing)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("decide_recording_confirmation", message)
    })
}
fn decide_recording_confirmation_blocking(
    app: &AppHandle,
    id: &str,
    stop: bool,
    expected_preparing: bool,
) -> Result<StopDecision, String> {
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
    let accepted = store.snapshot();
    emit(app, &accepted);
    restore(app, &accepted, false);
    if let Err(error) = RecorderController::stop_session(app, &session) {
        let value = store.fail(&id, error);
        emit(app, &value);
        restore(app, &value, false);
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

pub fn hide_recorder(app: &AppHandle) -> Result<(), String> {
    // Visibility only: no recorder lock, state read, stop, save or decision.
    // Even failed help dismissal must not trap the panel on screen.
    crate::recorder_panel::dismiss(app);
    let panel = app
        .get_webview_window("recorder")
        .ok_or("録音パネルがありません")?;
    crate::recorder_panel::hide(&panel)?;
    let _ = crate::recorder_info::hide(app, false);
    if let Some(main) = app.get_webview_window("main") {
        if !main.is_visible().map_err(|e| e.to_string())? {
            main.show().map_err(|e| e.to_string())?;
            main.set_focus().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
