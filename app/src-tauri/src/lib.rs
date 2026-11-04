mod analysis_flow;
mod analysis_preparation;
mod analysis_worker;
mod application_close;
mod audio;
mod audio_clock;
mod audio_import;
#[cfg(any(target_os = "macos", windows))]
mod bootstrap;
mod capture_inputs;
mod capture_manifest;
mod capture_scope;
mod capture_test;
mod command_roles;
mod credits;
mod current_formats;
mod data_controller;
mod diarization;
mod exports;
mod generation;
mod gpu;
mod import_controller;
mod input_levels;
mod input_status;
mod internal_logs;
#[cfg(test)]
mod ipc_acl;
pub(crate) mod job_codec;
mod local_engine;
#[cfg(target_os = "macos")]
mod macos_menu;
mod model_application;
mod model_asr_catalog;
#[cfg(target_os = "macos")]
mod model_bundled_runtime;
mod model_compatibility;
mod model_download;
mod model_download_ipc;
mod model_download_recipe;
mod model_inspection;
mod model_license;
mod model_official_catalog;
mod model_placement;
mod native_fs;
mod operation_observation;
#[cfg(target_os = "windows")]
mod platform_dialog;
mod playback;
mod processing_optimization;
mod recorder;
mod recorder_info;
mod recorder_operation_notices;
mod recorder_panel;
mod recording_confirmation;
mod recording_confirmation_window;
mod recording_recovery;
#[cfg(any(unix, windows))]
mod recording_segments;
mod recording_writes;
mod resampling;
mod residency;
mod restored_processing;
#[cfg(target_os = "macos")]
mod rounded_windows;
mod saved_artifact;
mod session;
mod speaker_completion;
mod speaker_features;
mod startup_attention;
mod startup_instance;
mod startup_notices;
mod startup_reset;
mod storage;
mod transcription;
mod voice;
#[cfg(target_os = "windows")]
mod window_chrome;
mod window_environment;
#[cfg(windows)]
mod windows_appearance;
#[cfg(target_os = "windows")]
mod windows_files;
#[cfg(all(test, target_os = "windows"))]
mod windows_integration_tests;
#[cfg(target_os = "windows")]
mod windows_runtime;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod test_profile_guards {
    #[test]
    fn efficient_test_profile_preserves_debug_assertions_and_overflow_panics() {
        assert!(cfg!(debug_assertions));
        assert!(std::panic::catch_unwind(|| debug_assert!(std::hint::black_box(false))).is_err());
        assert!(std::panic::catch_unwind(|| {
            std::hint::black_box(u64::MAX) + std::hint::black_box(1_u64)
        })
        .is_err());
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!("{:x}", Sha256::digest(vec![0x5a; 1024 * 1024])),
            "bf63d8a95fcc2e64619813aae35fdcbe871fdd9264caa3f365eb3aed0f679129"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

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
fn cancel_model_inspection(
    window: WebviewWindow,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can cancel model inspection".into());
        }
        window.state::<model_inspection::Inspector>().cancel(&token)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("cancel_model_inspection", message)
    })
}

#[tauri::command]
fn get_model_placements(
    window: WebviewWindow,
) -> Result<Vec<model_placement::CatalogEntry>, mon_contracts::AppError> {
    let result: Result<Vec<model_placement::CatalogEntry>, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read model placements".into());
        }
        window.state::<Storage>().list_model_placements()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_model_placements", message))
}

#[tauri::command]
fn get_model_profiles(
    window: WebviewWindow,
) -> Result<Vec<storage::model_profiles::ProfileView>, mon_contracts::AppError> {
    let result: Result<Vec<storage::model_profiles::ProfileView>, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read model profiles".into());
        }
        window.state::<Storage>().model_profiles()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_model_profiles", message))
}
#[tauri::command]
async fn preview_model_profile_application(
    window: WebviewWindow,
    token: String,
    profile: storage::model_profiles::ProfileName,
    package_id: String,
    expected_review_hash: String,
    check_operation_id: String,
    diarization_threads: Option<u32>,
    diarization_cluster_threshold_milli: Option<u32>,
    whisper_vad_options: Option<transcription::vad::VadOptions>,
    diarization_speech_sensitivity_milli: Option<i32>,
) -> Result<storage::model_profiles::ApplyPreview, mon_contracts::AppError> {
    let result: Result<storage::model_profiles::ApplyPreview, String> = async move {
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
            AnalysisController::with_model_preparation(
                &app,
                Some(&token),
                &|| guard.cancelled(),
                |lease| {
                    model_application::preview_with_sensitivity(
                        &app.state::<Storage>(),
                        &models_root,
                        profile,
                        &package_id,
                        &expected_review_hash,
                        &check_operation_id,
                        diarization_threads,
                        diarization_cluster_threshold_milli,
                        whisper_vad_options,
                        diarization_speech_sensitivity_milli,
                        &|| guard.cancelled() || lease.cancelled(),
                    )
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("preview_model_profile_application", message)
    })
}
#[tauri::command]
async fn apply_model_profile_application(
    window: WebviewWindow,
    token: String,
    preview: storage::model_profiles::ApplyPreview,
) -> Result<storage::model_profiles::ProfileView, mon_contracts::AppError> {
    let result: Result<storage::model_profiles::ProfileView, String> = async move {
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
            AnalysisController::with_model_preparation(
                &app,
                Some(&token),
                &|| guard.cancelled(),
                |lease| {
                    model_application::apply(
                        &app.state::<Storage>(),
                        &models_root,
                        &token,
                        &preview,
                        &|| guard.cancelled() || lease.cancelled(),
                    )
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("apply_model_profile_application", message)
    })
}

#[tauri::command]
async fn check_model_compatibility(
    window: WebviewWindow,
    token: String,
    package_id: String,
    expected_review_hash: String,
) -> Result<model_compatibility::CheckSummary, mon_contracts::AppError> {
    let result: Result<model_compatibility::CheckSummary, String> = async move {
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
            AnalysisController::with_model_preparation(
                &app,
                Some(&token),
                &|| guard.cancelled(),
                |lease| {
                    model_compatibility::check(
                        &app.state::<Storage>(),
                        &models_root,
                        &token,
                        &package_id,
                        &expected_review_hash,
                        &|| guard.cancelled() || lease.cancelled(),
                    )
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("check_model_compatibility", message)
    })
}

#[tauri::command]
fn get_audio_import(
    window: WebviewWindow,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read imports".into());
        }
        window.state::<ImportController>().snapshot()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_audio_import", message))
}
#[tauri::command]
async fn select_audio_import(
    window: WebviewWindow,
    path: Option<std::path::PathBuf>,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can select audio".into());
        }
        ImportController::select(&window, path).await
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("select_audio_import", message))
}
#[tauri::command]
fn start_audio_import(
    window: WebviewWindow,
    request: import_controller::StartRequest,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can import audio".into());
        }
        ImportController::start(window.app_handle(), request)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("start_audio_import", message))
}
#[tauri::command]
fn cancel_audio_import(
    window: WebviewWindow,
    token: String,
) -> Result<import_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<import_controller::Snapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can cancel imports".into());
        }
        ImportController::cancel_from_app(window.app_handle(), &token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_audio_import", message))
}

fn data_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    if window.label() != "main" {
        return Err("Only the main window can manage stored data".into());
    }
    Ok(window.app_handle().clone())
}

#[tauri::command]
fn get_data_operation(
    window: WebviewWindow,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> =
        (|| data_app(&window)?.state::<DataController>().snapshot())();
    result.map_err(|message| mon_contracts::AppError::from_command("get_data_operation", message))
}

#[cfg(any(unix, windows))]
#[tauri::command]
async fn get_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            storage::deletion::operations::listing(&app.state::<Storage>())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_deletion_items", message))
}
#[cfg(any(unix, windows))]
#[tauri::command]
async fn review_meeting_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_deletion(&app, meeting_id, false)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
async fn review_pending_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_deletion(&app, operation_id, true)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
fn get_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::operations::PathPage, mon_contracts::AppError> {
    let result: Result<storage::deletion::operations::PathPage, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .deletion_page(&confirmation_id, offset)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_deletion_paths", message))
}

#[cfg(any(target_os = "macos", windows))]
#[tauri::command]
async fn get_storage_bootstrap(
    window: WebviewWindow,
) -> Result<bootstrap::View, mon_contracts::AppError> {
    let result: Result<bootstrap::View, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let selection = app.state::<bootstrap::Bootstrap>();
            let mut view = selection.view()?;
            view.recovery = selection.recovery_view_for_storage(&app.state::<Storage>())?;
            Ok(view)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_storage_bootstrap", message))
}

#[cfg(any(unix, windows))]
#[tauri::command]
fn review_model_body_deletion(
    window: WebviewWindow,
    package_id: String,
    expected_review_hash: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = (|| {
        DataController::review_model_deletion(
            &data_app(&window)?,
            data_controller::ModelTarget::Fresh {
                package: package_id,
                review: expected_review_hash,
            },
        )
    })();
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
fn review_pending_model_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = (|| {
        DataController::review_model_deletion(
            &data_app(&window)?,
            data_controller::ModelTarget::Pending(operation_id),
        )
    })();
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
fn get_model_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::models::operations::PathPage, mon_contracts::AppError> {
    let result: Result<storage::deletion::models::operations::PathPage, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .model_deletion_page(&confirmation_id, offset)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_model_deletion_paths", message)
    })
}

#[cfg(any(unix, windows))]
#[tauri::command]
async fn get_model_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::models::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::models::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        let root = app.state::<Storage>().native_models_root().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            storage::deletion::models::operations::listing(&app.state::<Storage>(), &root)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_model_deletion_items", message)
    })
}

#[cfg(any(unix, windows))]
#[tauri::command]
async fn get_audio_deletion_items(
    window: WebviewWindow,
) -> Result<storage::deletion::audio::operations::Listing, mon_contracts::AppError> {
    let result: Result<storage::deletion::audio::operations::Listing, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            storage::deletion::audio::operations::listing(&app.state::<Storage>())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_audio_deletion_items", message)
    })
}
#[cfg(any(unix, windows))]
#[tauri::command]
async fn review_meeting_audio_deletion(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_audio_deletion(
                &app,
                data_controller::AudioTarget::Fresh(meeting_id),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
async fn review_pending_audio_deletion(
    window: WebviewWindow,
    operation_id: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            DataController::review_audio_deletion(
                &app,
                data_controller::AudioTarget::Pending(operation_id),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
}
#[cfg(any(unix, windows))]
#[tauri::command]
fn get_audio_deletion_paths(
    window: WebviewWindow,
    confirmation_id: String,
    offset: usize,
) -> Result<storage::deletion::audio::operations::PathPage, mon_contracts::AppError> {
    let result: Result<storage::deletion::audio::operations::PathPage, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .audio_deletion_page(&confirmation_id, offset)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_audio_deletion_paths", message)
    })
}

#[cfg(any(target_os = "macos", windows))]
#[tauri::command]
async fn review_storage_move(
    window: WebviewWindow,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || DataController::review_move(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("review_storage_move", message))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DataAvailability {
    available: bool,
    reason: Option<String>,
    reason_code: Option<&'static str>,
    deletion_available: bool,
    deletion_reason: Option<String>,
    deletion_reason_code: Option<&'static str>,
}

#[tauri::command]
async fn get_meeting_deletion_availability(
    window: WebviewWindow,
    meeting_ids: Vec<String>,
) -> Result<Vec<storage::meeting_access::Availability>, mon_contracts::AppError> {
    let result: Result<Vec<storage::meeting_access::Availability>, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let reason = app
                .state::<AnalysisController>()
                .meeting_deletion_blocker()?;
            let mut result = app
                .state::<Storage>()
                .meeting_deletion_availability(&meeting_ids)?;
            let recording = app.state::<SessionStore>().snapshot();
            if recording.is_active() {
                for item in &mut result {
                    if recording.meeting_id.is_none()
                        || recording.meeting_id.as_deref() == Some(item.meeting_id.as_str())
                    {
                        item.available = false;
                        if !item.blockers.iter().any(|b| b.kind == "recording") {
                            item.blockers.push(storage::meeting_access::Blocker {
                                kind: "recording".into(),
                                reason: "この会議の録音・音声保存が完了していません".into(),
                                job_id: None,
                            });
                        }
                    }
                }
            }
            if let Some(reason) = reason {
                for item in &mut result {
                    item.available = false;
                    item.blockers.push(storage::meeting_access::Blocker {
                        kind: "storage".into(),
                        reason: reason.clone(),
                        job_id: None,
                    });
                }
            }
            Ok(result)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_deletion_availability", message)
    })
}

#[tauri::command]
fn get_data_operation_availability(
    window: WebviewWindow,
) -> Result<DataAvailability, mon_contracts::AppError> {
    let result: Result<DataAvailability, String> = (|| {
        let app = data_app(&window)?;
        let importing = app.state::<ImportController>().snapshot()?;
        let (reason, reason_code) = if app.state::<SessionStore>().snapshot().is_active() {
            (Some("録音・音声保存中".into()), Some("recording_active"))
        } else if importing.busy || importing.recording_blocked {
            (
                Some("音声取り込み・録音開始の停止待ち中".into()),
                Some("import_active"),
            )
        } else {
            let controller = app.state::<AnalysisController>();
            let kind = controller.data_operation_blocker_kind()?;
            let reason = kind.map(|kind| AnalysisController::blocker_message(kind).to_owned());
            let code = kind.map(|kind| {
                if kind == mon_core::operation::Kind::Restart {
                    "relocation_restart_required"
                } else {
                    "operation_active"
                }
            });
            (reason, code)
        };
        let (reason, reason_code) =
            if reason.is_none() && app.state::<Storage>().has_scoped_meeting_deletion()? {
                (
                    Some("未完了の会議削除があります。削除の確認画面から解決してください".into()),
                    Some("meeting_deletion_pending"),
                )
            } else {
                (reason, reason_code)
            };
        let deletion_reason = reason.clone();
        Ok(DataAvailability {
            available: reason.is_none(),
            reason,
            reason_code,
            deletion_available: deletion_reason.is_none(),
            deletion_reason,
            deletion_reason_code: reason_code,
        })
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_data_operation_availability", message)
    })
}

#[tauri::command]
async fn list_data_jobs(
    window: WebviewWindow,
) -> Result<Vec<data_controller::DataJob>, mon_contracts::AppError> {
    let result: Result<_, String> = (|| data_app(&window)?.state::<DataController>().list_jobs())();
    result.map_err(|message| mon_contracts::AppError::from_command("list_data_jobs", message))
}

#[tauri::command]
async fn dismiss_data_job(
    window: WebviewWindow,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<_, String> = (|| {
        data_app(&window)?
            .state::<DataController>()
            .dismiss_job(&token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("dismiss_data_job", message))
}

#[tauri::command]
async fn list_restored_meeting_ids(
    window: WebviewWindow,
) -> Result<Vec<String>, mon_contracts::AppError> {
    let result: Result<Vec<String>, String> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().restored_meeting_ids())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("list_restored_meeting_ids", message)
    })
}

#[tauri::command]
async fn select_data_operation(
    window: WebviewWindow,
    kind: data_controller::Kind,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> = async move {
        data_app(&window)?;
        DataController::select(&window, kind).await
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("select_data_operation", message))
}

#[tauri::command]
async fn start_data_operation(
    window: WebviewWindow,
    request: data_controller::Request,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, mon_contracts::AppError> = async move {
        let app = data_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || DataController::start(&app, request))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result
}

#[tauri::command]
fn cancel_data_operation(
    window: WebviewWindow,
    token: String,
) -> Result<data_controller::Snapshot, mon_contracts::AppError> {
    let result: Result<data_controller::Snapshot, String> =
        (|| DataController::cancel(&data_app(&window)?, &token))();
    result
        .map_err(|message| mon_contracts::AppError::from_command("cancel_data_operation", message))
}

#[tauri::command]
fn get_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
) -> Result<storage::VocabularyBook, mon_contracts::AppError> {
    let result: Result<storage::VocabularyBook, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read vocabulary".into());
        }
        window
            .state::<Storage>()
            .vocabulary_book(meeting_id.as_deref())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_vocabulary_book", message))
}
#[tauri::command]
fn get_meeting_vocabulary(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::VocabularyView, mon_contracts::AppError> {
    let result: Result<storage::VocabularyView, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read vocabulary".into());
        }
        window.state::<Storage>().vocabulary_view(&meeting_id)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_meeting_vocabulary", message))
}
#[tauri::command]
fn save_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
    expected_lock_version: i64,
    payload: Vec<storage::VocabularyEntry>,
) -> Result<storage::VocabularyBook, mon_contracts::AppError> {
    let result: Result<storage::VocabularyBook, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can edit vocabulary".into());
        }
        window.state::<Storage>().save_vocabulary_book(
            meeting_id.as_deref(),
            expected_lock_version,
            payload,
        )
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("save_vocabulary_book", message))
}

#[tauri::command]
fn get_asr_preferences(
    window: WebviewWindow,
) -> Result<storage::PreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::PreferencesSnapshot, String> =
        (|| get_analysis_preferences(window).map_err(Into::into))();
    result.map_err(|message| mon_contracts::AppError::from_command("get_asr_preferences", message))
}

#[tauri::command]
fn get_asr_backends(
    window: WebviewWindow,
) -> Result<Vec<transcription::asr::Descriptor>, mon_contracts::AppError> {
    let result: Result<Vec<transcription::asr::Descriptor>, String> = (|| {
        if window.label() != "main" {
            return Err("メイン画面で操作してください".into());
        }
        Ok(transcription::asr::registry()
            .iter()
            .map(|a| a.descriptor())
            .collect())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_asr_backends", message))
}

#[tauri::command]
fn get_analysis_preferences(
    window: WebviewWindow,
) -> Result<storage::PreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::PreferencesSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read analysis settings".into());
        }
        window.state::<Storage>().analysis_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_preferences", message)
    })
}

#[tauri::command]
fn get_ui_preferences(
    window: WebviewWindow,
) -> Result<storage::UiPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::UiPreferencesSnapshot, String> = (|| {
        if !matches!(
            window.label(),
            "main" | "recorder" | recording_confirmation_window::LABEL | recorder_info::LABEL
        ) {
            return Err("Only application windows can read presentation settings".into());
        }
        window.state::<Storage>().ui_preferences()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_ui_preferences", message))
}

#[tauri::command]
fn save_ui_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::UiPreferences,
) -> Result<storage::UiPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::UiPreferencesSnapshot, String> = (|| {
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
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("save_ui_preferences", message))
}

#[tauri::command]
fn get_diarization_preferences(
    window: WebviewWindow,
) -> Result<storage::DiarizationPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::DiarizationPreferencesSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read diarization settings".into());
        }
        window.state::<Storage>().diarization_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_diarization_preferences", message)
    })
}
#[tauri::command]
async fn get_analysis_versions(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::VersionState, mon_contracts::AppError> {
    let result: Result<storage::VersionState, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis versions".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().analysis_versions(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_analysis_versions", message))
}

#[tauri::command]
async fn start_markdown_minutes_generation(
    window: WebviewWindow,
    request: generation::StartRequest,
    expected: Option<storage::minutes_markdown::MarkdownStamp>,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let job = request.enqueue_markdown(&app.state::<Storage>(), expected)?;
            AnalysisController::kick(&app)?;
            Ok(job)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("start_markdown_minutes_generation", message)
    })
}

fn minutes_app(window: &WebviewWindow) -> Result<tauri::AppHandle, String> {
    if window.label() != "main" {
        return Err("Only the main window can read or edit minutes".into());
    }
    Ok(window.app_handle().clone())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportStatus {
    active: bool,
    record: Option<storage::ExportRecord>,
}
#[tauri::command]
async fn export_plain_minutes_markdown(
    window: WebviewWindow,
    request: storage::markdown_exports::MarkdownExportRequest,
) -> Result<Option<storage::ExportRecord>, mon_contracts::AppError> {
    let result: Result<Option<storage::ExportRecord>, String> = async move {
        let app = minutes_app(&window)?;
        app.state::<Storage>().require_data_writable()?;
        let meeting_owner =
            app.state::<Storage>()
                .begin_meeting_work(&request.meeting_id, "export", None)?;
        let admission = app.state::<AnalysisController>().begin_export()?;
        let lease = app.state::<exports::ExportController>().begin()?;
        // A lost receiver does not release admission while the native dialog or
        // durable publication is still alive. Status reads never reopen a dialog.
        tauri::async_runtime::spawn(async move {
            let _admission = admission;
            let _meeting_owner = meeting_owner;
            let _lease = lease;
            let worker = app.clone();
            let query = request.clone();
            if let Some(record) = tauri::async_runtime::spawn_blocking(move || {
                worker.state::<Storage>().existing_markdown_export(&query)
            })
            .await
            .map_err(|e| e.to_string())??
            {
                return Ok(Some(record));
            }
            let worker = app.clone();
            let prepared = tauri::async_runtime::spawn_blocking(move || {
                worker.state::<Storage>().prepare_markdown_export(request)
            })
            .await
            .map_err(|e| e.to_string())??;
            let Some(path) = exports::save_dialog(&window, prepared.filename.clone()).await? else {
                return Ok(None);
            };
            tauri::async_runtime::spawn_blocking(move || {
                app.state::<Storage>()
                    .write_markdown_export(prepared, &path)
            })
            .await
            .map_err(|e| e.to_string())?
            .map(Some)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("export_plain_minutes_markdown", message)
    })
}
#[tauri::command]
async fn get_plain_minutes_export_status(
    window: WebviewWindow,
    request: storage::markdown_exports::MarkdownExportRequest,
) -> Result<ExportStatus, mon_contracts::AppError> {
    let result: Result<ExportStatus, String> = async move {
        let app = minutes_app(&window)?;
        // A publication can finish between the record query and the final active
        // read. If it was alive before querying, require another read rather than
        // misreporting an absent receipt as a safely stopped operation.
        let was_active = app.state::<exports::ExportController>().active();
        let worker = app.clone();
        let record = tauri::async_runtime::spawn_blocking(move || {
            worker.state::<Storage>().existing_markdown_export(&request)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(ExportStatus {
            active: was_active || app.state::<exports::ExportController>().active(),
            record,
        })
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_plain_minutes_export_status", message)
    })
}

#[tauri::command]
async fn get_minutes_preferences(
    window: WebviewWindow,
) -> Result<storage::MinutesPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::MinutesPreferencesSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().minutes_preferences())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_minutes_preferences", message)
    })
}

#[tauri::command]
async fn get_minutes_prompt_preferences(
    window: WebviewWindow,
) -> Result<
    storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
    mon_contracts::AppError,
> {
    let result: Result<
        storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
        String,
    > = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().minutes_prompt_preferences()
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_minutes_prompt_preferences", message)
    })
}

#[tauri::command]
async fn save_minutes_prompt_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::minutes_prompt_preferences::MinutesPromptPreferences,
) -> Result<
    storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
    mon_contracts::AppError,
> {
    let result: Result<
        storage::minutes_prompt_preferences::MinutesPromptPreferencesSnapshot,
        String,
    > = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .save_minutes_prompt_preferences(expected_lock_version, payload)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_minutes_prompt_preferences", message)
    })
}

#[tauri::command]
async fn get_minutes_markdown(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<storage::minutes_markdown::MarkdownDocument>, mon_contracts::AppError> {
    let result: Result<Option<storage::minutes_markdown::MarkdownDocument>, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().get_minutes_markdown(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_minutes_markdown", message))
}

#[tauri::command]
async fn save_minutes_markdown(
    window: WebviewWindow,
    request: storage::minutes_markdown::SaveMarkdownRequest,
) -> Result<storage::minutes_markdown::MarkdownDocument, mon_contracts::AppError> {
    let result: Result<storage::minutes_markdown::MarkdownDocument, String> = async move {
        let app = minutes_app(&window)?;
        let worker = app.clone();
        let document = tauri::async_runtime::spawn_blocking(move || {
            worker.state::<Storage>().save_minutes_markdown(request)
        })
        .await
        .map_err(|e| e.to_string())??;
        let _ = app.emit("minutes-markdown-changed", &document.meeting_id);
        Ok(document)
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("save_minutes_markdown", message))
}

#[tauri::command]
async fn get_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<storage::analysis_flows::AnalysisFlow>, mon_contracts::AppError> {
    let result: Result<Option<storage::analysis_flows::AnalysisFlow>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis flow".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().analysis_flow(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_analysis_flow", message))
}

#[tauri::command]
async fn get_transcription_regeneration_target(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::analysis_flows::regeneration::Admission, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::regeneration::Admission, String> = async move {
        if window.label() != "main" {
            return Err("文字起こしの再生成はメイン画面から操作してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().regeneration_admission(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_transcription_regeneration_target", message)
    })
}
#[tauri::command]
async fn regenerate_transcription(
    window: WebviewWindow,
    request: storage::analysis_flows::regeneration::Request,
) -> Result<storage::analysis_flows::AnalysisFlow, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::AnalysisFlow, String> = async move {
        if window.label() != "main" {
            return Err("文字起こしの再生成はメイン画面から操作してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _preparing =
                analysis_preparation::begin(&request.meeting_id, &request.operation_id);
            if let Some(flow) = app.state::<Storage>().regeneration_replay(&request)? {
                return Ok(flow);
            }
            let flow =
                AnalysisController::with_model_preparation(&app, None, &|| false, |lease| {
                    app.state::<capture_test::CaptureTests>().require_empty()?;
                    app.state::<Storage>()
                        .regenerate_analysis(&request, &|| lease.cancelled())
                })?;
            let _ = app.emit("analysis-flow-changed", flow.clone());
            Ok(flow)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("regenerate_transcription", message)
    })
}

#[tauri::command]
fn get_analysis_preparations(
    window: WebviewWindow,
) -> Result<Vec<analysis_preparation::Progress>, mon_contracts::AppError> {
    let result: Result<Vec<analysis_preparation::Progress>, String> = (|| {
        if window.label() != "main" {
            return Err("解析準備はメイン画面から確認してください".into());
        }
        let mut entries = window.state::<Storage>().preparation_progress()?;
        for current in analysis_preparation::snapshot() {
            if let Some(saved) = entries
                .iter_mut()
                .find(|p| p.meeting_id == current.meeting_id)
            {
                let started = if saved.operation_id == current.operation_id {
                    saved.started_at_ms.min(current.started_at_ms)
                } else {
                    current.started_at_ms
                };
                *saved = current;
                saved.started_at_ms = started;
            } else {
                entries.push(current);
            }
        }
        for p in &mut entries {
            p.meeting_title = window
                .state::<Storage>()
                .get_meeting(&p.meeting_id)?
                .map(|m| m.title)
                .unwrap_or_else(|| "会議音声".into());
        }
        Ok(entries)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_preparations", message)
    })
}
#[tauri::command]
async fn get_meeting_analysis_audio(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::RecordedAudioPlan, mon_contracts::AppError> {
    let result: Result<storage::RecordedAudioPlan, String> = async move {
        if window.label() != "main" {
            return Err("解析音声はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>()
                .recorded_analysis_input(&meeting_id)
                .map(|(source, _)| source)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_analysis_audio", message)
    })
}

#[tauri::command]
async fn get_ready_analysis_flows(
    window: WebviewWindow,
) -> Result<Vec<storage::analysis_flows::AnalysisFlow>, mon_contracts::AppError> {
    let result: Result<Vec<storage::analysis_flows::AnalysisFlow>, String> = async move {
        if window.label() != "main" {
            return Err("文字起こしの保存はメイン画面から操作してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().ready_analysis_flows())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_ready_analysis_flows", message)
    })
}
#[tauri::command]
async fn publish_regenerated_transcription(
    window: WebviewWindow,
    meeting_id: String,
    flow_id: String,
    expected_lock_version: i64,
    confirmed_target: Option<storage::analysis_flows::regeneration::Target>,
) -> Result<storage::analysis_flows::regeneration::Publication, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::regeneration::Publication, String> = async move {
        if window.label() != "main" {
            return Err("文字起こしの保存はメイン画面から操作してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = app.state::<Storage>().publish_regenerated_flow(
                &meeting_id,
                &flow_id,
                expected_lock_version,
                confirmed_target.as_ref(),
            );
            if let Ok(Some(flow)) = app.state::<Storage>().analysis_flow(&meeting_id) {
                let _ = app.emit("analysis-flow-changed", flow);
            }
            if matches!(
                &result,
                Ok(storage::analysis_flows::regeneration::Publication::Published { .. })
            ) {
                if let Err(error) =
                    speaker_completion::after_analysis(&app.state::<Storage>(), &meeting_id)
                {
                    internal_logs::report_error("voice_matching", &error);
                }
                let _ = AnalysisController::kick(&app);
            }
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("publish_regenerated_transcription", message)
    })
}

#[tauri::command]
fn start_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::analysis_flows::AnalysisFlow, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::AnalysisFlow, String> = (|| {
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
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("start_analysis_flow", message))
}

#[tauri::command]
fn continue_analysis_flow(
    window: WebviewWindow,
    meeting_id: String,
    expected_lock_version: i64,
) -> Result<storage::analysis_flows::AnalysisFlow, mon_contracts::AppError> {
    let result: Result<storage::analysis_flows::AnalysisFlow, String> = (|| {
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
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("continue_analysis_flow", message))
}

#[tauri::command]
async fn get_voice_people(
    window: WebviewWindow,
) -> Result<Vec<storage::voice::Person>, mon_contracts::AppError> {
    let result: Result<Vec<storage::voice::Person>, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().voice_people())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_voice_people", message))
}
#[tauri::command]
async fn prepare_voice_sample_playback(
    window: WebviewWindow,
    sample_id: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        let app = minutes_app(&window)?;
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            guard.check()?;
            let audio = app
                .state::<Storage>()
                .prepare_voice_sample_audio_cancellable(&sample_id, &|| guard.cancelled())?;
            guard.publish(|| app.state::<PlaybackStore>().insert(audio))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_voice_sample_playback", message)
    })
}

#[tauri::command]
async fn get_voice_context(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::voice::Context, mon_contracts::AppError> {
    let result: Result<storage::voice::Context, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().voice_context(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_voice_context", message))
}
#[derive(serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum VoiceStartReply {
    Accepted { job: storage::JobSnapshot },
    Rejected { message: String },
}
#[tauri::command]
async fn start_voice_job(
    window: WebviewWindow,
    request: storage::voice::Start,
) -> Result<VoiceStartReply, mon_contracts::AppError> {
    let result: Result<VoiceStartReply, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            match app.state::<Storage>().start_voice(request) {
                Ok(job) => {
                    if let Err(error) = AnalysisController::kick(&app) {
                        internal_logs::report_error("voice_matching", &error);
                    }
                    VoiceStartReply::Accepted { job }
                }
                Err(message) => VoiceStartReply::Rejected { message },
            }
        })
        .await
        .map_err(|e| e.to_string())
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("start_voice_job", message))
}

#[tauri::command]
async fn manage_voice_person(
    window: WebviewWindow,
    person_id: String,
    expected_version: i64,
    name: Option<String>,
    sample_id: Option<String>,
) -> Result<storage::voice::PersonMutation, mon_contracts::AppError> {
    let result: Result<storage::voice::PersonMutation, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().manage_voice_person(
                &person_id,
                expected_version,
                name,
                sample_id,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("manage_voice_person", message))
}
#[tauri::command]
async fn retry_voice_audio_storage(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let result = app.state::<Storage>().preserve_voice_audio(None, &|| false);
            let _ = app.emit("voice-identity-changed", ());
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("retry_voice_audio_storage", message)
    })
}
#[tauri::command]
fn get_voice_audio_storage_state(
    window: WebviewWindow,
) -> Result<storage::voice::AudioStorageState, mon_contracts::AppError> {
    let result: Result<storage::voice::AudioStorageState, String> = (|| {
        let app = minutes_app(&window)?;
        app.state::<Storage>().voice_audio_storage_state()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_voice_audio_storage_state", message)
    })
}
#[tauri::command]
async fn decide_voice_match(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
    speaker_id: String,
    person_id: Option<String>,
    expected_working_hash: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().decide_voice_match(
                &meeting_id,
                &analysis_id,
                &speaker_id,
                person_id,
                &expected_working_hash,
            )?;
            let _ = app.emit("voice-identity-changed", &meeting_id);
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("decide_voice_match", message))
}
#[tauri::command]
async fn retry_voice_job(
    window: WebviewWindow,
    job_id: String,
    operation_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let job = if app.state::<Storage>().get_job(&job_id)?.kind == "speaker_completion" {
                speaker_completion::retry(&app.state::<Storage>(), &job_id, &operation_id)?
            } else {
                voice::retry(&app.state::<Storage>(), &job_id, &operation_id)?
            };
            AnalysisController::kick(&app)?;
            Ok(job)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("retry_voice_job", message))
}

#[tauri::command]
async fn get_analysis_jobs(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Vec<storage::JobSnapshot>, mon_contracts::AppError> {
    let result: Result<Vec<storage::JobSnapshot>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis jobs".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().list_jobs(&meeting_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_analysis_jobs", message))
}

#[tauri::command]
async fn get_analysis_notification_jobs(
    window: WebviewWindow,
    watched_job_ids: Vec<String>,
) -> Result<Vec<storage::NotificationJob>, mon_contracts::AppError> {
    let result: Result<Vec<storage::NotificationJob>, String> = async move {
        if window.label() != "main" {
            return Err("解析通知の状態はメイン画面から確認してください".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().notification_jobs(&watched_job_ids)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_notification_jobs", message)
    })
}

#[tauri::command]
async fn get_analysis_notification_output(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::NotificationOutput, mon_contracts::AppError> {
    let result: Result<storage::NotificationOutput, String> = async move {
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
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_analysis_notification_output", message)
    })
}

#[tauri::command]
fn pause_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can pause analysis".into());
        }
        window.state::<Storage>().request_job_pause(&job_id, "user")
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("pause_analysis_job", message))
}

#[tauri::command]
fn resume_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can resume analysis".into());
        }
        let job = window.state::<Storage>().resume_job(&job_id)?;
        AnalysisController::kick(window.app_handle())?;
        Ok(job)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("resume_analysis_job", message))
}

#[tauri::command]
async fn cancel_analysis_job(
    window: WebviewWindow,
    job_id: String,
) -> Result<storage::JobSnapshot, mon_contracts::AppError> {
    let result: Result<storage::JobSnapshot, String> = async move {
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
                        "解析の安全な停止を待っています。完了後に取り消しを再試行してください"
                            .into(),
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_analysis_job", message))
}

#[tauri::command]
fn register_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can prepare playback".into());
        }
        window
            .state::<PlaybackPreparations>()
            .reserve(&preparation_id)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("register_playback_preparation", message)
    })
}

#[tauri::command]
fn cancel_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can cancel playback preparation".into());
        }
        window
            .state::<PlaybackPreparations>()
            .cancel(&preparation_id, &window.state::<PlaybackStore>())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("cancel_playback_preparation", message)
    })
}

#[tauri::command]
fn finish_playback_preparation(
    window: WebviewWindow,
    preparation_id: String,
    playback_token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can acknowledge playback preparation".into());
        }
        window
            .state::<PlaybackPreparations>()
            .acknowledge(&preparation_id, &playback_token)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("finish_playback_preparation", message)
    })
}

#[tauri::command]
async fn prepare_meeting_playback(
    window: WebviewWindow,
    request: storage::MeetingPlaybackRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let mut audio = app
                        .state::<Storage>()
                        .prepare_meeting_audio_cancellable(&request, &|| guard.cancelled())?;

                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert_meeting(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_meeting_playback", message)
    })
}

#[tauri::command]
async fn get_meeting_audio_summary(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::MeetingAudioSummary, mon_contracts::AppError> {
    let result: Result<storage::MeetingAudioSummary, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let summary = app.state::<Storage>().meeting_audio_summary(&meeting_id)?;
            Ok(summary)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_audio_summary", message)
    })
}

#[tauri::command]
async fn prepare_markdown_interval_playback(
    window: WebviewWindow,
    request: storage::MeetingIntervalRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        let app = minutes_app(&window)?;
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let mut audio = app
                        .state::<Storage>()
                        .prepare_meeting_interval_cancellable(&request, &|| guard.cancelled())?;

                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert_interval(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_markdown_interval_playback", message)
    })
}

#[tauri::command]
async fn prepare_utterance_playback(
    window: WebviewWindow,
    meeting_id: String,
    analysis_id: String,
    utterance_id: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner =
                        app.state::<Storage>()
                            .begin_meeting_work(&meeting_id, "playback", None)?;
                    let audio = app.state::<Storage>().prepare_utterance_audio_cancellable(
                        &meeting_id,
                        &analysis_id,
                        &utterance_id,
                        &|| guard.cancelled(),
                    )?;
                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_utterance_playback", message)
    })
}

#[tauri::command]
async fn prepare_draft_utterance_playback(
    window: WebviewWindow,
    request: storage::DraftUtteranceAudioRequest,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can play audio".into());
        }
        let app = window.app_handle().clone();
        let guard = app.state::<PlaybackPreparations>().claim(&preparation_id)?;
        tauri::async_runtime::spawn_blocking(move || {
            crate::internal_logs::observe(
                crate::internal_logs::Context {
                    category: "playback".into(),
                    meeting_id: Some(request.meeting_id.clone()),
                    operation_id: Some(preparation_id.clone()),
                    ..Default::default()
                },
                || {
                    guard.check()?;
                    let owner = app.state::<Storage>().begin_meeting_work(
                        &request.meeting_id,
                        "playback",
                        None,
                    )?;
                    let audio = app
                        .state::<Storage>()
                        .prepare_draft_utterance_audio_cancellable(&request, &|| {
                            guard.cancelled()
                        })?;
                    guard.publish(|| {
                        let store = app.state::<PlaybackStore>();
                        store.own_meeting(store.insert(audio)?, owner)
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_draft_utterance_playback", message)
    })
}

#[tauri::command]
async fn read_playback_chunk(
    window: WebviewWindow,
    token: String,
    offset_frames: u64,
) -> Result<tauri::ipc::Response, mon_contracts::AppError> {
    let result: Result<tauri::ipc::Response, String> = async move {
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
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("read_playback_chunk", message))
}

#[tauri::command]
fn close_playback(
    window: WebviewWindow,
    store: State<'_, PlaybackStore>,
    token: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can close playback".into());
        }
        store.close(&token)?;
        window
            .state::<capture_test::CaptureTests>()
            .release_playback(&token)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("close_playback", message))
}

#[tauri::command]
async fn finish_main_close(
    window: WebviewWindow,
    quit: bool,
    expected: Option<application_close::CloseExpectations>,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        if cfg!(any(windows, target_os = "macos")) && !quit {
            return Err("画面の収納とアプリの終了は別の操作です".into());
        }
        if window.label() != "main" {
            return Err("Only the main window can finish closing".into());
        }
        window
            .state::<capture_test::CaptureTests>()
            .require_exit_ready()?;
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
        let recording = window.state::<SessionStore>().snapshot();
        if recording.is_active() {
            if quit || !recording.can_hide_main_window() {
                return Err("録音の停止と保存が完了していません".into());
            }
            return window.hide().map_err(|error| error.to_string());
        }
        // Keep the exit epilogue alive too: a lost close response must neither
        // release running work nor leave the open app stranded behind an exit gate.
        tauri::async_runtime::spawn(async move {
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                RecorderController::with_transition(&app, || {
                    app.state::<capture_test::CaptureTests>().seal_exit()?;
                    let admission = if expected.is_some() {
                        app.state::<AnalysisController>().seal_exit()
                    } else {
                        app.state::<AnalysisController>().begin_exit()
                    };
                    if admission.is_err() {
                        app.state::<capture_test::CaptureTests>().abort_exit()?;
                    }
                    admission
                })?;
                let result = if let Some(target) = &expected {
                    DataController::shutdown_checked(&app, &target.data_token).and_then(|_| {
                        ImportController::shutdown_checked(&app, &target.import_token)
                    })
                } else {
                    DataController::shutdown(&app).and_then(|_| ImportController::shutdown(&app))
                }
                .and_then(|_| AnalysisController::shutdown(&app));
                if result.is_err() {
                    app.state::<AnalysisController>().abort_shutdown();
                    app.state::<capture_test::CaptureTests>().abort_exit()?;
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
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("finish_main_close", message))
}

#[tauri::command]
fn get_recording_snapshot(store: State<'_, SessionStore>) -> RecordingSnapshot {
    store.snapshot()
}
#[tauri::command]
async fn get_capture_test_snapshot(
    window: WebviewWindow,
    run_id: Option<String>,
) -> Result<Option<capture_test::Snapshot>, mon_contracts::AppError> {
    let result: Result<Option<capture_test::Snapshot>, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<capture_test::CaptureTests>()
                .snapshot(run_id.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_test_snapshot", message)
    })
}
#[tauri::command]
async fn reconcile_capture_test_start(
    window: WebviewWindow,
    run_id: String,
) -> Result<Option<capture_test::Snapshot>, mon_contracts::AppError> {
    let result: Result<Option<capture_test::Snapshot>, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                app.state::<capture_test::CaptureTests>()
                    .reconcile_start(&run_id)
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("reconcile_capture_test_start", message)
    })
}
#[tauri::command]
async fn start_capture_test(
    window: WebviewWindow,
    request: capture_test::Request,
) -> Result<capture_test::Snapshot, mon_contracts::AppError> {
    let result: Result<capture_test::Snapshot, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から開始してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::start(&app, request)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("start_capture_test", message))
}
#[tauri::command]
async fn cancel_capture_test(
    window: WebviewWindow,
    run_id: String,
) -> Result<capture_test::Snapshot, mon_contracts::AppError> {
    let result: Result<capture_test::Snapshot, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から停止してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::cancel(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("cancel_capture_test", message))
}
#[tauri::command]
async fn discard_capture_test(
    window: WebviewWindow,
    run_id: String,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から閉じてください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::discard(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("discard_capture_test", message))
}
#[tauri::command]
async fn get_capture_test_levels(
    window: WebviewWindow,
    run_id: String,
) -> Result<input_levels::InputLevels, mon_contracts::AppError> {
    let result: Result<input_levels::InputLevels, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::levels(&app, &run_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_test_levels", message)
    })
}
#[tauri::command]
async fn prepare_capture_test_playback(
    window: WebviewWindow,
    run_id: String,
    track_kind: String,
    preparation_id: String,
) -> Result<PlaybackInfo, mon_contracts::AppError> {
    let result: Result<PlaybackInfo, String> = async move {
        if window.label() != "main" {
            return Err("録音前テストはメイン画面から再生してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_test::CaptureTests::prepare_playback(
                &app,
                &run_id,
                &track_kind,
                &preparation_id,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("prepare_capture_test_playback", message)
    })
}
#[tauri::command]
async fn get_recording_levels(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_levels::InputLevels, mon_contracts::AppError> {
    let result: Result<input_levels::InputLevels, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力レベルは録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::levels(&app, &session_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_recording_levels", message))
}
#[tauri::command]
async fn get_recording_writes(
    window: WebviewWindow,
    session_id: String,
) -> Result<recording_writes::Snapshot, mon_contracts::AppError> {
    let result: Result<recording_writes::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("書込み状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::writes(&app, &session_id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_recording_writes", message))
}
#[tauri::command]
async fn get_recording_input_status(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_status::Snapshot, mon_contracts::AppError> {
    let result: Result<input_status::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_status(&app, &session_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_input_status", message)
    })
}
#[tauri::command]
async fn get_recording_input_history_status(
    window: WebviewWindow,
    session_id: String,
) -> Result<input_status::persistence::Snapshot, mon_contracts::AppError> {
    let result: Result<input_status::persistence::Snapshot, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力履歴の保存状態は録音画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = RecorderController::input_history_status(&app, &session_id);
            input_status::diagnostics::read_result(
                "live-journal/input-journal-status.json",
                Some(&session_id),
                None,
                result.as_ref().err().map(String::as_str),
            );
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_input_history_status", message)
    })
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NoticeReply<T> {
    paused: bool,
    value: Option<T>,
}
impl<T> NoticeReply<T> {
    fn ready(value: T) -> Self {
        Self {
            paused: false,
            value: Some(value),
        }
    }
    fn paused() -> Self {
        Self {
            paused: true,
            value: None,
        }
    }
}

#[tauri::command]
async fn get_archived_recording_input_notices(
    window: WebviewWindow,
    after_session_id: Option<String>,
) -> Result<NoticeReply<input_status::archived::Page>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::archived::Page>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        // The blocking worker owns admission until its actual I/O ends, even if the
        // WebView/awaiting caller disappears. Do not hold any recorder/child lock.
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            if analysis.notice_storage_paused(None) {
                return Ok(NoticeReply::paused());
            }
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .page_scoped(
                    &app.state::<Storage>(),
                    &analysis,
                    after_session_id.as_deref(),
                    &visible,
                )
                .map(NoticeReply::ready)
                .or_else(|error| {
                    if error == input_status::archived::PAUSED {
                        Ok(NoticeReply::paused())
                    } else {
                        Err(error)
                    }
                })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_archived_recording_input_notices", message)
    })
}

#[tauri::command]
async fn get_archived_recording_input_notice_rows(
    window: WebviewWindow,
    targets: Vec<input_status::archived::Target>,
) -> Result<NoticeReply<input_status::archived::SelectedRows>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::archived::SelectedRows>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .selected(&app.state::<Storage>(), &analysis, targets, &visible)
                .map(NoticeReply::ready)
                .or_else(|error| {
                    if error == input_status::archived::PAUSED {
                        Ok(NoticeReply::paused())
                    } else {
                        Err(error)
                    }
                })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_archived_recording_input_notice_rows", message)
    })
}

#[tauri::command]
async fn acknowledge_archived_recording_input_notice(
    window: WebviewWindow,
    request: input_status::archived::Acknowledge,
) -> Result<NoticeReply<input_status::Receipt>, mon_contracts::AppError> {
    let result: Result<NoticeReply<input_status::Receipt>, String> = async move {
        if window.label() != "main" {
            return Err("保存後通知の受付・表示・終了はメイン画面から確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let analysis = app.state::<AnalysisController>();
            let lease = match analysis.try_begin_scoped_notice_storage(Some(&request.meeting_id))? {
                Some(lease) => lease,
                None => return Ok(NoticeReply::paused()),
            };
            let visible = || {
                input_status::notice_window_visible(
                    window.is_visible().ok(),
                    window.is_minimized().ok(),
                )
            };
            app.state::<input_status::archived::ArchivedInputs>()
                .acknowledge(&app.state::<Storage>(), &lease, request, &visible)
                .map(NoticeReply::ready)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command(
            "acknowledge_archived_recording_input_notice",
            message,
        )
    })
}

#[tauri::command]
async fn receive_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: Option<u64>,
    receive_id: Option<String>,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        if window.label() != "main" && window.label() != "recorder" {
            return Err("入力通知は録音画面から受付してください".into());
        }
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                generation,
                input_status::queue::Action::Receive,
                receive_id.as_deref(),
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("receive_recording_input_notice", message)
    })
}
#[tauri::command]
async fn render_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: u64,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                Some(generation),
                input_status::queue::Action::Render,
                None,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("render_recording_input_notice", message)
    })
}
#[tauri::command]
async fn close_recording_input_notice(
    window: WebviewWindow,
    session_id: String,
    notice_id: String,
    recipient_id: String,
    generation: u64,
) -> Result<input_status::Receipt, mon_contracts::AppError> {
    let result: Result<input_status::Receipt, String> = async move {
        let app = window.app_handle().clone();
        let target = window.label().to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::input_notice_action(
                &app,
                &session_id,
                &notice_id,
                &recipient_id,
                &target,
                Some(generation),
                input_status::queue::Action::Close,
                None,
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("close_recording_input_notice", message)
    })
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationCloseActivity {
    analysis_busy: bool,
    model_preparation_busy: bool,
    recording_recovery_busy: bool,
}
#[tauri::command]
fn get_application_close_activity(
    window: WebviewWindow,
) -> Result<ApplicationCloseActivity, mon_contracts::AppError> {
    let result: Result<ApplicationCloseActivity, String> = (|| {
        if window.label() != "main" {
            return Err("終了状態はメイン画面から確認してください".into());
        }
        let (analysis_busy, model_preparation_busy) =
            window.state::<AnalysisController>().close_activity()?;
        Ok(ApplicationCloseActivity {
            analysis_busy,
            model_preparation_busy,
            recording_recovery_busy: window.state::<recording_recovery::Recovery>().busy(),
        })
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_application_close_activity", message)
    })
}

#[tauri::command]
fn capture_available(window: WebviewWindow) -> Result<bool, mon_contracts::AppError> {
    let result: Result<bool, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can inspect capture availability".into());
        }
        Ok(RecorderController::available(window.app_handle()))
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("capture_available", message))
}

#[tauri::command]
async fn start_recording(
    window: WebviewWindow,
    meeting_id: String,
    microphone_uid: Option<String>,
    capture_scope: capture_scope::CaptureScope,
    new_title: Option<String>,
    num_speakers: Option<u32>,
    meeting_vocabulary: Option<Vec<storage::VocabularyEntry>>,
) -> Result<RecordingSnapshot, mon_contracts::AppError> {
    let result: Result<RecordingSnapshot, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can start recording".into());
        }
        let app = window.app_handle().clone();
        let speaker_counts = storage::recordings::SpeakerCounts { num_speakers };
        speaker_counts.validate()?;
        if new_title.is_none() && meeting_vocabulary.is_some() {
            return Err("既存会議の用語は会議の用語画面で保存してください".into());
        }
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                app.state::<capture_test::CaptureTests>().require_empty()?;
                if app.state::<StopDecisionStore>().snapshot().visible() {
                    return Err("録音確認を完了してから開始してください".into());
                }
                if app.state::<SessionStore>().snapshot().is_active() {
                    return Err("別の録音が進行中です".into());
                }
                app.state::<Storage>().require_data_writable()?;
                let inputs = RecorderController::inputs(&app)?;
                if let Some(uid) = microphone_uid.as_deref() {
                    inputs.require_microphone(uid)?;
                } else {
                    inputs.require_automatic_microphone()?;
                }
                RecorderController::require_scope(&app, &capture_scope)?;
                if let Some(title) = new_title {
                    app.state::<Storage>()
                        .create_recording_draft_with_vocabulary(
                            &meeting_id,
                            &title,
                            &meeting_vocabulary.unwrap_or_default(),
                        )?;
                }
                ImportController::prepare_recording(&app)?;
                let result = AnalysisController::prepare_recording(&app).and_then(|_| {
                    RecorderController::start(
                        &app,
                        &meeting_id,
                        microphone_uid.as_deref(),
                        &capture_scope,
                        &speaker_counts,
                    )
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
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("start_recording", message))
}

#[tauri::command]
async fn select_recording_microphone(
    window: WebviewWindow,
    session_id: String,
    microphone_uid: Option<String>,
) -> Result<u64, mon_contracts::AppError> {
    let result: Result<u64, String> = async move {
        if window.label() != "main" {
            return Err("マイク切替はメイン画面で行ってください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::with_transition(&app, || {
                RecorderController::select_input(&app, &session_id, microphone_uid.as_deref())
            })
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("select_recording_microphone", message)
    })
}

#[tauri::command]
async fn get_recording_microphone_selection(
    window: WebviewWindow,
    session_id: String,
) -> Result<recorder::MicrophoneSelectionStatus, mon_contracts::AppError> {
    let result: Result<recorder::MicrophoneSelectionStatus, String> = async move {
        if window.label() != "main" {
            return Err("マイク選択状態はメイン画面で確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            RecorderController::selection_status(&app, &session_id)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recording_microphone_selection", message)
    })
}

#[tauri::command]
async fn get_capture_inputs(
    window: WebviewWindow,
) -> Result<capture_inputs::InputDevices, mon_contracts::AppError> {
    let result: Result<capture_inputs::InputDevices, String> = async move {
        if window.label() != "main" {
            return Err("マイク一覧はメイン画面で確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::inputs(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_capture_inputs", message))
}

#[tauri::command]
async fn get_capture_applications(
    window: WebviewWindow,
) -> Result<capture_scope::Applications, mon_contracts::AppError> {
    let result: Result<capture_scope::Applications, String> = async move {
        if window.label() != "main" {
            return Err("録音対象アプリの一覧はメイン画面で確認してください".into());
        }
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || RecorderController::applications(&app))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_capture_applications", message)
    })
}

#[tauri::command]
fn hide_recorder_panel(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "recorder" {
            return Err("Only the recorder panel can hide itself".into());
        }
        recording_confirmation_window::hide_recorder(window.app_handle())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("hide_recorder_panel", message))
}

#[tauri::command]
fn open_main_window(window: WebviewWindow) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        if window.label() != "recorder" {
            return Err("Only the recorder panel can reopen the main window".into());
        }
        residency::restore(window.app_handle())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("open_main_window", message))
}

#[tauri::command]
async fn list_meetings(window: WebviewWindow) -> Result<Vec<Meeting>, mon_contracts::AppError> {
    let result: Result<Vec<Meeting>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can list meetings".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().list_meetings())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("list_meetings", message))
}

#[tauri::command]
async fn get_meeting(
    window: WebviewWindow,
    id: String,
) -> Result<Option<Meeting>, mon_contracts::AppError> {
    let result: Result<Option<Meeting>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read meetings".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().get_meeting(&id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_meeting", message))
}

#[tauri::command]
fn rename_meeting(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    id: String,
    title: String,
    expected_lock_version: i64,
) -> Result<Meeting, mon_contracts::AppError> {
    let result: Result<Meeting, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can rename meetings".into());
        }
        storage.rename_meeting(&id, &title, expected_lock_version)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("rename_meeting", message))
}

#[tauri::command]
async fn get_current_analysis(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<AnalysisView>, mon_contracts::AppError> {
    let result: Result<Option<AnalysisView>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read analysis".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().load_current_analysis(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_current_analysis", message))
}

#[tauri::command]
fn get_speaker_completion_preferences(
    window: WebviewWindow,
) -> Result<storage::completion_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::completion_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window.state::<Storage>().speaker_completion_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_speaker_completion_preferences", message)
    })
}
#[tauri::command]
fn save_speaker_completion_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::completion_preferences::CompletionPreferences,
) -> Result<storage::completion_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::completion_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window
            .state::<Storage>()
            .save_speaker_completion_preferences(expected_lock_version, payload)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_speaker_completion_preferences", message)
    })
}
#[tauri::command]
fn get_voice_matching_preferences(
    window: WebviewWindow,
) -> Result<storage::voice_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::voice_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window.state::<Storage>().voice_matching_preferences()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_voice_matching_preferences", message)
    })
}
#[tauri::command]
fn save_voice_matching_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::voice_preferences::VoiceMatchingPreferences,
) -> Result<storage::voice_preferences::Snapshot, mon_contracts::AppError> {
    let result: Result<storage::voice_preferences::Snapshot, String> = (|| {
        minutes_app(&window)?;
        window
            .state::<Storage>()
            .save_voice_matching_preferences(expected_lock_version, payload)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("save_voice_matching_preferences", message)
    })
}
#[tauri::command]
async fn consolidate_transcript(
    window: WebviewWindow,
    request: storage::transcript_consolidation::Request,
) -> Result<storage::transcript_consolidation::Reply, mon_contracts::AppError> {
    let result: Result<storage::transcript_consolidation::Reply, String> = async move {
        let app = minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().consolidate_transcript(&request)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result
        .map_err(|message| mon_contracts::AppError::from_command("consolidate_transcript", message))
}

#[tauri::command]
async fn get_transcript_draft(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<Option<TranscriptDraft>, mon_contracts::AppError> {
    let result: Result<Option<TranscriptDraft>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read drafts".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().load_transcript_draft(&meeting_id)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_transcript_draft", message))
}

#[tauri::command]
fn begin_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    meeting_id: String,
    analysis_id: String,
) -> Result<TranscriptDraft, mon_contracts::AppError> {
    let result: Result<TranscriptDraft, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can edit transcripts".into());
        }
        storage.create_transcript_draft(&meeting_id, &analysis_id)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("begin_transcript_draft", message))
}

#[tauri::command]
fn save_transcript_draft(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    draft_id: String,
    expected_lock_version: i64,
    payload: TranscriptDraftPayload,
) -> Result<TranscriptDraft, mon_contracts::AppError> {
    let result: Result<TranscriptDraft, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can edit transcripts".into());
        }
        storage.save_transcript_draft(&draft_id, expected_lock_version, payload)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("save_transcript_draft", message))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    if let Some(code) = local_engine::guard_entry() {
        std::process::exit(code);
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(code) = residency::supervisor_entry() {
            std::process::exit(code);
        }
    }
    #[cfg(windows)]
    {
        if let Some(code) = startup_instance::supervisor_entry() {
            std::process::exit(code);
        }
    }
    let context = tauri::generate_context!();
    #[cfg(target_os = "windows")]
    let context = {
        let mut context = context;
        window_chrome::configure(context.config_mut());
        context
    };
    let builder = tauri::Builder::default();
    #[cfg(any(windows, target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        startup_instance::activate(app);
    }));
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    builder
        .manage(startup_attention::Startup::default())
        .manage(startup_reset::Reset::default())
        .manage(startup_notices::Notices::default())
        .manage(startup_instance::Instance::default())
        .manage(residency::Residency::default())
        .manage(recording_recovery::Recovery::default())
        .manage(SessionStore::default())
        .manage(RecorderController::default())
        .manage(recorder_panel::PanelVisibility::default())
        .manage(input_status::archived::ArchivedInputs::default())
        .manage(capture_test::CaptureTests::default())
        .manage(StopDecisionStore::default())
        .manage(recorder_operation_notices::Notices::default())
        .manage(ExitGuard::default())
        .manage(PlaybackStore::default())
        .manage(PlaybackPreparations::default())
        .manage(AnalysisController::default())
        .manage(ImportController::default())
        .manage(DataController::default())
        .manage(model_inspection::Inspector::default())
        .manage(exports::ExportController::default())
        .setup(|app| {
            // The single-instance plugin has already rejected secondary launches.
            // On Windows the restart relay must precede the kill-on-close job.
            #[cfg(windows)]
            startup_instance::prepare_supervisor();
            #[cfg(target_os = "macos")]
            residency::prepare_supervisor();
            let logs = internal_logs::initialize(
                app.path()
                    .app_log_dir()
                    .map(|root| root.join("internal"))
                    .map_err(|e| e.to_string()),
            );
            app.manage(logs);
            residency::initialize(app.handle());
            internal_logs::phase("application", None, None, "starting", None);
            #[cfg(target_os = "windows")]
            if let Err(error) = local_engine::windows::initialize() {
                app.state::<startup_attention::Startup>().block(
                    startup_attention::Failure::new(
                        startup_attention::Stage::Directories,
                        format!("Windowsの処理終了管理を準備できません: {error}"),
                        None,
                        None,
                    ),
                )?;
                startup_attention::show(app.handle());
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            macos_menu::install(app)?;
            #[cfg(target_os = "macos")]
            rounded_windows::configure(app.handle())?;
            #[cfg(windows)]
            if let Err(error) = windows_appearance::configure(app.handle()) {
                internal_logs::report_error("windows_appearance", &error);
            }
            let root = match app.path().app_data_dir() {
                Ok(root) => root,
                Err(error) => {
                    app.state::<startup_attention::Startup>().block(
                        startup_attention::Failure::new(
                            startup_attention::Stage::Directories,
                            error.to_string(),
                            None,
                            None,
                        ),
                    )?;
                    startup_attention::show(app.handle());
                    return Ok(());
                }
            };
            #[cfg(any(target_os = "macos", windows))]
            {
                let admission = app
                    .path()
                    .app_config_dir()
                    .map_err(|error| {
                        startup_attention::Failure::new(
                            startup_attention::Stage::Directories,
                            error.to_string(),
                            None,
                            None,
                        )
                    })
                    .and_then(|config| startup_attention::admit_storage(&config, &root));
                match admission {
                    Ok((storage, selection)) => {
                        let recovery = match selection.startup_recovery_view(&storage) {
                            Ok(recovery) => recovery,
                            Err(error) => {
                                let selected_root = selection.root().to_owned();
                                drop(storage);
                                drop(selection);
                                app.state::<startup_attention::Startup>().block(
                                    startup_attention::Failure::new(
                                        startup_attention::Stage::StorageAdmission,
                                        error,
                                        Some(&selected_root),
                                        app.path().app_config_dir().ok().as_deref(),
                                    ),
                                )?;
                                startup_attention::show(app.handle());
                                return Ok(());
                            }
                        };
                        app.state::<startup_attention::Startup>()
                            .set_recovery(recovery)?;
                        app.manage(storage);
                        app.manage(selection);
                    }
                    Err(failure) => {
                        // No Storage registration, import/capture initialization,
                        // recording recovery or analysis worker after refusal.
                        app.state::<startup_attention::Startup>().block(failure)?;
                        startup_attention::show(app.handle());
                        return Ok(());
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            match Storage::open(root.clone()) {
                Ok(storage) => {
                    app.manage(storage);
                }
                Err(error) => {
                    app.state::<startup_attention::Startup>().block(
                        startup_attention::Failure::new(
                            startup_attention::Stage::StorageAdmission,
                            error.to_string(),
                            Some(&root),
                            None,
                        ),
                    )?;
                    startup_attention::show(app.handle());
                    return Ok(());
                }
            }
            let prepared = ImportController::recover(app.handle()).and_then(|_| {
                if !app.state::<Storage>().data_write_blocked() {
                    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
                    capture_test::CaptureTests::initialize(app.handle(), &root)?;
                }
                Ok(())
            });
            if let Err(error) = prepared {
                app.state::<startup_attention::Startup>().block(
                    startup_attention::Failure::new(
                        startup_attention::Stage::StorageAdmission,
                        error,
                        Some(&root),
                        None,
                    ),
                )?;
                startup_attention::show(app.handle());
                return Ok(());
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let migrated = (|| -> Result<(), String> {
                    let storage = handle.state::<Storage>();
                    #[cfg(any(unix, windows))]
                    if !storage.data_write_blocked() {
                        data_controller::finish_interrupted_deletions(&handle)?;
                    }
                    if storage.data_write_blocked() {
                        // Recovery controls must remain reachable even when an
                        // audio migration is pending. Storage fences still deny
                        // normal writes and AnalysisController cannot run. The
                        // recovery receipt requires a clean restart before any
                        // deferred startup work, including migration, resumes.
                        return handle.state::<startup_attention::Startup>().ready();
                    }
                    let errors = std::sync::Mutex::new(Vec::new());
                    storage.recover_recordings_observed(&|_| {}, &|item| {
                        if item.phase == "failed" {
                            if let Some(error) = item.error {
                                errors.lock().unwrap().push(error);
                            }
                        }
                    })?;
                    let errors = errors
                        .into_inner()
                        .map_err(|_| "録音復旧結果を取得できません")?;
                    if !errors.is_empty() {
                        return Err(errors.join("\n"));
                    }
                    storage.finish_pending_meeting_audio()?;
                    storage.migrate_single_wav(&|done, total, meeting| {
                        let _ = handle
                            .state::<startup_attention::Startup>()
                            .migration_progress(done, total, meeting);
                    })?;
                    storage.upgrade_completed_audio_cleanup(&|done, total, meeting| {
                        let _ = handle
                            .state::<startup_attention::Startup>()
                            .migration_progress(done, total, meeting);
                    })?;
                    handle.state::<startup_attention::Startup>().ready()?;
                    if let Err(error) = storage.preserve_voice_audio(None, &|| false) {
                        internal_logs::report_error("voice_audio", &error);
                    }
                    let _ = handle.emit("voice-identity-changed", ());
                    internal_logs::phase("application", None, None, "ready", None);
                    Ok(())
                })();
                if let Err(error) = migrated {
                    let _ = handle.state::<startup_attention::Startup>().block(
                        startup_attention::Failure::new(
                            startup_attention::Stage::AudioMigration,
                            error,
                            Some(&root),
                            handle.path().app_config_dir().ok().as_deref(),
                        ),
                    );
                    startup_attention::show(&handle);
                    return;
                }
                if let Err(error) = AnalysisController::kick(&handle) {
                    internal_logs::report_error("analysis", &error);
                }
            });
            Ok(())
        })
        .invoke_handler(|invoke| {
            let webview = invoke.message.webview();
            if !command_roles::permits(invoke.message.command(), webview.label()) {
                invoke
                    .resolver
                    .reject("この画面では許可されていない操作です");
                return true;
            }
            if let Err(error) = webview
                .state::<startup_attention::Startup>()
                .admit_command(webview.label(), invoke.message.command())
            {
                invoke.resolver.reject(error);
                return true;
            }
            if webview.state::<startup_reset::Reset>().busy()
                && !matches!(
                    invoke.message.command(),
                    "get_window_environment"
                        | "get_startup_report"
                        | "get_startup_reset_state"
                        | "start_startup_reset"
                )
            {
                invoke
                    .resolver
                    .reject("データを初期化しています。完了をお待ちください");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool =
                include!("command_handler.rs");
            handler(invoke)
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            windows_appearance::event(window, event);
            if window.state::<startup_reset::Reset>().busy() {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                }
                return;
            }
            if window.state::<startup_attention::Startup>().blocked() {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if cfg!(any(windows, target_os = "macos")) {
                        if window.label() == "main" {
                            let _ = window
                                .app_handle()
                                .get_webview_window("main")
                                .map(residency::hide_main_to_background);
                        }
                    } else {
                        window.app_handle().exit(0);
                    }
                }
                return;
            }
            if window.label() == recorder_info::LABEL {
                // Native window/WebView focus transfers can emit a transient blur
                // during opening. Only explicit close or recorder lifecycle hides help.
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = recorder_info::hide(window.app_handle(), true);
                    }
                    _ => {}
                }
                return;
            }
            if window.label() == "main" {
                if let WindowEvent::Focused(focused) = event {
                    recorder_panel::main_focus_changed(window.app_handle(), *focused);
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == recording_confirmation_window::LABEL {
                    api.prevent_close();
                    recording_confirmation_window::close_requested(window.app_handle());
                } else if window.label() == "recorder" {
                    api.prevent_close();
                    let _ = recording_confirmation_window::hide_recorder(window.app_handle());
                } else if window.label() == "main" {
                    api.prevent_close();
                    if window.state::<StopDecisionStore>().snapshot().visible() {
                        recording_confirmation_window::focus(window.app_handle());
                    } else {
                        let _ =
                            window.emit("main-close-requested", serde_json::json!({"quit": false}));
                    }
                }
            }
        })
        .build(context)
        .expect("failed to start mon desktop")
        .run(|app, event| {
            if let RunEvent::Ready = &event {
                startup_instance::ready(app);
            }
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = &event {
                startup_instance::activate(app);
            }
            if let RunEvent::Exit = event {
                residency::stopping(app);
                internal_logs::phase("application", None, None, "stopped", None);
                if let Some(logs) = app.try_state::<internal_logs::LogService>() {
                    logs.flush();
                }
            }
            if let RunEvent::ExitRequested { api, .. } = event {
                if app.state::<startup_reset::Reset>().busy() {
                    api.prevent_exit();
                    return;
                }
                if app.state::<startup_attention::Startup>().blocked() {
                    return;
                }
                if !app.state::<ExitGuard>().0.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    if app.state::<StopDecisionStore>().snapshot().visible() {
                        recording_confirmation_window::focus(app);
                        return;
                    }
                    if let Some(main) = app.get_webview_window("main") {
                        let _ = residency::restore(app);
                        let _ =
                            main.emit("main-close-requested", serde_json::json!({"quit": true}));
                    }
                }
            }
        });
}
