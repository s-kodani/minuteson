use super::*;
use crate::minutes_document::{MinutesContent, SourceType};
use rusqlite::{Transaction, TransactionBehavior};
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

fn current_base(tx: &Transaction<'_>, meeting: &str, base: &str) -> Result<(), String> {
    let current: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1 AND current_minutes_id=?2)",
            params![meeting, base],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !current {
        return Err("現在の議事録版以外は編集できません".into());
    }
    Ok(())
}

pub(super) fn switch(
    tx: &Transaction<'_>,
    meeting: &str,
    next: &str,
    expected: Option<&DraftStamp>,
) -> Result<(), String> {
    let draft = load(tx, meeting, None)?;
    let stamp = draft.as_ref().map(|d| DraftStamp {
        id: d.id.clone(),
        lock_version: d.lock_version,
    });
    if stamp.as_ref() != expected {
        return Err("確認後に議事録下書きが変更されました。保存後に再確認してください".into());
    }
    if let Some(draft) = draft {
        current_base(tx, meeting, &draft.base_minutes_id)?;
    }
    load(tx, meeting, Some(next))?;
    tx.execute("UPDATE working_drafts SET active=0 WHERE meeting_id=?1 AND target_kind='minutes' AND active=1",params![meeting]).map_err(|e|e.to_string())?;
    tx.execute("UPDATE working_drafts SET active=1 WHERE meeting_id=?1 AND target_kind='minutes' AND base_minutes_id=?2",params![meeting,next]).map_err(|e|e.to_string())?;
    Ok(())
}

impl Storage {
    pub(crate) fn create_minutes_draft(
        &self,
        meeting: &str,
        base: &str,
    ) -> Result<MinutesDraft, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        current_base(&tx, meeting, base)?;
        let (view, _) = context(&tx, meeting, base)?;
        if let Some(draft) = load(&tx, meeting, None)? {
            if draft.base_minutes_id != base {
                return Err("議事録下書きの基底版が現在版と一致しません".into());
            }
            return Ok(draft);
        }
        if let Some(draft) = load(&tx, meeting, Some(base))? {
            tx.execute(
                "UPDATE working_drafts SET active=1 WHERE id=?1",
                params![draft.id],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(draft);
        }
        let payload = view.content;
        let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let content_hash = hash(&text);
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        tx.execute("INSERT INTO working_drafts (id,meeting_id,target_kind,base_minutes_id,payload_json,schema_version,content_hash,lock_version,created_at,updated_at) VALUES (?1,?2,'minutes',?3,?4,1,?5,0,?6,?6)",params![id,meeting,base,text,content_hash,now]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(MinutesDraft {
            id,
            meeting_id: meeting.into(),
            base_minutes_id: base.into(),
            payload,
            content_hash,
            lock_version: 0,
            updated_at: now,
        })
    }

    pub(crate) fn load_minutes_draft(
        &self,
        meeting: &str,
        base: Option<&str>,
    ) -> Result<Option<MinutesDraft>, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        if let Some(base) = base {
            context(&tx, meeting, base)?;
        }
        let draft = load(&tx, meeting, base)?;
        if base.is_none() {
            if let Some(d) = &draft {
                current_base(&tx, meeting, &d.base_minutes_id)?;
            }
        }
        Ok(draft)
    }

    pub(crate) fn save_minutes_draft(
        &self,
        id: &str,
        expected_lock: i64,
        payload: MinutesContent,
    ) -> Result<MinutesDraft, String> {
        if expected_lock < 0 {
            return Err("下書きの版番号が不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let meeting: String=tx.query_row("SELECT meeting_id FROM working_drafts WHERE id=?1 AND target_kind='minutes' AND active=1",params![id],|r|r.get(0)).optional().map_err(|e|e.to_string())?.ok_or("議事録下書きがありません")?;
        let mut draft = load(&tx, &meeting, None)?.ok_or("議事録下書きがありません")?;
        current_base(&tx, &meeting, &draft.base_minutes_id)?;
        if draft.id != id || draft.lock_version != expected_lock {
            return Err("別の画面で議事録下書きが更新されました".into());
        }
        let (view, input) = context(&tx, &meeting, &draft.base_minutes_id)?;
        validate(&payload, &view.content, &input)?;
        let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        if text.len() > MAX_BYTES {
            return Err("議事録下書きが大きすぎます".into());
        }
        if payload == draft.payload {
            return Ok(draft);
        }
        let old = serde_json::to_string(&draft.payload).map_err(|e| e.to_string())?;
        let sequence: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM edit_operations WHERE draft_id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        persist(
            &tx,
            &mut draft,
            payload,
            &text,
            &old,
            sequence,
            "replace_minutes",
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(draft)
    }

    pub(crate) fn undo_minutes_edit(
        &self,
        id: &str,
        expected_lock: i64,
    ) -> Result<MinutesDraft, String> {
        if expected_lock < 0 {
            return Err("下書きの版番号が不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let meeting: String=tx.query_row("SELECT meeting_id FROM working_drafts WHERE id=?1 AND target_kind='minutes' AND active=1",params![id],|r|r.get(0)).optional().map_err(|e|e.to_string())?.ok_or("議事録下書きがありません")?;
        let mut draft = load(&tx, &meeting, None)?.ok_or("議事録下書きがありません")?;
        current_base(&tx, &meeting, &draft.base_minutes_id)?;
        if draft.id != id || draft.lock_version != expected_lock {
            return Err("別の画面で議事録下書きが更新されました".into());
        }
        let prior:Option<(i64,String,String,String)>=tx.query_row("SELECT e.sequence,e.inverse_json,e.before_hash,e.after_hash FROM edit_operations e WHERE e.draft_id=?1 AND e.operation_kind='replace_minutes' AND NOT EXISTS(SELECT 1 FROM edit_operations u WHERE u.draft_id=e.draft_id AND u.operation_kind='undo_minutes:'||e.sequence) ORDER BY e.sequence DESC LIMIT 1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        let (target, text, before, after) = prior.ok_or("取り消せる議事録編集がありません")?;
        if text.len() > MAX_BYTES || hash(&text) != before || after != draft.content_hash {
            return Err("議事録編集履歴と下書きが一致しません".into());
        }
        let payload: MinutesContent = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let (view, input) = context(&tx, &meeting, &draft.base_minutes_id)?;
        validate(&payload, &view.content, &input)?;
        if serde_json::to_string(&payload).map_err(|e| e.to_string())? != text {
            return Err("議事録の取り消しデータが正規形式ではありません".into());
        }
        let old = serde_json::to_string(&draft.payload).map_err(|e| e.to_string())?;
        let sequence: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM edit_operations WHERE draft_id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        persist(
            &tx,
            &mut draft,
            payload,
            &text,
            &old,
            sequence,
            &format!("undo_minutes:{target}"),
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(draft)
    }
}

fn persist(
    tx: &Transaction<'_>,
    draft: &mut MinutesDraft,
    payload: MinutesContent,
    text: &str,
    old: &str,
    sequence: i64,
    kind: &str,
) -> Result<(), String> {
    if hash(old) != draft.content_hash {
        return Err("議事録下書きが正規形式ではありません".into());
    }
    let next = hash(text);
    let now = now_ms()?;
    tx.execute("INSERT INTO edit_operations (id,draft_id,sequence,operation_kind,payload_json,inverse_json,before_hash,after_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![Uuid::new_v4().to_string(),draft.id,sequence,kind,text,old,draft.content_hash,next,now]).map_err(|e|e.to_string())?;
    let changed=tx.execute("UPDATE working_drafts SET payload_json=?1,content_hash=?2,lock_version=lock_version+1,updated_at=?3 WHERE id=?4 AND lock_version=?5 AND active=1",params![text,next,now,draft.id,draft.lock_version]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("議事録下書きの更新競合が発生しました".into());
    }
    draft.payload = payload;
    draft.content_hash = next;
    draft.lock_version += 1;
    draft.updated_at = now;
    Ok(())
}
