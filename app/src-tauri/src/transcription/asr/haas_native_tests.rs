//! Opt-in CPU integration checks using synthetic Japanese audio and real Haas
//! weights. Set MON_HAAS_ASSETS and optionally MON_HAAS_WORKER. All execution,
//! placement and preferences are isolated from the user's storage.
use super::*;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

fn repository_path(value: impl AsRef<Path>) -> PathBuf {
    let path = value.as_ref();
    if path.is_absolute() {
        path.to_owned()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }
}

fn assets() -> PathBuf {
    repository_path(std::env::var_os("MON_HAAS_ASSETS").expect("set MON_HAAS_ASSETS"))
        .canonicalize()
        .unwrap()
}

fn configuration() -> AsrConfig {
    let assets = assets();
    let worker = std::env::var_os("MON_HAAS_WORKER")
        .map(repository_path)
        .unwrap_or_else(|| {
            repository_path(if cfg!(windows) {
                ".cache/haas-native/mon-transducer-worker.exe"
            } else {
                ".cache/inference-runtime/package/mon-transducer-worker"
            })
        });
    #[cfg(windows)]
    let library = crate::windows_runtime::resources()
        .unwrap()
        .join("sherpa/sherpa-onnx-c-api.dll");
    #[cfg(not(windows))]
    let library = assets.join("libsherpa-onnx-c-api.dylib");
    let fingerprint = |path: &Path| Fingerprint::capture(path, &|| false).unwrap();
    let mut config = super::tests::config(NEMO_TDT);
    config.executable = fingerprint(&worker.canonicalize().unwrap());
    config.model = fingerprint(&assets.join("encoder.quantized.onnx"));
    config.vad_model = Some(fingerprint(&assets.join("silero_vad.onnx")));
    config.vad_options = Some(adapter(&config).unwrap().descriptor().default_vad);
    let backend = config.backend.as_mut().unwrap();
    backend.files = [
        ("bpeVocab", "bpe.vocab"),
        ("data", "encoder.data"),
        ("decoder", "decoder.onnx"),
        ("joiner", "joiner.onnx"),
        ("tokens", "tokens.txt"),
    ]
    .into_iter()
    .map(|(slot, file)| (slot.to_owned(), fingerprint(&assets.join(file))))
    .collect();
    backend
        .files
        .insert("library".into(), fingerprint(&library));
    assert_eq!(
        config.model.sha256,
        "aba291d61d3d594bc6fb8c7f90acbd980e7bc7bf6b7caaf3965ddc25e6f42fac"
    );
    for (slot, expected) in [
        (
            "bpeVocab",
            "a4ac26a977eff548b292397ddf8357279dbe60ac33ff0adb56ec28fe1c616e40",
        ),
        (
            "data",
            "30befbf6aeb75383e6551f5e58de53efdb76972ddc7d0f76070c517899873586",
        ),
        (
            "decoder",
            "2947cb5512699cbba753fe45fe221428aadaa9856597debbb7aabe19cc253622",
        ),
        (
            "joiner",
            "8a68320e750890ff9e944a794b41ae1c42c6f439ac4c273503458a6b42adc12d",
        ),
        (
            "tokens",
            "ec78b13c5d9f156cf0d5fdbd6b5aec5785680f319c92630c11d5d11b14b83a84",
        ),
    ] {
        assert_eq!(
            config.backend.as_ref().unwrap().files[slot].sha256,
            expected
        );
    }
    config.verify(&|| false).unwrap();
    config
}

fn session(root: &Path, config: &AsrConfig) -> crate::local_engine::resident::Scope {
    let name = config
        .executable
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let snapshot = crate::local_engine::resident::Snapshot {
        protocol: 1,
        engine: "asr-transducer".into(),
        name: name.into(),
        bytes: config.executable.size,
        sha256: config.executable.sha256.clone(),
        gpu: None,
    };
    // An explicitly selected test worker need not replace the bundled runtime.
    // Install these exact bytes into this test's own content-addressed cache.
    let cache =
        crate::model_placement::fs::Directory::root(&root.join("inference-runtimes")).unwrap();
    let directory = cache.child(&snapshot.sha256, true).unwrap();
    fs::copy(&config.executable.path, directory.path().join(name)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory.path().join(name),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    snapshot.resolve(root).unwrap();
    crate::local_engine::resident::Scope::enter(root, Some(&snapshot)).unwrap()
}

fn wave_layout(path: &Path) -> crate::audio::wave::Layout {
    let mut file = fs::File::open(path).unwrap();
    let size = file.metadata().unwrap().len();
    crate::audio::wave::inspect(&mut file, size).unwrap()
}

fn checked_run(config: &AsrConfig, directory: &Path, wave: &Path) -> Vec<Segment> {
    fs::create_dir(directory).unwrap();
    let frames = wave_layout(wave).frames;
    let duration = (frames * 1_000_000 / 16_000) as i64;
    crate::transcription::input::Input::new(wave, 0, frames, &|| false)
        .unwrap()
        .write(directory)
        .unwrap();
    let adapter = adapter(config).unwrap();
    let segments = adapter
        .recognize(config, directory, duration, &|| false)
        .unwrap()
        .unwrap();
    let bytes = fs::read(directory.join("result.json")).unwrap();
    assert_eq!(
        adapter.parse_result(&bytes, config, duration).unwrap(),
        segments
    );
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(raw["backendId"], NEMO_TDT);
    assert!(raw["models"].get("data").is_some());
    assert!(raw["models"].get("bpeVocab").is_some());
    for segment in &segments {
        assert!(segment.start_us >= 0 && segment.end_us <= duration);
        assert!(segment.start_us < segment.end_us);
        assert!(!segment.text.trim().is_empty());
        assert!(!segment.text.contains('▁'), "{}", segment.text);
        assert!(!segment.text.contains("<0x"), "{}", segment.text);
    }
    segments
}

#[test]
#[ignore = "requires MON_HAAS_ASSETS and a built Haas-capable native worker"]
fn native_haas_reuses_resident_model_without_speech_or_hint_state_leaks() {
    let base = configuration();
    let voiced = assets().join("japanese-voiced-test.wav");
    for prompt in ["", "確認"] {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let silence = root_path.join("silence.wav");
        let mut bytes = crate::audio::wave::header(32_000).unwrap();
        bytes.extend(vec![0; 64_000]);
        fs::write(&silence, bytes).unwrap();
        let mut config = base.clone();
        config.prompt = prompt.into();
        config.vad_options.as_mut().unwrap().enabled = true;
        let scope = session(&root_path, &config);
        let mut first = None;
        let mut loads = 0;
        for (index, wave) in [&voiced, &silence, &voiced].into_iter().enumerate() {
            let directory = root_path.join(format!("unit-{index}"));
            let segments = checked_run(&config, &directory, wave);
            if index == 1 {
                assert!(
                    segments.is_empty(),
                    "silence leaked speech or hotwords: {segments:?}"
                );
            } else {
                assert!(!segments.is_empty());
                if let Some(first) = &first {
                    assert_eq!(first, &segments, "stream state leaked into repeated speech");
                } else {
                    eprintln!("Haas prompt={prompt:?}: {segments:?}");
                    first = Some(segments);
                }
            }
            loads += fs::read_to_string(directory.join("stderr.log"))
                .unwrap()
                .matches("mon_model_load: asr-transducer=1")
                .count();
        }
        drop(scope);
        assert_eq!(loads, 1, "model loaded repeatedly for prompt {prompt:?}");
    }
}

#[test]
#[ignore = "requires MON_HAAS_ASSETS and a built Haas-capable native worker"]
fn native_haas_long_audio_retains_speech_after_29_seconds_without_vad() {
    let base = configuration();
    let voiced = assets().join("japanese-voiced-test.wav");
    let layout = wave_layout(&voiced);
    let mut input = fs::File::open(&voiced).unwrap();
    input.seek(SeekFrom::Start(layout.data_offset)).unwrap();
    let mut pcm = vec![0; (layout.frames * 2) as usize];
    input.read_exact(&mut pcm).unwrap();
    assert!(layout.frames * 2 > 29 * 16_000);
    assert!(layout.frames * 2 <= 124 * 16_000);
    for prompt in ["", "確認"] {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let wave = root_path.join("long.wav");
        let mut bytes = crate::audio::wave::header(layout.frames * 2).unwrap();
        bytes.extend_from_slice(&pcm);
        bytes.extend_from_slice(&pcm);
        fs::write(&wave, bytes).unwrap();
        let mut config = base.clone();
        config.prompt = prompt.into();
        config.vad_options.as_mut().unwrap().enabled = false;
        let scope = session(&root_path, &config);
        let segments = checked_run(&config, &root_path.join("long-unit"), &wave);
        assert!(!segments.is_empty());
        assert!(segments.iter().any(|s| s.end_us > 29_000_000));
        assert!(segments
            .windows(2)
            .all(|pair| pair[0].end_us <= pair[1].start_us));
        drop(scope);
    }
}

#[test]
#[ignore = "requires MON_HAAS_ASSETS and a built Haas-capable native worker"]
fn native_haas_rejects_missing_changed_and_misplaced_external_data() {
    let config = configuration();
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let model = root_path.join("encoder.quantized.onnx");
    fs::copy(&config.model.path, &model).unwrap();
    let mut broken = config.clone();
    broken.model.path = model;
    let data_path = root_path.join("encoder.data");
    broken
        .backend
        .as_mut()
        .unwrap()
        .files
        .get_mut("data")
        .unwrap()
        .path = data_path.clone();
    assert!(
        broken.verify(&|| false).is_err(),
        "missing data was accepted"
    );
    fs::write(&data_path, b"changed external data").unwrap();
    assert!(
        broken.verify(&|| false).is_err(),
        "changed data was accepted"
    );
    broken
        .backend
        .as_mut()
        .unwrap()
        .files
        .get_mut("data")
        .unwrap()
        .path = config.backend.as_ref().unwrap().files["data"].path.clone();
    assert!(
        broken.validate_settings().is_err(),
        "misplaced data was accepted"
    );
}

#[test]
#[ignore = "requires MON_HAAS_ASSETS and worker; places real models in isolated temporary storage"]
fn native_haas_package_checks_both_decoders_and_applies_a_managed_profile() {
    use crate::model_inspection::{PackageFile, PackageManifest, Target};
    let config = configuration();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    let mut members = vec![
        (
            "bin/mon-transducer-worker".to_owned() + if cfg!(windows) { ".exe" } else { "" },
            Role::Executable,
            config.executable.path.clone(),
        ),
        (
            "models/encoder.quantized.onnx".into(),
            Role::Asr,
            config.model.path.clone(),
        ),
        (
            "models/silero_vad.onnx".into(),
            Role::Vad,
            config.vad_model.as_ref().unwrap().path.clone(),
        ),
    ];
    let mut backend_files = BTreeMap::new();
    let backend = config.backend.as_ref().unwrap();
    for (slot, proof) in &backend.files {
        if slot == "library" {
            continue;
        }
        let relative = format!(
            "models/{}",
            proof.path.file_name().unwrap().to_str().unwrap()
        );
        members.push((relative.clone(), Role::AsrAsset, proof.path.clone()));
        backend_files.insert(slot.clone(), relative);
    }
    let library = &backend.files["library"].path;
    for entry in fs::read_dir(library.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        let extension = if cfg!(windows) { "dll" } else { "dylib" };
        if path.extension().and_then(|v| v.to_str()) != Some(extension) {
            continue;
        }
        let relative = format!("lib/{}", path.file_name().unwrap().to_str().unwrap());
        if path.file_name() == library.file_name() {
            backend_files.insert("library".into(), relative.clone());
        }
        members.push((relative, Role::RuntimeLibrary, path));
    }
    let mut files = vec![];
    for (relative, role, input) in members {
        let destination = source.join(&relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        // Managed inspection requires single-owner regular files, including
        // the source. Do not weaken that boundary with cached hard links.
        fs::copy(&input, &destination).unwrap();
        let proof = Fingerprint::capture(&destination, &|| false).unwrap();
        files.push(PackageFile {
            relative_path: relative,
            role,
            expected_size: Some(proof.size),
            expected_hash: Some(proof.sha256),
        });
    }
    let manifest = PackageManifest {
        schema_version: 1,
        engine: Engine::Asr,
        engine_version: "v1.13.8".into(),
        model_name: "Haas isolated native integration".into(),
        model_version: "7f5ab0a4b1bf39db78afd6691eeecbc4f9d42e39".into(),
        quantization: Some("int4-int8-fp32".into()),
        license_ref: "Apache-2.0; synthetic test package from pinned community conversion".into(),
        target: Target::current().unwrap(),
        files,
        backend: Some(BackendManifest {
            id: NEMO_TDT.into(),
            files: backend_files,
        }),
    };
    fs::write(
        source.join(crate::model_inspection::MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let inspected =
        crate::model_inspection::inspect(&source.canonicalize().unwrap(), &|| false).unwrap();
    let models = root.path().join("models");
    let storage =
        crate::storage::Storage::open_with_models(root.path().join("data"), models.clone())
            .unwrap();
    let package = uuid::Uuid::new_v4().to_string();
    crate::model_placement::place(
        &storage,
        &models,
        &source.canonicalize().unwrap(),
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let summary = crate::model_compatibility::check(
        &storage,
        &models,
        &operation,
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    assert_eq!(
        summary.state,
        crate::model_compatibility::CheckState::Succeeded,
        "{summary:?}"
    );
    crate::model_compatibility::verify_success(
        &storage,
        &models,
        &operation,
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    let (_, report) = storage.model_check_record(&operation).unwrap().unwrap();
    let report = report.unwrap();
    assert_eq!(report.settings["additionalHotwordsCheck"], "確認");
    assert_eq!(report.settings["vadEnabled"], false);
    assert_eq!(report.artifacts.len(), 8);
    assert!(report
        .artifacts
        .iter()
        .any(|a| a.name == "with-hints/result.json"));
    let preview = crate::model_application::preview(
        &storage,
        &models,
        crate::storage::model_profiles::ProfileName::Standard,
        &package,
        &inspected.review_hash,
        &operation,
        &|| false,
    )
    .unwrap();
    let profile = crate::model_application::apply(
        &storage,
        &models,
        &uuid::Uuid::new_v4().to_string(),
        &preview,
        &|| false,
    )
    .unwrap();
    assert!(profile
        .components
        .iter()
        .any(|c| c.engine == Engine::Asr && c.settings_match && c.compatibility_current));
}
