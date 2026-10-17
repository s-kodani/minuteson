use crate::session::{RecordingPhase, RecordingSnapshot, SessionStore};
use crate::storage::{RecordingConfig, Storage};
use std::fs::File;
use std::io::{Read, Write};
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
    input_status: Arc<Mutex<crate::input_status::StatusReader>>,
    selection_revision: u64,
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

// Commit the canonical phase while owning the capture, then release it before
// dispatching anything that may enter a WebView or the native event loop.
fn publish_after_unlock<G, T>(
    guard: G,
    commit: impl FnOnce() -> T,
    dispatch: impl FnOnce(&T),
) -> T {
    let snapshot = commit();
    drop(guard);
    dispatch(&snapshot);
    snapshot
}

fn mark_started(store: &SessionStore, owner: &str) -> bool {
    store
        .update_if(|snapshot| {
            if snapshot.session_id.as_deref() != Some(owner)
                || snapshot.phase != RecordingPhase::Preparing
            {
                return false;
            }
            snapshot.phase = RecordingPhase::Recording;
            snapshot.microphone_connected = false;
            snapshot.system_audio_connected = false;
            true
        })
        .is_some()
}

fn recording_command(
    binary: &Path,
    output: &Path,
    parent_pid: u32,
    microphone_uid: Option<&str>,
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
        .arg("coreaudio")
        .arg("--scenario")
        .arg(format!("mon-app-{}", scope.mode))
        .arg("--stop-on-file")
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .arg("--until-stopped")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(uid) = microphone_uid {
        command.arg("--microphone-uid").arg(uid);
    }
    command.arg("--input-level-owner").arg(session_id);
    if let Some(identity) = &scope.application {
        command
            .arg("--pid")
            .arg(identity.pid.to_string())
            .arg("--application-identity")
            .arg(serde_json::to_string(identity).map_err(|e| e.to_string())?);
    }
    Ok(command)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MicrophoneSelectionRequest<'a> {
    schema_version: u32,
    owner_id: &'a str,
    revision: u64,
    mode: &'static str,
    uid: Option<&'a str>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MicrophoneSelectionStatus {
    schema_version: u32,
    owner_id: String,
    revision: u64,
    request_revision: u64,
    mode: String,
    #[serde(rename = "deviceUID")]
    device_uid: String,
    source: String,
    fallback_reason: Option<String>,
}

fn parse_microphone_selection_status(
    bytes: &[u8],
    owner: &str,
) -> Result<MicrophoneSelectionStatus, String> {
    if bytes.is_empty() || bytes.len() > 4096 || !crate::input_levels::valid_owner(owner) {
        return Err("マイク選択状態の対象または容量が不正です".into());
    }
    let value: MicrophoneSelectionStatus =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if value.schema_version != 1
        || value.owner_id != owner
        || !(1..=9_007_199_254_740_991).contains(&value.revision)
        || value.request_revision > 9_007_199_254_740_991
        || !crate::storage::valid_microphone_uid(&value.device_uid)
        || !match (
            value.mode.as_str(),
            value.source.as_str(),
            value.fallback_reason.as_deref(),
        ) {
            ("fixed", "fixed" | "pending", None)
            | ("automatic", "application", None)
            | ("automatic", "pending", None) => true,
            ("automatic", "systemDefault", Some(reason)) => matches!(
                reason,
                "mon_direct_recording"
                    | "application_input_unavailable"
                    | "application_input_ambiguous"
            ),
            _ => false,
        }
    {
        return Err("マイク選択状態の内容が不正です".into());
    }
    Ok(value)
}

fn publish_microphone_selection(
    output: &Path,
    owner: &str,
    revision: u64,
    uid: Option<&str>,
) -> Result<(), String> {
    if !crate::input_levels::valid_owner(owner)
        || revision == 0
        || revision > 9_007_199_254_740_991
        || uid.is_some_and(|value| !crate::storage::valid_microphone_uid(value))
    {
        return Err("マイク切替の対象または指定が不正です".into());
    }
    let data = serde_json::to_vec(&MicrophoneSelectionRequest {
        schema_version: 1,
        owner_id: owner,
        revision,
        mode: if uid.is_some() { "fixed" } else { "automatic" },
        uid,
    })
    .map_err(|error| error.to_string())?;
    if data.len() > 4096 {
        return Err("マイク切替の要求が大きすぎます".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(output).map_err(|e| e.to_string())?;
    temporary.write_all(&data).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist(output.join("microphone-selection.request"))
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

impl RecorderController {
    // Clone a dedicated notification owner under the recorder lock, then release
    // it before any delivery fsync. Slow notification storage must not hold the
    // capture/child/session mutexes needed to request recording stop.
    fn input_owner(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<
        (
            Arc<Mutex<crate::input_status::StatusReader>>,
            Arc<Mutex<Child>>,
        ),
        String,
    > {
        let controller = app.state::<Self>();
        controller.clone_input_owner(&app.state::<SessionStore>().snapshot(), session_id)
    }
    fn clone_input_owner(
        &self,
        snapshot: &RecordingSnapshot,
        session_id: &str,
    ) -> Result<
        (
            Arc<Mutex<crate::input_status::StatusReader>>,
            Arc<Mutex<Child>>,
        ),
        String,
    > {
        let active = self.0.lock().map_err(|_| "録音ロックを取得できません")?;
        let current = active.as_ref().ok_or("入力確認できる録音がありません")?;
        require_level_session(&current.session_id, snapshot, session_id)?;
        Ok((current.input_status.clone(), current.child.clone()))
    }
    fn verify_input_owner(
        app: &AppHandle,
        session_id: &str,
        reader: &Arc<Mutex<crate::input_status::StatusReader>>,
        child: &Arc<Mutex<Child>>,
    ) -> Result<(), String> {
        let controller = app.state::<Self>();
        let active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません")?;
        let current = active.as_ref().ok_or("入力確認の録音は終了しました")?;
        if !Arc::ptr_eq(reader, &current.input_status) || !Arc::ptr_eq(child, &current.child) {
            return Err("入力確認の録音所有者が変更されました".into());
        }
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("入力確認中に録音プロセスが終了しました".into());
        }
        Ok(())
    }
    pub(crate) fn input_notice_action(
        app: &AppHandle,
        session_id: &str,
        notice_id: &str,
        recipient_id: &str,
        target: &str,
        generation: Option<u64>,
        action: crate::input_status::queue::Action,
        receive_id: Option<&str>,
    ) -> Result<crate::input_status::Receipt, String> {
        if !matches!(target, "main" | "recorder")
            || notice_id.len() > 128
            || !crate::input_levels::valid_owner(session_id)
            || !crate::input_levels::valid_owner(recipient_id)
        {
            return Err("通知の受付引数が不正です".into());
        }
        app.state::<Storage>().require_data_writable()?;
        // The recording intent can end while this dedicated reader is syncing.
        // Keep companion-write admission until the actual I/O returns, without
        // holding the capture/child/session mutexes or delaying recording stop.
        let lease = app
            .state::<crate::analysis_worker::AnalysisController>()
            .begin_notice_storage()?;
        let (reader, child) = Self::input_owner(app, session_id)?;
        let mut input = reader
            .lock()
            .map_err(|_| "入力通知のロックを取得できません")?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        lease.validate_active()?;
        // Do not assign a new destination while confirming a previously offered
        // notice. Read the real current publication again before any mutation.
        let report = input.read(session_id, None)?;
        let visible = app.get_webview_window(target).is_some_and(|w| {
            crate::input_status::notice_window_visible(w.is_visible().ok(), w.is_minimized().ok())
        });
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        lease.validate_active()?;
        let result = input.queued_action(
            &report,
            notice_id,
            recipient_id,
            target,
            visible,
            generation.ok_or("通知の受付世代がありません")?,
            action,
            receive_id,
        )?;
        lease.validate_active()?;
        app.state::<Storage>().require_data_writable()?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        app.state::<SessionStore>()
            .with_active_owner(session_id, || Ok(result))
    }
    pub(crate) fn input_status(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<crate::input_status::Snapshot, String> {
        app.state::<Storage>().require_data_writable()?;
        let lease = app
            .state::<crate::analysis_worker::AnalysisController>()
            .begin_notice_storage()?;
        let (reader, child) = Self::input_owner(app, session_id)?;
        let mut input = reader
            .lock()
            .map_err(|_| "入力通知のロックを取得できません")?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        // One backend owner chooses one notification destination. Both windows
        // still read the same retained state; hiding a window never retries audio.
        let main_visible = app.get_webview_window("main").is_some_and(|w| {
            crate::input_status::notice_window_visible(w.is_visible().ok(), w.is_minimized().ok())
        });
        let recorder_visible = app.get_webview_window("recorder").is_some_and(|w| {
            crate::input_status::notice_window_visible(w.is_visible().ok(), w.is_minimized().ok())
        });
        let target = crate::input_status::notice_target(main_visible, recorder_visible);
        lease.validate_active()?;
        let result = input.read(session_id, target)?;
        lease.validate_active()?;
        app.state::<Storage>().require_data_writable()?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        app.state::<SessionStore>()
            .with_active_owner(session_id, || Ok(result))
    }
    pub(crate) fn input_history_status(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<crate::input_status::persistence::Snapshot, String> {
        app.state::<Storage>().require_data_writable()?;
        let lease = app
            .state::<crate::analysis_worker::AnalysisController>()
            .begin_notice_storage()?;
        let (reader, child) = Self::input_owner(app, session_id)?;
        let mut input = reader
            .lock()
            .map_err(|_| "入力履歴診断のロックを取得できません")?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        lease.validate_active()?;
        let result = input.persistence(session_id)?;
        lease.validate_active()?;
        app.state::<Storage>().require_data_writable()?;
        Self::verify_input_owner(app, session_id, &reader, &child)?;
        app.state::<SessionStore>()
            .with_active_owner(session_id, || Ok(result))
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
        microphone_uid: Option<&str>,
        scope: &crate::capture_scope::CaptureScope,
    ) -> Result<RecordingSnapshot, String> {
        app.state::<Storage>().require_data_writable()?;
        let inputs = Self::inputs(app)?;
        if let Some(uid) = microphone_uid {
            inputs.require_microphone(uid)?;
        } else {
            inputs.require_automatic_microphone()?;
        }
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
            backend: "coreaudio".into(),
            target_pids: scope
                .application
                .as_ref()
                .map(|a| vec![a.pid])
                .unwrap_or_default(),
            microphone_device_uid: microphone_uid.map(str::to_owned),
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
            input_status: Arc::new(Mutex::new(input_status)),
            selection_revision: 0,
        });
        let snapshot = publish_after_unlock(
            active,
            || {
                app.state::<SessionStore>().update(|state| {
                    state.phase = RecordingPhase::Preparing;
                    state.elapsed_ms = 0;
                    state.microphone_connected = false;
                    state.system_audio_connected = false;
                    state.meeting_id = Some(meeting_id.to_owned());
                    state.session_id = Some(session.id.clone());
                    state.error_message = None;
                })
            },
            |_| notify_session(app),
        );
        let worker_app = app.clone();
        let worker_session = session.id.clone();
        // Supervision starts independently of the focus-driven panel. A
        // delayed/failed UI must not leave an exited helper unobserved.
        std::thread::spawn(move || watch_probe(worker_app, child, worker_session, output));
        Ok(snapshot)
    }

    pub fn stop_session(app: &AppHandle, session: &str) -> Result<RecordingSnapshot, String> {
        Self::stop_target(app, Some(session))
    }

    pub fn select_input(
        app: &AppHandle,
        session_id: &str,
        microphone_uid: Option<&str>,
    ) -> Result<u64, String> {
        let inputs = Self::inputs(app)?;
        if let Some(uid) = microphone_uid {
            inputs.require_microphone(uid)?;
        } else {
            inputs.require_automatic_microphone()?;
        }
        let controller = app.state::<RecorderController>();
        let mut active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません".to_string())?;
        let current = active.as_mut().ok_or("切り替えられる録音がありません")?;
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if current.stop_operation_id.is_some() {
            return Err("録音停止中はマイクを切り替えられません".into());
        }
        if current
            .child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("録音ヘルパーが終了しているため切り替えられません".into());
        }
        let revision = current
            .selection_revision
            .checked_add(1)
            .filter(|revision| *revision <= 9_007_199_254_740_991)
            .ok_or("マイク切替の履歴上限に達しました")?;
        publish_microphone_selection(
            &current.level_directory,
            session_id,
            revision,
            microphone_uid,
        )?;
        current.selection_revision = revision;
        Ok(revision)
    }

    pub fn selection_status(
        app: &AppHandle,
        session_id: &str,
    ) -> Result<MicrophoneSelectionStatus, String> {
        let controller = app.state::<RecorderController>();
        let (directory, child) = {
            let active = controller
                .0
                .lock()
                .map_err(|_| "録音ロックを取得できません".to_string())?;
            let current = active.as_ref().ok_or("確認できる録音がありません")?;
            require_level_session(
                &current.session_id,
                &app.state::<SessionStore>().snapshot(),
                session_id,
            )?;
            (current.level_directory.clone(), current.child.clone())
        };
        let parent = crate::input_levels::open_capture_directory(&directory)?;
        let file =
            crate::input_levels::open_capture_file_at(&parent, "microphone-selection.status.json")?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > 4096 {
            return Err("マイク選択状態のファイルが不正です".into());
        }
        #[cfg(unix)]
        if std::os::unix::fs::MetadataExt::nlink(&metadata) != 1 {
            return Err("マイク選択状態のファイルの所有が不正です".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 != metadata.len() {
            return Err("マイク選択状態が読込み中に変更されました".into());
        }
        let status = parse_microphone_selection_status(&bytes, session_id)?;
        let active = controller
            .0
            .lock()
            .map_err(|_| "録音ロックを取得できません".to_string())?;
        let current = active.as_ref().ok_or("録音が終了しました")?;
        if !Arc::ptr_eq(&current.child, &child) {
            return Err("録音ヘルパーが変更されました".into());
        }
        require_level_session(
            &current.session_id,
            &app.state::<SessionStore>().snapshot(),
            session_id,
        )?;
        if child
            .lock()
            .map_err(|_| "録音プロセスのロックに失敗しました")?
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("録音ヘルパーが終了しました".into());
        }
        Ok(status)
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
        let snapshot = publish_after_unlock(
            active,
            || {
                app.state::<SessionStore>()
                    .update(|state| state.phase = RecordingPhase::Stopping)
            },
            |_| notify_session(app),
        );
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
    #[test]
    fn blocked_start_notification_does_not_block_concurrent_poll_or_stop() {
        let capture = Arc::new(Mutex::new(()));
        let store = Arc::new(SessionStore::default());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let worker_capture = capture.clone();
        let worker_store = store.clone();
        let worker = std::thread::spawn(move || {
            publish_after_unlock(
                worker_capture.lock().unwrap(),
                || {
                    worker_store.update(|s| {
                        s.session_id = Some(SESSION.into());
                        s.phase = RecordingPhase::Preparing;
                    })
                },
                |_| {
                    ready_tx.send(()).unwrap();
                    resume_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    assert_eq!(worker_store.snapshot().phase, RecordingPhase::Stopping);
                    assert!(!mark_started(&worker_store, SESSION));
                },
            );
        });
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let guard = capture
            .try_lock()
            .expect("UI dispatch retained capture lock");
        require_level_session(SESSION, &store.snapshot(), SESSION).unwrap();
        publish_after_unlock(
            guard,
            || store.update(|s| s.phase = RecordingPhase::Stopping),
            |_| {},
        );
        resume_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(store.snapshot().phase, RecordingPhase::Stopping);
    }
    #[test]
    fn start_notification_allows_immediate_capture_poll_and_stop() {
        let capture = Mutex::new(());
        let store = SessionStore::default();
        let guard = capture.lock().unwrap();
        let started = publish_after_unlock(
            guard,
            || {
                assert!(capture.try_lock().is_err());
                store.update(|s| {
                    s.session_id = Some(SESSION.into());
                    s.phase = RecordingPhase::Preparing;
                })
            },
            |snapshot| {
                // This callback stands at the native-dispatch boundary. Both poll
                // and stop must acquire the capture lock without waiting on UI.
                let _poll = capture
                    .try_lock()
                    .expect("capture held during notification");
                require_level_session(SESSION, &store.snapshot(), SESSION).unwrap();
                assert_eq!(snapshot.session_id.as_deref(), Some(SESSION));
                store.update(|s| s.phase = RecordingPhase::Stopping);
            },
        );
        assert_eq!(started.phase, RecordingPhase::Preparing);
        assert_eq!(store.snapshot().phase, RecordingPhase::Stopping);
        assert!(!mark_started(&store, SESSION));
    }

    #[test]
    fn delayed_start_observation_cannot_overwrite_stop_or_another_owner() {
        let store = SessionStore::default();
        store.update(|s| {
            s.session_id = Some(SESSION.into());
            s.phase = RecordingPhase::Preparing;
        });
        assert!(mark_started(&store, SESSION));
        assert!(!mark_started(&store, SESSION));
        store.update(|s| s.phase = RecordingPhase::Stopping);
        let stopped = store.snapshot();
        assert!(!mark_started(&store, SESSION));
        assert_eq!(store.snapshot(), stopped);
        store.update(|s| {
            s.session_id = Some("new-owner".into());
            s.phase = RecordingPhase::Preparing;
        });
        assert!(!mark_started(&store, SESSION));
        for phase in [
            RecordingPhase::Idle,
            RecordingPhase::Error,
            RecordingPhase::Saving,
        ] {
            store.update(|s| {
                s.session_id = Some(SESSION.into());
                s.phase = phase;
            });
        }
    }
    #[test]
    #[cfg(unix)]
    fn segmented_manifest_monitor_preserves_large_state_and_bounded_error() {
        let root = tempfile::tempdir().unwrap();
        let status = |lane| {
            serde_json::json!({"schemaVersion":2,"ownerId":SESSION,"lane":lane,
            "sealedSegments":0,"sealedFrames":0,"lastEntrySHA256":null,"producerClosed":false,"failure":null})
        };
        let mut value = serde_json::json!({"schemaVersion":2,"recordingOwnerId":SESSION,"state":"failed",
            "errors":["storage failed"],"microphone":{"rawSegments":status("microphone")},
            "system":{"rawSegments":status("system")},"padding":"x".repeat(256*1024)});
        std::fs::write(root.path().join("manifest.json"), value.to_string()).unwrap();
        assert_eq!(manifest_state(root.path()).as_deref(), Some("failed"));
        assert_eq!(
            manifest_error(root.path()).as_deref(),
            Some("storage failed")
        );
        value.as_object_mut().unwrap().remove("recordingOwnerId");
        std::fs::write(root.path().join("manifest.json"), value.to_string()).unwrap();
        assert!(manifest_state(root.path()).is_none());
        assert!(manifest_error(root.path()).is_none());
    }

    #[test]
    fn notification_owner_does_not_hold_capture_or_child_lock_while_reader_is_busy() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().canonicalize().unwrap();
        // A reaped no-op process, not a recording helper or product app.
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        child.wait().unwrap();
        let child = Arc::new(Mutex::new(child));
        let reader = Arc::new(Mutex::new(
            crate::input_status::StatusReader::pin(&output).unwrap(),
        ));
        let controller = RecorderController(
            Mutex::new(Some(ActiveCapture {
                session_id: SESSION.into(),
                output: output.clone(),
                child: child.clone(),
                stop_operation_id: None,
                level_directory: output.clone(),
                levels: Default::default(),
                writes: crate::recording_writes::WriteReader::pin(&output).unwrap(),
                input_status: reader.clone(),
                selection_revision: 0,
            })),
            Mutex::new(()),
        );
        let mut snapshot = RecordingSnapshot {
            session_id: Some(SESSION.into()),
            phase: RecordingPhase::Recording,
            ..Default::default()
        };
        let (owned_reader, owned_child) = controller.clone_input_owner(&snapshot, SESSION).unwrap();
        assert!(Arc::ptr_eq(&reader, &owned_reader));
        assert!(Arc::ptr_eq(&child, &owned_child));
        let _busy_notification_storage = owned_reader.lock().unwrap();
        // These are the exact locks used by stop_target and child termination.
        assert!(controller.0.try_lock().is_ok());
        assert!(owned_child.try_lock().is_ok());
        snapshot.phase = RecordingPhase::Stopping;
        assert!(controller.clone_input_owner(&snapshot, SESSION).is_err());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    }

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
                Some("usb"),
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
            Some("USB input/名前 with spaces"),
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
            "coreaudio",
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
    fn automatic_microphone_command_omits_fixed_uid_but_keeps_owner() {
        let scope = crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: None,
        };
        let command = recording_command(
            Path::new("/tmp/probe"),
            Path::new("/tmp/output"),
            1234,
            None,
            &scope,
            SESSION,
        )
        .unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(!args.contains(&"--microphone-uid"));
        assert_eq!(
            args[args
                .iter()
                .position(|arg| *arg == "--input-level-owner")
                .unwrap()
                + 1],
            SESSION
        );
    }

    #[test]
    fn microphone_selection_request_replaces_only_owned_bounded_policy() {
        let directory = tempfile::tempdir().unwrap();
        publish_microphone_selection(directory.path(), SESSION, 1, Some("usb-123")).unwrap();
        let file = directory.path().join("microphone-selection.request");
        let first: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(first["ownerId"], SESSION);
        assert_eq!(first["mode"], "fixed");
        assert_eq!(first["uid"], "usb-123");
        publish_microphone_selection(directory.path(), SESSION, 2, None).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(second["revision"], 2);
        assert_eq!(second["mode"], "automatic");
        assert!(second["uid"].is_null());
        assert!(publish_microphone_selection(directory.path(), "../wrong", 3, None).is_err());
        assert!(publish_microphone_selection(directory.path(), SESSION, 0, None).is_err());
        assert!(
            publish_microphone_selection(directory.path(), SESSION, 3, Some("bad\nuid")).is_err()
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(file).unwrap()).unwrap(),
            second
        );
    }

    #[test]
    fn microphone_selection_status_rejects_wrong_owner_or_unbounded_source() {
        let valid = serde_json::json!({
            "schemaVersion":1,"ownerId":SESSION,"revision":1,"requestRevision":0,
            "mode":"automatic","deviceUID":"built-in","source":"systemDefault",
            "fallbackReason":"application_input_unavailable"
        });
        assert_eq!(
            parse_microphone_selection_status(&serde_json::to_vec(&valid).unwrap(), SESSION)
                .unwrap()
                .fallback_reason
                .as_deref(),
            Some("application_input_unavailable")
        );
        let mut pending = valid.clone();
        pending["mode"] = serde_json::json!("fixed");
        pending["source"] = serde_json::json!("pending");
        pending["fallbackReason"] = serde_json::Value::Null;
        assert!(
            parse_microphone_selection_status(&serde_json::to_vec(&pending).unwrap(), SESSION)
                .is_ok()
        );
        for (field, invalid) in [
            ("ownerId", serde_json::json!("../other")),
            ("revision", serde_json::json!(0)),
            ("source", serde_json::json!("anyDevice")),
            ("fallbackReason", serde_json::json!("arbitrary user text")),
            ("deviceUID", serde_json::json!("bad\nuid")),
        ] {
            let mut value = valid.clone();
            value[field] = invalid;
            assert!(parse_microphone_selection_status(
                &serde_json::to_vec(&value).unwrap(),
                SESSION
            )
            .is_err());
        }
    }

    #[test]
    #[ignore = "Swift-generated fixture; run with MON_MIC_SELECTION_FIXTURE_DIR"]
    fn swift_generated_microphone_selection_status_matches_native_reader() {
        let directory = std::path::PathBuf::from(
            std::env::var("MON_MIC_SELECTION_FIXTURE_DIR").expect("Swift fixture directory"),
        );
        let bytes = std::fs::read(directory.join("microphone-selection.status.json")).unwrap();
        let result = parse_microphone_selection_status(&bytes, SESSION).unwrap();
        assert_eq!(result.revision, 1);
        assert_eq!(result.mode, "automatic");
        assert_eq!(result.source, "systemDefault");
        assert_eq!(result.device_uid, "built-in");
        assert_eq!(
            result.fallback_reason.as_deref(),
            Some("application_input_unavailable")
        );
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
            Some("usb"),
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
            Some("usb"),
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
    crate::internal_logs::phase(
        "recording",
        snapshot.session_id.clone(),
        snapshot.meeting_id.clone(),
        &format!("{:?}", snapshot.phase).to_lowercase(),
        snapshot.error_message.as_deref(),
    );
    notify_session(app);
    snapshot
}

fn notify_session(app: &AppHandle) {
    let handle = app.clone();
    // Never synchronously wait for UI work while holding the lifecycle lock.
    // Read the latest revision when delivered, so delayed work cannot restore
    // a preparing/recording state over an already stopped or newer session.
    let _ = app.run_on_main_thread(move || {
        let snapshot = handle.state::<SessionStore>().snapshot();
        crate::internal_logs::phase(
            "recording",
            snapshot.session_id.clone(),
            snapshot.meeting_id.clone(),
            &format!("{:?}", snapshot.phase).to_lowercase(),
            snapshot.error_message.as_deref(),
        );
        // Close the floating panel before publishing a stop/saving result to
        // the rest of the UI. A delayed start notification reads this latest
        // phase too, so it cannot reopen the panel after stop.
        let _ = crate::recorder_panel::synchronize(&handle, None);
        crate::recording_confirmation_window::session_changed(&handle, &snapshot);
        let _ = handle.emit("session-changed", snapshot);
    });
}

fn manifest_state(output: &Path) -> Option<String> {
    let manifest = manifest_bytes(output)?;
    let value: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    value.get("state")?.as_str().map(str::to_owned)
}

fn manifest_error(output: &Path) -> Option<String> {
    let bytes = manifest_bytes(output)?;
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

fn manifest_bytes(output: &Path) -> Option<Vec<u8>> {
    let file = File::open(output.join("manifest.json")).ok()?;
    if file.metadata().ok()?.len() > crate::capture_manifest::MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(crate::capture_manifest::MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    crate::capture_manifest::validate_size(&bytes).ok()?;
    Some(bytes)
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
                    if mark_started(&app.state::<SessionStore>(), &session_id) {
                        notify_session(&app);
                    }
                }
            } else if state.phase == RecordingPhase::Stopping {
                announced = true;
            }
        }
        let status = RecorderController::input_status(&app, &session_id).ok();
        let connected = status.as_ref().is_some_and(|status| status.connected());
        let system_connected = status
            .as_ref()
            .is_some_and(|status| status.system_connected());
        if app
            .state::<SessionStore>()
            .update_input_connections(&session_id, connected, system_connected)
            .is_some()
        {
            notify_session(&app);
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
            let completed_meeting = app.state::<SessionStore>().snapshot().meeting_id;
            publish(&app, |snapshot| {
                snapshot.phase = RecordingPhase::Idle;
                snapshot.elapsed_ms = 0;
                snapshot.microphone_connected = false;
                snapshot.system_audio_connected = false;
                snapshot.meeting_id = None;
                snapshot.session_id = None;
                snapshot.error_message = None;
            });
            if let Some(meeting) = completed_meeting {
                crate::playback::mix_cache::warm(&app, &meeting);
            }
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
