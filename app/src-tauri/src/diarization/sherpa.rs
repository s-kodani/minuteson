use crate::local_engine::{Fingerprint, ManagedChild, StableFile};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_SPEAKERS: u32 = 100;
const MAX_SEGMENTS: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SherpaConfig {
    pub executable: Fingerprint,
    pub segmentation_model: Fingerprint,
    pub embedding_model: Fingerprint,
    // Non-system dynamic libraries supplied with a native engine are part of
    // the fixed conditions too. Empty is appropriate for a static binary.
    pub runtime_libraries: Vec<Fingerprint>,
    pub threads: u32,
    // These integers avoid non-finite JSON/CLI floats and freeze every option.
    pub cluster_threshold_milli: u32,
    pub window_shift_milli: u32,
    pub min_duration_on_ms: u32,
    pub min_duration_off_ms: u32,
}

impl SherpaConfig {
    fn validate(&self, num_speakers: Option<u32>) -> Result<(), String> {
        if self.runtime_libraries.len() > 16
            || !(1..=64).contains(&self.threads)
            || self.cluster_threshold_milli > 2000
            || !(1..=1000).contains(&self.window_shift_milli)
            || self.min_duration_on_ms > 60_000
            || self.min_duration_off_ms > 60_000
            || num_speakers.is_some_and(|n| !(1..=MAX_SPEAKERS).contains(&n))
        {
            return Err("話者分離の人数または固定設定が不正です".into());
        }
        for proof in self.files() {
            if !proof.path.is_absolute()
                || proof
                    .path
                    .to_str()
                    .is_none_or(|s| s.contains(['\n', '\r']) || s.len() > 4096)
            {
                return Err("話者分離のモデル・実行ファイルパスが不正です".into());
            }
        }
        Ok(())
    }

    fn files(&self) -> Vec<&Fingerprint> {
        let mut files = vec![
            &self.executable,
            &self.segmentation_model,
            &self.embedding_model,
        ];
        files.extend(&self.runtime_libraries);
        files
    }

    pub(crate) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.validate(None)?;
        for proof in self.files() {
            proof.verify(cancelled)?;
        }
        Ok(())
    }

    // Source protocol (including config echo):
    // sherpa-onnx/csrc/{offline-speaker-diarization,fast-clustering-config}.cc
    // A changed CLI protocol must be explicitly supported, not guessed.
    fn expected_header(&self, num_speakers: Option<u32>) -> String {
        format!(
            "OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model=\"{}\", window_shift_ratio={}), num_threads={}, debug=False, provider=\"cpu\"), embedding=SpeakerEmbeddingExtractorConfig(model=\"{}\", num_threads={}, debug=False, provider=\"cpu\"), clustering=FastClusteringConfig(num_clusters={}, threshold={}, compute_confidence=False), min_duration_on={}, min_duration_off={})",
            self.segmentation_model.path.display(), decimal(self.window_shift_milli),
            self.threads, self.embedding_model.path.display(), self.threads,
            num_speakers.map_or(-1, |n| n as i32), decimal(self.cluster_threshold_milli),
            decimal(self.min_duration_on_ms), decimal(self.min_duration_off_ms)
        )
    }
}

fn decimal(milli: u32) -> String {
    if milli % 1000 == 0 {
        (milli / 1000).to_string()
    } else {
        format!("{}.{:03}", milli / 1000, milli % 1000)
            .trim_end_matches('0')
            .into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Segment {
    pub start_us: i64,
    pub end_us: i64,
    pub speaker: u32,
}

// Decode fixed three-decimal seconds without floating-point timestamp drift.
fn timestamp(value: &str) -> Result<i64, String> {
    let (seconds, millis) = value
        .split_once('.')
        .ok_or("話者区間の時刻形式が不正です")?;
    if seconds.is_empty()
        || !seconds.bytes().all(|b| b.is_ascii_digit())
        || millis.len() != 3
        || !millis.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("話者区間の時刻形式が不正です".into());
    }
    seconds
        .parse::<i64>()
        .ok()
        .and_then(|s| s.checked_mul(1_000_000))
        .and_then(|s| s.checked_add(millis.parse::<i64>().ok()? * 1000))
        .ok_or_else(|| "話者区間の時刻が範囲外です".into())
}

pub(crate) fn parse_output(
    bytes: &[u8],
    config: &SherpaConfig,
    num_speakers: Option<u32>,
    duration_us: i64,
) -> Result<Vec<Segment>, String> {
    config.validate(num_speakers)?;
    if bytes.len() as u64 > MAX_OUTPUT_BYTES || duration_us <= 0 {
        return Err("話者分離出力のサイズまたは音声時間が不正です".into());
    }
    let output = std::str::from_utf8(bytes).map_err(|_| "話者分離出力がUTF-8ではありません")?;
    let mut lines = output.lines();
    if lines.next() != Some(config.expected_header(num_speakers).as_str())
        || lines.next() != Some("Started")
    {
        return Err("sherpa-onnxの出力形式または固定設定が一致しません".into());
    }
    let mut result = Vec::new();
    let mut previous = None;
    let mut seen = HashSet::new();
    for line in lines {
        if result.len() >= MAX_SEGMENTS {
            return Err("話者区間数が上限を超えました".into());
        }
        let parts: Vec<&str> = line.split_ascii_whitespace().collect();
        if parts.len() != 4 || parts[1] != "--" {
            // No silently ignored garbage, logs, text, or silhouette scores
            // interpreted as identification probabilities.
            return Err("話者区間の出力形式が不正です".into());
        }
        let start_us = timestamp(parts[0])?;
        let end_us = timestamp(parts[2])?;
        let speaker_text = parts[3]
            .strip_prefix("speaker_")
            .ok_or("話者ラベルが不正です")?;
        let speaker = speaker_text
            .parse::<u32>()
            .map_err(|_| "話者ラベルが不正です")?;
        if speaker_text != format!("{speaker:02}")
            || speaker >= MAX_SPEAKERS
            || num_speakers.is_some_and(|n| speaker >= n)
            || start_us >= duration_us
            || end_us <= start_us
            // float32 seconds plus the printed millisecond rounding can
            // overrun the terminal sample by up to ~16ms near the WAV limit.
            || end_us > duration_us.saturating_add(20_000)
            // Native sorting precedes millisecond rounding; two printed
            // equal starts need not have ascending speaker IDs.
            || previous.is_some_and(|p| start_us < p)
            || !seen.insert((start_us, end_us, speaker))
        {
            return Err("話者区間の時刻・順序・人数が不正です".into());
        }
        previous = Some(start_us);
        result.push(Segment {
            start_us,
            end_us: end_us.min(duration_us),
            speaker,
        });
    }
    // A filtered cluster need not appear in the final segments. Keep the
    // native labels, including gaps; empty output may legitimately be silence.
    Ok(result)
}

// Whole-track unit; an unfinished inference is killed and never checkpointed.
// Caller uses a new task-owned directory and verifies persisted provenance.
pub(crate) fn run_track(
    config: &SherpaConfig,
    num_speakers: Option<u32>,
    directory: &Path,
    duration_us: i64,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<Segment>>, String> {
    match run_track_inner(
        config,
        num_speakers,
        directory,
        duration_us,
        timeout,
        cancelled,
    ) {
        Err(_) if cancelled() => Ok(None),
        result => result,
    }
}

fn run_track_inner(
    config: &SherpaConfig,
    num_speakers: Option<u32>,
    directory: &Path,
    duration_us: i64,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<Segment>>, String> {
    if cancelled() {
        return Ok(None);
    }
    config.validate(num_speakers)?;
    config.verify(cancelled)?;
    let watched: Vec<StableFile> = config
        .files()
        .into_iter()
        .map(StableFile::watch)
        .collect::<Result<_, _>>()?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let wav = directory.join("input.wav");
    // Validate the exact mono 16k PCM16 WAV produced by write_track. Don't
    // silently let a CLI interpret a truncated/differently timed input.
    let input = check_wav(&wav, duration_us, cancelled)?;
    let stdout_path = directory.join("stdout.log");
    let stderr_path = directory.join("stderr.log");
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stdout_path)
        .map_err(|e| e.to_string())?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stderr_path)
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(&config.executable.path);
    command
        // No inherited preload/library overrides, credentials, or locale can
        // redirect the declared native runtime or alter its text protocol.
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .arg(format!(
            "--segmentation.pyannote-model={}",
            config.segmentation_model.path.display()
        ))
        .arg(format!(
            "--embedding.model={}",
            config.embedding_model.path.display()
        ))
        .arg(format!("--segmentation.num-threads={}", config.threads))
        .arg(format!("--embedding.num-threads={}", config.threads))
        .args([
            "--segmentation.provider=cpu",
            "--embedding.provider=cpu",
            "--segmentation.debug=false",
            "--embedding.debug=false",
        ])
        .arg(format!(
            "--clustering.num-clusters={}",
            num_speakers.map_or(-1, |n| n as i32)
        ))
        .arg(format!(
            "--clustering.cluster-threshold={}",
            decimal(config.cluster_threshold_milli)
        ))
        .arg("--clustering.compute-confidence=false")
        .arg(format!(
            "--segmentation.pyannote-window-shift-ratio={}",
            decimal(config.window_shift_milli)
        ))
        .arg(format!(
            "--min-duration-on={}",
            decimal(config.min_duration_on_ms)
        ))
        .arg(format!(
            "--min-duration-off={}",
            decimal(config.min_duration_off_ms)
        ))
        .arg(&wav);
    let mut child = ManagedChild(
        command
            .spawn()
            .map_err(|e| format!("sherpa-onnxを起動できません: {e}"))?,
    );
    let started = Instant::now();
    loop {
        if cancelled() {
            return Ok(None);
        }
        if started.elapsed() > timeout {
            return Err("話者分離が制限時間を超えました".into());
        }
        for (path, limit) in [
            (&stdout_path, MAX_OUTPUT_BYTES),
            (&stderr_path, MAX_LOG_BYTES),
        ] {
            if path.metadata().map_err(|e| e.to_string())?.len() > limit {
                return Err("話者分離ログが上限を超えました".into());
            }
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("sherpa-onnxが失敗しました: {status}"));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if cancelled() {
        return Ok(None);
    }
    for file in &watched {
        file.verify()?;
    }
    config.verify(cancelled)?;
    input.verify(cancelled)?;
    let mut bytes = Vec::new();
    let file = File::open(&stdout_path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file()
        || stdout_path.canonicalize().map_err(|e| e.to_string())? != stdout_path
    {
        return Err("話者分離出力が通常ファイルではありません".into());
    }
    file.take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(Some(parse_output(
        &bytes,
        config,
        num_speakers,
        duration_us,
    )?))
}

fn check_wav(
    path: &Path,
    duration_us: i64,
    cancelled: &impl Fn() -> bool,
) -> Result<Fingerprint, String> {
    let proof = Fingerprint::capture(path, cancelled)?;
    if proof.path != path || proof.size <= 44 {
        return Err("話者分離入力WAVが不正です".into());
    }
    let mut header = [0; 44];
    File::open(path)
        .map_err(|e| e.to_string())?
        .read_exact(&mut header)
        .map_err(|e| e.to_string())?;
    let frames = (proof.size - 44) / 2;
    if (proof.size - 44) % 2 != 0
        || frames > u64::from((u32::MAX - 36) / 2)
        || &header[..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64 + 8 != proof.size
        || u32::from_le_bytes(header[16..20].try_into().unwrap()) != 16
        || u16::from_le_bytes(header[20..22].try_into().unwrap()) != 1
        || u16::from_le_bytes(header[22..24].try_into().unwrap()) != 1
        || u32::from_le_bytes(header[24..28].try_into().unwrap()) != 16_000
        || u32::from_le_bytes(header[28..32].try_into().unwrap()) != 32_000
        || u16::from_le_bytes(header[32..34].try_into().unwrap()) != 2
        || u16::from_le_bytes(header[34..36].try_into().unwrap()) != 16
        || &header[36..40] != b"data"
        || u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64 != proof.size - 44
        || duration_us <= 0
        || ((frames * 1_000_000 / 16_000) as i64 - duration_us).abs() > 100
    {
        return Err("話者分離入力WAVの形式・長さが固定入力と一致しません".into());
    }
    Ok(proof)
}

#[cfg(test)]
mod tests;
