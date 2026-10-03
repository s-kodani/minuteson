use super::*;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_DRAFT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerNameEdit {
    pub speaker_id: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtteranceEdit {
    pub utterance_id: String,
    pub effective_text: String,
    pub speaker_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptDraftPayload {
    pub schema_version: u32,
    pub speaker_names: Vec<SpeakerNameEdit>,
    pub utterances: Vec<UtteranceEdit>,
}

impl Default for TranscriptDraftPayload {
    fn default() -> Self {
        Self {
            schema_version: 1,
            speaker_names: vec![],
            utterances: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptDraft {
    pub id: String,
    pub meeting_id: String,
    pub base_analysis_id: String,
    pub payload: TranscriptDraftPayload,
    pub content_hash: String,
    pub lock_version: i64,
    pub updated_at: i64,
}

impl Storage {
    #[allow(dead_code)]
    pub fn create_transcript_draft(
        &self,
        meeting_id: &str,
        base_analysis_id: &str,
    ) -> Result<TranscriptDraft, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        ensure_committed_analysis(&transaction, meeting_id, base_analysis_id)?;
        if let Some(existing) = load_draft(&transaction, meeting_id)? {
            if existing.base_analysis_id != base_analysis_id {
                return Err("既存の下書きは別の解析版に基づいています。再基準化が必要です".into());
            }
            return Ok(existing);
        }
        let payload = TranscriptDraftPayload::default();
        let serialized = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        let hash = draft_hash(&serialized);
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO working_drafts (id, meeting_id, target_kind, base_analysis_id, payload_json, schema_version, content_hash, lock_version, created_at, updated_at) VALUES (?1, ?2, 'transcript', ?3, ?4, 1, ?5, 0, ?6, ?6)",
            params![id, meeting_id, base_analysis_id, serialized, hash, now],
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(TranscriptDraft {
            id,
            meeting_id: meeting_id.to_owned(),
            base_analysis_id: base_analysis_id.to_owned(),
            payload,
            content_hash: hash,
            lock_version: 0,
            updated_at: now,
        })
    }

    #[allow(dead_code)]
    pub fn load_transcript_draft(
        &self,
        meeting_id: &str,
    ) -> Result<Option<TranscriptDraft>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        load_draft(&connection, meeting_id)
    }

    #[allow(dead_code)]
    pub fn save_transcript_draft(
        &self,
        draft_id: &str,
        expected_lock_version: i64,
        payload: TranscriptDraftPayload,
    ) -> Result<TranscriptDraft, String> {
        if expected_lock_version < 0 {
            return Err("下書きの版番号が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let existing: Option<(String, String, String, String, i64)> = transaction.query_row(
            "SELECT meeting_id, base_analysis_id, payload_json, content_hash, lock_version FROM working_drafts WHERE id = ?1 AND target_kind = 'transcript'",
            params![draft_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (meeting_id, base_analysis_id, old_json, old_hash, lock_version) =
            existing.ok_or("下書きが見つかりません")?;
        if lock_version != expected_lock_version {
            return Err("別の画面で下書きが更新されました。再読込してください".into());
        }
        if draft_hash(&old_json) != old_hash {
            return Err("保存済み下書きの内容が変化しています".into());
        }
        ensure_committed_analysis(&transaction, &meeting_id, &base_analysis_id)?;
        let payload = validate_payload(&transaction, &base_analysis_id, payload)?;
        let serialized = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        if serialized.len() > MAX_DRAFT_BYTES {
            return Err("下書きが大きすぎます".into());
        }
        if serialized == old_json {
            return Ok(TranscriptDraft {
                id: draft_id.to_owned(),
                meeting_id,
                base_analysis_id,
                payload,
                content_hash: old_hash,
                lock_version,
                updated_at: transaction
                    .query_row(
                        "SELECT updated_at FROM working_drafts WHERE id = ?1",
                        params![draft_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?,
            });
        }
        let new_hash = draft_hash(&serialized);
        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM edit_operations WHERE draft_id = ?1",
                params![draft_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO edit_operations (id, draft_id, sequence, operation_kind, payload_json, inverse_json, before_hash, after_hash, created_at) VALUES (?1, ?2, ?3, 'replace_payload', ?4, ?5, ?6, ?7, ?8)",
            params![Uuid::new_v4().to_string(), draft_id, sequence, serialized, old_json, old_hash, new_hash, now],
        ).map_err(|error| error.to_string())?;
        let changed = transaction.execute(
            "UPDATE working_drafts SET payload_json = ?1, content_hash = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
            params![serialized, new_hash, now, draft_id, lock_version],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("下書きの更新競合が発生しました".into());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(TranscriptDraft {
            id: draft_id.to_owned(),
            meeting_id,
            base_analysis_id,
            payload,
            content_hash: new_hash,
            lock_version: lock_version + 1,
            updated_at: now,
        })
    }

    #[allow(dead_code)]
    pub fn undo_last_transcript_edit(
        &self,
        draft_id: &str,
        expected_lock_version: i64,
    ) -> Result<TranscriptDraft, String> {
        if expected_lock_version < 0 {
            return Err("下書きの版番号が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current: Option<(String, String, String, String, i64)> = transaction.query_row(
            "SELECT meeting_id, base_analysis_id, payload_json, content_hash, lock_version FROM working_drafts WHERE id = ?1 AND target_kind = 'transcript'",
            params![draft_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (meeting_id, base_analysis_id, current_json, current_hash, lock_version) =
            current.ok_or("下書きが見つかりません")?;
        if lock_version != expected_lock_version {
            return Err("別の画面で下書きが更新されました。再読込してください".into());
        }
        if draft_hash(&current_json) != current_hash {
            return Err("保存済み下書きの内容が変化しています".into());
        }
        let last_operation: Option<(i64, String, String, String)> = transaction.query_row(
            "SELECT sequence, inverse_json, before_hash, after_hash FROM edit_operations WHERE draft_id = ?1 ORDER BY sequence DESC LIMIT 1",
            params![draft_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (sequence, inverse_json, prior_hash, last_hash) =
            last_operation.ok_or("取り消せる編集がありません")?;
        if last_hash != current_hash || draft_hash(&inverse_json) != prior_hash {
            return Err("編集履歴と下書きの整合が取れません".into());
        }
        ensure_committed_analysis(&transaction, &meeting_id, &base_analysis_id)?;
        let inverse_payload: TranscriptDraftPayload =
            serde_json::from_str(&inverse_json).map_err(|error| error.to_string())?;
        let payload = validate_payload(&transaction, &base_analysis_id, inverse_payload)?;
        if serde_json::to_string(&payload).map_err(|error| error.to_string())? != inverse_json {
            return Err("取り消しデータの形式が不正です".into());
        }
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO edit_operations (id, draft_id, sequence, operation_kind, payload_json, inverse_json, before_hash, after_hash, created_at) VALUES (?1, ?2, ?3, 'undo', ?4, ?5, ?6, ?7, ?8)",
            params![Uuid::new_v4().to_string(), draft_id, sequence + 1, inverse_json, current_json, current_hash, prior_hash, now],
        ).map_err(|error| error.to_string())?;
        let changed = transaction.execute(
            "UPDATE working_drafts SET payload_json = ?1, content_hash = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
            params![inverse_json, prior_hash, now, draft_id, lock_version],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("下書きの更新競合が発生しました".into());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(TranscriptDraft {
            id: draft_id.to_owned(),
            meeting_id,
            base_analysis_id,
            payload,
            content_hash: prior_hash,
            lock_version: lock_version + 1,
            updated_at: now,
        })
    }
}

fn load_draft(
    connection: &Connection,
    meeting_id: &str,
) -> Result<Option<TranscriptDraft>, String> {
    let row: Option<(String, String, String, String, i64, i64)> = connection.query_row(
        "SELECT id, base_analysis_id, payload_json, content_hash, lock_version, updated_at FROM working_drafts WHERE meeting_id = ?1 AND target_kind = 'transcript'",
        params![meeting_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).optional().map_err(|error| error.to_string())?;
    let Some((id, base_analysis_id, serialized, content_hash, lock_version, updated_at)) = row
    else {
        return Ok(None);
    };
    if draft_hash(&serialized) != content_hash {
        return Err("保存済み下書きの内容が変化しています".into());
    }
    let payload = serde_json::from_str(&serialized).map_err(|error| error.to_string())?;
    Ok(Some(TranscriptDraft {
        id,
        meeting_id: meeting_id.to_owned(),
        base_analysis_id,
        payload,
        content_hash,
        lock_version,
        updated_at,
    }))
}

fn ensure_committed_analysis(
    connection: &Connection,
    meeting_id: &str,
    base_analysis_id: &str,
) -> Result<(), String> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state = 'committed')",
        params![base_analysis_id, meeting_id],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if !exists {
        return Err("同じ会議の確定済み解析版が必要です".into());
    }
    Ok(())
}

fn validate_payload(
    connection: &Connection,
    base_analysis_id: &str,
    mut payload: TranscriptDraftPayload,
) -> Result<TranscriptDraftPayload, String> {
    if payload.schema_version != 1 {
        return Err("未対応の下書き形式です".into());
    }
    let speakers: HashSet<String> = {
        let mut statement = connection
            .prepare("SELECT id FROM speakers WHERE analysis_revision_id = ?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![base_analysis_id], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?
    };
    let utterances: HashSet<String> = {
        let mut statement = connection
            .prepare("SELECT id FROM utterances WHERE analysis_revision_id = ?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![base_analysis_id], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?
    };
    let mut seen_speakers = HashSet::new();
    for edit in &payload.speaker_names {
        if !speakers.contains(&edit.speaker_id) || !seen_speakers.insert(&edit.speaker_id) {
            return Err("下書きに不正な話者IDがあります".into());
        }
        if edit
            .display_name
            .as_ref()
            .is_some_and(|name| name.chars().count() > 200)
        {
            return Err("話者名が長すぎます".into());
        }
    }
    let mut seen_utterances = HashSet::new();
    for edit in &mut payload.utterances {
        if !utterances.contains(&edit.utterance_id) || !seen_utterances.insert(&edit.utterance_id) {
            return Err("下書きに不正な発言IDがあります".into());
        }
        if edit.effective_text.chars().count() > 20_000 {
            return Err("発言本文が長すぎます".into());
        }
        edit.speaker_ids.sort();
        if edit.speaker_ids.windows(2).any(|pair| pair[0] == pair[1])
            || edit.speaker_ids.iter().any(|id| !speakers.contains(id))
        {
            return Err("発言の話者割り当てが不正です".into());
        }
    }
    payload
        .speaker_names
        .sort_by(|left, right| left.speaker_id.cmp(&right.speaker_id));
    payload
        .utterances
        .sort_by(|left, right| left.utterance_id.cmp(&right.utterance_id));
    Ok(payload)
}

fn draft_hash(serialized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mon-transcript-draft-v1");
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_analysis_with_utterance(storage: &Storage, meeting_id: &str) -> String {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type = 'recording' WHERE id = ?1",
            params![meeting_id],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', 0, 0)", params![meeting_id]).unwrap();
        db.execute("INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'flac', 48000, 1, 'ready', 0, 0)", params![meeting_id]).unwrap();
        db.execute("INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES ('analysis', ?1, 1, 'engine', 'candidate', ?2, '{}', '{}', ?3, 1, 0)", params![meeting_id, "a".repeat(64), "0".repeat(64)]).unwrap();
        for speaker in ["speaker-1", "speaker-2"] {
            db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES (?1, 'analysis', ?1, ?1, 'all', 'unknown', 0)", params![speaker]).unwrap();
        }
        db.execute("INSERT INTO utterances (id, meeting_id, analysis_revision_id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state) VALUES ('utterance-1', ?1, 'analysis', 'track', 'lineage', 0, 0, 1000, '原文', '原文', 'assigned')", params![meeting_id]).unwrap();
        drop(db);
        storage
            .commit_analysis_candidate(meeting_id, "analysis", 0, false)
            .unwrap();
        "analysis".into()
    }

    #[test]
    fn transcript_draft_autosave_is_versioned_and_persistent() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        let analysis_id = committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, &analysis_id)
            .unwrap();
        assert_eq!(draft.lock_version, 0);
        assert_eq!(
            storage
                .create_transcript_draft(&meeting.id, &analysis_id)
                .unwrap(),
            draft
        );
        let payload = TranscriptDraftPayload {
            schema_version: 1,
            speaker_names: vec![SpeakerNameEdit {
                speaker_id: "speaker-1".into(),
                display_name: Some("田中".into()),
            }],
            utterances: vec![UtteranceEdit {
                utterance_id: "utterance-1".into(),
                effective_text: "修正した発言".into(),
                speaker_ids: vec!["speaker-1".into()],
            }],
        };
        let saved = storage
            .save_transcript_draft(&draft.id, 0, payload.clone())
            .unwrap();
        assert_eq!(saved.lock_version, 1);
        assert_eq!(saved.payload, payload);
        assert_ne!(saved.content_hash, draft.content_hash);
        assert!(storage
            .save_transcript_draft(&draft.id, 0, payload.clone())
            .is_err());
        assert_eq!(
            storage
                .save_transcript_draft(&draft.id, 1, payload.clone())
                .unwrap(),
            saved
        );
        let operations: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM edit_operations WHERE draft_id = ?1",
                params![draft.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(operations, 1);
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(
            reopened.load_transcript_draft(&meeting.id).unwrap(),
            Some(saved)
        );
    }

    #[test]
    fn draft_rejects_unknown_references_and_changed_content() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        let invalid = TranscriptDraftPayload {
            schema_version: 1,
            speaker_names: vec![],
            utterances: vec![UtteranceEdit {
                utterance_id: "utterance-1".into(),
                effective_text: "本文".into(),
                speaker_ids: vec!["other-speaker".into()],
            }],
        };
        assert!(storage
            .save_transcript_draft(&draft.id, 0, invalid)
            .is_err());
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft.clone())
        );
        storage.connection.lock().unwrap().execute("UPDATE working_drafts SET payload_json = '{\"schema_version\":1,\"speaker_names\":[],\"utterances\":[] }' WHERE id = ?1", params![draft.id]).unwrap();
        assert!(storage.load_transcript_draft(&meeting.id).is_err());
    }

    #[test]
    fn draft_rejects_other_meeting_base_and_unknown_utterance() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        let other = storage.create_meeting("別会議").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        assert!(storage
            .create_transcript_draft(&other.id, "analysis")
            .is_err());
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        let invalid = TranscriptDraftPayload {
            schema_version: 1,
            speaker_names: vec![],
            utterances: vec![UtteranceEdit {
                utterance_id: "other-utterance".into(),
                effective_text: "本文".into(),
                speaker_ids: vec![],
            }],
        };
        assert!(storage
            .save_transcript_draft(&draft.id, 0, invalid)
            .is_err());
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft)
        );
    }

    #[test]
    fn undo_requires_current_version_and_preserves_auditable_chain() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let original = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assert!(storage.undo_last_transcript_edit(&original.id, 0).is_err());
        let first = storage
            .save_transcript_draft(
                &original.id,
                0,
                TranscriptDraftPayload {
                    schema_version: 1,
                    speaker_names: vec![],
                    utterances: vec![UtteranceEdit {
                        utterance_id: "utterance-1".into(),
                        effective_text: "修正文".into(),
                        speaker_ids: vec!["speaker-1".into()],
                    }],
                },
            )
            .unwrap();
        assert!(storage.undo_last_transcript_edit(&original.id, 0).is_err());
        let undone = storage.undo_last_transcript_edit(&original.id, 1).unwrap();
        assert_eq!(undone.payload, original.payload);
        assert_eq!(undone.content_hash, original.content_hash);
        assert_eq!(undone.lock_version, 2);
        let redone = storage.undo_last_transcript_edit(&original.id, 2).unwrap();
        assert_eq!(redone.payload, first.payload);
        assert_eq!(redone.content_hash, first.content_hash);
        assert_eq!(redone.lock_version, 3);
        let operations: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM edit_operations WHERE draft_id = ?1",
                params![original.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(operations, 3);
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(redone)
        );
    }
}
