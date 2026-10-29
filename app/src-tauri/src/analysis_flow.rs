//! Durable automatic transcription and speaker processing. The shared inference
//! worker owns this coordinator so recording priority and a single CLI remain.
use crate::diarization::sherpa::SherpaConfig;
use crate::local_engine::Fingerprint;
use crate::storage::model_profiles::jobs::{Execution, Snapshot};
use crate::storage::{analysis_flows::AnalysisFlow, MinutesJobRequest, Storage};
use crate::transcription::asr::AsrConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Plan {
    schema_version: u32,
    pub(crate) profile: String,
    pub(crate) whisper: AsrConfig,
    pub(crate) sherpa: SherpaConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) whisper_managed: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sherpa_managed: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) replacement: Option<crate::storage::analysis_flows::regeneration::Target>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) speaker_counts: Vec<crate::diarization::TrackSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) speaker_counts_manifest: Option<String>,
}

pub(crate) fn checked_speaker_counts(
    values: &[crate::diarization::TrackSettings],
) -> Result<Vec<crate::diarization::TrackSettings>, String> {
    if values.len() > 1
        || values.iter().any(|v| {
            v.track_id.is_empty()
                || v.track_id.len() > 256
                || v.track_id.contains('\0')
                || v.num_speakers
                    .is_some_and(|n| !(1..=crate::diarization::sherpa::MAX_SPEAKERS).contains(&n))
        })
    {
        return Err("会議全体の話者数は1〜100人、または自動で指定してください".into());
    }
    let mut result = values.to_vec();
    result.sort_by(|a, b| a.track_id.cmp(&b.track_id));
    if result.windows(2).any(|w| w[0].track_id == w[1].track_id) {
        return Err("話者数を指定する音声が重複しています".into());
    }
    result.retain(|v| v.num_speakers.is_some());
    Ok(result)
}

// Pure saved-plan decoding: never checks model availability or starts a CLI.
pub(crate) fn saved_plan(value: serde_json::Value, profile: &str) -> Result<Plan, String> {
    let plan: Plan = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if plan.schema_version != 1
        || plan.profile != profile
        || !matches!(profile, "standard" | "high_accuracy")
        || !plan.whisper.prompt.is_empty()
    {
        return Err("未対応の工程条件です".into());
    }
    plan.whisper.validate_settings()?;
    plan.sherpa.validate_settings(None)?;
    if checked_speaker_counts(&plan.speaker_counts)? != plan.speaker_counts
        || plan.speaker_counts.is_empty() != plan.speaker_counts_manifest.is_none()
        || plan.speaker_counts_manifest.as_ref().is_some_and(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err("固定した話者数と保存音声の条件が不正です".into());
    }
    if let Some(target) = &plan.replacement {
        target.validate()?;
    }
    if let Some(s) = &plan.whisper_managed {
        s.validate(profile, &Execution::Asr(&plan.whisper))?;
    }
    if let Some(s) = &plan.sherpa_managed {
        s.validate(profile, &Execution::Sherpa(&plan.sherpa))?;
    }
    Ok(plan)
}
pub(crate) fn prepare(
    storage: &Storage,
    profile: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Plan, String> {
    let verification = crate::local_engine::verification::Scope::join_or_enter(true)?;
    let w = storage.analysis_preferences()?.payload;
    let d = storage.diarization_preferences()?.payload;
    let (w, d) = match profile {
        "standard" => (w.standard, d.standard),
        "high_accuracy" => (w.high_accuracy, d.high_accuracy),
        _ => return Err("未対応の解析プロファイルです".into()),
    };
    // Empty defaults mean no model has been selected. Explain the actionable
    // prerequisite before fingerprinting paths; configured models still pass
    // all identity, file, compatibility and managed-selection verification.
    if w.executable_path.is_empty() || w.model_path.is_empty() {
        return Err("文字起こしモデルが未導入です。設定のローカルAIでモデルを選び、ダウンロードして使用してください。保存済み音声は保持しています".into());
    }
    if d.executable_path.is_empty()
        || d.segmentation_model_path.is_empty()
        || d.embedding_model_path.is_empty()
    {
        return Err("話者分離モデルが未導入です。設定のローカルAIでモデルを選び、ダウンロードして使用してください。保存済み音声は保持しています".into());
    }
    let capture = |path: &str| Fingerprint::capture_retained(std::path::Path::new(path), cancelled);
    let whisper = AsrConfig {
        executable: capture(&w.executable_path)?,
        model: capture(&w.model_path)?,
        vad_model: w.vad_model_path.as_deref().map(capture).transpose()?,
        vad_options: w.vad_options,
        threads: w.threads,
        use_gpu: w.use_gpu,
        prompt: String::new(),
        backend: w
            .backend
            .as_ref()
            .map(|b| b.capture(cancelled))
            .transpose()?,
    };
    let sherpa = d.capture(crate::diarization::sherpa::CaptureMode::Retained, cancelled)?;
    whisper.verify(cancelled)?;
    sherpa.verify(cancelled)?;
    let whisper_managed =
        storage.prepare_model_selection(profile, Execution::Asr(&whisper), cancelled)?;
    let sherpa_managed =
        storage.prepare_model_selection(profile, Execution::Sherpa(&sherpa), cancelled)?;
    verification.finish(cancelled)?;
    Ok(Plan {
        schema_version: 1,
        profile: profile.into(),
        whisper,
        sherpa,
        whisper_managed,
        sherpa_managed,
        replacement: None,
        speaker_counts: vec![],
        speaker_counts_manifest: None,
    })
}

pub(crate) fn advance(
    storage: &Storage,
    cancelled: &impl Fn() -> bool,
    changed: &impl Fn(AnalysisFlow),
) -> Result<(), String> {
    for mut flow in storage.pending_analysis_flows()? {
        let _meeting_owner = match storage.begin_meeting_work(&flow.meeting_id, "job", None) {
            Ok(owner) => owner,
            Err(_) => continue,
        };
        let _verification = crate::local_engine::verification::Scope::join_or_enter(true)?;
        let _audio_verification = crate::audio::reuse::Scope::join_or_enter()?;
        let _preparing = (flow.stage == "pending"
            || flow.stage == "transcription" && flow.transcription_job.is_none())
        .then(|| crate::analysis_preparation::begin(&flow.meeting_id, &flow.id));
        for _ in 0..4 {
            if cancelled() {
                return Ok(());
            }
            match step(storage, &flow, cancelled) {
                Ok(Some(next)) => {
                    changed(next.clone());
                    flow = next;
                }
                Ok(None) => break,
                Err(error) => {
                    if cancelled() {
                        return Ok(());
                    }
                    let stage = if flow.plan.is_none() {
                        "configuration"
                    } else {
                        "attention"
                    };
                    let next = storage.update_analysis_flow(
                        &flow,
                        stage,
                        flow.plan.clone(),
                        flow.transcription_job.as_ref(),
                        flow.diarization_job.as_ref(),
                        Some(&error),
                    )?;
                    changed(next);
                    break;
                }
            }
        }
    }
    Ok(())
}
fn step(
    storage: &Storage,
    flow: &AnalysisFlow,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<AnalysisFlow>, String> {
    let save = |stage, plan, trans, diar| {
        storage
            .update_analysis_flow(flow, stage, plan, trans, diar, None)
            .map(Some)
    };
    if flow.stage == "pending" {
        storage.ensure_meeting_mix(&flow.meeting_id, cancelled, &|done, total| {
            crate::analysis_preparation::update(
                &flow.meeting_id,
                &flow.id,
                "mix",
                Some(done),
                Some(total),
            )
        })?;
        crate::analysis_preparation::update(&flow.meeting_id, &flow.id, "verifying", None, None);
        let mut plan = prepare(storage, &flow.profile, cancelled)?;
        plan.replacement = storage.pending_analysis_target(&flow.id)?;
        let input = storage.recorded_analysis_input(&flow.meeting_id)?.0;
        plan.speaker_counts = storage.initial_speaker_counts(&flow.meeting_id, &input.tracks)?;
        if !plan.speaker_counts.is_empty() {
            plan.speaker_counts_manifest = Some(input.manifest_hash);
        }
        return save(
            "transcription",
            Some(serde_json::to_value(plan).map_err(|e| e.to_string())?),
            None,
            None,
        );
    }
    let plan = saved_plan(
        flow.plan.clone().ok_or("固定条件がありません")?,
        &flow.profile,
    )?;
    match flow.stage.as_str() {
        "transcription" => {
            if let Some(job) = &flow.transcription_job {
                if matches!(job.state.as_str(), "failed" | "cancelled") {
                    return save("attention", flow.plan.clone(), Some(job), None);
                }
                if job.state == "succeeded" {
                    return save("diarization", flow.plan.clone(), Some(job), None);
                }
            } else {
                let operation = format!("flow:{}:transcription", flow.id);
                let job = if let Some(job) = storage.flow_job_by_operation(&operation)? {
                    job
                } else {
                    crate::transcription::enqueue_with_stamp(
                        storage,
                        &flow.meeting_id,
                        &operation,
                        plan.whisper,
                        &plan.profile,
                        None,
                    )?
                };
                return save("transcription", flow.plan.clone(), Some(&job), None);
            }
        }
        "diarization" => {
            if let Some(job) = &flow.diarization_job {
                if matches!(job.state.as_str(), "failed" | "cancelled") {
                    return save(
                        "attention",
                        flow.plan.clone(),
                        flow.transcription_job.as_ref(),
                        Some(job),
                    );
                }
                if job.state == "succeeded" {
                    // Regeneration waits for the main window's edit barrier.
                    // It flushes even an open text field before the native CAS;
                    // later human edits therefore win without being discarded.
                    if plan.replacement.is_some() {
                        return Ok(None);
                    }
                    return storage.complete_analysis_flow(flow).map(Some);
                }
            } else {
                let trans = flow
                    .transcription_job
                    .as_ref()
                    .ok_or("成功した文字起こしがありません")?;
                let id = trans
                    .output_analysis_id
                    .as_deref()
                    .ok_or("文字起こし出力がありません")?;
                let source = storage
                    .load_visible_analysis(&flow.meeting_id, id)?
                    .ok_or("文字起こし候補がありません")?;
                let operation = format!("flow:{}:diarization", flow.id);
                let job = if let Some(job) = storage.flow_job_by_operation(&operation)? {
                    job
                } else {
                    let request = MinutesJobRequest {
                        meeting_id: flow.meeting_id.clone(),
                        operation_id: operation,
                        expected_analysis_id: source.id,
                        expected_content_hash: source.content_hash,
                        expected_meeting_lock_version: storage
                            .get_meeting(&flow.meeting_id)?
                            .ok_or("会議がありません")?
                            .lock_version,
                        expected_draft: None,
                    };
                    let input = storage.recorded_analysis_input(&flow.meeting_id)?.0;
                    if plan
                        .speaker_counts_manifest
                        .as_ref()
                        .is_some_and(|hash| hash != &input.manifest_hash)
                        || plan
                            .speaker_counts
                            .iter()
                            .any(|count| !input.tracks.iter().any(|t| t.id == count.track_id))
                    {
                        return Err(
                            "話者数を指定した後に保存音声が変わりました。生成し直してください"
                                .into(),
                        );
                    }
                    let tracks = input
                        .tracks
                        .iter()
                        .map(|t| crate::diarization::TrackSettings {
                            track_id: t.id.clone(),
                            num_speakers: plan
                                .speaker_counts
                                .iter()
                                .find(|count| count.track_id == t.id)
                                .and_then(|count| count.num_speakers),
                        })
                        .collect();
                    crate::diarization::enqueue_source(
                        storage,
                        request,
                        plan.sherpa,
                        &plan.profile,
                        tracks,
                        Some(&trans.id),
                    )?
                };
                return save("diarization", flow.plan.clone(), Some(trans), Some(&job));
            }
        }
        _ => {}
    }
    Ok(None)
}

// Private-data verification must never advance another meeting's pending work.
#[cfg(test)]
pub(crate) fn advance_selected_for_test(
    storage: &Storage,
    meeting: &str,
) -> Result<AnalysisFlow, String> {
    let mut flow = storage
        .analysis_flow(meeting)?
        .ok_or("対象の解析工程がありません")?;
    for _ in 0..4 {
        match step(storage, &flow, &|| false)? {
            Some(next) => flow = next,
            None => break,
        }
    }
    Ok(flow)
}

pub(crate) fn continue_flow(
    storage: &Storage,
    meeting: &str,
    expected: i64,
) -> Result<AnalysisFlow, String> {
    let flow = storage
        .analysis_flow(meeting)?
        .ok_or("解析工程がありません")?;
    if flow.lock_version != expected {
        return Err("確認後に解析工程が変化しています".into());
    }
    let stage = match flow.stage.as_str() {
        "configuration" => "pending",
        "attention" => {
            if flow
                .transcription_job
                .as_ref()
                .is_some_and(|j| j.state == "succeeded")
            {
                "diarization"
            } else {
                "transcription"
            }
        }
        _ => return Err("準備待ちまたは引継ぎ失敗だけ再開できます".into()),
    };
    storage.update_analysis_flow(
        &flow,
        stage,
        flow.plan.clone(),
        flow.transcription_job.as_ref(),
        flow.diarization_job.as_ref(),
        None,
    )
}
