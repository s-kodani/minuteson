//! Unconfirmed text and names are durable without publishing an edit or an undo
//! entry. A versioned empty record is a tombstone against delayed older writes.
use super::*;
use rusqlite::TransactionBehavior;
use std::collections::HashSet;

pub(crate) const MAX_BYTES: usize = 4 * 1024 * 1024;

pub(crate) mod review;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiscardProof {
    pub content_hash: String,
    pub lock_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TextBuffer {
    pub utterance_id: String,
    pub text: String,
    pub speaker_ids: Vec<String>,
    pub had_override: bool,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct NameBuffer {
    pub speaker_id: String,
    pub name: Option<String>,
    pub had_override: bool,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Payload {
    pub schema_version: u32,
    // Retain the exact acknowledged context even after a split, adoption or
    // durable Undo changes the current draft. Stale buffers remain readable.
    pub base_payload: TranscriptDraftPayload,
    pub buffers: Vec<TextBuffer>,
    // v3 adds names. Omit this field for v1/v2 so existing JSON and hashes
    // remain byte-for-byte compatible; v2 is still an entirely empty proof.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_buffers: Vec<NameBuffer>,
    // v2 empty tombstones identify the exact preview that was discarded, so a
    // lost acknowledgement cannot disguise reuse of an operation with new input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discarded_from: Option<DiscardProof>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Recovery {
    pub draft_id: String,
    pub meeting_id: String,
    pub base_analysis_id: String,
    pub operation_id: String,
    pub base_draft_lock_version: i64,
    pub base_draft_content_hash: String,
    pub payload: Payload,
    pub content_hash: String,
    pub lock_version: i64,
    pub updated_at: i64,
    pub matches_draft: bool,
    pub current_draft_lock_version: i64,
    pub current_draft_content_hash: String,
    pub current_context: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SaveRequest {
    pub meeting_id: String,
    pub draft_id: String,
    pub base_analysis_id: String,
    pub operation_id: String,
    pub expected_draft_lock_version: i64,
    pub expected_recovery_lock_version: i64,
    pub buffers: Vec<TextBuffer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_buffers: Vec<NameBuffer>,
}

fn uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn normalize(mut buffers: Vec<TextBuffer>) -> Result<Vec<TextBuffer>, String> {
    if buffers.len() > 128 {
        return Err("同時に回復保存できる発言は128件までです".into());
    }
    buffers.sort_by(|a, b| a.utterance_id.cmp(&b.utterance_id));
    if buffers
        .windows(2)
        .any(|pair| pair[0].utterance_id == pair[1].utterance_id)
    {
        return Err("回復対象の発言が重複しています".into());
    }
    for buffer in &mut buffers {
        if buffer.text.chars().count() > 20_000 || buffer.value.chars().count() > 20_000 {
            return Err("回復本文は1発言20000文字までです".into());
        }
        buffer.speaker_ids.sort();
        if buffer.speaker_ids.len() > 100
            || buffer.speaker_ids.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err("回復対象の話者が不正です".into());
        }
    }
    Ok(buffers)
}

fn normalize_names(mut names: Vec<NameBuffer>) -> Result<Vec<NameBuffer>, String> {
    if names.len() > 100 {
        return Err("同時に回復保存できる話者名は100件までです".into());
    }
    names.sort_by(|a, b| a.speaker_id.cmp(&b.speaker_id));
    if names
        .windows(2)
        .any(|pair| pair[0].speaker_id == pair[1].speaker_id)
    {
        return Err("回復対象の話者名が重複しています".into());
    }
    if names.iter().any(|name| {
        name.value.chars().count() > 200
            || name
                .name
                .as_ref()
                .is_some_and(|value| value.chars().count() > 200)
    }) {
        return Err("回復する話者名は200文字までです".into());
    }
    Ok(names)
}

pub(super) fn validate(db: &Connection, base: &str, payload: &Payload) -> Result<(), String> {
    let format = match (payload.schema_version, &payload.discarded_from) {
        (1, None) => payload.name_buffers.is_empty(),
        (3, None) => !payload.name_buffers.is_empty(),
        (2, Some(proof)) => {
            payload.buffers.is_empty()
                && payload.name_buffers.is_empty()
                && proof.lock_version > 0
                && valid_hash(&proof.content_hash)
        }
        _ => false,
    };
    if !format
        || normalize(payload.buffers.clone())? != payload.buffers
        || normalize_names(payload.name_buffers.clone())? != payload.name_buffers
    {
        return Err("回復下書きの形式が不正です".into());
    }
    // ID remapping can change lexical order without changing the exact saved
    // baseline/hash. Validate its meaning, preserving those original bytes.
    drafts::validate_payload(db, base, payload.base_payload.clone())?;
    let mut ids: HashSet<String> = db
        .prepare("SELECT id FROM utterances WHERE analysis_revision_id=?1")
        .map_err(|e| e.to_string())?
        .query_map(params![base], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for split in &payload.base_payload.utterance_splits {
        ids.remove(&split.source_utterance_id);
        ids.extend(split.parts.iter().map(|p| p.id.clone()));
    }
    let mut speakers: HashSet<String> = db
        .prepare("SELECT id FROM speakers WHERE analysis_revision_id=?1")
        .map_err(|e| e.to_string())?
        .query_map(params![base], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    speakers.extend(
        payload
            .base_payload
            .manual_speakers
            .iter()
            .map(|s| s.id.clone()),
    );
    if payload.buffers.iter().any(|b| {
        !ids.contains(&b.utterance_id) || b.speaker_ids.iter().any(|id| !speakers.contains(id))
    }) || payload
        .name_buffers
        .iter()
        .any(|name| !speakers.contains(&name.speaker_id))
    {
        return Err("回復下書きは同じ解析版の発言と話者に限ります".into());
    }
    if serde_json::to_vec(payload)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_BYTES
    {
        return Err("回復下書きが容量上限を超えています".into());
    }
    Ok(())
}

pub(super) fn read(db: &Connection, draft: &TranscriptDraft) -> Result<Option<Recovery>, String> {
    let row: Option<(String,i64,String,String,String,i64,i64)> = db.query_row("SELECT operation_id,base_draft_lock_version,base_draft_content_hash,payload_json,content_hash,lock_version,updated_at FROM transcript_edit_recovery WHERE draft_id=?1", params![draft.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e| e.to_string())?;
    let Some((operation_id, base_version, base_hash, json, hash, version, updated_at)) = row else {
        return Ok(None);
    };
    if json.len() > MAX_BYTES
        || drafts::draft_hash(&json) != hash
        || !uuid(&operation_id)
        || base_version < 0
        || base_version > draft.lock_version
        || version <= 0
        || updated_at < 0
    {
        return Err("回復下書きの内容または版番号が不正です".into());
    }
    let payload: Payload =
        serde_json::from_str(&json).map_err(|e| format!("回復下書きのJSONが不正です: {e}"))?;
    validate(db, &draft.base_analysis_id, &payload)?;
    if payload
        .discarded_from
        .as_ref()
        .is_some_and(|proof| proof.lock_version.checked_add(1) != Some(version))
    {
        return Err("破棄した回復記録の版番号が一致しません".into());
    }
    let baseline = serde_json::to_string(&payload.base_payload).map_err(|e| e.to_string())?;
    if drafts::draft_hash(&baseline) != base_hash {
        return Err("回復下書きの基底内容が一致しません".into());
    }
    let current_context: bool = db.query_row("SELECT d.active=1 AND coalesce(m.current_analysis_id=d.base_analysis_id,0) FROM working_drafts d JOIN meetings m ON m.id=d.meeting_id WHERE d.id=?1 AND d.meeting_id=?2 AND d.base_analysis_id=?3 AND d.target_kind='transcript'",params![draft.id,draft.meeting_id,draft.base_analysis_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(Some(Recovery {
        draft_id: draft.id.clone(),
        meeting_id: draft.meeting_id.clone(),
        base_analysis_id: draft.base_analysis_id.clone(),
        operation_id,
        base_draft_lock_version: base_version,
        base_draft_content_hash: base_hash.clone(),
        payload,
        content_hash: hash,
        lock_version: version,
        updated_at,
        matches_draft: base_version == draft.lock_version && base_hash == draft.content_hash,
        current_draft_lock_version: draft.lock_version,
        current_draft_content_hash: draft.content_hash.clone(),
        current_context,
    }))
}

impl Storage {
    pub(crate) fn transcript_edit_recovery(
        &self,
        meeting: &str,
        draft_id: &str,
        base: &str,
    ) -> Result<Option<Recovery>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let draft = drafts::load_draft_for_base(&db, meeting, Some(base))?
            .ok_or("回復対象の下書きがありません")?;
        if draft.id != draft_id {
            return Err("回復対象の下書きが一致しません".into());
        }
        read(&db, &draft)
    }

    pub(crate) fn save_transcript_edit_recovery(
        &self,
        request: SaveRequest,
    ) -> Result<Recovery, String> {
        self.require_data_writable()?;
        if !uuid(&request.operation_id)
            || request.expected_draft_lock_version < 0
            || request.expected_recovery_lock_version < 0
        {
            return Err("回復保存の操作IDまたは版番号が不正です".into());
        }
        let buffers = normalize(request.buffers)?;
        let names = normalize_names(request.name_buffers)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let draft =
            drafts::load_draft(&tx, &request.meeting_id)?.ok_or("回復対象の下書きがありません")?;
        if draft.id != request.draft_id || draft.base_analysis_id != request.base_analysis_id {
            return Err("回復保存の会議・下書き・解析版が一致しません".into());
        }
        let current = read(&tx, &draft)?;
        if let Some(saved) = &current {
            // A retry of the same request may prove a lost acknowledgement,
            // but cannot replace a later operation or revise that request.
            if saved.operation_id == request.operation_id {
                if saved.payload.discarded_from.is_some()
                    || saved.base_draft_lock_version != request.expected_draft_lock_version
                    || saved.lock_version
                        != request
                            .expected_recovery_lock_version
                            .checked_add(1)
                            .ok_or("回復版番号が範囲外です")?
                    || saved.payload.buffers != buffers
                    || saved.payload.name_buffers != names
                {
                    return Err("同じ回復操作の内容が変更されています".into());
                }
                return Ok(current.unwrap());
            }
        }
        if draft.lock_version != request.expected_draft_lock_version
            || current.as_ref().map_or(0, |r| r.lock_version)
                != request.expected_recovery_lock_version
        {
            return Err("下書きまたは回復保存が更新されています。結果を再確認してください".into());
        }
        let payload = Payload {
            schema_version: if names.is_empty() { 1 } else { 3 },
            base_payload: draft.payload.clone(),
            buffers,
            name_buffers: names,
            discarded_from: None,
        };
        let version = request
            .expected_recovery_lock_version
            .checked_add(1)
            .ok_or("回復版番号が範囲外です")?;
        let saved = write_record(&tx, &draft, &request.operation_id, payload, version)?;
        self.require_data_writable()?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(saved)
    }
}

fn write_record(
    tx: &rusqlite::Transaction<'_>,
    draft: &TranscriptDraft,
    operation: &str,
    payload: Payload,
    version: i64,
) -> Result<Recovery, String> {
    validate(tx, &draft.base_analysis_id, &payload)?;
    let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let hash = drafts::draft_hash(&json);
    tx.execute("INSERT INTO transcript_edit_recovery VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(draft_id) DO UPDATE SET operation_id=excluded.operation_id,base_draft_lock_version=excluded.base_draft_lock_version,base_draft_content_hash=excluded.base_draft_content_hash,payload_json=excluded.payload_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![draft.id,operation,draft.lock_version,draft.content_hash,json,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
    read(tx, draft)?.ok_or("回復保存結果がありません".into())
}

// Include inactive drafts and stale buffers in backup integrity checks. Their
// old context is preserved, never adopted over a newer/current transcript.
pub(super) fn verify_all(db: &Connection, cancelled: &impl Fn() -> bool) -> Result<(), String> {
    let exists: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='transcript_edit_recovery')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {
        return Ok(());
    }
    if db.query_row("SELECT EXISTS(SELECT 1 FROM transcript_edit_recovery r LEFT JOIN working_drafts d ON d.id=r.draft_id WHERE d.id IS NULL OR d.target_kind!='transcript')", [], |r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        return Err("回復下書きの所有が一致しません".into());
    }
    let mut query=db.prepare("SELECT d.id,d.meeting_id,d.base_analysis_id FROM transcript_edit_recovery r JOIN working_drafts d ON d.id=r.draft_id ORDER BY d.id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if cancelled() {
            return Err("回復下書きの検査を中止しました".into());
        }
        let (id, meeting, base): (String, String, String) = (
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
        );
        let draft = drafts::load_draft_for_base(db, &meeting, Some(&base))?
            .ok_or("回復対象の下書きがありません")?;
        if draft.id != id || read(db, &draft)?.is_none() {
            return Err("回復下書きの所有が一致しません".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    super::backup::input_snapshots::remove_schema_for_test(db);
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=28)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        // Only empty test-owned images may be converted to older fixtures.
        assert_eq!(
            db.query_row("SELECT count(*) FROM transcript_edit_recovery", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        db.execute_batch(
            "DROP TABLE transcript_edit_recovery; DELETE FROM schema_migrations WHERE version=28;",
        )
        .unwrap();
    }
}

#[cfg(test)]
mod name_tests;
#[cfg(test)]
mod tests;
