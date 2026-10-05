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
}

#[derive(Default)]
pub struct RecorderController(Mutex<Option<ActiveCapture>>);

impl RecorderController {
    pub fn available(app: &AppHandle) -> bool {
        probe_path(app).is_ok()
    }

    pub fn start(app: &AppHandle, meeting_id: &str) -> Result<RecordingSnapshot, String> {
        app.state::<Storage>().require_data_writable()?;
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
            mode: "system".into(),
            backend: "screencapturekit".into(),
            target_pids: vec![],
        };
        let session =
            storage.prepare_recording(meeting_id, &config, &Uuid::new_v4().to_string())?;
        let relative = PathBuf::from("meetings")
            .join(meeting_id)
            .join("audio")
            .join(&session.id);
        let output = storage.managed_path(&relative)?;
        let launch = (|| -> Result<Child, String> {
            std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
            let root = storage
                .managed_path(Path::new("meetings"))?
                .parent()
                .ok_or("保存先が不正です")?
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if output.canonicalize().map_err(|error| error.to_string())? != root.join(&relative) {
                return Err("録音先が管理領域外です".into());
            }
            if std::fs::read_dir(&output)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                return Err("録音先に既存ファイルがあります".into());
            }
            Command::new(&binary)
                .arg("--output")
                .arg(&output)
                .arg("--duration")
                .arg("14400")
                .arg("--mode")
                .arg("system")
                .arg("--backend")
                .arg("screencapturekit")
                .arg("--scenario")
                .arg("mon-app-system")
                .arg("--stop-on-file")
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())
        })();
        let child = match launch {
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
        let controller = app.state::<RecorderController>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません".to_string())?;
        let current = active.as_mut().ok_or("停止できる録音がありません")?;
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
                        snapshot.microphone_connected = true;
                        snapshot.system_audio_connected = true;
                    });
                }
            } else if state.phase == RecordingPhase::Stopping {
                announced = true;
            }
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
    if let Ok(mut active) = app.state::<RecorderController>().0.lock() {
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
