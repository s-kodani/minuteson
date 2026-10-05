pub use crate::local_engine::Fingerprint;
use crate::local_engine::{ManagedChild, StableFile};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WhisperConfig {
    pub executable: Fingerprint,
    pub model: Fingerprint,
    pub vad_model: Option<Fingerprint>,
    pub threads: u32,
    pub use_gpu: bool,
    pub prompt: String,
}

impl WhisperConfig {
    pub(crate) fn validate_settings(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.threads) || self.prompt.chars().count() > 8000 {
            return Err("文字起こしのスレッド数または用語ヒントが不正です".into());
        }
        Ok(())
    }
    pub fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.validate_settings()?;
        self.executable.verify(cancelled)?;
        self.model.verify(cancelled)?;
        if let Some(vad) = &self.vad_model {
            vad.verify(cancelled)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
}

#[derive(Deserialize)]
struct Output {
    model: Model,
    params: Params,
    transcription: Vec<RawSegment>,
}
#[derive(Deserialize)]
struct Model {
    multilingual: bool,
}
#[derive(Deserialize)]
struct Params {
    model: PathBuf,
    language: String,
    translate: bool,
}
#[derive(Deserialize)]
struct RawSegment {
    offsets: Offsets,
    text: String,
}
#[derive(Deserialize)]
struct Offsets {
    from: i64,
    to: i64,
}

// CLI offsets are milliseconds, not Whisper's 10 ms internal timestamp units.
// Schema: https://github.com/ggml-org/whisper.cpp/blob/master/examples/cli/cli.cpp
pub fn parse_output(bytes: &[u8], model: &Path, duration_us: i64) -> Result<Vec<Segment>, String> {
    if bytes.len() as u64 > MAX_JSON_BYTES || duration_us <= 0 {
        return Err("文字起こし出力のサイズまたは音声時間が不正です".into());
    }
    let output: Output =
        serde_json::from_slice(bytes).map_err(|e| format!("文字起こしJSONが不正です: {e}"))?;
    if !output.model.multilingual
        || output.params.language != "ja"
        || output.params.translate
        || output.params.model != model
    {
        return Err("多言語モデル・日本語・非翻訳の固定設定と実行結果が一致しません".into());
    }
    if output.transcription.len() > 100_000 {
        return Err("文字起こし区間数が多すぎます".into());
    }
    let mut previous = -1;
    let mut result = Vec::new();
    for raw in output.transcription {
        let start = raw
            .offsets
            .from
            .checked_mul(1000)
            .ok_or("文字起こし時刻が範囲外です")?;
        let end = raw
            .offsets
            .to
            .checked_mul(1000)
            .ok_or("文字起こし時刻が範囲外です")?;
        // A small terminal timestamp quantization overrun is clipped, not used
        // to invent audio beyond the input. Larger overruns are rejected.
        if start < 0
            || start < previous
            || end <= start
            || start >= duration_us
            || end > duration_us.saturating_add(100_000)
            || raw.text.chars().count() > 20_000
        {
            return Err("文字起こし区間の時刻または本文が不正です".into());
        }
        previous = start;
        let text = raw.text.trim();
        if !text.is_empty() {
            result.push(Segment {
                start_us: start,
                end_us: end.min(duration_us),
                text: text.into(),
            });
        }
    }
    Ok(result)
}

// A unique, task-owned unit directory is required. Existing artifacts are never
// overwritten. None means pause: the unfinished unit must not be checkpointed.
// Normalize only machine-generated model-path metadata after the physical
// output has passed the native parser. Save a separate logical result file;
// the original native JSON remains intact for inspection.
pub(crate) fn logical_output(
    bytes: &[u8],
    runtime: &Path,
    logical: &Path,
    duration: i64,
) -> Result<Vec<u8>, String> {
    let segments = parse_output(bytes, runtime, duration)?;
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    value["params"]["model"] = serde_json::to_value(logical).map_err(|e| e.to_string())?;
    let normalized = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    if parse_output(&normalized, logical, duration)? != segments {
        return Err("モデルパスの再結合で認識本文・時刻が変化しました".into());
    }
    Ok(normalized)
}

pub fn run_unit(
    config: &WhisperConfig,
    directory: &Path,
    duration_us: i64,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<Segment>>, String> {
    if cancelled() {
        return Ok(None);
    }
    if !(1..=64).contains(&config.threads) || config.prompt.chars().count() > 8000 {
        return Err("文字起こし設定が不正です".into());
    }
    let mut watched = vec![
        StableFile::watch(&config.executable)?,
        StableFile::watch(&config.model)?,
    ];
    if let Some(vad) = &config.vad_model {
        watched.push(StableFile::watch(vad)?);
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let wav = directory.join("input.wav");
    if !wav.is_file() {
        return Err("文字起こし入力WAVがありません".into());
    }
    let input_proof = Fingerprint::capture(&wav, cancelled)?;
    let output_base = directory.join("result");
    let json = directory.join("result.json");
    if json.exists() {
        return Err("この処理単位には既存の結果があります".into());
    }
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
        // Do not inherit injected libraries, credentials, or engine overrides.
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8")
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .arg("--model")
        .arg(&config.model.path)
        .arg("--file")
        .arg(wav)
        .args(["--language", "ja", "--output-json", "--output-file"])
        .arg(output_base)
        .arg("--threads")
        .arg(config.threads.to_string())
        .args(["--beam-size", "5"]);
    if !config.use_gpu {
        command.arg("--no-gpu");
    }
    if !config.prompt.is_empty() {
        command.arg("--prompt").arg(&config.prompt);
    }
    if let Some(vad) = &config.vad_model {
        command.arg("--vad").arg("--vad-model").arg(&vad.path);
    }
    let mut child = ManagedChild(
        command
            .spawn()
            .map_err(|e| format!("whisper.cppを起動できません: {e}"))?,
    );
    let started = Instant::now();
    loop {
        if cancelled() {
            return Ok(None);
        }
        if started.elapsed() > timeout {
            return Err("文字起こし処理が制限時間を超えました".into());
        }
        for path in [&stdout_path, &stderr_path] {
            if path.metadata().map_err(|e| e.to_string())?.len() > MAX_LOG_BYTES {
                return Err("文字起こしログが上限を超えました".into());
            }
        }
        if json.exists() && json.metadata().map_err(|e| e.to_string())?.len() > MAX_JSON_BYTES {
            return Err("文字起こし出力が上限を超えました".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("whisper.cppが失敗しました: {status}"));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let file = File::open(&json).map_err(|e| format!("文字起こし結果JSONがありません: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.len() > MAX_JSON_BYTES
        || json.canonicalize().map_err(|e| e.to_string())? != json
    {
        return Err("文字起こし結果が通常ファイルではないか範囲外です".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if cancelled() {
        return Ok(None);
    }
    for file in &watched {
        file.verify()?;
    }
    input_proof.verify(cancelled)?;
    Ok(Some(parse_output(&bytes, &config.model.path, duration_us)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn output() -> serde_json::Value {
        json!({"model":{"multilingual":true}, "params":{"model":"/model.bin","language":"ja","translate":false},"transcription":[{"offsets":{"from":125,"to":1000},"text":" 点検日 "}]})
    }

    #[test]
    fn model_rebinding_normalizes_only_the_verified_model_path_and_retains_raw_content() {
        let mut original = output();
        original["extra"] = json!({"retained":"原文"});
        let bytes = serde_json::to_vec(&original).unwrap();
        let unavailable = std::path::PathBuf::from(format!("mon-unavailable:{}", "a".repeat(64)));
        let normalized =
            logical_output(&bytes, Path::new("/model.bin"), &unavailable, 1_000_000).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(value["transcription"], original["transcription"]);
        assert_eq!(value["extra"], original["extra"]);
        assert_eq!(value["params"]["model"], json!(unavailable));
        assert_eq!(
            parse_output(&normalized, &unavailable, 1_000_000).unwrap(),
            parse_output(&bytes, Path::new("/model.bin"), 1_000_000).unwrap()
        );
        assert!(logical_output(&bytes, Path::new("/wrong.bin"), &unavailable, 1_000_000).is_err());
        original["params"]["translate"] = json!(true);
        assert!(logical_output(
            &serde_json::to_vec(&original).unwrap(),
            Path::new("/model.bin"),
            &unavailable,
            1_000_000
        )
        .is_err());
    }

    #[test]
    fn validates_cli_identity_time_units_and_empty_audio() {
        let path = Path::new("/model.bin");
        assert_eq!(
            parse_output(&serde_json::to_vec(&output()).unwrap(), path, 1_000_000).unwrap(),
            vec![Segment {
                start_us: 125_000,
                end_us: 1_000_000,
                text: "点検日".into()
            }]
        );
        for (key, value) in [
            ("language", json!("en")),
            ("translate", json!(true)),
            ("model", json!("/other.bin")),
        ] {
            let mut wrong = output();
            wrong["params"][key] = value;
            assert!(parse_output(&serde_json::to_vec(&wrong).unwrap(), path, 1_000_000).is_err());
        }
        let mut wrong = output();
        wrong["model"]["multilingual"] = json!(false);
        assert!(parse_output(&serde_json::to_vec(&wrong).unwrap(), path, 1_000_000).is_err());
        for (from, to) in [(-1, 100), (10, 10), (1001, 1010), (10, 1200), (0, i64::MAX)] {
            let mut wrong = output();
            wrong["transcription"][0]["offsets"] = json!({"from":from,"to":to});
            assert!(parse_output(&serde_json::to_vec(&wrong).unwrap(), path, 1_000_000).is_err());
        }
        let mut empty = output();
        empty["transcription"] = json!([]);
        assert!(
            parse_output(&serde_json::to_vec(&empty).unwrap(), path, 1_000_000)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fixed_files_are_verified_again_after_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.bin");
        std::fs::write(&path, b"model").unwrap();
        let fingerprint = Fingerprint::capture(&path, &|| false).unwrap();
        fingerprint.verify(&|| false).unwrap();
        std::fs::write(path, b"other").unwrap();
        assert!(fingerprint.verify(&|| false).is_err());
        assert!(Fingerprint::capture(Path::new("relative.bin"), &|| false).is_err());
    }

    #[test]
    #[ignore = "requires MON_WHISPER_CLI, MON_WHISPER_MODEL and MON_WHISPER_INPUT_CAF"]
    fn real_whisper_cli_reads_converted_caf_and_emits_valid_japanese_segments() {
        use crate::audio::VerifiedAudio;
        use crate::playback::{AudioSpan, PreparedAudio};
        let cli = PathBuf::from(std::env::var("MON_WHISPER_CLI").unwrap());
        let model = PathBuf::from(std::env::var("MON_WHISPER_MODEL").unwrap());
        let input = PathBuf::from(std::env::var("MON_WHISPER_INPUT_CAF").unwrap());
        let input_proof = Fingerprint::capture(&input, &|| false).unwrap();
        let verified = VerifiedAudio::open(
            File::open(input).unwrap(),
            input_proof.size,
            &input_proof.sha256,
        )
        .unwrap();
        let frames = verified.format.frames;
        let duration = (frames * 1_000_000 / u64::from(verified.format.sample_rate)) as i64;
        let mut audio = PreparedAudio {
            sample_rate: verified.format.sample_rate,
            channels: verified.format.channels,
            start_us: 0,
            end_us: duration,
            track_kind: "imported".into(),
            alignment: "unverified".into(),
            spans: vec![AudioSpan {
                audio: verified,
                first_frame: 0,
                frames,
            }],
        };
        let config = WhisperConfig {
            executable: Fingerprint::capture(&cli, &|| false).unwrap(),
            model: Fingerprint::capture(&model, &|| false).unwrap(),
            vad_model: None,
            threads: 4,
            use_gpu: false,
            prompt: String::new(),
        };
        config.verify(&|| false).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let converted = super::super::wave::write_window(
            &mut audio,
            0,
            frames,
            &directory.path().join("input.wav"),
            &|| false,
        )
        .unwrap();
        assert_eq!(converted, (duration as u64 * 16_000 + 500_000) / 1_000_000);
        let segments = run_unit(
            &config,
            directory.path(),
            duration,
            Duration::from_secs(180),
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert!(!segments.is_empty());
        assert!(segments.iter().all(|segment| segment.start_us >= 0
            && segment.end_us <= duration
            && segment.end_us > segment.start_us));
        config.verify(&|| false).unwrap();
        // No transcript is printed: the input may contain private meeting text.
        eprintln!(
            "real whisper.cpp integration: {} segments, input {} us, CLI SHA {}, model SHA {}",
            segments.len(),
            duration,
            config.executable.sha256,
            config.model.sha256
        );
    }

    #[cfg(unix)]
    #[test]
    fn changed_model_during_execution_is_not_checkpointable() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("model.bin");
        std::fs::write(&model, b"model").unwrap();
        let proof = Fingerprint::capture(&model, &|| false).unwrap();
        let executable = directory.path().join("fake-cli");
        let mut result = output();
        result["params"]["model"] = json!(proof.path);
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf changed > '{}'\nprintf '%s' '{}' > result.json\n",
                proof.path.display(),
                result
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = WhisperConfig {
            executable: Fingerprint::capture(&executable, &|| false).unwrap(),
            model: proof,
            vad_model: None,
            threads: 1,
            use_gpu: false,
            prompt: String::new(),
        };
        let unit = directory.path().join("unit");
        std::fs::create_dir(&unit).unwrap();
        std::fs::write(unit.join("input.wav"), b"WAV").unwrap();
        assert!(
            run_unit(&config, &unit, 1_000_000, Duration::from_secs(3), &|| false)
                .unwrap_err()
                .contains("変更")
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_requires_json_and_is_reaped_on_pause_and_timeout() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-cli");
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let model = directory.path().join("model.bin");
        std::fs::write(&model, b"model").unwrap();
        let mut config = WhisperConfig {
            executable: Fingerprint::capture(&executable, &|| false).unwrap(),
            model: Fingerprint::capture(&model, &|| false).unwrap(),
            vad_model: None,
            threads: 1,
            use_gpu: false,
            prompt: "点検日; $(false)".into(),
        };
        config.verify(&|| false).unwrap();
        let unit = directory.path().join("unit");
        std::fs::create_dir(&unit).unwrap();
        std::fs::write(unit.join("input.wav"), b"WAV").unwrap();
        assert!(
            run_unit(&config, &unit, 1_000_000, Duration::from_secs(3), &|| false)
                .unwrap_err()
                .contains("JSON")
        );
        std::fs::write(
            &executable,
            b"#!/bin/sh\nprintf '%s' $$ > child.pid\nexec /bin/sleep 30\n",
        )
        .unwrap();
        config.executable = Fingerprint::capture(&executable, &|| false).unwrap();
        for pause in [true, false] {
            let unit = directory
                .path()
                .join(if pause { "paused" } else { "timeout" });
            std::fs::create_dir(&unit).unwrap();
            std::fs::write(unit.join("input.wav"), b"WAV").unwrap();
            let started = Instant::now();
            let result = run_unit(
                &config,
                &unit,
                1_000_000,
                Duration::from_millis(350),
                &|| pause && started.elapsed() > Duration::from_millis(200),
            );
            if pause {
                assert_eq!(result.unwrap(), None);
            } else {
                assert!(result.unwrap_err().contains("制限時間"));
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            let pid = std::fs::read_to_string(unit.join("child.pid")).unwrap();
            assert!(!Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success());
        }
    }
}
