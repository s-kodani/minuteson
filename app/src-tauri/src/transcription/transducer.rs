//! Native transducer adapter. The worker emits the common ASR result contract.
use super::{
    asr,
    input::Input,
    whisper::{AsrConfig, Segment},
};
use crate::local_engine::{OwnedCommand, StableFile};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

const PROCESS_POLICY: mon_native::process::Policy =
    mon_native::process::Purpose::Inference.policy();

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Output {
    schema_version: u32,
    backend_id: String,
    models: BTreeMap<String, String>,
    duration_us: i64,
    segments: Vec<Segment>,
    tokens: Vec<Token>,
    ignored_hotwords: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Token {
    start_us: i64,
    text: String,
}

pub(crate) fn parse(
    bytes: &[u8],
    config: &AsrConfig,
    duration: i64,
) -> Result<Vec<Segment>, String> {
    if bytes.len() as u64 > PROCESS_POLICY.result_bytes || duration <= 0 {
        return Err("ASR出力のサイズ・時間が不正です".into());
    }
    let output: Output =
        serde_json::from_slice(bytes).map_err(|e| format!("ASR出力が不正です: {e}"))?;
    let backend = config.backend.as_ref().ok_or("ASR構成がありません")?;
    let mut models = backend
        .files
        .iter()
        .map(|(k, f)| (k.clone(), f.path.to_string_lossy().into_owned()))
        .collect::<BTreeMap<_, _>>();
    models.insert(
        "model".into(),
        config.model.path.to_string_lossy().into_owned(),
    );
    if output.schema_version != 1
        || output.backend_id != backend.id
        || output.models != models
        || (output.duration_us - duration).abs() > 100
        || output.segments.len() > 100_000
        || output.tokens.len() > 500_000
        || output.ignored_hotwords.len() > 1000
    {
        return Err("ASR出力が固定したモデル・音声範囲と一致しません".into());
    }
    let mut previous = 0;
    for s in &output.segments {
        if s.start_us < previous
            || s.end_us <= s.start_us
            || s.end_us > duration
            || s.text.trim().is_empty()
            || s.text.chars().count() > 20_000
        {
            return Err("ASR区間の本文・時刻が不正です".into());
        }
        previous = s.end_us;
    }
    previous = 0;
    for token in output.tokens {
        if token.start_us < previous || token.start_us >= duration || token.text.len() > 1024 {
            return Err("ASRトークン時刻が不正です".into());
        }
        previous = token.start_us;
    }
    if output.ignored_hotwords.iter().any(|s| s.len() > 800) {
        return Err("ASR用語診断が大きすぎます".into());
    }
    Ok(output.segments)
}

pub(crate) fn run(
    config: &AsrConfig,
    directory: &Path,
    duration: i64,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<Segment>>, String> {
    if cancelled() {
        return Ok(None);
    }
    config.validate_settings()?;
    let backend = config.backend.as_ref().ok_or("ASR構成がありません")?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let input = Input::load(&directory, cancelled)?;
    if input.frames() > 124 * 16000
        || (input.frames() as i64 * 1_000_000 / 16000 - duration).abs() > 100
    {
        return Err("ASR入力範囲が不正です".into());
    }
    let mut watched = vec![
        StableFile::watch(&config.executable)?,
        StableFile::watch(&config.model)?,
    ];
    for file in backend.files.values().chain(config.vad_model.iter()) {
        watched.push(StableFile::watch(file)?);
    }
    let result = directory.join("result.json");
    if result.exists() {
        return Err("ASR結果は既に存在しています".into());
    }
    let mut launch = OwnedCommand::for_purpose(
        &config.executable.path,
        mon_native::process::Purpose::Inference,
    )?;
    let c = &mut launch.command;
    c.env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8")
        .current_dir(&directory)
        .stdin(Stdio::null());
    for (name, is_out) in [("stdout.log", true), ("stderr.log", false)] {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .map_err(|e| e.to_string())?;
        if is_out {
            c.stdout(file);
        } else {
            c.stderr(file);
        }
    }
    let _logs = crate::internal_logs::watch_engine(
        &directory,
        &["stdout.log", "stderr.log"],
        "transcription",
    );
    c.arg("--model")
        .arg(&config.model.path)
        .arg("--file")
        .arg(&input.proof.path)
        .arg("--mon-first-frame")
        .arg(input.first_frame.to_string())
        .arg("--mon-end-frame")
        .arg(input.end_frame.to_string())
        .arg("--output")
        .arg(&result)
        .arg("--threads")
        .arg(config.threads.to_string())
        .arg("--hotwords")
        .arg(&config.prompt);
    for (key, file) in &backend.files {
        c.arg(format!("--{key}")).arg(&file.path);
    }
    let vad = config.vad_options.unwrap_or_default();
    if vad.enabled && config.vad_model.is_none() {
        return Err("VADモデルがありません".into());
    }
    c.arg("--vad-enabled")
        .arg(if vad.enabled { "1" } else { "0" })
        .arg("--vad")
        .arg(
            config
                .vad_model
                .as_ref()
                .map(|f| f.path.as_os_str())
                .unwrap_or_default(),
        )
        .arg("--vad-threshold")
        .arg((vad.threshold_milli as f64 / 1000.0).to_string())
        .arg("--vad-min-speech")
        .arg((vad.min_speech_duration_ms as f64 / 1000.0).to_string())
        .arg("--vad-min-silence")
        .arg((vad.min_silence_duration_ms as f64 / 1000.0).to_string())
        .arg("--vad-max-speech")
        .arg(vad.max_speech_duration_s.to_string())
        .arg("--vad-pad")
        .arg((vad.speech_pad_ms as f64 / 1000.0).to_string());
    crate::local_engine::native_environment(c);
    let mut child = crate::local_engine::resident::Run::spawn(
        &mut launch,
        asr::adapter(config)?.descriptor().worker,
        &directory,
        "stdout.log",
        "stderr.log",
    )?;
    let started = Instant::now();
    loop {
        if cancelled() {
            return Ok(None);
        }
        if PROCESS_POLICY.expired(started.elapsed(), None) {
            return Err("ASR処理が制限時間を超えました".into());
        }
        for (name, max) in [
            ("stdout.log", PROCESS_POLICY.stdout_bytes),
            ("stderr.log", PROCESS_POLICY.stderr_bytes),
            ("result.json", PROCESS_POLICY.result_bytes),
        ] {
            if directory.join(name).metadata().is_ok_and(|m| m.len() > max) {
                return Err("ASR出力が上限を超えました".into());
            }
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(format!("ASRエンジンが失敗しました: {status}"));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let bytes =
        crate::restored_processing::read_local(&result, PROCESS_POLICY.result_bytes, cancelled)?;
    for file in watched {
        file.verify()?;
    }
    input.proof.verify(cancelled)?;
    Ok(Some(parse(&bytes, config, duration)?))
}
