use super::*;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "mon-speaker-feature-test-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, samples: &[i16]) -> PathBuf {
        let mut bytes = Vec::new();
        let data = samples.len() as u32 * 2;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(data + 36).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&16_000_u32.to_le_bytes());
        bytes.extend_from_slice(&32_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let path = self.0.join("run.wav");
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn request(&self, library: &Path, mode: &str) -> Request {
        let model = self.0.join("model.bin");
        std::fs::write(&model, mode).unwrap();
        let samples: Vec<_> = (0..80_000)
            .map(|i| {
                if i < 16_000 {
                    4096
                } else if i < 64_000 {
                    8192
                } else {
                    -16384
                }
            })
            .collect();
        Request {
            library: library.to_owned(),
            model,
            wave: self.wave(&samples),
            first: 16_000,
            end: 64_000,
            threads: 1,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn args() -> Vec<String> {
    [
        "--protocol-version",
        "1",
        "--library",
        "/private/tmp/library.dylib",
        "--model",
        "/private/tmp/model.onnx",
        "--wave",
        "/private/tmp/run.wav",
        "--first-frame",
        "0",
        "--end-frame",
        "16000",
        "--threads",
        "1",
    ]
    .iter()
    .map(|v| v.to_string())
    .collect()
}

#[test]
fn named_request_refuses_duplicate_unknown_noncanonical_and_out_of_window_values() {
    assert_eq!(request(&args()).unwrap().end, 16_000);
    for (index, value) in [
        (0, "--wrong"),
        (1, "2"),
        (2, "--model"),
        (3, "relative"),
        (9, "01"),
        (9, "-1"),
        (9, "18446744073709551616"),
        (11, "0"),
        (11, "480001"),
        (13, "0"),
        (13, "65"),
    ] {
        let mut input = args();
        input[index] = value.into();
        assert!(request(&input).is_err(), "{index}: {value}");
    }
    assert!(request(&args()[..12]).is_err());
}

#[test]
fn selected_wave_preserves_exact_window_without_padding_or_modifying_source() {
    let fixture = Fixture::new();
    let path = fixture.wave(&[4096, 8192, -16384, -32768, 32767]);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(wave::selected(&path, 1, 4).unwrap(), vec![0.25, -0.5, -1.0]);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn truncated_changed_formats_extra_chunks_and_invalid_ranges_are_rejected() {
    let fixture = Fixture::new();
    let path = fixture.wave(&[8192; 100]);
    let original = std::fs::read(&path).unwrap();
    for offset in [0, 8, 12, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
        let mut bytes = original.clone();
        bytes[offset] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(wave::selected(&path, 0, 100).is_err(), "offset {offset}");
    }
    for bytes in [
        original[..40].to_vec(),
        original[..original.len() - 1].to_vec(),
        [original.clone(), vec![0; 8]].concat(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(wave::selected(&path, 0, 100).is_err());
    }
    std::fs::write(&path, original).unwrap();
    for (first, end) in [
        (0, 0),
        (1, 0),
        (0, 101),
        (0, 480001),
        (u64::MAX - 1, u64::MAX),
    ] {
        assert!(wave::selected(&path, first, end).is_err());
    }
}

#[test]
fn zero_pcm_is_real_data_but_empty_or_linked_waves_are_not_created_as_success() {
    let fixture = Fixture::new();
    let path = fixture.wave(&[0; 32]);
    assert_eq!(wave::selected(&path, 0, 32).unwrap(), vec![0.0; 32]);
    assert!(wave::selected(&fixture.wave(&[]), 0, 1).is_err());
    #[cfg(unix)]
    {
        fixture.wave(&[0; 32]);
        let alias = fixture.0.join("linked.wav");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(wave::selected(&alias, 0, 32).is_err());
    }
}

#[test]
fn protocol_keeps_ieee_bits_and_separates_unavailable_from_success() {
    let fixture = Fixture::new();
    let request = fixture.request(Path::new("/private/tmp/fixed.dylib"), "ready");
    let output = Output {
        dimension: 3,
        vector: Some(vec![0.25, -0.0, f32::MIN_POSITIVE]),
        reason: None,
    };
    let json = output.json(&request).unwrap();
    assert!(json.contains("\"vectorBits\":[\"3e800000\",\"80000000\",\"00800000\"]"));
    assert!(json.contains("\"firstFrame\":16000,\"endFrame\":64000"));
    assert!(Output {
        dimension: 3,
        vector: None,
        reason: Some("insufficient_audio")
    }
    .json(&request)
    .unwrap()
    .contains("\"status\":\"unavailable\""));
    for output in [
        Output {
            dimension: 0,
            vector: Some(vec![]),
            reason: None,
        },
        Output {
            dimension: 1,
            vector: Some(vec![f32::NAN]),
            reason: None,
        },
        Output {
            dimension: 1,
            vector: None,
            reason: None,
        },
        Output {
            dimension: 1,
            vector: None,
            reason: Some("invented"),
        },
        Output {
            dimension: 1,
            vector: Some(vec![1.0]),
            reason: Some("zero_embedding"),
        },
    ] {
        assert!(output.json(&request).is_err());
    }
}

#[test]
fn file_replacement_is_not_equal_even_with_same_length_and_modified_time() {
    let fixture = Fixture::new();
    let path = fixture.wave(&[0; 32]);
    let before = wave::identity(&path).unwrap();
    let replacement = fixture.0.join("other.wav");
    std::fs::copy(&path, &replacement).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    assert!(wave::same(&before, &wave::identity(&path).unwrap()).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn native_rejects_invalid_sample_count_and_ranges_before_opening_any_library() {
    let fixture = Fixture::new();
    let mut input = fixture.request(Path::new("/private/tmp/no-such-runtime.dylib"), "ready");
    for (first, end, samples) in [
        (1, 0, vec![0.25]),
        (0, 0, vec![]),
        (0, 1, vec![f32::NAN]),
        (0, 1, vec![1.1]),
        (0, 2, vec![0.25]),
        (0, 480001, vec![0.25; 480001]),
    ] {
        input.first = first;
        input.end = end;
        assert_eq!(
            native::extract(&input, &samples).err().unwrap(),
            "invalid native feature input"
        );
    }
}

fn cli_args(input: &Request) -> Vec<String> {
    vec![
        "--protocol-version".into(),
        "1".into(),
        "--library".into(),
        input.library.to_str().unwrap().into(),
        "--model".into(),
        input.model.to_str().unwrap().into(),
        "--wave".into(),
        input.wave.to_str().unwrap().into(),
        "--first-frame".into(),
        input.first.to_string(),
        "--end-frame".into(),
        input.end.to_string(),
        "--threads".into(),
        input.threads.to_string(),
    ]
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires task-owned helper and fake runtimes"]
fn headless_cli_preserves_window_bits_and_refuses_bad_abi_and_failed_extraction() {
    let helper = std::env::var("MON_EMBEDDING_HELPER").unwrap();
    let library = PathBuf::from(std::env::var("MON_EMBEDDING_FAKE_LIBRARY").unwrap())
        .canonicalize()
        .unwrap();
    for mode in [
        "ready",
        "notready",
        "nan",
        "zero",
        "nullcreate",
        "nullstream",
        "nullvector",
        "badready",
        "dimension",
        "tamper",
    ] {
        let fixture = Fixture::new();
        let input = fixture.request(&library, mode);
        let wave_before = std::fs::read(&input.wave).unwrap();
        let result = std::process::Command::new(&helper)
            .args(cli_args(&input))
            .output()
            .unwrap();
        if matches!(mode, "ready" | "notready" | "nan" | "zero") {
            assert!(
                result.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let json = String::from_utf8(result.stdout).unwrap();
            assert_eq!(json.lines().count(), 1);
            assert!(json.contains("\"firstFrame\":16000,\"endFrame\":64000,\"dimension\":3"));
            if mode == "ready" {
                assert!(json.contains("\"vectorBits\":[\"3e800000\",\"3e800000\",\"473b8000\"]"));
            } else {
                assert!(json.contains("\"status\":\"unavailable\""));
                assert!(json.ends_with("\"vectorBits\":null}\n"));
            }
        } else {
            assert_eq!(result.status.code(), Some(1));
            assert!(result.stdout.is_empty());
        }
        assert_eq!(std::fs::read(&input.wave).unwrap(), wave_before);
    }
    for name in [
        "MON_EMBEDDING_WRONG_VERSION_LIBRARY",
        "MON_EMBEDDING_MISSING_SYMBOL_LIBRARY",
    ] {
        let fixture = Fixture::new();
        let library = PathBuf::from(std::env::var(name).unwrap())
            .canonicalize()
            .unwrap();
        let input = fixture.request(&library, "ready");
        let result = std::process::Command::new(&helper)
            .args(cli_args(&input))
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(
            !input.model.with_extension("log").exists(),
            "model must not be opened under an unsupported ABI"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires owned MON_EMBEDDING_FAKE_LIBRARY fixture"]
fn fake_native_abi_checks_cleanup_unavailable_and_model_mutation_without_capture() {
    let library = PathBuf::from(std::env::var("MON_EMBEDDING_FAKE_LIBRARY").unwrap())
        .canonicalize()
        .unwrap();
    for mode in [
        "ready",
        "notready",
        "nan",
        "zero",
        "dimension",
        "nullcreate",
        "nullstream",
        "nullvector",
        "badready",
        "tamper",
    ] {
        let fixture = Fixture::new();
        let request = fixture.request(&library, mode);
        let samples = wave::selected(&request.wave, request.first, request.end).unwrap();
        let result = native::extract(&request, &samples);
        match mode {
            "ready" => {
                let output = result.unwrap();
                assert_eq!(output.vector.unwrap(), vec![0.25, 0.25, 48000.0]);
            }
            "notready" => assert_eq!(result.unwrap().reason, Some("insufficient_audio")),
            "nan" => assert_eq!(result.unwrap().reason, Some("non_finite_embedding")),
            "zero" => assert_eq!(result.unwrap().reason, Some("zero_embedding")),
            _ => assert!(result.is_err(), "{mode}"),
        }
        let log = std::fs::read_to_string(request.model.with_extension("log")).unwrap();
        if mode == "nullcreate" {
            assert_eq!(log, "create\n");
            continue;
        }
        assert!(log.ends_with("destroy_extractor\n"), "{mode}: {log}");
        if !matches!(mode, "dimension" | "nullstream") {
            assert!(log.ends_with("destroy_stream\ndestroy_extractor\n"));
        }
        if matches!(mode, "ready" | "nan" | "zero" | "tamper") {
            assert!(log.contains("destroy_embedding\ndestroy_stream\ndestroy_extractor"));
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires previously verified MON_EMBEDDING_REAL_LIBRARY and MON_EMBEDDING_REAL_MODEL"]
fn real_native_model_extracts_repeatable_synthetic_window_without_modifying_audio() {
    let fixture = Fixture::new();
    let library = PathBuf::from(std::env::var("MON_EMBEDDING_REAL_LIBRARY").unwrap())
        .canonicalize()
        .unwrap();
    let model = PathBuf::from(std::env::var("MON_EMBEDDING_REAL_MODEL").unwrap())
        .canonicalize()
        .unwrap();
    let samples: Vec<_> = (0..80_000)
        .map(|index| {
            ((index as f64 * 220.0 * std::f64::consts::TAU / 16_000.0).sin() * 8000.0) as i16
        })
        .collect();
    let wave = fixture.wave(&samples);
    let before = std::fs::read(&wave).unwrap();
    let request = Request {
        library,
        model,
        wave,
        first: 16_000,
        end: 64_000,
        threads: 1,
    };
    let samples = wave::selected(&request.wave, request.first, request.end).unwrap();
    let first = native::extract(&request, &samples).unwrap();
    let second = native::extract(&request, &samples).unwrap();
    assert!(first.vector.is_some());
    assert_eq!(
        first.json(&request).unwrap(),
        second.json(&request).unwrap()
    );
    assert_eq!(std::fs::read(request.wave).unwrap(), before);
}
