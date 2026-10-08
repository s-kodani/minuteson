use super::*;
use serde_json::json;

fn ready() -> serde_json::Value {
    json!({"schemaVersion":1,"engine":"sherpa-onnx","engineVersion":"1.13.8","sampleRate":16000,
        "firstFrame":16000,"endFrame":64000,"dimension":3,"status":"ready","reason":null,
        "vectorBits":["3e800000","80000000","3f800000"]})
}
fn checked(value: &serde_json::Value) -> Result<Output, String> {
    parse(&serde_json::to_vec(value).unwrap(), 16000, 64000)
}

#[test]
fn saved_feature_protocol_preserves_bits_and_unavailable_evidence() {
    let output = checked(&ready()).unwrap();
    let vector = output.vector(16000, 64000).unwrap().unwrap();
    assert_eq!(
        vector.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        vec![0x3e800000, 0x80000000, 0x3f800000]
    );
    let restored: Output = serde_json::from_slice(&serde_json::to_vec(&output).unwrap()).unwrap();
    assert_eq!(restored, output);
    assert!(output.vector(0, 48000).is_err());
    for reason in [
        "insufficient_audio",
        "non_finite_embedding",
        "zero_embedding",
    ] {
        let mut value = ready();
        value["status"] = json!("unavailable");
        value["reason"] = json!(reason);
        value["vectorBits"] = serde_json::Value::Null;
        assert!(checked(&value)
            .unwrap()
            .vector(16000, 64000)
            .unwrap()
            .is_none());
    }
}

#[test]
fn saved_feature_protocol_rejects_wrong_windows_engine_dimensions_status_and_values() {
    for (key, mutation) in [
        ("schemaVersion", json!(2)),
        ("engine", json!("other")),
        ("engineVersion", json!("1.13.9")),
        ("sampleRate", json!(48000)),
        ("firstFrame", json!(0)),
        ("endFrame", json!(64001)),
        ("dimension", json!(0)),
        ("dimension", json!(4097)),
        ("dimension", json!(2)),
        ("status", json!("invented")),
        ("status", json!("unavailable")),
        ("reason", json!("zero_embedding")),
        ("vectorBits", json!(null)),
        ("vectorBits", json!(["3e800000", "00000000", "7f800000"])),
        ("vectorBits", json!(["3e800000", "00000000", "7fc00000"])),
        ("vectorBits", json!(["00000000", "80000000", "00000000"])),
        ("vectorBits", json!(["3E800000", "00000000", "3f800000"])),
        ("vectorBits", json!(["+e800000", "00000000", "3f800000"])),
        ("vectorBits", json!(["3e80000", "00000000", "3f800000"])),
        ("extra", json!(true)),
    ] {
        let mut value = ready();
        value[key] = mutation;
        assert!(checked(&value).is_err(), "{key}: {value}");
    }
}

#[test]
fn feature_protocol_refuses_duplicate_missing_and_multiple_json_results() {
    let bytes = serde_json::to_vec(&ready()).unwrap();
    let duplicate = String::from_utf8(bytes.clone())
        .unwrap()
        .replacen("{", "{\"dimension\":3,", 1);
    assert!(parse(duplicate.as_bytes(), 16000, 64000).is_err());
    assert!(parse(&[bytes.clone(), bytes].concat(), 16000, 64000).is_err());
    assert!(parse(&vec![b' '; MAX_OUTPUT as usize + 1], 16000, 64000).is_err());
    for key in [
        "schemaVersion",
        "engine",
        "engineVersion",
        "sampleRate",
        "firstFrame",
        "endFrame",
        "dimension",
        "status",
        "reason",
        "vectorBits",
    ] {
        let mut value = ready();
        value.as_object_mut().unwrap().remove(key);
        assert!(checked(&value).is_err());
    }
}

#[test]
fn log_ownership_rejects_alias_replacement_and_excess_even_after_native_exit() {
    let directory = tempfile::tempdir().unwrap();
    let directory = directory.path().canonicalize().unwrap();
    let path = directory.join("result");
    let mut output = Log::create(path.clone(), 4).unwrap();
    std::fs::write(&path, b"1234").unwrap();
    assert_eq!(output.bytes().unwrap(), b"1234");
    std::fs::write(&path, b"12345").unwrap();
    assert!(output.bytes().is_err());
    let replacement = directory.join("replacement");
    std::fs::write(&replacement, b"1").unwrap();
    std::fs::rename(replacement, &path).unwrap();
    assert!(output.bytes().is_err());
}

#[test]
fn helper_conditions_require_exact_runtime_membership_and_preserve_legacy_engine_json() {
    let proof = |name: &str| Fingerprint {
        path: std::path::PathBuf::from(format!("/private/tmp/{name}")),
        size: 1,
        sha256: "a".repeat(64),
    };
    let engine = SherpaConfig {
        executable: proof("cli"),
        segmentation_model: proof("segmentation"),
        embedding_model: proof("embedding"),
        runtime_libraries: vec![proof("libsherpa-onnx-c-api.dylib")],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 500,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    };
    let original = serde_json::to_vec(&engine).unwrap();
    let config = ExtractorConfig {
        helper: proof("helper"),
        library: engine.runtime_libraries[0].clone(),
    };
    #[cfg(target_os = "macos")]
    config.validate(&engine).unwrap();
    for invalid in [
        ExtractorConfig {
            library: proof("other-runtime"),
            ..config.clone()
        },
        ExtractorConfig {
            helper: Fingerprint {
                sha256: "A".repeat(64),
                ..config.helper.clone()
            },
            ..config.clone()
        },
        ExtractorConfig {
            helper: Fingerprint {
                size: 0,
                ..config.helper.clone()
            },
            ..config.clone()
        },
        ExtractorConfig {
            helper: Fingerprint {
                path: "relative-helper".into(),
                ..config.helper.clone()
            },
            ..config.clone()
        },
    ] {
        assert!(invalid.validate(&engine).is_err());
    }
    let mut duplicates = engine.clone();
    duplicates.runtime_libraries.push(config.library.clone());
    assert!(config.validate(&duplicates).is_err());
    assert_eq!(serde_json::to_vec(&engine).unwrap(), original);
}

#[test]
fn unavailable_protocol_cannot_hide_vectors_or_unknown_reasons() {
    let mut value = ready();
    value["status"] = json!("unavailable");
    value["reason"] = json!("insufficient_audio");
    assert!(checked(&value).is_err());
    value["vectorBits"] = json!(null);
    value["reason"] = json!("invented");
    assert!(checked(&value).is_err());
    value["reason"] = json!(null);
    assert!(checked(&value).is_err());
    for (first, end) in [(0, 0), (1, 0), (0, 480001)] {
        value["firstFrame"] = json!(first);
        value["endFrame"] = json!(end);
        value["reason"] = json!("insufficient_audio");
        assert!(parse(&serde_json::to_vec(&value).unwrap(), first, end).is_err());
    }
}

fn write_wave(path: &Path) {
    let frames = 80000_u32;
    let data = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.extend_from_slice(&32000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for _ in 0..frames {
        bytes.extend_from_slice(&8192_i16.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

struct Fixture {
    directory: tempfile::TempDir,
    engine: SherpaConfig,
    config: ExtractorConfig,
    wave: Fingerprint,
}
impl Fixture {
    fn fake(mode: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let library = root.join("libsherpa-onnx-c-api.dylib");
        std::fs::copy(
            std::env::var("MON_EMBEDDING_FAKE_LIBRARY").unwrap(),
            &library,
        )
        .unwrap();
        let model = root.join("model.bin");
        std::fs::write(&model, mode).unwrap();
        let wave = root.join("run.wav");
        write_wave(&wave);
        let proof = |path: &Path| Fingerprint::capture(path, &|| false).unwrap();
        let engine = SherpaConfig {
            executable: proof(Path::new(&std::env::var("MON_EMBEDDING_HELPER").unwrap())),
            segmentation_model: proof(&model),
            embedding_model: proof(&model),
            runtime_libraries: vec![proof(&library)],
            threads: 1,
            cluster_threshold_milli: 500,
            window_shift_milli: 500,
            min_duration_on_ms: 0,
            min_duration_off_ms: 0,
        };
        let config = ExtractorConfig::capture(&engine.executable.path, &engine).unwrap();
        Self {
            directory,
            engine,
            config,
            wave: proof(&wave),
        }
    }
    fn root(&self) -> std::path::PathBuf {
        self.directory.path().canonicalize().unwrap()
    }
    fn job(&self) -> std::path::PathBuf {
        let path = self.root().join("job");
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.root().join("model.log"))
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("pid:")?.parse().ok())
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires task-owned helper and fake ABI runtime, no capture"]
fn feature_child_verifies_fixed_inputs_and_reaps_on_cancellation_timeout_overflow_and_failure() {
    for mode in [
        "ready",
        "notready",
        "nan",
        "zero",
        "tamper",
        "tamper_wave",
        "crash",
        "hang",
        "stdout_flood",
        "stderr_flood",
    ] {
        let fixture = Fixture::fake(mode);
        let job = fixture.job();
        let before = std::fs::read(&fixture.wave.path).unwrap();
        let result = run(
            &fixture.config,
            &fixture.engine,
            &fixture.wave,
            16000,
            64000,
            &job,
            Duration::from_millis(if mode == "hang" { 200 } else { 5000 }),
            &|| false,
        );
        match mode {
            "ready" => assert_eq!(
                result
                    .unwrap()
                    .unwrap()
                    .vector(16000, 64000)
                    .unwrap()
                    .unwrap(),
                vec![0.25, 0.25, 48000.0]
            ),
            "notready" | "nan" | "zero" => assert!(result
                .unwrap()
                .unwrap()
                .vector(16000, 64000)
                .unwrap()
                .is_none()),
            "hang" => assert!(result.err().unwrap().contains("制限時間")),
            "stdout_flood" | "stderr_flood" => assert!(result.err().unwrap().contains("上限")),
            "crash" => assert!(result.err().unwrap().contains("失敗")),
            _ => assert!(result.is_err(), "{mode}"),
        }
        if mode != "tamper_wave" {
            assert_eq!(std::fs::read(&fixture.wave.path).unwrap(), before);
        }
        if let Some(pid) = fixture.pid() {
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                -1,
                "child {pid} left alive in {mode}"
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
    let fixture = Fixture::fake("hang");
    let job = fixture.job();
    assert!(run(
        &fixture.config,
        &fixture.engine,
        &fixture.wave,
        16000,
        64000,
        &job,
        Duration::from_secs(5),
        &|| fixture.pid().is_some()
    )
    .unwrap()
    .is_none());
    let pid = fixture.pid().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    for role in ["model", "wave", "runtime", "helper"] {
        let mut fixture = Fixture::fake("ready");
        let job = fixture.job();
        if role == "helper" {
            // Modify an owned copy, never the executable belonging to the
            // test runner or another verification task.
            let helper = fixture.root().join("helper");
            std::fs::copy(&fixture.config.helper.path, &helper).unwrap();
            fixture.config.helper = Fingerprint::capture(&helper, &|| false).unwrap();
        }
        let path = match role {
            "model" => &fixture.engine.embedding_model.path,
            "wave" => &fixture.wave.path,
            "runtime" => &fixture.config.library.path,
            _ => &fixture.config.helper.path,
        };
        std::fs::write(path, "other").unwrap();
        assert!(
            run(
                &fixture.config,
                &fixture.engine,
                &fixture.wave,
                16000,
                64000,
                &job,
                Duration::from_secs(5),
                &|| false
            )
            .is_err(),
            "{role}"
        );
        assert!(!job.join("embedding.stdout.json").exists());
        assert!(!fixture.root().join("model.log").exists());
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires pinned real sherpa model/runtime and headless helper, synthetic PCM only"]
fn real_feature_child_repeats_exact_synthetic_features_with_fixed_library_model_and_wave() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let wave_path = root.join("run.wav");
    write_wave(&wave_path);
    let mut bytes = std::fs::read(&wave_path).unwrap();
    for (index, chunk) in bytes[44..].chunks_exact_mut(2).enumerate() {
        let value =
            ((index as f64 * 220.0 * std::f64::consts::TAU / 16000.0).sin() * 8000.0) as i16;
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    std::fs::write(&wave_path, &bytes).unwrap();
    let proof = |path: &Path| Fingerprint::capture(path, &|| false).unwrap();
    let helper = proof(Path::new(&std::env::var("MON_EMBEDDING_HELPER").unwrap()));
    let library = proof(Path::new(
        &std::env::var("MON_EMBEDDING_REAL_LIBRARY").unwrap(),
    ));
    let lib_dir = library.path.parent().unwrap();
    let engine = SherpaConfig {
        executable: proof(
            &lib_dir
                .parent()
                .unwrap()
                .join("bin/sherpa-onnx-offline-speaker-diarization"),
        ),
        segmentation_model: proof(Path::new(
            &std::env::var("MON_EMBEDDING_REAL_SEGMENTATION").unwrap(),
        )),
        embedding_model: proof(Path::new(
            &std::env::var("MON_EMBEDDING_REAL_MODEL").unwrap(),
        )),
        runtime_libraries: vec![
            library.clone(),
            proof(&lib_dir.join("libonnxruntime.dylib")),
            proof(&lib_dir.join("libsherpa-onnx-cxx-api.dylib")),
        ],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 500,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    };
    let config = ExtractorConfig { helper, library };
    let wave = proof(&wave_path);
    let mut previous = None;
    for name in ["first", "second"] {
        let job = root.join(name);
        std::fs::create_dir(&job).unwrap();
        let output = run(
            &config,
            &engine,
            &wave,
            16000,
            64000,
            &job,
            Duration::from_secs(30),
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert!(output.vector(16000, 64000).unwrap().is_some());
        let stdout = std::fs::read(job.join("embedding.stdout.json")).unwrap();
        assert_eq!(parse(&stdout, 16000, 64000).unwrap(), output);
        if let Some(previous) = previous {
            assert_eq!(output, previous);
        }
        previous = Some(output);
    }
    wave.verify(&|| false).unwrap();
    engine.verify(&|| false).unwrap();
    assert_eq!(std::fs::read(wave_path).unwrap(), bytes);
}
