//! Local whisper.cpp execution. No shell, network, speaker inference or text
//! substitutions are involved. The scheduler owns input/output provenance.
pub(crate) mod wave;
pub(crate) mod whisper;

use crate::storage::{
    AnalysisInput, JobKind, JobSnapshot, JobSpec, RecordedAudioPlan, Storage, UtteranceInput,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use whisper::WhisperConfig;

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
}

impl StartRequest {
    pub fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        let engine = WhisperConfig {
            executable: whisper::Fingerprint::capture(&self.executable_path, &|| false)?,
            model: whisper::Fingerprint::capture(&self.model_path, &|| false)?,
            vad_model: self
                .vad_model_path
                .map(|path| whisper::Fingerprint::capture(&path, &|| false))
                .transpose()?,
            threads: self.threads,
            use_gpu: self.use_gpu,
            prompt: self.prompt,
        };
        enqueue(
            storage,
            &self.meeting_id,
            &self.operation_id,
            engine,
            &self.profile,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    schema_version: u32,
    source: RecordedAudioPlan,
    engine: WhisperConfig,
    units: Vec<Unit>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Unit {
    track_index: usize,
    core_first: u64,
    core_end: u64,
    window_first: u64,
    window_end: u64,
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

fn units(source: &RecordedAudioPlan) -> Result<Vec<Unit>, String> {
    let mut units = Vec::new();
    for (track_index, track) in source.tracks.iter().enumerate() {
        let frames = track
            .chunks
            .iter()
            .try_fold(0_u64, |sum, chunk| {
                sum.checked_add(u64::try_from(chunk.frames).ok()?)
            })
            .ok_or("音声フレーム数が不正です")?;
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
            });
            first = end;
        }
    }
    Ok(units)
}

pub(crate) fn enqueue(
    storage: &Storage,
    meeting_id: &str,
    operation_id: &str,
    engine: WhisperConfig,
    profile: &str,
) -> Result<JobSnapshot, String> {
    if !matches!(profile, "standard" | "high_accuracy") {
        return Err("解析プロファイルが不正です".into());
    }
    engine.verify(&|| false)?;
    let source = storage.recorded_audio_plan(meeting_id)?;
    storage.verify_meeting_asset(meeting_id, &source.manifest_asset_id, &source.manifest_hash)?;
    let units = units(&source)?;
    let total = units.len() as i64;
    let parameters = Parameters {
        schema_version: 1,
        source,
        engine,
        units,
    };
    storage.enqueue_job(JobSpec { meeting_id: meeting_id.into(), kind: JobKind::Transcription, operation_id: operation_id.into(), input_analysis_id: None, input_hash: parameters.source.manifest_hash.clone(), profile_snapshot: serde_json::json!({"profile":profile,"engine":"whisper.cpp","modelSha256":parameters.engine.model.sha256,"qualityStatus":"unverified"}), parameters: serde_json::to_value(parameters).map_err(|e| e.to_string())?, total_units: Some(total) })
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
    let parameters: Parameters =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    if spec.kind != JobKind::Transcription
        || parameters.schema_version != 1
        || spec.input_hash != parameters.source.manifest_hash
        || spec.total_units != Some(parameters.units.len() as i64)
        || parameters.units != units(&parameters.source)?
    {
        return Err("解析ジョブの入力計画が不正です".into());
    }
    let mut checkpoint: Checkpoint = checkpoint
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |job| job.state != "running")
    };
    let job = storage.get_job(job_id)?;
    if checkpoint.outputs.len() as i64 != job.completed_units {
        return Err("完了区間と再開情報が一致しません".into());
    }
    let attempt = (|| {
        parameters.engine.verify(&cancelled)?;
        if storage.recorded_audio_plan(&spec.meeting_id)? != parameters.source {
            return Err("固定した録音入力が変更されています".into());
        }
        storage.verify_meeting_asset(
            &spec.meeting_id,
            &parameters.source.manifest_asset_id,
            &spec.input_hash,
        )?;
        for (index, output) in checkpoint.outputs.iter().enumerate() {
            if output.index != index {
                return Err("再開情報の区間順が不正です".into());
            }
            storage.verify_meeting_asset(
                &spec.meeting_id,
                &output.wav_asset_id,
                &output.wav_hash,
            )?;
            storage.verify_meeting_asset(
                &spec.meeting_id,
                &output.json_asset_id,
                &output.json_hash,
            )?;
        }
        let mut prepared = Vec::new();
        for track in &parameters.source.tracks {
            if cancelled() {
                return Ok(None);
            }
            let audio = storage.prepare_track_audio_cancellable(
                &spec.meeting_id,
                &track.id,
                track.start_us,
                track.end_us,
                &cancelled,
            )?;
            if audio.total_frames()
                != track
                    .chunks
                    .iter()
                    .map(|chunk| chunk.frames as u64)
                    .sum::<u64>()
            {
                return Err("録音入力のフレーム数が変更されています".into());
            }
            prepared.push(audio);
        }
        for index in checkpoint.outputs.len()..parameters.units.len() {
            if cancelled() {
                return Ok(None);
            }
            let unit = &parameters.units[index];
            let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
            let audio = &mut prepared[unit.track_index];
            let frames = wave::write_window(
                audio,
                unit.window_first,
                unit.window_end - unit.window_first,
                &directory.join("input.wav"),
                &cancelled,
            )?;
            let duration = (frames * 1_000_000 / u64::from(wave::WHISPER_RATE)) as i64;
            if whisper::run_unit(
                &parameters.engine,
                &directory,
                duration,
                Duration::from_secs(1800),
                &cancelled,
            )?
            .is_none()
            {
                return Ok(None);
            }
            if cancelled() {
                return Ok(None);
            }
            let wav = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("input.wav"),
                "audio/wav",
            )?;
            let json = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("result.json"),
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
        parameters.engine.verify(&cancelled)?;
        let mut utterances = Vec::new();
        for output in &checkpoint.outputs {
            if cancelled() {
                return Ok(None);
            }
            let unit = &parameters.units[output.index];
            let track = &parameters.source.tracks[unit.track_index];
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
                    start_us: track.start_us + window_start as i64 + segment.start_us,
                    end_us: (track.start_us + window_start as i64 + segment.end_us)
                        .min(track.end_us),
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
        let candidate = storage.finish_transcription_candidate(&spec.meeting_id, job_id, checkpoint.outputs.len() as i64, AnalysisInput { audio_manifest_hash: spec.input_hash.clone(), profile_snapshot: spec.profile_snapshot.clone(), config: serde_json::json!({"jobId":job_id,"specHash":job.spec_hash,"diarization":"pending","timelineAlignment":"unverified","chunkSeconds":120,"contextSeconds":2,"ownership":"segment_midpoint"}), speakers: vec![], segments: vec![], utterances })?;
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
