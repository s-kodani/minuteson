//! Local whisper.cpp execution. No shell, network, speaker inference or text
//! substitutions are involved. The scheduler owns input/output provenance.
mod restore;
pub(crate) mod wave;
pub(crate) mod whisper;
pub(crate) use restore::restore_saved_results;

use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::KnownInput;
use crate::storage::{
    AnalysisInput, JobKind, JobSnapshot, JobSpec, RecordedAudioPlan, Storage, UtteranceInput,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use whisper::WhisperConfig;

fn check_source(storage: &Storage, spec: &JobSpec, p: &Parameters) -> Result<(), String> {
    let current = if p.known_input.is_some() {
        storage.recorded_analysis_input(&spec.meeting_id)?
    } else {
        (storage.recorded_audio_plan(&spec.meeting_id)?, None)
    };
    if current != (p.source.clone(), p.known_input.clone()) {
        return Err("固定した録音入力・欠損・時刻補正が変更されています".into());
    }
    Ok(())
}

fn verify_sources(
    storage: &Storage,
    spec: &JobSpec,
    p: &Parameters,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check_source(storage, spec, p)?;
    for track in &p.source.tracks {
        if p.known_input.is_some() {
            storage.prepare_analysis_track(&spec.meeting_id, track, cancelled)?;
        } else {
            let audio = storage.prepare_track_audio_cancellable(
                &spec.meeting_id,
                &track.id,
                track.start_us,
                track.end_us,
                cancelled,
            )?;
            if audio.total_frames() != track.total_frames()?
                || audio.sample_rate != track.sample_rate
                || audio.channels != track.channels
            {
                return Err("固定した入力フレーム数が変更されています".into());
            }
        }
    }
    check_source(storage, spec, p)
}

fn unit_audio(
    storage: &Storage,
    spec: &JobSpec,
    p: &Parameters,
    unit: &Unit,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::playback::PreparedAudio, String> {
    let track = &p.source.tracks[unit.track_index];
    if let Some(index) = unit.run_index {
        let run = p
            .known_input
            .as_ref()
            .and_then(|known| known.runs.get(index))
            .ok_or("固定保存区間がありません")?;
        storage.prepare_analysis_run(&spec.meeting_id, track, run, cancelled)
    } else {
        storage.prepare_track_audio_cancellable(
            &spec.meeting_id,
            &track.id,
            track.start_us,
            track.end_us,
            cancelled,
        )
    }
}

#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StartRequest {
    pub meeting_id: String,
    pub operation_id: String,
    pub executable_path: std::path::PathBuf,
    pub model_path: std::path::PathBuf,
    pub vad_model_path: Option<std::path::PathBuf>,
    pub threads: u32,
    pub use_gpu: bool,
    pub profile: String,
    pub prompt: String,
    #[serde(default)]
    pub expected_vocabulary: Option<crate::storage::VocabularyStamp>,
}

#[cfg(test)]
impl StartRequest {
    pub fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        let engine = WhisperConfig {
            executable: whisper::Fingerprint::capture(
                &self.executable_path,
                &crate::test_support::never_cancelled,
            )?,
            model: whisper::Fingerprint::capture(
                &self.model_path,
                &crate::test_support::never_cancelled,
            )?,
            vad_model: self
                .vad_model_path
                .map(|path| {
                    whisper::Fingerprint::capture(&path, &crate::test_support::never_cancelled)
                })
                .transpose()?,
            threads: self.threads,
            use_gpu: self.use_gpu,
            prompt: self.prompt,
        };
        enqueue_with_stamp(
            storage,
            &self.meeting_id,
            &self.operation_id,
            engine,
            &self.profile,
            self.expected_vocabulary,
        )
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
    pub fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        let original = storage.get_job(&self.job_id)?;
        if original.meeting_id != self.meeting_id || original.spec_hash != self.expected_spec_hash {
            return Err("確認した再試行対象と会議・固定条件が一致しません".into());
        }
        if original.kind != "transcription"
            || !matches!(original.state.as_str(), "failed" | "cancelled")
        {
            return Err("失敗・取消済みの文字起こしだけ再試行できます".into());
        }
        if original.attempt >= 1000 {
            return Err("再試行は1000回以内です。新しい解析として開始してください".into());
        }
        let (spec, checkpoint) = storage.fixed_job_input(&original.id)?;
        retry_preflight(
            storage,
            &original,
            &spec,
            checkpoint.as_ref(),
            &crate::test_support::never_cancelled,
        )?;
        storage.enqueue_retry_job(&original, &spec, checkpoint.as_ref(), &self.operation_id)
    }
}

pub(crate) fn retry_preflight(
    storage: &Storage,
    original: &JobSnapshot,
    spec: &JobSpec,
    checkpoint: Option<&serde_json::Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let parameters = checked_parameters(storage, spec, &original.id)?;
    let decoded = checked_checkpoint(original, checkpoint.cloned())?;
    let runtime = managed::rebinding::whisper(&spec.profile_snapshot, &parameters.engine)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Whisper(&parameters.engine),
        cancelled,
    )?;
    check_source(storage, spec, &parameters)?;
    storage.verify_meeting_asset(
        &spec.meeting_id,
        &parameters.source.manifest_asset_id,
        &spec.input_hash,
    )?;
    verify_sources(storage, spec, &parameters, cancelled)?;
    for output in &decoded.outputs {
        verify_saved_pcm(storage, spec, &parameters, output, cancelled)?;
        let bytes = storage.read_analysis_result(
            &spec.meeting_id,
            &output.json_asset_id,
            &output.json_hash,
        )?;
        let unit = parameters
            .units
            .get(output.index)
            .ok_or("再試行区間が範囲外です")?;
        let rate = parameters.source.tracks[unit.track_index].sample_rate;
        let duration = ((unit.window_end - unit.window_first) * 1_000_000 / u64::from(rate)) as i64;
        whisper::parse_output(&bytes, &parameters.engine.model.path, duration)?;
    }
    if cancelled() {
        return Err("文字起こし再試行の照合を中断しました".into());
    }
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Whisper(&parameters.engine),
        cancelled,
    )
}

fn verify_saved_pcm(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    output: &UnitOutput,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    crate::restored_processing::verify_pcm(
        storage,
        &spec.meeting_id,
        &output.wav_asset_id,
        &output.wav_hash,
        cancelled,
        |path| {
            let unit = parameters
                .units
                .get(output.index)
                .ok_or("完了PCMの区間が範囲外です")?;
            let mut audio = unit_audio(storage, spec, parameters, unit, cancelled)?;
            wave::write_window(
                &mut audio,
                unit.window_first,
                unit.window_end - unit.window_first,
                path,
                cancelled,
            )
            .map(|_| ())
        },
    )
}

// Frozen per job so old checkpoints keep their original decoding settings.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RecognitionPolicy {
    SuppressNonSpeechV1,
}

// Frozen for new jobs only. `use_gpu` requests the engine's GPU backend; when
// no device exists whisper.cpp selects CPU. It is not evidence of GPU use.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum AccelerationPolicy {
    AutoMetalV1,
}

impl AccelerationPolicy {
    fn configure(self, engine: &mut WhisperConfig) {
        if cfg!(target_os = "macos") {
            engine.use_gpu = true;
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    schema_version: u32,
    source: RecordedAudioPlan,
    engine: WhisperConfig,
    units: Vec<Unit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vocabulary: Option<crate::storage::VocabularySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    known_input: Option<KnownInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recognition_policy: Option<RecognitionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acceleration_policy: Option<AccelerationPolicy>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Unit {
    track_index: usize,
    core_first: u64,
    core_end: u64,
    window_first: u64,
    window_end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_index: Option<usize>,
}

#[derive(Default, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Checkpoint {
    outputs: Vec<UnitOutput>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UnitOutput {
    index: usize,
    wav_asset_id: String,
    wav_hash: String,
    json_asset_id: String,
    json_hash: String,
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
        return Err("再開情報が固定した総区間数を超えています".into());
    }
    for output in &mut checkpoint.outputs {
        (output.wav_asset_id, output.wav_hash) =
            map(Role::TemporaryPcm, &output.wav_asset_id, &output.wav_hash)?;
        (output.json_asset_id, output.json_hash) = map(
            Role::TranscriptionResult,
            &output.json_asset_id,
            &output.json_hash,
        )?;
    }
    serde_json::to_value(checkpoint).map_err(|e| e.to_string())
}

fn checked_parameters(
    storage: &Storage,
    spec: &JobSpec,
    job_id: &str,
) -> Result<Parameters, String> {
    decode_parameters(spec, |snapshot| {
        storage.verify_vocabulary_snapshot(&spec.meeting_id, job_id, snapshot)
    })
}

fn decode_parameters(
    spec: &JobSpec,
    mut verify: impl FnMut(&crate::storage::VocabularySnapshot) -> Result<(), String>,
) -> Result<Parameters, String> {
    let parameters: Parameters =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    if spec.kind != JobKind::Transcription
        || !matches!(parameters.schema_version, 1 | 2 | 3)
        || (parameters.schema_version == 3) != parameters.known_input.is_some()
        || spec.input_hash != parameters.source.manifest_hash
        || spec.total_units != Some(parameters.units.len() as i64)
        || parameters.units != planned_units(&parameters.source, parameters.known_input.as_ref())?
    {
        return Err("解析ジョブの入力計画が不正です".into());
    }
    match (&parameters.vocabulary, parameters.schema_version) {
        (None, 1) => {}
        (Some(snapshot), 2 | 3) if parameters.engine.prompt == snapshot.actual_prompt => {
            verify(snapshot)?;
        }
        _ => return Err("固定用語とエンジンへのヒントが一致しません".into()),
    }
    managed::validate(
        &spec.profile_snapshot,
        Execution::Whisper(&parameters.engine),
    )?;
    Ok(parameters)
}

fn checked_checkpoint(
    job: &JobSnapshot,
    checkpoint: Option<serde_json::Value>,
) -> Result<Checkpoint, String> {
    let checkpoint: Checkpoint = checkpoint
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if checkpoint.outputs.len() as i64 != job.completed_units
        || checkpoint
            .outputs
            .iter()
            .enumerate()
            .any(|(index, output)| output.index != index)
    {
        return Err("完了区間と再開情報が一致しません".into());
    }
    Ok(checkpoint)
}

fn units(source: &RecordedAudioPlan) -> Result<Vec<Unit>, String> {
    let mut units = Vec::new();
    for (track_index, track) in source.tracks.iter().enumerate() {
        let frames = track.total_frames()?;
        let rate = u64::from(track.sample_rate);
        if !(8_000..=384_000).contains(&rate)
            || !(1..=8).contains(&track.channels)
            || frames == 0
            || frames / rate > 7 * 24 * 3600
        {
            return Err("解析入力の形式・長さが範囲外です".into());
        }
        let mut first = 0;
        while first < frames {
            let end = (first + rate * 120).min(frames);
            units.push(Unit {
                track_index,
                core_first: first,
                core_end: end,
                window_first: first.saturating_sub(rate * 2),
                window_end: (end + rate * 2).min(frames),
                run_index: None,
            });
            first = end;
        }
    }
    Ok(units)
}

fn planned_units(
    source: &RecordedAudioPlan,
    known: Option<&KnownInput>,
) -> Result<Vec<Unit>, String> {
    let Some(known) = known else {
        return units(source);
    };
    known.validate(source)?;
    if !known.needed(source) {
        return Err("連続音声へ保存区間別の固定条件を混在させることはできません".into());
    }
    let mut units = Vec::new();
    for (run_index, run) in known.runs.iter().enumerate() {
        let rate = u64::from(source.tracks[run.track_index].sample_rate);
        let frames = run.end_frame - run.first_frame;
        let mut first = 0;
        while first < frames {
            let end = (first + rate * 120).min(frames);
            units.push(Unit {
                track_index: run.track_index,
                core_first: first,
                core_end: end,
                window_first: first.saturating_sub(rate * 2),
                window_end: (end + rate * 2).min(frames),
                run_index: Some(run_index),
            });
            first = end;
        }
    }
    Ok(units)
}

#[cfg(test)]
pub(crate) fn enqueue(
    storage: &Storage,
    meeting_id: &str,
    operation_id: &str,
    engine: WhisperConfig,
    profile: &str,
) -> Result<JobSnapshot, String> {
    enqueue_with_stamp(storage, meeting_id, operation_id, engine, profile, None)
}
pub(crate) fn enqueue_with_stamp(
    storage: &Storage,
    meeting_id: &str,
    operation_id: &str,
    mut engine: WhisperConfig,
    profile: &str,
    expected: Option<crate::storage::VocabularyStamp>,
) -> Result<JobSnapshot, String> {
    if !matches!(profile, "standard" | "high_accuracy") {
        return Err("解析プロファイルが不正です".into());
    }
    engine.verify(&|| false)?;
    let selected = storage.job_model_selection(
        meeting_id,
        operation_id,
        profile,
        Execution::Whisper(&engine),
    )?;
    let (source, known_input) = storage.recorded_analysis_input(meeting_id)?;
    storage.verify_meeting_asset(meeting_id, &source.manifest_asset_id, &source.manifest_hash)?;
    let units = planned_units(&source, known_input.as_ref())?;
    let total = units.len() as i64;
    let extra = engine.prompt.clone();
    storage.enqueue_with_vocabulary(meeting_id,operation_id,&extra,expected,move |snapshot| {
        engine.prompt=snapshot.actual_prompt.clone();
        let parameters=Parameters{schema_version:if known_input.is_some(){3}else{2},source,engine,units,vocabulary:Some(snapshot),known_input,recognition_policy:Some(RecognitionPolicy::SuppressNonSpeechV1),acceleration_policy:cfg!(target_os = "macos").then_some(AccelerationPolicy::AutoMetalV1)};
        Ok(JobSpec { meeting_id: meeting_id.into(), kind: JobKind::Transcription, operation_id: operation_id.into(), input_analysis_id: None, input_hash: parameters.source.manifest_hash.clone(), profile_snapshot: managed::attach(serde_json::json!({"profile":profile,"engine":"whisper.cpp","modelSha256":parameters.engine.model.sha256,"qualityStatus":"unverified"}), selected)?, parameters: serde_json::to_value(parameters).map_err(|e| e.to_string())?, total_units: Some(total) })
    })
}

// Run a claimed job on a blocking thread. Only completed, durable results move
// the checkpoint forward. A pause kills/reaps the unfinished CLI before return.
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, checkpoint) = storage.fixed_job_input(job_id)?;
    let parameters = checked_parameters(storage, &spec, job_id)?;
    let job = storage.get_job(job_id)?;
    let mut checkpoint = checked_checkpoint(&job, checkpoint)?;
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |job| job.state != "running")
    };
    let attempt = (|| {
        let mut runtime = managed::rebinding::whisper(&spec.profile_snapshot, &parameters.engine)?;
        if let Some(policy) = parameters.acceleration_policy {
            policy.configure(&mut runtime);
        }
        runtime.verify(&cancelled)?;
        managed::verify(
            &spec.profile_snapshot,
            Execution::Whisper(&parameters.engine),
            &cancelled,
        )?;
        check_source(storage, &spec, &parameters)?;
        storage.verify_meeting_asset(
            &spec.meeting_id,
            &parameters.source.manifest_asset_id,
            &spec.input_hash,
        )?;
        for (index, output) in checkpoint.outputs.iter().enumerate() {
            if output.index != index {
                return Err("再開情報の区間順が不正です".into());
            }
            verify_saved_pcm(storage, &spec, &parameters, output, &cancelled)?;
            let bytes = storage.read_analysis_result(
                &spec.meeting_id,
                &output.json_asset_id,
                &output.json_hash,
            )?;
            let unit = parameters
                .units
                .get(index)
                .ok_or("再試行区間が範囲外です")?;
            let rate = parameters.source.tracks[unit.track_index].sample_rate;
            let duration =
                ((unit.window_end - unit.window_first) * 1_000_000 / u64::from(rate)) as i64;
            whisper::parse_output(&bytes, &parameters.engine.model.path, duration)?;
        }
        verify_sources(storage, &spec, &parameters, &cancelled)?;
        // At most one prepared run, not all gap-separated runs in memory.
        let mut prepared: Option<(usize, Option<usize>, crate::playback::PreparedAudio)> = None;
        for index in checkpoint.outputs.len()..parameters.units.len() {
            if cancelled() {
                return Ok(None);
            }
            let unit = &parameters.units[index];
            let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
            if prepared
                .as_ref()
                .is_none_or(|(track, run, _)| *track != unit.track_index || *run != unit.run_index)
            {
                prepared = Some((
                    unit.track_index,
                    unit.run_index,
                    unit_audio(storage, &spec, &parameters, unit, &cancelled)?,
                ));
                check_source(storage, &spec, &parameters)?;
            }
            let audio = &mut prepared.as_mut().unwrap().2;
            let frames = wave::write_window(
                audio,
                unit.window_first,
                unit.window_end - unit.window_first,
                &directory.join("input.wav"),
                &cancelled,
            )?;
            let duration = (frames * 1_000_000 / u64::from(wave::WHISPER_RATE)) as i64;
            managed::verify(
                &spec.profile_snapshot,
                Execution::Whisper(&parameters.engine),
                &cancelled,
            )?;
            if whisper::run_unit_with_suppression(
                &runtime,
                &directory,
                duration,
                Duration::from_secs(1800),
                parameters.recognition_policy.is_some(),
                &cancelled,
            )?
            .is_none()
            {
                return Ok(None);
            }
            if cancelled() {
                return Ok(None);
            }
            managed::verify(
                &spec.profile_snapshot,
                Execution::Whisper(&parameters.engine),
                &cancelled,
            )?;
            let wav = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("input.wav"),
                "audio/wav",
            )?;
            let mut result_path = directory.join("result.json");
            if runtime.model.path != parameters.engine.model.path {
                let bytes = crate::restored_processing::read_local(
                    &result_path,
                    8 * 1024 * 1024,
                    &cancelled,
                )?;
                let normalized = whisper::logical_output(
                    &bytes,
                    &runtime.model.path,
                    &parameters.engine.model.path,
                    duration,
                )?;
                result_path = directory.join("logical-result.json");
                crate::restored_processing::write_new(&result_path, &normalized)?;
            }
            let json = storage.register_analysis_artifact(
                &spec.meeting_id,
                &result_path,
                "application/json",
            )?;
            checkpoint.outputs.push(UnitOutput {
                index,
                wav_asset_id: wav.id,
                wav_hash: wav.content_hash,
                json_asset_id: json.id,
                json_hash: json.content_hash,
            });
            let saved = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            );
            let snapshot = match saved {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    checkpoint.outputs.pop();
                    return Err(error);
                }
            };
            let paused = snapshot.state == "paused";
            progress(snapshot);
            if paused {
                return Ok(None);
            }
        }
        runtime.verify(&cancelled)?;
        check_source(storage, &spec, &parameters)?;
        let mut utterances = Vec::new();
        managed::verify(
            &spec.profile_snapshot,
            Execution::Whisper(&parameters.engine),
            &cancelled,
        )?;
        for output in &checkpoint.outputs {
            if cancelled() {
                return Ok(None);
            }
            let unit = &parameters.units[output.index];
            let track = &parameters.source.tracks[unit.track_index];
            let run = unit
                .run_index
                .map(|index| &parameters.known_input.as_ref().unwrap().runs[index]);
            let origin = run.map_or(track.start_us, |run| {
                (run.first_frame * 1_000_000 / u64::from(track.sample_rate)) as i64
            });
            let known_end = run.map_or(track.end_us, |run| run.end_us);
            let duration = ((unit.window_end - unit.window_first) * 1_000_000
                / u64::from(track.sample_rate)) as i64;
            let bytes = storage.read_analysis_result(
                &spec.meeting_id,
                &output.json_asset_id,
                &output.json_hash,
            )?;
            for segment in whisper::parse_output(&bytes, &parameters.engine.model.path, duration)? {
                let window_start = unit.window_first * 1_000_000 / u64::from(track.sample_rate);
                let midpoint = (segment.start_us as u64 + segment.end_us as u64) / 2 + window_start;
                let core_first = unit.core_first * 1_000_000 / u64::from(track.sample_rate);
                let core_end = unit.core_end * 1_000_000 / u64::from(track.sample_rate);
                // Two seconds of context on both sides, with midpoint ownership
                // to avoid publishing the same context segment in two units.
                if midpoint < core_first || midpoint >= core_end {
                    continue;
                }
                utterances.push(UtteranceInput {
                    track_id: track.id.clone(),
                    start_us: origin + window_start as i64 + segment.start_us,
                    end_us: (origin + window_start as i64 + segment.end_us).min(known_end),
                    text: segment.text,
                    speaker_keys: vec![],
                    confidence: None,
                });
            }
        }
        utterances.sort_by(|left, right| {
            (left.start_us, &left.track_id, left.end_us).cmp(&(
                right.start_us,
                &right.track_id,
                right.end_us,
            ))
        });
        check_source(storage, &spec, &parameters)?;
        let mut config = serde_json::json!({"jobId":job_id,"specHash":job.spec_hash,"retryOfJobId":job.retry_of_job_id,"attempt":job.attempt,"diarization":"pending","timelineAlignment":"unverified","chunkSeconds":120,"contextSeconds":2,"ownership":"segment_midpoint","vocabulary":parameters.vocabulary});
        if let Some(policy) = parameters.recognition_policy {
            config["recognitionPolicy"] =
                serde_json::to_value(policy).map_err(|e| e.to_string())?;
        }
        if let Some(policy) = parameters.acceleration_policy {
            config["accelerationPolicy"] =
                serde_json::to_value(policy).map_err(|e| e.to_string())?;
        }
        if let Some(known) = &parameters.known_input {
            config["audioCoverage"] = serde_json::json!({"mode":"known_runs","runCount":known.runs.len(),"missingRangeCount":known.gaps.iter().filter(|gap|!gap.recovered).count(),"timeBase":"meeting"});
        }
        let candidate = storage.finish_transcription_candidate(
            &spec.meeting_id,
            job_id,
            checkpoint.outputs.len() as i64,
            AnalysisInput {
                audio_manifest_hash: spec.input_hash.clone(),
                profile_snapshot: spec.profile_snapshot.clone(),
                config,
                speakers: vec![],
                segments: vec![],
                utterances,
            },
        )?;
        progress(storage.get_job(job_id)?);
        Ok(Some(candidate))
    })();
    if cancelled() {
        let job = storage.get_job(job_id)?;
        if job.state == "running" && stopping() {
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
        // Success is terminal; cancellation becoming true after publication is
        // not an unfinished unit and must not convert success to a pause.
        if let Ok(Some(candidate)) = attempt {
            return Ok(Some(candidate));
        }
        return Ok(None);
    }
    attempt
}

#[cfg(test)]
mod recognition_policy_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_policy_preserves_old_parameter_bytes_and_unknown_policies_are_rejected() {
        let legacy = json!({"schemaVersion":1,"source":{"manifestAssetId":"manifest","manifestHash":"hash","tracks":[]},"engine":{"executable":{"path":"/cli","size":1,"sha256":"a".repeat(64)},"model":{"path":"/model","size":1,"sha256":"b".repeat(64)},"vadModel":null,"threads":1,"useGpu":false,"prompt":"用語"},"units":[]});
        let old: Parameters = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(old.recognition_policy, None);
        assert_eq!(old.acceleration_policy, None);
        assert_eq!(serde_json::to_value(old).unwrap(), legacy);
        let mut new = legacy;
        new["recognitionPolicy"] = json!("suppressNonSpeechV1");
        let decoded: Parameters = serde_json::from_value(new.clone()).unwrap();
        assert_eq!(
            decoded.recognition_policy,
            Some(RecognitionPolicy::SuppressNonSpeechV1)
        );
        assert_eq!(serde_json::to_value(decoded).unwrap(), new);
        new["recognitionPolicy"] = json!("unknownPolicy");
        assert!(serde_json::from_value::<Parameters>(new).is_err());
    }

    #[test]
    fn gpu_policy_is_optional_versioned_and_preserves_base_configuration() {
        let base = WhisperConfig {
            executable: whisper::Fingerprint {
                path: "/cli".into(),
                size: 1,
                sha256: "a".repeat(64),
            },
            model: whisper::Fingerprint {
                path: "/model".into(),
                size: 1,
                sha256: "b".repeat(64),
            },
            vad_model: None,
            threads: 4,
            use_gpu: false,
            prompt: "登録用語".into(),
        };
        let mut configured = base.clone();
        AccelerationPolicy::AutoMetalV1.configure(&mut configured);
        assert_eq!(configured.use_gpu, cfg!(target_os = "macos"));
        assert_eq!(configured.model, base.model);
        assert_eq!(configured.executable, base.executable);
        assert_eq!(configured.threads, base.threads);
        assert_eq!(configured.prompt, base.prompt);
        assert!(!base.use_gpu);
        assert_eq!(
            serde_json::to_value(AccelerationPolicy::AutoMetalV1).unwrap(),
            json!("autoMetalV1")
        );
        assert!(serde_json::from_value::<AccelerationPolicy>(json!("unknownPolicy")).is_err());
    }
}
