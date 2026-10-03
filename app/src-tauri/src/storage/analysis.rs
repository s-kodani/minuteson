use super::*;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SpeakerInput {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SpeakerSegmentInput {
    pub track_id: String,
    pub speaker_key: Option<String>,
    pub start_us: i64,
    pub end_us: i64,
    pub overlap_group: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UtteranceInput {
    pub track_id: String,
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
    pub speaker_keys: Vec<String>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnalysisInput {
    pub audio_manifest_hash: String,
    pub profile_snapshot: Value,
    pub config: Value,
    pub speakers: Vec<SpeakerInput>,
    pub segments: Vec<SpeakerSegmentInput>,
    pub utterances: Vec<UtteranceInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSpeaker {
    pub id: String,
    pub label: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUtterance {
    pub id: String,
    pub track_id: String,
    pub start_us: i64,
    pub end_us: i64,
    pub recognized_text: String,
    pub effective_text: String,
    pub attribution_state: String,
    pub speaker_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisView {
    pub id: String,
    pub meeting_id: String,
    pub content_hash: String,
    pub speakers: Vec<AnalysisSpeaker>,
    pub utterances: Vec<AnalysisUtterance>,
}

impl Storage {
    pub fn load_current_analysis(&self, meeting_id: &str) -> Result<Option<AnalysisView>, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let head: Option<(String, String)> = transaction.query_row(
            "SELECT a.id, a.content_hash FROM meetings m JOIN analysis_revisions a ON a.id = m.current_analysis_id AND a.meeting_id = m.id AND a.publication_state = 'committed' WHERE m.id = ?1",
            params![meeting_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((id, content_hash)) = head else {
            return Ok(None);
        };
        if super::revisions::analysis_content_hash(&transaction, &id)? != content_hash {
            return Err("解析版の内容が変更されています".into());
        }
        let speakers = {
            let mut statement = transaction.prepare(
                "SELECT id, label, display_name FROM speakers WHERE analysis_revision_id = ?1 ORDER BY label, id",
            ).map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![id], |row| {
                    Ok(AnalysisSpeaker {
                        id: row.get(0)?,
                        label: row.get(1)?,
                        display_name: row.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut utterances = {
            let mut statement = transaction.prepare(
                "SELECT id, track_id, start_us, end_us, recognized_text, effective_text, attribution_state FROM utterances WHERE analysis_revision_id = ?1 ORDER BY sequence, id",
            ).map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![id], |row| {
                    Ok(AnalysisUtterance {
                        id: row.get(0)?,
                        track_id: row.get(1)?,
                        start_us: row.get(2)?,
                        end_us: row.get(3)?,
                        recognized_text: row.get(4)?,
                        effective_text: row.get(5)?,
                        attribution_state: row.get(6)?,
                        speaker_ids: Vec::new(),
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let assignment_map: HashMap<String, Vec<String>> = {
            let mut statement = transaction.prepare(
                "SELECT utterance_id, speaker_id FROM utterance_speakers WHERE analysis_revision_id = ?1 ORDER BY utterance_id, CASE role WHEN 'primary' THEN 0 ELSE 1 END, speaker_id",
            ).map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            let mut assignments: HashMap<String, Vec<String>> = HashMap::new();
            for row in rows {
                let (utterance_id, speaker_id) = row.map_err(|e| e.to_string())?;
                assignments
                    .entry(utterance_id)
                    .or_default()
                    .push(speaker_id);
            }
            assignments
        };
        for utterance in &mut utterances {
            utterance.speaker_ids = assignment_map
                .get(&utterance.id)
                .cloned()
                .unwrap_or_default();
        }
        Ok(Some(AnalysisView {
            id,
            meeting_id: meeting_id.to_owned(),
            content_hash,
            speakers,
            utterances,
        }))
    }

    // Called by a local analysis worker after its output has been parsed. A
    // candidate does not replace the user's current analysis or draft.
    #[allow(dead_code)]
    pub fn create_analysis_candidate(
        &self,
        meeting_id: &str,
        input: AnalysisInput,
    ) -> Result<String, String> {
        if !valid_hash(&input.audio_manifest_hash)
            || !input.profile_snapshot.is_object()
            || !input.config.is_object()
            || input.utterances.is_empty()
            || input.utterances.len() > 100_000
            || input.speakers.len() > 100
            || input.segments.len() > 500_000
        {
            return Err("解析候補の入力形式または件数が不正です".into());
        }
        let profile = serde_json::to_string(&input.profile_snapshot).map_err(|e| e.to_string())?;
        let config = serde_json::to_string(&input.config).map_err(|e| e.to_string())?;
        if profile.len() > 65_536 || config.len() > 65_536 {
            return Err("解析設定が大きすぎます".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let source: Option<(String, String)> = transaction
            .query_row(
                "SELECT m.lifecycle, r.audio_manifest_hash FROM meetings m JOIN recording_sessions r ON r.meeting_id = m.id WHERE m.id = ?1 AND r.state = 'saved'",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if source.as_ref().is_none_or(|(lifecycle, hash)| {
            lifecycle != "recorded" || hash != &input.audio_manifest_hash
        }) {
            return Err("確定済み録音と解析入力が一致しません".into());
        }
        let tracks = {
            let mut statement = transaction
                .prepare("SELECT t.id, MAX(c.end_us) FROM audio_tracks t JOIN audio_chunks c ON c.track_id = t.id WHERE t.meeting_id = ?1 AND t.state = 'ready' AND c.state = 'ready' GROUP BY t.id")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![meeting_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<HashMap<_, _>, _>>()
                .map_err(|e| e.to_string())?
        };
        if tracks.is_empty() {
            return Err("解析可能な音声トラックがありません".into());
        }
        let mut speaker_keys = HashSet::new();
        for speaker in &input.speakers {
            if speaker.key.trim().is_empty()
                || speaker.key.len() > 128
                || speaker.label.trim().is_empty()
                || speaker.label.chars().count() > 200
                || !speaker_keys.insert(speaker.key.as_str())
            {
                return Err("話者の識別子または表示名が不正です".into());
            }
        }
        for segment in &input.segments {
            validate_span(&tracks, &segment.track_id, segment.start_us, segment.end_us)?;
            validate_confidence(segment.confidence)?;
            if segment
                .speaker_key
                .as_ref()
                .is_some_and(|key| !speaker_keys.contains(key.as_str()))
                || segment
                    .overlap_group
                    .as_ref()
                    .is_some_and(|key| key.len() > 128)
            {
                return Err("話者区間の参照が不正です".into());
            }
        }
        let mut last_start = -1;
        let mut total_text_bytes = 0_usize;
        for utterance in &input.utterances {
            validate_span(
                &tracks,
                &utterance.track_id,
                utterance.start_us,
                utterance.end_us,
            )?;
            validate_confidence(utterance.confidence)?;
            if utterance.start_us < last_start
                || utterance.text.trim().is_empty()
                || utterance.text.chars().count() > 20_000
                || utterance.speaker_keys.len() > 100
            {
                return Err("発言の時刻または本文が不正です".into());
            }
            total_text_bytes = total_text_bytes.saturating_add(utterance.text.len());
            if total_text_bytes > 64 * 1024 * 1024 {
                return Err("解析本文が大きすぎます".into());
            }
            last_start = utterance.start_us;
            let mut assigned = HashSet::new();
            if utterance
                .speaker_keys
                .iter()
                .any(|key| !speaker_keys.contains(key.as_str()) || !assigned.insert(key))
            {
                return Err("発言の話者割り当てが不正です".into());
            }
        }
        let (parent_id, parent_hash): (Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT a.id, a.content_hash FROM meetings m LEFT JOIN analysis_revisions a ON a.id = m.current_analysis_id WHERE m.id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if let (Some(parent_id), Some(parent_hash)) = (&parent_id, &parent_hash) {
            if super::revisions::analysis_content_hash(&transaction, parent_id)? != *parent_hash {
                return Err("現在の解析版の内容が変化しています".into());
            }
        }
        let revision_no: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(revision_no), 0) + 1 FROM analysis_revisions WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, parent_revision_id, revision_no, parent_content_hash, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'engine', 'candidate', ?6, ?7, ?8, ?9, 1, ?10)",
            params![id, meeting_id, parent_id, revision_no, parent_hash, input.audio_manifest_hash, profile, config, "0".repeat(64), now],
        ).map_err(|e| e.to_string())?;
        let mut speaker_ids = HashMap::new();
        for speaker in &input.speakers {
            let speaker_id = Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES (?1, ?2, ?1, ?3, 'all', 'unknown', ?4)",
                params![speaker_id, id, speaker.label, now],
            ).map_err(|e| e.to_string())?;
            speaker_ids.insert(&speaker.key, speaker_id);
        }
        for segment in &input.segments {
            let speaker_id = segment.speaker_key.as_ref().map(|key| &speaker_ids[key]);
            transaction.execute(
                "INSERT INTO speaker_segments (id, meeting_id, analysis_revision_id, track_id, speaker_id, start_us, end_us, overlap_group_id, confidence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![Uuid::new_v4().to_string(), meeting_id, id, segment.track_id, speaker_id, segment.start_us, segment.end_us, segment.overlap_group, segment.confidence],
            ).map_err(|e| e.to_string())?;
        }
        for (sequence, utterance) in input.utterances.iter().enumerate() {
            let utterance_id = Uuid::new_v4().to_string();
            let attribution = match utterance.speaker_keys.len() {
                0 => "unknown",
                1 => "assigned",
                _ => "overlap",
            };
            transaction.execute(
                "INSERT INTO utterances (id, meeting_id, analysis_revision_id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state, confidence) VALUES (?1, ?2, ?3, ?4, ?1, ?5, ?6, ?7, ?8, ?8, ?9, ?10)",
                params![utterance_id, meeting_id, id, utterance.track_id, sequence as i64, utterance.start_us, utterance.end_us, utterance.text.trim(), attribution, utterance.confidence],
            ).map_err(|e| e.to_string())?;
            for (index, key) in utterance.speaker_keys.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES (?1, ?2, ?3, ?4, 'inferred')",
                    params![utterance_id, speaker_ids[key], id, if index == 0 { "primary" } else { "overlap" }],
                ).map_err(|e| e.to_string())?;
            }
        }
        let content_hash = super::revisions::analysis_content_hash(&transaction, &id)?;
        transaction
            .execute(
                "UPDATE analysis_revisions SET content_hash = ?1 WHERE id = ?2",
                params![content_hash, id],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }
}

fn validate_span(
    tracks: &HashMap<String, i64>,
    track_id: &str,
    start: i64,
    end: i64,
) -> Result<(), String> {
    if start < 0 || end <= start || tracks.get(track_id).is_none_or(|duration| end > *duration) {
        return Err("解析区間が音声トラックの範囲外です".into());
    }
    Ok(())
}

fn validate_confidence(confidence: Option<f64>) -> Result<(), String> {
    if confidence.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        return Err("信頼度の値が不正です".into());
    }
    Ok(())
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_meeting(storage: &Storage) -> Meeting {
        let meeting = storage.create_meeting("解析候補").unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type = 'recording', lifecycle = 'recorded', duration_us = 1000000 WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, audio_manifest_hash, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', ?2, 0, 0)",
            params![meeting.id, "a".repeat(64)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'caf', 48000, 1, 'ready', 0, 0)",
            params![meeting.id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES ('asset', ?1, 'meeting', 'meetings/test/audio.caf', 'audio/x-caf', 16, 'sha256', ?2, 'ready', 0, 0)",
            params![meeting.id, "b".repeat(64)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES ('chunk', ?1, 'track', 'asset', 0, 0, 1000000, 48000, 48000, 1, 'ready', 0)",
            params![meeting.id],
        )
        .unwrap();
        meeting
    }

    fn input() -> AnalysisInput {
        AnalysisInput {
            audio_manifest_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({"engine": "test", "modelHash": "c".repeat(64)}),
            config: serde_json::json!({"speakerCount": 2}),
            speakers: vec![
                SpeakerInput {
                    key: "a".into(),
                    label: "話者A".into(),
                },
                SpeakerInput {
                    key: "b".into(),
                    label: "話者B".into(),
                },
            ],
            segments: vec![
                SpeakerSegmentInput {
                    track_id: "track".into(),
                    speaker_key: Some("a".into()),
                    start_us: 0,
                    end_us: 700_000,
                    overlap_group: Some("overlap-1".into()),
                    confidence: None,
                },
                SpeakerSegmentInput {
                    track_id: "track".into(),
                    speaker_key: Some("b".into()),
                    start_us: 500_000,
                    end_us: 900_000,
                    overlap_group: Some("overlap-1".into()),
                    confidence: Some(0.8),
                },
            ],
            utterances: vec![UtteranceInput {
                track_id: "track".into(),
                start_us: 100_000,
                end_us: 800_000,
                text: " 発言です ".into(),
                speaker_keys: vec!["a".into(), "b".into()],
                confidence: None,
            }],
        }
    }

    #[test]
    fn candidate_keeps_overlap_and_draft_when_reanalysis_is_created() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = saved_meeting(&storage);
        let first = storage
            .create_analysis_candidate(&meeting.id, input())
            .unwrap();
        assert_eq!(
            storage.get_meeting(&meeting.id).unwrap().unwrap().lifecycle,
            "recorded"
        );
        let committed = storage
            .commit_analysis_candidate(&meeting.id, &first, meeting.lock_version, false)
            .unwrap();
        assert!(storage.verify_analysis_content_hash(&first).unwrap());
        let view = storage.load_current_analysis(&meeting.id).unwrap().unwrap();
        assert_eq!(view.id, first);
        assert_eq!(view.speakers.len(), 2);
        assert_eq!(view.utterances.len(), 1);
        assert_eq!(view.utterances[0].speaker_ids.len(), 2);
        let draft = storage
            .create_transcript_draft(&meeting.id, &first)
            .unwrap();
        let second = storage
            .create_analysis_candidate(&meeting.id, input())
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            storage.load_current_analysis(&meeting.id).unwrap().unwrap(),
            view
        );
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap().unwrap(),
            draft
        );
        let db = storage.connection.lock().unwrap();
        let (parent, parent_hash): (String, String) = db.query_row(
            "SELECT parent_revision_id, parent_content_hash FROM analysis_revisions WHERE id = ?1",
            params![second],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!((parent, parent_hash), (first, committed.content_hash));
        let (attribution, recognized): (String, String) = db.query_row(
            "SELECT attribution_state, recognized_text FROM utterances WHERE analysis_revision_id = ?1",
            params![second],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(
            (attribution.as_str(), recognized.as_str()),
            ("overlap", "発言です")
        );
        let assignments: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM utterance_speakers WHERE analysis_revision_id = ?1",
                params![second],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(assignments, 2);
    }

    #[test]
    fn read_rejects_changed_committed_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = saved_meeting(&storage);
        assert!(storage
            .load_current_analysis(&meeting.id)
            .unwrap()
            .is_none());
        let candidate = storage
            .create_analysis_candidate(&meeting.id, input())
            .unwrap();
        storage
            .commit_analysis_candidate(&meeting.id, &candidate, meeting.lock_version, false)
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            db.execute_batch("DROP TRIGGER utterances_frozen_update;")
                .unwrap();
            db.execute(
                "UPDATE utterances SET effective_text = '書き換え' WHERE analysis_revision_id = ?1",
                params![candidate],
            )
            .unwrap();
        }
        assert!(storage.load_current_analysis(&meeting.id).is_err());
    }

    #[test]
    fn invalid_manifest_track_time_and_speaker_leave_no_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = saved_meeting(&storage);
        let mut wrong_hash = input();
        wrong_hash.audio_manifest_hash = "f".repeat(64);
        assert!(storage
            .create_analysis_candidate(&meeting.id, wrong_hash)
            .is_err());
        let mut wrong_track = input();
        wrong_track.utterances[0].track_id = "other".into();
        assert!(storage
            .create_analysis_candidate(&meeting.id, wrong_track)
            .is_err());
        let mut wrong_time = input();
        wrong_time.utterances[0].end_us = 1_000_001;
        assert!(storage
            .create_analysis_candidate(&meeting.id, wrong_time)
            .is_err());
        let mut wrong_speaker = input();
        wrong_speaker.utterances[0]
            .speaker_keys
            .push("other".into());
        assert!(storage
            .create_analysis_candidate(&meeting.id, wrong_speaker)
            .is_err());
        let count: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM analysis_revisions WHERE meeting_id = ?1",
                params![meeting.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
