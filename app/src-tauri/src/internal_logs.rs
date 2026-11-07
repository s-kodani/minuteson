//! Best-effort local diagnostics, independent of meeting storage and backups.
//! Producers never wait on disk or the viewer. Only this service owns its files.
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};
use tauri::{Manager, WebviewWindow};

mod engine;
mod store;
#[cfg(test)]
mod tests;
pub(crate) use engine::watch_engine;
const MAX_MESSAGE: usize = 16 * 1024;
const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const SEGMENT_BYTES: u64 = 1024 * 1024;
static GLOBAL: OnceLock<LogService> = OnceLock::new();
thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }

#[derive(Clone, Debug, Default)]
pub(crate) struct Context {
    pub category: String,
    pub meeting_id: Option<String>,
    pub job_id: Option<String>,
    pub operation_id: Option<String>,
}
pub(crate) struct Scope(Option<Context>);
impl Scope {
    pub(crate) fn enter(context: Context) -> Self {
        Self(CONTEXT.with(|current| current.replace(Some(context))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.with(|current| {
            current.replace(self.0.take());
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub id: u64,
    pub timestamp: i64,
    pub session_id: String,
    pub kind: String,
    pub level: String,
    pub category: String,
    pub meeting_id: Option<String>,
    pub job_id: Option<String>,
    pub operation_id: Option<String>,
    pub state: String,
    pub elapsed_ms: Option<u64>,
    pub message: String,
    pub stream: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}
impl Entry {
    fn new(context: &Context, kind: &str, level: &str, state: &str, message: &str) -> Self {
        Self {
            id: 0,
            timestamp: chrono::Utc::now().timestamp_millis(),
            session_id: String::new(),
            kind: kind.into(),
            level: level.into(),
            category: context.category.clone(),
            meeting_id: context.meeting_id.clone(),
            job_id: context.job_id.clone(),
            operation_id: context.operation_id.clone(),
            state: state.into(),
            elapsed_ms: None,
            message: message.into(),
            stream: None,
            source: None,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Filter {
    pub kind: Option<String>,
    pub level: Option<String>,
    pub category: Option<String>,
    pub related_id: Option<String>,
    pub text: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}
impl Filter {
    fn matches(&self, entry: &Entry) -> bool {
        self.kind.as_ref().is_none_or(|v| &entry.kind == v)
            && (entry.kind == "engine" || self.level.as_ref().is_none_or(|v| &entry.level == v))
            && self.category.as_ref().is_none_or(|v| &entry.category == v)
            && self.from.is_none_or(|v| entry.timestamp >= v)
            && self.to.is_none_or(|v| entry.timestamp <= v)
            && self.related_id.as_ref().is_none_or(|v| {
                [&entry.meeting_id, &entry.job_id, &entry.operation_id]
                    .iter()
                    .any(|id| id.as_ref().is_some_and(|id| id.contains(v)))
            })
            && self
                .text
                .as_ref()
                .is_none_or(|v| entry.message.to_lowercase().contains(&v.to_lowercase()))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Query {
    pub filter: Filter,
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}
impl Query {
    fn validate(&self) -> Result<(), String> {
        if self.before.is_some() && self.after.is_some()
            || self.limit.is_some_and(|n| n == 0 || n > 200)
            || [
                &self.filter.kind,
                &self.filter.level,
                &self.filter.category,
                &self.filter.related_id,
                &self.filter.text,
            ]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| s.len() > 512))
            || self
                .filter
                .from
                .zip(self.filter.to)
                .is_some_and(|(a, b)| a > b)
        {
            return Err("ログの取得条件が不正です".into());
        }
        Ok(())
    }
}
#[derive(Clone, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub storage_error: Option<String>,
    pub dropped_entries: u64,
    pub history_truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub entries: Vec<Entry>,
    pub older_cursor: u64,
    pub newer_cursor: u64,
    pub has_more: bool,
    pub expired: bool,
    pub status: Status,
}
enum Command {
    Record(Entry),
    Query(Query, mpsc::SyncSender<Result<Page, String>>),
    Flush(mpsc::SyncSender<()>),
}
#[derive(Clone)]
pub(crate) struct LogService {
    tx: mpsc::SyncSender<Command>,
    dropped: Arc<AtomicU64>,
    status: Arc<Mutex<Status>>,
    // Last phase per operation; bounded so completed jobs don't grow memory forever.
    transitions: Arc<Mutex<std::collections::HashMap<String, (String, Instant)>>>,
}
impl LogService {
    pub(crate) fn start(root: Result<PathBuf, String>) -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        let service = Self {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(Status::default())),
            transitions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let shared = service.clone();
        // Worker doesn't hold the sender; dropping all producers closes it.
        let dropped = shared.dropped.clone();
        let status = shared.status.clone();
        let spawned = std::thread::Builder::new()
            .name("mon-internal-logs".into())
            .spawn(move || store::run(root, rx, dropped, status));
        if let Err(error) = spawned {
            service.error(format!("ログ保存スレッドを開始できません: {error}"));
        }
        service
    }
    fn error(&self, message: String) {
        if let Ok(mut status) = self.status.lock() {
            status.storage_error = Some(message);
        }
    }
    fn detail(
        &self,
        context: &Context,
        state: &str,
        level: &str,
        elapsed: Duration,
        message: &str,
    ) {
        let mut entry = Entry::new(context, "process", level, state, message);
        entry.elapsed_ms = Some(elapsed.as_millis().min(u64::MAX as u128) as u64);
        self.record(entry);
    }
    fn record(&self, entry: Entry) {
        // Keep every raw byte of valid UTF-8 even for abnormally long lines.
        let mut remaining = entry.message.as_str();
        loop {
            let mut end = remaining.len().min(MAX_MESSAGE);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            let mut part = entry.clone();
            part.message = remaining[..end].into();
            if self.tx.try_send(Command::Record(part)).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            remaining = &remaining[end..];
            if remaining.is_empty() {
                break;
            }
        }
    }
    fn transition(&self, context: &Context, state: &str, level: &str, message: &str) {
        let key = format!(
            "{}:{}:{}:{}",
            context.category,
            context.meeting_id.as_deref().unwrap_or(""),
            context.job_id.as_deref().unwrap_or(""),
            context.operation_id.as_deref().unwrap_or("")
        );
        let Ok(mut phases) = self.transitions.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let signature = format!("{state}:{level}:{message}");
        if phases.get(&key).is_some_and(|(last, _)| last == &signature) {
            return;
        }
        let started = phases
            .get(&key)
            .map(|(_, start)| *start)
            .unwrap_or_else(Instant::now);
        if phases.len() >= 2048 && !phases.contains_key(&key) {
            phases.clear();
        }
        phases.insert(key, (signature, started));
        drop(phases);
        let mut entry = Entry::new(context, "process", level, state, message);
        entry.elapsed_ms = Some(started.elapsed().as_millis() as u64);
        self.record(entry);
    }
    pub(crate) fn query(&self, query: Query) -> Result<Page, String> {
        query.validate()?;
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .try_send(Command::Query(query, tx))
            .map_err(|_| "ログ処理が混み合っています。再度取得してください".to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "ログを取得できませんでした".to_string())?
    }
    pub(crate) fn flush(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.tx.try_send(Command::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }
}
pub(crate) fn initialize(root: Result<PathBuf, String>) -> LogService {
    GLOBAL.get_or_init(|| LogService::start(root)).clone()
}
pub(crate) fn transition(context: Context, state: &str, level: &str, message: &str) {
    if let Some(service) = GLOBAL.get() {
        service.transition(&context, state, level, message);
    }
}
/// Per-unit diagnostics bypass phase deduplication; never block inference on I/O.
pub(crate) fn current_context(category: &str) -> Context {
    CONTEXT
        .with(|c| c.borrow().clone())
        .unwrap_or_else(|| Context {
            category: category.into(),
            ..Context::default()
        })
}
pub(crate) fn detail(context: Context, state: &str, level: &str, elapsed: Duration, message: &str) {
    if let Some(service) = GLOBAL.get() {
        service.detail(&context, state, level, elapsed, message);
    }
}
pub(crate) fn failure(context: Context, message: &str) {
    transition(context, "failed", "error", message);
}
fn require_main(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("内部ログはメイン画面だけで確認できます".into())
    }
}
#[tauri::command]
pub(crate) fn record_meeting_open_failure(
    window: WebviewWindow,
    meeting_id: String,
    step: String,
    message: String,
) -> Result<(), mon_contracts::AppError> {
    let result = (|| {
        require_main(window.label())?;
        if uuid::Uuid::parse_str(&meeting_id).is_err()
            || !matches!(
                step.as_str(),
                "subscribe_analysis_events"
                    | "get_analysis_flow"
                    | "flush_pending_transcript_edits"
                    | "get_meeting"
                    | "list_meetings"
            )
            || message.is_empty()
            || message.len() > 4096
        {
            return Err("会議を開く際の診断ログの対象または内容が不正です".into());
        }
        detail(
            Context {
                category: "meeting_open_diagnostics".into(),
                meeting_id: Some(meeting_id),
                operation_id: Some(step.clone()),
                ..Context::default()
            },
            "failed",
            "error",
            Duration::ZERO,
            &format!("{step}: {message}"),
        );
        Ok(())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("record_meeting_open_failure", message)
    })
}

#[tauri::command]
pub(crate) async fn get_internal_logs(
    window: WebviewWindow,
    query: Query,
) -> Result<Page, mon_contracts::AppError> {
    let result: Result<Page, String> = async move {
        require_main(window.label())?;
        query.validate()?;
        let service = window.state::<LogService>().inner().clone();
        tauri::async_runtime::spawn_blocking(move || service.query(query))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_internal_logs", message))
}

pub(crate) fn report_error(category: &str, message: &str) {
    let context = CONTEXT
        .with(|current| current.borrow().clone())
        .unwrap_or_else(|| Context {
            category: category.into(),
            ..Context::default()
        });
    failure(context, message);
}
pub(crate) fn observe<T>(
    context: Context,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _scope = Scope::enter(context.clone());
    transition(context.clone(), "starting", "info", "処理を開始しました");
    let result = work();
    match &result {
        Ok(_) => transition(context, "succeeded", "info", "処理を完了しました"),
        Err(error) => {
            let cancelled =
                error.contains("取消") || error.contains("取り消") || error.contains("中断");
            transition(
                context,
                if cancelled { "cancelled" } else { "failed" },
                if cancelled { "warn" } else { "error" },
                error,
            );
        }
    }
    result
}
pub(crate) fn phase(
    category: &str,
    operation_id: Option<String>,
    meeting_id: Option<String>,
    state: &str,
    error: Option<&str>,
) {
    transition(
        Context {
            category: category.into(),
            operation_id,
            meeting_id,
            ..Context::default()
        },
        state,
        if error.is_some() { "error" } else { "info" },
        error.unwrap_or(state),
    );
}
pub(crate) fn runtime(value: &impl Serialize) {
    if GLOBAL.get().is_none() {
        return;
    }
    if let Ok(value) = serde_json::to_value(value) {
        if let Some(phase) = value.get("phase").and_then(|p| p.as_str()) {
            let context = CONTEXT
                .with(|c| c.borrow().clone())
                .unwrap_or_else(|| Context {
                    category: "minutes_generation".into(),
                    ..Context::default()
                });
            let unit = value.get("unitIndex").and_then(|v| v.as_u64()).unwrap_or(0);
            transition(
                context,
                phase,
                "info",
                &format!("工程: {phase} / 単位: {unit}"),
            );
        }
    }
}

pub(crate) fn job(job: &crate::storage::JobSnapshot) {
    crate::job_history::analysis(job);
    transition(
        Context {
            category: job.kind.to_string(),
            meeting_id: Some(job.meeting_id.clone()),
            job_id: Some(job.id.clone()),
            operation_id: Some(job.operation_id.clone()),
        },
        &job.state,
        if job.state == "failed" {
            "error"
        } else {
            "info"
        },
        &format!(
            "{} {}/{}{}",
            job.state,
            job.completed_units,
            job.total_units
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into()),
            job.error_code
                .as_ref()
                .map(|e| format!(" · {e}"))
                .unwrap_or_default()
        ),
    );
}
