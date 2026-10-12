//! Read-only integrity checks for structured drafts already saved before the
//! Markdown editor. There is no rich draft creation, editing, undo or switching.
//! Reading/conversion and current backup verification share these checks.
use super::*;
use crate::minutes_document::{MinutesContent, SourceType};
use rusqlite::Transaction;
use std::collections::HashMap;

const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesDraft {
    pub id: String,
    pub meeting_id: String,
    pub base_minutes_id: String,
    pub payload: MinutesContent,
    pub content_hash: String,
    pub lock_version: i64,
    pub updated_at: i64,
}

pub(super) fn hash(text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"mon-minutes-draft-v1");
    hash.update(text.as_bytes());
    format!("{:x}", hash.finalize())
}

pub(super) fn context(
    tx: &Transaction<'_>,
    meeting: &str,
    base: &str,
) -> Result<(MinutesView, MinutesInput), String> {
    let committed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM minutes_revisions WHERE id=?1 AND meeting_id=?2 AND publication_state='committed')",params![base,meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !committed {
        return Err("議事録下書きには確定済みの基底版が必要です".into());
    }
    let view = super::minutes::read_content(tx, meeting, base)?
        .ok_or("議事録の現在版・直前版がありません")?;
    let input = super::minutes_input::read_snapshot(
        tx,
        meeting,
        &view.input_analysis_id,
        &view.input_content_hash,
    )?;
    Ok((view, input))
}

pub(super) fn validate(
    payload: &MinutesContent,
    base: &MinutesContent,
    input: &MinutesInput,
) -> Result<(), String> {
    payload.validate(input)?;
    let original_items: HashMap<_, _> = base.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let original_evidence: HashMap<_, _> =
        base.evidence.iter().map(|e| (e.id.as_str(), e)).collect();
    // New or semantically changed items are manual additions/corrections, not
    // new engine claims. Generated originals remain frozen in the base version.
    for item in &payload.items {
        if item.source_type == SourceType::Engine
            && original_items.get(item.id.as_str()).copied() != Some(item)
        {
            return Err("追加・修正した議事録項目は手動追加として区別してください".into());
        }
    }
    for e in &payload.evidence {
        if let Some(original) = original_evidence.get(e.id.as_str()) {
            if (
                e.minutes_item_id.as_str(),
                e.analysis_revision_id.as_str(),
                e.utterance_id.as_str(),
                e.quote_text.as_str(),
                e.start_us,
                e.end_us,
            ) != (
                original.minutes_item_id.as_str(),
                original.analysis_revision_id.as_str(),
                original.utterance_id.as_str(),
                original.quote_text.as_str(),
                original.start_us,
                original.end_us,
            ) {
                return Err("既存の根拠IDの引用・参照先・時刻は変更できません。新しい根拠として指定してください".into());
            }
        } else if e.source_type == SourceType::Engine {
            return Err("新しい根拠は手動追加として区別してください".into());
        }
    }
    Ok(())
}

pub(super) fn load(
    tx: &Transaction<'_>,
    meeting: &str,
    base: Option<&str>,
) -> Result<Option<MinutesDraft>, String> {
    let row: Option<(String,String,String,String,i64,i64,i64)> = tx.query_row("SELECT id,base_minutes_id,payload_json,content_hash,lock_version,updated_at,schema_version FROM working_drafts WHERE meeting_id=?1 AND target_kind='minutes' AND ((?2 IS NULL AND active=1) OR base_minutes_id=?2)", params![meeting,base],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, base_minutes_id, text, content_hash, lock_version, updated_at, schema)) = row
    else {
        return Ok(None);
    };
    if schema != 1 || text.len() > MAX_BYTES || hash(&text) != content_hash {
        return Err("議事録下書きの形式・内容ハッシュが不正です".into());
    }
    let payload: MinutesContent = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let (view, input) = context(tx, meeting, &base_minutes_id)?;
    validate(&payload, &view.content, &input)?;
    Ok(Some(MinutesDraft {
        id,
        meeting_id: meeting.into(),
        base_minutes_id,
        payload,
        content_hash,
        lock_version,
        updated_at,
    }))
}
