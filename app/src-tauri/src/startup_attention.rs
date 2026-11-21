//! Immutable diagnostics for storage admission failures. Reset uses separate
//! native IPC and progress, without reopening storage through report reads.
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
    PartialSalvage,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Failure {
    stage: Stage,
    salvage_available: bool,
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
            salvage_available: stage == Stage::PartialSalvage,
            message_truncated: end < message.len(),
            message: message[..end].to_owned(),
            meeting_root: root.and_then(path),
            config_root: config.and_then(path),
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    fn from_storage(
        stage: Stage,
        error: crate::bootstrap::StartupStorageError,
        root: Option<&Path>,
        config: &Path,
    ) -> Self {
        if matches!(error, crate::bootstrap::StartupStorageError::PendingSalvage) {
            return Self::new(Stage::PartialSalvage, error.into(), root, Some(config));
        }
        let salvage_available = error.salvage_available();
        let mut failure = Self::new(stage, error.into(), root, Some(config));
        failure.salvage_available = salvage_available;
        failure
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
pub(crate) struct Recovery {
    pub kind: String,
    pub review_required: bool,
    pub reasons: Vec<String>,
    pub record_path: std::path::PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    schema_version: u8,
    platform: &'static str,
    phase: Phase,
    failure: Option<Failure>,
    #[serde(skip)]
    common_write_hold: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<Recovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset: Option<crate::startup_reset::Availability>,
}
pub(crate) struct Startup(Mutex<Report>);
impl Default for Startup {
    fn default() -> Self {
        Self(Mutex::new(Report {
            schema_version: 1,
            platform: if cfg!(windows) {
                "windows"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else {
                "other"
            },
            phase: Phase::Starting,
            failure: None,
            common_write_hold: false,
            recovery: None,
            reset: None,
        }))
    }
}
impl Startup {
    pub(crate) fn ready_for_work(&self) -> bool {
        self.snapshot().is_ok_and(|s| s.phase == Phase::Ready)
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn set_recovery(&self, recovery: Option<Recovery>) -> Result<(), String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "起動診断のロックを取得できません")?;
        if state.phase != Phase::Starting {
            return Err("起動結果は確定済みです".into());
        }
        state.recovery = recovery;
        Ok(())
    }
    pub(crate) fn set_common_write_hold(&self, hold: bool) -> Result<(), String> {
        let mut report = self
            .0
            .lock()
            .map_err(|_| "起動診断のロックを取得できません")?;
        if report.phase != Phase::Starting {
            return Err("起動結果は確定済みです".into());
        }
        report.common_write_hold = hold;
        Ok(())
    }
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
        crate::internal_logs::report_error("application", &format!("{failure:?}"));
        self.finish(Some(failure))
    }
    pub(crate) fn blocked(&self) -> bool {
        self.snapshot().map_or(true, |s| s.phase == Phase::Blocked)
    }
    pub(crate) fn failure_paths(
        &self,
    ) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>), String> {
        let report = self.snapshot()?;
        let failure = report.failure.ok_or("起動診断を取得できません")?;
        Ok((
            failure.meeting_root.map(Into::into),
            failure.config_root.map(Into::into),
        ))
    }

    pub(crate) fn admit_command(&self, label: &str, command: &str) -> Result<(), String> {
        if command == "get_window_environment" {
            return label
                .parse::<mon_contracts::WindowRole>()
                .map(|_| ())
                .map_err(|_| "不明な画面です".into());
        }
        // The Windows title bar also runs while storage admission is pending.
        if command == "record_background_read_failure" {
            return if label == "main" {
                Ok(())
            } else {
                Err("自動確認の診断ログはメイン画面だけで記録できます".into())
            };
        }
        let report = self.snapshot()?;
        if matches!(command, "get_residency_state" | "hide_main_to_background") {
            return if label == "main" {
                Ok(())
            } else {
                Err("常駐操作はメイン画面だけで利用できます".into())
            };
        }
        if command == "get_startup_report" {
            return if label == "main" {
                Ok(())
            } else {
                Err("起動診断はメイン画面だけで確認できます".into())
            };
        }
        if matches!(
            command,
            "get_salvage_state"
                | "recheck_salvage_environment"
                | "open_salvage"
                | "choose_salvage_folder"
                | "inspect_salvage"
                | "commit_salvage"
                | "cancel_salvage_work"
                | "leave_salvage"
        ) {
            return if label == "main"
                && report.phase == Phase::Blocked
                && report
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.salvage_available)
            {
                Ok(())
            } else {
                Err("現在の状態ではデータ復元を開始できません".into())
            };
        }
        if matches!(command, "get_salvage_result" | "get_salvage_source_cleanup") {
            return if label == "main" && report.phase == Phase::Ready {
                Ok(())
            } else {
                Err("起動後のメイン画面だけで復元結果を確認できます".into())
            };
        }
        if command == "export_salvage_edits" {
            return if label == "main" && report.phase == Phase::Ready {
                Ok(())
            } else {
                Err("起動後のメイン画面だけで未保存の入力を退避できます".into())
            };
        }
        if command == "delete_salvage_source" && (label != "main" || report.phase != Phase::Ready) {
            return Err("復元先で起動したメイン画面だけで元データを削除できます".into());
        }
        if report
            .failure
            .as_ref()
            .is_some_and(|f| f.stage == Stage::PartialSalvage)
            && matches!(
                command,
                "get_startup_reset_preview"
                    | "choose_startup_reset_backup_directory"
                    | "start_startup_reset"
                    | "get_startup_reset_state"
            )
        {
            return Err("データ復元を終了してから初期化を確認してください".into());
        }
        if matches!(
            command,
            "retry_startup"
                | "get_startup_reset_preview"
                | "choose_startup_reset_backup_directory"
                | "start_startup_reset"
                | "get_startup_reset_state"
        ) {
            return if label == "main" && report.phase == Phase::Blocked {
                Ok(())
            } else {
                Err("起動復旧の画面だけで操作できます".into())
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
            if (report.common_write_hold
                || report.recovery.as_ref().is_some_and(|r| r.review_required))
                && !matches!(
                    command,
                    "get_internal_logs"
                        | "get_recording_recovery"
                        | "get_storage_bootstrap"
                        | "list_meetings"
                        | "get_meeting_list_diagnostics"
                        | "get_storage_write_state"
                        | "get_meeting"
                        | "list_restored_meeting_ids"
                        | "get_current_analysis"
                        | "get_transcript_draft"
                        | "get_minutes_markdown"
                        | "get_meeting_body_for_review"
                        | "get_minutes_source"
                        | "get_meeting_audio_summary"
                        | "get_data_operation"
                        | "get_data_operation_availability"
                        | "get_deletion_items"
                        | "get_meeting_deletion_availability"
                        | "get_audio_deletion_items"
                        | "get_audio_deletion_paths"
                        | "get_model_deletion_items"
                        | "get_audio_import"
                        | "get_recording_snapshot"
                        | "get_capture_test_snapshot"
                        | "get_analysis_flow"
                        | "get_analysis_jobs"
                        | "get_analysis_notification_jobs"
                        | "get_analysis_notification_output"
                        | "get_ready_analysis_flows"
                        | "get_analysis_preparations"
                        | "get_meeting_analysis_audio"
                        | "get_ui_preferences"
                        | "get_model_download_guidance"
                        | "list_job_history"
                        | "get_asr_preferences"
                        | "get_asr_backends"
                        | "get_diarization_preferences"
                        | "get_minutes_preferences"
                        | "get_minutes_prompt_preferences"
                        | "get_voice_matching_preferences"
                        | "get_speaker_completion_preferences"
                        | "get_voice_people"
                        | "get_voice_context"
                        | "get_vocabulary_book"
                        | "get_meeting_vocabulary"
                        | "get_model_profiles"
                        | "get_model_placements"
                        | "get_recording_microphone_selection"
                        | "get_application_close_activity"
                        | "finish_main_close"
                        | "open_main_window"
                )
            {
                return Err("保存先の確認が必要です。現在は会議の閲覧のみ利用できます".into());
            }
            Ok(())
        } else {
            Err("まだアプリを開ける状態か確認できていません。起動時の案内を確認してください".into())
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
pub(crate) fn admit_storage(
    config: &Path,
    root: &Path,
) -> Result<(crate::storage::Storage, crate::bootstrap::Bootstrap), Failure> {
    crate::bootstrap::reset::recover(config, root)
        .map_err(|e| Failure::new(Stage::StorageSelection, e, None, Some(config)))?;
    let selection = crate::bootstrap::Bootstrap::open_for_startup(config, root)
        .map_err(|e| Failure::from_storage(Stage::StorageSelection, e, None, config))?;
    let storage = selection.admit_storage().map_err(|e| {
        Failure::from_storage(
            Stage::StorageAdmission,
            selection.with_salvage_issue(e),
            Some(selection.root()),
            config,
        )
    })?;
    selection.retain_invalid_salvage_journal();
    Ok((storage, selection))
}

pub(crate) fn show(app: &tauri::AppHandle) {
    let dedicated_salvage = app.state::<Startup>().snapshot().is_ok_and(|report| {
        report
            .failure
            .is_some_and(|f| f.stage == Stage::PartialSalvage)
    });
    if !dedicated_salvage {
        let availability = crate::startup_reset::availability(app);
        if let Ok(mut report) = app.state::<Startup>().0.lock() {
            report.reset = Some(availability);
        }
    }
    for label in ["recorder", "recording-confirmation"] {
        if let Some(window) = app.get_webview_window(label) {
            if label == "recorder" {
                let _ = crate::recorder_panel::hide(&window);
            } else {
                let _ = window.hide();
            }
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
) -> Result<Report, mon_contracts::AppError> {
    let result: Result<Report, String> = (|| {
        startup.admit_command(window.label(), "get_startup_report")?;
        startup.snapshot()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_startup_report", message))
}
#[tauri::command]
pub(crate) fn quit_blocked_startup(
    window: WebviewWindow,
    startup: State<'_, Startup>,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        startup.admit_command(window.label(), "quit_blocked_startup")?;
        crate::startup_reset::check_idle(window.app_handle())?;
        crate::startup_salvage::require_idle(window.app_handle())?;
        window.app_handle().exit(0);
        Ok(())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("quit_blocked_startup", message))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedicated_salvage_excludes_ordinary_storage_and_startup_reset_commands() {
        let startup = Startup::default();
        assert!(startup.admit_command("main", "open_salvage").is_err());
        startup
            .block(Failure::new(
                Stage::PartialSalvage,
                "救出".into(),
                None,
                None,
            ))
            .unwrap();
        for command in [
            "open_salvage",
            "commit_salvage",
            "get_salvage_state",
            "leave_salvage",
        ] {
            assert!(startup.admit_command("main", command).is_ok());
            assert!(startup.admit_command("recorder", command).is_err());
        }
        for command in [
            "start_startup_reset",
            "get_startup_reset_preview",
            "get_salvage_result",
            "get_salvage_source_cleanup",
            "delete_salvage_source",
            "list_meetings",
            "create_meeting",
        ] {
            assert!(startup.admit_command("main", command).is_err());
        }
    }

    #[test]
    fn ready_main_can_review_source_cleanup_while_shared_write_hold_prevents_deletion() {
        let startup = Startup::default();
        startup.ready().unwrap();
        assert!(startup
            .admit_command("main", "get_salvage_source_cleanup")
            .is_ok());
        assert!(startup
            .admit_command("main", "delete_salvage_source")
            .is_ok());
        assert!(startup
            .admit_command("recorder", "delete_salvage_source")
            .is_err());
        let read_only = Startup::default();
        read_only.set_common_write_hold(true).unwrap();
        read_only.ready().unwrap();
        assert!(read_only
            .admit_command("main", "get_salvage_source_cleanup")
            .is_ok());
        assert!(read_only
            .admit_command("main", "delete_salvage_source")
            .is_err());
    }

    #[test]
    fn recovery_commands_are_main_only_and_never_unlock_normal_storage_commands() {
        let startup = Startup::default();
        assert!(startup.admit_command("main", "retry_startup").is_err());
        startup
            .block(Failure::new(
                Stage::StorageSelection,
                "busy".into(),
                None,
                None,
            ))
            .unwrap();
        assert!(startup.admit_command("main", "retry_startup").is_ok());
        for label in ["recorder", "recording-confirmation", "unknown"] {
            assert!(startup.admit_command(label, "retry_startup").is_err());
        }
        for command in ["start_recording", "list_meetings", "finish_main_close"] {
            assert!(startup.admit_command("main", command).is_err());
        }
        for command in [
            "open_salvage",
            "recheck_salvage_environment",
            "commit_salvage",
        ] {
            assert!(startup.admit_command("main", command).is_err());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn recovered_pending_startup_allows_reading_but_never_dispatches_mutations() {
        let s = Startup::default();
        s.set_recovery(Some(Recovery {
            kind: "device_changed".into(),
            review_required: true,
            reasons: vec!["保存先移動が未完了です".into()],
            record_path: "/config/previous-bootstrap.json".into(),
        }))
        .unwrap();
        s.ready().unwrap();
        for command in [
            "list_meetings",
            "get_meeting",
            "get_minutes_markdown",
            "get_startup_report",
            "finish_main_close",
        ] {
            assert!(s.admit_command("main", command).is_ok(), "{command}");
        }
        for command in [
            "start_recording",
            "start_data_operation",
            "rename_meeting",
            "save_minutes_markdown",
            "start_capture_test",
        ] {
            assert!(s.admit_command("main", command).is_err(), "{command}");
        }
        assert!(s.snapshot().unwrap().recovery.unwrap().review_required);
    }
    #[test]
    fn starting_and_blocked_reject_every_normal_operation_and_never_reset() {
        let s = Startup::default();
        for label in ["main", "recorder", "recording-confirmation", "unknown"] {
            for command in [
                "create_meeting",
                "start_recording",
                "stop_recording",
                "start_data_operation",
                "get_recording_snapshot",
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
        for label in ["recorder", "recording-confirmation", "unknown"] {
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
            Some(Path::new(&crate::test_support::absolute("/config"))),
        );
        assert!(f.message.len() <= 16_384 && f.message_truncated);
        assert!(f.meeting_root.is_none());
        let v = serde_json::to_value(f).unwrap();
        assert_eq!(v["stage"], "storage_selection");
        assert_eq!(v["salvageAvailable"], false);
        assert_eq!(v["configRoot"], crate::test_support::absolute("/config"));
        assert!(v.get("messageTruncated").is_some());
        assert!(v.get("message_truncated").is_none());
    }
    #[test]
    fn report_wire_is_exact_and_read_access_stays_main_only_when_ready() {
        let s = Startup::default();
        s.ready().unwrap();
        assert_eq!(
            serde_json::to_value(s.snapshot().unwrap()).unwrap(),
            serde_json::json!({"schemaVersion":1,"platform":if cfg!(windows){"windows"}else if cfg!(target_os="macos"){"macos"}else{"other"},"phase":"ready","failure":null})
        );
        assert!(s.admit_command("recorder", "get_startup_report").is_err());
    }
    #[test]
    fn background_read_diagnostics_are_main_only_before_storage_admission() {
        let startup = Startup::default();
        let command = "record_background_read_failure";
        assert!(startup.admit_command("main", command).is_ok());
        assert!(startup.admit_command("recorder", command).is_err());
        assert!(startup
            .admit_command("recording-confirmation", command)
            .is_err());
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
        assert!(failure.salvage_available);
        assert!(failure.meeting_root.is_none());
        assert!(!root.exists());
        assert_eq!(
            std::fs::read(record).unwrap(),
            b"invalid synthetic bootstrap"
        );
    }
    #[cfg(any(target_os = "macos", windows))]
    fn admission_failure(config: &Path, root: &Path) -> Failure {
        match admit_storage(config, root) {
            Err(failure) => failure,
            Ok(_) => panic!("unexpected storage admission"),
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn native_startup_salvage_requires_evidence_and_admits_only_the_main_window() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let root = temp.path().join("app-data");
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        let database = storage.root().join("meetings.sqlite3");
        assert!(!admission_failure(&config, &root).salvage_available);
        drop(bootstrap);
        assert!(!admission_failure(&config, &root).salvage_available);
        drop(storage);

        std::fs::write(&database, [b'x'; 100]).unwrap();
        let failure = admission_failure(&config, &root);
        assert_eq!(failure.stage, Stage::StorageSelection);
        assert!(failure.salvage_available);
        let report = Startup::default();
        report.block(failure).unwrap();
        assert!(report.admit_command("main", "open_salvage").is_ok());
        assert!(report.admit_command("recorder", "open_salvage").is_err());
        assert_eq!(std::fs::read(&database).unwrap(), [b'x'; 100]);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn damaged_header_at_a_replaced_meeting_root_never_requests_salvage() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let root = temp.path().join("app-data");
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        let selected = storage.root().to_owned();
        drop(storage);
        drop(bootstrap);

        let original = temp.path().join("retained-meetings");
        std::fs::rename(&selected, &original).unwrap();
        std::fs::create_dir(&selected).unwrap();
        let rejected = selected.join("meetings.sqlite3");
        let mut file = std::fs::File::create(&rejected).unwrap();
        file.write_all(b"invalid header").unwrap();
        drop(file);

        let failure = admission_failure(&config, &root);
        assert_eq!(failure.stage, Stage::StorageSelection);
        assert!(!failure.salvage_available);
        assert!(
            failure
                .message
                .contains("会議データのファイルが現在のものと一致しません"),
            "{failure:?}"
        );
        assert_eq!(std::fs::read(&rejected).unwrap(), b"invalid header");
        assert!(original.join("meetings.sqlite3").is_file());
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn startup_offers_salvage_for_unverified_meeting_schema_but_not_sqlite_environment_errors() {
        use rusqlite::{ffi, Error::SqliteFailure};
        for code in [
            ffi::SQLITE_BUSY,
            ffi::SQLITE_PERM,
            ffi::SQLITE_FULL,
            ffi::SQLITE_CANTOPEN,
            ffi::SQLITE_IOERR,
        ] {
            let misleading = SqliteFailure(
                ffi::Error::new(code),
                Some("database disk image is malformed".into()),
            );
            assert!(!crate::storage::Storage::is_salvage_blocker(&misleading));
        }
        let corrupt = SqliteFailure(ffi::Error::new(ffi::SQLITE_CORRUPT), None);
        assert!(crate::storage::Storage::is_salvage_blocker(&corrupt));

        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let root = temp.path().join("app-data");
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        crate::storage::with_test_database(&storage, |db| {
            db.execute("UPDATE schema_migrations SET checksum='unrecognized'", [])
                .unwrap();
        });
        drop(storage);
        drop(bootstrap);

        let failure = admission_failure(&config, &root);
        assert_eq!(failure.stage, Stage::StorageAdmission);
        assert!(failure.salvage_available);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn unfinished_move_keeps_startup_salvage_unavailable_even_for_an_unverified_schema() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config");
        let root = temp.path().join("app-data");
        let target = temp.path().join("destination");
        crate::native_fs::private_directories(&target, false).unwrap();
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        bootstrap
            .begin_move(
                &uuid::Uuid::new_v4().to_string(),
                &uuid::Uuid::new_v4().to_string(),
                &target.canonicalize().unwrap(),
                &"a".repeat(64),
                bootstrap.view().unwrap().revision,
            )
            .unwrap();
        crate::storage::with_test_database(&storage, |db| {
            db.execute("UPDATE schema_migrations SET checksum='unrecognized'", [])
                .unwrap();
        });
        drop(storage);
        drop(bootstrap);

        let failure = admission_failure(&config, &root);
        assert_eq!(failure.stage, Stage::StorageAdmission);
        assert!(!failure.salvage_available);
        assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn unreadable_read_cache_keeps_the_verified_database_available_and_retains_cache_objects() {
        use std::os::unix::fs::DirBuilderExt;
        for kind in [
            "initialization_gap",
            "invalid_ledger",
            "linked_root",
            "attention",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let config = base.join("config");
            let fallback = base.join("legacy");
            let (storage, bootstrap) = admit_storage(&config, &fallback).unwrap();
            let selected = storage.root().to_owned();
            let meeting = storage
                .create_meeting("SYNTHETIC RETAINED MEETING")
                .unwrap();
            let audio = selected.join("retained-audio.flac");
            std::fs::write(&audio, b"SYNTHETIC KEEP AUDIO").unwrap();
            drop(storage);
            drop(bootstrap);
            let controls = || {
                [
                    "meetings.sqlite3",
                    "meetings.sqlite3-wal",
                    "meetings.sqlite3-shm",
                    ".mon-scratch.sqlite3",
                ]
                .map(|name| match std::fs::read(selected.join(name)) {
                    Ok(bytes) => Some(bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => panic!("source control read failed: {e}"),
                })
            };
            let original = controls();
            let record = std::fs::read(config.join("bootstrap.json")).unwrap();
            let cache = config.join(".mon-read-cache");
            let foreign = base.join("foreign");
            std::fs::create_dir(&foreign).unwrap();
            let foreign_payload = foreign.join("keep");
            std::fs::write(&foreign_payload, b"SYNTHETIC FOREIGN DO NOT ERASE").unwrap();
            let mut retained = None;
            let mut bad_ledger = None;
            match kind {
                "initialization_gap" => {
                    std::fs::DirBuilder::new()
                        .mode(0o700)
                        .create(&cache)
                        .unwrap();
                    let file = cache.join("unknown");
                    std::fs::write(&file, b"SYNTHETIC UNOWNED DO NOT ERASE").unwrap();
                    retained = Some(file);
                }
                "invalid_ledger" => {
                    drop(crate::storage::read_cache::Snapshot::create(&config).unwrap());
                    let ledger = cache.join(".mon-scratch.sqlite3");
                    let connection = rusqlite::Connection::open(&ledger).unwrap();
                    connection
                        .execute_batch("CREATE TABLE unknown(value TEXT);")
                        .unwrap();
                    drop(connection);
                    bad_ledger = Some((ledger.clone(), std::fs::read(ledger).unwrap()));
                }
                "linked_root" => std::os::unix::fs::symlink(&foreign, &cache).unwrap(),
                "attention" => {
                    let snapshot = crate::storage::read_cache::Snapshot::create(&config).unwrap();
                    let file = snapshot.path().join("unknown");
                    std::fs::write(&file, b"SYNTHETIC UNOWNED DO NOT ERASE").unwrap();
                    retained = Some(file);
                    drop(snapshot);
                }
                _ => unreachable!(),
            }
            let (storage, bootstrap) = admit_storage(&config, &fallback).unwrap();
            assert_eq!(storage.root(), selected);
            assert!(!storage.data_write_blocked());
            assert_eq!(
                storage.get_meeting(&meeting.id).unwrap(),
                Some(meeting.clone())
            );
            let diagnostics =
                serde_json::to_value(storage.meeting_list_diagnostics().unwrap()).unwrap();
            let message = diagnostics["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["reason"] == "read_cache")
                .unwrap()["message"]
                .as_str()
                .unwrap();
            assert!(
                message.contains(&format!("対象（起動設定内）: {}", cache.display())),
                "{kind}: {message}"
            );
            assert!(message.contains("一時という名前だけで削除・移動・上書きしない"));
            assert!(message.contains("所有と必要性を確認するまで保持"));
            assert!(message.contains("元の診断:"));
            drop(storage);
            drop(bootstrap);
            assert_eq!(controls(), original, "source controls changed: {kind}");
            assert_eq!(
                std::fs::read(config.join("bootstrap.json")).unwrap(),
                record
            );
            assert_eq!(std::fs::read(&audio).unwrap(), b"SYNTHETIC KEEP AUDIO");
            assert_eq!(
                std::fs::read(foreign_payload).unwrap(),
                b"SYNTHETIC FOREIGN DO NOT ERASE"
            );
            if let Some(file) = retained {
                assert_eq!(
                    std::fs::read(file).unwrap(),
                    b"SYNTHETIC UNOWNED DO NOT ERASE"
                );
            }
            if let Some((ledger, before)) = bad_ledger {
                assert_eq!(std::fs::read(ledger).unwrap(), before);
            }
            if kind == "initialization_gap" {
                assert!(!cache.join(".mon-scratch.sqlite3").exists());
            }
            assert!(std::fs::symlink_metadata(&cache).is_ok());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn broken_scratch_ledger_only_blocks_backup_unless_a_restore_is_pending() {
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
        let (storage, bootstrap) = admit_storage(&config, &root).unwrap();
        assert_eq!(storage.root(), selected);
        assert!(!storage.common_write_hold());
        assert!(storage
            .prepare_backup_snapshot(&crate::test_support::never_cancelled)
            .err()
            .unwrap()
            .contains("バックアップに使う一時ファイル"));
        assert!(storage.create_meeting("新規会議").is_ok());
        assert_eq!(
            storage.get_meeting(&meeting.id).unwrap(),
            Some(meeting.clone())
        );
        assert_eq!(
            storage
                .rename_meeting(&meeting.id, "既存会議を編集", meeting.lock_version)
                .unwrap()
                .title,
            "既存会議を編集"
        );
        let s = Startup::default();
        s.set_common_write_hold(storage.common_write_hold())
            .unwrap();
        s.ready().unwrap();
        for _ in 0..3 {
            assert_eq!(s.snapshot().unwrap().phase, Phase::Ready);
            assert!(s.admit_command("main", "start_data_operation").is_ok());
            assert!(s.admit_command("main", "list_meetings").is_ok());
            assert!(s.admit_command("main", "get_storage_write_state").is_ok());
        }
        drop(storage);
        drop(bootstrap);
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
            "既存会議を編集"
        );
        db.execute_batch(
            "INSERT INTO restore_batches(batch_id,archive_id,archive_fingerprint,mapping_hash,mapping_count,state,created_at)
             VALUES('pending-batch','archive',printf('%064d',0),printf('%064d',0),0,'prepared',1);
             INSERT INTO maintenance_operations(id,kind,state,restore_batch_id,target_manifest_json,target_hash,progress_json,created_at,updated_at)
             VALUES('pending-restore','restore_addition','prepared','pending-batch','{}',printf('%064d',0),'{}',1,1);",
        ).unwrap();
        drop(db);
        let (storage, _bootstrap) = admit_storage(&config, &root).unwrap();
        assert!(storage.common_write_hold());
        assert!(storage
            .require_data_writable()
            .unwrap_err()
            .contains("前回のバックアップからの復元"));
        assert!(storage.create_meeting("書込み不可").is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), before);
    }
}
