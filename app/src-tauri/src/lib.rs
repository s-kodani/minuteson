mod recorder;
mod session;
mod storage;

use recorder::RecorderController;
use session::{RecordingSnapshot, SessionStore};
use storage::{AnalysisView, Meeting, Storage, TranscriptDraft, TranscriptDraftPayload};
use tauri::{Emitter, Manager, State, WebviewWindow, WindowEvent};

#[tauri::command]
fn get_recording_snapshot(store: State<'_, SessionStore>) -> RecordingSnapshot {
    store.snapshot()
}

#[tauri::command]
fn capture_available(window: WebviewWindow) -> Result<bool, String> {
    if window.label() != "main" {
        return Err("Only the main window can inspect capture availability".into());
    }
    Ok(RecorderController::available(window.app_handle()))
}

#[tauri::command]
fn start_recording(window: WebviewWindow, meeting_id: String) -> Result<RecordingSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start recording".into());
    }
    RecorderController::start(window.app_handle(), &meeting_id)
}

#[tauri::command]
fn stop_recording(window: WebviewWindow) -> Result<RecordingSnapshot, String> {
    if !matches!(window.label(), "main" | "recorder") {
        return Err("Only the recording windows can stop recording".into());
    }
    RecorderController::stop(window.app_handle())
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

#[tauri::command]
fn get_current_analysis(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
) -> Result<Option<AnalysisView>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis".into());
    }
    storage.load_current_analysis(&meeting_id)
}

#[tauri::command]
fn get_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
) -> Result<Option<TranscriptDraft>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read drafts".into());
    }
    storage.load_transcript_draft(&meeting_id)
}

#[tauri::command]
fn begin_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
    analysis_id: String,
) -> Result<TranscriptDraft, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit transcripts".into());
    }
    storage.create_transcript_draft(&meeting_id, &analysis_id)
}

#[tauri::command]
fn save_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    draft_id: String,
    expected_lock_version: i64,
    payload: TranscriptDraftPayload,
) -> Result<TranscriptDraft, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit transcripts".into());
    }
    storage.save_transcript_draft(&draft_id, expected_lock_version, payload)
}

#[tauri::command]
fn undo_transcript_edit(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    draft_id: String,
    expected_lock_version: i64,
) -> Result<TranscriptDraft, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit transcripts".into());
    }
    storage.undo_last_transcript_edit(&draft_id, expected_lock_version)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(RecorderController::default())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            app.manage(Storage::open(root)?);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // Recovery can hash hours of audio, so never hold up window startup.
                match handle.state::<Storage>().recover_completed_recordings() {
                    Ok(ids) if !ids.is_empty() => {
                        let _ = handle.emit("recordings-recovered", ids);
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("録音回復の確認に失敗しました: {error}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_recording_snapshot,
            capture_available,
            start_recording,
            stop_recording,
            show_recorder_preview,
            hide_recorder_preview,
            open_main_window,
            create_meeting,
            list_meetings,
            get_meeting,
            rename_meeting,
            get_current_analysis,
            get_transcript_draft,
            begin_transcript_draft,
            save_transcript_draft,
            undo_transcript_edit
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
