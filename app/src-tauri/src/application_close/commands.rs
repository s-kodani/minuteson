use crate::analysis_worker::AnalysisController;
#[cfg(target_os = "macos")]
use crate::app_updates;
use crate::application_close;
#[cfg(any(target_os = "macos", windows))]
use crate::bootstrap;
use crate::capture_test;
use crate::data_controller::DataController;
use crate::exports;
use crate::import_controller::ImportController;
use crate::recorder::RecorderController;
use crate::recording_recovery;
use crate::session::SessionStore;
use crate::startup_instance;
use crate::startup_salvage;
use crate::storage::Storage;
use crate::ExitGuard;
use std::sync::atomic::Ordering;
use tauri::{Manager, WebviewWindow};

#[tauri::command]
pub(crate) async fn finish_main_close(
    window: WebviewWindow,
    quit: bool,
    update: Option<bool>,
    expected: Option<application_close::CloseExpectations>,
    salvage: Option<bool>,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async move {
        if cfg!(any(windows, target_os = "macos")) && !quit {
            return Err("画面の収納とアプリの終了は別の操作です".into());
        }
        if window.label() != "main" {
            return Err("Only the main window can finish closing".into());
        }
        let updating = update.unwrap_or(false);
        let salvaging = salvage.unwrap_or(false);
        if salvaging && (!quit || updating || !cfg!(any(windows, target_os = "macos"))) {
            return Err("この終了方法ではデータ復元に切り替えられません".into());
        }
        if salvaging && expected.is_none() {
            return Err("録音とデータ処理の状態を確認してから復元を開始してください".into());
        }
        #[cfg(any(target_os = "macos", windows))]
        if salvaging {
            window
                .state::<bootstrap::Bootstrap>()
                .require_ready_salvage(&window.state::<Storage>())?;
        }
        #[cfg(not(target_os = "macos"))]
        if updating {
            return Err("このOSはアプリ内更新の対象外です".into());
        }
        #[cfg(target_os = "macos")]
        if updating {
            if !quit {
                return Err("更新には再起動が必要です".into());
            }
            app_updates::macos::require_ready(window.app_handle())?;
            app_updates::macos::require_idle(window.app_handle())?;
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
        window
            .state::<recording_recovery::Recovery>()
            .require_exit_ready(
                expected
                    .as_ref()
                    .and_then(|target| target.recording_save.as_ref()),
            )?;
        // Keep the exit epilogue alive too: a lost close response must neither
        // release running work nor leave the open app stranded behind an exit gate.
        tauri::async_runtime::spawn(async move {
            let recovery = window.state::<startup_salvage::Salvage>();
            let _handoff = if salvaging {
                Some(recovery.begin_handoff()?)
            } else {
                None
            };
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                RecorderController::with_transition(&app, || {
                    app.state::<recording_recovery::Recovery>()
                        .require_exit_ready(
                            expected
                                .as_ref()
                                .and_then(|target| target.recording_save.as_ref()),
                        )?;
                    #[cfg(target_os = "macos")]
                    if updating {
                        app_updates::macos::require_idle(&app)?;
                    }
                    if salvaging && app.state::<SessionStore>().snapshot().is_active() {
                        return Err("録音の停止と保存を確認してから復元を開始してください".into());
                    }
                    app.state::<capture_test::CaptureTests>().seal_exit()?;
                    let admission = if updating || salvaging {
                        app.state::<AnalysisController>().begin_update_exit()
                    } else if expected.is_some() {
                        app.state::<AnalysisController>().seal_exit()
                    } else {
                        app.state::<AnalysisController>().begin_exit()
                    };
                    if admission.is_err() {
                        app.state::<capture_test::CaptureTests>().abort_exit()?;
                    }
                    admission
                })?;
                let result = if salvaging {
                    let target = expected
                        .as_ref()
                        .ok_or("データ処理の確認結果がありません")?;
                    DataController::seal_salvage_exit(&app, &target.data_token).and_then(|_| {
                        ImportController::seal_salvage_exit(&app, &target.import_token)
                    })
                } else if let Some(target) = &expected {
                    DataController::shutdown_checked(&app, &target.data_token).and_then(|_| {
                        ImportController::shutdown_checked(&app, &target.import_token)
                    })
                } else {
                    DataController::shutdown(&app).and_then(|_| ImportController::shutdown(&app))
                }
                .and_then(|_| {
                    if salvaging {
                        AnalysisController::shutdown_for_salvage(&app)
                    } else {
                        AnalysisController::shutdown(&app)
                    }
                })
                .and_then(|_| {
                    #[cfg(any(target_os = "macos", windows))]
                    if salvaging {
                        let b = app.state::<bootstrap::Bootstrap>();
                        let storage = app.state::<Storage>();
                        storage.keep_sqlite_controls_on_close(true)?;
                        let requested = b.request_ready_salvage(&storage).and_then(|_| {
                            if let Err(error) = startup_instance::prepare_restart() {
                                b.undo_salvage_request()?;
                                return Err(error);
                            }
                            Ok(())
                        });
                        if let Err(error) = requested {
                            storage.keep_sqlite_controls_on_close(false)?;
                            return Err(error);
                        }
                    }
                    Ok(())
                });
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
                #[cfg(target_os = "macos")]
                if result.is_ok() && updating {
                    if let Err(error) = app_updates::macos::install(&app) {
                        app.state::<AnalysisController>().abort_shutdown();
                        app.state::<capture_test::CaptureTests>().abort_exit()?;
                        ImportController::abort_shutdown(&app);
                        DataController::abort_shutdown(&app);
                        let _ = AnalysisController::kick(&app);
                        return Err(error);
                    }
                }
                result
            })
            .await
            .map_err(|e| e.to_string())??;
            window.state::<ExitGuard>().0.store(true, Ordering::SeqCst);
            if salvaging {
                startup_instance::restart(window.app_handle());
            } else if updating {
                window.app_handle().restart();
            } else {
                window.app_handle().exit(0);
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("finish_main_close", message))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplicationCloseActivity {
    analysis_busy: bool,
    model_preparation_busy: bool,
    recording_recovery_busy: bool,
}

#[tauri::command]
pub(crate) fn get_application_close_activity(
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
