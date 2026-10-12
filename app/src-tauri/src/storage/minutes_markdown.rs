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

fn uuid(value: &str) -> bool {
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

/// Migrate only a meeting's currently effective content, including its saved
/// draft. Existing source data is preserved, never initialized/deleted. Once a
/// Markdown row exists, later reads must not replace subsequent human editing.
/// An unopened invalid legacy document must not prevent app/recording startup.
fn import_current_document(
    db: &mut Connection,
    meeting: &str,
) -> Result<Option<MarkdownDocument>, String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if let Some(document) = read(&tx, meeting)? {
        return Ok(Some(document));
    }
    let base: Option<String> = tx
        .query_row(
            "SELECT current_minutes_id FROM meetings WHERE id=?1",
            [meeting],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(base) = base else {
        return Ok(None);
    };
    let view = super::minutes::read_content(&tx, &meeting, &base)?
        .ok_or("移行する議事録が見つかりません")?;
    let draft = super::minutes_drafts::load(&tx, &meeting, None)?;
    if draft.as_ref().is_some_and(|d| d.base_minutes_id != base) {
        return Err("議事録移行の下書きと現在版が一致しません".into());
    }
    let input = super::minutes_input::read_snapshot(
        &tx,
        &meeting,
        &view.input_analysis_id,
        &view.input_content_hash,
    )?;
    let markdown = draft
        .as_ref()
        .map_or(&view.content, |d| &d.payload)
        .plain_markdown(&input)?;
    let document = save(
        &tx,
        &SaveMarkdownRequest {
            meeting_id: meeting.into(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown,
        },
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(document))
}

impl Storage {
    pub(crate) fn get_minutes_markdown(
        &self,
        meeting: &str,
    ) -> Result<Option<MarkdownDocument>, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if let Some(document) = read(&db, meeting)? {
            return Ok(Some(document));
        }
        let has_legacy: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1 AND current_minutes_id IS NOT NULL)",[meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !has_legacy {
            return Ok(None);
        }
        self.require_data_writable()?;
        import_current_document(&mut db, meeting)
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
        // A missing new-format row is not proof that this meeting has no
        // existing saved minutes. Require their verified first read before a
        // native caller can claim an absent base and overwrite them.
        if read(&tx,&request.meeting_id)?.is_none() && tx.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1 AND current_minutes_id IS NOT NULL)",[&request.meeting_id],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
            return Err("保存済み議事録を読み込んでから保存してください。入力は保持してください".into());
        }
        let document = save(&tx, &request)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(document)
    }
}

#[cfg(test)]
#[path = "minutes_markdown_tests.rs"]
mod tests;
