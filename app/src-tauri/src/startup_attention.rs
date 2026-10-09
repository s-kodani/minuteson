//! Fail-closed guidance for storage admission failures, not a repair facility.
//! Reports only read process memory; they never reopen a rejected storage root.
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use tauri::{Manager, State, WebviewWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Directories,
    StorageSelection,
    StorageAdmission,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Failure {
    stage: Stage,
    message: String,
    message_truncated: bool,
    meeting_root: Option<String>,
    config_root: Option<String>,
}
impl Failure {
    pub(crate) fn new(
        stage: Stage,
        message: String,
        root: Option<&Path>,
        config: Option<&Path>,
    ) -> Self {
        let message = message.replace('\0', "\\0");
        let mut end = message.len().min(16_384);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let path = |p: &Path| {
            let text = p.to_string_lossy();
            (p.is_absolute() && text.len() <= 8192 && !text.contains('\0'))
                .then(|| text.into_owned())
        };
        Self {
            stage,
            message_truncated: end < message.len(),
            message: message[..end].to_owned(),
            meeting_root: root.and_then(path),
            config_root: config.and_then(path),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Starting,
    Ready,
    Blocked,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    schema_version: u8,
    phase: Phase,
    failure: Option<Failure>,
}
pub(crate) struct Startup(Mutex<Report>);
impl Default for Startup {
    fn default() -> Self {
        Self(Mutex::new(Report {
            schema_version: 1,
            phase: Phase::Starting,
            failure: None,
        }))
    }
}
impl Startup {
    pub(crate) fn snapshot(&self) -> Result<Report, String> {
        self.0
            .lock()
            .map(|s| s.clone())
            .map_err(|_| "起動診断のロックを取得できません".into())
    }
    fn finish(&self, failure: Option<Failure>) -> Result<(), String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "起動診断のロックを取得できません")?;
        if state.phase != Phase::Starting {
            return Err("起動診断の結果は上書きできません".into());
        }
        state.phase = if failure.is_some() {
            Phase::Blocked
        } else {
            Phase::Ready
        };
        state.failure = failure;
        Ok(())
    }
    pub(crate) fn ready(&self) -> Result<(), String> {
        self.finish(None)
    }
    pub(crate) fn block(&self, failure: Failure) -> Result<(), String> {
        self.finish(Some(failure))
    }
    pub(crate) fn blocked(&self) -> bool {
        self.snapshot().map_or(true, |s| s.phase == Phase::Blocked)
    }
    pub(crate) fn admit_command(&self, label: &str, command: &str) -> Result<(), String> {
        let report = self.snapshot()?;
        if command == "get_startup_report" {
            return if label == "main" {
                Ok(())
            } else {
                Err("起動診断はメイン画面だけで確認できます".into())
            };
        }
        if command == "quit_blocked_startup" {
            return if label == "main" && report.phase == Phase::Blocked {
                Ok(())
            } else {
                Err("起動拒否の終了操作は対象のメイン画面だけで行えます".into())
            };
        }
        if report.phase == Phase::Ready {
            Ok(())
        } else {
            Err("保存先の起動確認が完了していません。通常操作は開始しません。起動診断を確認してください".into())
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn admit_storage(
    config: &Path,
    root: &Path,
) -> Result<(crate::storage::Storage, crate::bootstrap::Bootstrap), Failure> {
    let selection = crate::bootstrap::Bootstrap::open(config, root)
        .map_err(|e| Failure::new(Stage::StorageSelection, e, None, Some(config)))?;
    let storage = selection.open_storage().map_err(|e| {
        Failure::new(
            Stage::StorageAdmission,
            e,
            Some(selection.root()),
            Some(config),
        )
    })?;
    Ok((storage, selection))
}

pub(crate) fn show(app: &tauri::AppHandle) {
    for label in ["recorder", "recorder-info", "recording-confirmation"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.hide();
        }
    }
    if let Some(main) = app.get_webview_window("main") {
        // The main entry reads this report before mounting any workspace
        // provider; do not depend on the WebView's early navigation state.
        let _ = main.show();
    }
}
#[tauri::command]
pub(crate) fn get_startup_report(
    window: WebviewWindow,
    startup: State<'_, Startup>,
) -> Result<Report, String> {
    startup.admit_command(window.label(), "get_startup_report")?;
    startup.snapshot()
}
#[tauri::command]
pub(crate) fn quit_blocked_startup(
    window: WebviewWindow,
    startup: State<'_, Startup>,
) -> Result<(), String> {
    startup.admit_command(window.label(), "quit_blocked_startup")?;
    window.app_handle().exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn starting_and_blocked_reject_every_normal_operation_and_never_reset() {
        let s = Startup::default();
        for label in [
            "main",
            "recorder",
            "recorder-info",
            "recording-confirmation",
            "unknown",
        ] {
            for command in [
                "create_meeting",
                "start_recording",
                "stop_recording",
                "start_data_operation",
                "get_recording_snapshot",
                "show_recorder_info",
                "unknown",
            ] {
                assert!(s.admit_command(label, command).is_err());
            }
        }
        s.block(Failure::new(
            Stage::StorageAdmission,
            "元診断".into(),
            Some(Path::new("/a")),
            None,
        ))
        .unwrap();
        let before = s.snapshot().unwrap();
        assert!(s.ready().is_err());
        assert!(s
            .block(Failure::new(Stage::Directories, "new".into(), None, None))
            .is_err());
        assert_eq!(s.snapshot().unwrap(), before);
        assert!(s.admit_command("main", "get_startup_report").is_ok());
        assert!(s.admit_command("main", "quit_blocked_startup").is_ok());
        for label in [
            "recorder",
            "recorder-info",
            "recording-confirmation",
            "unknown",
        ] {
            assert!(s.admit_command(label, "quit_blocked_startup").is_err());
            assert!(s.admit_command(label, "get_startup_report").is_err());
        }
        assert!(s.admit_command("main", "get_recording_snapshot").is_err());
    }
    #[test]
    fn ready_preserves_normal_dispatch_but_cannot_use_failure_exit() {
        let s = Startup::default();
        assert!(s.admit_command("main", "quit_blocked_startup").is_err());
        s.ready().unwrap();
        assert!(s.admit_command("main", "create_meeting").is_ok());
        assert!(s.admit_command("recorder", "stop_recording").is_ok());
        assert!(s.admit_command("main", "quit_blocked_startup").is_err());
    }
    #[test]
    fn diagnosis_is_bounded_on_utf8_and_keeps_stage_without_guessing_cause() {
        let f = Failure::new(
            Stage::StorageSelection,
            format!("{}\0", "故障".repeat(6000)),
            Some(Path::new("relative")),
            Some(Path::new("/config")),
        );
        assert!(f.message.len() <= 16_384 && f.message_truncated);
        assert!(f.meeting_root.is_none());
        let v = serde_json::to_value(f).unwrap();
        assert_eq!(v["stage"], "storage_selection");
        assert_eq!(v["configRoot"], "/config");
        assert!(v.get("messageTruncated").is_some());
        assert!(v.get("message_truncated").is_none());
    }
    #[test]
    fn report_wire_is_exact_and_read_access_stays_main_only_when_ready() {
        let s = Startup::default();
        s.ready().unwrap();
        assert_eq!(
            serde_json::to_value(s.snapshot().unwrap()).unwrap(),
            serde_json::json!({"schemaVersion":1,"phase":"ready","failure":null})
        );
        assert!(s.admit_command("recorder", "get_startup_report").is_err());
    }
    #[test]
    fn poisoned_diagnostic_lock_denies_commands_but_keeps_os_exit_path() {
        let s = Startup::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = s.0.lock().unwrap();
            panic!("synthetic diagnostic lock failure");
        }));
        assert!(s.snapshot().is_err());
        assert!(s.admit_command("main", "create_meeting").is_err());
        assert!(s.admit_command("main", "get_startup_report").is_err());
        assert!(s.blocked());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn invalid_bootstrap_retains_selection_record_and_never_opens_a_fallback_root() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        std::fs::create_dir(&config).unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700)).unwrap();
        let record = config.join("bootstrap.json");
        std::fs::write(&record, b"invalid synthetic bootstrap").unwrap();
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o600)).unwrap();
        let root = temp.path().join("must-not-be-opened");
        let failure = match admit_storage(&config, &root) {
            Err(f) => f,
            Ok(_) => panic!("invalid selection admitted"),
        };
        assert_eq!(failure.stage, Stage::StorageSelection);
        assert!(failure.meeting_root.is_none());
        assert!(!root.exists());
        assert_eq!(
            std::fs::read(record).unwrap(),
            b"invalid synthetic bootstrap"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn real_storage_refusal_retains_existing_meeting_audio_and_bad_ledger() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let root = temp.path().join("data");
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        let selected = storage.root().to_owned();
        let meeting = storage.create_meeting("既存会議").unwrap();
        let marker = selected.join("existing-audio.flac");
        std::fs::write(&marker, b"retained synthetic audio").unwrap();
        drop(storage);
        drop(bootstrap);
        let journal = selected.join(".mon-scratch.sqlite3");
        let connection = rusqlite::Connection::open(&journal).unwrap();
        connection
            .execute_batch("CREATE TABLE unknown_object(value TEXT);")
            .unwrap();
        drop(connection);
        let before = std::fs::read(&journal).unwrap();
        let failure = match admit_storage(&config, &root) {
            Err(f) => f,
            Ok(_) => panic!("invalid ledger admitted"),
        };
        assert_eq!(failure.stage, Stage::StorageAdmission);
        assert_eq!(failure.meeting_root.as_deref(), selected.to_str());
        assert!(failure.message.contains("台帳"));
        let s = Startup::default();
        s.block(failure).unwrap();
        for _ in 0..3 {
            assert_eq!(s.snapshot().unwrap().phase, Phase::Blocked);
            assert!(s.admit_command("main", "start_data_operation").is_err());
        }
        assert_eq!(std::fs::read(&journal).unwrap(), before);
        assert_eq!(std::fs::read(marker).unwrap(), b"retained synthetic audio");
        let db = rusqlite::Connection::open(selected.join("meetings.sqlite3")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT title FROM meetings WHERE id=?1",
                [&meeting.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "既存会議"
        );
    }
}
