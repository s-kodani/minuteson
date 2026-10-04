mod analysis_worker;
mod audio;
mod generation;
mod local_engine;
mod minutes_document;
mod playback;
mod recorder;
mod session;
mod storage;
mod transcription;

use analysis_worker::AnalysisController;
use playback::{PlaybackInfo, PlaybackStore};
use recorder::RecorderController;
use session::{RecordingSnapshot, SessionStore};
use std::sync::atomic::{AtomicBool, Ordering};
use storage::{AnalysisView, Meeting, Storage, TranscriptDraft, TranscriptDraftPayload};
use tauri::{Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};

#[derive(Default)]
struct ExitGuard(AtomicBool);

#[tauri::command]
fn get_analysis_preferences(window: WebviewWindow) -> Result<storage::PreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis settings".into());
    }
    window.state::<Storage>().analysis_preferences()
}

#[tauri::command]
fn save_analysis_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::AnalysisPreferences,
) -> Result<storage::PreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit analysis settings".into());
    }
    window
        .state::<Storage>()
        .save_analysis_preferences(expected_lock_version, payload)
}

#[tauri::command]
fn get_analysis_versions(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::VersionState, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis versions".into());
    }
    window.state::<Storage>().analysis_versions(&meeting_id)
}

#[tauri::command]
async fn get_analysis_preview(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
) -> Result<Option<AnalysisView>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis candidates".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .load_visible_analysis(&meeting_id, &analysis_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_analysis_version_draft(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
) -> Result<Option<TranscriptDraft>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read version drafts".into());
    }
    window
        .state::<Storage>()
        .visible_analysis_draft(&meeting_id, &analysis_id)
}

#[tauri::command]
async fn adopt_analysis_candidate(
    window: WebviewWindow,
    request: storage::AdoptRequest,
) -> Result<storage::CommittedAnalysis, String> {
    if window.label() != "main" {
        return Err("Only the main window can adopt analysis candidates".into());
    }
    let app = window.app_handle().clone();
    let meeting_id = request.meeting_id.clone();
    let worker_app = app.clone();
    let adopted = tauri::async_runtime::spawn_blocking(move || {
        worker_app.state::<Storage>().adopt_analysis(request)
    })
    .await
    .map_err(|e| e.to_string())??;
    // The notification is an invalidation, never a substitute for a DB commit.
    let _ = app.emit("analysis-version-changed", meeting_id);
    Ok(adopted)
}

#[tauri::command]
async fn restore_analysis_version(
    window: WebviewWindow,
    meeting_id: String,
    expected_meeting_lock_version: i64,
    expected_previous_id: String,
    expected_content_hash: String,
    expected_draft: Option<storage::DraftStamp>,
) -> Result<storage::CommittedAnalysis, String> {
    if window.label() != "main" {
        return Err("Only the main window can restore analysis".into());
    }
    let app = window.app_handle().clone();
    let changed_meeting = meeting_id.clone();
    let worker_app = app.clone();
    let restored = tauri::async_runtime::spawn_blocking(move || {
        worker_app.state::<Storage>().restore_analysis_checked(
            &meeting_id,
            expected_meeting_lock_version,
            &expected_previous_id,
            &expected_content_hash,
            expected_draft,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = app.emit("analysis-version-changed", changed_meeting);
    Ok(restored)
}

#[tauri::command]
async fn start_minutes_generation(
    window: WebviewWindow,
    request: generation::StartRequest,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start minutes generation".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let job = request.enqueue(&app.state::<Storage>())?;
        AnalysisController::kick(&app)?;
        Ok(job)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn start_transcription(
    window: WebviewWindow,
    request: transcription::StartRequest,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start analysis".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let job = request.enqueue(&app.state::<Storage>())?;
        AnalysisController::kick(&app)?;
        Ok(job)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_analysis_jobs(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Vec<storage::JobSnapshot>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis jobs".into());
    }
    window.state::<Storage>().list_jobs(&meeting_id)
}

#[tauri::command]
async fn retry_transcription(
    window: WebviewWindow,
    request: transcription::RetryRequest,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can retry analysis".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let job = request.enqueue(&app.state::<Storage>())?;
        AnalysisController::kick(&app)?;
        Ok(job)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn pause_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can pause analysis".into());
    }
    window.state::<Storage>().request_job_pause(&job_id, "user")
}

#[tauri::command]
fn resume_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can resume analysis".into());
    }
    let job = window.state::<Storage>().resume_job(&job_id)?;
    AnalysisController::kick(window.app_handle())?;
    Ok(job)
}

#[tauri::command]
async fn cancel_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can cancel analysis".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let storage = app.state::<Storage>();
        let job = storage.get_job(&job_id)?;
        if job.state == "running" {
            storage.request_job_pause(&job_id, "user")?;
        }
        let started = std::time::Instant::now();
        loop {
            let job = storage.get_job(&job_id)?;
            if job.state != "pausing" {
                return storage.finish_job(&job_id, job.completed_units, "cancelled", None);
            }
            if started.elapsed() > std::time::Duration::from_secs(15) {
                return Err(
                    "解析の安全な停止を待っています。完了後に取り消しを再試行してください".into(),
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_utterance_playback(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
    utterance_id: String,
) -> Result<PlaybackInfo, String> {
    if window.label() != "main" {
        return Err("Only the main window can play audio".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let audio = app.state::<Storage>().prepare_utterance_audio(
            &meeting_id,
            &analysis_id,
            &utterance_id,
        )?;
        app.state::<PlaybackStore>().insert(audio)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn read_playback_chunk(
    window: WebviewWindow,
    token: String,
    offset_frames: u64,
) -> Result<tauri::ipc::Response, String> {
    if window.label() != "main" {
        return Err("Only the main window can play audio".into());
    }
    let app = window.app_handle().clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        app.state::<PlaybackStore>().read(&token, offset_frames)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
fn close_playback(
    window: WebviewWindow,
    store: State<'_, PlaybackStore>,
    token: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can close playback".into());
    }
    store.close(&token)
}

#[tauri::command]
async fn finish_main_close(window: WebviewWindow, quit: bool) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can finish closing".into());
    }
    if window.state::<SessionStore>().snapshot().is_active() {
        if quit {
            return Err("録音の停止と保存が完了していません".into());
        }
        return window.hide().map_err(|error| error.to_string());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || AnalysisController::shutdown(&app))
        .await
        .map_err(|e| e.to_string())??;
    window.state::<ExitGuard>().0.store(true, Ordering::SeqCst);
    window.app_handle().exit(0);
    Ok(())
}

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
async fn start_recording(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<RecordingSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start recording".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::prepare_recording(&app)?;
        let result = RecorderController::start(&app, &meeting_id);
        if result.is_err() {
            AnalysisController::recording_finished(&app);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
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
        .manage(ExitGuard::default())
        .manage(PlaybackStore::default())
        .manage(AnalysisController::default())
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
            AnalysisController::kick(app.handle())?;
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
            undo_transcript_edit,
            prepare_utterance_playback,
            read_playback_chunk,
            close_playback,
            finish_main_close,
            start_transcription,
            start_minutes_generation,
            get_analysis_jobs,
            retry_transcription,
            pause_analysis_job,
            resume_analysis_job,
            cancel_analysis_job,
            get_analysis_preferences,
            save_analysis_preferences,
            get_analysis_versions,
            get_analysis_preview,
            get_analysis_version_draft,
            adopt_analysis_candidate,
            restore_analysis_version
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
                    api.prevent_close();
                    if window.state::<SessionStore>().snapshot().is_active() {
                        let _ = window.hide();
                    } else {
                        let _ =
                            window.emit("main-close-requested", serde_json::json!({"quit": false}));
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start mon desktop")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                if !app.state::<ExitGuard>().0.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    if let Some(main) = app.get_webview_window("main") {
                        let _ = main.show();
                        let _ = main.set_focus();
                        let _ =
                            main.emit("main-close-requested", serde_json::json!({"quit": true}));
                    }
                }
            }
        });
}
