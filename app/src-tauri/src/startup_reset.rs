//! IPC boundary for startup reset. WebViews receive display paths and tokens,
//! never ownership of the native deletion list or arbitrary reset targets.
#[cfg(any(target_os = "macos", windows))]
use crate::bootstrap::{self, reset};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{Manager, WebviewWindow};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Availability {
    pub available: bool,
    pub reason: Option<String>,
}
impl Availability {
    fn unavailable(reason: String) -> Self {
        Self {
            available: false,
            reason: Some(reason.replace('\0', "\\0").chars().take(4096).collect()),
        }
    }
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    operation_id: Option<String>,
    busy: bool,
    completed: bool,
    message: String,
    error: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    availability: Availability,
    operation_id: Option<String>,
    meeting_root: Option<PathBuf>,
    archive_path: Option<PathBuf>,
}
#[cfg(any(target_os = "macos", windows))]
struct Prepared {
    id: String,
    selection: reset::Selection,
    destination: Option<reset::Archive>,
}
#[derive(Default)]
struct Inner {
    #[cfg(any(target_os = "macos", windows))]
    prepared: Option<Prepared>,
    snapshot: Snapshot,
    choosing: bool,
}
#[derive(Default)]
pub(crate) struct Reset {
    inner: Mutex<Inner>,
    busy: AtomicBool,
}
impl Reset {
    pub(crate) fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    #[cfg(any(target_os = "macos", windows))]
    fn accept(
        &self,
        operation_id: String,
        archive: bool,
    ) -> Result<(Option<Prepared>, Snapshot), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "初期化状態を取得できません")?;
        if inner.snapshot.operation_id.as_deref() == Some(&operation_id) {
            return Ok((None, inner.snapshot.clone()));
        }
        if self.busy() || inner.choosing {
            return Err("初期化の操作中です".into());
        }
        let prepared = inner
            .prepared
            .as_ref()
            .filter(|p| p.id == operation_id)
            .ok_or("初期化の確認情報が無効です")?;
        if archive && prepared.destination.is_none() {
            return Err("退避先を選択してください".into());
        }
        let prepared = inner
            .prepared
            .take()
            .ok_or("初期化の確認情報がありません")?;
        self.busy.store(true, Ordering::Release);
        inner.snapshot = Snapshot {
            operation_id: Some(operation_id),
            busy: true,
            completed: false,
            message: "初期化を受け付けました".into(),
            error: None,
        };
        Ok((Some(prepared), inner.snapshot.clone()))
    }
    fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "初期化状態を取得できません")?
            .snapshot
            .clone())
    }
    fn progress(&self, message: &str) {
        if let Ok(mut state) = self.inner.lock() {
            state.snapshot.message = message.into();
        }
    }
}
fn require(window: &WebviewWindow) -> Result<(), String> {
    window
        .state::<crate::startup_attention::Startup>()
        .admit_command(window.label(), "get_startup_reset_state")
}
pub(crate) fn check_idle(app: &tauri::AppHandle) -> Result<(), String> {
    if app.state::<Reset>().busy() {
        Err("データを初期化しています。完了まで終了・再起動できません".into())
    } else {
        Ok(())
    }
}
#[cfg(any(target_os = "macos", windows))]
fn paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    Ok((
        app.path().app_config_dir().map_err(|e| e.to_string())?,
        app.path().app_data_dir().map_err(|e| e.to_string())?,
    ))
}
#[cfg(any(target_os = "macos", windows))]
fn selection(app: &tauri::AppHandle) -> Result<reset::Selection, String> {
    let (config, data) = paths(app)?;
    if let Some(b) = app.try_state::<bootstrap::Bootstrap>() {
        reset::inspect(&b, &data)
    } else {
        reset::inspect_paths(&config, &data)
    }
}
pub(crate) fn availability(app: &tauri::AppHandle) -> Availability {
    #[cfg(any(target_os = "macos", windows))]
    let result = selection(app).map(|_| ());
    #[cfg(not(any(target_os = "macos", windows)))]
    let result: Result<(), String> = Err("このOSでは初期化を利用できません".into());
    match result {
        Ok(()) => Availability {
            available: true,
            reason: None,
        },
        Err(reason) => Availability::unavailable(reason),
    }
}
#[tauri::command]
pub(crate) async fn get_startup_reset_preview(
    window: WebviewWindow,
) -> Result<Preview, mon_contracts::AppError> {
    let result: Result<Preview, String> = async move {
        require(&window)?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<Reset>();
            let mut inner = state
                .inner
                .lock()
                .map_err(|_| "初期化状態を取得できません")?;
            if state.busy() || inner.choosing {
                return Err("初期化の操作中です".into());
            }
            if inner.snapshot.operation_id.is_some() {
                return Err(
                    "初期化は受付済みです。状態を確認し、アプリを再起動してください".into(),
                );
            }
            #[cfg(any(target_os = "macos", windows))]
            {
                match selection(&app) {
                    Ok(selection) => {
                        let id = uuid::Uuid::new_v4().to_string();
                        let root = selection.root().into();
                        inner.prepared = Some(Prepared {
                            id: id.clone(),
                            selection,
                            destination: None,
                        });
                        Ok(Preview {
                            availability: Availability {
                                available: true,
                                reason: None,
                            },
                            operation_id: Some(id),
                            meeting_root: Some(root),
                            archive_path: None,
                        })
                    }
                    Err(reason) => Ok(Preview {
                        availability: Availability::unavailable(reason),
                        operation_id: None,
                        meeting_root: None,
                        archive_path: None,
                    }),
                }
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                Ok(Preview {
                    availability: availability(&app),
                    operation_id: None,
                    meeting_root: None,
                    archive_path: None,
                })
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_startup_reset_preview", message)
    })
}
#[tauri::command]
pub(crate) async fn choose_startup_reset_backup_directory(
    window: WebviewWindow,
    operation_id: String,
) -> Result<Option<PathBuf>, mon_contracts::AppError> {
    let result: Result<Option<PathBuf>, String> = async move {
        require(&window)?;
        let app = window.app_handle().clone();
        #[cfg(any(target_os = "macos", windows))]
        {
            {
                let state = app.state::<Reset>();
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| "初期化状態を取得できません")?;
                if state.busy()
                    || inner.choosing
                    || inner.prepared.as_ref().is_none_or(|p| p.id != operation_id)
                {
                    return Err("初期化の確認情報が無効か、操作中です".into());
                }
                inner.choosing = true;
            }
            let chosen = dialog(&window).await;
            let state = app.state::<Reset>();
            let mut inner = state
                .inner
                .lock()
                .map_err(|_| "初期化状態を取得できません")?;
            inner.choosing = false;
            let prepared = inner
                .prepared
                .as_mut()
                .filter(|p| p.id == operation_id)
                .ok_or("初期化の確認情報が変わっています")?;
            if let Some(parent) = chosen? {
                let parent = parent.canonicalize().map_err(|e| e.to_string())?;
                let destination =
                    reset::Archive::select(&prepared.selection, &parent, &prepared.id)?;
                let child = destination.path().to_owned();
                // Native canonical parent is retained for the immediate recheck.
                prepared.destination = Some(destination);
                Ok(Some(child))
            } else {
                Ok(None)
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = (app, operation_id);
            Err("このOSでは初期化を利用できません".into())
        }
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("choose_startup_reset_backup_directory", message)
    })
}
#[tauri::command]
pub(crate) fn get_startup_reset_state(
    window: WebviewWindow,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> = (|| {
        require(&window)?;
        window.state::<Reset>().snapshot()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_startup_reset_state", message)
    })
}
#[tauri::command]
#[allow(deprecated)] // Startup is blocked: no storage-backed WebView command can hold a State borrow.
pub(crate) fn start_startup_reset(
    window: WebviewWindow,
    operation_id: String,
    archive: bool,
) -> Result<Snapshot, mon_contracts::AppError> {
    let operation = operation_id.clone();
    let result: Result<Snapshot, String> = (|| {
        require(&window)?;
        #[cfg(any(target_os = "macos", windows))]
        {
            let app = window.app_handle().clone();
            let state = app.state::<Reset>();
            let (prepared, response) = state.accept(operation_id, archive)?;
            let Some(prepared) = prepared else {
                return Ok(response);
            };
            tauri::async_runtime::spawn_blocking(move || {
                let result = (|| {
                    let (config, data) = paths(&app)?;
                    app.state::<Reset>()
                        .progress("起動時のデータ処理を停止し、保存先を解放しています");
                    app.state::<crate::import_controller::ImportController>()
                        .drain_for_startup_reset()?;
                    let analysis = app.state::<crate::analysis_worker::AnalysisController>();
                    let _maintenance = analysis.begin_startup_reset_maintenance(
                        app.state::<crate::session::SessionStore>()
                            .snapshot()
                            .is_active(),
                        app.state::<crate::import_controller::ImportController>()
                            .snapshot()?
                            .recording_blocked,
                    )?;
                    app.state::<crate::capture_test::CaptureTests>()
                        .release_for_startup_reset()?;
                    // Background startup users are drained and normal IPC remains
                    // blocked, so no Storage/Bootstrap State borrow can outlive removal.
                    // Closing SQLite releases its WAL/registry handles before any copy.
                    drop(app.unmanage::<crate::storage::Storage>());
                    let b = match app.unmanage::<bootstrap::Bootstrap>() {
                        Some(b) => b,
                        None => reset::open(&config, &data)?,
                    };
                    if archive {
                        prepared
                            .destination
                            .as_ref()
                            .ok_or("退避先を選択してください")?
                            .verify()?;
                    }
                    reset::execute(
                        b,
                        &prepared.selection,
                        &prepared.id,
                        if archive {
                            prepared
                                .destination
                                .as_ref()
                                .map(|destination| destination.path())
                        } else {
                            None
                        },
                        &|m| app.state::<Reset>().progress(m),
                    )
                })();
                let state = app.state::<Reset>();
                if let Ok(mut inner) = state.inner.lock() {
                    inner.snapshot.busy = false;
                    inner.snapshot.completed = result.is_ok();
                    inner.snapshot.message = if result.is_ok() {
                        "初期化が完了しました。アプリを再起動しています".into()
                    } else {
                        "初期化を完了できませんでした".into()
                    };
                    inner.snapshot.error = result
                        .err()
                        .map(|error| error.replace('\0', "\\0").chars().take(16384).collect());
                }
                state.busy.store(false, Ordering::Release);
                if state.snapshot().is_ok_and(|s| s.completed) {
                    // Existing OS-specific restart always re-enters normal admission.
                    if let Some(main) = app.get_webview_window("main") {
                        if let Err(e) = crate::startup_instance::retry_startup(main) {
                            if let Ok(mut inner) = state.inner.lock() {
                                inner.snapshot.error = Some(e.to_string());
                                inner.snapshot.message =
                                    "初期化は完了しました。アプリを再起動してください".into();
                            }
                        }
                    }
                }
            });
            Ok(response)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = (operation_id, archive);
            Err("このOSでは初期化を利用できません".into())
        }
    })();
    result.map_err(|message| {
        let mut error = mon_contracts::AppError::from_command("start_startup_reset", message);
        error.operation_id = Some(operation);
        error
    })
}
#[cfg(windows)]
async fn dialog(window: &WebviewWindow) -> Result<Option<PathBuf>, String> {
    crate::platform_dialog::pick_directory(window, "旧データの退避先を選択").await
}
#[cfg(target_os = "macos")]
async fn dialog(window: &WebviewWindow) -> Result<Option<PathBuf>, String> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            use objc2::MainThreadMarker;
            use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
            use objc2_foundation::NSString;
            let result = (|| {
                let panel = NSOpenPanel::openPanel(
                    MainThreadMarker::new().ok_or("フォルダー選択にはメインスレッドが必要です")?,
                );
                panel.setTitle(Some(&NSString::from_str("旧データの退避先を選択")));
                panel.setCanChooseFiles(false);
                panel.setCanChooseDirectories(true);
                panel.setAllowsMultipleSelection(false);
                match panel.runModal() {
                    r if r == NSModalResponseCancel => Ok(None),
                    r if r == NSModalResponseOK => {
                        let url = panel.URL().ok_or("選択先がありません")?;
                        if !url.isFileURL() {
                            return Err("ローカルのフォルダーを選択してください".into());
                        }
                        Ok(Some(PathBuf::from(
                            url.path().ok_or("選択先のパスがありません")?.to_string(),
                        )))
                    }
                    _ => Err("フォルダー選択を完了できませんでした".into()),
                }
            })();
            let _ = send.send(result);
        })
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        receive
            .recv()
            .map_err(|_| "フォルダー選択の応答がありません".to_owned())?
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(all(test, any(target_os = "macos", windows)))]
mod tests {
    use super::*;
    #[test]
    fn duplicate_delivery_and_lost_reply_never_start_another_reset() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let data = base.join("data");
        std::fs::create_dir(&data).unwrap();
        #[cfg(not(windows))]
        std::fs::create_dir(data.join("data")).unwrap();
        let b = reset::open(&base.join("config"), &data).unwrap();
        let selection = reset::inspect(&b, &data).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let state = Reset::default();
        state.inner.lock().unwrap().prepared = Some(Prepared {
            id: id.clone(),
            selection,
            destination: None,
        });
        assert!(state.accept(id.clone(), true).is_err());
        assert!(!state.busy());
        state.inner.lock().unwrap().choosing = true;
        assert!(state.accept(id.clone(), false).is_err());
        state.inner.lock().unwrap().choosing = false;
        let (first, receipt) = state.accept(id.clone(), false).unwrap();
        assert!(first.is_some());
        assert!(receipt.busy);
        assert!(state.busy());
        // A lost response is recovered by status. Changing the archive bit in a
        // duplicate must not change the already accepted operation.
        assert_eq!(
            state.snapshot().unwrap().operation_id.as_deref(),
            Some(id.as_str())
        );
        assert!(state.accept(id.clone(), true).unwrap().0.is_none());
        assert!(state
            .accept(uuid::Uuid::new_v4().to_string(), false)
            .is_err());
        {
            let mut inner = state.inner.lock().unwrap();
            inner.snapshot.busy = false;
            inner.snapshot.completed = true;
        }
        state.busy.store(false, Ordering::Release);
        let (again, done) = state.accept(id, false).unwrap();
        assert!(again.is_none());
        assert!(done.completed);
    }
}
