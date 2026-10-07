use super::*;
use crate::audio::{tests::caf_fixture, VerifiedAudio};
use crate::playback::{AudioSpan, PreparedAudio};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;

fn config(directory: &Path) -> SherpaConfig {
    let files: Vec<_> = ["fake-cli", "segmentation ; safe.onnx", "embedding.onnx"]
        .into_iter()
        .map(|name| {
            let path = directory.join(name);
            std::fs::write(&path, b"fixture").unwrap();
            Fingerprint::capture(&path, &|| false).unwrap()
        })
        .collect();
    SherpaConfig {
        executable: files[0].clone(),
        segmentation_model: files[1].clone(),
        embedding_model: files[2].clone(),
        runtime_libraries: vec![],
        threads: 2,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    }
}

fn output(config: &SherpaConfig, count: Option<u32>, rows: &str) -> Vec<u8> {
    format!("{}\nStarted\n{rows}", config.expected_header(count)).into_bytes()
}

#[test]
fn model_rebinding_log_normalization_keeps_overlaps_labels_and_timing_but_cannot_execute_unavailable_paths(
) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = config(directory.path());
    let mut logical = runtime.clone();
    for proof in [
        &mut logical.executable,
        &mut logical.segmentation_model,
        &mut logical.embedding_model,
    ] {
        proof.path = PathBuf::from(format!("mon-unavailable:{}", proof.sha256));
    }
    let bytes = output(
        &runtime,
        Some(3),
        "0.000 -- 0.700 speaker_00\n0.400 -- 0.900 speaker_02\n",
    );
    let normalized = logical_output(&bytes, &runtime, &logical, Some(3), 1_000_000).unwrap();
    assert_eq!(
        parse_output(&normalized, &logical, Some(3), 1_000_000).unwrap(),
        parse_output(&bytes, &runtime, Some(3), 1_000_000).unwrap()
    );
    assert_eq!(
        std::str::from_utf8(&normalized)
            .unwrap()
            .split_once('\n')
            .unwrap()
            .1,
        std::str::from_utf8(&bytes)
            .unwrap()
            .split_once('\n')
            .unwrap()
            .1
    );
    assert!(logical.verify(&|| false).is_err());
    assert!(logical_output(&bytes, &logical, &runtime, Some(3), 1_000_000).is_err());
    let duplicates_after_clamping = output(
        &runtime,
        Some(3),
        "0.000 -- 1.001 speaker_00\n0.000 -- 1.002 speaker_00\n",
    );
    assert!(parse_output(&duplicates_after_clamping, &runtime, Some(3), 1_000_000).is_err());
}

fn wav(directory: &Path) {
    let pcm: Vec<u8> = (0..16_000).flat_map(|_| 0.25_f32.to_le_bytes()).collect();
    let bytes = caf_fixture(16_000, 1, 3, 32, 4, &pcm);
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    let verified = VerifiedAudio::open(
        file,
        bytes.len() as u64,
        &format!("{:x}", Sha256::digest(&bytes)),
    )
    .unwrap();
    let mut source = PreparedAudio {
        clock: None,
        sample_rate: 16_000,
        channels: 1,
        start_us: 0,
        end_us: 1_000_000,
        track_kind: "imported".into(),
        alignment: "unverified".into(),
        spans: vec![AudioSpan {
            audio: verified,
            first_frame: 0,
            frames: 16_000,
        }],
    };
    crate::transcription::wave::write_track(&mut source, &directory.join("input.wav"), &|| false)
        .unwrap();
}

#[test]
fn fixed_protocol_keeps_overlapping_and_disjoint_intervals_without_fabricating_confidence() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let rows = "0.000 -- 0.700 speaker_00\n0.400 -- 0.900 speaker_02\n0.900 -- 1.000 speaker_00\n";
    let segments =
        parse_output(&output(&config, Some(3), rows), &config, Some(3), 1_000_000).unwrap();
    assert_eq!(segments.len(), 3);
    assert_eq!(
        segments[1],
        Segment {
            start_us: 400_000,
            end_us: 900_000,
            speaker: 2
        }
    );
    assert!(!serde_json::to_string(&segments)
        .unwrap()
        .contains("confidence"));
    assert!(
        parse_output(&output(&config, None, ""), &config, None, 1_000_000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        parse_output(
            &output(&config, None, "0.999 -- 1.001 speaker_00\n"),
            &config,
            None,
            1_000_123
        )
        .unwrap()[0]
            .end_us,
        1_000_123
    );
    // More than 10 is accepted by the data model, not claimed as verified.
    assert!(parse_output(
        &output(&config, Some(11), "0.000 -- 0.100 speaker_10\n"),
        &config,
        Some(11),
        1_000_000
    )
    .is_ok());
    // The native sort uses unrounded floats; printed ties may reverse IDs.
    assert!(parse_output(
        &output(
            &config,
            None,
            "0.000 -- 0.500 speaker_02\n0.000 -- 0.100 speaker_00\n"
        ),
        &config,
        None,
        1_000_000
    )
    .is_ok());
}

#[test]
fn rejects_wrong_protocol_identity_settings_order_times_labels_and_extra_columns() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    for rows in [
        "-0.001 -- 0.200 speaker_00\n",
        "NaN -- 0.200 speaker_00\n",
        "0.0 -- 0.200 speaker_00\n",
        "0.000 -- 1.021 speaker_00\n",
        "0.000 -- 0.000 speaker_00\n",
        "1.000 -- 1.001 speaker_00\n",
        "999999999999999999999.000 -- 1.000 speaker_00\n",
        "0.000 -- 0.100 speaker_-1\n",
        "0.000 -- 0.100 speaker_100\n",
        "0.000 -- 0.100 speaker_0\n",
        "0.000 -- 0.100 speaker_+0\n",
        "0.000 -- 0.100 speaker_00 confidence=0.95\n",
        "garbage\n",
        "\n",
        "0.500 -- 0.600 speaker_00\n0.000 -- 0.100 speaker_00\n",
        "0.000 -- 0.100 speaker_00\n0.000 -- 0.100 speaker_00\n",
    ] {
        assert!(
            parse_output(&output(&config, None, rows), &config, None, 1_000_000).is_err(),
            "accepted {rows}"
        );
    }
    for bytes in [
        b"".to_vec(),
        b"Started\n".to_vec(),
        b"\xff\n".to_vec(),
        output(&config, Some(3), ""),
    ] {
        assert!(parse_output(&bytes, &config, None, 1_000_000).is_err());
    }
    let mut changed = config.clone();
    changed.threads = 3;
    assert!(parse_output(&output(&changed, None, ""), &config, None, 1_000_000).is_err());
    assert!(parse_output(
        &output(&config, Some(3), "0.000 -- 0.100 speaker_03\n"),
        &config,
        Some(3),
        1_000_000
    )
    .is_err());
    assert!(parse_output(
        &vec![b'x'; MAX_OUTPUT_BYTES as usize + 1],
        &config,
        None,
        1_000_000
    )
    .is_err());
    for n in [0, 101] {
        assert!(config.validate(Some(n)).is_err());
    }
    let mut bad = config.clone();
    bad.window_shift_milli = 0;
    assert!(bad.validate(None).is_err());
    bad.window_shift_milli = 1001;
    assert!(bad.validate(None).is_err());
    let mut bad = config.clone();
    bad.cluster_threshold_milli = 2001;
    assert!(bad.validate(None).is_err());
    bad = config.clone();
    bad.threads = 65;
    assert!(bad.validate(None).is_err());
    bad = config.clone();
    bad.min_duration_off_ms = 60_001;
    assert!(bad.validate(None).is_err());
}

#[cfg(unix)]
fn executable(config: &mut SherpaConfig, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(&config.executable.path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(
        &config.executable.path,
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    config.executable = Fingerprint::capture(&config.executable.path, &|| false).unwrap();
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
#[test]
fn native_process_gets_literal_frozen_arguments_and_does_not_overwrite_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = config(directory.path());
    let bytes = output(
        &config,
        Some(3),
        "0.000 -- 0.500 speaker_00\n0.200 -- 1.000 speaker_01\n",
    );
    executable(
        &mut config,
        &format!(
            "printf '%s\\n' \"$@\" > args.txt\nprintf '%s' {}",
            shell_quote(std::str::from_utf8(&bytes).unwrap())
        ),
    );
    let unit = directory.path().join("unit");
    std::fs::create_dir(&unit).unwrap();
    wav(&unit);
    let result = run_track(
        &config,
        Some(3),
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.len(), 2);
    let args = std::fs::read_to_string(unit.join("args.txt")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert!(args.contains(
        &format!(
            "--segmentation.pyannote-model={}",
            config.segmentation_model.path.display()
        )
        .as_str()
    ));
    assert!(args.contains(&"--clustering.num-clusters=3"));
    assert!(args.contains(&"--clustering.compute-confidence=false"));
    assert!(args.contains(&"--embedding.provider=cpu"));
    assert!(args.contains(&"--min-duration-off=0"));
    let canonical_wav = unit.join("input.wav").canonicalize().unwrap();
    assert_eq!(args.last(), Some(&canonical_wav.to_str().unwrap()));
    assert!(run_track(
        &config,
        Some(3),
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &|| false
    )
    .is_err());
    assert_eq!(std::fs::read(unit.join("stdout.log")).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn empty_exit_wrong_input_and_changed_dependencies_cannot_be_checkpointed() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = config(directory.path());
    executable(&mut config, "exit 0");
    let unit = directory.path().join("empty");
    std::fs::create_dir(&unit).unwrap();
    wav(&unit);
    assert!(run_track(
        &config,
        None,
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &|| false
    )
    .unwrap_err()
    .contains("出力形式"));
    let unit = directory.path().join("bad-input");
    std::fs::create_dir(&unit).unwrap();
    wav(&unit);
    let mut bytes = std::fs::read(unit.join("input.wav")).unwrap();
    bytes[24] = 1;
    std::fs::write(unit.join("input.wav"), &bytes).unwrap();
    assert!(run_track(
        &config,
        None,
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &|| false
    )
    .unwrap_err()
    .contains("WAV"));
    assert!(!unit.join("stdout.log").exists());
    let library = directory.path().join("runtime.dylib");
    std::fs::write(&library, b"fixture").unwrap();
    config
        .runtime_libraries
        .push(Fingerprint::capture(&library, &|| false).unwrap());
    for target in [
        config.segmentation_model.path.clone(),
        config.embedding_model.path.clone(),
        library,
    ] {
        std::fs::write(&target, b"fixture").unwrap();
        let bytes = output(&config, None, "0.000 -- 1.000 speaker_00\n");
        executable(
            &mut config,
            &format!(
                "printf changed > {}\nprintf '%s' {}",
                shell_quote(target.to_str().unwrap()),
                shell_quote(std::str::from_utf8(&bytes).unwrap())
            ),
        );
        let unit = directory.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&unit).unwrap();
        wav(&unit);
        assert!(run_track(
            &config,
            None,
            &unit,
            1_000_000,
            Duration::from_secs(3),
            &|| false
        )
        .unwrap_err()
        .contains("変更"));
        std::fs::write(&target, b"fixture").unwrap();
    }
}

#[cfg(unix)]
#[test]
fn nonzero_exit_and_excessive_logs_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = config(directory.path());
    for (body, message) in [
        ("exit 7".to_string(), "失敗"),
        (
            format!("exec /usr/bin/head -c {} /dev/zero", MAX_OUTPUT_BYTES + 1),
            "上限",
        ),
        (
            format!("exec /usr/bin/head -c {} /dev/zero >&2", MAX_LOG_BYTES + 1),
            "上限",
        ),
    ] {
        executable(&mut config, &body);
        let unit = directory.path().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&unit).unwrap();
        wav(&unit);
        assert!(run_track(
            &config,
            None,
            &unit,
            1_000_000,
            Duration::from_secs(3),
            &|| false
        )
        .unwrap_err()
        .contains(message));
    }
}

#[cfg(unix)]
#[test]
fn pause_and_timeout_reap_native_process_and_emit_no_result() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = config(directory.path());
    executable(
        &mut config,
        "printf '%s' $$ > child.pid\n: > child.ready\nexec /bin/sleep 30",
    );
    let preflight = directory.path().join("preflight");
    std::fs::create_dir(&preflight).unwrap();
    wav(&preflight);
    assert_eq!(
        run_track(
            &config,
            None,
            &preflight,
            1_000_000,
            Duration::from_millis(400),
            &|| true
        )
        .unwrap(),
        None
    );
    assert!(!preflight.join("child.pid").exists());
    assert!(!preflight.join("stdout.log").exists());
    for pause in [true, false] {
        let unit = directory
            .path()
            .join(if pause { "pause" } else { "timeout" });
        std::fs::create_dir(&unit).unwrap();
        wav(&unit);
        let started = Instant::now();
        let result = run_track(
            &config,
            None,
            &unit,
            1_000_000,
            if pause {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(400)
            },
            // Cancel only after the child's completed PID write. A wall-clock
            // delay starting before preflight can cancel before spawn under load,
            // which is correct but cannot prove that a running child was reaped.
            &|| pause && unit.join("child.ready").is_file(),
        );
        if pause {
            assert_eq!(result.unwrap(), None);
        } else {
            assert!(result.unwrap_err().contains("制限時間"));
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(unit.join("child.pid")).unwrap();
        assert!(!Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }
}

#[test]
#[ignore = "requires MON_SHERPA_CLI, MON_SHERPA_SEGMENTATION, MON_SHERPA_EMBEDDING, MON_SHERPA_INPUT_CAF"]
fn real_sherpa_diarizes_converted_caf_without_disclosing_meeting_text() {
    let proof =
        |name| Fingerprint::capture(Path::new(&std::env::var(name).unwrap()), &|| false).unwrap();
    let config = SherpaConfig {
        executable: proof("MON_SHERPA_CLI"),
        segmentation_model: proof("MON_SHERPA_SEGMENTATION"),
        embedding_model: proof("MON_SHERPA_EMBEDDING"),
        runtime_libraries: std::env::var("MON_SHERPA_RUNTIME_LIBRARY")
            .ok()
            .map(|path| Fingerprint::capture(Path::new(&path), &|| false).unwrap())
            .into_iter()
            .collect(),
        threads: 2,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    };
    let input = proof("MON_SHERPA_INPUT_CAF");
    let verified =
        VerifiedAudio::open(File::open(&input.path).unwrap(), input.size, &input.sha256).unwrap();
    let rate = verified.format.sample_rate;
    let frames = verified.format.frames;
    let duration = (frames * 1_000_000 / u64::from(rate)) as i64;
    let mut audio = PreparedAudio {
        clock: None,
        sample_rate: rate,
        channels: verified.format.channels,
        start_us: 0,
        end_us: duration,
        track_kind: "imported".into(),
        alignment: "unverified".into(),
        spans: vec![AudioSpan {
            audio: verified,
            first_frame: 0,
            frames,
        }],
    };
    for num_speakers in [None, Some(3)] {
        let directory = tempfile::tempdir().unwrap();
        crate::transcription::wave::write_track(
            &mut audio,
            &directory.path().join("input.wav"),
            &|| false,
        )
        .unwrap();
        let started = Instant::now();
        let result = run_track(
            &config,
            num_speakers,
            directory.path(),
            duration,
            Duration::from_secs(300),
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert!(!result.is_empty(), "use a speech fixture, not silence");
        let timeline = crate::diarization::timeline::Timeline::build(&result, duration).unwrap();
        assert!(!timeline.intervals.is_empty());
        eprintln!("real sherpa native integration: {num_speakers:?}, {} segments, {} ms; CLI SHA {}, segmentation SHA {}, embedding SHA {}, runtime SHAs {:?}", result.len(), started.elapsed().as_millis(), config.executable.sha256, config.segmentation_model.sha256, config.embedding_model.sha256, config.runtime_libraries.iter().map(|p| &p.sha256).collect::<Vec<_>>());
    }
}
