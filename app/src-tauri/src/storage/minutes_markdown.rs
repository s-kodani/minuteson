//! One exact Markdown string. No structured edit payload, evidence ledger,
//! generated-original copy, revision chain or inverse operation is created.
use super::*;
use rusqlite::{Transaction, TransactionBehavior};

pub(crate) const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOCK: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MarkdownStamp {
    pub lock_version: i64,
    pub content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkdownDocument {
    pub meeting_id: String,
    pub markdown: String,
    pub content_hash: String,
    pub lock_version: i64,
    pub operation_id: String,
    pub updated_at: i64,
}

impl MarkdownDocument {
    pub(crate) fn stamp(&self) -> MarkdownStamp {
        MarkdownStamp {
            lock_version: self.lock_version,
            content_hash: self.content_hash.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SaveMarkdownRequest {
    pub meeting_id: String,
    pub operation_id: String,
    pub expected: Option<MarkdownStamp>,
    pub markdown: String,
}

/// Fixed at generation admission; it contains only a CAS stamp, never the old
/// Markdown. The worker cannot choose a different target when it completes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct GenerationTarget {
    pub schema_version: u32,
    pub expected: Option<MarkdownStamp>,
}
impl GenerationTarget {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.expected.as_ref().is_some_and(|s| !valid_stamp(s)) {
            return Err("議事録生成の保存条件が不正です".into());
        }
        Ok(())
    }
    pub(crate) fn check(&self, db: &Connection, meeting: &str) -> Result<(), String> {
        self.validate()?;
        if read(db, meeting)?.as_ref().map(MarkdownDocument::stamp) != self.expected {
            return Err("生成の確認後に議事録が更新されています。人間の編集を保持します".into());
        }
        Ok(())
    }
}

pub(super) fn uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == ::uuid::Variant::RFC4122
            && id.to_string() == value
    })
}

pub(crate) fn content_hash(markdown: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"mon-minutes-markdown-v1\0");
    hash.update(markdown.as_bytes());
    format!("{:x}", hash.finalize())
}

pub(crate) fn validate_text(markdown: &str) -> Result<(), String> {
    // An intentionally empty human document is valid; generation may require
    // nonempty output separately. Never trim, repair or reserialize the text.
    if markdown.len() > MAX_BYTES || markdown.contains('\0') {
        return Err("議事録本文は2MiB以内で、NUL文字を含めず入力してください".into());
    }
    Ok(())
}

fn valid_stamp(stamp: &MarkdownStamp) -> bool {
    (1..MAX_LOCK).contains(&stamp.lock_version)
        && stamp.content_hash.len() == 64
        && stamp
            .content_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn read(db: &Connection, meeting: &str) -> Result<Option<MarkdownDocument>, String> {
    let document: Option<MarkdownDocument> = db.query_row(
        "SELECT meeting_id,markdown,content_hash,lock_version,operation_id,updated_at FROM minutes_markdown WHERE meeting_id=?1",
        [meeting], |r| Ok(MarkdownDocument { meeting_id:r.get(0)?, markdown:r.get(1)?, content_hash:r.get(2)?, lock_version:r.get(3)?, operation_id:r.get(4)?, updated_at:r.get(5)? })
    ).optional().map_err(|e|e.to_string())?;
    if let Some(document) = &document {
        validate_text(&document.markdown)?;
        let (base_lock, base_hash): (i64,Option<String>) = db.query_row(
            "SELECT base_lock_version,base_content_hash FROM minutes_markdown WHERE meeting_id=?1",
            [meeting], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        let valid_base = if base_lock == 0 {
            base_hash.is_none()
        } else {
            base_hash.as_ref().is_some_and(|hash| {
                valid_stamp(&MarkdownStamp {
                    lock_version: base_lock,
                    content_hash: hash.clone(),
                })
            })
        };
        if document.meeting_id != meeting
            || document.content_hash != content_hash(&document.markdown)
            || !(1..=MAX_LOCK).contains(&document.lock_version)
            || !uuid(&document.operation_id)
            || document.updated_at < 0
            || !valid_base
            || base_lock.checked_add(1) != Some(document.lock_version)
        {
            return Err("議事録本文の保存状態・内容ハッシュが不正です".into());
        }
    }
    Ok(document)
}

pub(crate) fn verify_all(db: &Connection) -> Result<(), String> {
    let meetings = db
        .prepare("SELECT meeting_id FROM minutes_markdown ORDER BY meeting_id")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for meeting in meetings {
        read(db, &meeting)?.ok_or("議事録本文が見つかりません")?;
    }
    Ok(())
}

pub(crate) fn save(
    tx: &Transaction<'_>,
    request: &SaveMarkdownRequest,
) -> Result<MarkdownDocument, String> {
    validate_text(&request.markdown)?;
    if !uuid(&request.operation_id) || request.expected.as_ref().is_some_and(|s| !valid_stamp(s)) {
        return Err("議事録保存の操作ID・確認済み保存状態が不正です".into());
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
            [&request.meeting_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("会議が見つかりません".into());
    }
    let current = read(tx, &request.meeting_id)?;
    let base_lock = request.expected.as_ref().map_or(0, |s| s.lock_version);
    let base_hash = request.expected.as_ref().map(|s| s.content_hash.as_str());
    if let Some(current) = &current {
        if current.operation_id == request.operation_id {
            let original: (i64,Option<String>) = tx.query_row(
                "SELECT base_lock_version,base_content_hash FROM minutes_markdown WHERE meeting_id=?1",
                [&request.meeting_id], |r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            if current.markdown != request.markdown
                || original.0 != base_lock
                || original.1.as_deref() != base_hash
            {
                return Err("同じ議事録保存操作で別の本文・保存状態が指定されています".into());
            }
            return Ok(current.clone());
        }
    }
    if current.as_ref().map(MarkdownDocument::stamp) != request.expected {
        return Err(
            "議事録が別の操作で更新されています。入力を保持し、保存状態を確認してください".into(),
        );
    }
    let lock = base_lock
        .checked_add(1)
        .filter(|v| *v <= MAX_LOCK)
        .ok_or("議事録保存回数の上限です")?;
    let hash = content_hash(&request.markdown);
    let updated = now_ms()?;
    // Upsert the sole row. Do not write old Markdown to any other table.
    tx.execute("INSERT INTO minutes_markdown(meeting_id,markdown,content_hash,lock_version,operation_id,base_lock_version,base_content_hash,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(meeting_id) DO UPDATE SET markdown=excluded.markdown,content_hash=excluded.content_hash,lock_version=excluded.lock_version,operation_id=excluded.operation_id,base_lock_version=excluded.base_lock_version,base_content_hash=excluded.base_content_hash,updated_at=excluded.updated_at",
        params![request.meeting_id,request.markdown,hash,lock,request.operation_id,base_lock,base_hash,updated]).map_err(|e|e.to_string())?;
    read(tx, &request.meeting_id)?.ok_or_else(|| "保存した議事録本文が見つかりません".into())
}

impl Storage {
    /// Publish the sole Markdown body and job success in one transaction. No
    /// candidate, old-body backup, item/evidence projection or draft is made.
    pub(crate) fn finish_minutes_markdown(
        &self,
        job_id: &str,
        completed_units: i64,
        markdown: String,
    ) -> Result<MarkdownDocument, String> {
        self.require_data_writable()?;
        validate_text(&markdown)?;
        if markdown.trim().is_empty() {
            return Err("議事録の生成結果が空です。現在の本文は保持します".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let job = super::jobs::load_job(&tx, job_id)?.ok_or("議事録ジョブがありません")?;
        let (spec, checkpoint) = super::jobs::read_fixed_input(&tx, job_id)?;
        if spec.kind != JobKind::MinutesGeneration
            || job.state != "running"
            || completed_units < 1
            || job.completed_units != completed_units
            || job.total_units != Some(completed_units)
            || job.output_minutes_id.is_some()
        {
            return Err("議事録生成の状態・完了位置が一致しません".into());
        }
        super::jobs::verify_latest_checkpoint(&tx, job_id)?;
        let target: GenerationTarget =
            serde_json::from_value(spec.parameters["generation"]["markdownTarget"].clone())
                .map_err(|_| "議事録生成の固定保存条件がありません")?;
        target.check(&tx, &spec.meeting_id)?;
        if checkpoint.as_ref().and_then(|c| c["markdownHash"].as_str())
            != Some(content_hash(&markdown).as_str())
        {
            return Err("議事録生成の保存済み結果と本文が一致しません".into());
        }
        // Also verify the fixed transcript snapshot. Its later human edits do
        // not invalidate this input, but tampering with the fixed copy does.
        super::minutes_input::read_input(&tx, &spec)?;
        let document = save(
            &tx,
            &SaveMarkdownRequest {
                meeting_id: spec.meeting_id,
                operation_id: spec.operation_id,
                expected: target.expected,
                markdown,
            },
        )?;
        let changed = tx.execute("UPDATE processing_jobs SET state='succeeded',finished_at=?1,updated_at=?1 WHERE id=?2 AND state='running' AND completed_units=?3",params![now_ms()?,job_id,completed_units]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("議事録生成の完了競合が発生しました".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(document)
    }

    pub(crate) fn get_minutes_markdown(
        &self,
        meeting: &str,
    ) -> Result<Option<MarkdownDocument>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // Opening the preview only reads the current Markdown. It never
        // converts or publishes content from a removed document format.
        read(&db, meeting)
    }

    pub(crate) fn save_minutes_markdown(
        &self,
        request: SaveMarkdownRequest,
    ) -> Result<MarkdownDocument, String> {
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let document = save(&tx, &request)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(document)
    }
}

#[cfg(test)]
#[path = "minutes_markdown_tests.rs"]
mod tests;
