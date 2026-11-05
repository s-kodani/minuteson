//! Conservative, track-local attribution for speech absent from detector output.
use crate::diarization::{embedding, sherpa::SherpaConfig};
use crate::local_engine::Fingerprint;
use crate::storage::{
    completion_preferences::CompletionPreferences, ExecutionProgress, JobSnapshot, Storage,
};
use crate::voice::{self, Window};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
pub(crate) mod result;

pub(crate) const ALGORITHM: &str = "missing_turn_body_only_v3";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Target {
    pub lineage: String,
    pub body: Window,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resident_runtime: Option<crate::local_engine::resident::Snapshot>,
    pub schema_version: u32,
    pub algorithm: String,
    pub options: CompletionPreferences,
    pub engine: SherpaConfig,
    pub extractor: embedding::ExtractorConfig,
    pub source: crate::storage::RecordedAudioPlan,
    pub known: crate::storage::KnownInput,
    pub working_hash: String,
    pub references: Vec<Window>,
    pub targets: Vec<Target>,
    pub excluded: BTreeMap<String, usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Decision {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_review: bool,
    pub lineage: String,
    pub speaker_id: Option<String>,
    pub body_best: Option<String>,
    pub similarity: Option<f64>,
    pub runner_up: Option<f64>,
    pub reason: String,
}
fn rank(
    query: &[f32],
    references: &BTreeMap<String, Vec<f32>>,
) -> Result<Vec<(String, f64)>, String> {
    let mut scores = references
        .iter()
        .map(|(id, v)| Ok((id.clone(), voice::similarity(query, v)?)))
        .collect::<Result<Vec<_>, String>>()?;
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(scores)
}

// v2 always attributes a usable utterance feature to its nearest body candidate.
// Context and thresholds qualify the review badge, not the selected identity.

pub(crate) fn decide_body_only(
    target: &Target,
    options: CompletionPreferences,
    references: &BTreeMap<String, Vec<f32>>,
    body: Option<&[f32]>,
) -> Result<Decision, String> {
    let mut decision = Decision {
        needs_review: false,
        lineage: target.lineage.clone(),
        speaker_id: None,
        body_best: None,
        similarity: None,
        runner_up: None,
        reason: "feature_unavailable".into(),
    };
    if references.is_empty() {
        decision.reason = "no_reference".into();
        return Ok(decision);
    }
    let Some(body) = body else {
        return Ok(decision);
    };
    let scores = rank(body, references)?;
    let (id, score) = &scores[0];
    let next = scores.get(1).map(|v| v.1);
    let reason = if next == Some(*score) {
        "nearest_tie"
    } else if !options.accepts(*score, next, true) {
        "nearest_below_threshold"
    } else {
        "applied"
    };
    decision.needs_review = reason != "applied";
    decision.speaker_id = Some(id.clone());
    decision.body_best = Some(id.clone());
    decision.similarity = Some(*score);
    decision.runner_up = next;
    decision.reason = reason.into();
    Ok(decision)
}

struct TargetFeatures {
    body: Option<Vec<f32>>,
    context: Option<Vec<f32>>,
    body_elapsed_ms: u64,
    context_elapsed_ms: Option<u64>,
}
fn extract_target_features(
    algorithm: &str,
    target: &Target,
    mut extract: impl FnMut(&Window, &str) -> Result<Option<Vec<f32>>, String>,
) -> Result<TargetFeatures, String> {
    let started = Instant::now();
    let body = extract(&target.body, "body")?;
    let body_elapsed_ms = started.elapsed().as_millis() as u64;
    if algorithm != ALGORITHM {
        return Err("非対応の話者補完方式です".into());
    }
    let context_elapsed_ms = None;
    let context = None;
    Ok(TargetFeatures {
        body,
        context,
        body_elapsed_ms,
        context_elapsed_ms,
    })
}

// Display progress is intentionally not a durable resume checkpoint.
struct ProgressScope<'a>(&'a Storage, &'a str);
impl Drop for ProgressScope<'_> {
    fn drop(&mut self) {
        self.0.clear_completion_progress(self.1);
    }
}

pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, _) = storage.fixed_job_input(job_id)?;
    let plan: Plan =
        serde_json::from_value(spec.parameters["completion"].clone()).map_err(|e| e.to_string())?;
    storage.clear_completion_progress(job_id);
    let _progress_scope = ProgressScope(storage, job_id);
    let started = Instant::now();
    let log_context = crate::internal_logs::Context {
        category: "speaker_completion".into(),
        meeting_id: Some(spec.meeting_id.clone()),
        job_id: Some(job_id.into()),
        operation_id: Some(spec.operation_id.clone()),
    };
    let log = |state: &str, level: &str, value: serde_json::Value| {
        crate::internal_logs::detail(
            log_context.clone(),
            state,
            level,
            started.elapsed(),
            &value.to_string(),
        );
    };
    let total = plan.references.len() + plan.targets.len() + 1;
    let update = |phase: &str,
                  completed: usize,
                  phase_completed: usize,
                  phase_total: usize|
     -> Result<(), String> {
        progress(storage.set_completion_progress(
            job_id,
            ExecutionProgress {
                phase: phase.into(),
                completed,
                total,
                phase_completed,
                phase_total,
            },
        )?);
        Ok(())
    };
    log(
        "starting",
        "info",
        serde_json::json!({"event":"補完開始","algorithm":plan.algorithm,"referenceWindows":plan.references.len(),"contextExtractionEnabled":plan.algorithm!=ALGORITHM,"targetUtterances":plan.targets.len(),"excluded":plan.excluded,"options":plan.options,"embeddingModelSha256":plan.engine.embedding_model.sha256}),
    );
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |j| j.state != "running")
    };
    let result = (|| {
        update("preparing", 0, 0, 0)?;
        storage.check_completion_plan(&spec, &plan)?;
        crate::storage::model_profiles::jobs::verify(
            &spec.profile_snapshot,
            crate::storage::model_profiles::jobs::Execution::Sherpa(&plan.engine),
            &cancelled,
        )?;
        let mut feature_session = crate::speaker_features::Session::new(
            storage,
            &spec.meeting_id,
            &plan.engine,
            &plan.extractor,
            &plan.source,
            plan.resident_runtime.as_ref(),
            log_context.clone(),
            &cancelled,
        )?;
        let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
        update("references", 0, 0, plan.references.len())?;
        let mut vectors: BTreeMap<(String, String), Vec<Vec<f32>>> = BTreeMap::new();
        for (i, w) in plan.references.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            log(
                "running",
                "info",
                serde_json::json!({"event":"参照音声抽出開始","index":i+1,"total":plan.references.len(),"trackId":w.track_id,"speakerId":w.speaker_id,"startUs":w.start_us,"endUs":w.end_us}),
            );
            let unit_started = Instant::now();
            let directory = temp.path().join(format!("ref-{i}"));
            let vector = feature_session.extract(w, &directory, &cancelled)?;
            log(
                "running",
                "info",
                serde_json::json!({"event":"参照音声抽出完了","index":i+1,"ready":vector.is_some(),"elapsedMs":unit_started.elapsed().as_millis()}),
            );
            if let Some(v) = vector {
                vectors
                    .entry((w.track_id.clone(), w.speaker_id.clone()))
                    .or_default()
                    .push(v);
            }
            update("references", i + 1, i + 1, plan.references.len())?;
        }
        let mut references: BTreeMap<String, BTreeMap<String, Vec<f32>>> = BTreeMap::new();
        for ((track, speaker), vs) in vectors {
            if let Ok(c) = voice::centroid(&vs) {
                references.entry(track).or_default().insert(speaker, c);
            }
        }
        log(
            "running",
            "info",
            serde_json::json!({"event":"参照話者準備完了","tracks":references.iter().map(|(track, refs)| (track, refs.len())).collect::<BTreeMap<_,_>>()}),
        );
        update("matching", plan.references.len(), 0, plan.targets.len())?;
        let mut decisions = Vec::new();
        for (i, t) in plan.targets.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let unit_started = Instant::now();
            log(
                "running",
                "info",
                serde_json::json!({"event":"発言照合開始","index":i+1,"total":plan.targets.len(),"lineage":t.lineage,"trackId":t.body.track_id,"bodyStartUs":t.body.start_us,"bodyEndUs":t.body.end_us}),
            );
            let refs = references
                .get(&t.body.track_id)
                .cloned()
                .unwrap_or_default();
            let features = if refs.is_empty() {
                TargetFeatures {
                    body: None,
                    context: None,
                    body_elapsed_ms: 0,
                    context_elapsed_ms: None,
                }
            } else {
                extract_target_features(&plan.algorithm, t, |window, label| {
                    let directory = temp.path().join(format!("{label}-{i}"));
                    feature_session.extract(window, &directory, &cancelled)
                })?
            };
            let body = features.body.as_deref();
            let context = features.context.as_deref();
            let decision = decide_body_only(t, plan.options, &refs, body)?;
            log(
                "running",
                "info",
                serde_json::json!({"event":"発言照合完了","index":i+1,"decision":decision,"margin":decision.similarity.zip(decision.runner_up).map(|(a,b)|a-b),"referenceSpeakers":refs.len(),"bodyFeatureReady":body.is_some(),"contextFeatureReady":context.is_some(),"bodyElapsedMs":features.body_elapsed_ms,"contextElapsedMs":features.context_elapsed_ms,"contextExtractionPerformed":features.context_elapsed_ms.is_some(),"contextBest":context.and_then(|v|rank(v, &refs).ok()).and_then(|scores|scores.into_iter().next()),"elapsedMs":unit_started.elapsed().as_millis()}),
            );
            decisions.push(decision);
            update(
                "matching",
                plan.references.len() + i + 1,
                i + 1,
                plan.targets.len(),
            )?;
        }
        if cancelled() {
            return Ok(None);
        }
        update("saving", total - 1, 0, 1)?;
        log(
            "saving",
            "info",
            serde_json::json!({"event":"補完結果保存開始"}),
        );
        feature_session.finish(&cancelled)?;
        storage.check_completion_plan(&spec, &plan)?;
        plan.engine.verify(&cancelled)?;
        plan.extractor.helper.verify(&cancelled)?;
        plan.extractor.library.verify(&cancelled)?;
        let saved = storage.publish_completion_result(&spec, &plan, &decisions, job_id)?;
        let mut reasons = BTreeMap::<String, usize>::new();
        for d in &decisions {
            *reasons.entry(d.reason.clone()).or_default() += 1;
        }
        log(
            "succeeded",
            "info",
            serde_json::json!({"event":"補完結果保存完了","updatedCount":decisions.iter().filter(|d|d.speaker_id.is_some()).count(),"needsReviewCount":decisions.iter().filter(|d|d.speaker_id.is_some() && d.needs_review).count(),"reasons":reasons,"outputAnalysisId":saved.output_analysis_id}),
        );
        progress(saved);
        Ok(Some(spec.meeting_id.clone()))
    })();
    if storage.get_job(job_id)?.state == "succeeded" {
        return result;
    }
    if cancelled() {
        let j = storage.get_job(job_id)?;
        log(
            &j.state,
            "warn",
            serde_json::json!({"event":"補完処理停止","state":j.state}),
        );
        if matches!(j.state.as_str(), "running" | "pausing") {
            if j.state == "running" {
                storage.request_job_pause(job_id, "exit")?;
            }
            progress(storage.save_job_checkpoint(
                job_id,
                0,
                0,
                serde_json::json!({"completionResult":false}),
            )?);
        }
        return Ok(None);
    }
    if let Err(error) = &result {
        log(
            "failed",
            "error",
            serde_json::json!({"event":"補完処理失敗","error":error}),
        );
    }
    result
}
pub(crate) fn retry(
    storage: &Storage,
    job_id: &str,
    operation: &str,
) -> Result<JobSnapshot, String> {
    let job = storage.get_job(job_id)?;
    let (spec, checkpoint) = storage.fixed_job_input(job_id)?;
    let plan =
        serde_json::from_value(spec.parameters["completion"].clone()).map_err(|e| e.to_string())?;
    storage.check_completion_plan(&spec, &plan)?;
    storage.enqueue_retry_job(&job, &spec, checkpoint.as_ref(), operation)
}

// Only new producer specs opt into automatic completion. Restoring/resuming an
// older analysis never retroactively changes its attribution behavior.
pub(crate) fn after_analysis(storage: &Storage, _meeting: &str) -> Result<(), String> {
    storage.drain_post_analysis(&|| false)
}

#[cfg(test)]
mod nearest_tests {
    use super::*;
    fn target() -> Target {
        Target {
            lineage: "u".into(),
            body: Window {
                speaker_id: String::new(),
                track_id: "t".into(),
                start_us: 1_000_000,
                end_us: 3_000_000,
            },
        }
    }
    #[test]
    fn body_only_extracts_once_and_rejects_previous_algorithms() {
        let t = target();
        let mut count = 0;
        extract_target_features(ALGORITHM, &t, |w, label| {
            assert_eq!(w, &t.body);
            assert_eq!(label, "body");
            count += 1;
            Ok(Some(vec![1., 0.]))
        })
        .unwrap();
        assert_eq!(count, 1);
        for old in [
            "missing_turn_context_cosine_v1",
            "missing_turn_body_nearest_v2",
        ] {
            assert!(extract_target_features(old, &t, |_, _| Ok(None)).is_err());
        }
    }
    #[test]
    fn body_only_review_uses_only_similarity_and_margin() {
        let mut t = target();

        let refs = BTreeMap::from([
            ("a".into(), vec![1., 0., 0.]),
            ("b".into(), vec![0., 1., 0.]),
        ]);
        let p = CompletionPreferences::default();
        let d = decide_body_only(&t, p, &refs, Some(&[1., 0., 0.])).unwrap();
        assert_eq!(d.speaker_id.as_deref(), Some("a"));
        assert!(!d.needs_review);
        assert_eq!(d.reason, "applied");
        let d = decide_body_only(&t, p, &refs, Some(&[0.5, 0.2, 0.84])).unwrap();
        assert!(d.needs_review);
        assert_eq!(d.reason, "nearest_below_threshold");
        assert_eq!(d.speaker_id.as_deref(), Some("a"));
        let d = decide_body_only(&t, p, &refs, Some(&[1., 1., 0.])).unwrap();
        assert!(d.needs_review);
        assert_eq!(d.reason, "nearest_tie");
        assert!(decide_body_only(&t, p, &refs, None)
            .unwrap()
            .speaker_id
            .is_none());
        assert!(
            decide_body_only(&t, p, &BTreeMap::new(), Some(&[1., 0., 0.]))
                .unwrap()
                .speaker_id
                .is_none()
        );
        let one = BTreeMap::from([("a".into(), vec![1., 0., 0.])]);
        assert!(
            !decide_body_only(&t, p, &one, Some(&[1., 0., 0.]))
                .unwrap()
                .needs_review
        );
    }
}
