//! Read-only manual package preflight. Never executes package binaries, installs
//! files, modifies jobs/settings, accesses the network, or grants AI readiness.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MANIFEST: &str = "mon-model.json";
const MAX_MANIFEST: usize = 65_536;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PackageManifest {
    pub schema_version: u32,
    pub engine: Engine,
    pub engine_version: String,
    pub model_name: String,
    pub model_version: String,
    pub quantization: Option<String>,
    pub license_ref: String,
    pub target: Target,
    pub files: Vec<PackageFile>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum Engine {
    #[serde(rename = "whisper.cpp")]
    Whisper,
    #[serde(rename = "sherpa-onnx")]
    Sherpa,
    #[serde(rename = "llama.cpp")]
    Llama,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Target {
    MacosAarch64,
    WindowsX86_64,
}
impl Target {
    fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacosAarch64),
            ("windows", "x86_64") => Ok(Self::WindowsX86_64),
            _ => Err("このOS・CPUはモデル取り込みの対応対象外です".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Executable,
    RuntimeLibrary,
    Asr,
    Vad,
    Segmentation,
    SpeakerEmbedding,
    Llm,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PackageFile {
    pub relative_path: String,
    pub role: Role,
    pub expected_size: Option<u64>,
    pub expected_hash: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileReport {
    pub relative_path: String,
    pub role: Role,
    pub actual_size: u64,
    pub actual_hash: String,
    pub expected_size_checked: bool,
    pub expected_hash_checked: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub manifest: PackageManifest,
    pub manifest_hash: String,
    pub files: Vec<FileReport>,
    pub total_bytes: u64,
    // Hash consistency and a declared target do not prove binary/model loading,
    // publisher authenticity, redistribution rights, performance, or quality.
    pub engine_compatibility: &'static str,
    pub publisher_authenticity: &'static str,
    pub installed: bool,
    pub ready: bool,
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn relative(value: &str) -> bool {
    // Portable package names: no traversal, drive/ADS, Windows aliases or
    // case-insensitive collisions. The manifest cannot list itself as a model.
    !value.is_empty()
        && value.len() <= 4096
        && value != MANIFEST
        && value.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
}
impl PackageManifest {
    fn validate(&self, target: Target) -> Result<(), String> {
        if self.schema_version != 1
            || self.target != target
            || !text(&self.engine_version, 128)
            || !text(&self.model_name, 256)
            || !text(&self.model_version, 128)
            || !text(&self.license_ref, 4096)
            || self.quantization.as_ref().is_some_and(|v| !text(v, 128))
            || self.files.is_empty()
            || self.files.len() > 64
        {
            return Err("モデルmanifestの形式版・対象OS/CPU・必須情報が不正です".into());
        }
        let mut paths = BTreeSet::new();
        for file in &self.files {
            if !relative(&file.relative_path)
                || !paths.insert(file.relative_path.to_lowercase())
                || file
                    .expected_size
                    .is_some_and(|n| n == 0 || n > i64::MAX as u64)
                || file.expected_hash.as_ref().is_some_and(|v| !hash(v))
                || file.relative_path.eq_ignore_ascii_case(MANIFEST)
            {
                return Err("モデルmanifestのファイルパス・容量・SHA・重複が不正です".into());
            }
        }
        // A file cannot also be an ancestor directory, including case aliases.
        for path in &paths {
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if paths.contains(prefix) {
                    return Err("モデルファイルと親フォルダーの指定が衝突しています".into());
                }
                parent = prefix;
            }
        }
        let count = |role| self.files.iter().filter(|f| f.role == role).count();
        let roles = match self.engine {
            Engine::Whisper => vec![Role::Executable, Role::Asr, Role::Vad, Role::RuntimeLibrary],
            Engine::Sherpa => vec![
                Role::Executable,
                Role::Segmentation,
                Role::SpeakerEmbedding,
                Role::RuntimeLibrary,
            ],
            Engine::Llama => vec![Role::Executable, Role::Llm, Role::RuntimeLibrary],
        };
        if count(Role::Executable) != 1
            || count(Role::RuntimeLibrary) > 16
            || count(Role::Vad) > 1
            || self.files.iter().any(|f| !roles.contains(&f.role))
            || match self.engine {
                Engine::Whisper => count(Role::Asr) != 1,
                Engine::Sherpa => {
                    count(Role::Segmentation) != 1 || count(Role::SpeakerEmbedding) != 1
                }
                Engine::Llama => count(Role::Llm) != 1,
            }
        {
            return Err("エンジンに必要なモデル・実行ファイルの構成が一致しません".into());
        }
        Ok(())
    }
}

fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("モデルの事前検査を取り消しました".into())
    } else {
        Ok(())
    }
}
fn unchanged(before: &Metadata, after: &Metadata) -> Result<(), String> {
    let mut same = before.is_file() == after.is_file()
        && before.is_dir() == after.is_dir()
        && before.len() == after.len()
        && before.modified().map_err(|e| e.to_string())?
            == after.modified().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same &= before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec();
    }
    if same {
        Ok(())
    } else {
        Err("検査中にモデルパッケージが変更されました".into())
    }
}
struct Source {
    file: File,
    parents: Vec<(PathBuf, Metadata)>,
    path: PathBuf,
    before: Metadata,
}
impl Source {
    fn open(root: &Path, relative: &str) -> Result<Self, String> {
        let mut path = root.to_path_buf();
        let mut parents = Vec::new();
        let parts = relative.split('/').collect::<Vec<_>>();
        for part in parts {
            let info = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !info.is_dir() || path.canonicalize().map_err(|e| e.to_string())? != path {
                return Err("モデルの親フォルダーにリンクまたは変更があります".into());
            }
            parents.push((path.clone(), info));
            path.push(part);
        }
        let before = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !before.is_file() {
            return Err("モデルはリンクではない通常ファイルで指定してください".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path).map_err(|e| e.to_string())?;
        unchanged(&before, &file.metadata().map_err(|e| e.to_string())?)?;
        let source = Self {
            file,
            parents,
            path,
            before,
        };
        source.verify()?;
        Ok(source)
    }
    fn verify(&self) -> Result<(), String> {
        unchanged(
            &self.before,
            &self.file.metadata().map_err(|e| e.to_string())?,
        )?;
        unchanged(
            &self.before,
            &std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?,
        )?;
        for (path, metadata) in &self.parents {
            let now = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !now.is_dir() || path.canonicalize().map_err(|e| e.to_string())? != *path {
                return Err("検査中にモデルの親フォルダーが変更されました".into());
            }
            unchanged(metadata, &now)?;
        }
        Ok(())
    }
}

pub(crate) fn inspect(directory: &Path, cancelled: &impl Fn() -> bool) -> Result<Report, String> {
    inspect_for_target(directory, Target::current()?, cancelled)
}
fn inspect_for_target(
    directory: &Path,
    target: Target,
    cancelled: &impl Fn() -> bool,
) -> Result<Report, String> {
    check(cancelled)?;
    if !directory.is_absolute()
        || !std::fs::symlink_metadata(directory)
            .map_err(|e| e.to_string())?
            .is_dir()
    {
        return Err(
            "モデルパッケージはリンクではないフォルダーの絶対パスで指定してください".into(),
        );
    }
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let mut manifest_file = Source::open(&root, MANIFEST)?;
    if manifest_file.before.len() > MAX_MANIFEST as u64 {
        return Err("モデルmanifestは64KiB以内です".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut manifest_file.file)
        .take(MAX_MANIFEST as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    manifest_file.verify()?;
    if bytes.len() > MAX_MANIFEST {
        return Err("モデルmanifestは64KiB以内です".into());
    }
    let manifest: PackageManifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("モデルmanifestを読めません: {e}"))?;
    manifest.validate(target)?;
    let mut files = Vec::new();
    let mut total = 0_u64;
    let mut sources = Vec::new();
    for entry in &manifest.files {
        check(cancelled)?;
        let mut source = Source::open(&root, &entry.relative_path)?;
        let size = source.before.len();
        if size == 0 || size > i64::MAX as u64 || entry.expected_size.is_some_and(|n| n != size) {
            return Err(format!(
                "モデルファイルの容量が一致しません: {}",
                entry.relative_path
            ));
        }
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 65_536];
        let mut read = 0_u64;
        loop {
            check(cancelled)?;
            let count = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            read = read
                .checked_add(count as u64)
                .ok_or("モデル容量が範囲外です")?;
            if read > size {
                return Err("照合中にモデルファイルが増大しました".into());
            }
            digest.update(&buffer[..count]);
        }
        let actual_hash = format!("{:x}", digest.finalize());
        source.verify()?;
        if read != size
            || entry
                .expected_hash
                .as_ref()
                .is_some_and(|h| *h != actual_hash)
        {
            return Err(format!(
                "モデルファイルのSHA・容量が一致しません: {}",
                entry.relative_path
            ));
        }
        total = total
            .checked_add(size)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("モデルパッケージ容量が範囲外です")?;
        files.push(FileReport {
            relative_path: entry.relative_path.clone(),
            role: entry.role,
            actual_size: size,
            actual_hash,
            expected_size_checked: entry.expected_size.is_some(),
            expected_hash_checked: entry.expected_hash.is_some(),
        });
        sources.push(source);
    }
    check(cancelled)?;
    manifest_file.verify()?;
    for source in &sources {
        source.verify()?;
    }
    Ok(Report {
        manifest,
        manifest_hash: format!("{:x}", Sha256::digest(&bytes)),
        files,
        total_bytes: total,
        engine_compatibility: "not_verified",
        publisher_authenticity: "not_verified",
        installed: false,
        ready: false,
    })
}

type Active = Option<(String, Arc<AtomicBool>)>;
#[derive(Default)]
pub(crate) struct Inspector(Arc<Mutex<Active>>);
pub(crate) struct InspectionGuard {
    owner: Arc<Mutex<Active>>,
    token: String,
    cancel: Arc<AtomicBool>,
}
impl Inspector {
    pub(crate) fn begin(&self, token: &str) -> Result<InspectionGuard, String> {
        if !uuid::Uuid::parse_str(token).is_ok_and(|id| id.to_string() == token) {
            return Err("モデル検査の操作IDが不正です".into());
        }
        let mut active = self
            .0
            .lock()
            .map_err(|_| "モデル検査ロックを取得できません")?;
        if active.is_some() {
            return Err("別のモデル検査が進行中です。完了または取消を待ってください".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some((token.into(), cancel.clone()));
        Ok(InspectionGuard {
            owner: self.0.clone(),
            token: token.into(),
            cancel,
        })
    }
    pub(crate) fn cancel(&self, token: &str) -> Result<(), String> {
        let active = self
            .0
            .lock()
            .map_err(|_| "モデル検査ロックを取得できません")?;
        if let Some((id, flag)) = &*active {
            if id == token {
                flag.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
}
impl InspectionGuard {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}
impl Drop for InspectionGuard {
    fn drop(&mut self) {
        let mut active = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|(id, _)| id == &self.token) {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests;
