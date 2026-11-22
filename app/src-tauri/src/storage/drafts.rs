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
pub struct ManualSpeaker {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakerMerge {
    pub source_id: String,
    pub target_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptDraftPayload {
    pub schema_version: u32,
    pub speaker_names: Vec<SpeakerNameEdit>,
    pub utterances: Vec<UtteranceEdit>,
    pub manual_speakers: Vec<ManualSpeaker>,
    pub speaker_merges: Vec<SpeakerMerge>,
    pub utterance_splits: Vec<super::draft_splits::UtteranceSplit>,
}

impl Default for TranscriptDraftPayload {
    fn default() -> Self {
        Self {
            schema_version: crate::current_formats::TRANSCRIPT_DRAFT,
            speaker_names: vec![],
            utterances: vec![],
            manual_speakers: vec![],
            speaker_merges: vec![],
            utterance_splits: vec![],
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
        self.require_draft_writable(&connection, meeting_id)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        ensure_committed_analysis(&transaction, meeting_id, base_analysis_id)?;
        if let Some(existing) =
            self.load_or_discard_draft(&transaction, meeting_id, Some(base_analysis_id))?
        {
            return Ok(existing);
        }
        let payload = TranscriptDraftPayload::default();
        let serialized = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        let hash = draft_hash(&serialized);
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO working_drafts (id, meeting_id, target_kind, base_analysis_id, payload_json, schema_version, content_hash, lock_version, created_at, updated_at) VALUES (?1, ?2, 'transcript', ?3, ?4, 4, ?5, 0, ?6, ?6)",
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
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| error.to_string())?;
        let draft = self.load_or_discard_draft(&transaction, meeting_id, None)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(draft)
    }

    fn load_or_discard_draft(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        meeting: &str,
        known_current: Option<&str>,
    ) -> Result<Option<TranscriptDraft>, String> {
        let Some(saved) = read_saved_draft(transaction, meeting, None)? else {
            return Ok(None);
        };
        let base = &saved.base_analysis_id;
        match known_current {
            Some(current) if current != base.as_str() => {
                return Err("既存の下書きは別の解析版に基づいています。再基準化が必要です".into());
            }
            Some(_) => {}
            None => ensure_committed_analysis(transaction, meeting, base)?,
        }
        match saved
            .draft
            .and_then(|draft| validated_draft(transaction, draft))
        {
            Ok(draft) => Ok(Some(draft)),
            Err(DraftValidation::Invalid(_)) => {
                verify_discard_base(transaction, meeting, base)?;
                if !self.data_write_blocked()
                    && !self.maintenance.load(std::sync::atomic::Ordering::Acquire)
                    && !super::meeting_access::reserved(transaction, meeting)?
                {
                    #[cfg(any(unix, windows))]
                    self.managed_anchor.verify()?;
                    super::deletion::database::trusted_schema(transaction)?;
                    let removed = transaction
                        .execute(
                            "DELETE FROM working_drafts WHERE rowid=?1 AND meeting_id=?2
                         AND target_kind='transcript' AND base_analysis_id=?3",
                            params![saved.rowid, meeting, base],
                        )
                        .map_err(|error| error.to_string())?;
                    if removed != 1 {
                        return Err("削除前に下書きの状態が変わりました".into());
                    }
                }
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn require_draft_writable(&self, db: &Connection, meeting: &str) -> Result<(), String> {
        self.require_data_writable()?;
        if self.maintenance.load(std::sync::atomic::Ordering::Acquire) {
            return Err("保存領域の切替・保守が進行中です".into());
        }
        if super::meeting_access::reserved(db, meeting)? {
            return Err("この会議は削除の確認・処理中です".into());
        }
        Ok(())
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
        let result = save_in_transaction(&transaction, draft_id, expected_lock_version, payload)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }
}

pub(super) fn load_draft(
    connection: &Connection,
    meeting_id: &str,
) -> Result<Option<TranscriptDraft>, String> {
    load_draft_for_base(connection, meeting_id, None)
}

pub(super) fn load_draft_for_base(
    connection: &Connection,
    meeting_id: &str,
    base: Option<&str>,
) -> Result<Option<TranscriptDraft>, String> {
    read_saved_draft(connection, meeting_id, base)?
        .map(|saved| saved.draft.map_err(|error| error.to_string()))
        .transpose()
}

pub(super) enum DraftValidation {
    Invalid(&'static str),
    Unsupported,
    Database(rusqlite::Error),
}

impl std::fmt::Display for DraftValidation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => output.write_str(message),
            Self::Unsupported => output.write_str("未対応の下書き形式です"),
            Self::Database(error) => write!(output, "{error}"),
        }
    }
}

impl From<&'static str> for DraftValidation {
    fn from(message: &'static str) -> Self {
        Self::Invalid(message)
    }
}

impl From<rusqlite::Error> for DraftValidation {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

struct SavedDraftRow {
    rowid: i64,
    base_analysis_id: String,
    draft: Result<TranscriptDraft, DraftValidation>,
}

fn read_saved_draft(
    connection: &Connection,
    meeting: &str,
    base: Option<&str>,
) -> Result<Option<SavedDraftRow>, String> {
    connection
        .query_row(
            "SELECT rowid,base_analysis_id,id,schema_version,
                CASE WHEN length(CAST(payload_json AS BLOB))<=?3 THEN payload_json END,
                content_hash,lock_version,updated_at
         FROM working_drafts WHERE meeting_id=?1 AND target_kind='transcript'
         AND (?2 IS NULL OR base_analysis_id=?2)",
            params![meeting, base, MAX_DRAFT_BYTES as i64],
            |row| {
                let rowid = row.get(0)?;
                let base_analysis_id: String = row.get(1)?;
                let draft = decode_saved_draft(row, meeting, &base_analysis_id);
                Ok(SavedDraftRow {
                    rowid,
                    base_analysis_id,
                    draft,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn decode_saved_draft(
    row: &rusqlite::Row<'_>,
    meeting: &str,
    base: &str,
) -> Result<TranscriptDraft, DraftValidation> {
    let malformed_cell = |error| match error {
        rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::FromSqlConversionFailure(..) => {
            DraftValidation::Invalid("保存済み下書きの形式が不正です")
        }
        other => DraftValidation::Database(other),
    };
    let schema_version = row.get::<_, i64>(3).map_err(malformed_cell)?;
    let id: String = row.get(2).map_err(malformed_cell)?;
    let serialized: String = row
        .get::<_, Option<String>>(4)
        .map_err(malformed_cell)?
        .ok_or("保存済み下書きが大きすぎます")?;
    let content_hash: String = row.get(5).map_err(malformed_cell)?;
    let lock_version: i64 = row.get(6).map_err(malformed_cell)?;
    let updated_at: i64 = row.get(7).map_err(malformed_cell)?;
    if schema_version <= 0
        || id.is_empty()
        || id.len() > 256
        || id.contains('\0')
        || lock_version < 0
        || updated_at < 0
        || draft_hash(&serialized) != content_hash
    {
        return Err("保存済み下書きの内容が変化しています".into());
    }
    if schema_version != i64::from(crate::current_formats::TRANSCRIPT_DRAFT) {
        let header: serde_json::Value =
            serde_json::from_str(&serialized).map_err(|_| "保存済み下書きの形式が不正です")?;
        if header
            .get("schema_version")
            .and_then(serde_json::Value::as_i64)
            != Some(schema_version)
        {
            return Err("保存済み下書きの形式版が一致しません".into());
        }
        // Matching version evidence can identify an intact draft from a newer reader.
        return Err(DraftValidation::Unsupported);
    }
    let payload: TranscriptDraftPayload =
        serde_json::from_str(&serialized).map_err(|_| "保存済み下書きの形式が不正です")?;
    if i64::from(payload.schema_version) != schema_version {
        return Err("保存済み下書きの形式版が一致しません".into());
    }
    Ok(TranscriptDraft {
        id,
        meeting_id: meeting.to_owned(),
        base_analysis_id: base.to_owned(),
        payload,
        content_hash,
        lock_version,
        updated_at,
    })
}

fn validated_draft(
    db: &Connection,
    mut draft: TranscriptDraft,
) -> Result<TranscriptDraft, DraftValidation> {
    draft.payload = validate_payload_inner(db, &draft.base_analysis_id, draft.payload)?;
    Ok(draft)
}

// Callers have already verified and loaded the current committed body under
// the same transaction. This read-only path never authorizes removal.
pub(super) fn draft_for_verified_body(
    db: &Connection,
    meeting: &str,
    base: &str,
) -> Result<Option<TranscriptDraft>, String> {
    let Some(saved) = read_saved_draft(db, meeting, None)? else {
        return Ok(None);
    };
    if saved.base_analysis_id != base {
        return Err("下書きの基底と現在版が一致しません".into());
    }
    match saved.draft.and_then(|draft| validated_draft(db, draft)) {
        Ok(draft) => Ok(Some(draft)),
        Err(DraftValidation::Invalid(_)) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn verify_discard_base(
    db: &rusqlite::Transaction<'_>,
    meeting: &str,
    base: &str,
) -> Result<(), String> {
    let hash: String = db
        .query_row(
            "SELECT a.content_hash FROM analysis_revisions a
         JOIN meetings m ON m.current_analysis_id=a.id AND m.id=a.meeting_id
         WHERE a.id=?1 AND m.id=?2 AND a.publication_state='committed'",
            params![base, meeting],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if super::revisions::analysis_content_hash(db, base)? != hash {
        return Err("文字起こしの現在版を検証できないため下書きを保持しています".into());
    }
    let body = super::analysis::read_stored_analysis_content(db, meeting, base, &hash)?;
    if body.utterances.len() > 100_000 || body.speakers.len() > 100 {
        return Err(
            "文字起こしの現在版が対応する件数を超えているため下書きを保持しています".into(),
        );
    }
    Ok(())
}

fn ensure_committed_analysis(
    connection: &Connection,
    meeting_id: &str,
    base_analysis_id: &str,
) -> Result<(), String> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM analysis_revisions a JOIN meetings m ON m.id = a.meeting_id AND m.current_analysis_id = a.id WHERE a.id = ?1 AND a.meeting_id = ?2 AND a.publication_state = 'committed')",
        params![base_analysis_id, meeting_id],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if !exists {
        return Err("同じ会議の現在の確定済み解析版が必要です".into());
    }
    Ok(())
}

pub(super) fn validate_payload(
    connection: &Connection,
    base_analysis_id: &str,
    payload: TranscriptDraftPayload,
) -> Result<TranscriptDraftPayload, String> {
    validate_payload_inner(connection, base_analysis_id, payload).map_err(|error| error.to_string())
}

fn validate_payload_inner(
    connection: &Connection,
    base_analysis_id: &str,
    mut payload: TranscriptDraftPayload,
) -> Result<TranscriptDraftPayload, DraftValidation> {
    if payload.schema_version != crate::current_formats::TRANSCRIPT_DRAFT {
        return Err(DraftValidation::Unsupported);
    }
    let mut speakers: HashSet<String> = {
        let mut statement =
            connection.prepare("SELECT id FROM speakers WHERE analysis_revision_id = ?1")?;
        let rows = statement.query_map(params![base_analysis_id], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    if speakers.len() + payload.manual_speakers.len() > 100 {
        return Err("話者は推定元を含め100件までです".into());
    }
    for speaker in &payload.manual_speakers {
        let valid_id = speaker
            .id
            .strip_prefix("manual:")
            .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id));
        if !valid_id
            || !speakers.insert(speaker.id.clone())
            || speaker.label.trim().is_empty()
            || speaker.label.chars().count() > 200
        {
            return Err("追加話者のIDまたはラベルが不正です".into());
        }
        let collision: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM speakers WHERE analysis_revision_id = ?1 AND lineage_id = ?2)",
            params![base_analysis_id, speaker.id], |r| r.get(0),
        )?;
        if collision {
            return Err("追加話者の由来が既存話者と重複しています".into());
        }
    }
    let mut merged = HashSet::new();
    for merge in &payload.speaker_merges {
        if merge.source_id == merge.target_id
            || !speakers.contains(&merge.source_id)
            || !speakers.contains(&merge.target_id)
            || !merged.insert(&merge.source_id)
        {
            return Err("話者統合の参照が不正です".into());
        }
    }
    if payload
        .speaker_merges
        .iter()
        .any(|m| merged.contains(&m.target_id))
    {
        return Err("話者統合は循環や多段参照を含めず、最終話者を指定してください".into());
    }
    let mut utterances: HashSet<String> = {
        let mut statement =
            connection.prepare("SELECT id FROM utterances WHERE analysis_revision_id = ?1")?;
        let rows = statement.query_map(params![base_analysis_id], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    super::draft_splits::validate_splits(
        connection,
        base_analysis_id,
        &mut payload,
        &speakers,
        &mut utterances,
    )?;
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
    payload.manual_speakers.sort_by(|a, b| a.id.cmp(&b.id));
    payload
        .speaker_merges
        .sort_by(|a, b| a.source_id.cmp(&b.source_id));
    Ok(payload)
}

pub(super) fn merged_assignments(payload: &TranscriptDraftPayload, ids: &[String]) -> Vec<String> {
    let mapping: std::collections::HashMap<_, _> = payload
        .speaker_merges
        .iter()
        .map(|m| (m.source_id.as_str(), m.target_id.as_str()))
        .collect();
    let mut result: Vec<_> = ids
        .iter()
        .map(|id| mapping.get(id.as_str()).unwrap_or(&id.as_str()).to_string())
        .collect();
    result.sort();
    result.dedup();
    result
}

pub(super) fn draft_hash(serialized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mon-transcript-draft-v1");
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_speakers_and_merges_round_trip_undo_in_current_format() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("話者編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assert_eq!(
            serde_json::to_string(&draft.payload).unwrap(),
            r#"{"schema_version":4,"speaker_names":[],"utterances":[],"manual_speakers":[],"speaker_merges":[],"utterance_splits":[]}"#
        );
        let id = format!("manual:{}", Uuid::new_v4());
        let mut payload = draft.payload.clone();
        payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        payload.manual_speakers.push(ManualSpeaker {
            id: id.clone(),
            label: "手動話者 1".into(),
        });
        payload.speaker_names.push(SpeakerNameEdit {
            speaker_id: id.clone(),
            display_name: Some("田中".into()),
        });
        payload.speaker_merges.push(SpeakerMerge {
            source_id: "speaker-1".into(),
            target_id: id.clone(),
        });
        let saved = storage
            .save_transcript_draft(&draft.id, 0, payload)
            .unwrap();
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(saved.clone())
        );
        assert!(storage
            .save_transcript_draft(&draft.id, 0, saved.payload.clone())
            .is_err());
        drop(storage);
        let storage = Storage::open(root.path().to_owned()).unwrap();
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(saved.clone())
        );
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT schema_version FROM working_drafts WHERE id=?1",
                params![draft.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM speakers WHERE analysis_revision_id='analysis'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }

    #[test]
    fn structural_drafts_reject_invalid_ids_chains_cycles_duplicates_and_bounds() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("話者編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        let mut valid = draft.payload.clone();
        valid.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        valid.manual_speakers.push(ManualSpeaker {
            id: format!("manual:{}", Uuid::new_v4()),
            label: "追加".into(),
        });
        let custom = valid.manual_speakers[0].id.clone();
        let mut cases = vec![];
        let mut p = valid.clone();
        p.schema_version = 1;
        cases.push(p);
        let mut p = valid.clone();
        p.manual_speakers[0].id = "foreign-id".into();
        cases.push(p);
        let mut p = valid.clone();
        p.manual_speakers.push(p.manual_speakers[0].clone());
        cases.push(p);
        let mut p = valid.clone();
        p.speaker_merges = vec![SpeakerMerge {
            source_id: "speaker-1".into(),
            target_id: "foreign".into(),
        }];
        cases.push(p);
        let mut p = valid.clone();
        p.speaker_merges = vec![
            SpeakerMerge {
                source_id: "speaker-1".into(),
                target_id: "speaker-2".into(),
            },
            SpeakerMerge {
                source_id: "speaker-2".into(),
                target_id: custom.clone(),
            },
        ];
        cases.push(p);
        let mut p = valid.clone();
        p.speaker_merges = vec![
            SpeakerMerge {
                source_id: "speaker-1".into(),
                target_id: "speaker-2".into(),
            },
            SpeakerMerge {
                source_id: "speaker-2".into(),
                target_id: "speaker-1".into(),
            },
        ];
        cases.push(p);
        let mut p = valid.clone();
        p.manual_speakers[0].label = "名".repeat(201);
        cases.push(p);
        let mut p = valid.clone();
        p.manual_speakers = (0..99)
            .map(|_| ManualSpeaker {
                id: format!("manual:{}", Uuid::new_v4()),
                label: "追加".into(),
            })
            .collect();
        cases.push(p);
        for p in cases {
            assert!(storage.save_transcript_draft(&draft.id, 0, p).is_err());
        }
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft)
        );
    }

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
            schema_version: crate::current_formats::TRANSCRIPT_DRAFT,
            speaker_names: vec![SpeakerNameEdit {
                speaker_id: "speaker-1".into(),
                display_name: Some("田中".into()),
            }],
            utterances: vec![UtteranceEdit {
                utterance_id: "utterance-1".into(),
                effective_text: "修正した発言".into(),
                speaker_ids: vec!["speaker-1".into()],
            }],
            ..Default::default()
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
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(
            reopened.load_transcript_draft(&meeting.id).unwrap(),
            Some(saved)
        );
    }

    #[test]
    fn draft_rejects_unknown_references_and_discards_a_damaged_saved_row() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        let invalid = TranscriptDraftPayload {
            schema_version: crate::current_formats::TRANSCRIPT_DRAFT,
            speaker_names: vec![],
            utterances: vec![UtteranceEdit {
                utterance_id: "utterance-1".into(),
                effective_text: "本文".into(),
                speaker_ids: vec!["other-speaker".into()],
            }],
            ..Default::default()
        };
        assert!(storage
            .save_transcript_draft(&draft.id, 0, invalid)
            .is_err());
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft.clone())
        );
        let verified = storage.load_current_analysis(&meeting.id).unwrap().unwrap();
        {
            let db = storage.connection.lock().unwrap();
            db.execute("UPDATE working_drafts SET payload_json = '{\"schema_version\":1,\"speaker_names\":[],\"utterances\":[] }' WHERE id = ?1", params![draft.id]).unwrap();
            assert!(load_draft(&db, &meeting.id).is_err());
            assert_eq!(stored_draft_count(&db, &draft.id), 1);
        }
        assert_eq!(storage.load_transcript_draft(&meeting.id).unwrap(), None);
        assert_eq!(storage.load_transcript_draft(&meeting.id).unwrap(), None);
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            0
        );
        assert_eq!(
            storage.load_current_analysis(&meeting.id).unwrap(),
            Some(verified)
        );
        let fresh = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assert_ne!(fresh.id, draft.id);
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(fresh)
        );
    }

    fn stored_draft_count(db: &Connection, id: &str) -> i64 {
        db.query_row(
            "SELECT count(*) FROM working_drafts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn assign_invalid_reference(storage: &Storage, draft: &TranscriptDraft) {
        let mut payload = draft.payload.clone();
        payload.utterances.push(UtteranceEdit {
            utterance_id: "invalid-reference".into(),
            effective_text: "存在しない発言の編集".into(),
            speaker_ids: vec![],
        });
        let json = serde_json::to_string(&payload).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE working_drafts SET payload_json=?1,content_hash=?2 WHERE id=?3",
                params![json, draft_hash(&json), draft.id],
            )
            .unwrap();
    }

    #[test]
    fn begin_edit_replaces_invalid_same_base_draft_and_uses_current_text() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assign_invalid_reference(&storage, &draft);
        {
            let db = storage.connection.lock().unwrap();
            let tx = db.unchecked_transaction().unwrap();
            let (body, working, _, _, _) =
                super::super::minutes_input::current_working(&tx, &meeting.id).unwrap();
            assert_eq!(body.utterances[0].effective_text, "原文");
            assert!(working.is_none());
            assert_eq!(stored_draft_count(&tx, &draft.id), 1);
        }
        let fresh = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assert_ne!(fresh.id, draft.id);
        assert_eq!(fresh.payload, TranscriptDraftPayload::default());
        let mut payload = fresh.payload;
        payload.utterances.push(UtteranceEdit {
            utterance_id: "utterance-1".into(),
            effective_text: "新しい編集".into(),
            speaker_ids: vec![],
        });
        assert!(storage.save_transcript_draft(&fresh.id, 0, payload).is_ok());
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            0
        );
    }

    #[test]
    fn mismatched_draft_version_is_discarded_but_intact_newer_format_is_retained() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE working_drafts SET schema_version=9 WHERE id=?1",
                [&draft.id],
            )
            .unwrap();
        }
        assert_eq!(storage.load_transcript_draft(&meeting.id).unwrap(), None);
        {
            let db = storage.connection.lock().unwrap();
            assert_eq!(stored_draft_count(&db, &draft.id), 0);
        }
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            let serialized = r#"{"schema_version":9,"edits":[]}"#;
            db.execute(
                "UPDATE working_drafts SET schema_version=9,payload_json=?2,content_hash=?3 WHERE id=?1",
                params![draft.id, serialized, draft_hash(serialized)],
            )
            .unwrap();
        }
        assert!(storage
            .load_transcript_draft(&meeting.id)
            .unwrap_err()
            .contains("未対応"));
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            1
        );
    }

    #[test]
    fn invalid_draft_is_retained_when_the_current_body_cannot_be_verified() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assign_invalid_reference(&storage, &draft);
        {
            let db = storage.connection.lock().unwrap();
            use rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER;
            db.set_db_config(SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            db.execute(
                "UPDATE analysis_revisions SET content_hash=?1 WHERE id='analysis'",
                ["1".repeat(64)],
            )
            .unwrap();
            db.set_db_config(SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
                .unwrap();
        }
        assert!(storage.load_transcript_draft(&meeting.id).is_err());
        assert!(storage
            .create_transcript_draft(&meeting.id, "analysis")
            .is_err());
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            1
        );
    }

    #[test]
    fn scoped_guard_retains_invalid_draft_while_reads_use_the_verified_body() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集中").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assign_invalid_reference(&storage, &draft);
        storage.reserve_meeting_deletion(&meeting.id).unwrap();
        assert_eq!(storage.load_transcript_draft(&meeting.id).unwrap(), None);
        assert!(storage
            .create_transcript_draft(&meeting.id, "analysis")
            .is_err());
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            1
        );
        storage.release_meeting_deletion(&meeting.id).unwrap();
        assert_eq!(storage.load_transcript_draft(&meeting.id).unwrap(), None);
        assert_eq!(
            stored_draft_count(&storage.connection.lock().unwrap(), &draft.id),
            0
        );
    }

    #[test]
    fn draft_is_retained_when_reading_references_fails() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("編集中").unwrap();
        committed_analysis_with_utterance(&storage, &meeting.id);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis")
            .unwrap();
        assign_invalid_reference(&storage, &draft);
        storage
            .connection
            .lock()
            .unwrap()
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Read {
                    table_name: "speakers",
                    ..
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
        assert!(storage.load_transcript_draft(&meeting.id).is_err());
        let db = storage.connection.lock().unwrap();
        assert_eq!(stored_draft_count(&db, &draft.id), 1);
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
            schema_version: crate::current_formats::TRANSCRIPT_DRAFT,
            speaker_names: vec![],
            utterances: vec![UtteranceEdit {
                utterance_id: "other-utterance".into(),
                effective_text: "本文".into(),
                speaker_ids: vec![],
            }],
            ..Default::default()
        };
        assert!(storage
            .save_transcript_draft(&draft.id, 0, invalid)
            .is_err());
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft)
        );
    }
}

// Shared by ordinary editing and atomic voice-name application.
pub(super) fn save_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    draft_id: &str,
    expected_lock_version: i64,
    payload: TranscriptDraftPayload,
) -> Result<TranscriptDraft, String> {
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
    let existing_payload =
        serde_json::from_str(&old_json).map_err(|_| "保存済み下書きの形式が不正です")?;
    validate_payload(transaction, &base_analysis_id, existing_payload)?;
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
    let now = now_ms()?;
    let changed = transaction.execute(
            "UPDATE working_drafts SET payload_json = ?1, content_hash = ?2, lock_version = lock_version + 1, updated_at = ?3, schema_version = ?6 WHERE id = ?4 AND lock_version = ?5",
            params![serialized, new_hash, now, draft_id, lock_version, payload.schema_version],
        ).map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("下書きの更新競合が発生しました".into());
    }
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
