mod analysis_flow;
mod analysis_worker;
mod application_close;
mod audio;
mod audio_clock;
mod audio_import;
#[cfg(target_os = "macos")]
mod bootstrap;
mod capture_inputs;
mod capture_scope;
mod data_controller;
mod diarization;
mod exports;
mod generation;
mod import_controller;
mod local_engine;
mod minutes_document;
mod model_application;
mod model_compatibility;
mod model_inspection;
mod model_placement;
mod model_rebinding;
mod playback;
mod recorder;
mod recording_confirmation;
mod recording_confirmation_window;
mod resampling;
mod restored_processing;
mod saved_artifact;
mod session;
mod storage;
mod transcription;

use analysis_worker::AnalysisController;
use data_controller::DataController;
use import_controller::ImportController;
use playback::{PlaybackInfo, PlaybackPreparations, PlaybackStore};
use recorder::RecorderController;
use recording_confirmation::StopDecisionStore;
use recording_confirmation_window::{
    decide_recording_confirmation, get_recording_confirmation, request_recording_confirmation,
};
use session::{RecordingSnapshot, SessionStore};
use std::sync::atomic::{AtomicBool, Ordering};
use storage::{AnalysisView, Meeting, Storage, TranscriptDraft, TranscriptDraftPayload};
use tauri::{Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};

#[derive(Default)]
struct ExitGuard(AtomicBool);

#[tauri::command]
async fn inspect_model_package(
    window: WebviewWindow,
    token: String,
    directory: std::path::PathBuf,
) -> Result<model_inspection::Report, String> {
    if window.label() != "main" {
        return Err("Only the main window can inspect models".into());
    }
    let guard = window
        .state::<model_inspection::Inspector>()
        .begin(&token)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_inspection::inspect(&directory, &|| guard.cancelled() || lease.cancelled())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_model_inspection(window: WebviewWindow, token: String) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can cancel model inspection".into());
    }
    window.state::<model_inspection::Inspector>().cancel(&token)
}

#[cfg(unix)]
#[tauri::command]
async fn preview_model_deletion(
    window: WebviewWindow,
    token: String,
    package_id: String,
    expected_review_hash: String,
) -> Result<storage::deletion::models::Preview, String> {
    if window.label() != "main" {
        return Err("Only the main window can inspect model deletion".into());
    }
    let guard = window
        .state::<model_inspection::Inspector>()
        .begin(&token)?;
    let app = window.app_handle().clone();
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            storage::deletion::models::preview(
                &app.state::<Storage>(),
                &root,
                &token,
                &package_id,
                &expected_review_hash,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(unix))]
#[tauri::command]
async fn preview_model_deletion(
    window: WebviewWindow,
    token: String,
    package_id: String,
    expected_review_hash: String,
) -> Result<serde_json::Value, String> {
    let _ = (window, token, package_id, expected_review_hash);
    Err("Windowsのモデル削除対象の検査は保留中です".into())
}

#[tauri::command]
fn get_model_placements(
    window: WebviewWindow,
) -> Result<Vec<model_placement::CatalogEntry>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read model placements".into());
    }
    window.state::<Storage>().list_model_placements()
}

#[tauri::command]
async fn get_model_catalog_history(
    window: WebviewWindow,
) -> Result<Vec<storage::model_profiles::ModelCatalogHistoryEntry>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read portable model history".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().model_catalog_history())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_model_profiles(
    window: WebviewWindow,
) -> Result<Vec<storage::model_profiles::ProfileView>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read model profiles".into());
    }
    window.state::<Storage>().model_profiles()
}
#[tauri::command]
async fn preview_model_profile_application(
    window: WebviewWindow,
    token: String,
    profile: storage::model_profiles::ProfileName,
    package_id: String,
    expected_review_hash: String,
    check_operation_id: String,
) -> Result<storage::model_profiles::ApplyPreview, String> {
    if window.label() != "main" {
        return Err("Only the main window can review model application".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_application::preview(
                &app.state::<Storage>(),
                &models_root,
                profile,
                &package_id,
                &expected_review_hash,
                &check_operation_id,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn apply_model_profile_application(
    window: WebviewWindow,
    token: String,
    preview: storage::model_profiles::ApplyPreview,
) -> Result<storage::model_profiles::ProfileView, String> {
    if window.label() != "main" {
        return Err("Only the main window can apply model profiles".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_application::apply(
                &app.state::<Storage>(),
                &models_root,
                &token,
                &preview,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn preview_job_model_rebinding(
    window: WebviewWindow,
    token: String,
    meeting_id: String,
    job_id: String,
    expected_spec_hash: String,
    package_id: String,
    expected_review_hash: String,
    check_operation_id: String,
) -> Result<model_rebinding::Preview, String> {
    if window.label() != "main" {
        return Err("Only the main window can review job model rebinding".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_rebinding::preview(
                &app.state::<Storage>(),
                &models_root,
                &meeting_id,
                &job_id,
                &expected_spec_hash,
                &package_id,
                &expected_review_hash,
                &check_operation_id,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn apply_job_model_rebinding(
    window: WebviewWindow,
    token: String,
    preview: model_rebinding::Preview,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can apply job model rebinding".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(job) = model_rebinding::result(&app.state::<Storage>(), &token, &preview)? {
            return Ok(job);
        }
        let job =
            AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
                model_rebinding::apply(
                    &app.state::<Storage>(),
                    &models_root,
                    &token,
                    &preview,
                    &|| guard.cancelled() || lease.cancelled(),
                )
            })?;
        AnalysisController::kick(&app)?;
        Ok(job)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_job_model_rebinding_result(
    window: WebviewWindow,
    operation_id: String,
    preview: model_rebinding::Preview,
) -> Result<model_rebinding::Receipt, String> {
    if window.label() != "main" {
        return Err("Only the main window can read job model rebinding results".into());
    }
    let app = window.app_handle().clone();
    // Receipt lookup must not acquire a preparation lease, run inference,
    // enqueue work, or reject a token previously used for cancellation.
    tauri::async_runtime::spawn_blocking(move || {
        model_rebinding::receipt(
            &app.state::<Storage>(),
            &app.state::<model_inspection::Inspector>(),
            &operation_id,
            &preview,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn check_model_compatibility(
    window: WebviewWindow,
    token: String,
    package_id: String,
    expected_review_hash: String,
) -> Result<model_compatibility::CheckSummary, String> {
    if window.label() != "main" {
        return Err("Only the main window can check model compatibility".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_compatibility::check(
                &app.state::<Storage>(),
                &models_root,
                &token,
                &package_id,
                &expected_review_hash,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn place_model_package(
    window: WebviewWindow,
    token: String,
    directory: std::path::PathBuf,
    expected_review_hash: String,
) -> Result<model_placement::PlacementReceipt, String> {
    if window.label() != "main" {
        return Err("Only the main window can place models".into());
    }
    let app = window.app_handle().clone();
    if app.state::<SessionStore>().snapshot().is_active() {
        return Err("録音が停止してからモデルを配置してください".into());
    }
    let guard = app.state::<model_inspection::Inspector>().begin(&token)?;
    // Model bodies stay in OS app-data even when the meeting root moves later.
    let models_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        AnalysisController::with_model_preparation(&app, &|| guard.cancelled(), |lease| {
            model_placement::place(
                &app.state::<Storage>(),
                &models_root,
                &directory,
                &token,
                &expected_review_hash,
                &|| guard.cancelled() || lease.cancelled(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_audio_import(window: WebviewWindow) -> Result<import_controller::Snapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can read imports".into());
    }
    window.state::<ImportController>().snapshot()
}
#[tauri::command]
async fn select_audio_import(
    window: WebviewWindow,
    path: Option<std::path::PathBuf>,
) -> Result<import_controller::Snapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can select audio".into());
    }
    ImportController::select(&window, path).await
}
#[tauri::command]
fn start_audio_import(
    window: WebviewWindow,
    request: import_controller::StartRequest,
) -> Result<import_controller::Snapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can import audio".into());
    }
    ImportController::start(window.app_handle(), request)
}
#[tauri::command]
fn cancel_audio_import(
    window: WebviewWindow,
    token: String,
) -> Result<import_controller::Snapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can cancel imports".into());
    }
    ImportController::cancel_from_app(window.app_handle(), &token)
}

fn data_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    if window.label() != "main" {
        return Err("Only the main window can manage stored data".into());
    }
    Ok(window.app_handle().clone())
}

#[tauri::command]
fn get_data_operation(window: WebviewWindow) -> Result<data_controller::Snapshot, String> {
    data_app(&window)?.state::<DataController>().snapshot()
}

#[cfg(unix)]
#[tauri::command]
async fn get_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::operations::Listing, String> {
    let app = data_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        storage::deletion::operations::listing(&app.state::<Storage>())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(unix)]
#[tauri::command]
fn review_meeting_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::review_deletion(&data_app(&window)?, meeting_id, false)
}
#[cfg(unix)]
#[tauri::command]
fn review_pending_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::review_deletion(&data_app(&window)?, operation_id, true)
}
#[cfg(unix)]
#[tauri::command]
fn get_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::operations::PathPage, String> {
    data_app(&window)?
        .state::<DataController>()
        .deletion_page(&confirmation_id, offset)
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn get_storage_bootstrap(window: WebviewWindow) -> Result<bootstrap::View, String> {
    let app = data_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<bootstrap::Bootstrap>().view())
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(unix)]
#[tauri::command]
async fn get_model_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::models::operations::Listing, String> {
    let app = data_app(&window)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    tauri::async_runtime::spawn_blocking(move || {
        storage::deletion::models::operations::listing(&app.state::<Storage>(), &root)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(unix)]
#[tauri::command]
fn review_model_body_deletion(
    window: WebviewWindow,
    package_id: String,
    expected_review_hash: String,
) -> Result<data_controller::Snapshot, String> {
    let app = data_app(&window)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    DataController::review_model_deletion(
        &app,
        root,
        data_controller::ModelTarget::Fresh {
            package: package_id,
            review: expected_review_hash,
        },
    )
}
#[cfg(unix)]
#[tauri::command]
fn review_pending_model_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, String> {
    let app = data_app(&window)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    DataController::review_model_deletion(
        &app,
        root,
        data_controller::ModelTarget::Pending(operation_id),
    )
}
#[cfg(unix)]
#[tauri::command]
fn get_model_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::models::operations::PathPage, String> {
    data_app(&window)?
        .state::<DataController>()
        .model_deletion_page(&confirmation_id, offset)
}

#[cfg(unix)]
#[tauri::command]
async fn get_audio_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::audio::operations::Listing, String> {
    let app = data_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        storage::deletion::audio::operations::listing(&app.state::<Storage>())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(unix)]
#[tauri::command]
fn review_meeting_audio_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::review_audio_deletion(
        &data_app(&window)?,
        data_controller::AudioTarget::Fresh(meeting_id),
    )
}
#[cfg(unix)]
#[tauri::command]
fn review_pending_audio_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::review_audio_deletion(
        &data_app(&window)?,
        data_controller::AudioTarget::Pending(operation_id),
    )
}
#[cfg(unix)]
#[tauri::command]
fn get_audio_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::audio::operations::PathPage, String> {
    data_app(&window)?
        .state::<DataController>()
        .audio_deletion_page(&confirmation_id, offset)
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn review_storage_move(window: WebviewWindow) -> Result<data_controller::Snapshot, String> {
    DataController::review_move(&data_app(&window)?)
}

#[tauri::command]
async fn get_recovery_items(
    window: WebviewWindow,
) -> Result<storage::backup::recovery::Listing, String> {
    let app = data_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().recovery_listing())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
fn review_recovery_item(
    window: WebviewWindow,
    source: storage::backup::recovery::Source,
    operation_id: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::review_recovery(&data_app(&window)?, source, operation_id)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DataAvailability {
    available: bool,
    reason: Option<String>,
}

#[tauri::command]
fn get_data_operation_availability(window: WebviewWindow) -> Result<DataAvailability, String> {
    let app = data_app(&window)?;
    let importing = app.state::<ImportController>().snapshot()?;
    let reason = if app.state::<SessionStore>().snapshot().is_active() {
        Some("録音・音声保存中".into())
    } else if importing.busy || importing.recording_blocked {
        Some("音声取り込み・録音開始の停止待ち中".into())
    } else {
        app.state::<AnalysisController>().data_operation_blocker()?
    };
    Ok(DataAvailability {
        available: reason.is_none(),
        reason,
    })
}

#[tauri::command]
async fn list_restored_meeting_ids(window: WebviewWindow) -> Result<Vec<String>, String> {
    let app = data_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().restored_meeting_ids())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_data_operation(
    window: WebviewWindow,
    kind: data_controller::Kind,
) -> Result<data_controller::Snapshot, String> {
    data_app(&window)?;
    DataController::select(&window, kind).await
}

#[tauri::command]
fn start_data_operation(
    window: WebviewWindow,
    request: data_controller::Request,
) -> Result<data_controller::Snapshot, String> {
    DataController::start(&data_app(&window)?, request)
}

#[tauri::command]
fn cancel_data_operation(
    window: WebviewWindow,
    token: String,
) -> Result<data_controller::Snapshot, String> {
    DataController::cancel(&data_app(&window)?, &token)
}

#[tauri::command]
fn get_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
) -> Result<storage::VocabularyBook, String> {
    if window.label() != "main" {
        return Err("Only the main window can read vocabulary".into());
    }
    window
        .state::<Storage>()
        .vocabulary_book(meeting_id.as_deref())
}
#[tauri::command]
fn get_meeting_vocabulary(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::VocabularyView, String> {
    if window.label() != "main" {
        return Err("Only the main window can read vocabulary".into());
    }
    window.state::<Storage>().vocabulary_view(&meeting_id)
}
#[tauri::command]
fn save_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
    expected_lock_version: i64,
    payload: Vec<storage::VocabularyEntry>,
) -> Result<storage::VocabularyBook, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit vocabulary".into());
    }
    window.state::<Storage>().save_vocabulary_book(
        meeting_id.as_deref(),
        expected_lock_version,
        payload,
    )
}

#[tauri::command]
fn get_analysis_preferences(window: WebviewWindow) -> Result<storage::PreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis settings".into());
    }
    window.state::<Storage>().analysis_preferences()
}

#[tauri::command]
fn get_ui_preferences(window: WebviewWindow) -> Result<storage::UiPreferencesSnapshot, String> {
    if !matches!(
        window.label(),
        "main" | "recorder" | recording_confirmation_window::LABEL
    ) {
        return Err("Only application windows can read presentation settings".into());
    }
    window.state::<Storage>().ui_preferences()
}

#[tauri::command]
fn save_ui_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::UiPreferences,
) -> Result<storage::UiPreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit presentation settings".into());
    }
    let saved = window
        .state::<Storage>()
        .save_ui_preferences(expected_lock_version, payload)?;
    // A hint only: every window reloads Storage. A missed event is covered by
    // periodic reads; event failure must not hide an already committed setting.
    let _ = window
        .app_handle()
        .emit("ui-preferences-changed", saved.lock_version);
    Ok(saved)
}

#[tauri::command]
fn get_diarization_preferences(
    window: WebviewWindow,
) -> Result<storage::DiarizationPreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can read diarization settings".into());
    }
    window.state::<Storage>().diarization_preferences()
}
#[tauri::command]
fn save_diarization_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::DiarizationPreferences,
) -> Result<storage::DiarizationPreferencesSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can edit diarization settings".into());
    }
    window
        .state::<Storage>()
        .save_diarization_preferences(expected_lock_version, payload)
}
#[tauri::command]
async fn get_speaker_reviews(
    window: WebviewWindow,
    request: storage::SpeakerReviewRequest,
) -> Result<storage::SpeakerReviewResponse, String> {
    if window.label() != "main" {
        return Err("Only the main window can read speaker intervals".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().speaker_reviews(request))
        .await
        .map_err(|e| e.to_string())?
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

fn minutes_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    if window.label() != "main" {
        return Err("Only the main window can read or edit minutes".into());
    }
    Ok(window.app_handle().clone())
}

#[tauri::command]
async fn get_minutes_exports(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Vec<storage::ExportRecord>, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().exports(&meeting_id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn export_minutes_markdown(
    window: WebviewWindow,
    request: storage::ExportRequest,
) -> Result<Option<storage::ExportRecord>, String> {
    let app = minutes_app(&window)?;
    app.state::<Storage>().require_data_writable()?;
    let admission = app.state::<AnalysisController>().begin_export()?;
    let lease = app.state::<exports::ExportController>().begin()?;
    // Own both guards in a detached task: losing the command receiver must not
    // unblock backup/exit while a save dialog or blocking publication is alive.
    tauri::async_runtime::spawn(async move {
        let _admission = admission;
        let _lease = lease;
        let worker = app.clone();
        let query = request.clone();
        if let Some(record) = tauri::async_runtime::spawn_blocking(move || {
            worker.state::<Storage>().existing_export(&query)
        })
        .await
        .map_err(|e| e.to_string())??
        {
            return Ok(Some(record));
        }
        let worker = app.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            worker.state::<Storage>().prepare_export(request)
        })
        .await
        .map_err(|e| e.to_string())??;
        let Some(path) = exports::save_dialog(&window, prepared.filename.clone()).await? else {
            return Ok(None);
        };
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().write_export(prepared, &path)
        })
        .await
        .map_err(|e| e.to_string())?
        .map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportStatus {
    active: bool,
    record: Option<storage::ExportRecord>,
}
#[tauri::command]
async fn get_minutes_export_status(
    window: WebviewWindow,
    request: storage::ExportRequest,
) -> Result<ExportStatus, String> {
    let app = minutes_app(&window)?;
    let worker = app.clone();
    let record = tauri::async_runtime::spawn_blocking(move || {
        worker.state::<Storage>().existing_export(&request)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(ExportStatus {
        active: app.state::<exports::ExportController>().active(),
        record,
    })
}

#[tauri::command]
fn get_minutes_preferences(
    window: WebviewWindow,
) -> Result<storage::MinutesPreferencesSnapshot, String> {
    minutes_app(&window)?
        .state::<Storage>()
        .minutes_preferences()
}

#[tauri::command]
fn save_minutes_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::MinutesPreferences,
) -> Result<storage::MinutesPreferencesSnapshot, String> {
    minutes_app(&window)?
        .state::<Storage>()
        .save_minutes_preferences(expected_lock_version, payload)
}

#[tauri::command]
async fn get_minutes_versions(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::MinutesVersionState, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>().minutes_versions(&meeting_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_minutes_preview(
    window: WebviewWindow,
    meeting_id: String,
    minutes_id: String,
) -> Result<Option<storage::MinutesView>, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .load_minutes_content(&meeting_id, &minutes_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_minutes_draft(
    window: WebviewWindow,
    meeting_id: String,
    minutes_id: Option<String>,
) -> Result<Option<storage::MinutesDraft>, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .load_minutes_draft(&meeting_id, minutes_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn create_minutes_draft(
    window: WebviewWindow,
    meeting_id: String,
    minutes_id: String,
) -> Result<storage::MinutesDraft, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .create_minutes_draft(&meeting_id, &minutes_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_minutes_draft(
    window: WebviewWindow,
    draft_id: String,
    expected_lock_version: i64,
    payload: minutes_document::MinutesContent,
) -> Result<storage::MinutesDraft, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .save_minutes_draft(&draft_id, expected_lock_version, payload)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn undo_minutes_edit(
    window: WebviewWindow,
    draft_id: String,
    expected_lock_version: i64,
) -> Result<storage::MinutesDraft, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>()
            .undo_minutes_edit(&draft_id, expected_lock_version)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn adopt_minutes_candidate(
    window: WebviewWindow,
    request: storage::MinutesAdoptRequest,
) -> Result<storage::CommittedMinutes, String> {
    let app = minutes_app(&window)?;
    let meeting = request.meeting_id.clone();
    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        worker.state::<Storage>().adopt_minutes(request)
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = app.emit("minutes-version-changed", meeting);
    Ok(result)
}

#[tauri::command]
async fn restore_minutes_version(
    window: WebviewWindow,
    meeting_id: String,
    expected_meeting_lock_version: i64,
    expected_previous_id: String,
    expected_content_hash: String,
    expected_draft: Option<storage::DraftStamp>,
) -> Result<storage::CommittedMinutes, String> {
    let app = minutes_app(&window)?;
    let meeting = meeting_id.clone();
    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        worker.state::<Storage>().restore_minutes_checked(
            &meeting_id,
            expected_meeting_lock_version,
            &expected_previous_id,
            &expected_content_hash,
            expected_draft,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = app.emit("minutes-version-changed", meeting);
    Ok(result)
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
fn get_analysis_flow(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
) -> Result<Option<storage::analysis_flows::AnalysisFlow>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read analysis flow".into());
    }
    storage.analysis_flow(&meeting_id)
}

#[tauri::command]
fn start_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::analysis_flows::AnalysisFlow, String> {
    if window.label() != "main" {
        return Err("Only the main window can start analysis flow".into());
    }
    let flow = window
        .state::<Storage>()
        .ensure_analysis_flow(&meeting_id)?;
    window
        .app_handle()
        .emit("analysis-flow-changed", flow.clone())
        .map_err(|e| e.to_string())?;
    AnalysisController::kick(window.app_handle())?;
    Ok(flow)
}

#[tauri::command]
fn continue_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
    expected_lock_version: i64,
) -> Result<storage::analysis_flows::AnalysisFlow, String> {
    if window.label() != "main" {
        return Err("Only the main window can continue analysis flow".into());
    }
    let flow = analysis_flow::continue_flow(
        &window.state::<Storage>(),
        &meeting_id,
        expected_lock_version,
    )?;
    window
        .app_handle()
        .emit("analysis-flow-changed", flow.clone())
        .map_err(|e| e.to_string())?;
    AnalysisController::kick(window.app_handle())?;
    Ok(flow)
}

#[tauri::command]
async fn start_diarization(
    window: WebviewWindow,
    request: diarization::StartRequest,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start diarization".into());
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
async fn retry_diarization(
    window: WebviewWindow,
    request: diarization::RetryRequest,
) -> Result<storage::JobSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can retry diarization".into());
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
async fn get_diarization_tracks(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::RecordedAudioPlan, String> {
    if window.label() != "main" {
        return Err("Only the main window can read diarization tracks".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>().recorded_audio_plan(&meeting_id)
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
fn get_analysis_notification_jobs(
    window: WebviewWindow,
    watched_job_ids: Vec<String>,
) -> Result<Vec<storage::NotificationJob>, String> {
    if window.label() != "main" {
        return Err("解析通知の状態はメイン画面から確認してください".into());
    }
    window
        .state::<Storage>()
        .notification_jobs(&watched_job_ids)
}

#[tauri::command]
async fn get_analysis_notification_output(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::NotificationOutput, String> {
    if window.label() != "main" {
        return Err("解析通知の保存結果はメイン画面から確認してください".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>().notification_output(&job_id)
    })
    .await
    .map_err(|e| e.to_string())?
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
async fn retry_minutes_generation(
    window: WebviewWindow,
    request: generation::RetryRequest,
) -> Result<storage::JobSnapshot, String> {
    let app = minutes_app(&window)?;
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
async fn get_minutes_evidence_source(
    window: WebviewWindow,
    request: storage::MinutesEvidenceRequest,
) -> Result<storage::MinutesEvidenceSource, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>().minutes_evidence_source(&request)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_minutes_transcript_page(
    window: WebviewWindow,
    request: storage::MinutesTranscriptRequest,
) -> Result<storage::MinutesTranscriptPage, String> {
    let app = minutes_app(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Storage>().minutes_transcript_page(&request)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn register_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can prepare playback".into());
    }
    window
        .state::<PlaybackPreparations>()
        .reserve(&preparation_id)
}

#[tauri::command]
fn cancel_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can cancel playback preparation".into());
    }
    window
        .state::<PlaybackPreparations>()
        .cancel(&preparation_id, &window.state::<PlaybackStore>())
}

#[tauri::command]
fn finish_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
    playback_token: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can acknowledge playback preparation".into());
    }
    window
        .state::<PlaybackPreparations>()
        .acknowledge(&preparation_id, &playback_token)
}

#[tauri::command]
async fn prepare_minutes_transcript_playback(
    window: WebviewWindow,
    request: storage::MinutesTranscriptAudioRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, String> {
    let app = minutes_app(&window)?;
    let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        guard.check()?;
        let audio = app
            .state::<Storage>()
            .prepare_minutes_transcript_audio_cancellable(&request, &|| guard.cancelled())?;
        guard.publish(|| app.state::<PlaybackStore>().insert(audio))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_minutes_evidence_playback(
    window: WebviewWindow,
    request: storage::MinutesEvidenceRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, String> {
    let app = minutes_app(&window)?;
    let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        guard.check()?;
        let audio = app
            .state::<Storage>()
            .prepare_minutes_evidence_audio_cancellable(&request, &|| guard.cancelled())?;
        guard.publish(|| app.state::<PlaybackStore>().insert(audio))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_meeting_playback(
    window: WebviewWindow,
    request: storage::MeetingPlaybackRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, String> {
    if window.label() != "main" {
        return Err("Only the main window can play audio".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        guard.check()?;
        let audio = app
            .state::<Storage>()
            .prepare_meeting_audio_cancellable(&request, &|| guard.cancelled())?;
        guard.publish(|| app.state::<PlaybackStore>().insert_meeting(audio))
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
    preparation_id: String,
) -> Result<PlaybackInfo, String> {
    if window.label() != "main" {
        return Err("Only the main window can play audio".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        guard.check()?;
        let audio = app.state::<Storage>().prepare_utterance_audio_cancellable(
            &meeting_id,
            &analysis_id,
            &utterance_id,
            &|| guard.cancelled(),
        )?;
        guard.publish(|| app.state::<PlaybackStore>().insert(audio))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_draft_utterance_playback(
    window: WebviewWindow,
    request: storage::DraftUtteranceAudioRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, String> {
    if window.label() != "main" {
        return Err("Only the main window can play audio".into());
    }
    let app = window.app_handle().clone();
    let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        guard.check()?;
        let audio = app
            .state::<Storage>()
            .prepare_draft_utterance_audio_cancellable(&request, &|| guard.cancelled())?;
        guard.publish(|| app.state::<PlaybackStore>().insert(audio))
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
async fn finish_main_close(
    window: WebviewWindow,
    quit: bool,
    expected: Option<application_close::CloseExpectations>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can finish closing".into());
    }
    if window.state::<exports::ExportController>().active() {
        return Err("Markdownの保存・取消が完了してから終了してください".into());
    }
    if window
        .state::<ImportController>()
        .snapshot()?
        .recording_blocked
        && !window.state::<SessionStore>().snapshot().is_active()
    {
        return Err("録音開始の停止待ちが進行中です。完了後に終了してください".into());
    }
    if window.state::<SessionStore>().snapshot().is_active() {
        if quit {
            return Err("録音の停止と保存が完了していません".into());
        }
        return window.hide().map_err(|error| error.to_string());
    }
    // Keep the exit epilogue alive too: a lost close response must neither
    // release running work nor leave the open app stranded behind an exit gate.
    tauri::async_runtime::spawn(async move {
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            if expected.is_some() {
                app.state::<AnalysisController>().seal_exit()?;
            } else {
                app.state::<AnalysisController>().begin_exit()?;
            }
            let result = if let Some(target) = &expected {
                DataController::shutdown_checked(&app, &target.data_token)
                    .and_then(|_| ImportController::shutdown_checked(&app, &target.import_token))
            } else {
                DataController::shutdown(&app).and_then(|_| ImportController::shutdown(&app))
            }
            .and_then(|_| AnalysisController::shutdown(&app));
            if result.is_err() {
                app.state::<AnalysisController>().abort_shutdown();
                if expected.as_ref().is_none_or(|e| {
                    app.state::<ImportController>()
                        .snapshot()
                        .is_ok_and(|s| s.token == e.import_token)
                }) {
                    ImportController::abort_shutdown(&app);
                }
                if expected.as_ref().is_none_or(|e| {
                    app.state::<DataController>()
                        .snapshot()
                        .is_ok_and(|s| s.token == e.data_token)
                }) {
                    DataController::abort_shutdown(&app);
                }
                let _ = AnalysisController::kick(&app);
            }
            result
        })
        .await
        .map_err(|e| e.to_string())??;
        window.state::<ExitGuard>().0.store(true, Ordering::SeqCst);
        window.app_handle().exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_recording_snapshot(store: State<'_, SessionStore>) -> RecordingSnapshot {
    store.snapshot()
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationCloseActivity {
    analysis_busy: bool,
    model_preparation_busy: bool,
}
#[tauri::command]
fn get_application_close_activity(
    window: WebviewWindow,
) -> Result<ApplicationCloseActivity, String> {
    if window.label() != "main" {
        return Err("終了状態はメイン画面から確認してください".into());
    }
    let (analysis_busy, model_preparation_busy) =
        window.state::<AnalysisController>().close_activity()?;
    Ok(ApplicationCloseActivity {
        analysis_busy,
        model_preparation_busy,
    })
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
    microphone_uid: String,
    capture_scope: capture_scope::CaptureScope,
) -> Result<RecordingSnapshot, String> {
    if window.label() != "main" {
        return Err("Only the main window can start recording".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        RecorderController::with_transition(&app, || {
            if app.state::<StopDecisionStore>().snapshot().visible() {
                return Err("録音確認を完了してから開始してください".into());
            }
            if app.state::<SessionStore>().snapshot().is_active() {
                return Err("別の録音が進行中です".into());
            }
            app.state::<Storage>().require_data_writable()?;
            RecorderController::inputs(&app)?.require_microphone(&microphone_uid)?;
            RecorderController::require_scope(&app, &capture_scope)?;
            ImportController::prepare_recording(&app)?;
            let result = AnalysisController::prepare_recording(&app).and_then(|_| {
                RecorderController::start(&app, &meeting_id, &microphone_uid, &capture_scope)
            });
            if result.is_err() {
                AnalysisController::recording_finished(&app);
            }
            result
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_capture_inputs(window: WebviewWindow) -> Result<capture_inputs::InputDevices, String> {
    if window.label() != "main" {
        return Err("マイク一覧はメイン画面で確認してください".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || RecorderController::inputs(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_capture_applications(
    window: WebviewWindow,
) -> Result<capture_scope::Applications, String> {
    if window.label() != "main" {
        return Err("録音対象アプリの一覧はメイン画面で確認してください".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || RecorderController::applications(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop_recording(window: WebviewWindow) -> Result<RecordingSnapshot, String> {
    if !matches!(window.label(), "main" | "recorder") {
        return Err("Only the recording windows can stop recording".into());
    }
    if window.state::<StopDecisionStore>().snapshot().visible() {
        return Err("開いている録音確認で決定してください。停止は重複送信していません".into());
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
fn get_transcript_edit_recovery(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
    draft_id: String,
    base_analysis_id: String,
) -> Result<Option<storage::edit_recovery::Recovery>, String> {
    if window.label() != "main" {
        return Err("Only the main window can read transcript recovery".into());
    }
    storage.transcript_edit_recovery(&meeting_id, &draft_id, &base_analysis_id)
}

#[tauri::command]
fn list_transcript_edit_recoveries(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
    after: Option<String>,
) -> Result<storage::edit_recovery::review::Page, String> {
    if window.label() != "main" {
        return Err("Only the main window can list transcript recovery".into());
    }
    storage.list_transcript_edit_recoveries(&meeting_id, after.as_deref())
}

#[tauri::command]
fn discard_transcript_edit_recovery(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    request: storage::edit_recovery::review::DiscardRequest,
) -> Result<storage::edit_recovery::Recovery, String> {
    if window.label() != "main" {
        return Err("Only the main window can discard transcript recovery".into());
    }
    storage.discard_transcript_edit_recovery(request)
}

#[tauri::command]
fn save_transcript_edit_recovery(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    request: storage::edit_recovery::SaveRequest,
) -> Result<storage::edit_recovery::Recovery, String> {
    if window.label() != "main" {
        return Err("Only the main window can save transcript recovery".into());
    }
    storage.save_transcript_edit_recovery(request)
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
        .manage(StopDecisionStore::default())
        .manage(ExitGuard::default())
        .manage(PlaybackStore::default())
        .manage(PlaybackPreparations::default())
        .manage(AnalysisController::default())
        .manage(ImportController::default())
        .manage(DataController::default())
        .manage(model_inspection::Inspector::default())
        .manage(exports::ExportController::default())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            #[cfg(target_os = "macos")]
            {
                let selection = bootstrap::Bootstrap::open(&app.path().app_config_dir()?, &root)?;
                let storage = selection.open_storage()?;
                app.manage(storage);
                app.manage(selection);
            }
            #[cfg(not(target_os = "macos"))]
            app.manage(Storage::open(root)?);
            ImportController::recover(app.handle())?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                // Recovery can hash hours of audio, so never hold up window startup.
                match handle.state::<Storage>().recover_completed_recordings() {
                    Ok(ids) if !ids.is_empty() => {
                        let _ = handle.emit("recordings-recovered", ids);
                        let _ = AnalysisController::kick(&handle);
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("録音回復の確認に失敗しました: {error}"),
                }
            });
            AnalysisController::kick(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inspect_model_package,
            preview_model_deletion,
            cancel_model_inspection,
            get_model_placements,
            get_model_profiles,
            get_model_catalog_history,
            preview_model_profile_application,
            apply_model_profile_application,
            preview_job_model_rebinding,
            apply_job_model_rebinding,
            get_job_model_rebinding_result,
            check_model_compatibility,
            place_model_package,
            get_audio_import,
            select_audio_import,
            start_audio_import,
            cancel_audio_import,
            get_data_operation,
            #[cfg(unix)]
            get_deletion_items,
            #[cfg(unix)]
            review_meeting_deletion,
            #[cfg(unix)]
            review_pending_deletion,
            #[cfg(unix)]
            get_deletion_paths,
            #[cfg(unix)]
            get_model_deletion_items,
            #[cfg(unix)]
            review_model_body_deletion,
            #[cfg(unix)]
            review_pending_model_deletion,
            #[cfg(unix)]
            get_model_deletion_paths,
            #[cfg(unix)]
            get_audio_deletion_items,
            #[cfg(unix)]
            review_meeting_audio_deletion,
            #[cfg(unix)]
            review_pending_audio_deletion,
            #[cfg(unix)]
            get_audio_deletion_paths,
            #[cfg(target_os = "macos")]
            get_storage_bootstrap,
            review_storage_move,
            get_recovery_items,
            review_recovery_item,
            get_data_operation_availability,
            list_restored_meeting_ids,
            select_data_operation,
            start_data_operation,
            cancel_data_operation,
            get_recording_snapshot,
            get_application_close_activity,
            capture_available,
            get_capture_inputs,
            get_capture_applications,
            start_recording,
            stop_recording,
            get_recording_confirmation,
            request_recording_confirmation,
            decide_recording_confirmation,
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
            get_transcript_edit_recovery,
            list_transcript_edit_recoveries,
            discard_transcript_edit_recovery,
            save_transcript_edit_recovery,
            save_transcript_draft,
            undo_transcript_edit,
            prepare_utterance_playback,
            register_playback_preparation,
            cancel_playback_preparation,
            finish_playback_preparation,
            prepare_meeting_playback,
            prepare_draft_utterance_playback,
            get_minutes_evidence_source,
            get_minutes_transcript_page,
            prepare_minutes_transcript_playback,
            prepare_minutes_evidence_playback,
            read_playback_chunk,
            close_playback,
            finish_main_close,
            start_transcription,
            get_analysis_flow,
            start_analysis_flow,
            continue_analysis_flow,
            start_diarization,
            retry_diarization,
            get_diarization_tracks,
            start_minutes_generation,
            get_minutes_versions,
            get_minutes_exports,
            get_minutes_export_status,
            export_minutes_markdown,
            get_minutes_preferences,
            save_minutes_preferences,
            get_minutes_preview,
            get_minutes_draft,
            create_minutes_draft,
            save_minutes_draft,
            undo_minutes_edit,
            adopt_minutes_candidate,
            restore_minutes_version,
            get_analysis_jobs,
            get_analysis_notification_jobs,
            get_analysis_notification_output,
            retry_transcription,
            retry_minutes_generation,
            pause_analysis_job,
            resume_analysis_job,
            cancel_analysis_job,
            get_analysis_preferences,
            get_ui_preferences,
            save_ui_preferences,
            get_diarization_preferences,
            save_diarization_preferences,
            get_speaker_reviews,
            get_vocabulary_book,
            get_meeting_vocabulary,
            save_vocabulary_book,
            save_analysis_preferences,
            get_analysis_versions,
            get_analysis_preview,
            get_analysis_version_draft,
            adopt_analysis_candidate,
            restore_analysis_version
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == recording_confirmation_window::LABEL {
                    api.prevent_close();
                    recording_confirmation_window::close_requested(window.app_handle());
                } else if window.label() == "recorder" {
                    api.prevent_close();
                    recording_confirmation_window::recorder_close_requested(
                        window.app_handle(),
                        window.state::<SessionStore>().snapshot(),
                    );
                } else if window.label() == "main" {
                    api.prevent_close();
                    if window.state::<StopDecisionStore>().snapshot().visible() {
                        recording_confirmation_window::focus(window.app_handle());
                    } else if window.state::<SessionStore>().snapshot().is_active() {
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
                    if app.state::<StopDecisionStore>().snapshot().visible() {
                        recording_confirmation_window::focus(app);
                        return;
                    }
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
