//! Device-local presentation history. It never authorizes execution, retries,
//! publication or deletion. Producers enqueue small lifecycle records; one
//! worker owns the separate database and retention, outside source DB locks.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::Duration,
};
use tauri::{Emitter, Manager, WebviewWindow};

pub(crate) const RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
pub(crate) const KINDS: &[&str] = &[
    "recording_save",
    "recording_recovery",
    "model",
    "audio_import",
    "transcription",
    "diarization",
    "minutes_generation",
    "voice_registration",
    "voice_matching",
    "speaker_completion",
    "backup",
    "restore",
    "deletion",
    "audio_deletion",
    "model_deletion",
];
const OUTCOMES: &[&str] = &[
    "succeeded",
    "failed",
    "cancelled",
    "interrupted",
    "attention",
];
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub meeting_id: Option<String>,
    pub state: String,
    pub accepted_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub observed_at: i64,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_meeting_ids: Vec<String>,
}
impl Entry {
    pub(crate) fn new(
        kind: &str,
        operation: &str,
        target: &str,
        meeting: Option<String>,
        state: &str,
    ) -> Self {
        let time = now();
        Self {
            id: format!("{kind}:{operation}"),
            kind: kind.into(),
            target: target.into(),
            meeting_id: meeting,
            state: state.into(),
            accepted_at: Some(time),
            finished_at: if ["succeeded", "failed", "cancelled"].contains(&state) {
                Some(time)
            } else {
                None
            },
            observed_at: time,
            detail: None,
            related_meeting_ids: Vec::new(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        if !KINDS.contains(&self.kind.as_str())
            || self.id.is_empty()
            || self.id.len() > 256
            || ![
                "queued",
                "running",
                "pausing",
                "paused",
                "succeeded",
                "failed",
                "cancelled",
                "interrupted",
                "attention",
            ]
            .contains(&self.state.as_str())
            || self.target.len() > 4096
            || self.detail.as_ref().is_some_and(|s| s.len() > 16384)
            || self.accepted_at.is_some_and(|t| t < 0)
            || self.finished_at.is_some_and(|t| t < 0)
            || self.observed_at < 0
        {
            return Err("ジョブ履歴の形式が不正です".into());
        }
        Ok(())
    }
    fn active(&self) -> bool {
        !OUTCOMES.contains(&self.state.as_str())
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Query {
    pub kind: Option<String>,
    pub state: Option<String>,
    pub cursor: Option<Cursor>,
    pub limit: Option<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Cursor {
    at: i64,
    id: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub items: Vec<Entry>,
    pub next_cursor: Option<Cursor>,
    pub storage_error: Option<String>,
}
enum Message {
    Record(Entry),
    Backfill(Vec<Entry>),
    RemoveMeeting(String),
    Flush(mpsc::SyncSender<Result<(), String>>),
    Query(
        Query,
        BTreeMap<String, String>,
        mpsc::SyncSender<Result<Page, String>>,
    ),
    Clear(mpsc::SyncSender<Result<(), String>>),
}
#[derive(Clone)]
struct Service {
    tx: mpsc::SyncSender<Message>,
    error: Arc<Mutex<Option<String>>>,
    last: Arc<Mutex<BTreeMap<String, Entry>>>,
}
static GLOBAL: OnceLock<Service> = OnceLock::new();

fn open(root: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let meta = std::fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("ジョブ履歴の保存先を確認できません".into());
    }
    #[cfg(windows)]
    crate::windows_files::set_private(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    // macOS temporary/app-data paths may contain an OS-owned alias such as
    // /var -> /private/var. Keep NOFOLLOW for the database itself while giving
    // SQLite the physical directory path it requires on reopen.
    let physical_root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let db = Connection::open_with_flags(
        physical_root.join("history.sqlite3"),
        rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version != 0 && version != 1 {
        return Err("ジョブ履歴の保存形式に対応していません。元の記録を保持しています。".into());
    }
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;
        CREATE TABLE IF NOT EXISTS history(id TEXT PRIMARY KEY,kind TEXT NOT NULL,target TEXT NOT NULL,meeting_id TEXT,state TEXT NOT NULL,accepted_at INTEGER,finished_at INTEGER,observed_at INTEGER NOT NULL,detail TEXT,active INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS history_order ON history(active,coalesce(finished_at,observed_at) DESC,id DESC);
        CREATE INDEX IF NOT EXISTS history_meeting ON history(meeting_id);
        CREATE TABLE IF NOT EXISTS removed_meetings(id TEXT PRIMARY KEY,removed_at INTEGER NOT NULL);
        PRAGMA user_version=1;
        UPDATE history SET state='interrupted',finished_at=NULL,active=0,detail='前回の処理結果が未確定です。現在の状態を確認してください。' WHERE active=1;") .map_err(|e|e.to_string())?;
    Ok(db)
}
fn upsert(db: &Connection, mut entry: Entry, time: i64) -> Result<bool, String> {
    entry.validate()?;
    if let Some(meeting) = &entry.meeting_id {
        if db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM removed_meetings WHERE id=?1)",
                [meeting],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
    }
    let old:Option<Entry>=db.query_row("SELECT id,kind,target,meeting_id,state,accepted_at,finished_at,observed_at,detail FROM history WHERE id=?1",[&entry.id],read).optional().map_err(|e|e.to_string())?;
    if let Some(old) = old {
        if entry.observed_at < old.observed_at {
            return Ok(false);
        }
        if entry.target.is_empty() {
            entry.target = old.target.clone();
        }
        entry.accepted_at = old.accepted_at.or(entry.accepted_at);
        if entry.state == old.state {
            entry.finished_at = old.finished_at.or(entry.finished_at);
            entry.observed_at = old.observed_at;
            if entry == old {
                return Ok(false);
            }
            entry.observed_at = time.max(old.observed_at);
        }
    }
    if entry.active() {
        entry.finished_at = None;
    }
    if !entry.active() && entry.finished_at.unwrap_or(entry.observed_at) < time - RETENTION_MS {
        return Ok(false);
    }
    db.execute("INSERT INTO history VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET target=excluded.target,meeting_id=excluded.meeting_id,state=excluded.state,accepted_at=excluded.accepted_at,finished_at=excluded.finished_at,observed_at=excluded.observed_at,detail=excluded.detail,active=excluded.active",params![entry.id,entry.kind,entry.target,entry.meeting_id,entry.state,entry.accepted_at,entry.finished_at,entry.observed_at,entry.detail,entry.active()]).map_err(|e|e.to_string())?;
    Ok(true)
}
fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        kind: r.get(1)?,
        target: r.get(2)?,
        meeting_id: r.get(3)?,
        state: r.get(4)?,
        accepted_at: r.get(5)?,
        finished_at: r.get(6)?,
        observed_at: r.get(7)?,
        detail: r.get(8)?,
        related_meeting_ids: Vec::new(),
    })
}
fn prune(db: &Connection, time: i64) -> Result<(), String> {
    db.execute(
        "DELETE FROM history WHERE active=0 AND coalesce(finished_at,observed_at)<?1",
        [time - RETENTION_MS],
    )
    .map_err(|e| e.to_string())?;
    db.execute(
        "DELETE FROM removed_meetings WHERE removed_at<?1",
        [time - RETENTION_MS],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
fn query(
    db: &Connection,
    query: Query,
    meetings: &BTreeMap<String, String>,
    time: i64,
) -> Result<Page, String> {
    if query.kind.as_deref().is_some_and(|k| !KINDS.contains(&k))
        || query
            .state
            .as_deref()
            .is_some_and(|s| !OUTCOMES.contains(&s))
        || query.limit.is_some_and(|n| n == 0 || n > 100)
        || query
            .cursor
            .as_ref()
            .is_some_and(|c| c.at < 0 || c.id.len() > 256)
    {
        return Err("履歴の絞り込み条件が不正です".into());
    }
    // Reconcile source existence before returning *any* stored names/details.
    // This also removes late producer records after a meeting deletion/crash.
    let json =
        serde_json::to_string(&meetings.keys().collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    db.execute("INSERT OR REPLACE INTO removed_meetings SELECT DISTINCT meeting_id,?2 FROM history WHERE meeting_id IS NOT NULL AND meeting_id NOT IN (SELECT value FROM json_each(?1))",params![json,time]).map_err(|e|e.to_string())?;
    let removed=db.execute("DELETE FROM history WHERE meeting_id IS NOT NULL AND meeting_id NOT IN (SELECT value FROM json_each(?1))",[json]).map_err(|e|e.to_string())?;
    if removed > 0 {
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(|e| e.to_string())?;
    }
    prune(db, time)?;
    let limit = query.limit.unwrap_or(50);
    let cursor_at = query.cursor.as_ref().map(|c| c.at);
    let cursor_id = query.cursor.as_ref().map(|c| c.id.as_str());
    let mut q=db.prepare("SELECT id,kind,target,meeting_id,state,accepted_at,finished_at,observed_at,detail FROM history WHERE active=0 AND coalesce(finished_at,observed_at)>=?1 AND (?2 IS NULL OR kind=?2) AND (?3 IS NULL OR state=?3) AND (?4 IS NULL OR (coalesce(finished_at,observed_at),id)<(?4,?5)) ORDER BY coalesce(finished_at,observed_at) DESC,id DESC LIMIT ?6").map_err(|e|e.to_string())?;
    let mut items = q
        .query_map(
            params![
                time - RETENTION_MS,
                query.kind,
                query.state,
                cursor_at,
                cursor_id,
                (limit + 1) as i64
            ],
            read,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items.last().map(|e| Cursor {
            at: e.finished_at.unwrap_or(e.observed_at),
            id: e.id.clone(),
        })
    } else {
        None
    };
    for entry in &mut items {
        if let Some(title) = entry.meeting_id.as_ref().and_then(|id| meetings.get(id)) {
            entry.target = title.clone();
        }
    }
    Ok(Page {
        items,
        next_cursor,
        storage_error: None,
    })
}
impl Service {
    fn start(root: PathBuf, app: Option<tauri::AppHandle>) -> Self {
        let (tx, rx) = mpsc::sync_channel(4096);
        let error = Arc::new(Mutex::new(None));
        let worker_error = error.clone();
        std::thread::spawn(move || {
            let mut database = open(&root);
            let mut last_prune = 0;
            loop {
                let message = match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(m) => Some(m),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(_) => break,
                };
                if database.is_err() {
                    database = open(&root);
                }
                let time = now();
                if time - last_prune >= 24 * 60 * 60 * 1000 {
                    if let Ok(db) = &database {
                        if let Err(e) = prune(db, time) {
                            *worker_error.lock().unwrap() = Some(e);
                        }
                    }
                    last_prune = time;
                }
                match message {
                    Some(Message::Backfill(entries)) => {
                        let result = database.as_ref().map_err(Clone::clone).and_then(|db| {
                            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
                            for entry in entries {
                                upsert(&tx, entry, time)?;
                            }
                            tx.commit().map_err(|e| e.to_string())
                        });
                        if let Err(error) = result {
                            *worker_error.lock().unwrap() = Some(error);
                        }
                    }
                    Some(Message::Flush(reply)) => {
                        let result = database.as_ref().map(|_| ()).map_err(Clone::clone);
                        let _ = reply.send(result);
                    }
                    Some(Message::RemoveMeeting(id)) => {
                        let result = database.as_ref().map_err(Clone::clone).and_then(|db| {
                            db.execute(
                                "INSERT OR REPLACE INTO removed_meetings VALUES(?1,?2)",
                                params![id, time],
                            )
                            .map_err(|e| e.to_string())?;
                            db.execute("DELETE FROM history WHERE meeting_id=?1", [id])
                                .map_err(|e| e.to_string())?;
                            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                                .map_err(|e| e.to_string())
                        });
                        if let Err(e) = result {
                            *worker_error.lock().unwrap() = Some(e);
                        }
                    }
                    Some(Message::Record(entry)) => {
                        let result = database
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|db| upsert(db, entry, time));
                        match result {
                            Ok(changed) => {
                                if changed {
                                    if let Some(app) = &app {
                                        let _ = app.emit_to("main", "job-history-changed", ());
                                    }
                                }
                            }
                            Err(e) => *worker_error.lock().unwrap() = Some(e),
                        }
                    }
                    Some(Message::Query(q, meetings, reply)) => {
                        let result = database
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|db| query(db, q, &meetings, time))
                            .map(|mut page| {
                                page.storage_error = worker_error.lock().unwrap().clone();
                                page
                            });
                        let _ = reply.send(result);
                    }
                    Some(Message::Clear(reply)) => {
                        let result=database.as_ref().map_err(Clone::clone).and_then(|db|db.execute_batch("DELETE FROM history; DELETE FROM removed_meetings; PRAGMA wal_checkpoint(TRUNCATE);").map_err(|e|e.to_string()));
                        if result.is_ok() {
                            *worker_error.lock().unwrap() = None;
                        }
                        let _ = reply.send(result);
                    }
                    None => {}
                }
            }
        });
        Self {
            tx,
            error,
            last: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    fn record(&self, entry: Entry) {
        // Progress measurements do not create disk work or fill the queue.
        let mut last = self.last.lock().unwrap();
        if last.get(&entry.id).is_some_and(|old| {
            old.kind == entry.kind
                && old.target == entry.target
                && old.meeting_id == entry.meeting_id
                && old.state == entry.state
                && old.detail == entry.detail
        }) {
            return;
        }
        if self.tx.try_send(Message::Record(entry.clone())).is_err() {
            *self.error.lock().unwrap() = Some(
                "ジョブ履歴の記録が混み合っています。一部の履歴を保存できませんでした。".into(),
            );
        } else {
            if last.len() >= 10000 {
                let key = last
                    .iter()
                    .find(|(_, v)| !v.active())
                    .map(|(k, _)| k.clone())
                    .or_else(|| last.keys().next().cloned());
                if let Some(key) = key {
                    last.remove(&key);
                }
            }
            last.insert(entry.id.clone(), entry);
        }
    }
    fn query(&self, q: Query, meetings: BTreeMap<String, String>) -> Result<Page, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .try_send(Message::Query(q, meetings, tx))
            .map_err(|_| "履歴を取得できません。もう一度お試しください")?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "履歴の取得がタイムアウトしました")?
    }
}
pub(crate) fn initialize(app: &tauri::AppHandle, root: PathBuf) {
    GLOBAL.get_or_init(|| Service::start(root, Some(app.clone())));
}
pub(crate) fn record(entry: Entry) {
    if let Some(service) = GLOBAL.get() {
        service.record(entry);
    }
}
pub(crate) fn remove_meeting(id: &str) {
    if let Some(service) = GLOBAL.get() {
        if service
            .tx
            .try_send(Message::RemoveMeeting(id.into()))
            .is_err()
        {
            *service.error.lock().unwrap() =
                Some("関連するジョブ履歴を削除できませんでした。履歴を再読込してください。".into());
        }
    }
}
pub(crate) fn clear() -> Result<(), String> {
    let Some(service) = GLOBAL.get() else {
        return Ok(());
    };
    let (tx, rx) = mpsc::sync_channel(1);
    service
        .tx
        .try_send(Message::Clear(tx))
        .map_err(|_| "ジョブ履歴を初期化できません")?;
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "ジョブ履歴の初期化がタイムアウトしました")??;
    service.last.lock().unwrap().clear();
    Ok(())
}
pub(crate) fn flush() -> Result<(), String> {
    let Some(service) = GLOBAL.get() else {
        return Ok(());
    };
    let (tx, rx) = mpsc::sync_channel(1);
    service
        .tx
        .try_send(Message::Flush(tx))
        .map_err(|_| "ジョブ履歴の保存完了を確認できません")?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "ジョブ履歴の保存待ちがタイムアウトしました")?
}
pub(crate) fn analysis(job: &crate::storage::JobSnapshot) {
    let mut entry = Entry::new(
        job.kind.as_str(),
        &job.id,
        "",
        Some(job.meeting_id.clone()),
        job.state.as_str(),
    );
    entry.accepted_at = Some(job.created_at);
    entry.observed_at = job.updated_at;
    if ["succeeded", "failed", "cancelled"].contains(&job.state.as_str()) {
        entry.finished_at = Some(job.updated_at);
    }
    entry.detail = job.error_code.clone();
    record(entry);
}
pub(crate) fn backfill(storage: &crate::storage::Storage) -> Result<(), String> {
    if let Some(service) = GLOBAL.get() {
        let entries = storage.history_source_records(now() - RETENTION_MS)?;
        service
            .tx
            .try_send(Message::Backfill(entries))
            .map_err(|_| "過去のジョブ履歴を取り込めません。再度取得してください")?;
    }
    Ok(())
}

fn model_failure<T: Serialize>(result: &Result<T, String>) -> Option<(&'static str, String)> {
    match result {
        Err(error) => {
            let outcome = if error.contains("取り消") || error.contains("取消") {
                "cancelled"
            } else if error.contains("中断") {
                "interrupted"
            } else {
                "failed"
            };
            Some((outcome, error.clone()))
        }
        Ok(value) => {
            let value = serde_json::to_value(value).ok()?;
            let outcome = match value["state"].as_str()? {
                "failed" => "failed",
                "cancelled" => "cancelled",
                _ => return None,
            };
            Some((
                outcome,
                value["errorMessage"]
                    .as_str()
                    .unwrap_or("モデルの動作確認を完了できませんでした")
                    .to_owned(),
            ))
        }
    }
}

pub(crate) fn model_stage<T: Serialize>(
    group: Option<&str>,
    target: &str,
    complete: bool,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let Some(group) = group else { return work() };
    uuid::Uuid::parse_str(group).map_err(|_| "モデル準備の履歴IDが不正です")?;
    record(Entry::new("model", group, target, None, "running"));
    let result = work();
    if let Some((outcome, error)) = model_failure(&result) {
        let mut entry = Entry::new("model", group, target, None, outcome);
        entry.detail = Some(error);
        record(entry);
    } else if complete {
        record(Entry::new("model", group, target, None, "succeeded"));
    }
    result
}

#[tauri::command]
pub(crate) async fn list_job_history(
    window: WebviewWindow,
    query: Query,
) -> Result<Page, mon_contracts::AppError> {
    let result = async move {
        let app = crate::minutes_app(&window)?;
        tauri::async_runtime::spawn_blocking(move || {
            let storage = app.state::<crate::storage::Storage>();
            backfill(&storage)?;
            let mut page = GLOBAL
                .get()
                .ok_or("ジョブ履歴の保存先が未準備です")?
                .query(query, storage.history_meetings()?)?;
            for entry in &mut page.items {
                if let Some(batch) = entry.id.strip_prefix("restore:") {
                    entry.related_meeting_ids = storage.history_restored_meetings(batch)?;
                }
            }
            Ok(page)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("list_job_history", message))
}

#[cfg(test)]
mod tests;
