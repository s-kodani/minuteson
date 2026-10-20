use crate::local_engine::{Fingerprint, OwnedCommand, StableFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
#[cfg(test)]
use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, Instant};

mod progress;

const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_SPEAKERS: u32 = 100;
pub(super) const MAX_SEGMENTS: usize = 100_000;

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
    #[serde(
        default,
        skip_serializing_if = "crate::diarization::sensitivity::is_standard"
    )]
    pub speech_sensitivity_milli: i32,
}

impl SherpaConfig {
    fn validate(&self, num_speakers: Option<u32>) -> Result<(), String> {
        self.validate_settings(num_speakers)?;
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

    pub(crate) fn validate_settings(&self, num_speakers: Option<u32>) -> Result<(), String> {
        if self.runtime_libraries.len() > 16
            || !(1..=64).contains(&self.threads)
            || self.cluster_threshold_milli > 2000
            || !(1..=1000).contains(&self.window_shift_milli)
            || self.min_duration_on_ms > 60_000
            || self.min_duration_off_ms > 60_000
            || !super::sensitivity::valid(self.speech_sensitivity_milli)
            || num_speakers.is_some_and(|n| !(1..=MAX_SPEAKERS).contains(&n))
        {
            return Err("話者分離の人数または固定設定が不正です".into());
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
    pub(crate) fn expected_header(&self, num_speakers: Option<u32>) -> String {
        format!(
            "OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model=\"{}\", window_shift_ratio={}), num_threads={}, debug=False, provider=\"cpu\"), embedding=SpeakerEmbeddingExtractorConfig(model=\"{}\", num_threads={}, debug=False, provider=\"cpu\"), clustering=FastClusteringConfig(num_clusters={}, threshold={}, compute_confidence=False), min_duration_on={}, min_duration_off={})",
            self.segmentation_model.path.display(), decimal(self.window_shift_milli),
            self.threads, self.embedding_model.path.display(), self.threads,
            num_speakers.map_or(-1, |n| n as i32), decimal(if self.cluster_threshold_milli==0 {500}else{self.cluster_threshold_milli}),
            decimal(if self.min_duration_on_ms==0 {300}else{self.min_duration_on_ms}), decimal(if self.min_duration_off_ms==0 {500}else{self.min_duration_off_ms})
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
    parse_with_merges(bytes, config, num_speakers, duration_us).map(|(segments, _)| segments)
}

pub(crate) fn parse_with_merges(
    bytes: &[u8],
    config: &SherpaConfig,
    num_speakers: Option<u32>,
    duration_us: i64,
) -> Result<(Vec<Segment>, Vec<SpeakerMerge>), String> {
    config.validate_settings(num_speakers)?;
    // Fixed logical conditions can be unavailable after restore. Accept only
    // exact fingerprint placeholders in this pure parser, never at execution.
    for proof in config.files() {
        if !proof.path.is_absolute()
            && proof.path.to_str() != Some(format!("mon-unavailable:{}", proof.sha256).as_str())
        {
            return Err("話者分離の固定参照が不正です".into());
        }
        if proof
            .path
            .to_str()
            .is_none_or(|p| p.contains(['\0', '\n', '\r']) || p.len() > 4096)
            || proof.size == 0
            || proof.sha256.len() != 64
            || !proof
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("話者分離の固定指紋が不正です".into());
        }
    }
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
            // Native cluster labels can be sparse. Their numeric value is
            // not the number of speakers; bound labels and count separately.
            || speaker as usize >= MAX_SEGMENTS
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
    let merges = limit_speakers(&mut result, num_speakers);
    if !merges.is_empty() {
        let mut unique = HashSet::new();
        result.retain(|s| unique.insert((s.start_us, s.end_us, s.speaker)));
    }
    let labels: BTreeSet<_> = result.iter().map(|s| s.speaker).collect();
    if labels.len() > MAX_SPEAKERS as usize {
        return Err(format!(
            "自動推定した話者クラスタが多すぎます（{}個、上限{}個）。実人数ではなく過剰な分割の可能性があるため、話者分離結果を保存しませんでした。保存済み音声と本文は保持しています",
            labels.len(), MAX_SPEAKERS
        ));
    }
    // Preserve already bounded labels. If filtering leaves large sparse IDs,
    // renumber deterministically without merging distinct speakers or turns.
    if labels.last().is_some_and(|label| *label >= MAX_SPEAKERS) {
        let labels: Vec<_> = labels.into_iter().collect();
        for segment in &mut result {
            segment.speaker = labels.binary_search(&segment.speaker).unwrap() as u32;
        }
    }
    // Empty output may legitimately be silence. Stored IDs remain bounded.
    validate_saved_segments(&result, num_speakers, duration_us)?;
    Ok((result, merges))
}

/// The native short-input path bypasses clustering. Enforce the product's
/// maximum without inventing a speaker or attributing an undetected interval.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SpeakerMerge {
    pub start_us: i64,
    pub end_us: i64,
    pub from_speaker: u32,
    pub into_speaker: u32,
}

pub(crate) fn limit_speakers(
    segments: &mut [Segment],
    requested: Option<u32>,
) -> Vec<SpeakerMerge> {
    let Some(limit) = requested else {
        return vec![];
    };
    let mut durations = std::collections::BTreeMap::<u32, i64>::new();
    for s in segments.iter() {
        *durations.entry(s.speaker).or_default() += s.end_us - s.start_us;
    }
    if durations.len() <= limit as usize && durations.keys().all(|id| *id < limit) {
        return vec![];
    }
    let mut ranked: Vec<_> = durations.into_iter().collect();
    ranked.sort_by_key(|(id, duration)| (std::cmp::Reverse(*duration), *id));
    let retained: BTreeSet<_> = ranked
        .into_iter()
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect();
    let originals = segments.to_vec();
    let mut destinations = std::collections::BTreeMap::<u32, (i64, u32)>::new();
    for s in originals.iter().filter(|s| !retained.contains(&s.speaker)) {
        if let Some(closest) = originals
            .iter()
            .filter(|v| retained.contains(&v.speaker))
            .map(|v| {
                let distance = (s.start_us - v.end_us).max(v.start_us - s.end_us).max(0);
                (distance, v.speaker)
            })
            .min()
        {
            destinations
                .entry(s.speaker)
                .and_modify(|v| *v = (*v).min(closest))
                .or_insert(closest);
        }
    }
    let mut merges = Vec::new();
    for s in segments.iter_mut() {
        if let Some((_, target)) = destinations.get(&s.speaker) {
            merges.push(SpeakerMerge {
                start_us: s.start_us,
                end_us: s.end_us,
                from_speaker: s.speaker,
                into_speaker: *target,
            });
            s.speaker = *target;
        }
    }
    let retained: Vec<_> = retained.into_iter().collect();
    for s in segments {
        s.speaker = retained.binary_search(&s.speaker).unwrap() as u32;
    }
    merges
}

pub(crate) fn validate_saved_segments(
    segments: &[Segment],
    num_speakers: Option<u32>,
    duration: i64,
) -> Result<(), String> {
    if duration <= 0
        || segments.len() > MAX_SEGMENTS
        || num_speakers.is_some_and(|n| !(1..=MAX_SPEAKERS).contains(&n))
    {
        return Err("保存済み話者区間の人数・時間・個数が不正です".into());
    }
    let mut previous = None;
    let mut seen = HashSet::new();
    for s in segments {
        if s.start_us < 0
            || s.start_us >= duration
            || s.end_us <= s.start_us
            || s.end_us > duration
            || s.speaker >= MAX_SPEAKERS
            || num_speakers.is_some_and(|n| s.speaker >= n)
            || previous.is_some_and(|p| s.start_us < p)
            || !seen.insert((s.start_us, s.end_us, s.speaker))
        {
            return Err("保存済み話者区間の時刻・順序・人数・重複が不正です".into());
        }
        previous = Some(s.start_us);
    }
    Ok(())
}

pub(crate) fn logical_output(
    bytes: &[u8],
    runtime: &SherpaConfig,
    logical: &SherpaConfig,
    speakers: Option<u32>,
    duration: i64,
) -> Result<Vec<u8>, String> {
    let segments = parse_output(bytes, runtime, speakers, duration)?;
    let header = runtime.expected_header(speakers);
    let tail = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .strip_prefix(&header)
        .ok_or("実行条件のヘッダーがありません")?;
    let normalized = format!("{}{tail}", logical.expected_header(speakers)).into_bytes();
    if parse_output(&normalized, logical, speakers, duration)? != segments {
        return Err("再結合で保存済み話者区間が変化しました".into());
    }
    Ok(normalized)
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
    run_track_observed(
        config,
        num_speakers,
        directory,
        duration_us,
        timeout,
        cancelled,
        &|_| {},
    )
}

pub(crate) fn run_track_observed(
    config: &SherpaConfig,
    num_speakers: Option<u32>,
    directory: &Path,
    duration_us: i64,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u32),
) -> Result<Option<Vec<Segment>>, String> {
    match run_track_inner(
        config,
        num_speakers,
        directory,
        duration_us,
        timeout,
        cancelled,
        progress,
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
    progress: &impl Fn(u32),
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
    let _log_capture =
        crate::internal_logs::watch_engine(&directory, &["stderr.log"], "diarization");
    let mut launch = OwnedCommand::new(&config.executable.path)?;
    let command = &mut launch.command;
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
    if crate::local_engine::resident::enabled() {
        let libraries: Vec<_> = config
            .runtime_libraries
            .iter()
            .filter(|f| {
                f.path
                    .file_name()
                    .is_some_and(|n| n == "libsherpa-onnx-c-api.dylib")
            })
            .collect();
        if libraries.len() != 1 {
            return Err("話者分離用C APIランタイムがありません".into());
        }
        command.arg("--library").arg(&libraries[0].path);
    }
    let mut child = crate::local_engine::resident::Run::spawn(
        &mut launch,
        "sherpa",
        &directory,
        "stdout.log",
        "stderr.log",
    )
    .map_err(|e| format!("sherpa-onnxを起動できません: {e}"))?;
    let mut progress_file = File::open(&stderr_path).ok();
    let mut log_progress = progress::LogProgress::default();
    progress(0);
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
        if let Some(file) = &mut progress_file {
            log_progress.poll(file, progress);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| failure_detail(&stderr_path, &e.to_string()))?
        {
            if !status.success() {
                return Err(failure_detail(
                    &stderr_path,
                    &format!("sherpa-onnxが失敗しました: {status}"),
                ));
            }
            if let Some(file) = &mut progress_file {
                log_progress.poll(file, progress);
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

fn failure_detail(stderr: &Path, error: &str) -> String {
    let log = std::fs::File::open(stderr)
        .ok()
        .and_then(|f| {
            let mut text = String::new();
            f.take(MAX_LOG_BYTES)
                .read_to_string(&mut text)
                .ok()
                .map(|_| text)
        })
        .unwrap_or_default();
    let combined = format!("{error} {log}").to_ascii_lowercase();
    if [
        "bad_alloc",
        "out of memory",
        "failed to allocate memory",
        "not enough memory",
        "memory allocation failed",
    ]
    .iter()
    .any(|s| combined.contains(s))
    {
        return "話者分離に必要なメモリを確保できませんでした。会議全体の処理を停止しました。元音声と保存済みの本文は保持しています".into();
    }
    format!("会議全体の話者分離を完了できませんでした: {error}")
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
