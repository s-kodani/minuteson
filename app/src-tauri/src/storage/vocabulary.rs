use super::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

// Internal restore merge; its live publication/controller is not wired yet.
#[allow(dead_code)]
mod restore;
pub(super) use restore::GlobalMerge;
pub(super) use restore::MergeReceipt;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct VocabularyEntry {
    pub id: String,
    pub term: String,
    pub reading: Option<String>,
    pub note: Option<String>,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VocabularyBook {
    pub id: String,
    pub payload: Vec<VocabularyEntry>,
    pub lock_version: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct VocabularyStamp {
    pub global_version: i64,
    pub meeting_version: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VocabularyView {
    pub global: VocabularyBook,
    pub meeting: VocabularyBook,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct FixedEntry {
    pub scope: String,
    pub entry_version: i64,
    pub entry: VocabularyEntry,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct VocabularySnapshot {
    pub schema_version: u32,
    pub id: String,
    pub meeting_id: String,
    pub stamp: VocabularyStamp,
    pub entries: Vec<FixedEntry>,
    pub method: String,
    pub extra_prompt: String,
    pub actual_prompt: String,
    pub content_hash: String,
}

fn hash<T: Serialize>(value: &T) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|e| e.to_string())?)
    ))
}
pub(crate) fn validate(entries: &[VocabularyEntry]) -> Result<(), String> {
    if entries.len() > 500 {
        return Err("用語は各リスト500件以内で登録してください".into());
    }
    let mut ids = HashSet::new();
    let mut terms = HashSet::new();
    for e in entries {
        if Uuid::parse_str(&e.id).is_err()
            || !ids.insert(&e.id)
            || !terms.insert(&e.term)
            || e.term.trim() != e.term
            || e.term.is_empty()
            || e.term.chars().count() > 200
            || e.term.chars().any(char::is_control)
            || e.reading.as_ref().is_some_and(|s| {
                s.trim().is_empty() || s.chars().count() > 200 || s.chars().any(char::is_control)
            })
            || e.note.as_ref().is_some_and(|s| {
                s.trim().is_empty()
                    || s.chars().count() > 1000
                    || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
            })
        {
            return Err(
                "用語の重複・ID・文字数・空欄を確認してください（用語/読み200文字、補足1000文字）"
                    .into(),
            );
        }
    }
    Ok(())
}
pub(super) fn save_initial_import_vocabulary(
    db: &Connection,
    meeting: &str,
    entries: &[VocabularyEntry],
    expected_global: Option<i64>,
) -> Result<(), String> {
    validate(entries)?;
    let global = load(db, "global")?.0;
    if expected_global.is_some_and(|v| v != global.lock_version) {
        return Err("共通用語が確認後に変更されました。再確認して取り込んでください".into());
    }
    if entries.is_empty() {
        return Ok(());
    }
    let id = scope(db, Some(meeting))?;
    let now = now_ms()?;
    let rows: Vec<_> = entries
        .iter()
        .map(|e| (e.clone(), 1_i64, now, now))
        .collect();
    let digest = hash(&(&rows, 1_i64))?;
    db.execute("INSERT INTO vocabulary_scopes(id,meeting_id,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,?4)",params![id,meeting,digest,now]).map_err(|e|e.to_string())?;
    for (position, e) in entries.iter().enumerate() {
        db.execute("INSERT INTO vocabulary_entries(id,scope_id,position,term,reading,note,enabled,lock_version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?8)",params![e.id,id,position as i64,e.term,e.reading,e.note,e.enabled,now]).map_err(|e|e.to_string())?;
    }
    Ok(())
}
fn scope(connection: &Connection, meeting: Option<&str>) -> Result<String, String> {
    if let Some(meeting) = meeting {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1 AND lifecycle != 'deleted')",
                params![meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("用語の会議が見つかりません".into());
        }
        Ok(format!("meeting:{meeting}"))
    } else {
        Ok("global".into())
    }
}
type StoredEntry = (VocabularyEntry, i64, i64, i64);
fn stored_entries(connection: &Connection, id: &str) -> Result<Vec<StoredEntry>, String> {
    let mut query = connection.prepare("SELECT id,term,reading,note,enabled,lock_version,created_at,updated_at FROM vocabulary_entries WHERE scope_id=?1 ORDER BY position").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![id], |r| {
            Ok((
                VocabularyEntry {
                    id: r.get(0)?,
                    term: r.get(1)?,
                    reading: r.get(2)?,
                    note: r.get(3)?,
                    enabled: r.get(4)?,
                },
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<StoredEntry>, _>>()
        .map_err(|e| e.to_string())?;
    let payload: Vec<_> = rows.iter().map(|r| r.0.clone()).collect();
    validate(&payload)?;
    Ok(rows)
}

/// Hash the actual stored rows, including entry versions and timestamps. Used
/// when new restore identities change the hash basis in a private database.
pub(super) fn stored_book_hash(connection: &Connection, id: &str) -> Result<String, String> {
    let version: i64 = connection
        .query_row(
            "SELECT lock_version FROM vocabulary_scopes WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    hash(&(stored_entries(connection, id)?, version))
}

fn load(connection: &Connection, id: &str) -> Result<(VocabularyBook, Vec<StoredEntry>), String> {
    let rows = stored_entries(connection, id)?;
    let payload: Vec<_> = rows.iter().map(|r| r.0.clone()).collect();
    let stored: Option<(String, i64)> = connection
        .query_row(
            "SELECT content_hash,lock_version FROM vocabulary_scopes WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let lock_version = if let Some((saved, version)) = stored {
        if saved != hash(&(&rows, version))? {
            return Err("保存済み用語が変更されています".into());
        }
        version
    } else {
        if !rows.is_empty() {
            return Err("用語リストの所有情報がありません".into());
        }
        0
    };
    Ok((
        VocabularyBook {
            id: id.into(),
            payload,
            lock_version,
        },
        rows,
    ))
}

pub(super) fn verify_saved_books(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("SELECT id FROM vocabulary_scopes ORDER BY id")
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        load(
            connection,
            &row.get::<_, String>(0).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

pub(super) fn compile_method(
    entries: &[FixedEntry],
    extra: &str,
    method: &str,
) -> Result<String, String> {
    let mut prompt = extra.to_owned();
    if !entries.is_empty() {
        if !prompt.is_empty() {
            prompt.push('\n');
        }
        match method {
            "transducer-hotwords-v1" => {
                prompt.push_str(&entries.iter().map(|e| e.entry.term.as_str()).collect::<Vec<_>>().join("\n"));
            }
            "whisper-vocabulary-v2" => {
                for (index, item) in entries.iter().enumerate() {
                    if index > 0 {
                        prompt.push('\n');
                    }
                    prompt.push_str(&item.entry.term);
                    if let Some(reading) = &item.entry.reading {
                        prompt.push('（');
                        prompt.push_str(reading);
                        prompt.push('）');
                    }
                }
            }
            _ => return Err("未対応の用語ヒント方式です".into()),
        }
    }
    if !matches!(method, "whisper-vocabulary-v2" | "transducer-hotwords-v1") {
        return Err("未対応の用語ヒント方式です".into());
    }
    if prompt.chars().count() > 8000 || prompt.contains('\0') {
        return Err("登録用語と今回のヒントを合わせた入力が8000文字を超えるか不正です。用語を省略せず停止しました。不要な用語を無効にしてください".into());
    }
    Ok(prompt)
}
impl VocabularySnapshot {
    pub(super) fn digest(&self) -> Result<String, String> {
        hash(&(
            self.schema_version,
            &self.id,
            &self.meeting_id,
            &self.stamp,
            &self.entries,
            &self.method,
            &self.extra_prompt,
            &self.actual_prompt,
        ))
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let items: Vec<_> = self.entries.iter().map(|e| e.entry.clone()).collect();
        // Two scopes may contain a total of 1,000 registered entries. The
        // bounded prompt check below is stricter than the registration limit.
        if items.len() > 1000
            || self.schema_version != 1
            || Uuid::parse_str(&self.id).is_err()
            || self.stamp.global_version < 0
            || self.stamp.meeting_version < 0
            || self.entries.iter().any(|e| {
                !matches!(e.scope.as_str(), "global" | "meeting")
                    || e.entry_version <= 0
                    || !e.entry.enabled
            })
            || self.actual_prompt
                != compile_method(&self.entries, &self.extra_prompt, &self.method)?
            || self.content_hash != self.digest()?
        {
            return Err("固定用語ヒントの形式・内容ハッシュが不正です".into());
        }
        let mut terms = HashSet::new();
        let mut ids = HashSet::new();
        for e in &items {
            validate(std::slice::from_ref(e))?;
            if !terms.insert(&e.term) || !ids.insert(&e.id) {
                return Err("固定用語が重複しています".into());
            }
        }
        Ok(())
    }
}

impl Storage {
    pub(crate) fn vocabulary_book(&self, meeting: Option<&str>) -> Result<VocabularyBook, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        Ok(load(&db, &scope(&db, meeting)?)?.0)
    }
    pub(crate) fn vocabulary_view(&self, meeting: &str) -> Result<VocabularyView, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        Ok(VocabularyView {
            global: load(&db, "global")?.0,
            meeting: load(&db, &scope(&db, Some(meeting))?)?.0,
        })
    }
    pub(crate) fn save_vocabulary_book(
        &self,
        meeting: Option<&str>,
        expected: i64,
        payload: Vec<VocabularyEntry>,
    ) -> Result<VocabularyBook, String> {
        validate(&payload)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let id = scope(&tx, meeting)?;
        let (current, old) = load(&tx, &id)?;
        if current.lock_version != expected {
            return Err(
                "用語リストが別の操作で変更されました。入力を保持して再読込してください".into(),
            );
        }
        if current.payload == payload {
            return Ok(current);
        }
        let now = now_ms()?;
        let old: HashMap<_, _> = old.into_iter().map(|e| (e.0.id.clone(), e)).collect();
        let version = expected.checked_add(1).ok_or("用語の版番号が範囲外です")?;
        tx.execute("INSERT INTO vocabulary_scopes(id,meeting_id,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![id,meeting,"0".repeat(64),version,now]).map_err(|e|e.to_string())?;
        tx.execute(
            "DELETE FROM vocabulary_entries WHERE scope_id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        let mut rows = vec![];
        for (position, e) in payload.iter().enumerate() {
            let (row_version, created, updated) = match old.get(&e.id) {
                Some(prior) if &prior.0 == e => (prior.1, prior.2, prior.3),
                Some(prior) => (
                    prior.1.checked_add(1).ok_or("用語の版番号が範囲外です")?,
                    prior.2,
                    now,
                ),
                None => (1, now, now),
            };
            tx.execute("INSERT INTO vocabulary_entries(id,scope_id,position,term,reading,note,enabled,lock_version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![e.id,id,position as i64,e.term,e.reading,e.note,e.enabled,row_version,created,updated]).map_err(|e|e.to_string())?;
            rows.push((e.clone(), row_version, created, updated));
        }
        tx.execute(
            "UPDATE vocabulary_scopes SET content_hash=?1 WHERE id=?2",
            params![hash(&(&rows, version))?, id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(VocabularyBook {
            id,
            payload,
            lock_version: version,
        })
    }
    pub(crate) fn enqueue_with_vocabulary(
        &self,
        meeting: &str,
        operation: &str,
        extra: &str,
        expected: Option<VocabularyStamp>,
        build: impl FnOnce(VocabularySnapshot) -> Result<JobSpec, String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_with_vocabulary_method(meeting, operation, extra, expected, "whisper-vocabulary-v2", build)
    }
    pub(crate) fn enqueue_with_vocabulary_method(
        &self, meeting: &str, operation: &str, extra: &str, expected: Option<VocabularyStamp>,
        method: &str, build: impl FnOnce(VocabularySnapshot) -> Result<JobSpec, String>,
    ) -> Result<JobSnapshot, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id=?1",
                params![operation],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let snapshot = if let Some(existing) = &existing {
            let spec = jobs::read_fixed_input(&tx, existing)?.0;
            let snapshot: VocabularySnapshot =
                serde_json::from_value(spec.parameters["vocabulary"].clone())
                    .map_err(|_| "同じ操作IDに異なる処理が登録されています")?;
            verify_snapshot(&tx, meeting, existing, &snapshot)?;
            if snapshot.extra_prompt != extra {
                return Err("同じ操作IDで異なる用語ヒントが指定されました".into());
            }
            snapshot
        } else {
            let (global, g) = load(&tx, "global")?;
            let (local, m) = load(&tx, &scope(&tx, Some(meeting))?)?;
            let stamp = VocabularyStamp {
                global_version: global.lock_version,
                meeting_version: local.lock_version,
            };
            if expected.is_some_and(|e| e != stamp) {
                return Err(
                    "確認した用語リストが変更されました。用語を再確認して開始してください".into(),
                );
            }
            let mut seen = HashSet::new();
            let mut entries = vec![];
            // Disabled meeting terms do not suppress enabled common terms.
            for (scope, rows) in [("meeting", m), ("global", g)] {
                for (entry, version, _, _) in rows {
                    if entry.enabled && seen.insert(entry.term.clone()) {
                        entries.push(FixedEntry {
                            scope: scope.into(),
                            entry_version: version,
                            entry,
                        });
                    }
                }
            }
            let mut result = VocabularySnapshot {
                schema_version: 1,
                id: Uuid::new_v4().to_string(),
                meeting_id: meeting.into(),
                stamp,
                actual_prompt: compile_method(&entries, extra, method)?,
                entries,
                method: method.into(),
                extra_prompt: extra.into(),
                content_hash: String::new(),
            };
            result.content_hash = result.digest()?;
            result.validate()?;
            result
        };
        let spec = build(snapshot.clone())?;
        if spec.meeting_id != meeting
            || spec.operation_id != operation
            || spec.kind != JobKind::Transcription
            || spec.parameters["vocabulary"]
                != serde_json::to_value(&snapshot).map_err(|e| e.to_string())?
        {
            return Err("用語スナップショットと解析条件が一致しません".into());
        }
        let job = jobs::enqueue_in_transaction(&tx, &spec)?;
        if existing.is_none() {
            tx.execute("INSERT INTO vocabulary_snapshots(id,meeting_id,job_id,payload_json,content_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![snapshot.id,meeting,job.id,serde_json::to_string(&snapshot).map_err(|e|e.to_string())?,snapshot.content_hash,now_ms()?]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        crate::internal_logs::job(&job);
        Ok(job)
    }
    pub(crate) fn verify_vocabulary_snapshot(
        &self,
        meeting: &str,
        job: &str,
        snapshot: &VocabularySnapshot,
    ) -> Result<(), String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        verify_snapshot(&db, meeting, job, snapshot)
    }
}
pub(super) fn verify_snapshot(
    db: &Connection,
    meeting: &str,
    job: &str,
    snapshot: &VocabularySnapshot,
) -> Result<(), String> {
    snapshot.validate()?;
    if snapshot.meeting_id != meeting {
        return Err("固定用語の会議が一致しません".into());
    }
    let row:(String,String,String)=db.query_row("SELECT job_id,payload_json,content_hash FROM vocabulary_snapshots WHERE id=?1 AND meeting_id=?2",params![snapshot.id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let saved: VocabularySnapshot = serde_json::from_str(&row.1).map_err(|e| e.to_string())?;
    if &saved != snapshot || row.2 != snapshot.content_hash {
        return Err("固定用語の保存内容が変更されています".into());
    }
    let mut current = job.to_owned();
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        if !seen.insert(current.clone()) {
            break;
        }
        let spec = jobs::read_fixed_input(db, &current)?.0;
        if spec.meeting_id != meeting
            || spec.kind != JobKind::Transcription
            || spec.parameters["vocabulary"]
                != serde_json::to_value(snapshot).map_err(|e| e.to_string())?
        {
            break;
        }
        if current == row.0 {
            return Ok(());
        }
        let parent: Option<String> = db
            .query_row(
                "SELECT retry_of_job_id FROM processing_jobs WHERE id=?1",
                params![current],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let Some(parent) = parent else {
            break;
        };
        current = parent;
    }
    Err("固定用語の元ジョブ・再試行の由来が一致しません".into())
}

#[cfg(test)]
#[path = "vocabulary_tests.rs"]
mod tests;
