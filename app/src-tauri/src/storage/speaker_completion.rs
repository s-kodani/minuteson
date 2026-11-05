//! Frozen plans and atomic publication; detector segments remain source evidence.
use super::*;
use crate::speaker_completion::{Decision, Plan, Target, ALGORITHM};
use crate::voice::Window;
use rusqlite::{Transaction, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

fn working(
    tx: &Transaction<'_>,
    meeting: &str,
) -> Result<(AnalysisView, Option<TranscriptDraft>, String), String> {
    let (mut view, draft, title, created, started) = minutes_input::current_working(tx, meeting)?;
    // Include the complete saved editor state, including its ID/version. A text
    // edit or an edit-and-undo must not authorize publication of an older job.
    let hash = jobs::hash_bytes(
        b"mon-speaker-completion-working-v1",
        &serde_json::to_vec(&(&view, &draft, &title, created, started))
            .map_err(|e| e.to_string())?,
    );
    if let Some(d) = &draft {
        minutes_input::apply_edits(&mut view, &d.payload);
    }
    Ok((view, draft, hash))
}
#[derive(Clone)]
struct Segment {
    track: String,
    speaker: Option<String>,
    start: i64,
    end: i64,
}
fn evidence(
    tx: &Transaction<'_>,
    view: &AnalysisView,
    known: &KnownInput,
    source: &RecordedAudioPlan,
) -> Result<(Vec<Window>, Vec<Target>, BTreeMap<String, usize>), String> {
    known.validate(source)?;
    let mut q=tx.prepare("SELECT track_id,speaker_id,start_us,end_us FROM speaker_segments WHERE analysis_revision_id=?1 ORDER BY track_id,start_us,end_us,id LIMIT 100001").map_err(|e|e.to_string())?;
    let mut segments = q
        .query_map([&view.id], |r| {
            Ok(Segment {
                track: r.get(0)?,
                speaker: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if segments.len() > 100000 {
        return Err("話者補完の話者区間が上限を超えています".into());
    }
    // A saved speaker merge is already the user's identity choice even before
    // it is materialized into a revision. Never match an alias that publication
    // will remove; use the same effective mapping as transcript assignment.
    if let Some(draft) = super::drafts::load_draft(tx, &view.meeting_id)? {
        if draft.base_analysis_id != view.id {
            return Err("話者統合の対象版が変わりました".into());
        }
        for segment in &mut segments {
            if let Some(id) = segment.speaker.as_ref() {
                segment.speaker = super::drafts::merged_assignments(&draft.payload, &[id.clone()])
                    .into_iter()
                    .next();
            }
        }
    }
    for gap in known.gaps.iter().filter(|g| !g.recovered) {
        segments.push(Segment {
            track: source.tracks[gap.track_index].id.clone(),
            speaker: None,
            start: gap.start_us,
            end: gap.end_us,
        });
    }
    let mut solo: BTreeMap<(String, String), Vec<(i64, i64)>> = BTreeMap::new();
    // Sweep each continuous run; an unknown detector label also prevents solo use.
    for run in &known.runs {
        let track = &source.tracks[run.track_index].id;
        let mut events: BTreeMap<i64, Vec<(String, i32)>> = BTreeMap::new();
        for s in segments
            .iter()
            .filter(|s| &s.track == track && s.start < run.end_us && s.end > run.start_us)
        {
            let id = s.speaker.clone().unwrap_or_default();
            events
                .entry(s.start.max(run.start_us))
                .or_default()
                .push((id.clone(), 1));
            events
                .entry(s.end.min(run.end_us))
                .or_default()
                .push((id, -1));
        }
        let mut active: BTreeMap<String, i32> = BTreeMap::new();
        let mut previous = run.start_us;
        for (time, changes) in events {
            if time > previous && active.len() == 1 {
                let id = active.keys().next().unwrap();
                if !id.is_empty() {
                    let spans = solo.entry((track.clone(), id.clone())).or_default();
                    if let Some(last) = spans
                        .last_mut()
                        .filter(|s| s.1 == previous && s.0 >= run.start_us)
                    {
                        last.1 = time;
                    } else {
                        spans.push((previous, time));
                    }
                }
            }
            for (id, change) in changes {
                let n = active.entry(id.clone()).or_default();
                *n += change;
                if *n == 0 {
                    active.remove(&id);
                }
            }
            previous = time;
        }
    }
    let mut references = Vec::new();
    for ((track, speaker), mut spans) in solo {
        if !view.speakers.iter().any(|s| s.id == speaker) {
            continue;
        }
        spans.retain(|(a, b)| b - a >= 3_000_000);
        spans.sort_by_key(|(a, b)| (std::cmp::Reverse(b - a), *a));
        for (mut start, mut end) in spans.into_iter().take(2) {
            if end - start > 30_000_000 {
                let mid = start + (end - start) / 2;
                start = mid - 15_000_000;
                end = mid + 15_000_000;
            }
            references.push(Window {
                speaker_id: speaker.clone(),
                track_id: track.clone(),
                start_us: start,
                end_us: end,
            });
        }
    }
    let manual = super::speaker_assignment_update::manual_lineages(tx, &view.id)?;
    let lineages = minutes_input::lineage_map(tx, "utterances", &view.id)?;
    let mut targets = Vec::new();
    let mut excluded = BTreeMap::new();
    for u in &view.utterances {
        if !u.speaker_ids.is_empty() {
            continue;
        }
        if known.gaps.iter().any(|g| {
            !g.recovered
                && source.tracks[g.track_index].id == u.track_id
                && g.start_us < u.end_us
                && u.start_us < g.end_us
        }) {
            *excluded.entry("missing_audio".into()).or_default() += 1;
            continue;
        }
        let lineage = lineages.get(&u.id).unwrap_or(&u.id);
        let reason = if manual.contains(lineage) {
            Some("manual")
        } else if u.end_us - u.start_us < 1_000_000 {
            Some("too_short")
        } else {
            None
        };
        if let Some(reason) = reason {
            *excluded.entry(reason.into()).or_default() += 1;
            continue;
        }
        let Some(run) = known.runs.iter().find(|r| {
            source.tracks[r.track_index].id == u.track_id
                && r.start_us <= u.start_us
                && u.end_us <= r.end_us
        }) else {
            *excluded.entry("missing_audio".into()).or_default() += 1;
            continue;
        };
        let pad = 0;
        let start = (u.start_us - pad).max(run.start_us);
        let end = u.end_us.saturating_add(pad).min(run.end_us);
        if end - start > 30_000_000 {
            *excluded.entry("too_long".into()).or_default() += 1;
            continue;
        }
        targets.push(Target {
            lineage: lineage.clone(),
            body: Window {
                speaker_id: String::new(),
                track_id: u.track_id.clone(),
                start_us: u.start_us,
                end_us: u.end_us,
            },
        });
    }
    if targets.len() > 1000 || references.len() > 400 {
        return Err("話者補完の対象が上限を超えています（発言1000件・参照400区間）".into());
    }
    Ok((references, targets, excluded))
}
fn valid_decision(d: &Decision, t: &Target, plan: &Plan) -> bool {
    let Some(score) = d
        .similarity
        .filter(|v| v.is_finite() && (-1.0..=1.0).contains(v))
    else {
        return false;
    };
    if d.runner_up
        .is_some_and(|v| !v.is_finite() || !(-1.0..=score).contains(&v))
    {
        return false;
    }
    if !d.needs_review {
        return !d.needs_review
            && d.reason == "applied"
            && plan.options.accepts(score, d.runner_up, true);
    }
    [
        "nearest_tie",
        "nearest_below_threshold",
        "nearest_different_winners",
        "nearest_other_speaker_in_context",
        "nearest_feature_unavailable",
    ]
    .contains(&d.reason.as_str())
}
fn check(tx: &Transaction<'_>, spec: &JobSpec, plan: &Plan) -> Result<(), String> {
    if spec.kind != JobKind::SpeakerCompletion
        || plan.schema_version != crate::current_formats::COMPLETION
        || plan.resident_runtime.is_none()
        || plan.algorithm != ALGORITHM
        || spec.parameters["completion"] != serde_json::to_value(plan).map_err(|e| e.to_string())?
    {
        return Err("話者補完の固定条件が一致しません".into());
    }
    plan.options.validate()?;
    if transcription::read_audio_plan(tx, &spec.meeting_id)? != plan.source {
        return Err("話者補完の音声構成が変わりました".into());
    }
    let (view, _, hash) = working(tx, &spec.meeting_id)?;
    if Some(&view.id) != spec.input_analysis_id.as_ref()
        || view.content_hash != spec.input_hash
        || hash != plan.working_hash
    {
        return Err("会議や編集内容が変わりました。話者割り当てを更新し直してください".into());
    }
    let known = transcription::known::read(tx, &spec.meeting_id, &plan.source)?;
    if known != plan.known {
        return Err("話者補完の保存音声区間が変わりました".into());
    }
    let (refs, targets, excluded) = evidence(tx, &view, &known, &plan.source)?;
    if refs != plan.references || targets != plan.targets || excluded != plan.excluded {
        return Err("話者補完の対象・参照区間が一致しません".into());
    }
    Ok(())
}
impl Storage {
    pub(crate) fn completion_opt_in(&self, meeting: &str) -> Result<bool, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let job:Option<String>=db.query_row("SELECT json_extract(a.config_json,'$.jobId') FROM meetings m JOIN analysis_revisions a ON a.id=m.current_analysis_id WHERE m.id=?1",[meeting],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
        let Some(id) = job else {
            return Ok(false);
        };
        Ok(
            jobs::read_fixed_input(&db, &id)?.0.parameters["diarization"]["completionAfter"]
                == true,
        )
    }

    #[cfg(test)]
    pub(crate) fn assignment_completion_reply(
        &self,
        request: &transcript_consolidation::Request,
    ) -> Result<Option<super::speaker_assignment_update::Reply>, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id=?1",
                [&request.operation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(id) = id else {
            return Ok(None);
        };
        let (spec, _) = jobs::read_fixed_input(&tx, &id)?;
        if spec.kind != JobKind::SpeakerCompletion
            || spec.meeting_id != request.meeting_id
            || spec.parameters["assignmentRequest"]
                != serde_json::to_value(request).map_err(|e| e.to_string())?
        {
            return Err("同じ操作IDの話者補完の条件が異なります".into());
        }
        let input = spec
            .input_analysis_id
            .as_deref()
            .ok_or("話者補完の入力版がありません")?;
        let count = if input == request.analysis_id {
            0
        } else {
            let text: String = tx
                .query_row(
                    "SELECT config_json FROM analysis_revisions WHERE id=?1",
                    [input],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let c: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if c["speakerAssignmentUpdate"]["operationId"] != request.operation_id {
                return Err("話者割り当ての更新記録が一致しません".into());
            }
            c["speakerAssignmentUpdate"]["updatedCount"]
                .as_u64()
                .ok_or("更新件数がありません")? as usize
        };
        let (view, draft, _, _, _) = minutes_input::current_working(&tx, &request.meeting_id)?;
        Ok(Some(super::speaker_assignment_update::Reply {
            analysis: view,
            draft,
            updated_count: count,
            matching_error: None,
            completion_job_id: Some(id),
        }))
    }
    pub(crate) fn enqueue_completion(&self, meeting: &str) -> Result<Option<JobSnapshot>, String> {
        self.enqueue_completion_for(meeting, None, None)
    }
    #[cfg(test)]
    pub(crate) fn enqueue_completion_for_assignment(
        &self,
        request: &transcript_consolidation::Request,
    ) -> Result<Option<JobSnapshot>, String> {
        self.enqueue_completion_for(&request.meeting_id, Some(request), None)
    }
    pub(crate) fn enqueue_completion_intent(
        &self,
        intent: &post_analysis::Intent,
    ) -> Result<Option<JobSnapshot>, String> {
        self.enqueue_completion_for(&intent.meeting_id, None, Some(intent))
    }
    fn enqueue_completion_for(
        &self,
        meeting: &str,
        request: Option<&transcript_consolidation::Request>,
        intent: Option<&post_analysis::Intent>,
    ) -> Result<Option<JobSnapshot>, String> {
        self.require_data_writable()?;
        if !cfg!(any(target_os = "macos", windows)) {
            return Ok(None);
        }
        let (source, _) = self.recorded_analysis_input(meeting)?;
        // Avoid requiring model files when there is no eligible missing speech.
        {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let (view, _, _) = working(&tx, meeting)?;
            let known = transcription::known::read(&tx, meeting, &source)?;
            let (references, targets, excluded) = evidence(&tx, &view, &known, &source)?;
            crate::internal_logs::detail(
                crate::internal_logs::Context {
                    category: "speaker_completion".into(),
                    meeting_id: Some(meeting.into()),
                    ..Default::default()
                },
                "preparing", "info", std::time::Duration::ZERO,
                &serde_json::json!({"event":"補完対象確認","referenceWindows":references.len(),"targetUtterances":targets.len(),"excluded":excluded}).to_string(),
            );
            if targets.is_empty() {
                return Ok(None);
            }
        }
        let (engine, extractor, managed, runtime) = if let Some(intent) = intent {
            let fixed = intent.execution()?;
            fixed.engine.verify(&|| false)?;
            (
                fixed.engine.clone(),
                fixed.extractor.clone(),
                fixed.managed.clone(),
                fixed.runtime.clone(),
            )
        } else {
            let engine = crate::voice::engine(self)?;
            let extractor = crate::diarization::embedding::ExtractorConfig::capture(
                &crate::diarization::known::helper_path()?,
                &engine,
            )?;
            let managed = self.prepare_model_selection(
                "standard",
                model_profiles::jobs::Execution::Sherpa(&engine),
                &|| false,
            )?;
            let runtime = crate::local_engine::resident::prepare(self.root(), "sherpa")?;
            (engine, extractor, managed, runtime)
        };
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (view, _, working_hash) = working(&tx, meeting)?;
        let known = transcription::known::read(&tx, meeting, &source)?;
        if let Some(intent) = intent {
            // The completion plan and intent bind different hash domains.
            // Preserve both guards without comparing their SHA values directly.
            let (_, _, intent_working_hash) = super::voice::working(&tx, meeting)?;
            intent.check(&tx, &view.id, &intent_working_hash)?;
        }
        let options = if let Some(intent) = intent {
            intent.policy.completion
        } else {
            super::completion_preferences::load(&tx)?.payload
        };

        let (references, targets, excluded) = evidence(&tx, &view, &known, &source)?;
        if targets.is_empty() {
            return Ok(None);
        }
        let plan = Plan {
            resident_runtime: runtime,
            schema_version: crate::current_formats::COMPLETION,
            algorithm: ALGORITHM.into(),
            options,
            engine,
            extractor,
            source,
            known,
            working_hash,
            references,
            targets,
            excluded,
        };
        let value = serde_json::to_value(&plan).map_err(|e| e.to_string())?;
        let mut q=tx.prepare("SELECT id,parameters_json FROM processing_jobs WHERE meeting_id=?1 AND kind='speaker_completion' AND input_analysis_id=?2 AND state NOT IN ('failed','cancelled') ORDER BY created_at DESC,rowid DESC").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![meeting, view.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(q);
        for (id, text) in rows {
            let old: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if old["completion"] == value {
                let job = jobs::load_job(&tx, &id)?;
                if let (Some(intent), Some(job)) =
                    (intent, job.as_ref().filter(|j| j.state != "succeeded"))
                {
                    intent.enqueued(&tx, &job.id, true)?;
                    tx.commit().map_err(|e| e.to_string())?;
                }
                return Ok(job.filter(|j| j.state != "succeeded"));
            }
        }
        let spec = JobSpec {
            meeting_id: meeting.into(),
            kind: JobKind::SpeakerCompletion,
            operation_id: intent
                .map(|i| i.completion_operation_id.clone())
                .or_else(|| request.map(|r| r.operation_id.clone()))
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            input_analysis_id: Some(view.id),
            input_hash: view.content_hash,
            profile_snapshot: model_profiles::jobs::attach(
                serde_json::json!({"profile":"standard","engine":"sherpa-onnx"}),
                managed,
            )?,
            parameters: serde_json::json!({"completion":plan,"assignmentRequest":request,"automaticAnalysisProgress":request.is_none()}),
            total_units: Some(1),
        };
        let job = jobs::enqueue_in_transaction(&tx, &spec)?;
        if let Some(intent) = intent {
            intent.enqueued(&tx, &job.id, true)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some(job))
    }
    pub(crate) fn check_completion_plan(&self, spec: &JobSpec, plan: &Plan) -> Result<(), String> {
        self.require_data_writable()?;
        if self.recorded_analysis_input(&spec.meeting_id)?.0 != plan.source {
            return Err("話者補完に使う音声が変わりました".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        check(&tx, spec, plan)
    }
    pub(crate) fn publish_completion_result(
        &self,
        spec: &JobSpec,
        plan: &Plan,
        decisions: &[Decision],
        job_id: &str,
    ) -> Result<JobSnapshot, String> {
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        check(&tx, spec, plan)?;
        let job = jobs::load_job(&tx, job_id)?.ok_or("話者補完のジョブがありません")?;
        if job.state != "running" || jobs::read_fixed_input(&tx, job_id)?.0 != *spec {
            return Err("話者補完が停止したか固定条件が異なります".into());
        }
        if decisions.len() != plan.targets.len()
            || decisions.iter().zip(&plan.targets).any(|(d, t)| {
                d.lineage != t.lineage
                    || d.speaker_id.as_ref().is_some_and(|id| {
                        !valid_decision(d, t, plan)
                            || !plan
                                .references
                                .iter()
                                .any(|w| w.track_id == t.body.track_id && &w.speaker_id == id)
                            || d.body_best.as_ref() != Some(id)
                    })
            })
        {
            return Err("話者補完の結果が固定条件と一致しません".into());
        }
        let updated = decisions.iter().filter(|d| d.speaker_id.is_some()).count();
        let (mut source, draft, title, created, started) =
            minutes_input::current_working(&tx, &spec.meeting_id)?;
        let payload = draft
            .as_ref()
            .map(|d| d.payload.clone())
            .unwrap_or_default();
        minutes_input::apply_edits(&mut source, &payload);
        let mut reasons = plan.excluded.clone();
        for d in decisions {
            if d.speaker_id.is_none() {
                *reasons.entry(d.reason.clone()).or_default() += 1;
            }
        }
        let mut receipt = serde_json::json!({"schemaVersion":1,"completionResult":true,"jobId":job_id,"updatedCount":updated,"needsReviewCount":decisions.iter().filter(|d|d.speaker_id.is_some() && d.needs_review).count(),"skipped":reasons,"decisions":decisions,"outputAnalysisId":null});
        let output = if updated > 0 {
            let request = transcript_consolidation::Request {
                meeting_id: spec.meeting_id.clone(),
                analysis_id: source.id.clone(),
                expected_content_hash: source.content_hash.clone(),
                expected_draft: draft.as_ref().map(|d| DraftStamp {
                    id: d.id.clone(),
                    lock_version: d.lock_version,
                }),
                operation_id: job.operation_id.clone(),
            };
            let speaker_lineages = minutes_input::lineage_map(&tx, "speakers", &source.id)?;
            let update = |tx: &Transaction<'_>, id: &str| -> Result<(), String> {
                transcript_consolidation::consolidate_copied_rows(
                    tx,
                    &source,
                    &payload,
                    id,
                    &request,
                    &[],
                )?;
                for d in decisions.iter().filter(|d| d.speaker_id.is_some()) {
                    let speaker = d.speaker_id.as_ref().unwrap();
                    let lineage = speaker_lineages.get(speaker).unwrap_or(speaker);
                    let mapped:String=tx.query_row("SELECT id FROM speakers WHERE analysis_revision_id=?1 AND lineage_id=?2",params![id,lineage],|r|r.get(0)).map_err(|e|e.to_string())?;
                    let u:String=tx.query_row("SELECT id FROM utterances WHERE analysis_revision_id=?1 AND lineage_id=?2",params![id,d.lineage],|r|r.get(0)).map_err(|e|e.to_string())?;
                    tx.execute("INSERT INTO utterance_speakers(utterance_id,speaker_id,analysis_revision_id,role,assignment_source) VALUES(?1,?2,?3,'primary','inferred')",params![u,mapped,id]).map_err(|e|e.to_string())?;
                    tx.execute(
                        "UPDATE utterances SET attribution_state='assigned' WHERE id=?1",
                        [u],
                    )
                    .map_err(|e| e.to_string())?;
                }
                let text: String = tx
                    .query_row(
                        "SELECT config_json FROM analysis_revisions WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let mut config: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| e.to_string())?;
                config
                    .as_object_mut()
                    .ok_or("解析設定が不正です")?
                    .remove("utteranceConsolidation");
                if !config["speakerIdentification"].is_object() {
                    config["speakerIdentification"] = serde_json::json!({});
                }
                for (d, target) in decisions
                    .iter()
                    .zip(&plan.targets)
                    .filter(|(d, _)| d.speaker_id.is_some())
                {
                    let speaker = d.speaker_id.as_ref().unwrap();
                    let lineage = speaker_lineages.get(speaker).unwrap_or(speaker);
                    config["speakerIdentification"][&d.lineage] = serde_json::json!({
                        "speakerLineage":lineage,"trackId":target.body.track_id,
                        "startUs":target.body.start_us,"endUs":target.body.end_us,
                        "similarity":d.similarity,"runnerUp":d.runner_up,
                        "needsReview":d.needs_review,"reason":d.reason,"algorithm":plan.algorithm,
                    });
                }
                config["speakerCompletion"] = serde_json::json!({"schemaVersion":1,"jobId":job_id,"updatedCount":updated,"algorithm":plan.algorithm});
                if config.to_string().len() > 2 * 1024 * 1024 {
                    return Err("話者識別の由来情報が容量上限を超えています".into());
                }
                tx.execute(
                    "UPDATE analysis_revisions SET config_json=?1 WHERE id=?2",
                    params![config.to_string(), id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            };
            let id = minutes_input::copy_snapshot_with_update(
                &tx,
                &source,
                &payload,
                &title,
                created,
                started,
                &plan.working_hash,
                None,
                None,
                Some(&update),
            )?;
            transcript_consolidation::copy_voice_matches(&tx, &source, &id)?;
            tx.execute(
                "DELETE FROM working_drafts WHERE meeting_id=?1 AND target_kind='transcript'",
                [&spec.meeting_id],
            )
            .map_err(|e| e.to_string())?;
            if tx.execute("UPDATE meetings SET current_analysis_id=?1,lock_version=lock_version+1,updated_at=?3 WHERE id=?4 AND current_analysis_id=?2",params![id,source.id,now_ms()?,spec.meeting_id]).map_err(|e|e.to_string())?!=1{return Err("補完対象の版が変わりました".into());}
            Some(id)
        } else {
            None
        };
        receipt["outputAnalysisId"] = serde_json::json!(output);
        let text = receipt.to_string();
        if text.len() > 1024 * 1024 {
            return Err("話者補完の結果が容量上限を超えています".into());
        }
        let sequence: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM job_checkpoints WHERE job_id=?1",
                [job_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO job_checkpoints(id,job_id,sequence,completed_units,checkpoint_json,checkpoint_hash,created_at) VALUES(?1,?2,?3,1,?4,?5,?6)",params![Uuid::new_v4().to_string(),job_id,sequence,text,jobs::checkpoint_hash(&job.spec_hash,1,&text),now_ms()?]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE processing_jobs SET state='succeeded',completed_units=1,finished_at=?1,updated_at=?1 WHERE id=?2 AND state='running'",params![now_ms()?,job_id]).map_err(|e|e.to_string())?;
        let (followup_view, _, followup_hash) = super::voice::working(&tx, &spec.meeting_id)?;
        let advanced=tx.execute("UPDATE post_analysis_intents SET phase='matching_pending',current_analysis_id=?1,working_hash=?2,updated_at=?3 WHERE completion_job_id=?4 AND phase='completion_enqueued'",params![followup_view.id,followup_hash,now_ms()?,job_id]).map_err(|e|e.to_string())?;
        if advanced == 0 {
            super::post_analysis::record_after_completion(&tx, spec, plan, &followup_view.id)?;
        }
        let job = jobs::load_job(&tx, job_id)?.ok_or("話者補完のジョブがありません")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(job)
    }
}

#[cfg(test)]
pub(in crate::storage) mod tests;

// Domain publication checks use the same read transaction as the notification
// dispatcher; a valid historical job alone does not prove saved output content.
impl crate::speaker_completion::result::OutputResolver for Transaction<'_> {
    fn verify(&self, job: &JobSnapshot, id: &str) -> Result<(), String> {
        let valid:bool=self.query_row("SELECT EXISTS(SELECT 1 FROM analysis_revisions WHERE id=?1 AND meeting_id=?2 AND json_extract(config_json,'$.speakerCompletion.jobId')=?3)",params![id,job.meeting_id,job.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !valid {
            return Err("話者補完の保存版が一致しません".into());
        }
        let stored: String = self
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if stored != super::revisions::analysis_content_hash(self, id)? {
            return Err("話者補完の保存版の内容が変わっています".into());
        }
        Ok(())
    }
}
