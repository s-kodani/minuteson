//! One native diarization call over the shared meeting audio. Meeting-local
//! labels never identify a registered person without voice matching.
mod restore;
pub(crate) mod runtime;
pub(crate) mod sensitivity;
// The feature extractor has its own fixed helper identity.
pub(crate) mod embedding;
pub(crate) mod known;
use crate::storage::model_profiles::jobs::{self as managed, Execution};
pub(crate) mod sherpa;
pub(crate) mod timeline;
pub(crate) use restore::restore_saved_results;

use crate::local_engine::Fingerprint;
use crate::storage::{
    AnalysisInput, DiarizationInput, JobKind, JobSnapshot, JobSpec, MinutesJobRequest,
    RecordedAudioPlan, SpeakerInput, SpeakerSegmentInput, Storage, UtteranceInput,
};
use serde::{Deserialize, Serialize};
use sherpa::{Segment, SherpaConfig};
use std::path::PathBuf;
use std::time::Duration;
use timeline::{Attribution, Timeline};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TrackSettings {
    pub track_id: String,
    pub num_speakers: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StartRequest {
    pub input: MinutesJobRequest,
    pub executable_path: PathBuf,
    pub segmentation_model_path: PathBuf,
    pub embedding_model_path: PathBuf,
    pub runtime_library_paths: Vec<PathBuf>,
    pub threads: u32,
    pub profile: String,
    pub tracks: Vec<TrackSettings>,
    pub cluster_threshold_milli: u32,
    pub window_shift_milli: u32,
    pub min_duration_on_ms: u32,
    pub min_duration_off_ms: u32,
    #[serde(default)]
    pub speech_sensitivity_milli: i32,
}

impl StartRequest {
    pub(crate) fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        if self.runtime_library_paths.len() > 16 {
            return Err("同梱ランタイムの指定が多すぎます".into());
        }
        let capture = |path: &PathBuf| Fingerprint::capture(path, &|| false);
        let engine = SherpaConfig {
            executable: capture(&self.executable_path)?,
            segmentation_model: capture(&self.segmentation_model_path)?,
            embedding_model: capture(&self.embedding_model_path)?,
            runtime_libraries: self
                .runtime_library_paths
                .iter()
                .map(capture)
                .collect::<Result<_, _>>()?,
            threads: self.threads,
            cluster_threshold_milli: self.cluster_threshold_milli,
            window_shift_milli: self.window_shift_milli,
            min_duration_on_ms: self.min_duration_on_ms,
            min_duration_off_ms: self.min_duration_off_ms,
            speech_sensitivity_milli: self.speech_sensitivity_milli,
        };
        enqueue(storage, self.input, engine, &self.profile, self.tracks)
    }
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RetryRequest {
    pub meeting_id: String,
    pub job_id: String,
    pub operation_id: String,
    pub expected_spec_hash: String,
}

#[cfg(test)]
impl RetryRequest {
    pub(crate) fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        let job = storage.get_job(&self.job_id)?;
        if job.meeting_id != self.meeting_id
            || job.spec_hash != self.expected_spec_hash
            || job.kind != "diarization"
            || !matches!(job.state.as_str(), "failed" | "cancelled")
            || job.attempt >= 1000
        {
            return Err("確認した失敗・取消済みの話者分離と固定条件が一致しません".into());
        }
        let (spec, raw) = storage.fixed_job_input(&job.id)?;
        retry_preflight(
            storage,
            &job,
            &spec,
            raw.as_ref(),
            &crate::test_support::never_cancelled,
        )?;
        storage.enqueue_retry_job(&job, &spec, raw.as_ref(), &self.operation_id)
    }
}

#[cfg(test)]
pub(crate) fn retry_preflight(
    storage: &Storage,
    job: &JobSnapshot,
    spec: &JobSpec,
    raw: Option<&serde_json::Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let parameters = checked_parameters(storage, spec, &job.id)?;
    let optimization =
        crate::processing_optimization::Scope::enter(parameters.optimization_version)?;
    let checkpoint = checked_checkpoint(job, raw.cloned())?;
    preflight(storage, spec, &parameters, cancelled)?;
    for output in &checkpoint.outputs {
        load_output(storage, spec, &parameters, output, cancelled)?;
    }
    if cancelled() {
        return Err("話者分離再試行の照合を中断しました".into());
    }
    managed::verify(
        &spec.profile_snapshot,
        Execution::Sherpa(&parameters.engine),
        cancelled,
    )?;
    optimization.finish(cancelled)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    optimization_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resident_runtime: Option<crate::local_engine::resident::Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attribution_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_after: Option<bool>,
    source: RecordedAudioPlan,
    engine: SherpaConfig,
    tracks: Vec<TrackSettings>,
    manual_assignment_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    known_input: Option<crate::storage::KnownInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matching: Option<known::Policy>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope {
    schema_version: u32,
    request: MinutesJobRequest,
    diarization: Parameters,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    outputs: Vec<UnitOutput>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UnitOutput {
    index: usize,
    wav_asset_id: String,
    wav_hash: String,
    stdout_asset_id: String,
    stdout_hash: String,
    result_asset_id: String,
    result_hash: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ResultFile {
    schema_version: u32,
    origin_operation_id: String,
    input_analysis_id: String,
    input_hash: String,
    source_manifest_hash: String,
    track_id: String,
    track_start_us: i64,
    duration_us: i64,
    engine: SherpaConfig,
    num_speakers: Option<u32>,
    segments: Vec<Segment>,
    #[serde(default)]
    speaker_merges: Vec<sherpa::SpeakerMerge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<crate::storage::KnownRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matching: Option<known::Policy>,
}

pub(crate) fn restore_checkpoint_references(
    job: &JobSnapshot,
    value: serde_json::Value,
    mut map: impl FnMut(crate::saved_artifact::Role, &str, &str) -> Result<(String, String), String>,
) -> Result<serde_json::Value, String> {
    use crate::saved_artifact::Role;
    let mut checkpoint = checked_checkpoint(job, Some(value))?;
    if job
        .total_units
        .is_some_and(|total| checkpoint.outputs.len() as i64 > total)
    {
        return Err("再開情報が固定した総トラック数を超えています".into());
    }
    for output in &mut checkpoint.outputs {
        (output.wav_asset_id, output.wav_hash) =
            map(Role::TemporaryPcm, &output.wav_asset_id, &output.wav_hash)?;
        (output.stdout_asset_id, output.stdout_hash) = map(
            Role::DiarizationLog,
            &output.stdout_asset_id,
            &output.stdout_hash,
        )?;
        (output.result_asset_id, output.result_hash) = map(
            Role::DiarizationResult,
            &output.result_asset_id,
            &output.result_hash,
        )?;
    }
    serde_json::to_value(checkpoint).map_err(|e| e.to_string())
}

/// Rebase structured results, not native stdout logs or model availability.
pub(crate) fn restore_result(
    bytes: &[u8],
    old: &JobSpec,
    new: &JobSpec,
) -> Result<Vec<u8>, String> {
    known::restore_result(bytes, old, new)
}

pub(crate) fn enqueue(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: SherpaConfig,
    profile: &str,
    tracks: Vec<TrackSettings>,
) -> Result<JobSnapshot, String> {
    enqueue_source(storage, request, engine, profile, tracks, None)
}

pub(crate) fn enqueue_source(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: SherpaConfig,
    profile: &str,
    tracks: Vec<TrackSettings>,
    transcription_job: Option<&str>,
) -> Result<JobSnapshot, String> {
    let (source, known) = storage.recorded_analysis_input(&request.meeting_id)?;
    #[cfg(test)]
    let fixture_helper = engine
        .executable
        .path
        .ancestors()
        .skip(1)
        .take(5)
        .map(|p| p.join("test-embedding-helper"))
        .find(|p| p.is_file());
    #[cfg(not(test))]
    let fixture_helper: Option<PathBuf> = None;
    known::enqueue(
        storage,
        request,
        engine,
        profile,
        tracks,
        transcription_job,
        source,
        known.ok_or("保存区間がありません")?,
        fixture_helper.as_deref(),
    )
}
fn completion_after(
    storage: &Storage,
    meeting: &str,
    operation: &str,
) -> Result<Option<bool>, String> {
    if let Some(job) = storage
        .list_jobs(meeting)?
        .iter()
        .find(|j| j.operation_id == operation)
    {
        let (spec, _) = storage.fixed_job_input(&job.id)?;
        return spec.parameters["diarization"]
            .get("completionAfter")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string());
    }
    Ok(Some(true))
}
fn attribution_policy(
    storage: &Storage,
    meeting: &str,
    operation: &str,
) -> Result<Option<String>, String> {
    if let Some(job) = storage
        .list_jobs(meeting)?
        .iter()
        .find(|j| j.operation_id == operation)
    {
        let (spec, _) = storage.fixed_job_input(&job.id)?;
        return spec.parameters["diarization"]
            .get("attributionPolicy")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string());
    }
    Ok(Some("all_detected_v1".into()))
}
fn validate_attribution_policy(p: &Parameters) -> Result<(), String> {
    if p.resident_runtime.is_none()
        || p.optimization_version != Some(crate::current_formats::OPTIMIZATION)
    {
        return Err("未対応の処理最適化です".into());
    }
    if p.attribution_policy.as_deref() != Some("all_detected_v1") {
        return Err("話者割り当ての固定条件が不正です".into());
    }
    Ok(())
}
fn checked_parameters(storage: &Storage, spec: &JobSpec, job: &str) -> Result<Parameters, String> {
    known::checked_parameters(storage, spec, job)
}
fn checked_checkpoint(
    job: &JobSnapshot,
    raw: Option<serde_json::Value>,
) -> Result<Checkpoint, String> {
    let checkpoint: Checkpoint = raw
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if checkpoint.outputs.len() as i64 != job.completed_units
        || checkpoint
            .outputs
            .iter()
            .enumerate()
            .any(|(i, o)| i != o.index)
    {
        return Err("話者分離の完了位置と再開情報が一致しません".into());
    }
    Ok(checkpoint)
}
fn preflight(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<crate::playback::PreparedAudio>, String> {
    known::preflight(storage, spec, parameters, cancelled)
}
fn load_output(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    output: &UnitOutput,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Segment>, String> {
    known::load_output(storage, spec, parameters, output, cancelled).map(|r| r.segments)
}

#[cfg(test)]
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    process_observed(storage, job_id, stopping, progress, &|_| {})
}

pub(crate) fn process_observed(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
    runtime_progress: &impl Fn(runtime::Progress),
) -> Result<Option<String>, String> {
    known::process_observed(storage, job_id, stopping, progress, runtime_progress)
}
