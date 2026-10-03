mod session;
mod storage;

use session::{RecordingSnapshot, SessionStore};
use storage::{Meeting, Storage};
use tauri::{Manager, State, WebviewWindow, WindowEvent};

#[tauri::command]
fn get_recording_snapshot(store: State<'_, SessionStore>) -> RecordingSnapshot {
    store.snapshot()
}

#[tauri::command]
fn show_recorder_preview(
    window: WebviewWindow,
    store: State<'_, SessionStore>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can open the preview".into());
    }
    if store.snapshot().is_active() {
        return Err("Preview is only available while idle".into());
    }
    let panel = window
        .app_handle()
        .get_webview_window("recorder")
        .ok_or("Recorder panel unavailable")?;
    panel.show().map_err(|error| error.to_string())?;
    panel.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_recorder_preview(
    window: WebviewWindow,
    store: State<'_, SessionStore>,
) -> Result<(), String> {
    if window.label() != "recorder" {
        return Err("Only the recorder panel can close its preview".into());
    }
    if store.snapshot().is_active() {
        return Err("An active recording panel cannot be hidden".into());
    }
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_main_window(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "recorder" {
        return Err("Only the recorder panel can reopen the main window".into());
    }
    let main = window
        .app_handle()
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    main.show().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn create_meeting(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    title: String,
) -> Result<Meeting, String> {
    if window.label() != "main" {
        return Err("Only the main window can create meetings".into());
    }
    storage.create_meeting(&title)
}

#[tauri::command]
fn list_meetings(
    window: WebviewWindow,
    storage: State<'_, Storage>,
) -> Result<Vec<Meeting>, String> {
    if window.label() != "main" {
        return Err("Only the main window can list meetings".into());
    }
    storage.list_meetings()
}

#[tauri::command]
fn get_meeting(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    id: String,
) -> Result<Option<Meeting>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read meetings".into());
    }
    storage.get_meeting(&id)
}

#[tauri::command]
fn rename_meeting(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    id: String,
    title: String,
    expected_lock_version: i64,
) -> Result<Meeting, String> {
    if window.label() != "main" {
        return Err("Only the main window can rename meetings".into());
    }
    storage.rename_meeting(&id, &title, expected_lock_version)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            app.manage(Storage::open(root)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_recording_snapshot,
            show_recorder_preview,
            hide_recorder_preview,
            open_main_window,
            create_meeting,
            list_meetings,
            get_meeting,
            rename_meeting
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "recorder" {
                    if window.state::<SessionStore>().snapshot().is_active() {
                        api.prevent_close();
                    } else {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                } else if window.label() == "main" {
                    if window.state::<SessionStore>().snapshot().is_active() {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        window.app_handle().exit(0);
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start mon desktop");
}
