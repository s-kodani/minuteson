use crate::local_engine::{Fingerprint, ManagedChild, StableFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROMPT_BYTES: usize = 64 * 1024;

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
            || !(2048..=32768).contains(&self.context_tokens)
            || !(64..=8192).contains(&self.prediction_tokens)
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
pub(crate) fn parse_output(bytes: &[u8]) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("議事録出力のサイズが不正です".into());
    }
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<StrictValue>();
    let value = stream
        .next()
        .ok_or("議事録のJSON出力がありません")?
        .map_err(|e| format!("議事録JSONが不正です: {e}"))?
        .0;
    let tail = std::str::from_utf8(&bytes[stream.byte_offset()..])
        .map_err(|_| "議事録出力がUTF-8ではありません")?
        .trim();
    if !value.is_object() || (!tail.is_empty() && tail != "[end of text]") {
        return Err("議事録出力に会話ログ・余分な内容があります".into());
    }
    Ok(value)
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
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Value>, String> {
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
        || timeout.is_zero()
        || timeout > Duration::from_secs(1800)
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
    let mut command = Command::new(&config.executable.path);
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
        .args([
            "--temp",
            "0",
            "--conversation",
            "--jinja",
            "--single-turn",
            "--no-display-prompt",
            "--no-context-shift",
            "--no-escape",
            "--offline",
            "--no-warmup",
            "--reasoning",
            "off",
            "--color",
            "off",
            "--log-colors",
            "off",
            "--log-verbosity",
            "2",
            "--simple-io",
        ]);
    let mut child = ManagedChild(
        command
            .spawn()
            .map_err(|e| format!("llama.cppを起動できません: {e}"))?,
    );
    let started = Instant::now();
    loop {
        if cancelled() {
            return Ok(None);
        }
        if started.elapsed() > timeout {
            return Err("議事録生成が制限時間を超えました".into());
        }
        for (path, limit) in [(&result_path, MAX_OUTPUT_BYTES), (&log_path, MAX_LOG_BYTES)] {
            let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > limit {
                return Err("議事録の出力・ログが上限を超えたか通常ファイルではありません".into());
            }
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("llama.cppが失敗しました: {status}"));
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
    let value = parse_output(&bytes)?;
    // Freeze exactly the validated adapter result, including the fixed CLI's
    // optional end marker. The scheduler later registers and hashes this file.
    File::open(&result_path)
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(value))
}

#[cfg(test)]
#[path = "llama_tests.rs"]
mod tests;
