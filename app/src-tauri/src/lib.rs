mod analysis_flow;
mod analysis_preparation;
mod analysis_worker;
mod app_updates;
mod application_close;
mod audio;
mod audio_clock;
mod audio_finalization;
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
mod job_history;
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
mod recorder_operation_notices;
mod recorder_panel;
mod recording_confirmation;
mod recording_confirmation_window;
mod recording_recovery;
#[cfg(any(unix, windows))]
mod recording_segments;
mod recording_windows;
mod recording_writes;
mod resampling;
mod residency;
mod restored_processing;
#[cfg(target_os = "macos")]
mod rounded_windows;
mod session;
mod speaker_completion;
mod speaker_features;
mod startup_attention;
mod startup_instance;
#[cfg(any(target_os = "macos", windows))]
mod startup_recovery_review;
mod startup_reset;
mod startup_salvage;
mod startup_window;
mod storage;
mod transcription;
mod voice;
mod voice_model_names;
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
            mon_core::hash::sha256_hex(vec![0x5a; 1024 * 1024]),
            "bf63d8a95fcc2e64619813aae35fdcbe871fdd9264caa3f365eb3aed0f679129"
        );
        assert_eq!(
            mon_core::hash::sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

use analysis_flow::commands::{
    continue_analysis_flow, get_analysis_flow, get_analysis_preparations, get_current_analysis,
    get_meeting_analysis_audio, get_ready_analysis_flows,
};
use analysis_worker::commands::{
    cancel_analysis_job, get_analysis_jobs, get_analysis_notification_jobs,
    get_analysis_notification_output, pause_analysis_job, resume_analysis_job,
};
use analysis_worker::AnalysisController;
use application_close::commands::{finish_main_close, get_application_close_activity};
use capture_inputs::commands::{get_capture_applications, get_capture_inputs};
use capture_test::commands::{
    cancel_capture_test, discard_capture_test, get_capture_test_levels, get_capture_test_snapshot,
    prepare_capture_test_playback, reconcile_capture_test_start, start_capture_test,
};
use data_controller::commands::data_app;
use data_controller::commands::{
    cancel_data_operation, dismiss_data_job, get_data_operation, get_data_operation_availability,
    get_meeting_deletion_availability, list_data_jobs, list_restored_meeting_ids,
    select_data_operation, start_data_operation,
};
#[cfg(any(unix, windows))]
use data_controller::commands::{
    get_audio_deletion_items, get_audio_deletion_paths, get_deletion_items,
    get_model_deletion_items, review_meeting_audio_deletion, review_meeting_deletion,
    review_model_body_deletion, review_pending_audio_deletion, review_pending_deletion,
    review_pending_model_deletion,
};
#[cfg(any(target_os = "macos", windows))]
use data_controller::commands::{get_storage_bootstrap, review_storage_move};
use data_controller::DataController;
use diarization::commands::get_diarization_preferences;
use exports::commands::{export_plain_minutes_markdown, get_plain_minutes_export_status};
use generation::commands::minutes_app;
use generation::commands::{
    get_meeting_body_for_review, get_minutes_markdown, get_minutes_preferences,
    get_minutes_prompt_preferences, get_minutes_source, save_minutes_markdown,
    save_minutes_prompt_preferences, start_markdown_minutes_generation,
};
use import_controller::commands::{
    cancel_audio_import, get_audio_import, select_audio_import, start_audio_import,
};
use import_controller::ImportController;
use input_status::commands::{
    acknowledge_archived_recording_input_notice, close_recording_input_notice,
    get_archived_recording_input_notice_rows, get_archived_recording_input_notices,
    receive_recording_input_notice, render_recording_input_notice,
};
use model_application::commands::{
    apply_model_profile_application, get_model_profiles, preview_model_profile_application,
};
use model_compatibility::commands::check_model_compatibility;
use model_inspection::commands::cancel_model_inspection;
use model_placement::commands::get_model_placements;
use playback::commands::{
    cancel_playback_preparation, close_playback, finish_playback_preparation,
    get_meeting_audio_summary, prepare_draft_utterance_playback,
    prepare_markdown_interval_playback, prepare_meeting_playback, prepare_utterance_playback,
    read_playback_chunk, register_playback_preparation,
};
use playback::{PlaybackPreparations, PlaybackStore};
use recorder::commands::{
    capture_available, get_recording_input_history_status, get_recording_input_status,
    get_recording_levels, get_recording_microphone_selection, get_recording_writes,
    select_recording_microphone, start_recording,
};
use recorder::RecorderController;
use recorder_panel::commands::{hide_recorder_panel, open_main_window};
use recording_confirmation::StopDecisionStore;
use recording_confirmation_window::{
    decide_recording_confirmation, get_recording_confirmation, request_recording_confirmation,
};
use session::commands::get_recording_snapshot;
use session::SessionStore;
use std::sync::atomic::{AtomicBool, Ordering};
use storage::commands::{
    dismiss_model_download_guidance, get_meeting, get_meeting_list_diagnostics,
    get_meeting_vocabulary, get_model_download_guidance, get_storage_write_state,
    get_ui_preferences, get_vocabulary_book, list_meetings, rename_meeting, save_ui_preferences,
    save_vocabulary_book,
};
use storage::Storage;
use tauri::{Emitter, Manager, RunEvent, WebviewWindow, WindowEvent};
use transcription::commands::{
    begin_transcript_draft, consolidate_transcript, get_asr_backends, get_asr_preferences,
    get_transcript_draft, get_transcription_regeneration_target, publish_regenerated_transcription,
    regenerate_transcription, save_transcript_draft,
};
use voice::commands::{
    decide_voice_match, get_voice_audio_storage_state, get_voice_context,
    get_voice_matching_preferences, get_voice_people, manage_voice_person,
    prepare_voice_sample_playback, retry_voice_audio_storage, retry_voice_job,
    save_voice_matching_preferences, start_voice_job,
};

#[derive(Default)]
struct ExitGuard(AtomicBool);

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
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .manage(startup_attention::Startup::default())
        .manage(startup_reset::Reset::default())
        .manage(startup_salvage::Salvage::default())
        .manage(startup_instance::Instance::default())
        .manage(residency::Residency::default())
        .manage(recording_recovery::Recovery::default())
        .manage(app_updates::Updates::default())
        .manage(SessionStore::default())
        .manage(recording_windows::RecordingWindows::default())
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
            startup_window::initialize(app.handle());
            residency::initialize(app.handle());
            internal_logs::phase("application", None, None, "starting", None);
            #[cfg(target_os = "windows")]
            let process_management_error = local_engine::windows::initialize()
                .err()
                .map(|error| format!("Windowsの外部処理の終了管理を準備できません: {error}"));
            #[cfg(target_os = "macos")]
            macos_menu::install(app)?;
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
            job_history::initialize(app.handle(), root.join("job-history"));
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
            let storage = app.state::<Storage>();
            #[cfg(target_os = "windows")]
            if let Some(message) = process_management_error {
                storage.startup_diagnostic(None, "external_processes", &message);
                internal_logs::report_error("application", &message);
            }
            if !storage.data_write_blocked() {
                if let Err(error) = capture_test::CaptureTests::initialize(app.handle(), &root) {
                    storage.startup_diagnostic(None, "capture_test", &error);
                    internal_logs::report_error("capture_test", &error);
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let storage = handle.state::<Storage>();
                if let Err(error) = job_history::clear_for_new_workspace(&storage) {
                    internal_logs::report_error("job_history", &error);
                }
                let recover = |category: &str, result: Result<(), String>| {
                    if let Err(error) = result {
                        storage.startup_diagnostic(None, category, &error);
                        internal_logs::report_error(category, &error);
                    }
                };
                #[cfg(any(unix, windows))]
                if !storage.common_write_hold() {
                    recover(
                        "automatic_cleanup",
                        data_controller::finish_interrupted_deletions(&handle),
                    );
                }
                if !storage.data_write_blocked() {
                    recover("recording_recovery", recording_recovery::run(&handle));
                    recover(
                        "import_audio_recovery",
                        storage.finish_pending_import_audio(),
                    );
                    recover("audio_import", ImportController::recover(&handle));
                }
                let startup = handle.state::<startup_attention::Startup>();
                if let Err(error) = startup
                    .set_common_write_hold(storage.common_write_hold())
                    .and_then(|()| startup.ready())
                {
                    internal_logs::report_error("application", &error);
                    return;
                }
                job_history::start_backfill(&handle);
                if !storage.data_write_blocked() {
                    app_updates::start(&handle);
                    if let Err(error) = storage.retry_voice_audio_cleanup() {
                        internal_logs::report_error("voice_audio", &error);
                    }
                    let _ = handle.emit("voice-identity-changed", ());
                    if let Err(error) = AnalysisController::kick(&handle) {
                        internal_logs::report_error("analysis", &error);
                    }
                }
                internal_logs::phase("application", None, None, "ready", None);
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
            if webview.state::<startup_salvage::Salvage>().engaged()
                && !startup_salvage::allowed_while_engaged(invoke.message.command())
            {
                invoke
                    .resolver
                    .reject("通常操作を停止しています。データ復元画面を確認してください");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool =
                include!("command_handler.rs");
            handler(invoke)
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            windows_appearance::event(window, event);
            if window.state::<startup_reset::Reset>().busy()
                || window.state::<startup_salvage::Salvage>().busy()
            {
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
                app_updates::stop(app);
                residency::stopping(app);
                if let Err(error) = job_history::flush() {
                    internal_logs::report_error("job_history", &error);
                }
                internal_logs::phase("application", None, None, "stopped", None);
                if let Some(logs) = app.try_state::<internal_logs::LogService>() {
                    logs.flush();
                }
            }
            if let RunEvent::ExitRequested { api, .. } = event {
                if app.state::<startup_reset::Reset>().busy()
                    || app.state::<startup_salvage::Salvage>().busy()
                {
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
