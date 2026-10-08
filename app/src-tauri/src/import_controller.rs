//! One main-window file import, with revisioned state and cancellable work.
use crate::analysis_worker::AnalysisController;
use crate::audio_import::{self, AudioInfo, ImportOptions, SelectedAudio};
use crate::local_engine::Fingerprint;
use crate::session::SessionStore;
use crate::storage::{imports::ImportSnapshot, Storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub phase: String,
    pub busy: bool,
    pub recording_blocked: bool,
    pub token: Option<String>,
    pub file_name: Option<String>,
    pub info: Option<AudioInfo>,
    pub source_bytes: Option<u64>,
    pub required_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub completed: u64,
    pub saved: Option<ImportSnapshot>,
    pub error_message: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: "idle".into(),
            busy: false,
            recording_blocked: false,
            token: None,
            file_name: None,
            info: None,
            source_bytes: None,
            required_bytes: None,
            available_bytes: None,
            completed: 0,
            saved: None,
            error_message: None,
        }
    }
}
struct Selection {
    audio: SelectedAudio,
    helper: Fingerprint,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StartRequest {
    pub token: String,
    pub title: String,
    pub options: ImportOptions,
}
#[derive(Default)]
struct State {
    snapshot: Snapshot,
    selected: Option<Selection>,
    request: Option<StartRequest>,
    cancel: Arc<AtomicBool>,
    closing: bool,
    last_progress: Option<Instant>,
}
#[derive(Default)]
pub(crate) struct ImportController {
    state: Mutex<State>,
    idle: Condvar,
}
impl ImportController {
    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "取り込み状態を取得できません")?
            .snapshot
            .clone())
    }
    fn begin(&self, phase: &str) -> Result<Snapshot, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "取り込みロックを取得できません")?;
        if state.snapshot.busy || state.snapshot.recording_blocked || state.closing {
            return Err("取り込み・録音開始・終了処理が進行中です".into());
        }
        let revision = state.snapshot.revision + 1;
        state.snapshot = Snapshot {
            revision,
            phase: phase.into(),
            busy: true,
            token: Some(Uuid::new_v4().to_string()),
            ..Snapshot::default()
        };
        state.selected = None;
        state.request = None;
        state.last_progress = None;
        state.cancel = Arc::new(AtomicBool::new(false));
        Ok(state.snapshot.clone())
    }
    fn change(&self, change: impl FnOnce(&mut State)) -> Result<Snapshot, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "取り込みロックを取得できません")?;
        change(&mut state);
        state.snapshot.revision += 1;
        if !state.snapshot.busy {
            self.idle.notify_all();
        }
        Ok(state.snapshot.clone())
    }
    fn emit(app: &AppHandle, snapshot: Snapshot) {
        let _ = app.emit("audio-import-changed", snapshot);
    }
    fn update(app: &AppHandle, change: impl FnOnce(&mut State)) {
        if let Ok(snapshot) = app.state::<Self>().change(change) {
            Self::emit(app, snapshot);
        }
    }
    pub(crate) async fn select(
        window: &WebviewWindow,
        path: Option<PathBuf>,
    ) -> Result<Snapshot, String> {
        let app = window.app_handle().clone();
        if app.state::<SessionStore>().snapshot().is_active() {
            return Err("録音中は音声を取り込めません".into());
        }
        let initial = app.state::<Self>().begin(if path.is_some() {
            "inspecting"
        } else {
            "choosing"
        })?;
        Self::emit(&app, initial);
        let selected = match path {
            Some(path) => Ok(Some(path)),
            None => select_file(window).await,
        };
        match selected {
            Ok(Some(path)) => {
                Self::update(&app, |s| s.snapshot.phase = "inspecting".into());
                let worker = app.clone();
                std::thread::spawn(move || Self::inspect(&worker, path));
            }
            Ok(None) => Self::update(&app, |s| {
                s.snapshot.phase = "idle".into();
                s.snapshot.busy = false;
            }),
            Err(error) => Self::update(&app, |s| {
                s.snapshot.phase = "failed".into();
                s.snapshot.error_message = Some(error);
                s.snapshot.busy = false;
            }),
        }
        app.state::<Self>().snapshot()
    }
    fn inspect(app: &AppHandle, path: PathBuf) {
        let cancel = app.state::<Self>().state.lock().unwrap().cancel.clone();
        let cancelled = || cancel.load(Ordering::Acquire);
        let mut prepared = false;
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_, String> {
                AnalysisController::prepare_import(app, &cancelled)?;
                prepared = true;
                let helper = Fingerprint::capture(&crate::recorder::probe_path(app)?, &cancelled)?;
                let storage = app.state::<Storage>();
                let audio =
                    audio_import::inspect(&path, &helper, storage.import_scratch()?, &cancelled)?;
                let available = audio_import::available_bytes(storage.import_scratch()?)?;
                let required = audio.info.required_bytes(audio.proof.size)?;
                if cancelled() {
                    return Err("音声の確認を取り消しました".into());
                }
                Ok((Selection { audio, helper }, required, available))
            }))
            .unwrap_or_else(|_| Err("音声の確認処理が予期せず停止しました".into()));
        if prepared {
            AnalysisController::import_finished(app);
        }
        Self::update(app, |s| {
            s.snapshot.busy = false;
            match result {
                Ok((selection, required, available)) if !cancelled() => {
                    s.snapshot.phase = "ready".into();
                    s.snapshot.file_name = Some(selection.audio.file_name.clone());
                    s.snapshot.info = Some(selection.audio.info.clone());
                    s.snapshot.source_bytes = Some(selection.audio.proof.size);
                    s.snapshot.required_bytes = Some(required);
                    s.snapshot.available_bytes = Some(available);
                    s.snapshot.error_message = if available < required {
                        Some(
                            "保存先の空き容量が不足しています。容量を確保して再確認してください"
                                .into(),
                        )
                    } else {
                        None
                    };
                    s.selected = Some(selection);
                }
                _ if cancelled() => {
                    s.snapshot.phase = "cancelled".into();
                    s.snapshot.error_message =
                        Some("確認を取り消しました。元ファイルは変更していません".into());
                }
                Err(error) => {
                    s.snapshot.phase = "failed".into();
                    s.snapshot.error_message = Some(error);
                }
                _ => {}
            }
        });
        let _ = AnalysisController::kick(app);
    }
    pub(crate) fn start(app: &AppHandle, request: StartRequest) -> Result<Snapshot, String> {
        app.state::<Storage>().require_data_writable()?;
        request.options.validate()?;
        if app.state::<SessionStore>().snapshot().is_active() {
            return Err("録音中は取り込みを開始できません".into());
        }
        let controller = app.state::<Self>();
        let (selection, cancel, snapshot) = {
            let mut s = controller
                .state
                .lock()
                .map_err(|_| "取り込みロックを取得できません")?;
            if s.snapshot.token.as_deref() != Some(&request.token) {
                return Err("確認した音声が変更されました。選び直してください".into());
            }
            if s.request.as_ref() == Some(&request)
                && (s.snapshot.busy || s.snapshot.saved.is_some())
            {
                return Ok(s.snapshot.clone());
            }
            if s.snapshot.busy
                || s.snapshot.recording_blocked
                || s.closing
                || s.snapshot.phase != "ready"
            {
                return Err("この音声の取り込みは開始できません".into());
            }
            if request.title.trim().is_empty() || request.title.chars().count() > 200 {
                return Err("会議名を200文字以内で入力してください".into());
            }
            let selection = s.selected.take().ok_or("確認済みの音声がありません")?;
            s.request = Some(request.clone());
            s.cancel = Arc::new(AtomicBool::new(false));
            s.snapshot.busy = true;
            s.snapshot.phase = "waiting".into();
            s.snapshot.error_message = None;
            s.snapshot.revision += 1;
            (selection, s.cancel.clone(), s.snapshot.clone())
        };
        Self::emit(app, snapshot.clone());
        let app = app.clone();
        std::thread::spawn(move || {
            let cancelled = || cancel.load(Ordering::Acquire);
            let mut prepared = false;
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_, String> {
                    AnalysisController::prepare_import(&app, &cancelled)?;
                    prepared = true;
                    audio_import::import_configured(
                        &app.state::<Storage>(),
                        selection.audio,
                        &selection.helper,
                        &request.title,
                        &format!("import:{}", request.token),
                        &request.options,
                        &cancelled,
                        &|phase, n| {
                            Self::progress(&app, phase, n);
                        },
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(
                        "取り込み処理が予期せず停止しました。保存状況は再起動時に再検証します"
                            .into(),
                    )
                });
            // Publish terminal state before allowing waiting recording/exit to proceed.
            if prepared {
                AnalysisController::import_finished(&app);
            }
            Self::update(&app, |s| {
                s.snapshot.busy = false;
                match result {
                    Ok(saved) => {
                        s.snapshot.phase = "saved".into();
                        s.snapshot.saved = Some(saved);
                        s.snapshot.error_message = None;
                    }
                    Err(error) => {
                        s.snapshot.phase = if cancelled() { "cancelled" } else { "failed" }.into();
                        s.snapshot.error_message = Some(error);
                    }
                }
            });
            let _ = AnalysisController::kick(&app);
        });
        Ok(snapshot)
    }
    fn progress(app: &AppHandle, phase: &str, n: u64) {
        let c = app.state::<Self>();
        let snapshot = if let Ok(mut s) = c.state.lock() {
            if s.snapshot.phase == phase
                && s.last_progress
                    .is_some_and(|last| last.elapsed() < Duration::from_millis(100))
            {
                return;
            }
            s.last_progress = Some(Instant::now());
            if !s.cancel.load(Ordering::Acquire) {
                s.snapshot.phase = phase.into();
            }
            s.snapshot.completed = n;
            s.snapshot.revision += 1;
            s.snapshot.clone()
        } else {
            return;
        };
        Self::emit(app, snapshot);
    }
    pub(crate) fn cancel(&self, token: &str) -> Result<Snapshot, String> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| "取り込みロックを取得できません")?;
        if s.snapshot.token.as_deref() != Some(token) {
            return Err("取消対象が変更されました".into());
        }
        s.cancel.store(true, Ordering::Release);
        s.selected = None;
        if s.snapshot.busy {
            s.snapshot.phase = "cancelling".into();
        } else if s.snapshot.saved.is_none() {
            s.snapshot.phase = "cancelled".into();
        }
        s.snapshot.revision += 1;
        if !s.snapshot.busy {
            self.idle.notify_all();
        }
        Ok(s.snapshot.clone())
    }
    pub(crate) fn cancel_from_app(app: &AppHandle, token: &str) -> Result<Snapshot, String> {
        let snapshot = app.state::<Self>().cancel(token)?;
        Self::emit(app, snapshot.clone());
        Ok(snapshot)
    }
    fn wait_idle(&self, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        let mut s = self
            .state
            .lock()
            .map_err(|_| "取り込み停止を確認できません")?;
        while s.snapshot.busy {
            let remaining = timeout
                .checked_sub(started.elapsed())
                .ok_or("取り込みの安全な停止を待っています。完了後に再試行してください")?;
            s = self
                .idle
                .wait_timeout(s, remaining)
                .map_err(|_| "取り込み停止を待機できません")?
                .0;
        }
        Ok(())
    }
    pub(crate) fn prepare_recording(app: &AppHandle) -> Result<(), String> {
        Self::prepare_recording_inner(app, None)
    }
    pub(crate) fn prepare_recording_checked(
        app: &AppHandle,
        expected: &crate::capture_test::ImportExpectation,
    ) -> Result<(), String> {
        Self::prepare_recording_inner(app, Some(expected))
    }
    fn prepare_recording_inner(
        app: &AppHandle,
        expected: Option<&crate::capture_test::ImportExpectation>,
    ) -> Result<(), String> {
        let c = app.state::<Self>();
        let snapshot = {
            let mut s = c
                .state
                .lock()
                .map_err(|_| "取り込みロックを取得できません")?;
            if s.snapshot.recording_blocked || s.closing {
                return Err("録音開始または終了処理が進行中です".into());
            }
            if let Some(expected) = expected {
                expected.require(&s.snapshot)?;
            }
            s.snapshot.recording_blocked = true;
            s.cancel.store(true, Ordering::Release);
            if s.snapshot.busy {
                s.snapshot.phase = "cancelling".into();
            }
            s.snapshot.revision += 1;
            s.snapshot.clone()
        };
        Self::emit(app, snapshot);
        if let Err(error) = c.wait_idle(Duration::from_secs(15)) {
            Self::recording_finished(app);
            return Err(error);
        }
        Ok(())
    }
    pub(crate) fn recording_finished(app: &AppHandle) {
        Self::update(app, |s| s.snapshot.recording_blocked = false);
    }
    pub(crate) fn shutdown(app: &AppHandle) -> Result<(), String> {
        Self::shutdown_inner(app, None)
    }
    pub(crate) fn shutdown_checked(
        app: &AppHandle,
        expected: &Option<String>,
    ) -> Result<(), String> {
        Self::shutdown_inner(app, Some(expected))
    }
    fn shutdown_inner(app: &AppHandle, expected: Option<&Option<String>>) -> Result<(), String> {
        let c = app.state::<Self>();
        let snapshot = c.begin_shutdown(expected)?;
        Self::emit(app, snapshot);
        if let Err(error) = c.wait_idle(Duration::from_secs(15)) {
            Self::update(app, |s| s.closing = false);
            return Err(error);
        }
        Ok(())
    }
    fn begin_shutdown(&self, expected: Option<&Option<String>>) -> Result<Snapshot, String> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| "取り込み状態を取得できません")?;
        if let Some(expected) = expected {
            crate::application_close::require_target(expected, &s.snapshot.token)?;
        }
        s.closing = true;
        s.cancel.store(true, Ordering::Release);
        s.selected = None;
        if s.snapshot.busy {
            s.snapshot.phase = "cancelling".into();
        }
        s.snapshot.revision += 1;
        Ok(s.snapshot.clone())
    }
    pub(crate) fn abort_shutdown(app: &AppHandle) {
        Self::update(app, |s| {
            s.closing = false;
            if !s.snapshot.busy && s.snapshot.saved.is_none() {
                s.snapshot.phase = "cancelled".into();
            }
        });
    }
    pub(crate) fn recover(app: &AppHandle) -> Result<(), String> {
        if app.state::<Storage>().data_write_blocked() {
            return Ok(());
        }
        let snapshot = app.state::<Self>().begin("recovering")?;
        Self::emit(app, snapshot);
        let app = app.clone();
        std::thread::spawn(move || {
            let cancel = app.state::<Self>().state.lock().unwrap().cancel.clone();
            let cancelled = || cancel.load(Ordering::Acquire);
            let mut prepared = false;
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<_, String> {
                    AnalysisController::prepare_import(&app, &cancelled)?;
                    prepared = true;
                    app.state::<Storage>()
                        .recover_audio_imports_cancellable(&cancelled)
                }))
                .unwrap_or_else(|_| {
                    Err("取り込みの回復検証が停止しました。次回起動時に再検証します".into())
                });
            if prepared {
                AnalysisController::import_finished(&app);
            }
            if let Ok(imports) = &result {
                if !imports.is_empty() {
                    let _ = app.emit(
                        "recordings-recovered",
                        imports
                            .iter()
                            .map(|i| i.meeting_id.clone())
                            .collect::<Vec<_>>(),
                    );
                }
            }
            Self::update(&app, |s| {
                s.snapshot.busy = false;
                s.snapshot.phase = if cancelled() {
                    "cancelled"
                } else if result.is_ok() {
                    "idle"
                } else {
                    "failed"
                }
                .into();
                s.snapshot.error_message = result.err();
            });
            let _ = AnalysisController::kick(&app);
        });
        Ok(())
    }
}

async fn select_file(window: &WebviewWindow) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
                use objc2_foundation::{NSArray, NSString};
                let result = (|| -> Result<_, String> {
                    let mtm =
                        MainThreadMarker::new().ok_or("音声選択にはメインスレッドが必要です")?;
                    let panel = NSOpenPanel::openPanel(mtm);
                    panel.setTitle(Some(&NSString::from_str("音声を取り込む")));
                    panel.setCanChooseFiles(true);
                    panel.setCanChooseDirectories(false);
                    panel.setAllowsMultipleSelection(false);
                    #[allow(deprecated)]
                    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                        NSString::from_str("wav"),
                        NSString::from_str("mp3"),
                        NSString::from_str("m4a"),
                        NSString::from_str("flac"),
                    ])));
                    match panel.runModal() {
                        response if response == NSModalResponseCancel => Ok(None),
                        response if response == NSModalResponseOK => {
                            let url = panel.URL().ok_or("選択した音声がありません")?;
                            if !url.isFileURL() {
                                return Err("ローカル音声ファイルを選んでください".into());
                            }
                            Ok(Some(PathBuf::from(
                                url.path().ok_or("音声のパスがありません")?.to_string(),
                            )))
                        }
                        _ => Err("音声選択を完了できませんでした".into()),
                    }
                })();
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receive
                .recv()
                .map_err(|_| "音声選択の応答がありません".to_owned())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("このOSの音声選択は保留中です".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked_close_rejects_changed_import_without_cancelling_or_discarding() {
        let controller = super::ImportController::default();
        let before = controller.begin("inspecting").unwrap();
        assert!(controller.begin_shutdown(Some(&None)).is_err());
        assert!(controller
            .begin_shutdown(Some(&Some("foreign".into())))
            .is_err());
        let state = controller.state.lock().unwrap();
        assert!(!state.closing);
        assert!(!state.cancel.load(super::Ordering::Acquire));
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(&state.snapshot).unwrap()
        );
    }
    use super::*;

    #[test]
    fn selection_tokens_reject_stale_cancel_without_mutating_current_work() {
        let c = ImportController::default();
        let first = c.begin("inspecting").unwrap();
        assert!(c.begin("inspecting").is_err());
        assert!(c.cancel("other").is_err());
        assert_eq!(c.snapshot().unwrap().revision, first.revision);
        assert!(!c.state.lock().unwrap().cancel.load(Ordering::Acquire));
        let cancelling = c.cancel(first.token.as_deref().unwrap()).unwrap();
        assert!(cancelling.busy);
        assert_eq!(cancelling.phase, "cancelling");
        assert!(c.wait_idle(Duration::ZERO).is_err());
        c.change(|s| {
            s.snapshot.busy = false;
            s.snapshot.phase = "cancelled".into();
        })
        .unwrap();
        c.wait_idle(Duration::ZERO).unwrap();
        let second = c.begin("choosing").unwrap();
        assert_ne!(first.token, second.token);
        assert!(c.cancel(first.token.as_deref().unwrap()).is_err());
        assert_eq!(c.snapshot().unwrap().revision, second.revision);
    }

    #[test]
    fn recording_and_exit_gates_block_new_imports_and_terminal_state_wakes_waiters() {
        let c = Arc::new(ImportController::default());
        c.change(|s| s.snapshot.recording_blocked = true).unwrap();
        assert!(c.begin("inspecting").is_err());
        c.change(|s| {
            s.snapshot.recording_blocked = false;
            s.closing = true;
        })
        .unwrap();
        assert!(c.begin("inspecting").is_err());
        c.change(|s| s.closing = false).unwrap();
        c.begin("converting").unwrap();
        let waiting = c.clone();
        let waiter = std::thread::spawn(move || waiting.wait_idle(Duration::from_secs(2)));
        c.change(|s| {
            s.snapshot.busy = false;
            s.snapshot.phase = "saved".into();
        })
        .unwrap();
        waiter.join().unwrap().unwrap();
        assert_eq!(c.snapshot().unwrap().phase, "saved");
    }
}
