//! Path-free, immutable worker identity. Runtime bytes stay outside meeting
//! backups and are retained by content hash across app upgrades.
use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub protocol: u32,
    pub engine: String,
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
impl Snapshot {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let name = match self.engine.as_str() {
            "whisper" => "mon-whisper-worker",
            "llama" => "mon-llama-worker",
            "sherpa" => "mon-sherpa-worker",
            _ => return Err("未知の常駐推論エンジンです".into()),
        };
        if self.protocol != 1
            || self.name != name
            || self.bytes == 0
            || self.bytes > 128 * 1024 * 1024
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err("固定した常駐ランタイムの定義が不正です".into());
        }
        Ok(())
    }
    pub(crate) fn accepts(&self, kind: &str) -> Result<(), String> {
        self.validate()?;
        if kind != self.engine
            && !(self.engine == "sherpa" && kind == "embedding")
            && !(self.engine == "llama" && matches!(kind, "llama_tokenize" | "llama_prefix"))
        {
            return Err("ジョブと推論エンジンが一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn resolve(&self, root: &Path) -> Result<Fingerprint, String> {
        self.validate()?;
        let cache = root.join("inference-runtimes");
        ensure_directory(&cache)?;
        let directory = cache.join(&self.sha256);
        ensure_directory(&directory)?;
        let path = directory.join(&self.name);
        if path.symlink_metadata().is_err() {
            let source = resources()?.join(&self.name);
            let proof = Fingerprint {
                path: source.clone(),
                size: self.bytes,
                sha256: self.sha256.clone(),
            };
            proof.verify(&|| false).map_err(|_| "このジョブに必要な固定版推論ワーカーがありません。対応するアプリのランタイムを復元してください".to_string())?;
            let temporary = directory.join(format!(".install-{}", uuid::Uuid::new_v4()));
            let install = (|| {
                let mut input = File::open(&source).map_err(|e| e.to_string())?;
                let mut options = OpenOptions::new();
                options.create_new(true).write(true);
                #[cfg(unix)]
                options.mode(0o700);
                let mut output = options.open(&temporary).map_err(|e| e.to_string())?;
                std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
                output.sync_all().map_err(|e| e.to_string())?;
                Fingerprint {
                    path: temporary.clone(),
                    size: self.bytes,
                    sha256: self.sha256.clone(),
                }
                .verify(&|| false)?;
                match std::fs::hard_link(&temporary, &path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.to_string()),
                }
                File::open(&directory)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())
            })();
            let _ = std::fs::remove_file(&temporary);
            install?;
        }
        let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("固定版推論ワーカーの実体・実行権限が不正です".into());
        }
        #[cfg(unix)]
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("固定版推論ワーカーの実行権限がありません".into());
        }
        let proof = Fingerprint {
            path,
            size: self.bytes,
            sha256: self.sha256.clone(),
        };
        proof.verify(&|| false)?;
        Ok(proof)
    }
}
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
fn ensure_directory(path: &Path) -> Result<(), String> {
    match std::fs::create_dir(path) {
        Ok(()) => {
            #[cfg(unix)]
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    if !path.symlink_metadata().map_err(|e| e.to_string())?.is_dir()
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("推論ランタイムの管理領域が不正です".into());
    }
    Ok(())
}
fn resources() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = executable.parent().ok_or("アプリの実行位置がありません")?;
    if parent.file_name().is_some_and(|v| v == "MacOS") {
        return Ok(parent
            .parent()
            .ok_or("アプリのResourcesがありません")?
            .join("Resources/inference-runtime"));
    }
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cache/inference-runtime/package")
        .canonicalize()
        .map_err(|_| "常駐推論ランタイムをビルドしてください".to_string())?)
}
pub(crate) fn prepare(root: &Path, engine: &str) -> Result<Option<Snapshot>, String> {
    // Existing fixtures deliberately exercise their frozen single-shot engines.
    // Resident lifecycle tests construct and bind an explicit snapshot instead.
    #[cfg(test)]
    if let Some(snapshot) = super::ADMISSION.with(|v| v.borrow().clone()) {
        snapshot.accepts(engine)?;
        snapshot.resolve(root)?;
        return Ok(Some(snapshot));
    }
    #[cfg(test)]
    {
        return synthetic(root, engine).map(Some);
    }
    #[cfg(not(test))]
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err("この環境の常駐推論ワーカーは未対応です".into());
    }
    let mut bytes = Vec::new();
    File::open(resources()?.join("runtime-index.json"))
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("推論ランタイムの定義が上限を超えています".into());
    }
    let snapshots: Vec<Snapshot> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if snapshots.len() != 3 {
        return Err("推論ランタイムの構成が不正です".into());
    }
    for snapshot in &snapshots {
        snapshot.validate()?;
    }
    let snapshot = snapshots
        .into_iter()
        .find(|s| s.engine == engine)
        .ok_or("推論ランタイムがありません")?;
    snapshot.resolve(root)?;
    Ok(Some(snapshot))
}

// Preserve the initial execution mode on lost admission responses, including
// historical operations that predate resident workers.
pub(crate) fn prepare_job(
    storage: &crate::storage::Storage,
    meeting: &str,
    operation: &str,
    engine: &str,
) -> Result<Option<Snapshot>, String> {
    if let Some(existing) = storage
        .list_jobs(meeting)?
        .iter()
        .find(|j| j.operation_id == operation)
    {
        let (spec, _) = storage.fixed_job_input(&existing.id)?;
        let parameters = match engine {
            "whisper" => &spec.parameters,
            "sherpa" => &spec.parameters["diarization"],
            "llama" => &spec.parameters["generation"],
            _ => return Err("未知の常駐推論工程です".into()),
        };
        let value = parameters
            .get("residentRuntime")
            .filter(|v| !v.is_null())
            .ok_or("常駐推論情報がありません")?;
        return serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| e.to_string());
    }
    prepare(storage.root(), engine)
}

#[cfg(test)]
const SYNTHETIC: &[u8] = b"# MON unit-test engine substitute\n";
#[cfg(test)]
pub(super) fn is_synthetic(snapshot: &Snapshot) -> bool {
    use sha2::Digest;
    snapshot.sha256 == format!("{:x}", sha2::Sha256::digest(SYNTHETIC))
}
#[cfg(test)]
fn synthetic(root: &Path, engine: &str) -> Result<Snapshot, String> {
    use sha2::Digest;
    let snapshot = Snapshot {
        protocol: 1,
        engine: engine.into(),
        name: format!("mon-{engine}-worker"),
        bytes: SYNTHETIC.len() as u64,
        sha256: format!("{:x}", sha2::Sha256::digest(SYNTHETIC)),
    };
    let cache = root.join("inference-runtimes");
    ensure_directory(&cache)?;
    let directory = cache.join(&snapshot.sha256);
    ensure_directory(&directory)?;
    let file = directory.join(&snapshot.name);
    if !file.exists() {
        std::fs::write(&file, SYNTHETIC).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(snapshot)
}
