use super::minutes_input::{
    apply_edits, copy_snapshot, current_working, lineage_map, read_snapshot, working_hash,
};
use super::*;
use rusqlite::{Transaction, TransactionBehavior};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub(crate) struct DiarizationInput {
    pub analysis: AnalysisView,
    pub audio_manifest_hash: String,
    pub parent_id: String,
    pub parent_hash: String,
    pub speaker_lineages: HashMap<String, String>,
    pub utterance_lineages: HashMap<String, String>,
    pub manual_utterances: BTreeSet<String>,
    pub manual_assignment_hash: String,
}

pub(crate) fn prior_speaker_key(id: &str) -> String {
    format!("prior:{id}")
}

impl Storage {
    pub(crate) fn enqueue_diarization_job(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        configure: impl FnOnce(&DiarizationInput) -> Result<(Value, i64), String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_diarization_source(request, profile, None, configure)
    }

    pub(crate) fn enqueue_diarization_source(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        transcription_job: Option<&str>,
        configure: impl FnOnce(&DiarizationInput) -> Result<(Value, i64), String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_diarization_source_version(
            request,
            profile,
            transcription_job,
            crate::current_formats::DIARIZATION,
            configure,
        )
    }

    pub(crate) fn enqueue_diarization_source_version(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        transcription_job: Option<&str>,
        schema_version: u32,
        configure: impl FnOnce(&DiarizationInput) -> Result<(Value, i64), String>,
    ) -> Result<JobSnapshot, String> {
        if request.operation_id.trim().is_empty()
            || request.operation_id.len() > 128
            || request.expected_meeting_lock_version < 0
            || !profile.is_object()
            || schema_version != crate::current_formats::DIARIZATION
        {
            return Err("話者分離の操作・固定入力指定が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        super::deletion::audio::require_source(&tx, &request.meeting_id)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id=?1",
                params![request.operation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = existing {
            let (spec, _) = super::jobs::read_fixed_input(&tx, &id)?;
            let input = read_input(&tx, &spec)?;
            let (parameters, units) = configure(&input)?;
            if spec.meeting_id != request.meeting_id
                || spec.profile_snapshot != profile
                || spec.parameters
                    != json!({"schemaVersion":schema_version,"request":request,"diarization":parameters})
                || spec.total_units != Some(units)
            {
                return Err("同じ操作IDで異なる話者分離が指定されました".into());
            }
            return super::jobs::load_job(&tx, &id)?.ok_or("処理が見つかりません".into());
        }
        let (mut source, draft, title, created, started) = if let Some(job) = transcription_job {
            // Only the exact successful transcription output can be handed on.
            // Reading it never adopts it or activates a draft.
            let job = super::jobs::load_job(&tx, job)?.ok_or("引継ぎ元ジョブがありません")?;
            let (spec, _) = super::jobs::read_fixed_input(&tx, &job.id)?;
            if job.kind != "transcription"
                || job.state != "succeeded"
                || job.meeting_id != request.meeting_id
                || job.output_analysis_id.as_deref() != Some(&request.expected_analysis_id)
                || request.expected_draft.is_some()
            {
                return Err("成功した同じ会議の文字起こしだけを引き継げます".into());
            }
            let (hash,audio,state):(String,String,String) = tx.query_row("SELECT content_hash,audio_manifest_hash,publication_state FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",params![request.expected_analysis_id,request.meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
            if !matches!(state.as_str(), "candidate" | "committed")
                || audio != spec.input_hash
                || hash != request.expected_content_hash
                || super::revisions::analysis_content_hash(&tx, &request.expected_analysis_id)?
                    != hash
            {
                return Err("引継ぎ元の文字起こし内容または音声の由来が一致しません".into());
            }
            let view = super::analysis::read_analysis_content(
                &tx,
                &request.meeting_id,
                &request.expected_analysis_id,
                &hash,
            )?;
            let (title, created, started) = tx
                .query_row(
                    "SELECT title,created_at,started_at_utc_ms FROM meetings WHERE id=?1",
                    params![request.meeting_id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, Option<i64>>(2)?,
                        ))
                    },
                )
                .map_err(|e| e.to_string())?;
            (view, None, title, created, started)
        } else {
            current_working(&tx, &request.meeting_id)?
        };
        let lock: i64 = tx
            .query_row(
                "SELECT lock_version FROM meetings WHERE id=?1",
                params![request.meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let stamp = draft.as_ref().map(|d| DraftStamp {
            id: d.id.clone(),
            lock_version: d.lock_version,
        });
        if source.id != request.expected_analysis_id
            || source.content_hash != request.expected_content_hash
            || lock != request.expected_meeting_lock_version
            || stamp != request.expected_draft
        {
            return Err("話者分離の開始前に現在版・下書き・会議が変化しています。保存後に再確認してください".into());
        }
        let payload = draft
            .as_ref()
            .map(|d| d.payload.clone())
            .unwrap_or_default();
        let lineages = lineage_map(&tx, "utterances", &source.id)?;
        let mut manual_lineages =
            super::speaker_assignment_update::manual_lineages(&tx, &source.id)?;
        let edits: HashMap<_, _> = payload
            .utterances
            .iter()
            .map(|e| (&e.utterance_id, e))
            .collect();
        for utterance in &source.utterances {
            let manual: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM utterance_speakers WHERE utterance_id=?1 AND analysis_revision_id=?2 AND assignment_source='manual') OR EXISTS(SELECT 1 FROM processing_jobs j JOIN diarization_manual_assignments d ON d.analysis_id=j.input_analysis_id JOIN utterances u ON u.id=d.utterance_id AND u.analysis_revision_id=d.analysis_id WHERE j.output_analysis_id=?2 AND j.kind='diarization' AND u.lineage_id=?3)",params![utterance.id,source.id,lineages[&utterance.id]],|r|r.get(0)).map_err(|e|e.to_string())?;
            // A text-only edit of an unknown ASR speaker does not prevent its
            // first inference. Changing an assignment (including to unknown)
            // does, and remains explicit through subsequent reanalyses.
            let changed = edits
                .get(&utterance.id)
                .is_some_and(|e| e.speaker_ids != utterance.speaker_ids)
                || super::drafts::merged_assignments(&payload, &utterance.speaker_ids) != {
                    let mut ids = utterance.speaker_ids.clone();
                    ids.sort();
                    ids
                };
            if manual || changed {
                manual_lineages.insert(lineages[&utterance.id].clone());
            }
        }
        apply_edits(&mut source, &payload);
        for part in payload.utterance_splits.iter().flat_map(|s| &s.parts) {
            // The user supplied both boundaries and assignments, including an
            // explicit unknown. Inference must not undo that decision.
            manual_lineages.insert(part.id.clone());
        }
        let working = working_hash(&tx, &source, &title, created, started, Some(&payload))?;
        let provenance = draft
            .as_ref()
            .map(|d| json!({"id":d.id,"lockVersion":d.lock_version,"contentHash":d.content_hash}));
        let id = copy_snapshot(
            &tx, &source, &payload, &title, created, started, &working, provenance,
        )?;
        let fixed_lineages = lineage_map(&tx, "utterances", &id)?;
        for (utterance, lineage) in &fixed_lineages {
            if manual_lineages.contains(lineage) {
                tx.execute("INSERT INTO diarization_manual_assignments (analysis_id,utterance_id) VALUES (?1,?2)",params![id,utterance]).map_err(|e|e.to_string())?;
            }
        }
        let hash: String = tx
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut spec = JobSpec {
            meeting_id: request.meeting_id.clone(),
            kind: JobKind::Diarization,
            operation_id: request.operation_id.clone(),
            input_analysis_id: Some(id),
            input_hash: hash,
            profile_snapshot: profile,
            parameters: json!({"schemaVersion":schema_version,"request":request}),
            total_units: None,
        };
        let input = read_input(&tx, &spec)?;
        let (parameters, total) = configure(&input)?;
        if !parameters.is_object()
            || parameters["manualAssignmentHash"] != input.manual_assignment_hash
        {
            return Err("話者分離の固定割り当てと条件が一致しません".into());
        }
        spec.parameters["diarization"] = parameters;
        spec.total_units = Some(total);
        if schema_version == crate::current_formats::DIARIZATION {
            let (fixed_source, known) = crate::diarization::known::fixed_coverage(&spec)?;
            let source =
                super::transcription::read_analysis_audio_plan(&tx, &spec.meeting_id, None)?;
            if source != fixed_source
                || super::transcription::known::read(&tx, &spec.meeting_id, &source)? != known
            {
                return Err("開始前に保存音声・欠損・取得時計が変更されています".into());
            }
        }
        let job = super::jobs::enqueue_in_transaction(&tx, &spec)?;
        tx.commit().map_err(|e| e.to_string())?;
        crate::internal_logs::job(&job);
        Ok(job)
    }

    pub(crate) fn load_diarization_input(&self, job_id: &str) -> Result<DiarizationInput, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let (spec, _) = super::jobs::read_fixed_input(&tx, job_id)?;
        read_input(&tx, &spec)
    }
}

pub(super) fn read_input(tx: &Transaction<'_>, spec: &JobSpec) -> Result<DiarizationInput, String> {
    if spec.kind != JobKind::Diarization {
        return Err("話者分離ジョブではありません".into());
    }
    #[cfg(unix)]
    super::deletion::audio::require_source(tx, &spec.meeting_id)?;
    let id = spec
        .input_analysis_id
        .as_deref()
        .ok_or("固定入力版がありません")?;
    let snapshot = read_snapshot(tx, &spec.meeting_id, id, &spec.input_hash)?;
    let request: MinutesJobRequest =
        serde_json::from_value(spec.parameters["request"].clone()).map_err(|e| e.to_string())?;
    let (audio,parent_id,parent_hash): (String,String,String) = tx.query_row("SELECT a.audio_manifest_hash,s.source_analysis_id,s.source_content_hash FROM analysis_revisions a JOIN analysis_input_snapshots s ON s.analysis_id=a.id WHERE a.id=?1 AND a.meeting_id=?2",params![id,spec.meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if request.meeting_id != spec.meeting_id
        || request.expected_analysis_id != parent_id
        || request.expected_content_hash != parent_hash
    {
        return Err("話者分離の入力由来が固定した元版と一致しません".into());
    }
    let utterance_lineages = lineage_map(tx, "utterances", id)?;
    let speaker_lineages = lineage_map(tx, "speakers", id)?;
    let mut query = tx.prepare("SELECT utterance_id FROM diarization_manual_assignments WHERE analysis_id=?1 ORDER BY utterance_id").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let manual_utterances = rows
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|e| e.to_string())?;
    let manual_lineages: BTreeSet<_> = manual_utterances
        .iter()
        .map(|id| {
            utterance_lineages
                .get(id)
                .cloned()
                .ok_or("固定割り当ての発言がありません")
        })
        .collect::<Result<_, _>>()?;
    let manual_assignment_hash = manual_assignment_hash(&manual_lineages)?;
    if spec.parameters.get("diarization").is_some()
        && spec.parameters["diarization"]["manualAssignmentHash"] != manual_assignment_hash
    {
        return Err("話者分離の固定割り当てが変更されています".into());
    }
    Ok(DiarizationInput {
        analysis: snapshot.analysis,
        audio_manifest_hash: audio,
        parent_id,
        parent_hash,
        speaker_lineages,
        utterance_lineages,
        manual_utterances,
        manual_assignment_hash,
    })
}

pub(super) fn manual_assignment_hash(lineages: &BTreeSet<String>) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(lineages).map_err(|e| e.to_string())?)
    ))
}

#[cfg(all(test, unix))]
#[path = "diarization_tests.rs"]
pub(super) mod tests;
