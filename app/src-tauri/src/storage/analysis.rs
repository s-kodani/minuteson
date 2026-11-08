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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnalysisSpeaker {
    pub id: String,
    pub label: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SpeakerIdentification {
    pub similarity_milli: i32,
    pub runner_up_milli: Option<i32>,
    pub needs_review: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnalysisUtterance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_identification: Option<SpeakerIdentification>,
    pub id: String,
    pub track_id: String,
    pub start_us: i64,
    pub end_us: i64,
    pub recognized_text: String,
    pub effective_text: String,
    pub attribution_state: String,
    pub speaker_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recognition_is_source: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnalysisView {
    pub review_ranges: Vec<crate::transcription::ReviewRange>,
    pub id: String,
    pub meeting_id: String,
    pub content_hash: String,
    pub speakers: Vec<AnalysisSpeaker>,
    pub utterances: Vec<AnalysisUtterance>,
}

// UI metadata stays outside AnalysisView: its serialization participates in
// existing voice-job working hashes and must not change for a color fix.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisPresentation {
    #[serde(flatten)]
    pub analysis: AnalysisView,
    pub speaker_color_keys: HashMap<String, String>,
}

pub(super) fn read_analysis_content(
    connection: &Connection,
    meeting_id: &str,
    id: &str,
    content_hash: &str,
) -> Result<AnalysisView, String> {
    let mut view = read_stored_analysis_content(connection, meeting_id, id, content_hash)?;
    // Playback uses the current source; immutable snapshots retain their original
    // track IDs so retiring audio cannot change a saved working-content hash.
    let canonical: Option<String> = connection
        .query_row(
            "SELECT track_id FROM meeting_audio_commits WHERE meeting_id=?1",
            [meeting_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(track) = canonical {
        for utterance in &mut view.utterances {
            utterance.track_id = track.clone();
        }
    }
    Ok(view)
}

pub(super) fn read_stored_analysis_content(
    connection: &Connection,
    meeting_id: &str,
    id: &str,
    content_hash: &str,
) -> Result<AnalysisView, String> {
    let speakers = {
        let mut statement = connection.prepare(
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
    let config: String = connection
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let config: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    let source_lineages: HashSet<String> =
        if let Some(value) = config.get("sourceRecognitionLineages") {
            serde_json::from_value(value.clone())
                .map_err(|e| format!("分割元の由来情報が不正です: {e}"))?
        } else {
            HashSet::new()
        };
    let mut utterances = {
        let mut statement = connection.prepare(
                "SELECT id, track_id, start_us, end_us, recognized_text, effective_text, attribution_state,lineage_id FROM utterances WHERE analysis_revision_id = ?1 ORDER BY sequence, id",
            ).map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok(AnalysisUtterance {
                    speaker_identification: None,
                    id: row.get(0)?,
                    track_id: row.get(1)?,
                    start_us: row.get(2)?,
                    end_us: row.get(3)?,
                    recognized_text: row.get(4)?,
                    effective_text: row.get(5)?,
                    attribution_state: row.get(6)?,
                    speaker_ids: Vec::new(),
                    recognition_is_source: source_lineages.contains(&row.get::<_, String>(7)?),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let assignment_map: HashMap<String, Vec<String>> = {
        let mut statement = connection.prepare(
                "SELECT a.utterance_id, a.speaker_id FROM utterances u CROSS JOIN utterance_speakers a ON a.utterance_id=u.id AND a.analysis_revision_id=u.analysis_revision_id WHERE u.analysis_revision_id=?1 ORDER BY u.id, CASE a.role WHEN 'primary' THEN 0 ELSE 1 END, a.speaker_id",
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
    let has_identifications = config["speakerIdentification"].is_object();
    let lineage_map = if has_identifications {
        super::minutes_input::lineage_map(connection, "utterances", id)?
    } else {
        HashMap::new()
    };
    let speaker_lineages = if has_identifications {
        super::minutes_input::lineage_map(connection, "speakers", id)?
    } else {
        HashMap::new()
    };
    for utterance in &mut utterances {
        utterance.speaker_ids = assignment_map
            .get(&utterance.id)
            .cloned()
            .unwrap_or_default();
        if has_identifications && utterance.speaker_ids.len() == 1 {
            let lineage = lineage_map.get(&utterance.id).unwrap_or(&utterance.id);
            let info = &config["speakerIdentification"][lineage];
            let speaker = &utterance.speaker_ids[0];
            let owner = speaker_lineages.get(speaker).unwrap_or(speaker);
            if info["speakerLineage"].as_str() == Some(owner.as_str())
                && info["trackId"].as_str() == Some(utterance.track_id.as_str())
                && info["startUs"].as_i64() == Some(utterance.start_us)
                && info["endUs"].as_i64() == Some(utterance.end_us)
            {
                if let (Some(score), Some(review), Some(reason)) = (
                    info["similarity"].as_f64(),
                    info["needsReview"].as_bool(),
                    info["reason"].as_str(),
                ) {
                    let next = info["runnerUp"].as_f64();
                    if score.is_finite()
                        && (-1.0..=1.0).contains(&score)
                        && next.is_none_or(|v| v.is_finite() && (-1.0..=score).contains(&v))
                    {
                        utterance.speaker_identification = Some(SpeakerIdentification {
                            similarity_milli: (score * 1000.).round() as i32,
                            runner_up_milli: next.map(|v| (v * 1000.).round() as i32),
                            needs_review: review,
                            reason: reason.into(),
                        });
                    }
                }
            }
        }
    }
    let review_ranges: Vec<crate::transcription::ReviewRange> = serde_json::from_value(
        config
            .get("reviewRanges")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|e| e.to_string())?;
    if review_ranges.len() > 100_000
        || review_ranges.iter().any(|r| {
            r.start_us < 0 || r.end_us <= r.start_us || r.reason != "repetition_after_recovery"
        })
    {
        return Err("文字起こしの要確認範囲が不正です".into());
    }
    Ok(AnalysisView {
        review_ranges,
        id: id.to_owned(),
        meeting_id: meeting_id.to_owned(),
        content_hash: content_hash.to_owned(),
        speakers,
        utterances,
    })
}

impl Storage {
    pub fn load_current_analysis(&self, meeting_id: &str) -> Result<Option<AnalysisView>, String> {
        self.load_analysis_view(meeting_id, None, |_, view| Ok(view))
    }

    pub(crate) fn load_current_analysis_presentation(
        &self,
        meeting_id: &str,
    ) -> Result<Option<AnalysisPresentation>, String> {
        self.load_analysis_view(meeting_id, None, |db, analysis| {
            let speaker_color_keys =
                super::minutes_input::lineage_map(db, "speakers", &analysis.id)?;
            Ok(AnalysisPresentation {
                analysis,
                speaker_color_keys,
            })
        })
    }

    pub(crate) fn load_visible_analysis(
        &self,
        meeting_id: &str,
        revision_id: &str,
    ) -> Result<Option<AnalysisView>, String> {
        self.load_analysis_view(meeting_id, Some(revision_id), |_, view| Ok(view))
    }

    fn load_analysis_view<T>(
        &self,
        meeting_id: &str,
        revision_id: Option<&str>,
        present: impl FnOnce(&Connection, AnalysisView) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let head: Option<(String, String)> = transaction.query_row(
            "SELECT a.id, a.content_hash FROM meetings m JOIN analysis_revisions a ON a.meeting_id = m.id WHERE m.id = ?1 AND ((?2 IS NULL AND a.id = m.current_analysis_id AND a.publication_state = 'committed') OR (?2 = a.id AND (a.publication_state = 'candidate' OR (a.publication_state = 'committed' AND a.id = m.current_analysis_id))))",
            params![meeting_id, revision_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((id, content_hash)) = head else {
            return Ok(None);
        };
        if super::revisions::analysis_content_hash(&transaction, &id)? != content_hash {
            return Err("解析版の内容が変更されています".into());
        }
        let analysis = read_analysis_content(&transaction, meeting_id, &id, &content_hash)?;
        present(&transaction, analysis).map(Some)
    }

    // Called by a local analysis worker after its output has been parsed. A
    // candidate does not replace the user's current analysis or draft.
    #[allow(dead_code)]
    pub fn create_analysis_candidate(
        &self,
        meeting_id: &str,
        input: AnalysisInput,
    ) -> Result<String, String> {
        self.create_analysis_candidate_inner(meeting_id, input, None)
    }

    pub(crate) fn finish_transcription_candidate(
        &self,
        meeting_id: &str,
        job_id: &str,
        completed_units: i64,
        input: AnalysisInput,
    ) -> Result<String, String> {
        self.create_analysis_candidate_inner(meeting_id, input, Some((job_id, completed_units)))
    }

    pub(crate) fn finish_diarization_candidate(
        &self,
        meeting: &str,
        job: &str,
        completed: i64,
        input: AnalysisInput,
    ) -> Result<String, String> {
        self.create_analysis_candidate_inner(meeting, input, Some((job, completed)))
    }

    fn create_analysis_candidate_inner(
        &self,
        meeting_id: &str,
        input: AnalysisInput,
        job: Option<(&str, i64)>,
    ) -> Result<String, String> {
        if !valid_hash(&input.audio_manifest_hash)
            || !input.profile_snapshot.is_object()
            || !input.config.is_object()
            || input.utterances.len() > 100_000
            || input.speakers.len() > 100
            || input.segments.len() > 500_000
        {
            return Err("解析候補の入力形式または件数が不正です".into());
        }
        let profile = serde_json::to_string(&input.profile_snapshot).map_err(|e| e.to_string())?;
        let mut config = serde_json::to_string(&input.config).map_err(|e| e.to_string())?;
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
        let mut fixed_source = None;
        let mut fixed_known = None;
        if let Some((job_id, completed_units)) = job {
            let (spec, _) = super::jobs::read_fixed_input(&transaction, job_id)?;
            if spec.kind == JobKind::Diarization {
                if spec.parameters["schemaVersion"] == crate::current_formats::DIARIZATION {
                    let known: super::transcription::known::KnownInput = serde_json::from_value(
                        spec.parameters["diarization"]["knownInput"].clone(),
                    )
                    .map_err(|e| e.to_string())?;
                    let source = super::transcription::read_audio_plan(&transaction, meeting_id)?;
                    known.validate(&source)?;
                    if spec.parameters["diarization"]["source"]
                        != serde_json::to_value(&source).map_err(|e| e.to_string())?
                        || known
                            != super::transcription::known::read(&transaction, meeting_id, &source)?
                        || input.config["audioCoverage"]
                            != serde_json::json!({"mode":"known_runs","runCount":known.runs.len(),"missingRangeCount":known.gaps.iter().filter(|gap|!gap.recovered).count(),"timeBase":"meeting"})
                    {
                        return Err("話者候補公開前に保存区間・欠損が変更されています".into());
                    }
                    for segment in &input.segments {
                        if !known.runs.iter().any(|run| {
                            source.tracks[run.track_index].id == segment.track_id
                                && segment.start_us >= run.start_us
                                && segment.end_us <= run.end_us
                        }) {
                            return Err("話者候補が固定した保存区間の外側へ出ています".into());
                        }
                    }
                }
                let source = super::diarization::read_input(&transaction, &spec)?;
                if input.audio_manifest_hash != source.audio_manifest_hash
                    || input.config["jobId"] != job_id
                    || input.config["inputAnalysisId"] != source.analysis.id
                    || input.config["inputContentHash"] != spec.input_hash
                    || input.utterances.len() != source.analysis.utterances.len()
                {
                    return Err("話者分離の候補と固定入力が一致しません".into());
                }
                for (utterance, original) in
                    input.utterances.iter().zip(&source.analysis.utterances)
                {
                    if utterance.track_id != original.track_id
                        || utterance.start_us != original.start_us
                        || utterance.end_us != original.end_us
                        || utterance.text != original.recognized_text
                    {
                        return Err("話者分離は元の認識本文・時刻を変更できません".into());
                    }
                    if source.manual_utterances.contains(&original.id)
                        && utterance.speaker_keys
                            != original
                                .speaker_ids
                                .iter()
                                .map(|id| super::diarization::prior_speaker_key(id))
                                .collect::<Vec<_>>()
                    {
                        return Err("手動の話者割り当ては保持する必要があります".into());
                    }
                }
                for original in source.analysis.speakers.iter().filter(|speaker| {
                    source.analysis.utterances.iter().any(|u| {
                        source.manual_utterances.contains(&u.id)
                            && u.speaker_ids.contains(&speaker.id)
                    })
                }) {
                    if !input.speakers.iter().any(|s| {
                        s.key == super::diarization::prior_speaker_key(&original.id)
                            && s.label == original.label
                    }) {
                        return Err("既存の話者名・ラベルを保持する必要があります".into());
                    }
                }
                fixed_source = Some(source);
            } else if spec.kind != JobKind::Transcription {
                return Err("未対応の解析候補ジョブです".into());
            } else if spec.parameters["schemaVersion"] == crate::current_formats::TRANSCRIPTION {
                let known: super::transcription::known::KnownInput =
                    serde_json::from_value(spec.parameters["knownInput"].clone())
                        .map_err(|e| e.to_string())?;
                let source = super::transcription::read_audio_plan(&transaction, meeting_id)?;
                if spec.parameters["source"]
                    != serde_json::to_value(&source).map_err(|e| e.to_string())?
                    || known
                        != super::transcription::known::read(&transaction, meeting_id, &source)?
                {
                    return Err("公開直前に固定した保存音声・欠損が変更されています".into());
                }
                known.validate(&source)?;
                if input.config["audioCoverage"]
                    != serde_json::json!({"mode":"known_runs","runCount":known.runs.len(),"missingRangeCount":known.gaps.iter().filter(|gap|!gap.recovered).count(),"timeBase":"meeting"})
                {
                    return Err("保存区間別の候補と固定した音声範囲が一致しません".into());
                }
                fixed_known = Some((source, known));
            }
            let valid: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE id = ?1 AND meeting_id = ?2 AND kind IN ('transcription','diarization') AND state = 'running' AND completed_units = ?3 AND total_units = ?3 AND input_hash = ?4 AND profile_snapshot_json = ?5 AND output_analysis_id IS NULL)",
                params![job_id, meeting_id, completed_units, if fixed_source.is_some() { &spec.input_hash } else { &input.audio_manifest_hash }, profile], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if !valid {
                return Err("解析ジョブの状態・入力・完了位置が一致しません".into());
            }
            super::jobs::verify_latest_checkpoint(&transaction, job_id)?;
        }
        let source_type: Option<String> = transaction
            .query_row(
                "SELECT source_type FROM meetings WHERE id=?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let source: Option<(String, String)> = match source_type.as_deref() {
            Some("recording") => transaction.query_row(
                "SELECT m.lifecycle,r.audio_manifest_hash FROM meetings m JOIN recording_sessions r ON r.meeting_id=m.id WHERE m.id=?1 AND r.state='saved'",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(|e| e.to_string())?,
            Some("audio_import") => transaction.query_row(
                "SELECT m.lifecycle,p.manifest_hash FROM meetings m JOIN audio_imports i ON i.meeting_id=m.id JOIN audio_import_commits p ON p.import_id=i.id AND p.meeting_id=m.id WHERE m.id=?1 AND i.state='ready'",
                params![meeting_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(|e| e.to_string())?,
            _ => None,
        };
        if source.as_ref().is_none_or(|(lifecycle, hash)| {
            !matches!(lifecycle.as_str(), "recorded" | "imported")
                || hash != &input.audio_manifest_hash
        }) {
            return Err("確定済み音声と解析入力が一致しません".into());
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
            if let Some((source, known)) = &fixed_known {
                if !known.runs.iter().any(|run| {
                    source.tracks[run.track_index].id == utterance.track_id
                        && utterance.start_us >= run.start_us
                        && utterance.end_us <= run.end_us
                }) {
                    return Err("文字起こし候補が固定した保存区間の外側へ出ています".into());
                }
            }
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
        let (mut parent_id, mut parent_hash): (Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT a.id, a.content_hash FROM meetings m LEFT JOIN analysis_revisions a ON a.id = m.current_analysis_id WHERE m.id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if let Some(source) = &fixed_source {
            parent_id = Some(source.parent_id.clone());
            parent_hash = Some(source.parent_hash.clone());
        }
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
        if let Some(source) = &fixed_source {
            let mut stored = input.config.clone();
            let original: String = transaction
                .query_row(
                    "SELECT config_json FROM analysis_revisions WHERE id=?1",
                    params![source.analysis.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let original: Value = serde_json::from_str(&original).map_err(|e| e.to_string())?;
            for key in [
                "manualAssignmentLineages",
                "sourceRecognitionLineages",
                "reviewRanges",
                "completionStatus",
            ] {
                if let Some(value) = original.get(key) {
                    stored[key] = value.clone();
                }
            }
            config = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
            if config.len() > 2 * 1024 * 1024 {
                return Err("手動分割の由来情報が大きすぎます".into());
            }
        }
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, parent_revision_id, revision_no, parent_content_hash, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'engine', 'candidate', ?6, ?7, ?8, ?9, 1, ?10)",
            params![id, meeting_id, parent_id, revision_no, parent_hash, input.audio_manifest_hash, profile, config, "0".repeat(64), now],
        ).map_err(|e| e.to_string())?;
        let mut speaker_ids = HashMap::new();
        for speaker in &input.speakers {
            let speaker_id = Uuid::new_v4().to_string();
            let prior = fixed_source.as_ref().and_then(|source| {
                source
                    .analysis
                    .speakers
                    .iter()
                    .find(|s| speaker.key == super::diarization::prior_speaker_key(&s.id))
            });
            let lineage = if let (Some(source), Some(prior)) = (&fixed_source, prior) {
                &source.speaker_lineages[&prior.id]
            } else {
                &speaker_id
            };
            let name = prior.and_then(|s| s.display_name.as_deref());
            let (scope, identification) = if let Some(prior) = prior {
                transaction
                    .query_row(
                        "SELECT input_scope, identification_state FROM speakers WHERE id=?1 AND analysis_revision_id=?2",
                        params![prior.id, fixed_source.as_ref().unwrap().analysis.id],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )
                    .map_err(|e| e.to_string())?
            } else if fixed_source.is_some() {
                (
                    speaker.key.split(':').next().unwrap_or("all").into(),
                    "unknown".into(),
                )
            } else {
                ("all".into(), "unknown".into())
            };
            transaction.execute(
                "INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, display_name, input_scope, identification_state, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![speaker_id, id, lineage, speaker.label, name, scope, identification, now],
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
            let original = fixed_source
                .as_ref()
                .map(|s| &s.analysis.utterances[sequence]);
            let lineage = if let (Some(source), Some(original)) = (&fixed_source, original) {
                &source.utterance_lineages[&original.id]
            } else {
                &utterance_id
            };
            let effective = original.map_or(utterance.text.trim(), |u| u.effective_text.as_str());
            let recognized = original.map_or(utterance.text.trim(), |u| u.recognized_text.as_str());
            // Diarization does not replace the ASR recognition confidence.
            // Its own speaker intervals carry only native values (or NULL).
            let confidence =
                if let Some(original) = original {
                    transaction.query_row(
                    "SELECT confidence FROM utterances WHERE id=?1 AND analysis_revision_id=?2",
                    params![original.id, fixed_source.as_ref().unwrap().analysis.id],
                    |r| r.get::<_, Option<f64>>(0),
                ).map_err(|e|e.to_string())?
                } else {
                    utterance.confidence
                };
            let assignment_source = if original.is_some_and(|u| {
                fixed_source
                    .as_ref()
                    .unwrap()
                    .manual_utterances
                    .contains(&u.id)
            }) {
                "manual"
            } else {
                "inferred"
            };
            let attribution = match utterance.speaker_keys.len() {
                0 => "unknown",
                1 => "assigned",
                _ => "overlap",
            };
            transaction.execute(
                "INSERT INTO utterances (id, meeting_id, analysis_revision_id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state, confidence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![utterance_id, meeting_id, id, utterance.track_id, lineage, sequence as i64, utterance.start_us, utterance.end_us, recognized, effective, attribution, confidence],
            ).map_err(|e| e.to_string())?;
            for (index, key) in utterance.speaker_keys.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![utterance_id, speaker_ids[key], id, if index == 0 { "primary" } else { "overlap" }, assignment_source],
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
        if let Some((job_id, _)) = job {
            transaction.execute("UPDATE processing_jobs SET state = 'succeeded', output_analysis_id = ?1, finished_at = ?2, updated_at = ?2 WHERE id = ?3", params![id, now, job_id]).map_err(|e| e.to_string())?;
        }
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
pub(crate) mod tests {
    use super::*;

    pub(crate) fn saved_meeting(storage: &Storage) -> Meeting {
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
            "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'processed', 'wav', 16000, 1, 'ready', 0, 0)",
            params![meeting.id],
        )
        .unwrap();
        db.execute(
            "INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES ('asset', ?1, 'meeting', 'meetings/test/audio.caf', 'audio/wav', 32044, 'sha256', ?2, 'ready', 0, 0)",
            params![meeting.id, "b".repeat(64)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES ('chunk', ?1, 'track', 'asset', 0, 0, 1000000, 16000, 16000, 1, 'ready', 0)",
            params![meeting.id],
        )
        .unwrap();
        db.execute("INSERT INTO file_assets (id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) VALUES ('manifest',?1,'meeting','meetings/test/manifest.json','application/json',16,'sha256',?2,'ready',0,0)",params![meeting.id,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO meeting_audio_commits VALUES(?1,'track','manifest',?2,16000,'{}','[]','cleaned',0)",params![meeting.id,"a".repeat(64)]).unwrap();
        meeting
    }

    pub(crate) fn input() -> AnalysisInput {
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
