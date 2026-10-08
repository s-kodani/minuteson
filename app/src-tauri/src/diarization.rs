//! Whole-track native diarization. Cluster IDs are local to one track/run;
//! they are never treated as voice identities or shared across meetings.
mod restore;
// The gap-aware worker will consume this pure, track-local feature reducer.
// It does not change the legacy whole-track worker's immutable conditions.
#[allow(dead_code)]
mod clustering;
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
use sherpa::{Segment, SherpaConfig, MAX_SPEAKERS};
use std::fs::OpenOptions;
use std::io::Write;
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
        };
        enqueue(storage, self.input, engine, &self.profile, self.tracks)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RetryRequest {
    pub meeting_id: String,
    pub job_id: String,
    pub operation_id: String,
    pub expected_spec_hash: String,
}

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
        retry_preflight(storage, &job, &spec, raw.as_ref(), &|| false)?;
        storage.enqueue_retry_job(&job, &spec, raw.as_ref(), &self.operation_id)
    }
}

pub(crate) fn retry_preflight(
    storage: &Storage,
    job: &JobSnapshot,
    spec: &JobSpec,
    raw: Option<&serde_json::Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let parameters = checked_parameters(storage, spec, &job.id)?;
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
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    source: RecordedAudioPlan,
    engine: SherpaConfig,
    tracks: Vec<TrackSettings>,
    manual_assignment_hash: String,
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
#[derive(Debug, Deserialize, Serialize)]
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
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("話者分離結果が容量上限を超えています".into());
    }
    let mut result: ResultFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let old_envelope: Envelope =
        serde_json::from_value(old.parameters.clone()).map_err(|e| e.to_string())?;
    let new_envelope: Envelope =
        serde_json::from_value(new.parameters.clone()).map_err(|e| e.to_string())?;
    let index = old_envelope
        .diarization
        .source
        .tracks
        .iter()
        .position(|t| t.id == result.track_id)
        .ok_or("保存結果のトラックがありません")?;
    let old_track = &old_envelope.diarization.source.tracks[index];
    let new_track = new_envelope
        .diarization
        .source
        .tracks
        .get(index)
        .ok_or("再構成後のトラックがありません")?;
    let native_duration = duration(&old_envelope.diarization.source, index)?;
    if old.kind != JobKind::Diarization
        || new.kind != JobKind::Diarization
        || old_envelope.schema_version != 1
        || new_envelope.schema_version != 1
        || result.schema_version != 1
        || result.origin_operation_id != old_envelope.request.operation_id
        || Some(result.input_analysis_id.as_str()) != old.input_analysis_id.as_deref()
        || result.input_hash != old.input_hash
        || result.source_manifest_hash != old_envelope.diarization.source.manifest_hash
        || result.track_start_us != old_track.start_us
        || result.duration_us != native_duration
        || result.engine != old_envelope.diarization.engine
        || result.num_speakers
            != old_envelope
                .diarization
                .tracks
                .get(index)
                .ok_or("話者設定がありません")?
                .num_speakers
        || result.engine != new_envelope.diarization.engine
        || old_track.start_us != new_track.start_us
        || native_duration != duration(&new_envelope.diarization.source, index)?
        || result.num_speakers
            != new_envelope
                .diarization
                .tracks
                .get(index)
                .ok_or("再構成後の話者設定がありません")?
                .num_speakers
    {
        return Err("保存済みの話者分離結果の由来・固定設定が一致しません".into());
    }
    sherpa::validate_saved_segments(&result.segments, result.num_speakers, native_duration)?;
    result.origin_operation_id = new_envelope.request.operation_id;
    result.input_analysis_id = new
        .input_analysis_id
        .clone()
        .ok_or("再構成後の固定入力がありません")?;
    result.input_hash = new.input_hash.clone();
    result.source_manifest_hash = new_envelope.diarization.source.manifest_hash;
    result.track_id = new_track.id.clone();
    let bytes = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("再構成した話者分離結果が容量上限を超えています".into());
    }
    Ok(bytes)
}

fn track_frames(source: &RecordedAudioPlan, index: usize) -> Result<u64, String> {
    let track = &source.tracks[index];
    let frames = track.total_frames()?;
    if frames == 0
        || !(8_000..=384_000).contains(&track.sample_rate)
        || !(1..=8).contains(&track.channels)
        || track.end_us <= track.start_us
        || track.start_us < 0
    {
        return Err("話者分離の入力形式・時間が不正です".into());
    }
    Ok(frames)
}
fn duration(source: &RecordedAudioPlan, index: usize) -> Result<i64, String> {
    let frames = track_frames(source, index)?;
    let rate = u64::from(source.tracks[index].sample_rate);
    let converted = frames
        .checked_mul(16_000)
        .and_then(|n| n.checked_add(rate / 2))
        .ok_or("音声時間が範囲外です")?
        / rate;
    if converted == 0 || converted > u64::from((u32::MAX - 36) / 2) {
        return Err("話者分離用WAVのサイズ上限を超えています".into());
    }
    Ok((converted * 1_000_000 / 16_000) as i64)
}
fn validate_tracks(source: &RecordedAudioPlan, tracks: &[TrackSettings]) -> Result<(), String> {
    if source.tracks.is_empty() || source.tracks.len() > 2 || tracks.len() != source.tracks.len() {
        return Err("保存済みの全トラックに人数の適用条件が必要です".into());
    }
    for (index, (track, settings)) in source.tracks.iter().zip(tracks).enumerate() {
        if track.id != settings.track_id
            || settings
                .num_speakers
                .is_some_and(|n| !(1..=MAX_SPEAKERS).contains(&n))
        {
            return Err("人数の適用トラックまたは人数が不正です".into());
        }
        duration(source, index)?;
    }
    Ok(())
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
    if !matches!(profile, "standard" | "high_accuracy") {
        return Err("話者分離のプロファイルが不正です".into());
    }
    engine.verify(&|| false)?;
    let selected = storage.job_model_selection(
        &request.meeting_id,
        &request.operation_id,
        profile,
        Execution::Sherpa(&engine),
    )?;
    let source = storage.recorded_audio_plan(&request.meeting_id)?;
    storage.verify_meeting_asset(
        &request.meeting_id,
        &source.manifest_asset_id,
        &source.manifest_hash,
    )?;
    validate_tracks(&source, &tracks)?;
    let profile = managed::attach(
        serde_json::json!({"profile":profile,"engine":"sherpa-onnx","segmentationSha256":engine.segmentation_model.sha256,"embeddingSha256":engine.embedding_model.sha256,"qualityStatus":"unverified"}),
        selected,
    )?;
    let configure = move |input: &DiarizationInput| -> Result<(serde_json::Value, i64), String> {
        if input.audio_manifest_hash != source.manifest_hash {
            return Err("文字起こしと保存音声の由来が一致しません".into());
        }
        for utterance in &input.analysis.utterances {
            if !source.tracks.iter().any(|t| {
                t.id == utterance.track_id
                    && utterance.start_us >= t.start_us
                    && utterance.end_us <= t.end_us
            }) {
                return Err("固定した発言の音声トラック・時刻が一致しません".into());
            }
        }
        let parameters = Parameters {
            source,
            engine,
            tracks,
            manual_assignment_hash: input.manual_assignment_hash.clone(),
        };
        let total = parameters.tracks.len() as i64;
        Ok((
            serde_json::to_value(parameters).map_err(|e| e.to_string())?,
            total,
        ))
    };
    if transcription_job.is_some() {
        storage.enqueue_diarization_source(request, profile, transcription_job, configure)
    } else {
        storage.enqueue_diarization_job(request, profile, configure)
    }
}
fn checked_parameters(storage: &Storage, spec: &JobSpec, job: &str) -> Result<Parameters, String> {
    let envelope: Envelope =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    let input = storage.load_diarization_input(job)?;
    let parameters = envelope.diarization;
    if spec.kind != JobKind::Diarization
        || envelope.schema_version != 1
        || envelope.request.meeting_id != spec.meeting_id
        || envelope.request.operation_id != storage.fixed_job_initial_operation(job)?
        || parameters.source.manifest_hash != input.audio_manifest_hash
        || parameters.manual_assignment_hash != input.manual_assignment_hash
        || spec.total_units != Some(parameters.tracks.len() as i64)
    {
        return Err("話者分離の固定入力計画が不正です".into());
    }
    validate_tracks(&parameters.source, &parameters.tracks)?;
    managed::validate(
        &spec.profile_snapshot,
        Execution::Sherpa(&parameters.engine),
    )?;
    Ok(parameters)
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
    managed::rebinding::sherpa(&spec.profile_snapshot, &parameters.engine)?.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Sherpa(&parameters.engine),
        cancelled,
    )?;
    if storage.recorded_audio_plan(&spec.meeting_id)? != parameters.source {
        return Err("固定した録音入力が変更されています".into());
    }
    storage.verify_meeting_asset(
        &spec.meeting_id,
        &parameters.source.manifest_asset_id,
        &parameters.source.manifest_hash,
    )?;
    let mut prepared = Vec::new();
    for (index, track) in parameters.source.tracks.iter().enumerate() {
        let audio = storage.prepare_track_audio_cancellable(
            &spec.meeting_id,
            &track.id,
            track.start_us,
            track.end_us,
            cancelled,
        )?;
        if audio.total_frames() != track_frames(&parameters.source, index)?
            || audio.sample_rate != track.sample_rate
            || audio.channels != track.channels
        {
            return Err("固定した音声形式・フレーム数が変更されています".into());
        }
        prepared.push(audio);
    }
    if storage.recorded_audio_plan(&spec.meeting_id)? != parameters.source {
        return Err("固定した録音入力・時刻補正が音声準備中に変更されています".into());
    }
    Ok(prepared)
}
fn load_output(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    output: &UnitOutput,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Segment>, String> {
    if cancelled() {
        return Err("話者分離の照合を中断しました".into());
    }
    let track = parameters
        .source
        .tracks
        .get(output.index)
        .ok_or("再開トラックの位置が不正です")?;
    crate::restored_processing::verify_pcm(
        storage,
        &spec.meeting_id,
        &output.wav_asset_id,
        &output.wav_hash,
        cancelled,
        |path| {
            let mut audio = storage.prepare_track_audio_cancellable(
                &spec.meeting_id,
                &track.id,
                track.start_us,
                track.end_us,
                cancelled,
            )?;
            crate::transcription::wave::write_track(&mut audio, path, cancelled).map(|_| ())
        },
    )?;
    let bytes = storage.read_analysis_result(
        &spec.meeting_id,
        &output.result_asset_id,
        &output.result_hash,
    )?;
    let result: ResultFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let native_duration = duration(&parameters.source, output.index)?;
    if result.schema_version != 1
        || result.origin_operation_id != spec.parameters["request"]["operationId"]
        || Some(result.input_analysis_id.as_str()) != spec.input_analysis_id.as_deref()
        || result.input_hash != spec.input_hash
        || result.source_manifest_hash != parameters.source.manifest_hash
        || result.track_id != track.id
        || result.track_start_us != track.start_us
        || result.duration_us != native_duration
        || result.engine != parameters.engine
        || result.num_speakers != parameters.tracks[output.index].num_speakers
    {
        return Err("保存済みの話者分離結果の由来・固定設定が一致しません".into());
    }
    sherpa::validate_saved_segments(&result.segments, result.num_speakers, native_duration)?;
    if crate::storage::backup::omitted_processing_asset(
        storage,
        &spec.meeting_id,
        &output.stdout_asset_id,
        &output.stdout_hash,
        crate::saved_artifact::Role::DiarizationLog,
    )?
    .is_some()
    {
        // This canonical result was validated by additive restore. The raw
        // native log was deliberately omitted; never fabricate it or mark it
        // available. Validate the durable result's provenance and semantics.
        return Ok(result.segments);
    }
    let stdout = storage.read_inference_artifact(
        &spec.meeting_id,
        &output.stdout_asset_id,
        &output.stdout_hash,
        "text/plain",
        16 * 1024 * 1024,
    )?;
    let segments = sherpa::parse_output(
        &stdout,
        &parameters.engine,
        result.num_speakers,
        native_duration,
    )?;
    if segments != result.segments {
        return Err("話者分離の保存結果とエンジン出力が一致しません".into());
    }
    Ok(segments)
}

pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, raw) = storage.fixed_job_input(job_id)?;
    let parameters = checked_parameters(storage, &spec, job_id)?;
    let job = storage.get_job(job_id)?;
    let mut checkpoint = checked_checkpoint(&job, raw)?;
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |j| j.state != "running")
    };
    let attempt = (|| {
        let runtime = managed::rebinding::sherpa(&spec.profile_snapshot, &parameters.engine)?;
        let mut prepared = preflight(storage, &spec, &parameters, &cancelled)?;
        let mut segments = Vec::new();
        // Parse every saved result before running any new native inference.
        for output in &checkpoint.outputs {
            segments.push(load_output(
                storage,
                &spec,
                &parameters,
                output,
                &cancelled,
            )?);
        }
        for (index, audio) in prepared
            .iter_mut()
            .enumerate()
            .skip(checkpoint.outputs.len())
        {
            if cancelled() {
                return Ok(None);
            }
            let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
            let frames = crate::transcription::wave::write_track(
                audio,
                &directory.join("input.wav"),
                &cancelled,
            )?;
            let native_duration = (frames * 1_000_000 / 16_000) as i64;
            managed::verify(
                &spec.profile_snapshot,
                Execution::Sherpa(&parameters.engine),
                &cancelled,
            )?;
            let Some(parsed) = sherpa::run_track(
                &runtime,
                parameters.tracks[index].num_speakers,
                &directory,
                native_duration,
                Duration::from_secs(24 * 3600),
                &cancelled,
            )?
            else {
                return Ok(None);
            };
            if cancelled() {
                return Ok(None);
            }
            managed::verify(
                &spec.profile_snapshot,
                Execution::Sherpa(&parameters.engine),
                &cancelled,
            )?;
            let track = &parameters.source.tracks[index];
            let result = ResultFile {
                schema_version: 1,
                origin_operation_id: spec.parameters["request"]["operationId"]
                    .as_str()
                    .ok_or("元の操作IDがありません")?
                    .into(),
                input_analysis_id: spec
                    .input_analysis_id
                    .clone()
                    .ok_or("固定入力版がありません")?,
                input_hash: spec.input_hash.clone(),
                source_manifest_hash: parameters.source.manifest_hash.clone(),
                track_id: track.id.clone(),
                track_start_us: track.start_us,
                duration_us: native_duration,
                engine: parameters.engine.clone(),
                num_speakers: parameters.tracks[index].num_speakers,
                segments: parsed,
            };
            let bytes = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("話者分離結果が保存サイズ上限を超えています".into());
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("result.json"))
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            let wav = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("input.wav"),
                "audio/wav",
            )?;
            let mut stdout_path = directory.join("stdout.log");
            if runtime != parameters.engine {
                let bytes = crate::restored_processing::read_local(
                    &stdout_path,
                    16 * 1024 * 1024,
                    &cancelled,
                )?;
                let normalized = sherpa::logical_output(
                    &bytes,
                    &runtime,
                    &parameters.engine,
                    parameters.tracks[index].num_speakers,
                    native_duration,
                )?;
                stdout_path = directory.join("logical-stdout.log");
                crate::restored_processing::write_new(&stdout_path, &normalized)?;
            }
            let stdout =
                storage.register_analysis_artifact(&spec.meeting_id, &stdout_path, "text/plain")?;
            let json = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("result.json"),
                "application/json",
            )?;
            let output = UnitOutput {
                index,
                wav_asset_id: wav.id,
                wav_hash: wav.content_hash,
                stdout_asset_id: stdout.id,
                stdout_hash: stdout.content_hash,
                result_asset_id: json.id,
                result_hash: json.content_hash,
            };
            let verified = load_output(storage, &spec, &parameters, &output, &cancelled)?;
            checkpoint.outputs.push(output);
            let saved = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            );
            let saved = match saved {
                Ok(j) => j,
                Err(e) => {
                    checkpoint.outputs.pop();
                    return Err(e);
                }
            };
            let paused = saved.state == "paused";
            segments.push(verified);
            progress(saved);
            if paused {
                return Ok(None);
            }
        }
        if cancelled() {
            return Ok(None);
        }
        runtime.verify(&cancelled)?;
        preflight(storage, &spec, &parameters, &cancelled)?;
        // Recheck files at the publication boundary as well as at resume.
        for (index, output) in checkpoint.outputs.iter().enumerate() {
            if load_output(storage, &spec, &parameters, output, &cancelled)? != segments[index] {
                return Err("公開前に話者区間が変化しています".into());
            }
        }
        let fixed = storage.load_diarization_input(job_id)?;
        let candidate = build_candidate(&spec, &job, &parameters, &fixed, &segments)?;
        let id = storage.finish_diarization_candidate(
            &spec.meeting_id,
            job_id,
            checkpoint.outputs.len() as i64,
            candidate,
        )?;
        progress(storage.get_job(job_id)?);
        Ok(Some(id))
    })();
    if cancelled() {
        let current = storage.get_job(job_id)?;
        if current.state == "running" && stopping() {
            storage.request_job_pause(job_id, "exit")?;
        }
        if storage.get_job(job_id)?.state == "pausing" {
            progress(storage.save_job_checkpoint(
                job_id,
                checkpoint.outputs.len() as i64,
                checkpoint.outputs.len() as i64,
                serde_json::to_value(checkpoint).map_err(|e| e.to_string())?,
            )?);
        }
        if let Ok(Some(id)) = attempt {
            return Ok(Some(id));
        }
        return Ok(None);
    }
    attempt
}

fn build_candidate(
    spec: &JobSpec,
    job: &JobSnapshot,
    parameters: &Parameters,
    input: &DiarizationInput,
    raw: &[Vec<Segment>],
) -> Result<AnalysisInput, String> {
    let mut speakers: Vec<_> = input
        .analysis
        .speakers
        .iter()
        .map(|s| SpeakerInput {
            key: format!("prior:{}", s.id),
            label: s.label.clone(),
        })
        .collect();
    let mut segments = Vec::new();
    let mut timelines = std::collections::HashMap::new();
    let key = |track: &str, speaker: u32| format!("{track}:speaker_{speaker:02}");
    for (index, track) in parameters.source.tracks.iter().enumerate() {
        let source_duration = track.end_us - track.start_us;
        let clipped: Vec<_> = raw[index]
            .iter()
            .filter(|s| s.start_us < source_duration)
            .map(|s| Segment {
                start_us: s.start_us,
                end_us: s.end_us.min(source_duration),
                speaker: s.speaker,
            })
            .collect();
        let timeline = Timeline::build(&clipped, source_duration)?;
        let native_speakers: std::collections::BTreeSet<_> =
            clipped.iter().map(|s| s.speaker).collect();
        for speaker in native_speakers {
            speakers.push(SpeakerInput {
                key: key(&track.id, speaker),
                label: format!(
                    "{} 話者{}",
                    match track.kind.as_str() {
                        "mic" => "マイク",
                        "system" => "PC音声",
                        "imported" => "取り込み音声",
                        _ => "音声",
                    },
                    speaker + 1
                ),
            });
        }
        for s in clipped {
            segments.push(SpeakerSegmentInput {
                track_id: track.id.clone(),
                speaker_key: Some(key(&track.id, s.speaker)),
                start_us: track.start_us + s.start_us,
                end_us: track.start_us + s.end_us,
                overlap_group: None,
                confidence: None,
            });
        }
        timelines.insert(&track.id, (track.start_us, timeline));
    }
    let mut utterances = Vec::new();
    for original in &input.analysis.utterances {
        let (offset, timeline) = timelines
            .get(&original.track_id)
            .ok_or("発言の対象トラックがありません")?;
        let attribution =
            timeline.attribute(original.start_us - offset, original.end_us - offset)?;
        let speaker_keys = if input.manual_utterances.contains(&original.id) {
            original
                .speaker_ids
                .iter()
                .map(|id| format!("prior:{id}"))
                .collect()
        } else {
            match attribution {
                Attribution::Assigned { speaker } => vec![key(&original.track_id, speaker)],
                Attribution::Overlap { speakers } => speakers
                    .into_iter()
                    .map(|s| key(&original.track_id, s))
                    .collect(),
                _ => vec![],
            }
        };
        utterances.push(UtteranceInput {
            track_id: original.track_id.clone(),
            start_us: original.start_us,
            end_us: original.end_us,
            text: original.recognized_text.clone(),
            speaker_keys,
            confidence: None,
        });
    }
    Ok(AnalysisInput {
        audio_manifest_hash: parameters.source.manifest_hash.clone(),
        profile_snapshot: spec.profile_snapshot.clone(),
        config: serde_json::json!({"jobId":job.id,"specHash":job.spec_hash,"retryOfJobId":job.retry_of_job_id,"attempt":job.attempt,"inputAnalysisId":input.analysis.id,"inputContentHash":spec.input_hash,"diarization":"sherpa-onnx","tracks":parameters.tracks,"estimatedSpeakers":raw.iter().map(|s|s.iter().map(|s|s.speaker).collect::<std::collections::BTreeSet<_>>().len()).collect::<Vec<_>>(),"speakerScope":"track_local","overlapDetection":"same_track_interval_intersection","ambiguousTurns":"keep_text_unknown_needs_split","manualAssignments":"preserved","timelineAlignment":"unverified"}),
        speakers,
        segments,
        utterances,
    })
}
