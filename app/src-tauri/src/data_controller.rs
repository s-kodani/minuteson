//! Main-window data operations retain server-owned confirmation capabilities.
//! Revisioned snapshots survive lost RPC/event responses; client paths and IDs
//! never substitute for the verified selection. No work runs on the UI thread.
use crate::analysis_worker::AnalysisController;
use crate::storage::backup::{archive::BackupReceipt, operations::*};
#[cfg(any(unix, windows))]
use crate::storage::deletion::audio::operations as audio_deletion;
#[cfg(any(unix, windows))]
use crate::storage::deletion::models::operations as model_deletion;
#[cfg(any(unix, windows))]
use crate::storage::deletion::operations as deletion;
use crate::storage::Storage;
use mon_contracts::{AppError, DeletionKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use uuid::Uuid;

#[cfg(any(unix, windows))]
mod automatic;
mod dialog;
mod work;
#[cfg(any(unix, windows))]
pub(crate) use work::{AudioTarget, ModelTarget};
use work::{Completion, Work};

#[cfg(any(unix, windows))]
pub(crate) fn finish_interrupted_deletions(app: &AppHandle) -> Result<(), String> {
    automatic::run(app)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Backup,
    Restore,
    Relocation,
    Deletion,
    AudioDeletion,
    ModelDeletion,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MovePreview {
    pub confirmation_id: String,
    pub operation_id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub meeting_count: usize,
    pub file_count: Option<usize>,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub pending: bool,
    pub resume_allowed: bool,
    pub resume_reason: Option<String>,
    pub copied_paths: Option<usize>,
    pub expected_paths: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoveReceipt {
    pub operation_id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub restart_required: bool,
    pub retained: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Refresh,
    Publish,
    Keep,
    DeleteIncludingUnregistered,
    ReviewRemaining,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub token: String,
    pub confirmation_id: String,
    pub action: Action,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub token: Option<String>,
    pub kind: Option<Kind>,
    pub phase: String,
    pub busy: bool,
    pub progress: Option<crate::operation_observation::Progress>,
    pub backup_preview: Option<BackupPreview>,
    pub restore_preview: Option<RestorePreview>,
    pub backup_receipt: Option<BackupReceipt>,
    pub restore_receipt: Option<RestoreReceipt>,
    pub relocation_preview: Option<MovePreview>,
    pub relocation_receipt: Option<MoveReceipt>,
    #[cfg(any(unix, windows))]
    pub deletion_preview: Option<deletion::Preview>,
    #[cfg(any(unix, windows))]
    pub deletion_receipt: Option<deletion::Receipt>,
    #[cfg(any(unix, windows))]
    pub audio_deletion_preview: Option<audio_deletion::Preview>,
    #[cfg(any(unix, windows))]
    pub audio_deletion_receipt: Option<audio_deletion::Receipt>,
    #[cfg(any(unix, windows))]
    pub model_deletion_preview: Option<model_deletion::Preview>,
    #[cfg(any(unix, windows))]
    pub model_deletion_receipt: Option<model_deletion::Receipt>,
    pub restart_required: bool,
    pub error_message: Option<String>,
    pub attention_required: bool,
    pub retry_allowed: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            token: None,
            kind: None,
            phase: "idle".into(),
            busy: false,
            progress: None,
            backup_preview: None,
            restore_preview: None,
            backup_receipt: None,
            restore_receipt: None,
            relocation_preview: None,
            relocation_receipt: None,
            #[cfg(any(unix, windows))]
            deletion_preview: None,
            #[cfg(any(unix, windows))]
            deletion_receipt: None,
            #[cfg(any(unix, windows))]
            audio_deletion_preview: None,
            #[cfg(any(unix, windows))]
            audio_deletion_receipt: None,
            #[cfg(any(unix, windows))]
            model_deletion_preview: None,
            #[cfg(any(unix, windows))]
            model_deletion_receipt: None,
            restart_required: false,
            error_message: None,
            attention_required: false,
            retry_allowed: false,
        }
    }
}

enum Selection {
    Backup(PreparedBackup),
    Restore(PreparedRestore),
    #[cfg(any(target_os = "macos", windows))]
    Relocation(crate::bootstrap::movement::Prepared),
    #[cfg(any(unix, windows))]
    Deletion(deletion::Prepared),
    #[cfg(any(unix, windows))]
    AudioDeletion(audio_deletion::Prepared),
    #[cfg(any(unix, windows))]
    ModelDeletion(model_deletion::Prepared),
}

impl Selection {
    fn confirmation_id(&self) -> String {
        match self {
            Self::Backup(p) => p.preview().confirmation_id,
            Self::Restore(p) => p.preview().confirmation_id,
            #[cfg(any(target_os = "macos", windows))]
            Self::Relocation(p) => p.preview().confirmation_id,
            #[cfg(any(unix, windows))]
            Self::Deletion(p) => p.preview().confirmation_id,
            #[cfg(any(unix, windows))]
            Self::AudioDeletion(p) => p.preview().confirmation_id,
            #[cfg(any(unix, windows))]
            Self::ModelDeletion(p) => p.preview().confirmation_id,
        }
    }
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    selection: Option<Selection>,
    request: Option<Request>,
    cancel: Arc<AtomicBool>,
    closing: bool,
    started_at_ms: i64,
    jobs: std::collections::BTreeMap<String, DataJob>,
}

/// Session-local job results. Durable publication/recovery remains in Storage.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataJob {
    snapshot: Snapshot,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
}

#[derive(Default)]
pub(crate) struct DataController {
    state: Mutex<State>,
    idle: Condvar,
}

impl DataController {
    pub(crate) fn list_jobs(&self) -> Result<Vec<DataJob>, String> {
        let s = self
            .state
            .lock()
            .map_err(|_| "データ操作の状態を取得できません")?;
        let mut jobs = s.jobs.clone();
        if matches!(s.snapshot.kind, Some(Kind::Backup | Kind::Restore))
            && !matches!(
                s.snapshot.phase.as_str(),
                "idle" | "choosing" | "discarding"
            )
            && (s.snapshot.busy || s.snapshot.phase == "ready")
        {
            if let Some(token) = &s.snapshot.token {
                jobs.insert(
                    token.clone(),
                    DataJob {
                        snapshot: s.snapshot.clone(),
                        started_at_ms: s.started_at_ms,
                        finished_at_ms: None,
                    },
                );
            }
        }
        let mut jobs: Vec<_> = jobs.into_values().collect();
        jobs.sort_by_key(|j| j.started_at_ms);
        Ok(jobs)
    }

    pub(crate) fn dismiss_job(&self, token: &str) -> Result<(), String> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| "データ操作の状態を取得できません")?;
        if s.snapshot.token.as_deref() == Some(token)
            && (s.snapshot.busy || matches!(s.snapshot.phase.as_str(), "ready" | "attention"))
        {
            return Err("進行中・確認待ちのジョブは閉じられません".into());
        }
        s.jobs.remove(token);
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "データ操作の状態を取得できません")?
            .snapshot
            .clone())
    }

    fn begin(&self, kind: Kind) -> Result<(Snapshot, Arc<AtomicBool>, Option<Selection>), String> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| "データ操作ロックを取得できません")?;
        if s.closing
            || s.snapshot.busy
            || (s.snapshot.restart_required && kind != Kind::ModelDeletion)
        {
            return Err("バックアップ・復元・終了処理が進行中です".into());
        }
        if matches!(s.snapshot.kind, Some(Kind::Backup | Kind::Restore))
            && s.snapshot.phase == "ready"
        {
            if let Some(token) = s.snapshot.token.clone() {
                let mut snapshot = s.snapshot.clone();
                snapshot.phase = "cancelled".into();
                snapshot.retry_allowed = false;
                snapshot.progress = None;
                let job = DataJob {
                    snapshot,
                    started_at_ms: s.started_at_ms,
                    finished_at_ms: Some(chrono::Utc::now().timestamp_millis()),
                };
                s.jobs.insert(token, job);
            }
        }
        let previous = s.selection.take();
        s.started_at_ms = chrono::Utc::now().timestamp_millis();
        s.snapshot = Snapshot {
            revision: s.snapshot.revision + 1,
            token: Some(Uuid::new_v4().to_string()),
            kind: Some(kind),
            phase: "choosing".into(),
            busy: true,
            ..Snapshot::default()
        };
        s.cancel = Arc::new(AtomicBool::new(false));
        s.request = None;
        Ok((s.snapshot.clone(), s.cancel.clone(), previous))
    }

    fn request_replay(s: &State, request: &Request) -> Result<bool, String> {
        if s.snapshot.token.as_deref() != Some(&request.token) {
            return Err(
                "データ操作の選択が変更されています。現在の状態を再確認してください".into(),
            );
        }
        // Check an accepted request BEFORE its old confirmation: refreshing
        // mints a new ID, and a lost refresh response still has to be replayable.
        #[cfg(any(unix, windows))]
        let deletion_completed = s.snapshot.deletion_receipt.is_some()
            || s.snapshot.audio_deletion_receipt.is_some()
            || s.snapshot.model_deletion_receipt.is_some();
        #[cfg(not(any(unix, windows)))]
        let deletion_completed = false;
        if s.request.as_ref() == Some(request)
            && (s.snapshot.busy
                || s.snapshot.backup_receipt.is_some()
                || s.snapshot.restore_receipt.is_some()
                || s.snapshot.relocation_receipt.is_some()
                || deletion_completed
                || s.snapshot.attention_required
                || (matches!(request.action, Action::Refresh | Action::ReviewRemaining)
                    && s.snapshot.phase == "ready"))
        {
            return Ok(true);
        }
        #[cfg(any(unix, windows))]
        let deletion_resolution = matches!(
            (&s.selection, request.action),
            (Some(Selection::Deletion(_)), Action::Refresh)
        ) || matches!((&s.selection,request.action),(Some(Selection::Deletion(p)),Action::Keep) if p.preview().keep_allowed)
            || matches!((&s.selection,request.action),(Some(Selection::Deletion(p)),Action::ReviewRemaining) if p.preview().reconfirmation_allowed);
        #[cfg(any(unix, windows))]
        let deletion_resolution = deletion_resolution
            || matches!(
                (&s.selection, request.action),
                (Some(Selection::AudioDeletion(_)), Action::Refresh)
            )
            || matches!((&s.selection, request.action), (Some(Selection::AudioDeletion(p)), Action::Keep) if p.preview().keep_allowed)
            || matches!((&s.selection, request.action), (Some(Selection::AudioDeletion(p)), Action::ReviewRemaining) if p.preview().reconfirmation_allowed);
        #[cfg(any(unix, windows))]
        let deletion_resolution = deletion_resolution
            || matches!(
                (&s.selection, request.action),
                (Some(Selection::ModelDeletion(_)), Action::Refresh)
            )
            || matches!((&s.selection, request.action), (Some(Selection::ModelDeletion(p)), Action::Keep) if p.preview().keep_allowed)
            || matches!((&s.selection, request.action), (Some(Selection::ModelDeletion(p)), Action::ReviewRemaining) if p.preview().reconfirmation_allowed);
        #[cfg(not(any(unix, windows)))]
        let deletion_resolution = false;
        #[cfg(any(unix, windows))]
        let deletion_resume = (matches!(&s.selection, Some(Selection::Deletion(p)) if p.preview().pending)
            || matches!(&s.selection, Some(Selection::AudioDeletion(p)) if p.preview().pending)
            || matches!(&s.selection, Some(Selection::ModelDeletion(p)) if p.preview().pending))
            && !deletion_completed;
        #[cfg(not(any(unix, windows)))]
        let deletion_resume = false;
        if s.closing
            || s.snapshot.busy
            || (s.snapshot.attention_required && !deletion_resolution)
            || (s.snapshot.restart_required && !deletion_resume)
        {
            return Err("別のデータ操作または終了処理が進行中です。結果を確認してください".into());
        }
        let selection = s
            .selection
            .as_ref()
            .ok_or("確認できるバックアップがありません。選び直してください")?;
        if selection.confirmation_id() != request.confirmation_id {
            return Err("確認内容が変更されています。現在の確認内容を使用してください".into());
        }
        let allowed = matches!(
            (selection, request.action),
            (Selection::Backup(_), Action::Publish)
                | (Selection::Restore(_), Action::Publish | Action::Refresh)
        );
        #[cfg(any(target_os = "macos", windows))]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::Relocation(_), Action::Refresh) => true,
                (Selection::Relocation(p), Action::Publish) => p.preview().resume_allowed,
                (Selection::Relocation(p), Action::Keep) => p.preview().pending,
                _ => false,
            };
        #[cfg(any(unix, windows))]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::Deletion(_), Action::Refresh) => true,
                (Selection::Deletion(p), Action::ReviewRemaining) => {
                    p.preview().reconfirmation_allowed
                }
                (Selection::Deletion(p), Action::Keep) => p.preview().keep_allowed,
                (Selection::Deletion(p), Action::Publish) => {
                    let v = p.preview();
                    // A newly observed scope needs its own explicit choice;
                    // fixed file plans already retain that recorded policy.
                    v.execute_allowed
                        && !(v.unregistered_count > 0
                            && matches!(
                                v.phase.as_str(),
                                "review"
                                    | "not_started"
                                    | "reconfirmation"
                                    | "database_reconfirmation"
                            ))
                }
                (Selection::Deletion(p), Action::DeleteIncludingUnregistered) => {
                    let v = p.preview();
                    v.execute_allowed
                        && matches!(
                            v.phase.as_str(),
                            "review" | "not_started" | "reconfirmation" | "database_reconfirmation"
                        )
                        && v.unregistered_count > 0
                }
                _ => false,
            };
        #[cfg(any(unix, windows))]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::AudioDeletion(_), Action::Refresh) => true,
                (Selection::AudioDeletion(p), Action::Keep) => p.preview().keep_allowed,
                (Selection::AudioDeletion(p), Action::ReviewRemaining) => {
                    p.preview().reconfirmation_allowed
                }
                (Selection::AudioDeletion(p), Action::Publish) => {
                    let v = p.preview();
                    v.execute_allowed
                        && !(v.unregistered_audio_files > 0
                            && matches!(v.phase.as_str(), "review" | "reconfirmation"))
                }
                (Selection::AudioDeletion(p), Action::DeleteIncludingUnregistered) => {
                    let v = p.preview();
                    v.execute_allowed
                        && v.unregistered_audio_files > 0
                        && matches!(v.phase.as_str(), "review" | "reconfirmation")
                }
                _ => false,
            };
        #[cfg(any(unix, windows))]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::ModelDeletion(_), Action::Refresh) => true,
                (Selection::ModelDeletion(p), Action::Keep) => p.preview().keep_allowed,
                (Selection::ModelDeletion(p), Action::ReviewRemaining) => {
                    p.preview().reconfirmation_allowed
                }
                (
                    Selection::ModelDeletion(p),
                    action @ (Action::Publish | Action::DeleteIncludingUnregistered),
                ) => {
                    let v = p.preview();
                    let new_scope = matches!(
                        v.phase.as_str(),
                        "review" | "not_started" | "reconfirmation"
                    );
                    v.execute_allowed
                        && if action == Action::Publish {
                            !(new_scope && v.unregistered_count > 0)
                        } else {
                            new_scope && v.unregistered_count > 0
                        }
                }
                _ => false,
            };
        if !allowed {
            return Err("確認した対象と実行する操作が一致しません".into());
        }
        if s.snapshot.kind == Some(Kind::Relocation)
            && matches!(request.action, Action::Publish | Action::Keep)
            && s.snapshot.phase != "ready"
        {
            return Err("保存先変更の確認が失効しています。同じ対象を再確認してください".into());
        }
        if matches!(
            s.snapshot.kind,
            Some(Kind::Deletion | Kind::AudioDeletion | Kind::ModelDeletion)
        ) && matches!(
            request.action,
            Action::Publish | Action::DeleteIncludingUnregistered
        ) && s.snapshot.phase != "ready"
        {
            return Err("削除確認が失効しています。残りの状態を再検査してください".into());
        }
        if s.snapshot.backup_receipt.is_some()
            || s.snapshot.restore_receipt.is_some()
            || s.snapshot.relocation_receipt.is_some()
            || deletion_completed
        {
            return Err("この操作は完了済みです。別の操作を明示して選択してください".into());
        }
        Ok(false)
    }

    fn begin_request(
        &self,
        request: Request,
    ) -> Result<Option<(Work, Arc<AtomicBool>, Snapshot)>, String> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| "データ操作ロックを取得できません")?;
        if Self::request_replay(&s, &request)? {
            return Ok(None);
        }
        let selection = s.selection.take().ok_or("確認済みの選択がありません")?;
        s.snapshot.busy = true;
        s.snapshot.phase = if matches!(request.action, Action::Refresh | Action::ReviewRemaining) {
            "verifying"
        } else {
            "publishing"
        }
        .into();
        s.snapshot.error_message = None;
        s.snapshot.retry_allowed = false;
        s.snapshot.revision += 1;
        s.cancel = Arc::new(AtomicBool::new(false));
        s.request = Some(request.clone());
        Ok(Some((
            Work::Request(selection, request),
            s.cancel.clone(),
            s.snapshot.clone(),
        )))
    }

    fn progress(&self, token: &str, phase: &str) -> Option<Snapshot> {
        let mut s = self.state.lock().ok()?;
        if s.snapshot.token.as_deref() != Some(token)
            || !s.snapshot.busy
            || s.cancel.load(Ordering::Acquire)
        {
            return None;
        }
        if s.snapshot.phase == phase {
            return None;
        }
        s.snapshot.phase = phase.into();
        s.snapshot.revision += 1;
        Some(s.snapshot.clone())
    }

    fn finish(&self, token: &str, completion: Completion) -> Option<Snapshot> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.snapshot.token.as_deref() != Some(token) || !s.snapshot.busy {
            return None;
        }
        s.selection = completion.selection;
        s.snapshot.backup_receipt = completion.backup;
        s.snapshot.restore_receipt = completion.restore;
        s.snapshot.relocation_receipt = completion.relocation;
        #[cfg(any(unix, windows))]
        {
            s.snapshot.deletion_receipt = completion.deletion;
            s.snapshot.audio_deletion_receipt = completion.audio_deletion;
            s.snapshot.model_deletion_receipt = completion.model_deletion;
        }
        s.snapshot.restart_required |= completion.restart_required;
        s.snapshot.attention_required = completion.attention;
        s.snapshot.error_message = completion.error;
        #[cfg(any(unix, windows))]
        let deletion_completed = s.snapshot.deletion_receipt.is_some()
            || s.snapshot.audio_deletion_receipt.is_some()
            || s.snapshot.model_deletion_receipt.is_some();
        #[cfg(not(any(unix, windows)))]
        let deletion_completed = false;
        let published = s.snapshot.backup_receipt.is_some()
            || s.snapshot.restore_receipt.is_some()
            || s.snapshot.relocation_receipt.is_some()
            || deletion_completed;
        // Publication wins over late cancellation. Never label a committed
        // restore cancelled or destroy its receipt after a successful commit.
        s.snapshot.phase = if published {
            "completed"
        } else if completion.attention {
            "attention"
        } else if completion.cancelled || s.cancel.load(Ordering::Acquire) {
            "cancelled"
        } else if s.snapshot.error_message.is_some() {
            "failed"
        } else if s.selection.is_some() {
            "ready"
        } else {
            "idle"
        }
        .into();
        s.snapshot.backup_preview = match &s.selection {
            Some(Selection::Backup(p)) => Some(p.preview()),
            _ => None,
        };
        s.snapshot.restore_preview = match &s.selection {
            Some(Selection::Restore(p)) => Some(p.preview()),
            _ => None,
        };
        s.snapshot.relocation_preview = match &s.selection {
            #[cfg(any(target_os = "macos", windows))]
            Some(Selection::Relocation(p)) => Some(p.preview()),
            _ => None,
        };
        #[cfg(any(unix, windows))]
        {
            s.snapshot.deletion_preview = match &s.selection {
                Some(Selection::Deletion(p)) => Some(p.preview()),
                _ => None,
            };
            s.snapshot.model_deletion_preview = match &s.selection {
                Some(Selection::ModelDeletion(p)) => Some(p.preview()),
                _ => None,
            };
            s.snapshot.audio_deletion_preview = match &s.selection {
                Some(Selection::AudioDeletion(p)) => Some(p.preview()),
                _ => None,
            };
        }
        s.snapshot.retry_allowed = s.selection.is_some() && !published && !completion.attention;
        s.snapshot.busy = false;
        s.snapshot.revision += 1;
        if matches!(s.snapshot.kind, Some(Kind::Backup | Kind::Restore))
            && s.snapshot.phase != "idle"
        {
            if let Some(token) = s.snapshot.token.clone() {
                let finished_at_ms = (!matches!(s.snapshot.phase.as_str(), "ready" | "attention"))
                    .then(|| chrono::Utc::now().timestamp_millis());
                let job = DataJob {
                    snapshot: s.snapshot.clone(),
                    started_at_ms: s.started_at_ms,
                    finished_at_ms,
                };
                s.jobs.insert(token, job);
            }
        }
        s.snapshot.progress = None;
        self.idle.notify_all();
        Some(s.snapshot.clone())
    }

    fn emit(app: &AppHandle, snapshot: Snapshot) {
        let category = snapshot
            .kind
            .map(|kind| format!("{kind:?}").to_lowercase())
            .unwrap_or_else(|| "data".into());
        crate::internal_logs::phase(
            &category,
            snapshot.token.clone(),
            None,
            &snapshot.phase,
            snapshot.error_message.as_deref(),
        );
        let _ = app.emit("data-operation-changed", snapshot);
    }

    pub(crate) async fn select(window: &WebviewWindow, kind: Kind) -> Result<Snapshot, String> {
        if matches!(
            kind,
            Kind::Deletion | Kind::AudioDeletion | Kind::ModelDeletion
        ) {
            return Err("会議または要確認一覧から対象を選択してください".into());
        }
        let app = window.app_handle().clone();
        let admission_app = app.clone();
        let lease =
            tauri::async_runtime::spawn_blocking(move || work::Admission::acquire(&admission_app))
                .await
                .map_err(|e| e.to_string())??;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(kind)?;
        let token = snapshot.token.clone().ok_or("データ操作IDがありません")?;
        Self::emit(&app, snapshot.clone());
        let selected_window = window.clone();
        // Keep the lease and busy state until the actual dialog response and
        // owned prior selection cleanup complete, even if the RPC disappears.
        tauri::async_runtime::spawn(async move {
            let path = dialog::select(&selected_window, kind).await;
            Self::launch(
                &app,
                token,
                cancel,
                lease,
                Work::Select(kind, path, previous),
            );
        });
        Ok(snapshot)
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(crate) fn review_move(app: &AppHandle) -> Result<Snapshot, String> {
        let lease = work::Admission::acquire(app)?;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(Kind::Relocation)?;
        let token = snapshot.token.clone().ok_or("確認操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        Self::launch(app, token, cancel, lease, Work::PendingMove(previous));
        Ok(snapshot)
    }

    pub(crate) fn start(app: &AppHandle, request: Request) -> Result<Snapshot, AppError> {
        let request_id = request.token.clone();
        #[cfg(any(unix, windows))]
        let mut deletion_admission = None;
        #[cfg(any(unix, windows))]
        let mut audio_admission = None;
        #[cfg(any(unix, windows))]
        let mut model_admission = None;
        {
            let controller = app.state::<Self>();
            let s = controller
                .state
                .lock()
                .map_err(|_| "データ操作ロックを取得できません")?;
            if Self::request_replay(&s, &request)? {
                return Ok(s.snapshot.clone());
            }
            #[cfg(any(unix, windows))]
            if let Some(Selection::Deletion(p)) = &s.selection {
                let v = p.preview();
                deletion_admission = Some((
                    v.meeting_id,
                    if v.pending || app.state::<Storage>().data_write_blocked() {
                        Some(v.operation_id)
                    } else {
                        None
                    },
                ));
            }
            #[cfg(any(unix, windows))]
            if let Some(Selection::ModelDeletion(p)) = &s.selection {
                let v = p.preview();
                model_admission = Some(
                    if v.pending || app.state::<Storage>().data_write_blocked() {
                        Some(v.operation_id)
                    } else {
                        None
                    },
                );
            }
            #[cfg(any(unix, windows))]
            if let Some(Selection::AudioDeletion(p)) = &s.selection {
                audio_admission = Some(p.admission_operation().map(str::to_owned));
            }
        }
        #[cfg(any(unix, windows))]
        let lease = if let Some(operation) = model_admission {
            work::Admission::acquire_model_deletion(app, operation.as_deref()).map_err(|e| {
                AppError::not_started(
                    DeletionKind::Model,
                    e,
                    operation.clone().or_else(|| Some(request_id.clone())),
                )
            })?
        } else if let Some(operation) = audio_admission {
            work::Admission::acquire_audio_deletion(app, operation.as_deref()).map_err(|e| {
                AppError::not_started(
                    DeletionKind::Audio,
                    e,
                    operation.clone().or_else(|| Some(request_id.clone())),
                )
            })?
        } else if let Some((meeting, operation)) = deletion_admission {
            work::Admission::acquire_meeting_deletion(app, &meeting, operation.as_deref()).map_err(
                |e| {
                    AppError::not_started(
                        DeletionKind::Meeting,
                        e,
                        operation.clone().or_else(|| Some(request_id.clone())),
                    )
                },
            )?
        } else {
            work::Admission::acquire(app)?
        };
        #[cfg(not(any(unix, windows)))]
        let lease = work::Admission::acquire(app)?;
        if let Some((work, cancel, snapshot)) = app.state::<Self>().begin_request(request)? {
            let token = snapshot.token.clone().ok_or("データ操作IDがありません")?;
            Self::emit(app, snapshot.clone());
            Self::launch(app, token, cancel, lease, work);
            Ok(snapshot)
        } else {
            app.state::<Self>().snapshot().map_err(AppError::from)
        }
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn review_deletion(
        app: &AppHandle,
        id: String,
        pending: bool,
    ) -> Result<Snapshot, AppError> {
        let meeting = if pending {
            app.state::<Storage>().deletion_operation_meeting(&id)?
        } else {
            id.clone()
        };
        let lease = work::Admission::acquire_meeting_deletion(
            app,
            &meeting,
            pending.then_some(id.as_str()),
        )
        .map_err(|e| {
            AppError::not_started(DeletionKind::Meeting, e, pending.then(|| id.clone()))
        })?;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(Kind::Deletion)?;
        let token = snapshot
            .token
            .clone()
            .ok_or("削除確認の操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        Self::launch(
            app,
            token,
            cancel,
            lease,
            Work::Deletion(id, pending, previous),
        );
        Ok(snapshot)
    }
    #[cfg(any(unix, windows))]
    pub(crate) fn deletion_page(
        &self,
        confirmation: &str,
        offset: usize,
    ) -> Result<deletion::PathPage, String> {
        let s = self
            .state
            .lock()
            .map_err(|_| "削除一覧のロックを取得できません")?;
        if s.snapshot.busy {
            return Err("削除対象の検査・実行中です".into());
        }
        match &s.selection {
            Some(Selection::Deletion(p)) => p.page(confirmation, offset),
            _ => Err("確認済みの削除対象がありません".into()),
        }
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn review_audio_deletion(
        app: &AppHandle,
        target: work::AudioTarget,
    ) -> Result<Snapshot, AppError> {
        let operation = match &target {
            work::AudioTarget::Pending(id) => Some(id.as_str()),
            _ => None,
        };
        let lease = work::Admission::acquire_audio_deletion(app, operation).map_err(|e| {
            AppError::not_started(DeletionKind::Audio, e, operation.map(str::to_owned))
        })?;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(Kind::AudioDeletion)?;
        let token = snapshot
            .token
            .clone()
            .ok_or("音声削除確認の操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        Self::launch(
            app,
            token,
            cancel,
            lease,
            Work::AudioDeletion(target, previous),
        );
        Ok(snapshot)
    }
    #[cfg(any(unix, windows))]
    pub(crate) fn audio_deletion_page(
        &self,
        confirmation: &str,
        offset: usize,
    ) -> Result<audio_deletion::PathPage, String> {
        let s = self
            .state
            .lock()
            .map_err(|_| "音声削除一覧のロックを取得できません")?;
        if s.snapshot.busy {
            return Err("音声削除の検査・実行中です".into());
        }
        match &s.selection {
            Some(Selection::AudioDeletion(p)) => p.page(confirmation, offset),
            _ => Err("確認済みの音声削除対象がありません".into()),
        }
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn review_model_deletion(
        app: &AppHandle,
        target: work::ModelTarget,
    ) -> Result<Snapshot, AppError> {
        let operation = match &target {
            work::ModelTarget::Pending(id) => Some(id.clone()),
            _ => None,
        };
        let root = app.state::<Storage>().native_models_root().to_owned();
        let (snapshot, cancel, previous) = app
            .state::<Self>()
            .begin(Kind::ModelDeletion)
            .map_err(|e| AppError::not_started(DeletionKind::Model, e, operation))?;
        let token = snapshot
            .token
            .clone()
            .ok_or("モデル削除の操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        let acquisition_target = target.clone();
        Self::launch_with_admission(
            app,
            token,
            cancel,
            move |app, cancelled| {
                let operation = match &acquisition_target {
                    work::ModelTarget::Pending(id) => Some(id.as_str()),
                    _ => None,
                };
                work::Admission::acquire_model_deletion_cancelled(app, operation, cancelled)
            },
            Work::ModelDeletion(target, root, previous),
        );
        Ok(snapshot)
    }
    #[cfg(any(unix, windows))]
    pub(crate) fn model_deletion_page(
        &self,
        confirmation: &str,
        offset: usize,
    ) -> Result<model_deletion::PathPage, String> {
        let s = self
            .state
            .lock()
            .map_err(|_| "モデル削除一覧のロックを取得できません")?;
        if s.snapshot.busy {
            return Err("モデル削除の検査・実行中です".into());
        }
        match &s.selection {
            Some(Selection::ModelDeletion(p)) => p.page(confirmation, offset),
            _ => Err("確認済みのモデル削除対象がありません".into()),
        }
    }

    fn launch(
        app: &AppHandle,
        token: String,
        cancel: Arc<AtomicBool>,
        lease: work::Admission,
        work: Work,
    ) {
        Self::launch_with_admission(app, token, cancel, move |_, _| Ok(lease), work);
    }

    fn launch_with_admission(
        app: &AppHandle,
        token: String,
        cancel: Arc<AtomicBool>,
        acquire: impl FnOnce(&AppHandle, &dyn Fn() -> bool) -> Result<work::Admission, String>
            + Send
            + 'static,
        work: Work,
    ) {
        let worker = app.clone();
        let worker_token = token.clone();
        let spawned = std::thread::Builder::new()
            .name("mon-data-operation".into())
            .spawn(move || {
                let progress_app = worker.clone();
                let progress_token = worker_token.clone();
                let _observations =
                    crate::operation_observation::Scope::enter("data", move |value| {
                        let controller = progress_app.state::<Self>();
                        let snapshot = {
                            let mut s = controller.state.lock().unwrap_or_else(|e| e.into_inner());
                            if s.snapshot.token.as_deref() != Some(&progress_token)
                                || !s.snapshot.busy
                                || s.cancel.load(Ordering::Acquire)
                            {
                                return;
                            }
                            s.snapshot.progress = Some(value);
                            s.snapshot.revision += 1;
                            s.snapshot.clone()
                        };
                        Self::emit(&progress_app, snapshot);
                    });
                #[cfg(any(unix, windows))]
                let _verification = crate::storage::deletion::verification::Scope::enter();
                crate::operation_observation::stage("waiting", None, "bytes");
                let lease = match acquire(&worker, &|| cancel.load(Ordering::Acquire)) {
                    Ok(lease) => lease,
                    Err(error) => {
                        if let Some(snapshot) = worker
                            .state::<Self>()
                            .finish(&worker_token, Completion::failed(error))
                        {
                            Self::emit(&worker, snapshot);
                        }
                        return;
                    }
                };
                let cancelled = || cancel.load(Ordering::Acquire) || lease.cancelled();
                let progress = |phase: &str| {
                    crate::operation_observation::stage(phase, None, "bytes");
                    if let Some(snapshot) = worker.state::<Self>().progress(&worker_token, phase) {
                        Self::emit(&worker, snapshot);
                    }
                };
                #[cfg(any(target_os = "macos", windows))]
                let completion = work.run_with_bootstrap_admitted(
                    &worker.state::<Storage>(),
                    &worker.state::<crate::bootstrap::Bootstrap>(),
                    &cancelled,
                    &progress,
                    lease.proof(),
                );
                #[cfg(not(any(target_os = "macos", windows)))]
                let completion = work.run_admitted(
                    &worker.state::<Storage>(),
                    &cancelled,
                    &progress,
                    lease.proof(),
                );
                // Set the permanent admission fence before publishing the result
                // or releasing the temporary lease, including ambiguous cutover.
                if worker.state::<Storage>().restart_required() {
                    worker
                        .state::<AnalysisController>()
                        .require_storage_restart();
                }
                if let Some(snapshot) = worker.state::<Self>().finish(&worker_token, completion) {
                    Self::emit(&worker, snapshot);
                }
                // Snapshot terminalization precedes admission release. Cancellation
                // or a disconnected webview cannot release a running writer.
                drop(lease);
            });
        if let Err(error) = spawned {
            if let Some(snapshot) = app.state::<Self>().finish(
                &token,
                Completion::failed(format!("データ処理を開始できません: {error}")),
            ) {
                Self::emit(app, snapshot);
            }
        }
    }

    pub(crate) fn cancel(app: &AppHandle, token: &str) -> Result<Snapshot, String> {
        let controller = app.state::<Self>();
        let (snapshot, discarded) = {
            let mut s = controller
                .state
                .lock()
                .map_err(|_| "データ操作ロックを取得できません")?;
            if s.snapshot.token.as_deref() != Some(token) {
                return Err("取消対象のデータ操作が変更されています".into());
            }
            if s.snapshot.busy {
                s.cancel.store(true, Ordering::Release);
                s.snapshot.phase = "cancelling".into();
                s.snapshot.revision += 1;
                (s.snapshot.clone(), None)
            } else if s.selection.is_some()
                && s.snapshot.backup_receipt.is_none()
                && s.snapshot.restore_receipt.is_none()
                && s.snapshot.relocation_receipt.is_none()
                && !s.snapshot.attention_required
                && {
                    #[cfg(any(unix, windows))]
                    {
                        s.snapshot.deletion_receipt.is_none()
                            && s.snapshot.model_deletion_receipt.is_none()
                            && !s
                                .snapshot
                                .model_deletion_preview
                                .as_ref()
                                .is_some_and(|p| p.pending)
                            && s.snapshot.audio_deletion_receipt.is_none()
                            && !s
                                .snapshot
                                .audio_deletion_preview
                                .as_ref()
                                .is_some_and(|p| p.pending)
                            && !s
                                .snapshot
                                .deletion_preview
                                .as_ref()
                                .is_some_and(|p| p.pending)
                    }
                    #[cfg(not(any(unix, windows)))]
                    {
                        true
                    }
                }
            {
                let discarded = s.selection.take();
                s.snapshot.busy = true;
                s.snapshot.phase = "discarding".into();
                s.snapshot.retry_allowed = false;
                s.snapshot.revision += 1;
                (s.snapshot.clone(), discarded)
            } else {
                (s.snapshot.clone(), None)
            }
        };
        Self::emit(app, snapshot.clone());
        if let Some(selection) = discarded {
            let worker = app.clone();
            let token = token.to_string();
            // Owned private copies can be large; cleanup also participates in
            // shutdown waiting but does not need the inference/write lane.
            tauri::async_runtime::spawn_blocking(move || {
                drop(selection);
                let mut completion = Completion::default();
                completion.cancelled = true;
                if let Some(snapshot) = worker.state::<Self>().finish(&token, completion) {
                    Self::emit(&worker, snapshot);
                }
            });
        }
        Ok(snapshot)
    }

    fn stop(&self, timeout: Duration) -> Result<Option<Selection>, String> {
        self.stop_checked(timeout, None)
    }
    fn stop_checked(
        &self,
        timeout: Duration,
        expected: Option<&Option<String>>,
    ) -> Result<Option<Selection>, String> {
        let started = Instant::now();
        let mut s = self
            .state
            .lock()
            .map_err(|_| "データ操作ロックを取得できません")?;
        if let Some(expected) = expected {
            crate::application_close::require_target(expected, &s.snapshot.token)?;
        }
        s.closing = true;
        s.cancel.store(true, Ordering::Release);
        while s.snapshot.busy {
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                s.closing = false;
                return Err(
                    "バックアップ／復元の安全な停止を待っています。完了後に終了してください".into(),
                );
            };
            s = self
                .idle
                .wait_timeout(s, remaining.min(Duration::from_millis(100)))
                .map_err(|_| "データ操作の停止待ちに失敗しました")?
                .0;
        }
        Ok(s.selection.take())
    }

    pub(crate) fn shutdown(app: &AppHandle) -> Result<(), String> {
        let selection = app.state::<Self>().stop(Duration::from_secs(15))?;
        drop(selection);
        Ok(())
    }
    pub(crate) fn shutdown_checked(
        app: &AppHandle,
        expected: &Option<String>,
    ) -> Result<(), String> {
        let selection = app
            .state::<Self>()
            .stop_checked(Duration::from_secs(15), Some(expected))?;
        drop(selection);
        Ok(())
    }

    pub(crate) fn abort_shutdown(app: &AppHandle) {
        let controller = app.state::<Self>();
        let mut s = controller.state.lock().unwrap_or_else(|e| e.into_inner());
        s.closing = false;
        if s.selection.is_none() {
            s.snapshot.backup_preview = None;
            s.snapshot.restore_preview = None;
            #[cfg(any(unix, windows))]
            if s.snapshot.deletion_receipt.is_none() {
                s.snapshot.deletion_preview = None;
            }
            #[cfg(any(unix, windows))]
            if s.snapshot.model_deletion_receipt.is_none() {
                s.snapshot.model_deletion_preview = None;
            }
            #[cfg(any(unix, windows))]
            if s.snapshot.audio_deletion_receipt.is_none() {
                s.snapshot.audio_deletion_preview = None;
            }
            if s.snapshot.relocation_receipt.is_none() {
                s.snapshot.relocation_preview = None;
            }
            s.snapshot.retry_allowed = false;
        }
        s.snapshot.revision += 1;
        Self::emit(app, s.snapshot.clone());
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod deletion_tests;

#[cfg(all(test, unix))]
mod audio_deletion_tests;

#[cfg(all(test, unix))]
pub(crate) mod restored_audio_synthetic;

#[cfg(all(test, any(unix, windows)))]
mod model_deletion_tests;
