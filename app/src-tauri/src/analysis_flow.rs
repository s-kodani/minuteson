//! Durable automatic handoffs, not automatic adoption. The shared inference
//! worker owns this coordinator so recording priority and a single CLI remain.
use crate::diarization::sherpa::SherpaConfig;
use crate::local_engine::Fingerprint;
use crate::storage::{analysis_flows::AnalysisFlow, MinutesJobRequest, Storage};
use crate::transcription::whisper::WhisperConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Plan {
    schema_version: u32,
    profile: String,
    whisper: WhisperConfig,
    sherpa: SherpaConfig,
}
fn prepare(
    storage: &Storage,
    profile: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Plan, String> {
    let w = storage.analysis_preferences()?.payload;
    let d = storage.diarization_preferences()?.payload;
    let (w, d) = match profile {
        "standard" => (w.standard, d.standard),
        "high_accuracy" => (w.high_accuracy, d.high_accuracy),
        _ => return Err("未対応の解析プロファイルです".into()),
    };
    let capture = |path: &str| Fingerprint::capture(std::path::Path::new(path), cancelled);
    let whisper = WhisperConfig {
        executable: capture(&w.executable_path)?,
        model: capture(&w.model_path)?,
        vad_model: w.vad_model_path.as_deref().map(capture).transpose()?,
        threads: w.threads,
        use_gpu: w.use_gpu,
        prompt: String::new(),
    };
    let sherpa = SherpaConfig {
        executable: capture(&d.executable_path)?,
        segmentation_model: capture(&d.segmentation_model_path)?,
        embedding_model: capture(&d.embedding_model_path)?,
        runtime_libraries: d
            .runtime_library_paths
            .iter()
            .map(|p| capture(p))
            .collect::<Result<_, _>>()?,
        threads: d.threads,
        cluster_threshold_milli: d.cluster_threshold_milli,
        window_shift_milli: d.window_shift_milli,
        min_duration_on_ms: d.min_duration_on_ms,
        min_duration_off_ms: d.min_duration_off_ms,
    };
    whisper.verify(cancelled)?;
    sherpa.verify(cancelled)?;
    Ok(Plan {
        schema_version: 1,
        profile: profile.into(),
        whisper,
        sherpa,
    })
}

pub(crate) fn advance(
    storage: &Storage,
    cancelled: &impl Fn() -> bool,
    changed: &impl Fn(AnalysisFlow),
) -> Result<(), String> {
    for mut flow in storage.pending_analysis_flows()? {
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
        let plan = prepare(storage, &flow.profile, cancelled)?;
        return save(
            "transcription",
            Some(serde_json::to_value(plan).map_err(|e| e.to_string())?),
            None,
            None,
        );
    }
    let plan: Plan = serde_json::from_value(flow.plan.clone().ok_or("固定条件がありません")?)
        .map_err(|e| e.to_string())?;
    if plan.schema_version != 1
        || plan.profile != flow.profile
        || !matches!(plan.profile.as_str(), "standard" | "high_accuracy")
    {
        return Err("未対応の工程条件です".into());
    }
    match flow.stage.as_str() {
        "transcription" => {
            if let Some(job) = &flow.transcription_job {
                if matches!(job.state.as_str(), "failed" | "cancelled") {
                    if let Some(next) = storage.flow_retry(job)? {
                        return save("transcription", flow.plan.clone(), Some(&next), None);
                    }
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
                    if let Some(next) = storage.flow_retry(job)? {
                        return save(
                            "diarization",
                            flow.plan.clone(),
                            flow.transcription_job.as_ref(),
                            Some(&next),
                        );
                    }
                }
                if job.state == "succeeded" {
                    return save(
                        "review",
                        flow.plan.clone(),
                        flow.transcription_job.as_ref(),
                        Some(job),
                    );
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
                    let tracks = storage
                        .recorded_audio_plan(&flow.meeting_id)?
                        .tracks
                        .iter()
                        .map(|t| crate::diarization::TrackSettings {
                            track_id: t.id.clone(),
                            num_speakers: None,
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
