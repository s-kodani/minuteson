//! Path-free, immutable worker identity. Runtime bytes stay outside meeting
//! backups and are retained by content hash across app upgrades.
use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Accelerated {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub protocol: u32,
    pub engine: String,
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<Accelerated>,
}
impl Snapshot {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.protocol == 2 {
            let name = match self.engine.as_str() {
                "whisper" => "whisper-cli.exe",
                "llama" => "llama-completion.exe",
                "sherpa" => "sherpa-onnx-offline-speaker-diarization.exe",
                _ => return Err("未知のWindows推論エンジンです".into()),
            };
            if self.name != name
                || self.gpu.is_some()
                || self.bytes == 0
                || self.bytes > 128 * 1024 * 1024
                || !crate::model_download_ipc::hash(&self.sha256)
            {
                return Err("Windows推論ランタイムの固定情報が不正です".into());
            }
            return Ok(());
        }
        let asr_binary = crate::transcription::asr::worker_binary(&self.engine).map(|n| {
            if cfg!(windows) {
                format!("{n}.exe")
            } else {
                n.to_owned()
            }
        });
        let name = match self.engine.as_str() {
            #[cfg(windows)]
            "whisper" => "mon-whisper-worker.exe",
            #[cfg(windows)]
            "llama" => "mon-llama-worker.exe",
            #[cfg(windows)]
            "sherpa" => "mon-sherpa-worker.exe",
            #[cfg(not(windows))]
            "whisper" => "mon-whisper-worker",
            #[cfg(not(windows))]
            "llama" => "mon-llama-worker",
            #[cfg(not(windows))]
            "sherpa" => "mon-sherpa-worker",
            _ => asr_binary.as_deref().ok_or("未知の常駐推論エンジンです")?,
        };
        let accelerated_name = match self.engine.as_str() {
            "whisper" => "mon-whisper-vulkan-worker.exe",
            "llama" => "mon-llama-vulkan-worker.exe",
            _ => "",
        };
        if let Some(gpu) = &self.gpu {
            if accelerated_name.is_empty()
                || gpu.name != accelerated_name
                || gpu.bytes == 0
                || gpu.bytes > 128 * 1024 * 1024
                || !crate::model_download_ipc::hash(&gpu.sha256)
            {
                return Err("固定Vulkanワーカーの定義が不正です".into());
            }
        }
        // Saved worker names are portable provenance. Resolution still requires
        // the original binary; restoring a job never changes its OS or engine.
        let portable_name = match self.engine.as_str() {
            "whisper" => "mon-whisper-worker",
            "llama" => "mon-llama-worker",
            "sherpa" => "mon-sherpa-worker",
            _ => crate::transcription::asr::worker_binary(&self.engine).unwrap_or(""),
        };
        if self.protocol != 1
            || (self.name != name
                && self.name != portable_name
                && self.name != format!("{portable_name}.exe")
                && !(!accelerated_name.is_empty()
                    && self.name == accelerated_name
                    && self.gpu.is_none()))
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
    pub(crate) fn accelerated(&self) -> Result<Self, String> {
        self.validate()?;
        let gpu = self.gpu.as_ref().ok_or("固定Vulkanワーカーがありません")?;
        Ok(Self {
            protocol: 1,
            engine: self.engine.clone(),
            name: gpu.name.clone(),
            bytes: gpu.bytes,
            sha256: gpu.sha256.clone(),
            gpu: None,
        })
    }
    pub(crate) fn accepts(&self, kind: &str) -> Result<(), String> {
        self.validate()?;
        if kind != self.engine
            && !(self.engine == "sherpa" && kind == "embedding")
            && !(self.engine == "llama"
                && matches!(kind, "llama_tokenize" | "llama_prefix" | "llama_memory"))
        {
            return Err("ジョブと推論エンジンが一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn resolve(&self, root: &Path) -> Result<Fingerprint, String> {
        #[cfg(windows)]
        if self.protocol == 2 {
            self.validate()?;
            let proof = Fingerprint {
                path: crate::windows_runtime::resources()?
                    .join(&self.engine)
                    .join(&self.name),
                size: self.bytes,
                sha256: self.sha256.clone(),
            };
            proof.verify(&|| false).map_err(|error| format!("このジョブが記録したWindows実行環境を利用できません。実行条件は変更していません。現在のモデル設定で新しく実行してください: {error}"))?;
            return Ok(proof);
        }
        self.resolve_from(root, || Ok(resources()?.join(&self.name)))
    }
    fn resolve_from(
        &self,
        root: &Path,
        source: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Result<Fingerprint, String> {
        self.validate()?;
        let cache = root.join("inference-runtimes");
        ensure_directory(&cache)?;
        let directory = cache.join(&self.sha256);
        ensure_directory(&directory)?;
        let path = directory.join(&self.name);
        if path.symlink_metadata().is_err() {
            let source = source()?;
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
                // The staging name is removed after publication. Verify its
                // bytes without retaining that name in the operation's final
                // verification set; resolve() retains the published file.
                let staged = Fingerprint::capture(&temporary, &|| false)?;
                if staged.size != self.bytes || staged.sha256 != self.sha256 {
                    return Err("配置中の推論ワーカーのハッシュが一致しません".into());
                }
                match std::fs::hard_link(&temporary, &path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.to_string()),
                }
                crate::native_fs::sync_directory(&directory)
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

#[cfg(test)]
mod portable_metadata_tests {
    use super::*;
    #[test]
    fn cross_platform_worker_metadata_preserves_windows_gpu_and_legacy_protocol_two() {
        for engine in ["whisper", "llama", "sherpa"] {
            let mut saved = Snapshot {
                protocol: 1,
                engine: engine.into(),
                name: format!("mon-{engine}-worker.exe"),
                bytes: 1024,
                sha256: "a".repeat(64),
                gpu: None,
            };
            saved.validate().unwrap();
            if engine != "sherpa" {
                saved.gpu = Some(Accelerated {
                    name: format!("mon-{engine}-vulkan-worker.exe"),
                    bytes: 2048,
                    sha256: "b".repeat(64),
                });
                saved.validate().unwrap();
                let bytes = serde_json::to_vec(&saved).unwrap();
                assert_eq!(serde_json::from_slice::<Snapshot>(&bytes).unwrap(), saved);
            }
            saved.gpu = None;
            saved.protocol = 2;
            saved.name = match engine {
                "whisper" => "whisper-cli.exe",
                "llama" => "llama-completion.exe",
                _ => "sherpa-onnx-offline-speaker-diarization.exe",
            }
            .into();
            saved.validate().unwrap();
            saved.gpu = Some(Accelerated {
                name: "mon-llama-vulkan-worker.exe".into(),
                bytes: 2048,
                sha256: "b".repeat(64),
            });
            assert!(saved.validate().is_err());
        }
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
    #[cfg(windows)]
    {
        return Ok(crate::windows_runtime::resources()?.join("workers"));
    }
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
    // Unit fixtures use frozen synthetic engines on every OS. Real bundled
    // runtime tests opt into prepare_installed rather than ambient resources.
    #[cfg(test)]
    {
        if let Some(snapshot) = super::ADMISSION.with(|v| v.borrow().clone()) {
            snapshot.accepts(engine)?;
            snapshot.resolve(root)?;
            return Ok(Some(snapshot));
        }
        if super::INSTALLED_ADMISSION.with(|cell| cell.get()) {
            return prepare_installed(root, engine);
        }
        synthetic(root, engine).map(Some)
    }
    #[cfg(not(test))]
    {
        prepare_installed(root, engine)
    }
}

pub(crate) fn prepare_installed(root: &Path, engine: &str) -> Result<Option<Snapshot>, String> {
    #[cfg(windows)]
    {
        let path = crate::windows_runtime::resources()?
            .join("workers")
            .join("runtime-index.json");
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Windows推論定義が上限を超えています".into());
        }
        let entries: Vec<Snapshot> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let snapshot = entries
            .into_iter()
            .find(|s| s.engine == engine)
            .ok_or("Windows推論エンジンがありません")?;
        snapshot.resolve(root)?;
        return Ok(Some(snapshot));
    }
    #[cfg(not(windows))]
    {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return Err("この環境の常駐推論ワーカーは未対応です".into());
        }
        prepare_bundled(root, engine, &resources()?).map(Some)
    }
}

fn prepare_bundled(root: &Path, engine: &str, bundle: &Path) -> Result<Snapshot, String> {
    let mut bytes = Vec::new();
    File::open(bundle.join("runtime-index.json"))
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("推論ランタイムの定義が上限を超えています".into());
    }
    let snapshots: Vec<Snapshot> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    // Several ASR adapters can use the same worker (ReazonSpeech and Haas).
    // Validate the complete set of worker engines, independently of model count.
    let mut engines = ["llama", "sherpa"]
        .into_iter()
        .chain(
            crate::transcription::asr::registry()
                .into_iter()
                .map(|adapter| adapter.descriptor().worker),
        )
        .collect::<std::collections::BTreeSet<_>>();
    if snapshots.len() != engines.len() {
        return Err("推論ランタイムの構成が不正です".into());
    }
    for snapshot in &snapshots {
        snapshot.validate()?;
        if !engines.remove(snapshot.engine.as_str()) {
            return Err("推論ランタイムの構成が不正です".into());
        }
    }
    let snapshot = snapshots
        .into_iter()
        .find(|s| s.engine == engine)
        .ok_or("推論ランタイムがありません")?;
    snapshot.resolve_from(root, || Ok(bundle.join(&snapshot.name)))?;
    Ok(snapshot)
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
            "whisper" | "asr-transducer" => &spec.parameters,
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
        name: crate::transcription::asr::worker_binary(engine)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("mon-{engine}-worker")),
        bytes: SYNTHETIC.len() as u64,
        sha256: format!("{:x}", sha2::Sha256::digest(SYNTHETIC)),
        gpu: None,
    };
    snapshot.validate()?;
    let cache = root.join("inference-runtimes");
    ensure_directory(&cache)?;
    let directory = cache.join(&snapshot.sha256);
    ensure_directory(&directory)?;
    let file = directory.join(&snapshot.name);
    if !file.exists() {
        std::fs::write(&file, SYNTHETIC).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    snapshot.resolve(root)?;
    Ok(snapshot)
}

#[cfg(test)]
mod bundled_runtime_tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn unit_preparation_uses_owned_synthetic_workers_without_installed_resources() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        assert!(prepare(&root, "unregistered-unit-test-engine").is_err());
        assert!(!root.join("inference-runtimes").exists());
        for engine in ["whisper", "llama", "sherpa", "asr-transducer"] {
            let snapshot = prepare(&root, engine).unwrap().unwrap();
            assert!(is_synthetic(&snapshot));
            assert_eq!(snapshot.engine, engine);
            let fingerprint = snapshot.resolve(&root).unwrap();
            assert!(fingerprint
                .path
                .starts_with(root.join("inference-runtimes")));
            assert_eq!(std::fs::read(fingerprint.path).unwrap(), SYNTHETIC);
        }
        let worker = prepare(&root, "whisper").unwrap().unwrap();
        std::fs::write(worker.resolve(&root).unwrap().path, b"changed").unwrap();
        assert!(prepare(&root, "whisper").is_err());
    }

    #[test]
    fn installed_admission_is_explicit_nested_and_restores_after_unwind() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        assert!(!super::super::INSTALLED_ADMISSION.with(|cell| cell.get()));
        let unwind = std::panic::catch_unwind(|| {
            let _outer = super::super::InstalledAdmission::enter();
            {
                let _inner = super::super::InstalledAdmission::enter();
                assert!(super::super::INSTALLED_ADMISSION.with(|cell| cell.get()));
            }
            assert!(super::super::INSTALLED_ADMISSION.with(|cell| cell.get()));
            // Installed admission always rejects an unregistered engine,
            // regardless of whether real application resources are present.
            assert!(prepare(&root, "unregistered-unit-test-engine").is_err());
            panic!("exercise installed-admission unwind");
        });
        assert!(unwind.is_err());
        assert!(!super::super::INSTALLED_ADMISSION.with(|cell| cell.get()));
        assert!(is_synthetic(&prepare(&root, "llama").unwrap().unwrap()));
    }

    #[test]
    fn unit_preparation_retains_explicit_admission_and_verifies_engine_and_bytes() {
        struct RestoreAdmission(Option<Snapshot>);
        impl Drop for RestoreAdmission {
            fn drop(&mut self) {
                super::super::ADMISSION.with(|cell| cell.replace(self.0.take()));
            }
        }
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let (bundle, _) = bundle_fixture(&root);
        let expected = prepare_bundled(&root, "llama", &bundle).unwrap();
        let _restore = RestoreAdmission(
            super::super::ADMISSION.with(|cell| cell.replace(Some(expected.clone()))),
        );
        // A frozen admission must keep precedence over ambient app resources,
        // including when a real-runtime integration scope is active.
        let _installed = super::super::InstalledAdmission::enter();
        let actual = prepare(&root, "llama").unwrap().unwrap();
        assert!(!is_synthetic(&actual));
        assert_eq!(actual.sha256, expected.sha256);
        assert!(prepare(&root, "whisper").is_err());
        std::fs::write(actual.resolve(&root).unwrap().path, b"changed").unwrap();
        assert!(prepare(&root, "llama").is_err());
    }

    fn bundle_fixture(root: &Path) -> (PathBuf, Vec<Snapshot>) {
        let bundle = root.join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        let snapshots = ["whisper", "llama", "sherpa", "asr-transducer"]
            .into_iter()
            .map(|engine| {
                let name = if engine == "asr-transducer" {
                    "mon-transducer-worker".to_owned()
                } else {
                    format!("mon-{engine}-worker")
                };
                let bytes = format!("#!/bin/sh\n# {engine}\nexit 0\n");
                std::fs::write(bundle.join(&name), bytes.as_bytes()).unwrap();
                Snapshot {
                    protocol: 1,
                    engine: engine.into(),
                    name,
                    bytes: bytes.len() as u64,
                    sha256: format!("{:x}", sha2::Sha256::digest(bytes.as_bytes())),
                    gpu: None,
                }
            })
            .collect::<Vec<_>>();
        std::fs::write(
            bundle.join("runtime-index.json"),
            serde_json::to_vec(&snapshots).unwrap(),
        )
        .unwrap();
        (bundle, snapshots)
    }

    #[test]
    fn bundled_runtime_prepares_shared_asr_worker_through_production_admission() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let (bundle, snapshots) = bundle_fixture(&root);
        assert_eq!(
            crate::transcription::asr::find(crate::transcription::asr::TRANSDUCER)
                .unwrap()
                .descriptor()
                .worker,
            crate::transcription::asr::find(crate::transcription::asr::NEMO_TDT)
                .unwrap()
                .descriptor()
                .worker
        );
        for expected in snapshots {
            let actual = prepare_bundled(&root, &expected.engine, &bundle).unwrap();
            assert_eq!(actual, expected);
            actual.resolve(&root).unwrap().verify(&|| false).unwrap();
        }
    }

    #[test]
    fn bundled_runtime_rejects_missing_duplicate_unknown_or_tampered_workers() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let (bundle, snapshots) = bundle_fixture(&root);
        for corruption in ["missing", "duplicate", "unknown", "invalid-hash"] {
            let mut entries = snapshots.clone();
            match corruption {
                "missing" => {
                    entries.pop();
                }
                "duplicate" => entries[3] = entries[0].clone(),
                "unknown" => entries[3].engine = "unregistered-worker".into(),
                "invalid-hash" => entries[0].sha256 = "invalid".into(),
                _ => unreachable!(),
            }
            std::fs::write(
                bundle.join("runtime-index.json"),
                serde_json::to_vec(&entries).unwrap(),
            )
            .unwrap();
            assert!(
                prepare_bundled(&root, "whisper", &bundle).is_err(),
                "{corruption}"
            );
            assert!(!root.join("inference-runtimes").exists());
        }
        std::fs::write(
            bundle.join("runtime-index.json"),
            serde_json::to_vec(&snapshots).unwrap(),
        )
        .unwrap();
        std::fs::write(bundle.join("mon-whisper-worker"), b"tampered").unwrap();
        assert!(prepare_bundled(&root, "whisper", &bundle).is_err());
    }

    #[test]
    #[ignore = "requires a built inference runtime bundle; no model or user data is used"]
    fn bundled_runtime_prepares_actual_app_workers_through_production_admission() {
        let bundle = std::env::var_os("MON_TEST_INFERENCE_RUNTIME")
            .map(PathBuf::from)
            .unwrap_or_else(|| resources().unwrap())
            .canonicalize()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        for engine in ["whisper", "llama", "sherpa", "asr-transducer"] {
            let snapshot = prepare_bundled(&root, engine, &bundle).unwrap();
            assert_eq!(snapshot.engine, engine);
            snapshot.resolve(&root).unwrap().verify(&|| false).unwrap();
        }
    }
}

#[cfg(all(test, unix))]
mod installation_tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn first_installation_can_finish_shared_verification_after_staging_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let source = directory.join("bundled-worker");
        let bytes = b"#!/bin/sh\nexit 0\n";
        std::fs::write(&source, bytes).unwrap();
        let snapshot = Snapshot {
            protocol: 1,
            engine: "sherpa".into(),
            name: "mon-sherpa-worker".into(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
            gpu: None,
        };
        let scope = crate::local_engine::verification::Scope::enter(true).unwrap();
        let proof = snapshot
            .resolve_from(&directory, || Ok(source.clone()))
            .unwrap();
        assert_eq!(std::fs::read(&proof.path).unwrap(), bytes);
        assert!(!std::fs::read_dir(proof.path.parent().unwrap())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".install-")));
        scope.finish(&|| false).unwrap();
        assert_eq!(
            snapshot
                .resolve_from(&directory, || panic!("cached worker must be reused"))
                .unwrap(),
            proof
        );
        scope.finish(&|| false).unwrap();
        // Retained verification still protects the published executable.
        std::fs::write(&proof.path, b"tampered").unwrap();
        assert!(scope.finish(&|| false).is_err());
    }
}
