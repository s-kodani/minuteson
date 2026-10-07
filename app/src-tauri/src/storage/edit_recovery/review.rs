//! Discovery includes inactive drafts, but neither listing nor discarding a
//! journal activates a draft, adopts an old analysis or publishes any text.
use super::*;

const PAGE_SIZE: usize = 20;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    pub draft_id: String,
    pub base_analysis_id: String,
    pub operation_id: String,
    pub content_hash: String,
    pub lock_version: i64,
    pub updated_at: i64,
    pub buffer_count: usize,
    pub matches_draft: bool,
    pub current_context: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub items: Vec<Summary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiscardRequest {
    pub meeting_id: String,
    pub draft_id: String,
    pub base_analysis_id: String,
    pub operation_id: String,
    pub expected_draft_lock_version: i64,
    pub expected_draft_content_hash: String,
    pub expected_recovery_lock_version: i64,
    pub expected_recovery_content_hash: String,
}

impl Storage {
    pub(crate) fn list_transcript_edit_recoveries(
        &self,
        meeting: &str,
        after: Option<&str>,
    ) -> Result<Page, String> {
        if after.is_some_and(|cursor| !uuid(cursor)) {
            return Err("回復一覧のカーソルが不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let mut candidates: Vec<(String, String)> = {
            let mut statement = tx.prepare("SELECT d.id,d.base_analysis_id FROM working_drafts d JOIN transcript_edit_recovery r ON r.draft_id=d.id WHERE d.meeting_id=?1 AND d.target_kind='transcript' AND (?2 IS NULL OR d.id>?2) ORDER BY d.id LIMIT 21").map_err(|e|e.to_string())?;
            let rows = statement
                .query_map(params![meeting, after], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let has_more = candidates.len() > PAGE_SIZE;
        candidates.truncate(PAGE_SIZE);
        let next_cursor = has_more.then(|| candidates.last().unwrap().0.clone());
        let mut items = Vec::with_capacity(candidates.len());
        for (id, base) in candidates {
            let draft = drafts::load_draft_for_base(&tx, meeting, Some(&base))?
                .ok_or("回復一覧の下書きがありません")?;
            if draft.id != id {
                return Err("回復一覧の所有が一致しません".into());
            }
            let record = read(&tx, &draft)?.ok_or("回復一覧の記録がありません")?;
            // Validate one bounded payload at a time, retaining only metadata.
            // Include empty tombstones; hiding them must not hide corruption.
            items.push(Summary {
                draft_id: record.draft_id,
                base_analysis_id: record.base_analysis_id,
                operation_id: record.operation_id,
                content_hash: record.content_hash,
                lock_version: record.lock_version,
                updated_at: record.updated_at,
                buffer_count: record.payload.buffers.len(),
                matches_draft: record.matches_draft,
                current_context: record.current_context,
            });
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Page { items, next_cursor })
    }

    pub(crate) fn discard_transcript_edit_recovery(
        &self,
        request: DiscardRequest,
    ) -> Result<Recovery, String> {
        self.require_data_writable()?;
        if !uuid(&request.operation_id)
            || request.expected_draft_lock_version < 0
            || request.expected_recovery_lock_version <= 0
            || !valid_hash(&request.expected_draft_content_hash)
            || !valid_hash(&request.expected_recovery_content_hash)
        {
            return Err("回復破棄の操作・版番号・内容ハッシュが不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let draft =
            drafts::load_draft_for_base(&tx, &request.meeting_id, Some(&request.base_analysis_id))?
                .ok_or("破棄する回復記録の下書きがありません")?;
        if draft.id != request.draft_id {
            return Err("破棄する回復記録の所有が一致しません".into());
        }
        let current = read(&tx, &draft)?.ok_or("破棄する回復記録がありません")?;
        let proof = DiscardProof {
            content_hash: request.expected_recovery_content_hash.clone(),
            lock_version: request.expected_recovery_lock_version,
        };
        let version = request
            .expected_recovery_lock_version
            .checked_add(1)
            .ok_or("回復版番号が範囲外です")?;
        if current.operation_id == request.operation_id {
            if current.payload.discarded_from.as_ref() != Some(&proof)
                || current.lock_version != version
                || current.base_draft_lock_version != request.expected_draft_lock_version
                || current.base_draft_content_hash != request.expected_draft_content_hash
                || !current.payload.buffers.is_empty()
            {
                return Err("同じ回復破棄操作の内容が変更されています".into());
            }
            return Ok(current);
        }
        if current.lock_version != request.expected_recovery_lock_version
            || current.content_hash != request.expected_recovery_content_hash
            || draft.lock_version != request.expected_draft_lock_version
            || draft.content_hash != request.expected_draft_content_hash
        {
            return Err(
                "確認後に下書きまたは回復記録が変更されています。破棄せず保持します".into(),
            );
        }
        let payload = Payload {
            schema_version: 2,
            base_payload: draft.payload.clone(),
            buffers: vec![],
            discarded_from: Some(proof),
        };
        let saved = write_record(&tx, &draft, &request.operation_id, payload, version)?;
        self.require_data_writable()?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests;
