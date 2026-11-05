//! Built-in ASR adapters. Model selection, scheduling and artifact storage use
//! this contract; native argument and output formats stay inside each adapter.
use crate::local_engine::Fingerprint;
use crate::model_inspection::{Engine, PackageManifest, Role};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AsrConfig {
    pub executable: Fingerprint,
    pub model: Fingerprint,
    pub vad_model: Option<Fingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vad_options: Option<super::vad::VadOptions>,
    pub threads: u32,
    pub use_gpu: bool,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendBinding>,
}

impl AsrConfig {
    pub(crate) fn validate_settings(&self) -> Result<(), String> {
        adapter(self)?.prepare(self)?;
        if let Some(options) = self.vad_options {
            options.validate()?;
            if !adapter(self)?.descriptor().supports_vad_overlap && options.samples_overlap_ms != 0
            {
                return Err("このバックエンドはVAD区間の重なりに対応していません".into());
            }
        }
        if !(1..=64).contains(&self.threads) || self.prompt.chars().count() > 8000 {
            return Err("文字起こしのスレッド数または用語ヒントが不正です".into());
        }
        Ok(())
    }
    pub fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.validate_settings()?;
        self.executable.verify(cancelled)?;
        self.model.verify(cancelled)?;
        if let Some(backend) = &self.backend {
            for file in backend.files.values() {
                file.verify(cancelled)?;
            }
        }
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

pub(crate) const WHISPER: &str = "whisper.cpp";
pub(crate) const TRANSDUCER: &str = "sherpa-onnx-transducer";

pub(crate) use mon_contracts::BackendManifest;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BackendPreferences {
    pub id: String,
    pub files: BTreeMap<String, String>,
}
impl BackendPreferences {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let descriptor = find(&self.id)?.descriptor();
        if self.files.keys().map(String::as_str).collect::<Vec<_>>() != descriptor.slots {
            return Err("文字起こしバックエンドのファイル構成が不正です".into());
        }
        for path in self.files.values() {
            if !Path::new(path).is_absolute() || path.len() > 4096 || path.contains('\0') {
                return Err("文字起こしモデルの参照が不正です".into());
            }
        }
        Ok(())
    }
    pub(crate) fn capture(&self, cancelled: &impl Fn() -> bool) -> Result<BackendBinding, String> {
        self.validate()?;
        Ok(BackendBinding {
            id: self.id.clone(),
            files: self
                .files
                .iter()
                .map(|(k, p)| {
                    Ok((
                        k.clone(),
                        Fingerprint::capture_retained(Path::new(p), cancelled)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct BackendBinding {
    pub id: String,
    pub files: BTreeMap<String, Fingerprint>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Descriptor {
    pub id: &'static str,
    pub revision: u32,
    pub worker: &'static str,
    pub worker_binary: &'static str,
    pub slots: &'static [&'static str],
    pub supports_gpu: bool,
    pub supports_vad: bool,
    pub supports_vad_overlap: bool,
    pub default_vad: super::vad::VadOptions,
    pub vocabulary_method: &'static str,
    pub vocabulary_description: &'static str,
    pub max_input_seconds: u32,
}

pub(crate) trait AsrAdapter: Sync {
    fn descriptor(&self) -> Descriptor;
    fn recognition_policy(&self) -> super::RecognitionPolicy {
        super::RecognitionPolicy::AdapterV1
    }
    fn prepare(&self, config: &AsrConfig) -> Result<(), String> {
        self.validate(config)
    }
    fn validate(&self, config: &AsrConfig) -> Result<(), String>;
    fn recognize(
        &self,
        config: &AsrConfig,
        directory: &Path,
        duration: i64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Segment>>, String>;
    fn parse_result(
        &self,
        bytes: &[u8],
        config: &AsrConfig,
        duration: i64,
    ) -> Result<Vec<Segment>, String>;
    fn guarded(
        &self,
        config: &AsrConfig,
        directory: &Path,
        duration: i64,
        _check: bool,
        _hinted: bool,
        _vocabulary: Option<&crate::storage::VocabularySnapshot>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<super::guard::Outcome>, mon_contracts::ProcessingError> {
        Ok(self
            .recognize(config, directory, duration, cancelled)?
            .map(|_| super::guard::Outcome {
                selected: directory.join("result.json"),
                rejected: None,
                reasons: vec![],
            }))
    }
    fn logical_output(
        &self,
        bytes: &[u8],
        runtime: &AsrConfig,
        logical: &AsrConfig,
        duration: i64,
    ) -> Result<Vec<u8>, String> {
        self.parse_result(bytes, runtime, duration)?;
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        value["models"]["model"] = serde_json::json!(logical.model.path);
        if let Some(backend) = &logical.backend {
            for (slot, file) in &backend.files {
                value["models"][slot] = serde_json::json!(file.path);
            }
        }
        let result = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        self.parse_result(&result, logical, duration)?;
        Ok(result)
    }
}
struct WhisperAdapter;
struct TransducerAdapter;
static WHISPER_ADAPTER: WhisperAdapter = WhisperAdapter;
static TRANSDUCER_ADAPTER: TransducerAdapter = TransducerAdapter;
pub(crate) fn registry() -> [&'static dyn AsrAdapter; 2] {
    [&WHISPER_ADAPTER, &TRANSDUCER_ADAPTER]
}
fn lookup<'a>(
    adapters: impl IntoIterator<Item = &'a dyn AsrAdapter>,
    id: &str,
) -> Result<&'a dyn AsrAdapter, String> {
    adapters
        .into_iter()
        .find(|a| a.descriptor().id == id)
        .ok_or_else(|| "未対応の文字起こしバックエンドです".into())
}
pub(crate) fn find(id: &str) -> Result<&'static dyn AsrAdapter, String> {
    lookup(registry(), id)
}
pub(crate) fn adapter(config: &AsrConfig) -> Result<&'static dyn AsrAdapter, String> {
    find(config.backend.as_ref().map_or(WHISPER, |b| b.id.as_str()))
}
pub(crate) fn engine(config: &AsrConfig) -> Engine {
    if config.backend.is_some() {
        Engine::Asr
    } else {
        Engine::Whisper
    }
}
pub(crate) fn validate_manifest(manifest: &PackageManifest) -> Result<(), String> {
    if manifest.engine != Engine::Asr {
        if manifest.backend.is_some() || manifest.files.iter().any(|f| f.role == Role::AsrAsset) {
            return Err("ASR以外の構成にバックエンド固有ファイルがあります".into());
        }
        return Ok(());
    }
    let backend = manifest
        .backend
        .as_ref()
        .ok_or("ASRバックエンドがありません")?;
    let descriptor = find(&backend.id)?.descriptor();
    if backend.files.keys().map(String::as_str).collect::<Vec<_>>() != descriptor.slots {
        return Err("ASRモデルの必要ファイルが一致しません".into());
    }
    let mut paths = std::collections::BTreeSet::new();
    for (slot, path) in &backend.files {
        let role = if slot == "library" {
            Role::RuntimeLibrary
        } else {
            Role::AsrAsset
        };
        if !paths.insert(path)
            || !manifest
                .files
                .iter()
                .any(|f| &f.relative_path == path && f.role == role)
        {
            return Err("ASRモデルの参照・役割が一致しません".into());
        }
    }
    if manifest
        .files
        .iter()
        .any(|f| f.role == Role::AsrAsset && !paths.contains(&f.relative_path))
    {
        return Err("ASRモデルに未参照のファイルがあります".into());
    }
    Ok(())
}
impl AsrAdapter for WhisperAdapter {
    fn recognition_policy(&self) -> super::RecognitionPolicy {
        super::RecognitionPolicy::AdaptiveRecoveryV3
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: WHISPER,
            revision: 1,
            worker: "whisper",
            worker_binary: "mon-whisper-worker",
            slots: &[],
            supports_gpu: true,
            supports_vad: true,
            supports_vad_overlap: true,
            default_vad: Default::default(),
            vocabulary_method: "whisper-vocabulary-v2",
            vocabulary_description:
                "対応：表記・読み。用語と読みを文章のヒントとして渡します。補足は保存・表示用です。",
            max_input_seconds: 30,
        }
    }
    fn validate(&self, _: &AsrConfig) -> Result<(), String> {
        Ok(())
    }
    fn recognize(
        &self,
        c: &AsrConfig,
        d: &Path,
        t: i64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Segment>>, String> {
        super::whisper::run_unit(c, d, t, std::time::Duration::from_secs(1800), &|| {
            cancelled()
        })
    }
    fn parse_result(&self, b: &[u8], c: &AsrConfig, t: i64) -> Result<Vec<Segment>, String> {
        super::whisper::parse_output(b, &c.model.path, t)
    }
    fn guarded(
        &self,
        c: &AsrConfig,
        d: &Path,
        t: i64,
        check: bool,
        hinted: bool,
        vocabulary: Option<&crate::storage::VocabularySnapshot>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<super::guard::Outcome>, mon_contracts::ProcessingError> {
        super::guard::run(c, d, t, check, hinted, vocabulary, &|| cancelled())
    }
    fn logical_output(
        &self,
        bytes: &[u8],
        runtime: &AsrConfig,
        logical: &AsrConfig,
        duration: i64,
    ) -> Result<Vec<u8>, String> {
        super::whisper::logical_output(bytes, &runtime.model.path, &logical.model.path, duration)
    }
}
impl AsrAdapter for TransducerAdapter {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: TRANSDUCER,
            revision: 1,
            worker: "asr-transducer",
            worker_binary: "mon-transducer-worker",
            slots: &["decoder", "joiner", "library", "tokens"],
            supports_gpu: false,
            supports_vad: true,
            supports_vad_overlap: false,
            default_vad: super::vad::VadOptions {
                threshold_milli: 500,
                min_speech_duration_ms: 250,
                min_silence_duration_ms: 500,
                speech_pad_ms: 200,
                samples_overlap_ms: 0,
                ..Default::default()
            },
            vocabulary_method: "transducer-hotwords-v1",
            vocabulary_description: "対応：表記のみ。登録した表記で文字起こしされやすくします。読み・補足は保存・表示用です。",
            max_input_seconds: 30,
        }
    }
    fn validate(&self, c: &AsrConfig) -> Result<(), String> {
        let b = c.backend.as_ref().ok_or("ASRモデル構成がありません")?;
        if b.files.keys().map(String::as_str).collect::<Vec<_>>() != self.descriptor().slots
            || c.use_gpu
        {
            return Err("Transducerのファイル構成またはCPU設定が不正です".into());
        }
        Ok(())
    }
    fn recognize(
        &self,
        c: &AsrConfig,
        d: &Path,
        t: i64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Segment>>, String> {
        super::transducer::run(c, d, t, &|| cancelled())
    }
    fn parse_result(&self, b: &[u8], c: &AsrConfig, t: i64) -> Result<Vec<Segment>, String> {
        super::transducer::parse(b, c, t)
    }
}

/// Worker identity is declared once by an adapter, independently of model names.
pub(crate) fn worker_binary(worker: &str) -> Option<&'static str> {
    registry()
        .into_iter()
        .map(|a| a.descriptor())
        .find(|d| d.worker == worker)
        .map(|d| d.worker_binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn proof(name: &str) -> Fingerprint {
        Fingerprint {
            path: Path::new("/models").join(name),
            size: 1,
            sha256: "a".repeat(64),
        }
    }
    fn config(id: &str) -> AsrConfig {
        AsrConfig {
            executable: proof("engine"),
            model: proof("model"),
            vad_model: None,
            vad_options: None,
            threads: 4,
            use_gpu: false,
            prompt: String::new(),
            backend: if id == WHISPER {
                None
            } else {
                Some(BackendBinding {
                    id: id.into(),
                    files: find(id)
                        .unwrap()
                        .descriptor()
                        .slots
                        .iter()
                        .map(|s| (s.to_string(), proof(s)))
                        .collect(),
                })
            },
        }
    }
    #[test]
    fn asr_adapters_share_a_validated_segment_contract() {
        for id in [WHISPER, TRANSDUCER] {
            let c = config(id);
            let a = adapter(&c).unwrap();
            a.prepare(&c).unwrap();
            let mut value = if id == WHISPER {
                json!({"model":{"multilingual":true},"params":{"model":c.model.path,"language":"ja","translate":false},"transcription":[{"offsets":{"from":100,"to":500},"text":"確認します"}]})
            } else {
                let mut models = c
                    .backend
                    .as_ref()
                    .unwrap()
                    .files
                    .iter()
                    .map(|(k, f)| (k.clone(), f.path.clone()))
                    .collect::<BTreeMap<_, _>>();
                models.insert("model".into(), c.model.path.clone());
                json!({"schemaVersion":1,"backendId":id,"models":models,"durationUs":1_000_000,"segments":[{"startUs":100_000,"endUs":500_000,"text":"確認します"}],"tokens":[],"ignoredHotwords":[]})
            };
            let result = a
                .parse_result(&serde_json::to_vec(&value).unwrap(), &c, 1_000_000)
                .unwrap();
            assert_eq!(
                result,
                vec![Segment {
                    start_us: 100_000,
                    end_us: 500_000,
                    text: "確認します".into()
                }]
            );
            if id == WHISPER {
                value["params"]["language"] = json!("en");
            } else {
                value["models"]["decoder"] = json!("/different");
            }
            assert!(a
                .parse_result(&serde_json::to_vec(&value).unwrap(), &c, 1_000_000)
                .is_err());
        }
        assert!(find("unregistered-backend").is_err());
    }
    #[test]
    fn asr_test_adapter_registers_without_scheduler_or_ui_branches() {
        struct TestAdapter;
        impl AsrAdapter for TestAdapter {
            fn descriptor(&self) -> Descriptor {
                let mut d = WHISPER_ADAPTER.descriptor();
                d.id = "test-backend";
                d.worker = "test-worker";
                d.worker_binary = "test-worker";
                d
            }
            fn validate(&self, _: &AsrConfig) -> Result<(), String> {
                Ok(())
            }
            fn recognize(
                &self,
                _: &AsrConfig,
                _: &Path,
                _: i64,
                cancelled: &dyn Fn() -> bool,
            ) -> Result<Option<Vec<Segment>>, String> {
                Ok(if cancelled() { None } else { Some(vec![]) })
            }
            fn parse_result(
                &self,
                b: &[u8],
                _: &AsrConfig,
                _: i64,
            ) -> Result<Vec<Segment>, String> {
                serde_json::from_slice(b).map_err(|e| e.to_string())
            }
        }
        let test = TestAdapter;
        let adapters = registry()
            .into_iter()
            .chain(std::iter::once(&test as &dyn AsrAdapter))
            .collect::<Vec<_>>();
        let a = lookup(adapters, "test-backend").unwrap();
        let c = config(WHISPER);
        assert!(a
            .recognize(&c, Path::new("/"), 1, &|| true)
            .unwrap()
            .is_none());
        assert_eq!(a.parse_result(b"[]", &c, 1).unwrap(), vec![]);
    }
    #[test]
    fn asr_preferences_reject_unknown_backends_and_missing_slots() {
        let mut b = BackendPreferences {
            id: TRANSDUCER.into(),
            files: BTreeMap::new(),
        };
        assert!(b.validate().is_err());
        b.files = find(TRANSDUCER)
            .unwrap()
            .descriptor()
            .slots
            .iter()
            .map(|s| {
                let path = std::env::current_dir().unwrap().join("models").join(s);
                (s.to_string(), path.to_string_lossy().into_owned())
            })
            .collect();
        b.validate().unwrap();
        b.files.insert("extra".into(), "/models/extra".into());
        assert!(b.validate().is_err());
    }

    #[test]
    #[ignore = "requires MON_ASR_ASSETS and built native transducer worker"]
    fn native_asr_models_reuse_one_resident_session_and_parse_real_japanese_audio() {
        let assets = std::fs::canonicalize(std::env::var("MON_ASR_ASSETS").unwrap()).unwrap();
        let worker_name = if cfg!(windows) {
            "mon-transducer-worker.exe"
        } else {
            "mon-transducer-worker"
        };
        let package = if cfg!(windows) {
            "../../.cache/windows-runtime/package/workers"
        } else {
            "../../.cache/inference-runtime/package"
        };
        let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(package)
            .join(worker_name)
            .canonicalize()
            .unwrap();
        #[cfg(windows)]
        let library = crate::windows_runtime::resources()
            .unwrap()
            .join("sherpa/sherpa-onnx-c-api.dll");
        #[cfg(not(windows))]
        let library = assets.join("libsherpa-onnx-c-api.dylib");
        let fingerprint = |path: &Path| Fingerprint::capture(path, &|| false).unwrap();
        let binary = fingerprint(&exe);
        let audio = assets.join("japanese-voiced-test.wav");
        let mut file = std::fs::File::open(&audio).unwrap();
        let length = file.metadata().unwrap().len();
        let layout = crate::audio::wave::inspect(&mut file, length).unwrap();
        assert!(layout.frames > 16000);
        for variant in ["int8-fp32", "fp32"] {
            let root = tempfile::tempdir().unwrap();
            let root_path = root.path().canonicalize().unwrap();
            let snapshot = crate::local_engine::resident::Snapshot {
                protocol: 1,
                engine: "asr-transducer".into(),
                name: worker_name.into(),
                bytes: binary.size,
                sha256: binary.sha256.clone(),
                gpu: None,
            };
            let suffix = if variant == "fp32" { "" } else { ".int8" };
            let mut c = config(TRANSDUCER);
            c.executable = binary.clone();
            c.model = fingerprint(&assets.join(format!("encoder-epoch-99-avg-1{suffix}.onnx")));
            c.vad_model = Some(fingerprint(&assets.join("silero_vad.onnx")));
            c.vad_options = Some(adapter(&c).unwrap().descriptor().default_vad);
            c.backend.as_mut().unwrap().files = [
                ("decoder", "decoder-epoch-99-avg-1.onnx".to_owned()),
                ("joiner", format!("joiner-epoch-99-avg-1{suffix}.onnx")),
                ("tokens", "tokens.txt".to_owned()),
            ]
            .into_iter()
            .map(|(k, p)| (k.to_owned(), fingerprint(&assets.join(p))))
            .collect();
            c.backend
                .as_mut()
                .unwrap()
                .files
                .insert("library".into(), fingerprint(&library));
            c.verify(&|| false).unwrap();
            snapshot.resolve(&root_path).unwrap();
            let scope =
                crate::local_engine::resident::Scope::enter(&root_path, Some(&snapshot)).unwrap();
            let mut previous = None;
            let mut load_count = 0;
            for i in 0..2 {
                let dir = root.path().join(format!("unit-{i}"));
                std::fs::create_dir(&dir).unwrap();
                super::super::input::Input::new(&audio, 0, layout.frames, &|| false)
                    .unwrap()
                    .write(&dir)
                    .unwrap();
                let result = adapter(&c)
                    .unwrap()
                    .recognize(
                        &c,
                        &dir,
                        (layout.frames * 1_000_000 / 16000) as i64,
                        &|| false,
                    )
                    .unwrap()
                    .unwrap();
                assert!(!result.is_empty());
                if let Some(before) = &previous {
                    assert_eq!(before, &result);
                }
                previous = Some(result);
                let log = std::fs::read_to_string(dir.join("stderr.log")).unwrap();
                load_count += log.matches("mon_model_load: asr-transducer=1").count();
            }
            drop(scope);
            assert_eq!(load_count, 1, "{variant}");
        }
    }
}
