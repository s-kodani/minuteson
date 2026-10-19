use super::analysis::{read_analysis_content, AnalysisSpeaker, AnalysisUtterance, AnalysisView};
use super::drafts::{load_draft, merged_assignments, validate_payload, TranscriptDraftPayload};
use super::*;
use rusqlite::{Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesJobRequest {
    pub meeting_id: String,
    pub operation_id: String,
    pub expected_analysis_id: String,
    pub expected_content_hash: String,
    pub expected_meeting_lock_version: i64,
    pub expected_draft: Option<DraftStamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MinutesInput {
    pub analysis: AnalysisView,
    pub meeting_title: String,
    pub meeting_created_at: i64,
    pub meeting_started_at_utc_ms: Option<i64>,
    pub working_content_hash: String,
}

// The native generation entry point checks the selected local engine first.
// The request carries editor stamps; pending saves must be flushed by its UI.
impl Storage {
    // Read-only preparation; callers release this transaction before invoking
    // a tokenizer and compare working_content_hash again during admission.
    pub(crate) fn preview_minutes_optimization_input(
        &self,
        meeting: &str,
    ) -> Result<MinutesInput, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (mut analysis, draft, title, created, started) = current_working(&tx, meeting)?;
        let payload = draft.map(|d| d.payload).unwrap_or_default();
        apply_edits(&mut analysis, &payload);
        let hash = working_hash(&tx, &analysis, &title, created, started, Some(&payload))?;
        Ok(MinutesInput {
            analysis,
            meeting_title: title,
            meeting_created_at: created,
            meeting_started_at_utc_ms: started,
            working_content_hash: hash,
        })
    }
    // Historical database fixtures only; the product admits Markdown jobs.
    #[cfg(test)]
    pub(crate) fn enqueue_minutes_job(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        generation_parameters: Value,
        total_units: Option<i64>,
    ) -> Result<JobSnapshot, String> {
        if !generation_parameters.is_object() {
            return Err("議事録生成の設定が不正です".into());
        }
        self.enqueue_minutes_job_configured(request, profile, |_| {
            Ok((generation_parameters, total_units))
        })
    }

    #[cfg(test)]
    pub(crate) fn enqueue_minutes_job_configured(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        make_generation: impl FnOnce(&MinutesInput) -> Result<(Value, Option<i64>), String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_minutes_job_with_target(request, profile, None, |input, _, _| {
            make_generation(input)
        })
    }

    pub(crate) fn enqueue_markdown_minutes_job_configured(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        target: super::minutes_markdown::GenerationTarget,
        make_generation: impl FnOnce(&MinutesInput) -> Result<(Value, Option<i64>), String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_minutes_job_with_target(request, profile, Some(target), |input, _, _| {
            make_generation(input)
        })
    }

    pub(crate) fn enqueue_markdown_minutes_job_with_prompts(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        target: super::minutes_markdown::GenerationTarget,
        make_generation: impl FnOnce(
            &MinutesInput,
            &Connection,
            Option<&Value>,
        ) -> Result<(Value, Option<i64>), String>,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_minutes_job_with_target(request, profile, Some(target), make_generation)
    }

    fn enqueue_minutes_job_with_target(
        &self,
        request: MinutesJobRequest,
        profile: Value,
        target: Option<super::minutes_markdown::GenerationTarget>,
        make_generation: impl FnOnce(
            &MinutesInput,
            &Connection,
            Option<&Value>,
        ) -> Result<(Value, Option<i64>), String>,
    ) -> Result<JobSnapshot, String> {
        self.require_data_writable()?;
        if target.is_some() && !super::minutes_markdown::uuid(&request.operation_id) {
            return Err("議事録生成の操作IDが不正です".into());
        }
        if request.operation_id.trim().is_empty()
            || request.operation_id.len() > 128
            || request.expected_meeting_lock_version < 0
            || !profile.is_object()
        {
            return Err("議事録生成の操作・設定が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let bind_target = |mut parameters: Value| -> Result<Value, String> {
            if let Some(target) = &target {
                let object = parameters
                    .as_object_mut()
                    .ok_or("議事録生成の設定が不正です")?;
                if object.contains_key("markdownTarget") {
                    return Err("議事録生成の保存条件が重複しています".into());
                }
                object.insert(
                    "markdownTarget".into(),
                    serde_json::to_value(target).map_err(|e| e.to_string())?,
                );
            }
            Ok(parameters)
        };
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id = ?1",
                params![request.operation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = existing {
            let (spec, _) = super::jobs::read_fixed_input(&tx, &id)?;
            let input = read_input(&tx, &spec)?;
            let (generation_parameters, total_units) =
                make_generation(&input, &tx, Some(&spec.parameters["generation"]))?;
            let generation_parameters = bind_target(generation_parameters)?;
            let parameters =
                json!({"schemaVersion":1, "request":request, "generation":generation_parameters});
            if spec.kind != JobKind::MinutesGeneration
                || spec.meeting_id != request.meeting_id
                || spec.profile_snapshot != profile
                || spec.parameters != parameters
                || spec.total_units != total_units
            {
                return Err("同じ操作IDで異なる議事録生成が指定されました".into());
            }
            return super::jobs::load_job(&tx, &id)?.ok_or("処理が見つかりません".into());
        }
        // Existing operation IDs may be read back after completion, but a new
        // operation must compare its human-confirmed base before copying input.
        if let Some(target) = &target {
            target.check(&tx, &request.meeting_id)?;
        }
        let (mut source, draft, title, created_at, started_at) =
            current_working(&tx, &request.meeting_id)?;
        let lock: i64 = tx
            .query_row(
                "SELECT lock_version FROM meetings WHERE id = ?1",
                params![request.meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let actual_draft = draft.as_ref().map(|d| DraftStamp {
            id: d.id.clone(),
            lock_version: d.lock_version,
        });
        if source.id != request.expected_analysis_id
            || source.content_hash != request.expected_content_hash
            || actual_draft != request.expected_draft
            || lock != request.expected_meeting_lock_version
        {
            return Err("生成開始前に文字起こし・下書き・会議情報が変更されています。保存後に再確認してください".into());
        }
        let payload = draft
            .as_ref()
            .map(|d| d.payload.clone())
            .unwrap_or_default();
        apply_edits(&mut source, &payload);
        let working_hash =
            working_hash(&tx, &source, &title, created_at, started_at, Some(&payload))?;
        let provenance = draft
            .as_ref()
            .map(|d| json!({"id":d.id,"lockVersion":d.lock_version,"contentHash":d.content_hash}));
        let id = copy_snapshot(
            &tx,
            &source,
            &payload,
            &title,
            created_at,
            started_at,
            &working_hash,
            provenance,
        )?;
        let hash: String = tx
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let fixed = read_snapshot(&tx, &request.meeting_id, &id, &hash)?;
        let (generation_parameters, total_units) = make_generation(&fixed, &tx, None)?;
        let generation_parameters = bind_target(generation_parameters)?;
        if !generation_parameters.is_object() {
            return Err("議事録生成の設定が不正です".into());
        }
        let parameters =
            json!({"schemaVersion":1, "request":request, "generation":generation_parameters});
        let spec = JobSpec {
            meeting_id: request.meeting_id,
            kind: JobKind::MinutesGeneration,
            operation_id: request.operation_id,
            input_analysis_id: Some(id),
            input_hash: hash,
            profile_snapshot: profile,
            parameters,
            total_units,
        };
        let job = super::jobs::enqueue_in_transaction(&tx, &spec)?;
        read_input(&tx, &spec)?;
        tx.commit().map_err(|e| e.to_string())?;
        crate::internal_logs::job(&job);
        Ok(job)
    }

    #[allow(dead_code)]
    pub(crate) fn load_minutes_input(&self, job_id: &str) -> Result<MinutesInput, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (spec, _) = super::jobs::read_fixed_input(&tx, job_id)?;
        read_input(&tx, &spec)
    }

    #[allow(dead_code)]
    pub(crate) fn minutes_input_updated(&self, job_id: &str) -> Result<bool, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (spec, _) = super::jobs::read_fixed_input(&tx, job_id)?;
        let fixed = read_input(&tx, &spec)?;
        input_updated(&tx, &spec.meeting_id, &fixed)
    }
}

pub(super) fn input_updated(
    tx: &Transaction<'_>,
    meeting_id: &str,
    fixed: &MinutesInput,
) -> Result<bool, String> {
    let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND current_analysis_id IS NOT NULL)", params![meeting_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !current {
        return Ok(true);
    }
    let (mut view, draft, title, created_at, started_at) = current_working(tx, meeting_id)?;
    if let Some(draft) = &draft {
        apply_edits(&mut view, &draft.payload);
    }
    Ok(working_hash(
        &tx,
        &view,
        &title,
        created_at,
        started_at,
        draft.as_ref().map(|d| &d.payload),
    )? != fixed.working_content_hash)
}

pub(super) fn current_working(
    tx: &Transaction<'_>,
    meeting: &str,
) -> Result<
    (
        AnalysisView,
        Option<TranscriptDraft>,
        String,
        i64,
        Option<i64>,
    ),
    String,
> {
    let (id, hash, title, created_at, started_at): (String, String, String, i64, Option<i64>) = tx.query_row(
        "SELECT a.id, a.content_hash, m.title, m.created_at, m.started_at_utc_ms FROM meetings m JOIN analysis_revisions a ON a.id = m.current_analysis_id AND a.meeting_id = m.id WHERE m.id = ?1 AND a.publication_state = 'committed'",
        params![meeting], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(|e| e.to_string())?.ok_or("議事録には現在の確定済み文字起こしが必要です")?;
    if super::revisions::analysis_content_hash(tx, &id)? != hash {
        return Err("現在版の内容が変更されています".into());
    }
    let view = read_analysis_content(tx, meeting, &id, &hash)?;
    if view.utterances.len() > 100_000 || view.speakers.len() > 100 {
        return Err("生成入力の件数が大きすぎます".into());
    }
    let mut draft = load_draft(tx, meeting)?;
    if let Some(d) = &mut draft {
        if d.base_analysis_id != id {
            return Err("下書きの基底と現在版が一致しません".into());
        }
        d.payload = validate_payload(tx, &id, d.payload.clone())?;
    }
    Ok((view, draft, title, created_at, started_at))
}

pub(super) fn apply_edits(view: &mut AnalysisView, payload: &TranscriptDraftPayload) {
    view.speakers
        .extend(payload.manual_speakers.iter().map(|s| AnalysisSpeaker {
            id: s.id.clone(),
            label: s.label.clone(),
            display_name: None,
        }));
    for edit in &payload.speaker_names {
        if let Some(speaker) = view.speakers.iter_mut().find(|s| s.id == edit.speaker_id) {
            speaker.display_name = edit.display_name.clone();
        }
    }
    let edits: HashMap<_, _> = payload
        .utterances
        .iter()
        .map(|e| (e.utterance_id.as_str(), e))
        .collect();
    for utterance in &mut view.utterances {
        let previous_ids = utterance.speaker_ids.clone();
        if let Some(edit) = edits.get(utterance.id.as_str()) {
            utterance.effective_text = edit.effective_text.clone();
            utterance.speaker_ids = edit.speaker_ids.clone();
        }
        if edits.contains_key(utterance.id.as_str()) || !payload.speaker_merges.is_empty() {
            utterance.speaker_ids = merged_assignments(payload, &utterance.speaker_ids);
            utterance.attribution_state = match utterance.speaker_ids.len() {
                0 => "unknown",
                1 => "assigned",
                _ => "overlap",
            }
            .into();
        }
        if previous_ids != utterance.speaker_ids {
            utterance.speaker_identification = None;
        }
    }
    if !payload.utterance_splits.is_empty() {
        let splits: HashMap<_, _> = payload
            .utterance_splits
            .iter()
            .map(|s| (s.source_utterance_id.as_str(), s))
            .collect();
        let mut result = Vec::new();
        for original in &view.utterances {
            if let Some(split) = splits.get(original.id.as_str()) {
                for part in &split.parts {
                    let edit = edits.get(part.id.as_str());
                    let ids = merged_assignments(
                        payload,
                        edit.map_or(&part.speaker_ids, |e| &e.speaker_ids),
                    );
                    result.push(super::analysis::AnalysisUtterance {
                        speaker_identification: None,
                        id: part.id.clone(),
                        track_id: original.track_id.clone(),
                        start_us: part.start_us,
                        end_us: part.end_us,
                        recognized_text: original.recognized_text.clone(),
                        effective_text: edit
                            .map_or(&part.effective_text, |e| &e.effective_text)
                            .clone(),
                        attribution_state: match ids.len() {
                            0 => "unknown",
                            1 => "assigned",
                            _ => "overlap",
                        }
                        .into(),
                        speaker_ids: ids,
                        recognition_is_source: true,
                    });
                }
            } else {
                result.push(original.clone());
            }
        }
        result.sort_by(|a, b| {
            (a.start_us, &a.track_id, &a.id).cmp(&(b.start_us, &b.track_id, &b.id))
        });
        view.utterances = result;
    }
}

// Stable lineage IDs allow comparison with the edited source despite remapped
// snapshot IDs. Only fields consumed by generation participate in this hash.
pub(super) fn working_hash(
    connection: &Connection,
    view: &AnalysisView,
    title: &str,
    created_at: i64,
    started_at: Option<i64>,
    payload: Option<&TranscriptDraftPayload>,
) -> Result<String, String> {
    let mut speaker_lineages = lineage_map(connection, "speakers", &view.id)?;
    if let Some(payload) = payload {
        for speaker in &payload.manual_speakers {
            speaker_lineages.insert(speaker.id.clone(), speaker.id.clone());
        }
    }
    let mut utterance_lineages = lineage_map(connection, "utterances", &view.id)?;
    if let Some(payload) = payload {
        for part in payload.utterance_splits.iter().flat_map(|s| &s.parts) {
            utterance_lineages.insert(part.id.clone(), part.id.clone());
        }
    }
    let mut speakers = vec![];
    for speaker in &view.speakers {
        let lineage = speaker_lineages
            .get(&speaker.id)
            .ok_or("話者の由来がありません")?;
        speakers.push((lineage, &speaker.label, &speaker.display_name));
    }
    speakers.sort_by(|a, b| a.0.cmp(&b.0));
    let mut utterances = vec![];
    for utterance in &view.utterances {
        let lineage = utterance_lineages
            .get(&utterance.id)
            .ok_or("発言の由来がありません")?;
        let mut assigned = utterance
            .speaker_ids
            .iter()
            .map(|id| {
                speaker_lineages
                    .get(id)
                    .cloned()
                    .ok_or("発言の話者参照が不正です")
            })
            .collect::<Result<Vec<_>, _>>()?;
        assigned.sort();
        utterances.push((
            lineage,
            &utterance.track_id,
            utterance.start_us,
            utterance.end_us,
            &utterance.effective_text,
            &utterance.attribution_state,
            assigned,
        ));
    }
    let serialized = serde_json::to_vec(&(title, created_at, started_at, speakers, utterances))
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(b"mon-minutes-working-input-v1");
    hash.update(serialized);
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn lineage_map(
    connection: &Connection,
    table: &str,
    analysis_id: &str,
) -> Result<HashMap<String, String>, String> {
    let sql = match table {
        "speakers" => "SELECT id, lineage_id FROM speakers WHERE analysis_revision_id = ?1",
        "utterances" => "SELECT id, lineage_id FROM utterances WHERE analysis_revision_id = ?1",
        _ => return Err("未対応の由来テーブルです".into()),
    };
    let mut query = connection.prepare(sql).map_err(|e| e.to_string())?;
    let rows = query
        .query_map(params![analysis_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub(super) fn copy_snapshot(
    tx: &Transaction<'_>,
    source: &AnalysisView,
    payload: &TranscriptDraftPayload,
    title: &str,
    created_at: i64,
    started_at: Option<i64>,
    working_hash: &str,
    draft: Option<Value>,
) -> Result<String, String> {
    copy_snapshot_with_consolidation(
        tx,
        source,
        payload,
        title,
        created_at,
        started_at,
        working_hash,
        draft,
        None,
    )
}

pub(super) fn copy_snapshot_with_consolidation(
    tx: &Transaction<'_>,
    source: &AnalysisView,
    payload: &TranscriptDraftPayload,
    title: &str,
    created_at: i64,
    started_at: Option<i64>,
    working_hash: &str,
    draft: Option<Value>,
    consolidation: Option<(
        &super::transcript_consolidation::Request,
        &[Vec<AnalysisUtterance>],
    )>,
) -> Result<String, String> {
    copy_snapshot_with_update(
        tx,
        source,
        payload,
        title,
        created_at,
        started_at,
        working_hash,
        draft,
        consolidation,
        None,
    )
}

pub(super) fn copy_snapshot_with_update(
    tx: &Transaction<'_>,
    source: &AnalysisView,
    payload: &TranscriptDraftPayload,
    title: &str,
    created_at: i64,
    started_at: Option<i64>,
    working_hash: &str,
    draft: Option<Value>,
    consolidation: Option<(
        &super::transcript_consolidation::Request,
        &[Vec<AnalysisUtterance>],
    )>,
    update: Option<&dyn Fn(&Transaction<'_>, &str) -> Result<(), String>>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let now = now_ms()?;
    let (audio_hash, profile, config): (String,String,String) = tx.query_row("SELECT audio_manifest_hash, profile_snapshot_json, config_json FROM analysis_revisions WHERE id = ?1", params![source.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e| e.to_string())?;
    let mut config: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    if !config.is_object() {
        return Err("解析設定が不正です".into());
    }
    config["inputSnapshot"] = json!({"schemaVersion":1,"sourceAnalysisId":source.id,"sourceContentHash":source.content_hash,"sourceDraft":draft,"workingContentHash":working_hash,"meetingTitle":title,"meetingCreatedAt":created_at,"meetingStartedAtUtcMs":started_at});
    if !payload.manual_speakers.is_empty() || !payload.speaker_merges.is_empty() {
        // Source labels and inferred segments remain intact; only effective
        // assignments change. Explicit provenance avoids false 1:1 lineage.
        config["manualSpeakerOperations"] = json!({"schemaVersion":1,"added":payload.manual_speakers,"merges":payload.speaker_merges});
    }
    let lineages = lineage_map(tx, "utterances", &source.id)?;
    if let Some(info) = config["speakerIdentification"].as_object_mut() {
        for u in source
            .utterances
            .iter()
            .filter(|u| u.speaker_identification.is_none())
        {
            info.remove(lineages.get(&u.id).unwrap_or(&u.id));
        }
    }
    let mut source_recognition: Vec<_> = source
        .utterances
        .iter()
        .filter(|u| u.recognition_is_source)
        .map(|u| lineages.get(&u.id).cloned().unwrap_or_else(|| u.id.clone()))
        .collect();
    source_recognition.sort();
    if !source_recognition.is_empty() {
        config["sourceRecognitionLineages"] = json!(source_recognition);
    }
    if !payload.utterance_splits.is_empty() {
        let previous = config
            .get("manualUtteranceSplits")
            .cloned()
            .unwrap_or(json!([]));
        let mut operations: Vec<Value> =
            serde_json::from_value(previous).map_err(|e| e.to_string())?;
        operations.push(json!({"sourceAnalysisId":source.id,"sourceContentHash":source.content_hash,"splits":payload.utterance_splits}));
        config["manualUtteranceSplits"] = json!(operations);
    }
    if config.to_string().len() > 2 * 1024 * 1024 {
        return Err("固定入力の由来情報が大きすぎます".into());
    }
    let sequence: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(revision_no),0) + 1 FROM analysis_revisions WHERE meeting_id = ?1",
            params![source.meeting_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO analysis_revisions (id, meeting_id, parent_revision_id, parent_content_hash, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1,?2,?3,?4,?5,'manual_snapshot','building',?6,?7,?8,?9,1,?10)", params![id,source.meeting_id,source.id,source.content_hash,sequence,audio_hash,profile,config.to_string(),"0".repeat(64),now]).map_err(|e| e.to_string())?;
    let mut speaker_ids = HashMap::new();
    for speaker in &source.speakers {
        let new_id = Uuid::new_v4().to_string();
        let changed = if payload.manual_speakers.iter().any(|s| s.id == speaker.id) {
            tx.execute("INSERT INTO speakers (id,analysis_revision_id,lineage_id,label,display_name,input_scope,identification_state,created_at) VALUES (?1,?2,?3,?4,?5,'all','manual',?6)", params![new_id,id,speaker.id,speaker.label,speaker.display_name,now])
        } else {
            tx.execute("INSERT INTO speakers (id,analysis_revision_id,lineage_id,label,display_name,input_scope,identification_state,created_at) SELECT ?1,?2,lineage_id,label,?3,input_scope,identification_state,?4 FROM speakers WHERE id = ?5 AND analysis_revision_id = ?6", params![new_id,id,speaker.display_name,now,speaker.id,source.id])
        }.map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("話者の固定入力保存に失敗しました".into());
        }
        speaker_ids.insert(&speaker.id, new_id);
    }
    let mut manual: std::collections::HashSet<_> = payload
        .utterances
        .iter()
        .map(|e| e.utterance_id.clone())
        .collect();
    if !payload.speaker_merges.is_empty() {
        let merges = serde_json::to_string(&payload.speaker_merges).map_err(|e| e.to_string())?;
        let mut query = tx.prepare("SELECT DISTINCT utterance_id FROM utterance_speakers WHERE analysis_revision_id=?1 AND speaker_id IN (SELECT json_extract(value,'$.source_id') FROM json_each(?2))").map_err(|e|e.to_string())?;
        for row in query
            .query_map(params![source.id, merges], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
        {
            manual.insert(row.map_err(|e| e.to_string())?);
        }
    }
    let split_sources: HashMap<_, _> = payload
        .utterance_splits
        .iter()
        .flat_map(|s| {
            s.parts
                .iter()
                .map(move |p| (p.id.as_str(), s.source_utterance_id.as_str()))
        })
        .collect();
    for (sequence, utterance) in source.utterances.iter().enumerate() {
        let new_id = Uuid::new_v4().to_string();
        let changed=if split_sources.contains_key(utterance.id.as_str()) {
            manual.insert(utterance.id.clone());
            tx.execute("INSERT INTO utterances (id,meeting_id,analysis_revision_id,track_id,lineage_id,sequence,start_us,end_us,recognized_text,effective_text,attribution_state,confidence) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL)",params![new_id,source.meeting_id,id,utterance.track_id,utterance.id,sequence as i64,utterance.start_us,utterance.end_us,utterance.recognized_text,utterance.effective_text,utterance.attribution_state])
        } else {
            tx.execute("INSERT INTO utterances (id,meeting_id,analysis_revision_id,track_id,lineage_id,sequence,start_us,end_us,recognized_text,effective_text,attribution_state,confidence) SELECT ?1,meeting_id,?2,track_id,lineage_id,?7,start_us,end_us,recognized_text,?3,?4,confidence FROM utterances WHERE id = ?5 AND analysis_revision_id = ?6", params![new_id,id,utterance.effective_text,utterance.attribution_state,utterance.id,source.id,sequence as i64])
        }.map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("発言の固定入力保存に失敗しました".into());
        }
        for (index, old_speaker) in utterance.speaker_ids.iter().enumerate() {
            let new_speaker = speaker_ids
                .get(old_speaker)
                .ok_or("話者の対応付けが不正です")?;
            if manual.contains(&utterance.id) {
                tx.execute("INSERT INTO utterance_speakers (utterance_id,speaker_id,analysis_revision_id,role,assignment_source) VALUES (?1,?2,?3,?4,'manual')", params![new_id,new_speaker,id,if index == 0 {"primary"} else {"overlap"}]).map_err(|e| e.to_string())?;
            } else {
                tx.execute("INSERT INTO utterance_speakers (utterance_id,speaker_id,analysis_revision_id,role,assignment_source) SELECT ?1,?2,?3,role,assignment_source FROM utterance_speakers WHERE utterance_id = ?4 AND speaker_id = ?5 AND analysis_revision_id = ?6", params![new_id,new_speaker,id,utterance.id,old_speaker,source.id]).map_err(|e| e.to_string())?;
            }
        }
    }
    let segments = {
        let mut query = tx
            .prepare("SELECT id,speaker_id FROM speaker_segments WHERE analysis_revision_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![source.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    if segments.len() > 500_000 {
        return Err("話者区間の件数が大きすぎます".into());
    }
    for (segment, speaker) in segments {
        let mapped = speaker
            .as_ref()
            .map(|old| speaker_ids.get(old).ok_or("話者区間の参照が不正です"))
            .transpose()?;
        tx.execute("INSERT INTO speaker_segments (id,meeting_id,analysis_revision_id,track_id,speaker_id,start_us,end_us,overlap_group_id,confidence) SELECT ?1,meeting_id,?2,track_id,?3,start_us,end_us,overlap_group_id,confidence FROM speaker_segments WHERE id = ?4 AND analysis_revision_id = ?5", params![Uuid::new_v4().to_string(),id,mapped,segment,source.id]).map_err(|e| e.to_string())?;
    }
    if let Some((request, groups)) = consolidation {
        super::transcript_consolidation::consolidate_copied_rows(
            tx, source, payload, &id, request, groups,
        )?;
    }
    if let Some(update) = update {
        update(tx, &id)?;
    }
    let hash = super::revisions::analysis_content_hash(tx, &id)?;
    let state = if consolidation.is_some() || update.is_some() {
        "committed"
    } else {
        "internal_snapshot"
    };
    tx.execute("UPDATE analysis_revisions SET content_hash = ?1, publication_state = ?3 WHERE id = ?2 AND publication_state = 'building'", params![hash,id,state]).map_err(|e| e.to_string())?;
    if consolidation.is_none() && update.is_none() {
        tx.execute("INSERT INTO analysis_input_snapshots (analysis_id,meeting_id,source_analysis_id,source_content_hash,source_draft_json,working_content_hash,meeting_title,meeting_created_at,meeting_started_at_utc_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![id,source.meeting_id,source.id,source.content_hash,draft.map(|d| d.to_string()),working_hash,title,created_at,started_at]).map_err(|e| e.to_string())?;
    }
    Ok(id)
}

pub(super) fn read_input(tx: &Transaction<'_>, spec: &JobSpec) -> Result<MinutesInput, String> {
    if spec.kind != JobKind::MinutesGeneration {
        return Err("議事録生成ジョブではありません".into());
    }
    let id = spec
        .input_analysis_id
        .as_deref()
        .ok_or("固定入力版がありません")?;
    read_snapshot(tx, &spec.meeting_id, id, &spec.input_hash)
}

pub(super) fn read_snapshot(
    tx: &Transaction<'_>,
    meeting_id: &str,
    id: &str,
    input_hash: &str,
) -> Result<MinutesInput, String> {
    let (hash,source,source_hash,draft,working,title,created_at,started_at,config): (String,String,String,Option<String>,String,String,i64,Option<i64>,String) = tx.query_row("SELECT a.content_hash,s.source_analysis_id,s.source_content_hash,s.source_draft_json,s.working_content_hash,s.meeting_title,s.meeting_created_at,s.meeting_started_at_utc_ms,a.config_json FROM analysis_input_snapshots s JOIN analysis_revisions a ON a.id = s.analysis_id AND a.meeting_id = s.meeting_id WHERE a.id = ?1 AND a.meeting_id = ?2 AND a.publication_state = 'internal_snapshot'", params![id,meeting_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|e| e.to_string())?.ok_or("議事録の固定入力スナップショットがありません")?;
    if hash != input_hash || super::revisions::analysis_content_hash(tx, id)? != hash {
        return Err("生成入力スナップショットが変更されています".into());
    }
    let config: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    let draft: Option<Value> = draft
        .map(|d| serde_json::from_str(&d))
        .transpose()
        .map_err(|e| e.to_string())?;
    if config["inputSnapshot"]
        != json!({"schemaVersion":1,"sourceAnalysisId":source,"sourceContentHash":source_hash,"sourceDraft":draft,"workingContentHash":working,"meetingTitle":title,"meetingCreatedAt":created_at,"meetingStartedAtUtcMs":started_at})
    {
        return Err("生成入力の由来情報が変更されています".into());
    }
    let view = read_analysis_content(tx, meeting_id, id, &hash)?;
    if working_hash(tx, &view, &title, created_at, started_at, None)? != working {
        return Err("生成入力の作業内容ハッシュが一致しません".into());
    }
    Ok(MinutesInput {
        analysis: view,
        meeting_title: title,
        meeting_created_at: created_at,
        meeting_started_at_utc_ms: started_at,
        working_content_hash: working,
    })
}

#[cfg(test)]
#[path = "minutes_input_tests.rs"]
mod tests;
