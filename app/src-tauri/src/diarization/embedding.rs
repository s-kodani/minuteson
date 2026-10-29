//! Bounded, task-owned headless feature extraction for saved diarization runs.
//! Does not record, change source PCM, or infer identity from local labels.
use super::sherpa::SherpaConfig;
use crate::local_engine::{Fingerprint, OwnedCommand, StableFile};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

const PROCESS_POLICY: mon_native::process::Policy =
    mon_native::process::Purpose::Embedding.policy();
const MAX_OUTPUT: u64 = PROCESS_POLICY.stdout_bytes;
const MAX_LOG: u64 = PROCESS_POLICY.stderr_bytes;
const MAX_DIMENSION: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ExtractorConfig {
    pub library_index: usize,
    pub helper: Fingerprint,
    pub library: Fingerprint,
}

impl ExtractorConfig {
    pub(crate) fn capture(helper: &Path, engine: &SherpaConfig) -> Result<Self, String> {
        let matches: Vec<_> = engine
            .runtime_libraries
            .iter()
            .filter(|proof| {
                proof.path.file_name().is_some_and(|name| {
                    name == if cfg!(windows) {
                        "sherpa-onnx-c-api.dll"
                    } else {
                        "libsherpa-onnx-c-api.dylib"
                    }
                })
            })
            .collect();
        if matches.is_empty() {
            return Err("この話者分離モデルは声の登録に必要な部品を含まない旧構成です。「設定」の「ローカルAI」で話者分離モデルの「ダウンロードして使用」を実行してから、声を登録してください".into());
        }
        if matches.len() != 1 {
            return Err("固定した話者エンジンに一意のC APIランタイムが必要です".into());
        }
        let config = Self {
            library_index: engine
                .runtime_libraries
                .iter()
                .position(|p| std::ptr::eq(p, matches[0]))
                .ok_or("抽出ランタイムの位置がありません")?,
            helper: Fingerprint::capture(helper, &|| false)?,
            library: matches[0].clone(),
        };
        config.validate(engine)?;
        Ok(config)
    }

    fn validate(&self, engine: &SherpaConfig) -> Result<(), String> {
        if !cfg!(any(target_os = "macos", windows)) {
            return Err("保存区間の特徴量抽出は現在macOSだけに対応しています".into());
        }
        engine.validate_settings(None)?;
        if engine.runtime_libraries.get(self.library_index) != Some(&self.library)
            || engine
                .runtime_libraries
                .iter()
                .filter(|p| *p == &self.library)
                .count()
                != 1
        {
            return Err("特徴量抽出のランタイムが固定エンジンと一致しません".into());
        }
        for proof in [&self.helper, &self.library] {
            if !proof.path.is_absolute()
                || proof
                    .path
                    .to_str()
                    .is_none_or(|text| text.len() > 4096 || text.contains(['\0', '\n', '\r']))
                || proof.size == 0
                || proof.sha256.len() != 64
                || !proof
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("特徴量抽出の固定指紋が不正です".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnavailableReason {
    InsufficientAudio,
    NonFiniteEmbedding,
    ZeroEmbedding,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Output {
    schema_version: u32,
    engine: String,
    engine_version: String,
    sample_rate: u32,
    first_frame: u64,
    end_frame: u64,
    pub dimension: usize,
    status: Status,
    #[serde(deserialize_with = "required_option")]
    pub reason: Option<UnavailableReason>,
    #[serde(deserialize_with = "required_option")]
    vector_bits: Option<Vec<String>>,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

impl Output {
    // Also used when loading a previously fingerprinted result. Absence of
    // evidence remains unavailable, not a zero/synthetic identity vector.
    pub(crate) fn vector(&self, first: u64, end: u64) -> Result<Option<Vec<f32>>, String> {
        if self.schema_version != 1
            || self.engine != "sherpa-onnx"
            || self.engine_version != "1.13.8"
            || self.sample_rate != 16_000
            || self.first_frame != first
            || self.end_frame != end
            || first >= end
            || end - first > 480_000
            || !(1..=MAX_DIMENSION).contains(&self.dimension)
        {
            return Err("特徴量出力の形式・時間範囲・エンジンが一致しません".into());
        }
        match (&self.status, &self.reason, &self.vector_bits) {
            (Status::Unavailable, Some(_), None) => Ok(None),
            (Status::Ready, None, Some(bits)) if bits.len() == self.dimension => {
                let vector: Vec<_> = bits
                    .iter()
                    .map(|bits| {
                        if bits.len() != 8
                            || !bits
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        {
                            return Err("特徴量の数値表現が不正です".to_string());
                        }
                        let value = f32::from_bits(
                            u32::from_str_radix(bits, 16).map_err(|e| e.to_string())?,
                        );
                        if !value.is_finite() {
                            return Err("特徴量に有限でない値が含まれています".into());
                        }
                        Ok(value)
                    })
                    .collect::<Result<_, _>>()?;
                let norm = vector
                    .iter()
                    .map(|value| f64::from(*value).powi(2))
                    .sum::<f64>()
                    .sqrt();
                if !norm.is_finite() || norm <= 1e-12 {
                    return Err("特徴量の長さが不正です".into());
                }
                Ok(Some(vector))
            }
            _ => Err("特徴量の取得状態と数値が一致しません".into()),
        }
    }
}

pub(crate) fn parse(bytes: &[u8], first: u64, end: u64) -> Result<Output, String> {
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err("特徴量出力が上限を超えました".into());
    }
    let result: Output = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    result.vector(first, end)?;
    Ok(result)
}

struct Log {
    path: std::path::PathBuf,
    file: File,
    identity: std::fs::Metadata,
    limit: u64,
}
impl Log {
    fn create(path: std::path::PathBuf, limit: u64) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let identity = file.metadata().map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            file,
            identity,
            limit,
        })
    }
    fn check(&self) -> Result<(), String> {
        let current = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !current.is_file()
            || current.len() > self.limit
            || self.file.metadata().map_err(|e| e.to_string())?.len() > self.limit
            || self.path.canonicalize().map_err(|e| e.to_string())? != self.path
        {
            return Err("特徴量出力が上限を超えたか差し替えられました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if current.dev() != self.identity.dev() || current.ino() != self.identity.ino() {
                return Err("特徴量出力が差し替えられました".into());
            }
        }
        Ok(())
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        self.check()?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&self.file)
            .take(self.limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        self.check()?;
        if bytes.len() as u64 > self.limit {
            return Err("特徴量出力が上限を超えました".into());
        }
        Ok(bytes)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    config: &ExtractorConfig,
    engine: &SherpaConfig,
    wave: &Fingerprint,
    first: u64,
    end: u64,
    directory: &Path,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Output>, String> {
    // Cancellation during SHA verification is cancellation too; never leave an
    // unmanaged native child alive or publish partially validated vectors.
    match run_inner(
        config, engine, wave, first, end, directory, timeout, cancelled,
    ) {
        Err(_) if cancelled() => Ok(None),
        result => result,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_inner(
    config: &ExtractorConfig,
    engine: &SherpaConfig,
    wave: &Fingerprint,
    first: u64,
    end: u64,
    directory: &Path,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Output>, String> {
    if cancelled() {
        return Ok(None);
    }
    config.validate(engine)?;
    if first >= end
        || end - first > 480_000
        || timeout.is_zero()
        || timeout > PROCESS_POLICY.timeout().expect("embedding deadline")
        || !wave.path.is_absolute()
        || wave
            .path
            .to_str()
            .is_none_or(|text| text.len() > 4096 || text.contains(['\0', '\n', '\r']))
    {
        return Err("特徴量抽出の時間範囲・制限時間・音声パスが不正です".into());
    }
    engine.verify(cancelled)?;
    let proofs = [&config.helper, &config.library, wave];
    for proof in proofs {
        proof.verify(cancelled)?;
    }
    let mut watched = proofs
        .iter()
        .map(|proof| StableFile::watch(proof))
        .collect::<Result<Vec<_>, _>>()?;
    for proof in [
        &engine.executable,
        &engine.segmentation_model,
        &engine.embedding_model,
    ]
    .into_iter()
    .chain(engine.runtime_libraries.iter())
    {
        watched.push(StableFile::watch(proof)?);
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let mut stdout = Log::create(directory.join("embedding.stdout.json"), MAX_OUTPUT)?;
    let stderr = Log::create(directory.join("embedding.stderr.log"), MAX_LOG)?;
    if cancelled() {
        return Ok(None);
    }
    let _log_capture =
        crate::internal_logs::watch_engine(&directory, &["embedding.stderr.log"], "diarization");
    let mut launch =
        OwnedCommand::for_purpose(&config.helper.path, mon_native::process::Purpose::Embedding)?;
    launch
        .command
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(stdout.file.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.file.try_clone().map_err(|e| e.to_string())?)
        .args(["--protocol-version", "1", "--library"])
        .arg(&config.library.path)
        .arg("--model")
        .arg(&engine.embedding_model.path)
        .arg("--wave")
        .arg(&wave.path)
        .arg("--first-frame")
        .arg(first.to_string())
        .arg("--end-frame")
        .arg(end.to_string())
        .arg("--threads")
        .arg(engine.threads.to_string());
    let mut child = crate::local_engine::resident::Run::spawn(
        &mut launch,
        "embedding",
        &directory,
        "embedding.stdout.json",
        "embedding.stderr.log",
    )
    .map_err(|e| format!("特徴量抽出を起動できません: {e}"))?;
    let started = Instant::now();
    loop {
        if cancelled() {
            return Ok(None);
        }
        if PROCESS_POLICY.expired(started.elapsed(), Some(timeout)) {
            return Err("特徴量抽出が制限時間を超えました".into());
        }
        stdout.check()?;
        stderr.check()?;
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("特徴量抽出が失敗しました: {status}"));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    stdout.check()?;
    stderr.check()?;
    for file in &watched {
        file.verify()?;
    }
    engine.verify(cancelled)?;
    for proof in proofs {
        proof.verify(cancelled)?;
    }
    let output = parse(&stdout.bytes()?, first, end)?;
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(output))
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
