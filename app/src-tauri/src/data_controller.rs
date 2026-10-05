//! Main-window data operations retain server-owned confirmation capabilities.
//! Revisioned snapshots survive lost RPC/event responses; client paths and IDs
//! never substitute for the verified selection. No work runs on the UI thread.
use crate::analysis_worker::AnalysisController;
use crate::storage::backup::recovery;
use crate::storage::backup::{archive::BackupReceipt, operations::*};
#[cfg(unix)]
use crate::storage::deletion::operations as deletion;
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use uuid::Uuid;

mod dialog;
mod work;
use work::{Completion, Work};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Backup,
    Restore,
    Recovery,
    Relocation,
    Deletion,
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
    Recover,
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
    pub backup_preview: Option<BackupPreview>,
    pub restore_preview: Option<RestorePreview>,
    pub backup_receipt: Option<BackupReceipt>,
    pub restore_receipt: Option<RestoreReceipt>,
    pub recovery_preview: Option<recovery::Preview>,
    pub recovery_receipt: Option<recovery::Receipt>,
    pub relocation_preview: Option<MovePreview>,
    pub relocation_receipt: Option<MoveReceipt>,
    #[cfg(unix)]
    pub deletion_preview: Option<deletion::Preview>,
    #[cfg(unix)]
    pub deletion_receipt: Option<deletion::Receipt>,
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
            backup_preview: None,
            restore_preview: None,
            backup_receipt: None,
            restore_receipt: None,
            recovery_preview: None,
            recovery_receipt: None,
            relocation_preview: None,
            relocation_receipt: None,
            #[cfg(unix)]
            deletion_preview: None,
            #[cfg(unix)]
            deletion_receipt: None,
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
    Recovery(recovery::Prepared),
    #[cfg(target_os = "macos")]
    Relocation(crate::bootstrap::movement::Prepared),
    #[cfg(unix)]
    Deletion(deletion::Prepared),
}

impl Selection {
    fn confirmation_id(&self) -> String {
        match self {
            Self::Backup(p) => p.preview().confirmation_id,
            Self::Restore(p) => p.preview().confirmation_id,
            Self::Recovery(p) => p.preview().confirmation_id,
            #[cfg(target_os = "macos")]
            Self::Relocation(p) => p.preview().confirmation_id,
            #[cfg(unix)]
            Self::Deletion(p) => p.preview().confirmation_id,
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
}

#[derive(Default)]
pub(crate) struct DataController {
    state: Mutex<State>,
    idle: Condvar,
}

impl DataController {
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
        if s.closing || s.snapshot.busy || s.snapshot.restart_required {
            return Err("バックアップ・復元・終了処理が進行中です".into());
        }
        let previous = s.selection.take();
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
        #[cfg(unix)]
        let deletion_completed = s.snapshot.deletion_receipt.is_some();
        #[cfg(not(unix))]
        let deletion_completed = false;
        if s.request.as_ref() == Some(request)
            && (s.snapshot.busy
                || s.snapshot.backup_receipt.is_some()
                || s.snapshot.restore_receipt.is_some()
                || s.snapshot.recovery_receipt.is_some()
                || s.snapshot.relocation_receipt.is_some()
                || deletion_completed
                || s.snapshot.attention_required
                || (matches!(request.action, Action::Refresh | Action::ReviewRemaining)
                    && s.snapshot.phase == "ready"))
        {
            return Ok(true);
        }
        #[cfg(unix)]
        let deletion_resolution = matches!(
            (&s.selection, request.action),
            (Some(Selection::Deletion(_)), Action::Refresh)
        ) || matches!((&s.selection,request.action),(Some(Selection::Deletion(p)),Action::Keep) if p.preview().keep_allowed)
            || matches!((&s.selection,request.action),(Some(Selection::Deletion(p)),Action::ReviewRemaining) if p.preview().reconfirmation_allowed);
        #[cfg(not(unix))]
        let deletion_resolution = false;
        #[cfg(unix)]
        let deletion_resume = matches!(&s.selection, Some(Selection::Deletion(p)) if p.preview().pending)
            && !deletion_completed;
        #[cfg(not(unix))]
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
                | (
                    Selection::Recovery(_),
                    Action::Refresh | Action::Recover | Action::Keep
                )
        );
        #[cfg(target_os = "macos")]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::Relocation(_), Action::Refresh) => true,
                (Selection::Relocation(p), Action::Publish) => p.preview().resume_allowed,
                (Selection::Relocation(p), Action::Keep) => p.preview().pending,
                _ => false,
            };
        #[cfg(unix)]
        let allowed = allowed
            || match (selection, request.action) {
                (Selection::Deletion(_), Action::Refresh) => true,
                (Selection::Deletion(p), Action::ReviewRemaining) => {
                    p.preview().reconfirmation_allowed
                }
                (Selection::Deletion(p), Action::Keep) => p.preview().keep_allowed,
                (Selection::Deletion(p), Action::Publish) => p.preview().execute_allowed,
                (Selection::Deletion(p), Action::DeleteIncludingUnregistered) => {
                    let v = p.preview();
                    v.execute_allowed
                        && matches!(
                            v.phase.as_str(),
                            "review" | "not_started" | "reconfirmation"
                        )
                        && v.unregistered_count > 0
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
        if s.snapshot.kind == Some(Kind::Deletion)
            && matches!(
                request.action,
                Action::Publish | Action::DeleteIncludingUnregistered
            )
            && s.snapshot.phase != "ready"
        {
            return Err("削除確認が失効しています。残りの状態を再検査してください".into());
        }
        if s.snapshot.backup_receipt.is_some()
            || s.snapshot.restore_receipt.is_some()
            || s.snapshot.recovery_receipt.is_some()
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
        s.snapshot.recovery_receipt = completion.recovery;
        s.snapshot.relocation_receipt = completion.relocation;
        #[cfg(unix)]
        {
            s.snapshot.deletion_receipt = completion.deletion;
        }
        s.snapshot.restart_required |= completion.restart_required;
        s.snapshot.attention_required = completion.attention;
        s.snapshot.error_message = completion.error;
        #[cfg(unix)]
        let deletion_completed = s.snapshot.deletion_receipt.is_some();
        #[cfg(not(unix))]
        let deletion_completed = false;
        let published = s.snapshot.backup_receipt.is_some()
            || s.snapshot.restore_receipt.is_some()
            || s.snapshot.recovery_receipt.is_some()
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
        s.snapshot.recovery_preview = match &s.selection {
            Some(Selection::Recovery(p)) => Some(p.preview()),
            _ => None,
        };
        s.snapshot.relocation_preview = match &s.selection {
            #[cfg(target_os = "macos")]
            Some(Selection::Relocation(p)) => Some(p.preview()),
            _ => None,
        };
        #[cfg(unix)]
        {
            s.snapshot.deletion_preview = match &s.selection {
                Some(Selection::Deletion(p)) => Some(p.preview()),
                _ => None,
            };
        }
        s.snapshot.retry_allowed = s.selection.is_some() && !published && !completion.attention;
        s.snapshot.busy = false;
        s.snapshot.revision += 1;
        self.idle.notify_all();
        Some(s.snapshot.clone())
    }

    fn emit(app: &AppHandle, snapshot: Snapshot) {
        let _ = app.emit("data-operation-changed", snapshot);
    }

    pub(crate) async fn select(window: &WebviewWindow, kind: Kind) -> Result<Snapshot, String> {
        if matches!(kind, Kind::Recovery | Kind::Deletion) {
            return Err("会議または要確認一覧から対象を選択してください".into());
        }
        let app = window.app_handle().clone();
        let lease = work::Admission::acquire(&app)?;
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

    pub(crate) fn review_recovery(
        app: &AppHandle,
        source: recovery::Source,
        id: String,
    ) -> Result<Snapshot, String> {
        let lease = work::Admission::acquire(app)?;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(Kind::Recovery)?;
        let token = snapshot.token.clone().ok_or("確認操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        Self::launch(
            app,
            token,
            cancel,
            lease,
            Work::Recovery(source, id, previous),
        );
        Ok(snapshot)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn review_move(app: &AppHandle) -> Result<Snapshot, String> {
        let lease = work::Admission::acquire(app)?;
        let (snapshot, cancel, previous) = app.state::<Self>().begin(Kind::Relocation)?;
        let token = snapshot.token.clone().ok_or("確認操作IDがありません")?;
        Self::emit(app, snapshot.clone());
        Self::launch(app, token, cancel, lease, Work::PendingMove(previous));
        Ok(snapshot)
    }

    pub(crate) fn start(app: &AppHandle, request: Request) -> Result<Snapshot, String> {
        #[cfg(unix)]
        let mut deletion_admission = None;
        {
            let controller = app.state::<Self>();
            let s = controller
                .state
                .lock()
                .map_err(|_| "データ操作ロックを取得できません")?;
            if Self::request_replay(&s, &request)? {
                return Ok(s.snapshot.clone());
            }
            #[cfg(unix)]
            if let Some(Selection::Deletion(p)) = &s.selection {
                let v = p.preview();
                deletion_admission = Some(
                    if v.pending || app.state::<Storage>().data_write_blocked() {
                        Some(v.operation_id)
                    } else {
                        None
                    },
                );
            }
        }
        #[cfg(unix)]
        let lease = if let Some(operation) = deletion_admission {
            work::Admission::acquire_deletion(app, operation.as_deref())?
        } else {
            work::Admission::acquire(app)?
        };
        #[cfg(not(unix))]
        let lease = work::Admission::acquire(app)?;
        if let Some((work, cancel, snapshot)) = app.state::<Self>().begin_request(request)? {
            let token = snapshot.token.clone().ok_or("データ操作IDがありません")?;
            Self::emit(app, snapshot.clone());
            Self::launch(app, token, cancel, lease, work);
            Ok(snapshot)
        } else {
            app.state::<Self>().snapshot()
        }
    }

    #[cfg(unix)]
    pub(crate) fn review_deletion(
        app: &AppHandle,
        id: String,
        pending: bool,
    ) -> Result<Snapshot, String> {
        let lease = work::Admission::acquire_deletion(app, pending.then_some(id.as_str()))?;
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
    #[cfg(unix)]
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

    fn launch(
        app: &AppHandle,
        token: String,
        cancel: Arc<AtomicBool>,
        lease: work::Admission,
        work: Work,
    ) {
        let worker = app.clone();
        let worker_token = token.clone();
        let spawned = std::thread::Builder::new()
            .name("mon-data-operation".into())
            .spawn(move || {
                let cancelled = || cancel.load(Ordering::Acquire) || lease.cancelled();
                let progress = |phase: &str| {
                    if let Some(snapshot) = worker.state::<Self>().progress(&worker_token, phase) {
                        Self::emit(&worker, snapshot);
                    }
                };
                #[cfg(target_os = "macos")]
                let completion = work.run_with_bootstrap_admitted(
                    &worker.state::<Storage>(),
                    &worker.state::<crate::bootstrap::Bootstrap>(),
                    &cancelled,
                    &progress,
                    lease.proof(),
                );
                #[cfg(not(target_os = "macos"))]
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
                && s.snapshot.recovery_receipt.is_none()
                && s.snapshot.relocation_receipt.is_none()
                && !s.snapshot.attention_required
                && {
                    #[cfg(unix)]
                    {
                        s.snapshot.deletion_receipt.is_none()
                            && !s
                                .snapshot
                                .deletion_preview
                                .as_ref()
                                .is_some_and(|p| p.pending)
                    }
                    #[cfg(not(unix))]
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
        let started = Instant::now();
        let mut s = self
            .state
            .lock()
            .map_err(|_| "データ操作ロックを取得できません")?;
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

    pub(crate) fn abort_shutdown(app: &AppHandle) {
        let controller = app.state::<Self>();
        let mut s = controller.state.lock().unwrap_or_else(|e| e.into_inner());
        s.closing = false;
        if s.selection.is_none() {
            s.snapshot.backup_preview = None;
            s.snapshot.restore_preview = None;
            s.snapshot.recovery_preview = None;
            #[cfg(unix)]
            if s.snapshot.deletion_receipt.is_none() {
                s.snapshot.deletion_preview = None;
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
