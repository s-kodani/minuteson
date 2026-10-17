use crate::local_engine::{Fingerprint, OwnedCommand, StableFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
#[cfg(test)]
use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROMPT_BYTES: usize = 64 * 1024;

// Qwen3's published non-thinking ChatML generation prefix. b11533 completion
// does not pass its --reasoning option into the initial template application,
// so fix the no-thinking prefix explicitly rather than stripping model output.
const QWEN3_NON_THINKING_TEMPLATE: &str = r#"{%- for message in messages %}{{- '<|im_start|>' + message.role + '\n' + message.content + '<|im_end|>\n' }}{%- endfor %}{%- if add_generation_prompt %}{{- '<|im_start|>assistant\n<think>\n\n</think>\n\n' }}{%- endif %}"#;
fn model_arguments(model: &Fingerprint) -> &'static [&'static str] {
    if matches!(
        (model.size, model.sha256.as_str()),
        (
            5_027_783_488,
            "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785"
        ) | (
            2_497_280_256,
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"
        )
    ) {
        &[
            "--temp",
            "0.7",
            "--top-p",
            "0.8",
            "--top-k",
            "20",
            "--min-p",
            "0",
            "--presence-penalty",
            "1.5",
            "--chat-template",
            QWEN3_NON_THINKING_TEMPLATE,
        ]
    } else {
        &["--temp", "0"]
    }
}

// Value's default object decoder silently keeps the last duplicate field.
// Reject duplicates at every level before downstream typed deserialization.
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate fields")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut result = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if result.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON field"));
                    }
                    result.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(result)))
            }
            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> Result<Self::Value, S::Error> {
                let mut result = vec![];
                while let Some(value) = seq.next_element::<StrictValue>()? {
                    result.push(value.0);
                }
                Ok(StrictValue(Value::Array(result)))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v.into())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct LlamaConfig {
    // llama-completion, not the newer interactive/server-backed llama-cli.
    pub executable: Fingerprint,
    pub model: Fingerprint,
    pub threads: u32,
    pub context_tokens: u32,
    pub prediction_tokens: u32,
    pub gpu_layers: u32,
    pub seed: u32,
}
impl LlamaConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.threads)
            || !(2048..=40960).contains(&self.context_tokens)
            || !(64..=32768).contains(&self.prediction_tokens)
            || self.prediction_tokens > self.context_tokens - 256
            || self.gpu_layers > 256
        {
            return Err("議事録エンジンのスレッド数・コンテキスト・出力上限が不正です".into());
        }
        Ok(())
    }
    #[allow(dead_code)]
    pub(crate) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.validate()?;
        self.executable.verify(cancelled)?;
        self.model.verify(cancelled)
    }
}

// Only the official completion adapter's exact end marker is allowed after
// one JSON object. Never search arbitrary logs for a plausible JSON substring,
// remove thinking traces, repair truncated output, or accept a second object.
// Some completion builds emit literal line breaks/tabs inside JSON strings.
// Escape only those whitespace bytes, preserving the decoded text exactly;
// never change structure, quote boundaries, escapes or other control bytes.
fn framed_json(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let (mut quoted, mut escaped) = (false, false);
    for &byte in bytes {
        if quoted && !escaped {
            match byte {
                b'\n' => {
                    result.extend_from_slice(b"\\n");
                    continue;
                }
                b'\r' => {
                    result.extend_from_slice(b"\\r");
                    continue;
                }
                b'\t' => {
                    result.extend_from_slice(b"\\t");
                    continue;
                }
                _ => {}
            }
        }
        result.push(byte);
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        }
    }
    result
}
pub(crate) fn parse_output(bytes: &[u8]) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("議事録出力のサイズが不正です".into());
    }
    let framed = framed_json(bytes);
    let mut stream = serde_json::Deserializer::from_slice(&framed).into_iter::<StrictValue>();
    let value = stream
        .next()
        .ok_or("議事録のJSON出力がありません")?
        .map_err(|e| if e.is_eof() {
            "議事録の生成が途中で終了しました。未完成の本文は保存していません。議事録を再生成してください。（minutes_output_incomplete）".to_string()
        } else { format!("議事録JSONが不正です: {e}") })?
        .0;
    let tail = std::str::from_utf8(&framed[stream.byte_offset()..])
        .map_err(|_| "議事録出力がUTF-8ではありません")?
        .trim();
    if !value.is_object() || (!tail.is_empty() && tail != "[end of text]") {
        return Err("議事録出力に会話ログ・余分な内容があります".into());
    }
    Ok(value)
}

pub(super) fn validate_markdown_bound(value: &Value, schema: &Value) -> Result<(), String> {
    if let (Some(text), Some(limit)) = (
        value["markdown"].as_str(),
        schema["properties"]["markdown"]["maxLength"].as_u64(),
    ) {
        if text.chars().count() as u64 > limit {
            return Err("議事録本文が固定の生成上限を超えています。未完成・上限超過の本文は保存しません。（minutes_output_limit）".into());
        }
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

// A fresh task-owned directory is mandatory. Partial logs are retained, but
// cancellation never publishes a usable result or a durable checkpoint.
#[allow(dead_code)]
pub(crate) fn run_unit(
    config: &LlamaConfig,
    directory: &Path,
    system_prompt: &str,
    prompt: &str,
    schema: &Value,
    idle_timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Value>, String> {
    run_unit_observed(
        config,
        directory,
        system_prompt,
        prompt,
        schema,
        idle_timeout,
        cancelled,
        &|_| {},
    )
}
pub(crate) fn run_unit_observed(
    config: &LlamaConfig,
    directory: &Path,
    system_prompt: &str,
    prompt: &str,
    schema: &Value,
    idle_timeout: Duration,
    cancelled: &impl Fn() -> bool,
    observe: &impl Fn(super::llama_metrics::Phase),
) -> Result<Option<Value>, String> {
    run_unit_observed_on_device(
        config,
        directory,
        system_prompt,
        prompt,
        schema,
        idle_timeout,
        cancelled,
        observe,
        None,
    )
}
pub(crate) fn run_unit_observed_on_device(
    config: &LlamaConfig,
    directory: &Path,
    system_prompt: &str,
    prompt: &str,
    schema: &Value,
    idle_timeout: Duration,
    cancelled: &impl Fn() -> bool,
    observe: &impl Fn(super::llama_metrics::Phase),
    device: Option<&str>,
) -> Result<Option<Value>, String> {
    use super::llama_metrics::{Metrics, Phase};
    if device.is_some_and(|v| v != "MTL0" || config.gpu_layers == 0) {
        return Err("GPUデバイスの固定条件が不正です".into());
    }
    let started = Instant::now();
    observe(Phase::Verifying);
    if cancelled() {
        return Ok(None);
    }
    config.validate()?;
    let schema_bytes = serde_json::to_vec(schema).map_err(|e| e.to_string())?;
    if system_prompt.trim().is_empty()
        || prompt.trim().is_empty()
        || system_prompt.len().saturating_add(prompt.len()) > MAX_PROMPT_BYTES
        || system_prompt.contains('\0')
        || prompt.contains('\0')
        || !schema.is_object()
        || schema_bytes.len() > 32 * 1024
        || idle_timeout.is_zero()
        || idle_timeout > Duration::from_secs(1800)
    {
        return Err("議事録の入力・schema・制限時間が不正です".into());
    }
    if let Err(error) = config.verify(cancelled) {
        return if cancelled() { Ok(None) } else { Err(error) };
    }
    let watched = [
        StableFile::watch(&config.executable)?,
        StableFile::watch(&config.model)?,
    ];
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    for name in [
        "system.txt",
        "prompt.txt",
        "schema.json",
        "result.json",
        "stderr.log",
    ] {
        if directory.join(name).symlink_metadata().is_ok() {
            return Err("議事録の処理単位に既存ファイルがあります".into());
        }
    }
    let system_path = directory.join("system.txt");
    let prompt_path = directory.join("prompt.txt");
    let schema_path = directory.join("schema.json");
    write_new(&system_path, system_prompt.as_bytes())?;
    write_new(&prompt_path, prompt.as_bytes())?;
    write_new(&schema_path, &schema_bytes)?;
    let proofs = [&system_path, &prompt_path, &schema_path]
        .into_iter()
        .map(|p| Fingerprint::capture(p, cancelled))
        .collect::<Result<Vec<_>, _>>();
    let proofs = match proofs {
        Ok(proofs) => proofs,
        Err(error) => return if cancelled() { Ok(None) } else { Err(error) },
    };
    let result_path = directory.join("result.json");
    let log_path = directory.join("stderr.log");
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&result_path)
        .map_err(|e| e.to_string())?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let _log_capture =
        crate::internal_logs::watch_engine(&directory, &["stderr.log"], "minutes_generation");
    let mut launch = OwnedCommand::new(&config.executable.path)?;
    let command = &mut launch.command;
    command
        .current_dir(&directory)
        // Do not inherit LLAMA_ARG_RPC, model URLs, credentials or options that
        // could silently override the fixed job or enable remote execution.
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8")
        .stdin(Stdio::null())
        .stdout(result)
        .stderr(log)
        .arg("--model")
        .arg(&config.model.path)
        .arg("--file")
        .arg(&prompt_path)
        .arg("--system-prompt-file")
        .arg(&system_path)
        .arg("--json-schema-file")
        .arg(&schema_path)
        .arg("--threads")
        .arg(config.threads.to_string())
        .arg("--threads-batch")
        .arg(config.threads.to_string())
        .arg("--ctx-size")
        .arg(config.context_tokens.to_string())
        .arg("--predict")
        .arg(config.prediction_tokens.to_string())
        .arg("--gpu-layers")
        .arg(config.gpu_layers.to_string())
        .arg("--seed")
        .arg(config.seed.to_string())
        .args(model_arguments(&config.model))
        .args([
            "--conversation",
            "--jinja",
            "--single-turn",
            "--no-display-prompt",
            "--no-context-shift",
            "--no-escape",
            "--offline",
            "--no-warmup",
            // Job limits are fixed; the CLI must not auto-fit them to a device.
            "--fit",
            "off",
            "--reasoning",
            "off",
            "--color",
            "off",
            "--log-colors",
            "off",
            "--simple-io",
        ]);
    command.args(["--log-verbosity", if device.is_some() { "4" } else { "3" }]);
    if let Some(device) = device {
        command.args(["--device", device]);
    }
    if config.gpu_layers == 0 {
        // Zero model layers alone still permits Metal KV/host-op offload in
        // official shared builds. CPU trials and CPU jobs must stay on CPU.
        command.args(["--device", "none", "--no-kv-offload", "--no-op-offload"]);
    }
    let verification = started.elapsed();
    observe(Phase::Starting);
    let spawning = Instant::now();
    let mut child = launch
        .spawn()
        .map_err(|e| format!("llama.cppを起動できません: {e}"))?;
    // A CPU job may legitimately need longer than 30 minutes. Bound lack of
    // output, not total wall time; --predict and byte limits still bound work.
    // Only result growth counts: a noisy stderr must not keep a stuck job alive.
    let spawn = spawning.elapsed();
    observe(Phase::Input);
    let mut first_output = None;
    let mut notified = Instant::now();
    let mut last_output = Instant::now();
    let mut output_bytes = 0;
    loop {
        if cancelled() {
            return Ok(None);
        }
        for (path, limit) in [(&result_path, MAX_OUTPUT_BYTES), (&log_path, MAX_LOG_BYTES)] {
            let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > limit {
                return Err("議事録の出力・ログが上限を超えたか通常ファイルではありません".into());
            }
            if path == &result_path && meta.len() > output_bytes {
                if first_output.is_none() {
                    first_output = Some(started.elapsed());
                    observe(Phase::Generating);
                }
                output_bytes = meta.len();
                last_output = Instant::now();
            }
        }
        if notified.elapsed() >= Duration::from_secs(1) {
            observe(if first_output.is_some() {
                Phase::Generating
            } else {
                Phase::Input
            });
            notified = Instant::now();
        }
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("llama.cppが失敗しました: {status}"));
            }
            break;
        }
        if last_output.elapsed() > idle_timeout {
            return Err("議事録の出力が30分間更新されなかったため処理を停止しました。完了済みの解析結果は保持しています。（minutes_generation_stalled）".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if cancelled() {
        return Ok(None);
    }
    observe(Phase::Saving);
    let validating = Instant::now();
    for file in &watched {
        file.verify()?;
    }
    for proof in &proofs {
        if let Err(error) = proof.verify(cancelled) {
            return if cancelled() { Ok(None) } else { Err(error) };
        }
    }
    let file = File::open(&result_path).map_err(|e| e.to_string())?;
    if result_path.canonicalize().map_err(|e| e.to_string())? != result_path {
        return Err("議事録出力が処理ディレクトリ外を参照しています".into());
    }
    let mut bytes = vec![];
    file.take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if let Some(device) = device {
        let log = std::fs::read_to_string(&log_path).map_err(|e| e.to_string())?;
        if !super::llama_metrics::metal_matches(&log, device, config.gpu_layers) {
            return Err("固定したMetal GPUの利用を確認できません。処理済みの結果を保持しています。新しく生成してGPU設定を確認してください。（minutes_gpu_unavailable）".into());
        }
    }
    let value = parse_output(&bytes)?;
    validate_markdown_bound(&value, schema)?;
    // Freeze exactly the validated adapter result, including the fixed CLI's
    // optional end marker. The scheduler later registers and hashes this file.
    File::open(&result_path)
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    if cancelled() {
        return Ok(None);
    }
    let log_text = std::fs::read_to_string(&log_path).unwrap_or_default();
    let metrics = Metrics::new(
        config,
        &log_text,
        system_prompt.len() + prompt.len(),
        started.elapsed(),
        verification,
        spawn,
        first_output,
        validating.elapsed(),
    );
    // Optional diagnostics cannot turn a valid, durable result into a failure.
    if let Ok(bytes) = serde_json::to_vec_pretty(&metrics) {
        let _ = write_new(&directory.join("performance.json"), &bytes);
    }
    Ok(Some(value))
}

#[cfg(test)]
#[path = "llama_tests.rs"]
mod tests;
