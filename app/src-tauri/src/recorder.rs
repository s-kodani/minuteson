use crate::session::{RecordingPhase, RecordingSnapshot, SessionStore};
use crate::storage::{RecordingConfig, Storage};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

struct ActiveCapture {
    session_id: String,
    output: PathBuf,
    child: Arc<Mutex<Child>>,
    stop_operation_id: Option<String>,
    level_directory: PathBuf,
    levels: crate::input_levels::LevelReader,
    writes: crate::recording_writes::WriteReader,
    input_status: crate::input_status::StatusReader,
}

#[derive(Default)]
pub struct RecorderController(Mutex<Option<ActiveCapture>>, Mutex<()>);

fn require_level_session(
    current: &str,
    snapshot: &RecordingSnapshot,
    requested: &str,
) -> Result<(), String> {
    if !crate::input_levels::valid_owner(requested)
        || current != requested
        || snapshot.session_id.as_deref() != Some(requested)
        || !matches!(
            snapshot.phase,
            RecordingPhase::Preparing | RecordingPhase::Recording
        )
    {
        return Err("入力確認する録音が変更されたか、停止しました".into());
    }
    Ok(())
}

fn recording_command(
    binary: &Path,
    output: &Path,
    parent_pid: u32,
    microphone_uid: &str,
    scope: &crate::capture_scope::CaptureScope,
    session_id: &str,
) -> Result<Command, String> {
    scope.validate()?;
    if !crate::input_levels::valid_owner(session_id) {
        return Err("入力確認の録音IDが不正です".into());
    }
    let mut command = Command::new(binary);
    command
        .arg("--output")
        .arg(output)
        .arg("--mode")
        .arg(&scope.mode)
        .arg("--backend")
        .arg("screencapturekit")
        .arg("--scenario")
        .arg(format!("mon-app-{}", scope.mode))
        .arg("--stop-on-file")
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .arg("--until-stopped")
        .arg("--microphone-uid")
        .arg(microphone_uid)
        .arg("--input-level-owner")
        .arg(session_id)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(identity) = &scope.application {
        command
            .arg("--pid")
            .arg(identity.pid.to_string())
            .arg("--application-identity")
            .arg(serde_json::to_string(identity).map_err(|e| e.to_string())?);
    }
    Ok(command)
}

impl RecorderController {
    pub(crate) fn input_status(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<crate::input_status::Snapshot, String> {
        let controller = app.state::<Self>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません")?;
        let current = active
            .as_mut()
            .ok_or("マイク状態を確認できる録音がありません")?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        let mut child = current
            .child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("録音プロセスは終了しました".into());
        }
        // One backend owner chooses one notification destination. Both windows
        // still read the same retained state; hiding a window never retries audio.
        let main_visible = app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false));
        let recorder_visible = app
            .get_webview_window("recorder")
            .is_some_and(|w| w.is_visible().unwrap_or(false));
        let target = crate::input_status::notice_target(main_visible, recorder_visible);
        let result = current.input_status.read(session_id, target)?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("マイク状態の確認中に録音プロセスが終了しました".into());
        }
        Ok(result)
    }
    pub(crate) fn writes(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<crate::recording_writes::Snapshot, String> {
        let controller = app.state::<Self>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません")?;
        let current = active.as_mut().ok_or("書込み確認できる録音がありません")?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        let mut child = current
            .child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("録音プロセスは終了しました".into());
        }
        let result = current.writes.read(&current.level_directory, session_id)?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("書込み確認中に録音プロセスが終了しました".into());
        }
        Ok(result)
    }
    pub(crate) fn levels(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<crate::input_levels::InputLevels, String> {
        let controller = app.state::<Self>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません")?;
        let current = active.as_mut().ok_or("入力確認できる録音がありません")?;
        let snapshot = app.state::<SessionStore>().snapshot();
        require_level_session(&current.session_id, &snapshot, session_id)?;
        let mut child = current
            .child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("入力確認の録音プロセスは終了しました".into());
        }
        let levels = current.levels.read(&current.level_directory, session_id)?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("入力確認中に録音プロセスが終了しました".into());
        }
        Ok(levels)
    }
    // Starting includes analysis/import admission; ending includes releasing
    // those intents. Neither may publish over the other session's transition.
    pub fn with_transition<T>(
        app: &AppHandle,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let controller = app.state::<Self>();
        let _transition = controller
            .1
            .lock()
            .map_err(|_| "録音状態の切替ロックに失敗しました")?;
        action()
    }
    pub fn available(app: &AppHandle) -> bool {
        probe_path(app).is_ok()
    }

    pub(crate) fn inputs(app: &AppHandle) -> Result<crate::capture_inputs::InputDevices, String> {
        let binary = probe_path(app)?;
        let mut inputs = crate::capture_inputs::read(&binary)?;
        if probe_path(app)? != binary {
            return Err("録音ヘルパーが変更されました。再確認してください".into());
        }
        inputs.selected_microphone_uid = app.state::<Storage>().selected_microphone_uid()?;
        Ok(inputs)
    }

    pub(crate) fn applications(
        app: &AppHandle,
    ) -> Result<crate::capture_scope::Applications, String> {
        let binary = probe_path(app)?;
        let applications = crate::capture_scope::read(&binary)?;
        if probe_path(app)? != binary {
            return Err("録音ヘルパーが変更されました。再確認してください".into());
        }
        Ok(applications)
    }

    pub(crate) fn require_scope(
        app: &AppHandle,
        scope: &crate::capture_scope::CaptureScope,
    ) -> Result<(), String> {
        scope.validate()?;
        if let Some(identity) = &scope.application {
            Self::applications(app)?.require(identity)?;
        }
        Ok(())
    }

    pub fn start(
        app: &AppHandle,
        meeting_id: &str,
        microphone_uid: &str,
        scope: &crate::capture_scope::CaptureScope,
    ) -> Result<RecordingSnapshot, String> {
        app.state::<Storage>().require_data_writable()?;
        Self::inputs(app)?.require_microphone(microphone_uid)?;
        Self::require_scope(app, scope)?;
        let binary = probe_path(app)?;
        let controller = app.state::<RecorderController>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません".to_string())?;
        if active.is_some() {
            return Err("別の録音が進行中です".into());
        }
        let storage = app.state::<Storage>();
        if storage.request_pause_for_recording()? {
            return Err("解析処理の安全な停止を待っています。停止後に再試行してください".into());
        }
        let config = RecordingConfig {
            mode: scope.mode.clone(),
            backend: "screencapturekit".into(),
            target_pids: scope
                .application
                .as_ref()
                .map(|a| vec![a.pid])
                .unwrap_or_default(),
            microphone_device_uid: Some(microphone_uid.into()),
            target_application_identity: scope.application.clone(),
        };
        let session =
            storage.prepare_recording(meeting_id, &config, &Uuid::new_v4().to_string())?;
        let relative = PathBuf::from("meetings")
            .join(meeting_id)
            .join("audio")
            .join(&session.id);
        let output = storage.managed_path(&relative)?;
        let launch =
            (|| -> Result<(Child, PathBuf, crate::recording_writes::WriteReader, crate::input_status::StatusReader), String> {
                std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
                let root = storage
                    .managed_path(Path::new("meetings"))?
                    .parent()
                    .ok_or("保存先が不正です")?
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                if output.canonicalize().map_err(|error| error.to_string())? != root.join(&relative)
                {
                    return Err("録音先が管理領域外です".into());
                }
                if std::fs::read_dir(&output)
                    .map_err(|error| error.to_string())?
                    .next()
                    .is_some()
                {
                    return Err("録音先に既存ファイルがあります".into());
                }
                let writes = crate::recording_writes::WriteReader::pin(&root.join(&relative))?;
                let input_status = crate::input_status::StatusReader::pin(&root.join(&relative))?;
                let child = recording_command(
                    &binary,
                    &output,
                    std::process::id(),
                    microphone_uid,
                    scope,
                    &session.id,
                )?
                .spawn()
                .map_err(|error| error.to_string())?;
                Ok((child, root.join(&relative), writes, input_status))
            })();
        let (child, level_directory, writes, input_status) = match launch {
            Ok(child) => child,
            Err(error) => {
                let _ = storage.fail_recording(&session.id);
                return Err(format!("録音プローブを起動できません: {error}"));
            }
        };
        let child = Arc::new(Mutex::new(child));
        *active = Some(ActiveCapture {
            session_id: session.id.clone(),
            output: output.clone(),
            child: child.clone(),
            stop_operation_id: None,
            level_directory,
            levels: Default::default(),
            writes,
            input_status,
        });
        let snapshot = publish(app, |state| {
            state.phase = RecordingPhase::Preparing;
            state.elapsed_ms = 0;
            state.microphone_connected = false;
            state.system_audio_connected = false;
            state.meeting_id = Some(meeting_id.to_owned());
            state.session_id = Some(session.id.clone());
            state.error_message = None;
        });
        if let Some(panel) = app.get_webview_window("recorder") {
            let _ = panel.show();
        }
        let worker_app = app.clone();
        let worker_session = session.id;
        std::thread::spawn(move || watch_probe(worker_app, child, worker_session, output));
        Ok(snapshot)
    }

    pub fn stop(app: &AppHandle) -> Result<RecordingSnapshot, String> {
        Self::stop_target(app, None)
    }

    pub fn stop_session(app: &AppHandle, session: &str) -> Result<RecordingSnapshot, String> {
        Self::stop_target(app, Some(session))
    }

    fn stop_target(app: &AppHandle, expected: Option<&str>) -> Result<RecordingSnapshot, String> {
        let controller = app.state::<RecorderController>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません".to_string())?;
        let current = active.as_mut().ok_or("停止できる録音がありません")?;
        if expected.is_some_and(|session| session != current.session_id) {
            return Err("停止対象の録音が変更されました。別の録音には停止を送っていません".into());
        }
        if current.stop_operation_id.is_some()
            && matches!(
                app.state::<SessionStore>().snapshot().phase,
                RecordingPhase::Stopping | RecordingPhase::Saving
            )
        {
            return Ok(app.state::<SessionStore>().snapshot());
        }
        let operation = current
            .stop_operation_id
            .get_or_insert_with(|| Uuid::new_v4().to_string())
            .clone();
        app.state::<Storage>()
            .request_recording_stop(&current.session_id, &operation)?;
        let request = current.output.join("stop.request");
        if let Err(error) = File::create(&request) {
            let _ = current
                .child
                .lock()
                .map_err(|_| "録音プロセスのロックに失敗しました".to_string())?
                .kill();
            return Err(format!("録音停止要求を書き込めません: {error}"));
        }
        let snapshot = publish(app, |state| state.phase = RecordingPhase::Stopping);
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";

    #[test]
    fn levels_require_current_canonical_active_session_not_connection() {
        let mut snapshot = RecordingSnapshot {
            session_id: Some(SESSION.into()),
            phase: RecordingPhase::Preparing,
            ..Default::default()
        };
        assert!(require_level_session(SESSION, &snapshot, SESSION).is_ok());
        snapshot.phase = RecordingPhase::Recording;
        assert!(require_level_session(SESSION, &snapshot, SESSION).is_ok());
        assert!(
            require_level_session(SESSION, &snapshot, "735027d4-a03e-43d3-830a-21e767dfaf89")
                .is_err()
        );
        assert!(require_level_session("other-capture", &snapshot, SESSION).is_err());
        snapshot.session_id = None;
        assert!(require_level_session(SESSION, &snapshot, SESSION).is_err());
        snapshot.session_id = Some(SESSION.into());
        snapshot.microphone_connected = true;
        snapshot.system_audio_connected = true;
        for phase in [
            RecordingPhase::Idle,
            RecordingPhase::Stopping,
            RecordingPhase::Saving,
            RecordingPhase::Error,
        ] {
            snapshot.phase = phase;
            assert!(require_level_session(SESSION, &snapshot, SESSION).is_err());
        }
    }

    #[test]
    fn invalid_level_owner_never_becomes_a_capture_command() {
        let scope = crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: None,
        };
        for id in [
            "../outside",
            "735027d4-a03e-13d3-830a-21e767dfaf88",
            "735027d4-a03e-43d3-030a-21e767dfaf88",
        ] {
            assert!(recording_command(
                Path::new("/tmp/probe"),
                Path::new("/tmp/output"),
                1234,
                "usb",
                &scope,
                id
            )
            .is_err());
        }
    }

    #[test]
    fn product_recording_waits_for_stop_or_parent_exit_without_a_duration_deadline() {
        let binary = Path::new("/tmp/mon-command-test/probe");
        let output = Path::new("/tmp/mon-command-test/audio with spaces");
        let scope = crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: None,
        };
        let command = recording_command(
            binary,
            output,
            1234,
            "USB input/名前 with spaces",
            &scope,
            SESSION,
        )
        .unwrap();
        assert_eq!(command.get_program(), binary.as_os_str());
        let arguments: Vec<_> = command.get_args().collect();
        let expected = [
            "--output",
            output.to_str().unwrap(),
            "--mode",
            "system",
            "--backend",
            "screencapturekit",
            "--scenario",
            "mon-app-system",
            "--stop-on-file",
            "--parent-pid",
            "1234",
            "--until-stopped",
            "--microphone-uid",
            "USB input/名前 with spaces",
            "--input-level-owner",
            SESSION,
        ];
        assert_eq!(
            arguments,
            expected
                .iter()
                .map(std::ffi::OsStr::new)
                .collect::<Vec<_>>()
        );
        assert!(!arguments.contains(&std::ffi::OsStr::new("--duration")));
    }

    #[test]
    fn application_command_binds_the_process_instance_as_one_json_argument() {
        let identity = crate::capture_scope::ApplicationIdentity {
            pid: 456,
            bundle_identifier: "com.example.名前 with spaces".into(),
            launch_time_us: 1791100000000001,
        };
        let scope = crate::capture_scope::CaptureScope {
            mode: "application".into(),
            application: Some(identity.clone()),
        };
        let command = recording_command(
            Path::new("/tmp/probe"),
            Path::new("/tmp/output"),
            1234,
            "usb",
            &scope,
            SESSION,
        )
        .unwrap();
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        let value = |flag: &str| args[args.iter().position(|a| *a == flag).unwrap() + 1];
        assert_eq!(value("--mode"), "application");
        assert_eq!(value("--scenario"), "mon-app-application");
        assert_eq!(value("--pid"), "456");
        assert_eq!(
            serde_json::from_str::<crate::capture_scope::ApplicationIdentity>(value(
                "--application-identity"
            ))
            .unwrap(),
            identity
        );
        assert_eq!(
            args.iter()
                .filter(|a| **a == "--application-identity")
                .count(),
            1
        );
        assert!(!args.contains(&"--duration"));
        assert!(args.contains(&"--until-stopped"));
        let malformed = crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: scope.application,
        };
        assert!(recording_command(
            Path::new("/tmp/probe"),
            Path::new("/tmp/output"),
            1234,
            "usb",
            &malformed,
            SESSION
        )
        .is_err());
    }
}

pub(crate) fn probe_path(app: &AppHandle) -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("この録音プローブはmacOS専用です".into());
    }
    let relative = Path::new("MONCaptureProbe.app/Contents/MacOS/mon-macos-capture");
    let mut candidates = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join(relative));
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../probes/macos-capture/.build")
            .join(relative),
    );
    for candidate in candidates {
        if let Ok(path) = candidate.canonicalize() {
            if path.is_file() {
                let bundle = path.parent().and_then(Path::parent).and_then(Path::parent);
                if bundle.is_some_and(|bundle| {
                    Command::new("codesign")
                        .arg("--verify")
                        .arg("--deep")
                        .arg("--strict")
                        .arg(bundle)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|status| status.success())
                }) {
                    return Ok(path);
                }
            }
        }
    }
    Err("署名済みmacOS録音プローブが見つからないか署名検証に失敗しました。probes/macos-capture/build-bundle.shを実行してください".into())
}

fn publish(app: &AppHandle, change: impl FnOnce(&mut RecordingSnapshot)) -> RecordingSnapshot {
    let snapshot = app.state::<SessionStore>().update(change);
    crate::recording_confirmation_window::session_changed(app, &snapshot);
    let _ = app.emit("session-changed", snapshot.clone());
    snapshot
}

fn manifest_state(output: &Path) -> Option<String> {
    let manifest = std::fs::read(output.join("manifest.json")).ok()?;
    if manifest.len() > 256 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    value.get("state")?.as_str().map(str::to_owned)
}

fn manifest_error(output: &Path) -> Option<String> {
    let bytes = std::fs::read(output.join("manifest.json")).ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let details = value
        .get("errors")?
        .as_array()?
        .iter()
        .filter_map(|error| error.as_str())
        .take(3)
        .collect::<Vec<_>>()
        .join("; ");
    if details.is_empty() {
        None
    } else {
        Some(details.chars().take(400).collect())
    }
}

fn watch_probe(app: AppHandle, child: Arc<Mutex<Child>>, session_id: String, output: PathBuf) {
    let started = Instant::now();
    let mut announced = false;
    let mut last_second = 0;
    let exit = loop {
        let status = child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました".to_string())
            .and_then(|mut process| process.try_wait().map_err(|error| error.to_string()));
        match status {
            Ok(Some(status)) => break Ok(status.success()),
            Err(error) => {
                if let Ok(mut process) = child.lock() {
                    let _ = process.kill();
                }
                break Err(error);
            }
            Ok(None) => {}
        }
        if !announced && manifest_state(&output).as_deref() == Some("recording") {
            let state = app.state::<SessionStore>().snapshot();
            if state.phase == RecordingPhase::Preparing {
                if app
                    .state::<Storage>()
                    .mark_recording_started(&session_id)
                    .is_ok()
                {
                    announced = true;
                    publish(&app, |snapshot| {
                        snapshot.phase = RecordingPhase::Recording;
                        snapshot.microphone_connected = false;
                        snapshot.system_audio_connected = true;
                    });
                }
            } else if state.phase == RecordingPhase::Stopping {
                announced = true;
            }
        }
        let connected = RecorderController::input_status(&app, &session_id)
            .is_ok_and(|status| status.connected());
        if let Some(snapshot) = app.state::<SessionStore>().update_if(|snapshot| {
            if snapshot.session_id.as_deref() != Some(&session_id)
                || !matches!(
                    snapshot.phase,
                    RecordingPhase::Preparing | RecordingPhase::Recording
                )
                || snapshot.microphone_connected == connected
            {
                return false;
            }
            snapshot.microphone_connected = connected;
            true
        }) {
            crate::recording_confirmation_window::session_changed(&app, &snapshot);
            let _ = app.emit("session-changed", snapshot);
        }
        let seconds = started.elapsed().as_secs();
        if seconds > last_second {
            last_second = seconds;
            publish(&app, |snapshot| {
                snapshot.elapsed_ms = seconds.saturating_mul(1000)
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let storage = app.state::<Storage>();
    let (result, recoverable) = match exit {
        Ok(true) => {
            if !announced {
                let _ = storage.mark_recording_started(&session_id);
            }
            publish(&app, |snapshot| snapshot.phase = RecordingPhase::Saving);
            (storage.finalize_recording(&session_id).map(|_| ()), true)
        }
        Ok(false) => (
            Err(manifest_error(&output).unwrap_or_else(|| {
                "録音プローブが失敗しました。録音権限と保存先を確認してください".into()
            })),
            false,
        ),
        Err(error) => (Err(format!("録音プロセスを確認できません: {error}")), false),
    };
    if result.is_err() {
        if recoverable {
            let _ = storage.interrupt_recording(&session_id);
        } else {
            let _ = storage.fail_recording(&session_id);
        }
    }
    let controller = app.state::<RecorderController>();
    let _transition = controller
        .1
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Ok(mut active) = controller.0.lock() {
        if !active
            .as_ref()
            .is_some_and(|capture| capture.session_id == session_id)
        {
            return;
        }
        if active
            .as_ref()
            .is_some_and(|capture| capture.session_id == session_id)
        {
            *active = None;
        }
    }
    match result {
        Ok(()) => {
            publish(&app, |snapshot| {
                snapshot.phase = RecordingPhase::Idle;
                snapshot.elapsed_ms = 0;
                snapshot.microphone_connected = false;
                snapshot.system_audio_connected = false;
                snapshot.meeting_id = None;
                snapshot.session_id = None;
                snapshot.error_message = None;
            });
        }
        Err(error) => {
            publish(&app, |snapshot| {
                snapshot.phase = RecordingPhase::Error;
                snapshot.microphone_connected = false;
                snapshot.system_audio_connected = false;
                snapshot.error_message = Some(if recoverable {
                    format!("録音ファイルの確定に失敗しました。元ファイルは保持し、次回起動時に再検証します: {error}")
                } else {
                    error
                });
            });
        }
    }
    crate::analysis_worker::AnalysisController::recording_finished(&app);
}
