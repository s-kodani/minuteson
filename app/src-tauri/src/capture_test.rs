//! One explicit, meeting-free capture test. Read-only reconciliation never
//! replays start; cancellation remains busy until the owned process exits.
mod files;
#[cfg(target_os = "macos")]
mod journal;
#[cfg(windows)]
#[path = "capture_test/journal_windows.rs"]
mod journal;
pub(crate) mod recovery;
#[cfg(test)]
mod tests;
use crate::analysis_worker::AnalysisController;
use crate::capture_scope::CaptureScope;
use crate::import_controller::ImportController;
use crate::playback::{PlaybackInfo, PlaybackPreparations, PlaybackStore};
use crate::recorder::RecorderController;
use crate::session::SessionStore;
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ImportExpectation {
    pub(crate) token: Option<String>,
    pub(crate) revision: u64,
    pub(crate) busy: bool,
}
impl ImportExpectation {
    pub(crate) fn require(
        &self,
        current: &crate::import_controller::Snapshot,
    ) -> Result<(), String> {
        if self.revision > 9_007_199_254_740_991
            || self
                .token
                .as_ref()
                .is_some_and(|t| !crate::input_levels::valid_owner(t))
            || current.recording_blocked
            || current.token != self.token
            || (!self.busy && (current.revision != self.revision || current.busy))
        {
            return Err(
                "確認した取り込み状態が変更されています。別の取り込みには取消を送っていません"
                    .into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub(crate) run_id: String,
    pub(crate) microphone_uid: String,
    pub(crate) capture_scope: CaptureScope,
    pub(crate) expected_import: ImportExpectation,
}
impl Request {
    fn validate(&self) -> Result<(), String> {
        if !crate::input_levels::valid_owner(&self.run_id)
            || !crate::storage::valid_microphone_uid(&self.microphone_uid)
        {
            return Err("録音前テストのIDまたはマイクが不正です".into());
        }
        self.capture_scope.validate()
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    revision: u64,
    run_id: String,
    phase: String,
    busy: bool,
    microphone_uid: String,
    microphone_name: Option<String>,
    capture_scope: CaptureScope,
    application_name: Option<String>,
    microphone: Option<files::TrackInfo>,
    system: Option<files::TrackInfo>,
    cleanup_pending: bool,
    error_message: Option<String>,
}
struct Run {
    request: Request,
    snapshot: Snapshot,
    cancelled: Arc<AtomicBool>,
    task_done: bool,
    directory: Option<Arc<files::TestDirectory>>,
    child: Option<Arc<Probe>>,
    ready: Option<files::Ready>,
    levels: crate::input_levels::LevelReader,
    playback_tokens: Vec<String>,
}
impl Snapshot {
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }
}
#[derive(Default)]
struct State {
    current: Option<Run>,
    used: HashSet<String>,
    closing: bool,
    ownership: recovery::Ownership,
}
#[derive(Default)]
pub(crate) struct CaptureTests(Mutex<State>);
struct Probe(Mutex<Child>);
impl Probe {
    #[cfg(windows)]
    fn failure_detail(&self) -> Option<String> {
        use std::{io::Read, os::windows::io::AsRawHandle};
        let mut child = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pipe = child.stderr.as_mut()?;
        let mut available = 0;
        // Only consume bytes already in the pipe. An inherited writer must
        // never make cancellation or error reporting wait indefinitely.
        if unsafe {
            windows_sys::Win32::System::Pipes::PeekNamedPipe(
                pipe.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
            || available == 0
        {
            return None;
        }
        let mut bytes = vec![
            0;
            (available as usize).min(
                mon_native::process::Purpose::CaptureTest
                    .policy()
                    .stderr_bytes as usize
            )
        ];
        let count = pipe.read(&mut bytes).ok()?;
        let text = String::from_utf8_lossy(&bytes[..count]);
        let detail = text.trim();
        (!detail.is_empty()).then(|| detail.to_owned())
    }
    fn exited(&self) -> Result<Option<std::process::ExitStatus>, String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_wait()
            .map_err(|e| e.to_string())
    }
    fn terminate(&self) -> Result<(), String> {
        let mut child = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            child.kill().map_err(|e| e.to_string())?;
        }
        child.wait().map(|_| ()).map_err(|e| e.to_string())
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}
fn command(
    binary: &std::path::Path,
    directory: &std::path::Path,
    request: &Request,
) -> Result<Command, String> {
    command_for(
        binary,
        directory,
        request,
        "mon-five-second-test",
        std::process::id(),
    )
}
fn command_for(
    binary: &std::path::Path,
    directory: &std::path::Path,
    request: &Request,
    scenario: &str,
    parent_pid: u32,
) -> Result<Command, String> {
    request.validate()?;
    let mut command = Command::new(binary);
    mon_native::process::configure(&mut command, mon_native::process::Purpose::CaptureTest);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
        .arg("--output")
        .arg(directory)
        .arg("--mode")
        .arg(&request.capture_scope.mode)
        .arg("--backend")
        .arg(crate::recorder::capture_backend())
        .arg("--scenario")
        .arg(scenario)
        .arg("--stop-on-file")
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .arg("--microphone-uid")
        .arg(&request.microphone_uid)
        .arg("--five-second-test")
        .arg(&request.run_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.stderr(Stdio::piped());
    if let Some(identity) = &request.capture_scope.application {
        command
            .arg("--pid")
            .arg(identity.pid.to_string())
            .arg("--application-identity")
            .arg(serde_json::to_string(identity).map_err(|e| e.to_string())?);
    }
    Ok(command)
}
#[cfg(any(target_os = "macos", windows))]
fn owned_command(
    binary: &std::path::Path,
    directory: &files::TestDirectory,
    request: &Request,
) -> Result<Command, String> {
    with_lease(command(binary, &directory.path, request)?, directory)
}
#[cfg(any(target_os = "macos", windows))]
fn with_lease(mut launch: Command, directory: &files::TestDirectory) -> Result<Command, String> {
    let descriptor = directory.inherit_lease(&mut launch)?;
    #[cfg(target_os = "macos")]
    launch
        .arg("--capture-test-lease-fd")
        .arg(descriptor.to_string());
    #[cfg(windows)]
    {
        let _ = descriptor;
        launch.arg("--capture-test-lease-stdin").arg("1");
    }
    Ok(launch)
}
impl CaptureTests {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, String> {
        self.0
            .lock()
            .map_err(|_| "録音前テストの状態を取得できません".into())
    }
    pub(crate) fn snapshot(&self, expected: Option<&str>) -> Result<Option<Snapshot>, String> {
        let state = self.lock()?;
        if expected.is_some_and(|id| {
            state
                .current
                .as_ref()
                .is_none_or(|r| r.request.run_id != id)
        }) {
            return Err("指定した録音前テストはありません。開始は再送していません".into());
        }
        Ok(state.current.as_ref().map(|r| r.snapshot.clone()))
    }
    pub(crate) fn require_empty(&self) -> Result<(), String> {
        let state = self.lock()?;
        state.ownership.require_clear()?;
        if state.closing {
            return Err("アプリの終了処理が進行中です".into());
        }
        if state.current.is_some() {
            return Err("5秒録音テストがまだ終了していません。テスト画面を閉じてから、もう一度操作してください。テスト音声は自動で削除されます。".into());
        }
        Ok(())
    }
    // Caller holds RecorderController's transition lock, the same lock used
    // by start. Retiring an absent ID prevents an already queued start from
    // appearing after the UI has confirmed absence and allowed exit/retry.
    pub(crate) fn reconcile_start(&self, id: &str) -> Result<Option<Snapshot>, String> {
        if !crate::input_levels::valid_owner(id) {
            return Err("録音前テストの開始IDが不正です".into());
        }
        let mut state = self.lock()?;
        if state.closing {
            return Err("アプリの終了処理が進行中です".into());
        }
        if let Some(run) = &state.current {
            if run.request.run_id != id {
                return Err("別の録音前テストが残っています".into());
            }
            return Ok(Some(run.snapshot.clone()));
        }
        if !state.used.contains(id) && state.used.len() >= 4096 {
            return Err("録音前テストの開始IDの上限に達しました".into());
        }
        state.used.insert(id.to_string());
        Ok(None)
    }
    pub(crate) fn seal_exit(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        if state.current.is_some() || state.closing || state.ownership.checking() {
            return Err("録音前テストの解放または終了処理が未完了です".into());
        }
        state.closing = true;
        Ok(())
    }
    pub(crate) fn require_exit_ready(&self) -> Result<(), String> {
        let state = self.lock()?;
        if state.current.is_some() || state.closing || state.ownership.checking() {
            return Err("テストまたは回復処理が進行中です。停止・確認後に終了してください".into());
        }
        // Attention means retain the journal and audio, not erase them to exit.
        // The next startup must still check the inherited helper's live lease.
        Ok(())
    }
    pub(crate) fn abort_exit(&self) -> Result<(), String> {
        self.lock()?.closing = false;
        Ok(())
    }
    fn reserve(&self, request: Request) -> Result<Snapshot, String> {
        request.validate()?;
        let mut state = self.lock()?;
        state.ownership.require_clear()?;
        if state.closing
            || state.current.is_some()
            || state.used.contains(&request.run_id)
            || state.used.len() >= 4096
        {
            return Err(
                "別のテストが残っているか、同じ開始IDは受付済みです。開始は再送していません".into(),
            );
        }
        let snapshot = Snapshot {
            schema_version: 1,
            revision: 1,
            run_id: request.run_id.clone(),
            phase: "preparing".into(),
            busy: true,
            microphone_uid: request.microphone_uid.clone(),
            microphone_name: None,
            capture_scope: request.capture_scope.clone(),
            application_name: None,
            microphone: None,
            system: None,
            cleanup_pending: false,
            error_message: None,
        };
        state.used.insert(request.run_id.clone());
        state.current = Some(Run {
            request,
            snapshot: snapshot.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            task_done: false,
            directory: None,
            child: None,
            ready: None,
            levels: Default::default(),
            playback_tokens: vec![],
        });
        Ok(snapshot)
    }
    pub(crate) fn release_for_startup_reset(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        if state.current.is_some() {
            return Err("録音前テストが実行中です。初期化を開始できません".into());
        }
        state.ownership.release_for_startup_reset()?;
        state.closing = true;
        Ok(())
    }
    pub(crate) fn recovery_snapshot(&self) -> Result<recovery::Snapshot, String> {
        Ok(self.lock()?.ownership.snapshot())
    }
    pub(crate) fn recovery_blocked(&self) -> bool {
        self.lock().map_or(true, |state| state.ownership.blocked())
    }
    pub(crate) fn initialize(app: &AppHandle, root: &std::path::Path) -> Result<(), String> {
        app.state::<Self>().lock()?.ownership.configure(root)?;
        let handle = app.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("mon-test-audio-recovery".into())
            .spawn(move || {
                let _ = Self::recover(&handle, None);
                // Cleanup belongs to the native controller, not a UI panel.
                // Retrying only an attention revision retains the same owned
                // targets and never kills a live writer or scans by filename.
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    let retry = handle.state::<Self>().lock().ok().and_then(|state| {
                        (!state.closing && state.current.is_none())
                            .then(|| state.ownership.retry_revision())
                            .flatten()
                    });
                    if let Some(revision) = retry {
                        let _ = Self::recover(&handle, Some(revision));
                    }
                }
            })
        {
            app.state::<Self>()
                .lock()?
                .ownership
                .attention(format!("テスト音声の回復処理を開始できません: {error}"));
        }
        Ok(())
    }
    pub(crate) fn recover(
        app: &AppHandle,
        expected_revision: Option<u64>,
    ) -> Result<recovery::Snapshot, String> {
        let result = RecorderController::with_transition(app, || {
            let controller = app.state::<Self>();
            let mut state = controller.lock()?;
            if state.current.is_some() || state.closing {
                return Err(
                    "現在のテストまたは終了処理があります。以前の音声の回収は開始していません"
                        .into(),
                );
            }
            if let Some(revision) = expected_revision {
                state.ownership.require_retry(revision)?;
            }
            Ok(state.ownership.reconcile())
        })?;
        let _ = app.emit("capture-test-recovery-changed", &result);
        if result.phase == "clear" {
            let _ = AnalysisController::kick(app);
        }
        Ok(result)
    }
    fn update(app: &AppHandle, id: &str, action: impl FnOnce(&mut Run)) -> Result<(), String> {
        let controller = app.state::<Self>();
        let snapshot = {
            let mut state = controller.lock()?;
            let run = state
                .current
                .as_mut()
                .filter(|r| r.request.run_id == id)
                .ok_or("録音前テストの所有が変更されています")?;
            action(run);
            run.snapshot.revision += 1;
            run.snapshot.clone()
        };
        let _ = app.emit("capture-test-changed", Some(snapshot));
        Ok(())
    }
    pub(crate) fn start(app: &AppHandle, request: Request) -> Result<Snapshot, String> {
        RecorderController::with_transition(app, || {
            let initial = app.state::<Self>().reserve(request.clone())?;
            let _ = app.emit("capture-test-changed", Some(&initial));
            // Record an owned terminal failure even when preflight rejects the
            // request. Losing its reply must not require resending start to
            // discover whether anything was accepted.
            let admission = (|| {
                if app.state::<SessionStore>().snapshot().is_active()
                    || app
                        .state::<crate::recording_confirmation::StopDecisionStore>()
                        .snapshot()
                        .visible()
                {
                    return Err("録音または録音確認が進行中です".to_string());
                }
                app.state::<Storage>().require_data_writable()?;
                request
                    .expected_import
                    .require(&app.state::<ImportController>().snapshot()?)
            })();
            if let Err(error) = admission {
                Self::update(app, &request.run_id, |run| {
                    run.task_done = true;
                    run.snapshot.busy = false;
                    run.snapshot.phase = "error".into();
                    run.snapshot.error_message = Some(error);
                })?;
                return app
                    .state::<Self>()
                    .snapshot(Some(&request.run_id))?
                    .ok_or("録音前テストの受付結果がありません".into());
            }
            let worker = app.clone();
            let id = request.run_id.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("mon-capture-test".into())
                .spawn(move || Self::worker(worker, request))
            {
                Self::update(app, &id, |run| {
                    run.task_done = true;
                    run.snapshot.busy = false;
                    run.snapshot.phase = "error".into();
                    run.snapshot.error_message = Some(error.to_string());
                })?;
            }
            app.state::<Self>()
                .snapshot(Some(&id))?
                .ok_or("録音前テストの受付結果がありません".into())
        })
    }
    fn worker(app: AppHandle, request: Request) {
        let id = request.run_id.clone();
        let cancel = match app
            .state::<Self>()
            .lock()
            .ok()
            .and_then(|s| s.current.as_ref().map(|r| r.cancelled.clone()))
        {
            Some(c) => c,
            None => return,
        };
        let cancelled = || cancel.load(Ordering::Acquire);
        let mut priority = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<files::Ready, String> {
                let (directory, child) = RecorderController::with_transition(&app, || {
                    if cancelled() {
                        return Err("録音前テストを取り消しました".into());
                    }
                    if app.state::<SessionStore>().snapshot().is_active() {
                        return Err("別の録音が進行中です".into());
                    }
                    ImportController::prepare_recording_checked(&app, &request.expected_import)?;
                    priority = true;
                    AnalysisController::prepare_recording(&app)?;
                    if cancelled() {
                        return Err("録音前テストを取り消しました".into());
                    }
                    let inputs = RecorderController::inputs(&app)?;
                    inputs.require_microphone(&request.microphone_uid)?;
                    RecorderController::require_scope(&app, &request.capture_scope)?;
                    let microphone_name = inputs
                        .devices
                        .iter()
                        .find(|d| d.uid == request.microphone_uid)
                        .map(|d| d.name.clone());
                    let application_name =
                        if let Some(identity) = &request.capture_scope.application {
                            RecorderController::applications(&app)?
                                .applications
                                .into_iter()
                                .find(|a| {
                                    a.pid == identity.pid
                                        && a.bundle_identifier == identity.bundle_identifier
                                        && a.launch_time_us == identity.launch_time_us
                                })
                                .map(|a| a.name)
                        } else {
                            None
                        };
                    let binary = crate::recorder::probe_path(&app)?;
                    #[cfg(any(target_os = "macos", windows))]
                    let directory = Arc::new(
                        app.state::<Self>()
                            .lock()?
                            .ownership
                            .allocate(request.clone())?,
                    );
                    #[cfg(not(any(target_os = "macos", windows)))]
                    let directory = Arc::new(files::TestDirectory::new()?);
                    Self::update(&app, &id, |run| {
                        run.directory = Some(directory.clone());
                        run.snapshot.microphone_name = microphone_name;
                        run.snapshot.application_name = application_name;
                    })?;
                    if cancelled() {
                        return Err("録音前テストを取り消しました".into());
                    }
                    #[cfg(any(target_os = "macos", windows))]
                    let mut launch = owned_command(&binary, &directory, &request)?;
                    #[cfg(not(any(target_os = "macos", windows)))]
                    let mut launch = command(&binary, &directory.path, &request)?;
                    let spawned = launch.spawn().map_err(|e| e.to_string());
                    // The command's pre_exec closure owns a duplicate. Release
                    // it before any collector tests whether the helper exited.
                    drop(launch);
                    let child = Arc::new(Probe(Mutex::new(spawned?)));
                    Self::update(&app, &id, |run| run.child = Some(child.clone()))?;
                    Ok((directory, child))
                })?;
                watch(
                    &directory,
                    &child,
                    &request,
                    &cancel,
                    Duration::from_secs(90),
                    || {
                        Self::update(&app, &id, |run| {
                            if !run.cancelled.load(Ordering::Acquire)
                                && run.snapshot.phase == "preparing"
                            {
                                run.snapshot.phase = "recording".into();
                            }
                        })
                    },
                )?;
                Self::update(&app, &id, |run| {
                    run.child = None;
                    if !cancelled() {
                        run.snapshot.phase = "checking".into();
                    }
                })?;
                if cancelled() {
                    return Err("録音前テストを取り消しました".into());
                }
                files::verify(&directory, &request, &cancelled)
            },
        ))
        .unwrap_or_else(|_| Err("録音前テストの処理が停止しました".into()));
        // Release process ownership before marking a cancellation terminal,
        // including error/unwind paths and failed writes of the stop request.
        let child = app
            .state::<Self>()
            .lock()
            .ok()
            .and_then(|s| s.current.as_ref().and_then(|r| r.child.clone()));
        if let Some(child) = child {
            // Never surrender the process handle or recording priority merely
            // because kill/wait failed once. Keep the worker and owner alive,
            // expose the unresolved stop, and retry confirmation of exit.
            reap_until_confirmed(
                || child.terminate(),
                |error| {
                    let _ = Self::update(&app, &id, |run| {
                        run.cancelled.store(true, Ordering::Release);
                        run.snapshot.phase = "cancelling".into();
                        run.snapshot.error_message = Some(format!(
                            "取得プロセスの終了確認を再試行しています。録音は開始できません: {error}"
                        ));
                    });
                },
                || std::thread::sleep(Duration::from_secs(1)),
            );
        }
        let _ = RecorderController::with_transition(&app, || {
            let updated = Self::update(&app, &id, |run| {
                if let Some(directory) = &run.directory {
                    directory.release_parent_lease();
                }
                run.child = None;
                run.task_done = true;
                run.snapshot.busy = false;
                match result {
                    Ok(ready) if !cancelled() => {
                        run.snapshot.phase = "ready".into();
                        run.snapshot.microphone =
                            ready.microphone.as_ref().map(|track| track.info.clone());
                        run.snapshot.system = ready.system.as_ref().map(|track| track.info.clone());
                        run.ready = Some(ready);
                    }
                    other => {
                        run.snapshot.phase = if cancelled() { "cancelled" } else { "error" }.into();
                        run.snapshot.error_message = terminal_error(other.err(), cancelled());
                        if let Some(directory) = &run.directory {
                            match directory.cleanup() {
                                Ok(()) => run.directory = None,
                                Err(error) => {
                                    run.snapshot.cleanup_pending = true;
                                    let primary =
                                        run.snapshot.error_message.take().unwrap_or_default();
                                    run.snapshot.error_message = Some(format!(
                                        "{primary} / 一時音声の回収が未完了です: {error}"
                                    ));
                                }
                            }
                        }
                    }
                }
            });
            if priority {
                AnalysisController::recording_finished(&app);
            }
            updated
        });
    }
    pub(crate) fn cancel(app: &AppHandle, id: &str) -> Result<Snapshot, String> {
        Self::update(app, id, |run| {
            if !run.task_done {
                run.cancelled.store(true, Ordering::Release);
                run.snapshot.phase = "cancelling".into();
            }
        })?;
        app.state::<Self>()
            .snapshot(Some(id))?
            .ok_or("録音前テストがありません".into())
    }
    pub(crate) fn discard(app: &AppHandle, id: &str) -> Result<(), String> {
        RecorderController::with_transition(app, || {
            let controller = app.state::<Self>();
            let mut state = controller.lock()?;
            let run = state
                .current
                .as_mut()
                .filter(|r| r.request.run_id == id)
                .ok_or("破棄対象のテストが変更されています")?;
            if !run.task_done || run.child.is_some() {
                return Err("テストの停止・検証が終わってから一時音声を閉じてください".into());
            }
            for token in &run.playback_tokens {
                app.state::<PlaybackStore>().close(token)?;
            }
            run.playback_tokens.clear();
            if let Some(directory) = &run.directory {
                directory.cleanup()?;
            }
            state.current = None;
            drop(state);
            let _ = app.emit("capture-test-changed", Option::<Snapshot>::None);
            Ok(())
        })
    }
    pub(crate) fn levels(
        app: &AppHandle,
        id: &str,
    ) -> Result<crate::input_levels::InputLevels, String> {
        let controller = app.state::<Self>();
        let mut state = controller.lock()?;
        let run = state
            .current
            .as_mut()
            .filter(|r| r.request.run_id == id)
            .ok_or("入力確認するテストが変更されています")?;
        if run.cancelled.load(Ordering::Acquire)
            || !matches!(run.snapshot.phase.as_str(), "preparing" | "recording")
        {
            return Err("入力確認のテストは停止しています".into());
        }
        let child = run.child.as_ref().ok_or("テスト入力の初期化待ちです")?;
        let directory = run.directory.as_ref().ok_or("テストの保存先がありません")?;
        if child.exited()?.is_some() {
            return Err("テストの取得プロセスは終了しました".into());
        }
        let report = run
            .levels
            .read_file(directory.open("input-levels.json", 16 * 1024)?, id)?;
        if child.exited()?.is_some() || run.cancelled.load(Ordering::Acquire) {
            return Err("入力確認中にテストが停止しました".into());
        }
        Ok(report)
    }
    pub(crate) fn prepare_playback(
        app: &AppHandle,
        id: &str,
        kind: &str,
        preparation_id: &str,
    ) -> Result<PlaybackInfo, String> {
        let guard = app.state::<PlaybackPreparations>().claim(preparation_id)?;
        let controller = app.state::<Self>();
        let mut state = controller.lock()?;
        let run = state
            .current
            .as_mut()
            .filter(|r| r.request.run_id == id)
            .ok_or("再生するテストが変更されています")?;
        if run.playback_tokens.len() >= 128 {
            return Err(
                "テストの未確認の再生が多すぎます。一時音声を閉じて再確認してください".into(),
            );
        }
        if !run.task_done || run.snapshot.phase != "ready" || run.cancelled.load(Ordering::Acquire)
        {
            return Err("停止・検証済みのテスト音声だけを再生できます".into());
        }
        let ready = run.ready.as_ref().ok_or("検証済み音声がありません")?;
        let track = match kind {
            "mic" => ready.microphone.as_ref().ok_or(
                "マイク音声は未受信のため再生できません。マイクを確認して再テストしてください",
            )?,
            "system" => ready
                .system
                .as_ref()
                .ok_or("PC音声は未受信のため再生できません。音声を再生して再テストしてください")?,
            _ => return Err("テストの再生トラックが不正です".into()),
        };
        let audio = track.playback(
            run.directory
                .as_ref()
                .ok_or("テスト音声の保存先がありません")?,
            kind,
            &|| guard.cancelled(),
        )?;
        let info = guard.publish(|| app.state::<PlaybackStore>().insert(audio))?;
        run.playback_tokens.push(info.token.clone());
        Ok(info)
    }
    pub(crate) fn release_playback(&self, token: &str) -> Result<(), String> {
        if let Some(run) = self.lock()?.current.as_mut() {
            run.playback_tokens.retain(|owned| owned != token);
        }
        Ok(())
    }
}
fn reap_until_confirmed(
    mut terminate: impl FnMut() -> Result<(), String>,
    mut unresolved: impl FnMut(String),
    mut retry_wait: impl FnMut(),
) {
    while let Err(error) = terminate() {
        unresolved(error);
        retry_wait();
    }
}
fn watch(
    directory: &files::TestDirectory,
    child: &Probe,
    request: &Request,
    cancel: &AtomicBool,
    timeout: Duration,
    recording: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let started = Instant::now();
    let mut stopping: Option<Instant> = None;
    let mut reported = false;
    loop {
        if let Some(status) = child.exited()? {
            if !cancel.load(Ordering::Acquire) {
                if let Some(manifest) = directory.manifest()? {
                    if let Some(reason) = manifest.failure(request)? {
                        return Err(reason);
                    }
                }
            }
            if status.success() && !cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            if cancel.load(Ordering::Acquire) {
                return Err("録音前テストを取り消しました".into());
            }
            #[cfg(windows)]
            if let Some(reason) = child.failure_detail() {
                return Err(format!(
                    "録音前テストを開始・保存できませんでした: {reason}"
                ));
            }
            return Err("テストを取り消したか、録音ヘルパーが保存を完了できませんでした".into());
        }
        if cancel.load(Ordering::Acquire) && stopping.is_none() {
            directory.request_stop()?;
            stopping = Some(Instant::now());
        }
        if started.elapsed() > timeout
            || stopping.is_some_and(|t| t.elapsed() > Duration::from_secs(10))
        {
            child.terminate()?;
            return Err(
                "テストの初期化または停止が時間上限を超えました。取得プロセスは終了しました".into(),
            );
        }
        if let Some(manifest) = directory.manifest()? {
            if let Some(reason) = manifest.failure(request)? {
                return Err(reason);
            }
            manifest.require_owner(request)?;
            if manifest.state == "recording" && !reported && !cancel.load(Ordering::Acquire) {
                recording()?;
                reported = true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// Cancellation is an expected outcome. Keep unrelated failures (including stop
// timeouts) visible, and append cleanup failures separately at the call site.
fn terminal_error(error: Option<String>, cancelled: bool) -> Option<String> {
    error.filter(|message| {
        !cancelled
            || !matches!(
                message.as_str(),
                "録音前テストを取り消しました"
                    | "テストの音声検証を取り消しました"
                    | "音声ファイルの照合を中断しました"
                    | "録音音声の照合を中断しました"
                    | "FLAC音声処理を中断しました"
            )
    })
}

#[cfg(test)]
mod cancellation_tests {
    use super::terminal_error;

    #[test]
    fn normal_cancellation_has_no_error() {
        for message in [
            "録音前テストを取り消しました",
            "テストの音声検証を取り消しました",
            "音声ファイルの照合を中断しました",
            "録音音声の照合を中断しました",
            "FLAC音声処理を中断しました",
        ] {
            assert_eq!(terminal_error(Some(message.into()), true), None);
            assert_eq!(
                terminal_error(Some(message.into()), false),
                Some(message.into())
            );
        }
        assert_eq!(terminal_error(None, true), None);
    }

    #[test]
    fn cancellation_preserves_real_failures() {
        let message = "テストの初期化または停止が時間上限を超えました。取得プロセスは終了しました";
        assert_eq!(
            terminal_error(Some(message.into()), true),
            Some(message.into())
        );
    }
}
