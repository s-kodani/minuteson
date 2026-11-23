use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

mod analysis;
pub(crate) mod analysis_flows;
mod analysis_ui;
mod body_review;
// Backup snapshot/archive core; additive restore and controller integration follow.
mod audio_publication;
#[allow(dead_code)]
pub(crate) mod backup;
mod capture_preferences;
pub(crate) mod completion_preferences;
#[cfg(test)]
pub(crate) mod current_minutes_fixture;
#[allow(dead_code)] // Shared deletion admission and user confirmation follow.
pub(crate) mod deletion;
mod diarization;
mod diarization_preferences;
mod draft_splits;
mod drafts;
mod exports;
#[cfg(all(test, unix))]
mod generation_tests;
pub(crate) mod imports;
#[cfg(test)]
mod initialization_tests;
mod job_history;
mod jobs;
pub(crate) mod markdown_exports;
#[cfg(all(test, unix))]
pub(crate) mod markdown_generation_tests;
pub(crate) mod meeting_access;
#[cfg(test)]
mod meeting_access_tests;
pub(crate) mod meeting_audio;
pub(crate) mod meeting_mix;
mod meeting_playback;
mod minutes_input;
pub(crate) mod minutes_markdown;
mod minutes_preferences;
pub(crate) mod minutes_prompt_preferences;
mod model_checks;
mod model_metadata;
mod model_placements;
pub(crate) mod model_profiles;
mod playback;
pub(crate) mod post_analysis;
mod preferences;
#[cfg(any(unix, windows))]
mod publication_files;
#[cfg(any(target_os = "macos", windows))]
pub(crate) mod read_cache;
mod recording_preparation;
pub(crate) mod recordings;
mod revisions;
#[cfg(any(unix, windows))]
pub(crate) mod salvage;
mod salvage_schema;
pub(crate) mod speaker_assignment_update;
pub(crate) mod speaker_completion;
pub(crate) mod startup_diagnostics;
pub(crate) mod transcript_consolidation;
mod transcription;
pub(crate) mod voice_preferences;
#[cfg(all(test, target_os = "windows"))]
mod windows_tests;
pub(crate) use transcription::known::{KnownInput, Run as KnownRun};
#[cfg(all(test, unix))]
pub(crate) use transcription::tests::known_tests::split_fixture as split_audio_fixture;
pub(crate) mod model_download_guidance;
mod ui_preferences;
pub(crate) mod vocabulary;
pub(crate) mod voice;
pub(crate) use analysis::AnalysisPresentation;
pub use analysis::AnalysisView;
pub(crate) use analysis::{AnalysisInput, UtteranceInput};
#[cfg(test)]
pub(crate) use analysis::{AnalysisSpeaker, AnalysisUtterance};
pub(crate) use analysis::{SpeakerInput, SpeakerSegmentInput};
pub(crate) use analysis_ui::{DraftStamp, MinutesSource};
pub(crate) use diarization::DiarizationInput;
pub(crate) use diarization_preferences::{DiarizationPreferences, DiarizationPreferencesSnapshot};
pub(crate) use draft_splits::DraftUtteranceAudioRequest;
pub use drafts::{TranscriptDraft, TranscriptDraftPayload};
pub(crate) use exports::ExportRecord;
pub(crate) use jobs::{
    ExecutionProgress, JobKind, JobSnapshot, JobSpec, NotificationJob, NotificationOutput,
};
pub(crate) use meeting_playback::{
    MeetingAudioSummary, MeetingIntervalRequest, MeetingPlaybackMode, MeetingPlaybackRequest,
};
pub(crate) use minutes_input::{MinutesInput, MinutesJobRequest};
pub(crate) use minutes_preferences::{MinutesPreferences, MinutesPreferencesSnapshot};
pub(crate) use preferences::{AnalysisPreferences, PreferencesSnapshot};
pub(crate) use recordings::valid_microphone_uid;
pub use recordings::RecordingConfig;
#[cfg(test)]
pub(crate) use revisions::CommittedAnalysis;
pub(crate) use transcription::RecordedAudioPlan;
pub(crate) use ui_preferences::{UiPreferences, UiPreferencesSnapshot};
pub(crate) use vocabulary::{
    VocabularyBook, VocabularyEntry, VocabularySnapshot, VocabularyStamp, VocabularyView,
};

pub(crate) const SCHEMA_VERSION: usize = 40;
mod schema_40;
const SCHEMA_SQL: &str = include_str!("schema.sql");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub lifecycle: String,
    pub source_type: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub lock_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAsset {
    pub id: String,
    pub meeting_id: String,
    pub relative_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub content_hash: String,
    pub state: String,
}

pub struct Storage {
    root: PathBuf,
    connection: Mutex<Connection>,
    meeting_owners: meeting_access::Owners,

    pub(crate) feature_vectors: Mutex<crate::speaker_features::Cache>,
    execution_progress: Mutex<std::collections::HashMap<String, jobs::ExecutionProgress>>,
    post_analysis_drain: Mutex<()>,
    voice_audio_lock: Mutex<()>,
    playback_wav: crate::audio::PlaybackWavCache,
    maintenance: std::sync::atomic::AtomicBool,
    // Process-local, irreversible. Bootstrap decides which root to open next.
    retired: std::sync::atomic::AtomicBool,
    scratch: Option<std::sync::Arc<backup::scratch::Registry>>,
    scratch_startup_error: Option<String>,
    deletion_fence: deletion::fence::Fence,
    recovery_hold: bool,
    startup_write_holds: Vec<String>,
    startup_shared_database_corrupt: bool,
    startup_diagnostics: startup_diagnostics::Diagnostics,
    #[cfg(any(unix, windows))]
    managed_anchor: deletion::fs::RootAnchor,
    #[cfg(any(unix, windows))]
    native_models_root: PathBuf,
    #[cfg(any(target_os = "macos", windows))]
    restored_models: Option<crate::bootstrap::salvage::SelectionRecord>,
    #[cfg(any(target_os = "macos", windows))]
    flac_history: Option<std::sync::Arc<recordings::flac::relocation::Aliases>>,
    #[cfg(any(target_os = "macos", windows))]
    wav_history: Option<std::sync::Arc<backup::relocation::wav::Aliases>>,
    #[cfg(any(target_os = "macos", windows))]
    _reset_guard: std::sync::Arc<crate::bootstrap::reset::RootLease>,
}

#[cfg(test)]
pub(crate) fn with_test_database<T>(storage: &Storage, action: impl FnOnce(&Connection) -> T) -> T {
    action(&storage.connection.lock().unwrap())
}

impl Storage {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn native_models_root(&self) -> Result<&Path, String> {
        self.require_model_storage()?;
        Ok(&self.native_models_root)
    }

    pub(crate) fn require_model_storage(&self) -> Result<(), String> {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(binding) = &self.restored_models {
            binding.verify_models().map_err(|error| {
                format!(
                    "復元したモデル保存先を確認できません。モデル保存先を戻してください: {error}"
                )
            })?;
        }
        Ok(())
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn verify_bootstrap_database(&self) -> Result<(), String> {
        self.managed_anchor.verify_sqlite_header()
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn keep_sqlite_controls_on_close(&self, keep: bool) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|_| "会議DBのロックを取得できません")?
            .set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                keep,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn restart_required(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
            || self.deletion_fence.restart_required()
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn deletion_resume_restart_required(&self) -> bool {
        self.deletion_fence.resume_restart_required()
    }

    pub(crate) fn common_write_hold(&self) -> bool {
        self.recovery_hold
            || !self.startup_write_holds.is_empty()
            || self.deletion_fence.unknown_startup_scope()
    }

    pub(crate) fn data_write_blocked(&self) -> bool {
        self.common_write_hold()
            || self.retired.load(std::sync::atomic::Ordering::Acquire)
            || self.deletion_fence.blocked()
    }

    pub(crate) fn require_data_writable(&self) -> Result<(), String> {
        if self.recovery_hold {
            Err("保存先を復旧しました。中断操作の再確認が必要なため閲覧のみ利用できます。元の記録とファイルを保持しています".into())
        } else if let Some(message) = self.startup_write_holds.first() {
            Err(message.clone())
        } else if self.deletion_fence.unknown_startup_scope() {
            Err("対象を確認できない削除記録があります。元のデータを保持し、保存先の書込みを停止しました".into())
        } else if self.deletion_fence.pending() && self.deletion_fence.blocked() {
            Err("未完了の会議削除／音声削除／モデル削除があります。確認対象を保持するため書込みと処理開始を停止しています".into())
        } else if self.deletion_fence.restart_required() {
            Err(
                "削除の確認記録を解決しました。保留した起動時回復を行うため再起動してください"
                    .into(),
            )
        } else if self.restart_required() {
            Err("旧保存先の書込みは停止済みです。再起動してください".into())
        } else {
            Ok(())
        }
    }

    // Relational deleting state; do not rewrite the immutable confirmed meeting
    // cells or change Meeting's portable fingerprints just to display a gate.
    #[allow(dead_code)] // Deletion confirmation/recovery UI integration follows.
    pub(crate) fn deleting_meeting_ids(&self) -> Result<Vec<String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = db.prepare(&format!("SELECT DISTINCT meeting_id FROM maintenance_operations WHERE kind='delete_meeting' AND {} AND meeting_id IS NOT NULL ORDER BY meeting_id", deletion::fence::predicate(&db)?)).map_err(|e|e.to_string())?;
        let result = query
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string());
        result
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn validate_startup_reset(&self) -> Result<(), String> {
        let db = self.connection.lock().map_err(|_| "新DBを検証できません")?;
        let check: String = db
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let empty: bool = db.query_row("SELECT (SELECT count(*) FROM meetings)=0 AND (SELECT count(*) FROM model_packages)=0", [], |r| r.get(0)).map_err(|e|e.to_string())?;
        if check != "ok" || !empty {
            return Err("新DBの初期化結果を確認できません".into());
        }
        Ok(())
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn open_for_reset(
        root: PathBuf,
        models: PathBuf,
        guard: std::sync::Arc<crate::bootstrap::reset::RootLease>,
    ) -> Result<Self, Box<dyn Error>> {
        Self::open_inner(root, Some(models), None, None, None, Some(guard), None)
    }
    // macOS production always supplies its separate native model location.
    #[cfg(any(test, not(target_os = "macos")))]
    pub fn open(root: PathBuf) -> Result<Self, Box<dyn Error>> {
        #[cfg(any(unix, windows))]
        {
            Self::open_inner(
                root,
                None,
                #[cfg(any(target_os = "macos", windows))]
                None,
                #[cfg(any(target_os = "macos", windows))]
                None,
                #[cfg(any(target_os = "macos", windows))]
                None,
                #[cfg(any(target_os = "macos", windows))]
                None,
                #[cfg(any(target_os = "macos", windows))]
                None,
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            Self::open_inner(root)
        }
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn open_with_models(
        root: PathBuf,
        models_root: PathBuf,
    ) -> Result<Self, Box<dyn Error>> {
        Self::open_inner(
            root,
            Some(models_root),
            #[cfg(any(target_os = "macos", windows))]
            None,
            #[cfg(any(target_os = "macos", windows))]
            None,
            #[cfg(any(target_os = "macos", windows))]
            None,
            #[cfg(any(target_os = "macos", windows))]
            None,
            #[cfg(any(target_os = "macos", windows))]
            None,
        )
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn open_with_model_binding(
        root: PathBuf,
        binding: crate::bootstrap::salvage::SelectionRecord,
    ) -> Result<Self, Box<dyn Error>> {
        let models_root = binding.models_root().to_owned();
        Self::open_inner(
            root,
            Some(models_root),
            None,
            None,
            None,
            None,
            Some(binding),
        )
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn open_with_history(
        admission: &crate::bootstrap::placement::NativeHistoryAdmission<'_, '_>,
        restored_models: Option<crate::bootstrap::salvage::SelectionRecord>,
    ) -> Result<Self, Box<dyn Error>> {
        admission.verify()?;
        let flac_history = recordings::flac::relocation::admitted_history(admission)?;
        let history = deletion::relocation::receiver::admitted_history(admission)?;
        let wav_history = backup::relocation::wav::admitted_history(admission)?;
        let mut storage = Self::open_inner(
            admission.root().to_owned(),
            Some(admission.models_root().to_owned()),
            history,
            flac_history,
            None,
            #[cfg(any(target_os = "macos", windows))]
            None,
            restored_models,
        )?;
        storage.wav_history = wav_history;
        admission.verify()?;
        Ok(storage)
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn open_recovered(
        root: PathBuf,
        models: PathBuf,
        admission: &crate::bootstrap::recovery::Admission,
        restored_models: Option<crate::bootstrap::salvage::SelectionRecord>,
    ) -> Result<Self, Box<dyn Error>> {
        Self::open_inner(
            root,
            Some(models),
            None,
            None,
            Some(admission),
            None,
            restored_models,
        )
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn is_salvage_blocker(error: &(dyn Error + 'static)) -> bool {
        use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
        error.is::<schema_40::UnsupportedSchema>()
            || error
                .downcast_ref::<rusqlite::Error>()
                .and_then(rusqlite::Error::sqlite_error_code)
                .is_some_and(|code| matches!(code, DatabaseCorrupt | NotADatabase))
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn initialize_rebinding_scratch(root: &Path, journal: &Path) -> Result<(), String> {
        drop(backup::scratch::Registry::open_at(root, journal)?);
        Ok(())
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn ancillary_binding_changed(root: &Path, journal: &Path, cache: &Path) -> bool {
        // This caller has already matched the selected root and DB identities.
        // An I/O failure here does not prove displacement. Normal admission
        // independently opens each ancillary resource with its original proof.
        let scratch = backup::scratch::Registry::binding_changed(root, journal);
        let cache = cache.join(".mon-read-cache");
        let cache = match crate::native_fs::symlink_metadata(&cache) {
            Ok(_) => backup::scratch::Registry::binding_changed(&cache, &cache),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        };
        let mut changed = false;
        for (resource, result) in [("scratch", scratch), ("read_cache", cache)] {
            match result {
                Ok(value) => changed |= value,
                Err(error) => crate::internal_logs::report_error(resource, &error),
            }
        }
        changed
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn inspect_rebinding(root: &Path, cache: &Path) -> Result<Vec<String>, String> {
        deletion::relocation::receiver::inspect_rebinding(root, cache, Self::validate_rebinding)
    }

    #[cfg(any(target_os = "macos", windows))]
    fn validate_rebinding(db: &Connection) -> Result<Vec<String>, String> {
        deletion::database::trusted_schema(db)?;
        let integrity: String = db
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err("復旧対象DBの整合性検査に失敗しました".into());
        }
        let mut pending = Vec::new();
        for (sql, reason) in [
            ("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE state IN ('prepared','installing','cleanup','attention') OR (state='completed' AND json_extract(progress_json,'$.version')=4 AND (kind='delete_meeting' OR json_extract(progress_json,'$.phase')='removing')))", "削除・復元などの保守操作が未完了です"),
            ("SELECT EXISTS(SELECT 1 FROM restore_batches WHERE state='prepared')", "バックアップからの復元が未完了です"),
            ("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE state!='saved')", "録音・音声変換の再確認が必要です"),
            ("SELECT EXISTS(SELECT 1 FROM audio_imports WHERE state!='ready')", "音声取り込みの再確認が必要です"),
            ("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('queued','running','pausing','paused','interrupted'))", "解析処理が未完了です"),
            ("SELECT EXISTS(SELECT 1 FROM model_packages WHERE state='preparing')", "モデル配置が未完了です"),
            ("SELECT EXISTS(SELECT 1 FROM export_records WHERE state='writing')", "書き出しが未完了です"),
            ("SELECT EXISTS(SELECT 1 FROM audio_tracks WHERE state='writing') OR EXISTS(SELECT 1 FROM analysis_revisions WHERE publication_state='building')", "音声・解析結果の作成が未完了です"),
        ] {
            if db.query_row(sql, [], |r| r.get::<_, bool>(0)).map_err(|e| e.to_string())? { pending.push(reason.into()); }
        }
        Ok(pending)
    }

    #[cfg(any(target_os = "macos", windows))]
    fn validate_rebinding_before_migration(db: &Connection) -> Result<Vec<String>, String> {
        // Recovery admission precedes the one supported DB upgrade. Its
        // schema39 exception requires a query-only connection, even though
        // startup owns this connection for the later writable migration.
        let previous: bool = db
            .query_row("PRAGMA query_only", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if !previous {
            db.execute_batch("PRAGMA query_only=ON;")
                .map_err(|error| error.to_string())?;
        }
        let validated = Self::validate_rebinding(db);
        if !previous {
            // Restore on validation failure as well. A restore failure stops
            // startup before journal changes, migration or cleanup can run.
            if let Err(error) = db.execute_batch("PRAGMA query_only=OFF;") {
                return Err(match validated {
                    Err(original) => format!("{original}。DB接続の状態を戻せません: {error}"),
                    Ok(_) => format!("DB接続の状態を戻せません: {error}"),
                });
            }
        }
        validated
    }

    fn open_inner(
        root: PathBuf,
        #[cfg(any(unix, windows))] models_root: Option<PathBuf>,
        #[cfg(any(target_os = "macos", windows))] history: Option<
            std::sync::Arc<deletion::relocation::receiver::Aliases>,
        >,
        #[cfg(any(target_os = "macos", windows))] flac_history: Option<
            std::sync::Arc<recordings::flac::relocation::Aliases>,
        >,
        #[cfg(any(target_os = "macos", windows))] recovery: Option<
            &crate::bootstrap::recovery::Admission,
        >,
        #[cfg(any(target_os = "macos", windows))] reset_guard: Option<
            std::sync::Arc<crate::bootstrap::reset::RootLease>,
        >,
        #[cfg(any(target_os = "macos", windows))] restored_models: Option<
            crate::bootstrap::salvage::SelectionRecord,
        >,
    ) -> Result<Self, Box<dyn Error>> {
        std::fs::create_dir_all(&root)?;
        // Resolve system aliases (e.g. macOS /var -> /private/var) once.
        // Managed children still must match this canonical root exactly.
        let root = root.canonicalize()?;
        #[cfg(any(target_os = "macos", windows))]
        let resetting = reset_guard.is_some();
        #[cfg(not(any(target_os = "macos", windows)))]
        let resetting = false;
        #[cfg(windows)]
        crate::windows_files::local_ntfs(&root)?;
        #[cfg(windows)]
        crate::windows_files::set_private(&root)?;
        #[cfg(any(target_os = "macos", windows))]
        let reset_guard = match reset_guard {
            Some(guard) => {
                guard.verify(&root)?;
                guard
            }
            None => crate::bootstrap::reset::RootLease::acquire(&root, false)?,
        };
        #[cfg(any(unix, windows))]
        let root_directory = deletion::fs::RootAnchor::directory(&root)?;
        #[cfg(any(target_os = "macos", windows))]
        let recovery_content = recovery
            .map(|admission| admission.capture_content())
            .transpose()?;
        let mut flags = rusqlite::OpenFlags::default();
        #[cfg(any(target_os = "macos", windows))]
        if recovery.is_some() {
            flags.remove(rusqlite::OpenFlags::SQLITE_OPEN_CREATE);
        }
        // An existing rollback journal may require writing the database just to read
        // it. Preserve those files and diagnose before opening the connection.
        match crate::native_fs::symlink_metadata(root.join("meetings.sqlite3-journal")) {
            // Explicit reset has already moved the old files and owns the
            // empty sidecar it reserved for the new database.
            Ok(metadata) if resetting && metadata.is_file() && metadata.len() == 0 => {}
            Ok(_) => {
                return Err(
                    "会議DBの未確認のジャーナルを検出しました。元のデータを保持しています".into(),
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let database = root.join("meetings.sqlite3");
        if !resetting {
            check_database_without_wal(&root, &database)?;
        }
        let mut connection = Connection::open_with_flags(
            &database,
            flags | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        use rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE;
        connection.set_db_config(SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)?;
        // Set EXCLUSIVE before *any* access to an existing WAL database so
        // SQLite keeps the wal-index on the heap rather than mmap'ing -shm.
        // Switching after journal_mode has read the DB is too late on reopen.
        connection
            .execute_batch("PRAGMA main.locking_mode = EXCLUSIVE; PRAGMA foreign_keys = ON;")?;
        // Refuse old/unknown databases before journal-mode changes, recovery
        // writes, temporary registries or filesystem cleanup can begin.
        empty_or_current_database(&connection)?;
        connection.set_db_config(SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, false)?;
        let mut recovery_hold = false;
        #[cfg(any(target_os = "macos", windows))]
        if let Some(admission) = recovery {
            // Verify again through SQLite before any mutable startup work.
            // Admission checks only metadata and never closes another live DB FD.
            Self::validate_rebinding_before_migration(&connection)?;
            recovery_hold = admission.pending();
            admission.verify()?;
            recovery_content
                .as_ref()
                .ok_or("復旧対象の接続前検査がありません")?
                .verify()?;
        }
        connection.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA secure_delete = ON; BEGIN IMMEDIATE; COMMIT;",
        )?;
        #[cfg(any(unix, windows))]
        let mut managed_anchor = deletion::fs::RootAnchor::bind(root_directory)?;
        #[cfg(any(target_os = "macos", windows))]
        if let Some(admission) = recovery {
            managed_anchor = managed_anchor.with_startup_binding(admission.previous_identities());
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = history {
            managed_anchor = managed_anchor.with_inspection_history(history)?;
        }
        #[cfg(any(unix, windows))]
        let native_models_root = models_root.unwrap_or_else(|| root.join("models"));
        initialize_database_with_guidance(&mut connection, true)
            .map_err(|e| format!("会議DBの初期化に失敗しました: {e:?}"))?;
        let mut startup_write_holds = Vec::new();
        let mut startup_shared_database_corrupt = false;
        let startup_diagnostics = startup_diagnostics::Diagnostics::default();
        #[cfg(any(unix, windows))]
        let inspect_model_files = Some(native_models_root.as_path());
        #[cfg(any(target_os = "macos", windows))]
        let inspect_model_files = inspect_model_files.filter(|_| {
            restored_models
                .as_ref()
                .is_none_or(|binding| binding.verify_models().is_ok())
        });
        #[cfg(any(unix, windows))]
        if let Err(error) =
            deletion::startup::inspect(&connection, &managed_anchor, inspect_model_files)
        {
            startup_write_holds.push(format!(
                "削除記録の属先を確認できません。元のデータを保持しています: {error}"
            ));
        }
        let deletion_fence = deletion::fence::Fence::open(&mut connection)?;
        let mut scratch = None;
        let mut scratch_startup_error = None;
        if !deletion_fence.blocked()
            && !recovery_hold
            && startup_write_holds.is_empty()
            && !resetting
        {
            #[cfg(any(unix, windows))]
            {
                #[cfg(any(target_os = "macos", windows))]
                let registry = match recovery {
                    Some(admission) => {
                        backup::scratch::Registry::open_at(&root, &admission.scratch_root())
                    }
                    None => backup::scratch::Registry::open(&root),
                };
                #[cfg(not(any(target_os = "macos", windows)))]
                let registry = backup::scratch::Registry::open(&root);
                match registry {
                    Ok(registry) => {
                        if let Err(error) = registry.require_no_attention() {
                            startup_diagnostics.record(None, "scratch_attention", &error);
                        }
                        scratch = Some(registry);
                    }
                    Err(error) => {
                        crate::internal_logs::report_error("backup_scratch", &error);
                        scratch_startup_error = Some("バックアップに使う一時ファイルの状態を確認できませんでした。元のデータは残っています。".into());
                    }
                }
            }
            let registry = match scratch_startup_error.as_deref() {
                Some(error) => Err(error),
                None => Ok(scratch.as_deref()),
            };
            if let Err(error) = backup::recover_restore_operations(&root, &connection, registry) {
                crate::internal_logs::report_error("backup_restore_recovery", &error);
                startup_write_holds.push("前回のバックアップからの復元が完了したか確認できません。会議の閲覧は続けられます。".into());
            }
        }
        if !deletion_fence.blocked() && !recovery_hold && startup_write_holds.is_empty() {
            // Establish operation scopes before any mutable recovery. Stale
            // states belonging to a protected meeting are left as evidence.
            let unreserved = meeting_access::unreserved_sql(&connection, "meeting_id")?;
            let updates = (|| -> Result<(), String> {
                let tx = connection
                    .unchecked_transaction()
                    .map_err(|error| error.to_string())?;
                if let Err(error) = normalize_startup_states(&tx, &unreserved, None) {
                    startup_shared_database_corrupt = error
                        .downcast_ref::<rusqlite::Error>()
                        .is_some_and(startup_diagnostics::structural_database_error);
                    return Err(error.to_string());
                }
                let now = now_ms()?;
                tx.execute("UPDATE model_compatibility_checks SET state='interrupted',error_message='前回の互換性確認は未完了です。明示的に再確認してください',finished_at=max(created_at,?1) WHERE state='preparing'", params![now]).map_err(|error| error.to_string())?;
                // Never resume execution from an old placement/check record.
                tx.execute("UPDATE model_files SET state='unavailable' WHERE state='preparing' AND package_id IN (SELECT id FROM model_packages WHERE source_type IN ('manual_import','download') AND state='preparing')", []).map_err(|error| error.to_string())?;
                tx.execute("UPDATE model_packages SET state='unavailable' WHERE source_type IN ('manual_import','download') AND state='preparing'", []).map_err(|error| error.to_string())?;
                tx.commit().map_err(|error| error.to_string())
            })();
            if let Err(error) = updates {
                startup_write_holds.push(format!("中断した処理の状態を確認できません: {error}"));
            }
        }
        #[cfg(any(unix, windows))]
        managed_anchor.seal_controls()?;
        if recovery_hold || !startup_write_holds.is_empty() {
            connection.authorizer(Some(deletion::fence::readonly))?;
        }
        Ok(Self {
            root,
            connection: Mutex::new(connection),
            meeting_owners: Default::default(),

            feature_vectors: Mutex::default(),
            execution_progress: Mutex::default(),
            post_analysis_drain: Mutex::default(),
            voice_audio_lock: Mutex::default(),
            playback_wav: Default::default(),
            maintenance: std::sync::atomic::AtomicBool::new(false),
            retired: std::sync::atomic::AtomicBool::new(false),
            scratch,
            scratch_startup_error,
            deletion_fence,
            recovery_hold,
            startup_write_holds,
            startup_shared_database_corrupt,
            startup_diagnostics,
            #[cfg(any(unix, windows))]
            managed_anchor,
            #[cfg(any(unix, windows))]
            native_models_root,
            #[cfg(any(target_os = "macos", windows))]
            restored_models,
            #[cfg(any(target_os = "macos", windows))]
            _reset_guard: reset_guard,
            #[cfg(any(target_os = "macos", windows))]
            flac_history,
            #[cfg(any(target_os = "macos", windows))]
            wav_history: None,
        })
    }

    fn backup_stage(&self) -> Result<backup::scratch::Stage, String> {
        // Serialize allocation with source retirement, including its registry
        // write. Existing snapshot cleanup remains limited to its owned stage.
        let _db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        if let Some(error) = &self.scratch_startup_error {
            return Err(error.clone());
        }
        if meeting_access::any_reserved(&_db)? || deletion::fence::has_pending(&_db)? {
            return Err("削除の確認を完了してから一時保存領域を使用してください".into());
        }
        if self.retired.load(std::sync::atomic::Ordering::Acquire) {
            return Err("旧保存先の書込みは停止済みです。再起動してください".into());
        }
        match &self.scratch {
            Some(registry) => registry.stage(),
            None => backup::scratch::Stage::create(&self.root),
        }
    }

    pub fn create_meeting(&self, title: &str) -> Result<Meeting, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("会議名は1～200文字で入力してください".into());
        }
        let now = now_ms()?;
        let meeting = Meeting {
            id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            lifecycle: "draft".into(),
            source_type: None,
            created_at: now,
            updated_at: now,
            lock_version: 0,
        };
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        self.require_data_writable()?;
        db.execute(
                "INSERT INTO meetings (id, title, lifecycle, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![meeting.id, meeting.title, meeting.lifecycle, now, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(meeting)
    }

    pub fn list_meetings(&self) -> Result<Vec<Meeting>, String> {
        Ok(self.meeting_list()?.0)
    }

    // Separate read view: do not change Meeting's portable serialization and
    // therefore the fingerprints of existing backups and prepared batches.
    pub(crate) fn restored_meeting_ids(&self) -> Result<Vec<String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = db.prepare("SELECT DISTINCT m.id FROM meetings m JOIN restore_provenance p ON p.meeting_id=m.id AND p.domain='meetings' AND p.entity_key=m.id ORDER BY m.id").map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_meeting(&self, id: &str) -> Result<Option<Meeting>, String> {
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .query_row(
                "SELECT id, title, lifecycle, created_at, updated_at, lock_version, source_type FROM meetings WHERE id = ?1",
                params![id],
                |row| {
                    startup_diagnostics::meeting_row(row).map(|(meeting, _)| meeting)
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn rename_meeting(
        &self,
        id: &str,
        title: &str,
        expected_lock_version: i64,
    ) -> Result<Meeting, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("会議名は1～200文字で入力してください".into());
        }
        if expected_lock_version < 0 {
            return Err("会議の版番号が不正です".into());
        }
        let now = now_ms()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        self.require_data_writable()?;
        if meeting_access::reserved(&connection, id)? {
            return Err("この会議は削除の確認・処理中です".into());
        }
        let changed = connection
            .execute(
                "UPDATE meetings SET title = ?1, updated_at = ?2, lock_version = lock_version + 1 WHERE id = ?3 AND lock_version = ?4",
                params![title, now, id, expected_lock_version],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            return Err(if exists {
                "別の画面で会議が更新されました。再読込してください"
            } else {
                "会議が見つかりません"
            }
            .into());
        }
        connection
            .query_row(
                "SELECT id, title, lifecycle, created_at, updated_at, lock_version, source_type FROM meetings WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Meeting {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        lifecycle: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        lock_version: row.get(5)?,
                        source_type: row.get(6)?,
                    })
                },
            )
            .map_err(|error| error.to_string())
    }

    // Only identifiers generated by the app may form managed paths. Never use a meeting title.
    #[allow(dead_code)]
    pub fn managed_path(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("管理領域から外れるパスです".into());
        }
        Ok(crate::native_fs::join(&self.root, relative))
    }

    // Called only after the recorder has finished and closed a managed file.
    // The UI cannot provide an arbitrary source path to this method.
    #[allow(dead_code)]
    pub fn register_ready_meeting_asset(
        &self,
        meeting_id: &str,
        relative: &Path,
        media_type: &str,
    ) -> Result<FileAsset, String> {
        let mut parts = relative.components();
        if !matches!(parts.next(), Some(Component::Normal(name)) if name == "meetings")
            || !matches!(parts.next(), Some(Component::Normal(name)) if name == meeting_id)
            || parts.next().is_none()
        {
            return Err("会議の管理パスが不正です".into());
        }
        if media_type.is_empty() || media_type.len() > 100 {
            return Err("メディア形式が不正です".into());
        }
        let relative_text = relative
            .to_str()
            .ok_or("管理パスを文字列化できません")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative_text.contains('\\') {
            return Err("管理パスに使用できない文字があります".into());
        }
        #[cfg(any(unix, windows))]
        if deletion::audio::is_audio_media(media_type) {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            deletion::audio::require_source(&db, meeting_id)?;
        }
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if !actual.starts_with(&root) || actual != root.join(relative) {
            return Err("管理領域から外れるファイルです".into());
        }
        if !actual.is_file() {
            return Err("管理対象は通常ファイルに限ります".into());
        }
        let (size_bytes, content_hash) = hash_file(&actual)?;
        let now = now_ms()?;
        let asset = FileAsset {
            id: Uuid::new_v4().to_string(),
            meeting_id: meeting_id.to_owned(),
            relative_path: relative_text,
            media_type: media_type.to_owned(),
            size_bytes,
            content_hash,
            state: "ready".into(),
        };
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        #[cfg(any(unix, windows))]
        if deletion::audio::is_audio_media(media_type) {
            deletion::audio::require_source(&db, meeting_id)?;
        }
        db.execute(
                "INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES (?1, ?2, 'meeting', ?3, ?4, ?5, 'sha256', ?6, 'ready', ?7, ?7)",
                params![asset.id, asset.meeting_id, asset.relative_path, asset.media_type, asset.size_bytes, asset.content_hash, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(asset)
    }

    #[allow(dead_code)]
    pub fn verify_asset(&self, asset_id: &str) -> Result<bool, String> {
        let asset: Option<FileAsset> = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .query_row(
                "SELECT id, meeting_id, relative_path, media_type, size_bytes, content_hash, state FROM file_assets WHERE id = ?1",
                params![asset_id],
                |row| Ok(FileAsset {
                    id: row.get(0)?, meeting_id: row.get(1)?, relative_path: row.get(2)?, media_type: row.get(3)?,
                    size_bytes: row.get(4)?, content_hash: row.get(5)?, state: row.get(6)?,
                }),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let asset = asset.ok_or("管理ファイルが見つかりません")?;
        let relative = Path::new(&asset.relative_path);
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if !actual.starts_with(&root) || actual != root.join(relative) {
            return Err("管理領域から外れるファイルです".into());
        }
        let (size_bytes, content_hash) = hash_file(&actual)?;
        Ok(size_bytes == asset.size_bytes && content_hash == asset.content_hash)
    }
}

fn normalize_startup_states(
    db: &Connection,
    selector: &str,
    meeting: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let now = now_ms()?;
    let jobs = jobs::unreserved_audio_job_sql(db, "processing_jobs")?;
    let recordings = deletion::fence::audio_unreserved(db, "recording_sessions.meeting_id")?;
    let imports = deletion::fence::audio_unreserved(db, "audio_imports.meeting_id")?;
    for sql in [
        format!("UPDATE processing_jobs SET state='interrupted',updated_at=?1 WHERE state IN ('running','pausing'){selector}{jobs}"),
        format!("UPDATE recording_sessions SET state='interrupted',updated_at=?1 WHERE state IN ('preparing','recording','stopping'){selector} AND {recordings}"),
        format!("UPDATE export_records SET state='interrupted',error_message='前回の書き出し結果が未確定です。外部ファイルを確認してください',updated_at=?1 WHERE state='writing'{selector}"),
        format!("UPDATE audio_imports SET state='failed',error_code='interrupted',updated_at=?1 WHERE state IN ('copying','converting'){selector} AND {imports}"),
    ] {
        let result = if let Some(meeting) = meeting {
            db.execute(&sql, params![now, meeting])
        } else {
            db.execute(&sql, params![now])
        };
        result?;
    }
    Ok(())
}

fn normalize_startup_audio_states(db: &Connection, meeting: &str) -> Result<(), String> {
    let now = now_ms()?;
    let audio_kinds = jobs::audio_job_kinds();
    for sql in [
        format!("UPDATE processing_jobs SET state='interrupted',updated_at=?1
                 WHERE meeting_id=?2 AND state IN ('running','pausing') AND kind IN ({audio_kinds})"),
        "UPDATE recording_sessions SET state='interrupted',updated_at=?1
         WHERE meeting_id=?2 AND state IN ('preparing','recording','stopping')".into(),
        "UPDATE audio_imports SET state='failed',error_code='interrupted',updated_at=?1
         WHERE meeting_id=?2 AND state IN ('copying','converting')".into(),
    ] {
        db.execute(&sql, params![now, meeting]).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn check_database_without_wal(root: &Path, database: &Path) -> Result<(), Box<dyn Error>> {
    use rusqlite::OpenFlags;

    let exists = |path: &Path| match crate::native_fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let wal = root.join("meetings.sqlite3-wal");
    let Some(metadata) = exists(database)? else {
        return Ok(());
    };
    if metadata.len() == 0 || exists(&wal)?.is_some() {
        return Ok(());
    }
    // A read-write connection creates an empty sidecar when the main file's
    // header selects WAL mode. Use an immutable read only when no WAL exists.
    #[cfg(any(unix, windows))]
    let anchor = {
        let mut anchor =
            deletion::fs::RootAnchor::bind(deletion::fs::RootAnchor::directory(root)?)?;
        anchor.seal_controls()?;
        anchor
    };
    if exists(&wal)?.is_some() || exists(&root.join("meetings.sqlite3-journal"))?.is_some() {
        return Err("会議DBの確認中に制御ファイルが変更されました".into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    #[cfg(windows)]
    let connection = {
        // Keep the canonical verbatim prefix when passing the path to SQLite.
        let encoded = database
            .to_str()
            .ok_or("会議DBの確認用パスを作成できません")?
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        Connection::open_with_flags_and_vfs(
            format!("file:{encoded}?immutable=1"),
            flags,
            "win32-longpath",
        )?
    };
    #[cfg(not(windows))]
    let connection = {
        let mut uri = tauri::Url::from_file_path(database)
            .map_err(|_| "会議DBの確認用パスを作成できません")?;
        uri.set_query(Some("immutable=1"));
        Connection::open_with_flags(uri.as_str(), flags)?
    };
    let admission = empty_or_current_database(&connection);
    drop(connection);
    #[cfg(any(unix, windows))]
    anchor.verify()?;
    admission.map(|_| ())
}

fn empty_or_current_database(connection: &Connection) -> Result<bool, Box<dyn Error>> {
    let empty: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*')",
        [],
        |row| row.get(0),
    )?;
    if !empty {
        schema_40::verify_startup(connection)?;
    }
    Ok(empty)
}

pub(crate) fn initialize_database(connection: &mut Connection) -> Result<(), Box<dyn Error>> {
    initialize_database_with_guidance(connection, false)
}

fn initialize_database_with_guidance(
    connection: &mut Connection,
    guidance: bool,
) -> Result<(), Box<dyn Error>> {
    if !empty_or_current_database(connection)? {
        schema_40::migrate(connection)?;
        return salvage_schema::migrate(connection);
    }
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if !empty_or_current_database(&transaction)? {
        return Ok(());
    }
    transaction.execute_batch(canonical_schema_sql())?;
    salvage_schema::install(&transaction)?;
    if guidance {
        model_download_guidance::seed(&transaction)?;
    }
    transaction.execute(
        "INSERT INTO schema_migrations (version, checksum, applied_at) VALUES (?1, ?2, ?3)",
        params![SCHEMA_VERSION as i64, schema_checksum(), now_ms()?],
    )?;
    transaction.commit()?;
    Ok(())
}

fn canonical_schema_sql() -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| SCHEMA_SQL.replace("\r\n", "\n"))
}

fn schema_checksum() -> &'static str {
    static CHECKSUM: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CHECKSUM.get_or_init(|| format!("{:x}", Sha256::digest(canonical_schema_sql().as_bytes())))
}

pub(crate) fn verify_database_schema(connection: &Connection) -> Result<(), Box<dyn Error>> {
    schema_40::current_layout(connection).map(|_| ())
}

fn hash_file(path: &Path) -> Result<(i64, String), String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut size = 0_i64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(i64::try_from(count).map_err(|error| error.to_string())?)
            .ok_or("ファイルが大きすぎます")?;
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn now_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let created = storage.create_meeting("  定例会  ").unwrap();
        assert_eq!(created.title, "定例会");
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(reopened.list_meetings().unwrap(), vec![created]);
    }

    #[test]
    fn second_instance_cannot_recover_live_recording_as_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let first = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = first.create_meeting("録音中").unwrap();
        let session = first
            .prepare_recording(
                &meeting.id,
                &RecordingConfig {
                    mode: "system".into(),
                    backend: "screencapturekit".into(),
                    target_pids: vec![],
                    microphone_device_uid: None,
                    target_application_identity: None,
                },
                "start",
            )
            .unwrap();
        first.mark_recording_started(&session.id).unwrap();
        assert!(Storage::open(temp.path().to_owned()).is_err());
        let state: String = first
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM recording_sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "recording");
        drop(first);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        let recovered: String = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM recording_sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered, "interrupted");
    }

    #[test]
    fn path_cannot_escape_root() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.managed_path(Path::new("../outside")).is_err());
        assert!(storage.managed_path(Path::new("/tmp/outside")).is_err());
        assert!(storage
            .managed_path(Path::new("meetings/id/audio/file.flac"))
            .is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_wal_reopen_uses_no_shared_index_and_preserves_large_rows() {
        let temp = tempfile::tempdir().unwrap();
        let title = "共有メモリなしで再起動";
        let first = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = first.create_meeting(title).unwrap();
        // Overflow pages exercise the same WAL lookup used to read minutes.
        let payload = "synthetic saved minutes ".repeat(4096);
        first
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE wal_reopen_fixture(id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
            )
            .unwrap();
        first
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO wal_reopen_fixture VALUES (1, ?1)",
                params![payload],
            )
            .unwrap();
        assert!(!temp.path().join("meetings.sqlite3-shm").exists());
        drop(first);

        for _ in 0..3 {
            let reopened = Storage::open(temp.path().to_owned()).unwrap();
            assert!(
                !temp.path().join("meetings.sqlite3-shm").exists(),
                "exclusive mode must precede the first access to an existing WAL database"
            );
            reopened.managed_anchor.verify().unwrap();
            let db = reopened.connection.lock().unwrap();
            // The unqualified pragma returns the connection's default for
            // subsequently attached databases, not main's configured mode.
            let mode: String = db
                .query_row("PRAGMA main.locking_mode", [], |row| row.get(0))
                .unwrap();
            let sync: i64 = db
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            let restored: String = db
                .query_row(
                    "SELECT body FROM wal_reopen_fixture WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(mode, "exclusive");
            assert_eq!(sync, 2); // FULL durability must not be weakened.
            assert_eq!(restored, payload);
            drop(db);
            assert_eq!(
                reopened.get_meeting(&meeting.id).unwrap().unwrap().title,
                title
            );
            let probe = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::tests::exclusive_wal_external_reader_fixture",
                    "--ignored",
                ])
                .env(
                    "MON_WAL_REOPEN_TEST_DATABASE",
                    temp.path().join("meetings.sqlite3"),
                )
                .output()
                .unwrap();
            assert!(
                probe.status.success()
                    && String::from_utf8_lossy(&probe.stdout).contains("1 passed; 0 failed"),
                "external reader must remain locked out after identity verification"
            );
            assert!(
                !temp.path().join("meetings.sqlite3-shm").exists(),
                "a rejected external reader must not introduce a shared index"
            );
            reopened.managed_anchor.verify().unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess-only read-only check of parent-owned synthetic WAL database"]
    fn exclusive_wal_external_reader_fixture() {
        let path = std::env::var_os("MON_WAL_REOPEN_TEST_DATABASE").expect("parent-owned fixture");
        let db =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        db.busy_timeout(std::time::Duration::ZERO).unwrap();
        let error = db
            .query_row("SELECT COUNT(*) FROM wal_reopen_fixture", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
    }

    #[test]
    fn changed_migration_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE schema_migrations SET checksum = 'changed'", [])
            .unwrap();
        drop(storage);
        assert!(Storage::open(temp.path().to_owned()).is_err());
    }

    #[test]
    fn empty_title_is_rejected_without_insert() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.create_meeting("  ").is_err());
        assert!(storage.list_meetings().unwrap().is_empty());
    }

    #[test]
    fn rename_requires_current_version_and_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let created = storage.create_meeting("旧名称").unwrap();
        let renamed = storage
            .rename_meeting(&created.id, " 新名称 ", created.lock_version)
            .unwrap();
        assert_eq!(renamed.title, "新名称");
        assert_eq!(renamed.lock_version, created.lock_version + 1);
        assert!(storage
            .rename_meeting(&created.id, "古い更新", created.lock_version)
            .is_err());
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(reopened.get_meeting(&created.id).unwrap(), Some(renamed));
    }

    #[test]
    fn missing_meeting_cannot_be_renamed() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.rename_meeting("missing", "name", 0).is_err());
    }

    #[test]
    fn managed_asset_is_hashed_and_tampering_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("音声").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", meeting.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio bytes").unwrap();
        let asset = storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .unwrap();
        assert_eq!(asset.size_bytes, 11);
        assert!(storage.verify_asset(&asset.id).unwrap());
        assert!(storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .is_err());
        std::fs::write(&file, b"audio bytes changed").unwrap();
        assert!(!storage.verify_asset(&asset.id).unwrap());
    }

    #[test]
    fn asset_cannot_cross_meeting_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", one.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio").unwrap();
        assert!(storage
            .register_ready_meeting_asset(&two.id, &relative, "audio/flac")
            .is_err());
        assert!(storage
            .register_ready_meeting_asset("missing", &relative, "audio/flac")
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_outside_is_not_registered() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("音声").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", meeting.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let outside_file = outside.path().join("other.flac");
        std::fs::write(&outside_file, b"private").unwrap();
        symlink(&outside_file, &file).unwrap();
        assert!(storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .is_err());
    }

    #[test]
    fn revision_heads_require_committed_revisions_from_same_meeting() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let db = storage.connection.lock().unwrap();
        let analysis_id = Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, 1, 'engine', 'candidate', ?3, '{}', '{}', ?4, 1, 0)",
            params![analysis_id, one.id, "a".repeat(64), "b".repeat(64)],
        ).unwrap();
        assert!(db
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
                params![analysis_id, one.id]
            )
            .is_err());
        db.execute(
            "UPDATE analysis_revisions SET publication_state = 'committed' WHERE id = ?1",
            params![analysis_id],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
                params![analysis_id, two.id]
            )
            .is_err());
        db.execute(
            "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
            params![analysis_id, one.id],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE analysis_revisions SET content_hash = ?1 WHERE id = ?2",
                params!["c".repeat(64), analysis_id]
            )
            .is_err());
        assert!(db
            .execute(
                "DELETE FROM analysis_revisions WHERE id = ?1",
                params![analysis_id]
            )
            .is_err());
        let child_id = Uuid::new_v4().to_string();
        assert!(db.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, parent_revision_id, revision_no, parent_content_hash, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, 2, ?4, 'engine', 'candidate', ?5, '{}', '{}', ?6, 1, 0)",
            params![child_id, one.id, analysis_id, "wrong".to_owned(), "a".repeat(64), "d".repeat(64)],
        ).is_err());
    }

    #[test]
    fn recording_tracks_and_chunks_cannot_cross_meetings() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let session_id = Uuid::new_v4().to_string();
        let track_id = Uuid::new_v4().to_string();
        let insert_session = "INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES (?1, ?2, 'preparing', 'system', '{}', ?3, 0, 0)";
        {
            let db = storage.connection.lock().unwrap();
            assert!(db
                .execute(insert_session, params![session_id, one.id, "start-one"])
                .is_err());
            db.execute(
                "UPDATE meetings SET source_type = 'recording' WHERE id IN (?1, ?2)",
                params![one.id, two.id],
            )
            .unwrap();
            db.execute(insert_session, params![session_id, one.id, "start-one"])
                .unwrap();
            let insert_track = "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'mic', 'flac', 48000, 1, 'ready', 0, 0)";
            assert!(db
                .execute(insert_track, params![track_id, two.id, session_id])
                .is_err());
            db.execute(insert_track, params![track_id, one.id, session_id])
                .unwrap();
            db.execute(
                "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('derived-track', ?1, ?2, 'processed', 'flac', 48000, 1, 'ready', 0, 0)",
                params![one.id, session_id],
            ).unwrap();
            db.execute("INSERT INTO track_inputs (meeting_id, derived_track_id, input_track_id, role) VALUES (?1, 'derived-track', ?2, 'source')", params![one.id, track_id]).unwrap();
            assert!(db.execute("INSERT INTO track_inputs (meeting_id, derived_track_id, input_track_id, role) VALUES (?1, ?2, 'derived-track', 'source')", params![one.id, track_id]).is_err());
            assert!(db
                .execute(
                    "UPDATE meetings SET source_type = 'audio_import' WHERE id = ?1",
                    params![one.id]
                )
                .is_err());
        }
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", one.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio").unwrap();
        let asset = storage
            .register_ready_meeting_asset(&one.id, &relative, "audio/flac")
            .unwrap();
        let db = storage.connection.lock().unwrap();
        let chunk_id = Uuid::new_v4().to_string();
        let insert_chunk = "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES (?1, ?2, ?3, ?4, 0, 0, 1000000, 48000, 48000, 1, 'ready', 0)";
        assert!(db
            .execute(insert_chunk, params![chunk_id, two.id, track_id, asset.id])
            .is_err());
        db.execute(insert_chunk, params![chunk_id, one.id, track_id, asset.id])
            .unwrap();
        assert!(db
            .execute(
                insert_chunk,
                params![Uuid::new_v4().to_string(), one.id, track_id, asset.id]
            )
            .is_err());
        let problems: i64 = db
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(problems, 0);
    }

    #[test]
    fn transcript_references_are_version_scoped_and_frozen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("content").unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type = 'recording' WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', 0, 0)", params![meeting.id]).unwrap();
        db.execute("INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'flac', 48000, 1, 'ready', 0, 0)", params![meeting.id]).unwrap();
        for (id, revision_no, hash) in [("analysis-1", 1, "b"), ("analysis-2", 2, "c")] {
            db.execute("INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, 'engine', 'candidate', ?4, '{}', '{}', ?5, 1, 0)", params![id, meeting.id, revision_no, "a".repeat(64), hash.repeat(64)]).unwrap();
        }
        db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('speaker-1', 'analysis-1', 'lineage-1', '話者1', 'all', 'unknown', 0)", []).unwrap();
        db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('speaker-2', 'analysis-2', 'lineage-2', '話者2', 'all', 'unknown', 0)", []).unwrap();
        db.execute("INSERT INTO utterances (id, meeting_id, analysis_revision_id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state) VALUES ('utterance-1', ?1, 'analysis-1', 'track', 'utterance-lineage', 0, 100, 1000, '発言', '発言', 'assigned')", params![meeting.id]).unwrap();
        assert!(db.execute("INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES ('utterance-1', 'speaker-2', 'analysis-1', 'primary', 'inferred')", []).is_err());
        db.execute("INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES ('utterance-1', 'speaker-1', 'analysis-1', 'primary', 'inferred')", []).unwrap();
        assert!(db.execute("INSERT INTO speaker_segments (id, meeting_id, analysis_revision_id, track_id, speaker_id, start_us, end_us) VALUES ('segment-bad', ?1, 'analysis-1', 'track', 'speaker-2', 100, 1000)", params![meeting.id]).is_err());
        db.execute(
            "UPDATE analysis_revisions SET publication_state = 'committed' WHERE id = 'analysis-1'",
            [],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE utterances SET effective_text = '改変' WHERE id = 'utterance-1'",
                []
            )
            .is_err());
        assert!(db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('late', 'analysis-1', 'late', '後付け', 'all', 'unknown', 0)", []).is_err());
    }
}
