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
            Fingerprint::capture(&path, &crate::test_support::never_cancelled).unwrap()
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

#[test]
fn shared_capture_builder_preserves_settings_fingerprints_and_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let expected = config(directory.path());
    let library = directory.path().join("native-library");
    std::fs::write(&library, b"library fixture").unwrap();
    let settings = SherpaSettings {
        threads: 3,
        cluster_threshold_milli: 700,
        window_shift_milli: 250,
        min_duration_on_ms: 125,
        min_duration_off_ms: 800,
    };
    let paths = [
        expected.executable.path.as_path(),
        expected.segmentation_model.path.as_path(),
        expected.embedding_model.path.as_path(),
    ];
    let first = SherpaConfig::capture(
        paths,
        &[library.as_path()],
        settings,
        CaptureMode::Transient,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let second = SherpaConfig::capture(
        paths,
        &[library.as_path()],
        settings,
        CaptureMode::Retained,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        (
            first.threads,
            first.cluster_threshold_milli,
            first.window_shift_milli,
            first.min_duration_on_ms,
            first.min_duration_off_ms,
        ),
        (3, 700, 250, 125, 800)
    );
    assert_eq!(first.executable, expected.executable);
    assert_eq!(first.segmentation_model, expected.segmentation_model);
    assert_eq!(first.embedding_model, expected.embedding_model);
    assert_eq!(
        first.runtime_libraries[0],
        Fingerprint::capture(&library, &crate::test_support::never_cancelled).unwrap()
    );
    assert!(SherpaConfig::capture(paths, &[], settings, CaptureMode::Transient, &|| true).is_err());
    assert!(SherpaConfig::capture(
        paths,
        &[library.as_path(); 17],
        settings,
        CaptureMode::Transient,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

fn output(config: &SherpaConfig, count: Option<u32>, rows: &str) -> Vec<u8> {
    format!("{}\nStarted\n{rows}", config.expected_header(count)).into_bytes()
}

#[test]
fn standalone_cli_rejects_partial_input_before_launching_or_creating_logs() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    crate::test_support::write_wave(&root.path().join("source.wav"));
    for (index, (first, end)) in [(0, 8000), (8000, 16000)].into_iter().enumerate() {
        let unit = root.path().join(index.to_string());
        std::fs::create_dir(&unit).unwrap();
        crate::transcription::input::Input::new(
            &root.path().join("source.wav"),
            first,
            end,
            &|| false,
        )
        .unwrap()
        .write(&unit)
        .unwrap();
        let error = run_standalone_track(
            &config,
            None,
            &unit,
            500_000,
            Duration::from_secs(1),
            &|| false,
        )
        .unwrap_err();
        assert!(error.contains("音声ファイル全体"), "{error}");
        assert!(!unit.join("stdout.log").exists());
        assert!(!unit.join("stderr.log").exists());
    }
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
    assert!(logical
        .verify(&crate::test_support::never_cancelled)
        .is_err());
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
    crate::transcription::wave::write_track(
        &mut source,
        &directory.join("input.wav"),
        &crate::test_support::never_cancelled,
    )
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
fn terminal_receptive_field_offset_is_clamped_to_recorded_audio() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = config(directory.path());
    // Regression: 592,718 samples at 16 kHz. The native result's final
    // receptive-field center is 27.125 ms beyond the actual last sample.
    let duration = 37_044_875;
    let bytes = output(
        &runtime,
        None,
        "0.031 -- 19.336 speaker_00\n20.062 -- 37.072 speaker_00\n",
    );
    let expected = vec![
        Segment {
            start_us: 31_000,
            end_us: 19_336_000,
            speaker: 0,
        },
        Segment {
            start_us: 20_062_000,
            end_us: duration,
            speaker: 0,
        },
    ];
    assert_eq!(
        parse_output(&bytes, &runtime, None, duration).unwrap(),
        expected
    );
    validate_saved_segments(&expected, None, duration).unwrap();
    let mut logical = runtime.clone();
    for proof in [
        &mut logical.executable,
        &mut logical.segmentation_model,
        &mut logical.embedding_model,
    ] {
        proof.path = PathBuf::from(format!("mon-unavailable:{}", proof.sha256));
    }
    let normalized = logical_output(&bytes, &runtime, &logical, None, duration).unwrap();
    assert_eq!(
        parse_output(&normalized, &logical, None, duration).unwrap(),
        expected
    );
}

#[test]
fn terminal_tolerance_is_bounded_and_does_not_relax_saved_timestamps() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    // Includes the receptive-field offset, float32 error near the WAV limit,
    // and millisecond printing. Gross overruns must still fail.
    for duration in [1_000_000, 134_000_000_000] {
        let rows = format!("0.000 -- {}.{:03} speaker_00\n", duration / 1_000_000, 50);
        let segments =
            parse_output(&output(&config, None, &rows), &config, None, duration).unwrap();
        assert_eq!(segments[0].end_us, duration);
        let rows = format!("0.000 -- {}.{:03} speaker_00\n", duration / 1_000_000, 51);
        assert!(parse_output(&output(&config, None, &rows), &config, None, duration).is_err());
        assert!(validate_saved_segments(
            &[Segment {
                start_us: 0,
                end_us: duration + 1,
                speaker: 0
            }],
            None,
            duration
        )
        .is_err());
    }
}

#[test]
fn rejects_wrong_protocol_identity_settings_order_times_labels_and_extra_columns() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    for rows in [
        "-0.001 -- 0.200 speaker_00\n",
        "NaN -- 0.200 speaker_00\n",
        "0.0 -- 0.200 speaker_00\n",
        "0.000 -- 1.051 speaker_00\n",
        "0.000 -- 0.000 speaker_00\n",
        "1.000 -- 1.001 speaker_00\n",
        "999999999999999999999.000 -- 1.000 speaker_00\n",
        "0.000 -- 0.100 speaker_-1\n",
        "0.000 -- 0.100 speaker_100000\n",
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
    .is_ok());
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

#[test]
fn sparse_native_labels_are_not_mistaken_for_speaker_counts() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let parsed = parse_output(
        &output(
            &config,
            None,
            "0.000 -- 0.500 speaker_896\n0.100 -- 0.400 speaker_101\n0.500 -- 0.900 speaker_896\n",
        ),
        &config,
        None,
        1_000_000,
    )
    .unwrap();
    assert_eq!(
        parsed.iter().map(|s| s.speaker).collect::<Vec<_>>(),
        vec![1, 0, 1]
    );
    assert_eq!((parsed[1].start_us, parsed[1].end_us), (100_000, 400_000));
    assert!(parse_output(
        &output(&config, Some(5), "0.000 -- 0.100 speaker_896\n"),
        &config,
        Some(5),
        1_000_000
    )
    .is_ok());
}

#[test]
fn excessive_clusters_report_count_without_inventing_a_speaker_number() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let rows = (0..101)
        .map(|i| format!("0.000 -- 0.100 speaker_{i:02}\n"))
        .collect::<String>();
    let error = parse_output(&output(&config, None, &rows), &config, None, 1_000_000).unwrap_err();
    assert!(error.contains("101個"));
    assert!(error.contains("実人数ではなく"));
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
    config.executable = Fingerprint::capture(
        &config.executable.path,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
        &crate::test_support::never_cancelled,
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
    assert!(args.contains(&"--min-duration-off=0.5"));
    let canonical_wav = unit.join("input.wav").canonicalize().unwrap();
    let input_index = args
        .iter()
        .position(|arg| *arg == canonical_wav.to_str().unwrap())
        .unwrap();
    assert_eq!(
        &args[input_index + 1..],
        &["--mon-first-frame", "0", "--mon-end-frame", "16000"]
    );
    assert!(run_track(
        &config,
        Some(3),
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &crate::test_support::never_cancelled
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
        &crate::test_support::never_cancelled
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
        &crate::test_support::never_cancelled
    )
    .unwrap_err()
    .contains("WAV"));
    assert!(!unit.join("stdout.log").exists());
    let library = directory.path().join("runtime.dylib");
    std::fs::write(&library, b"fixture").unwrap();
    config
        .runtime_libraries
        .push(Fingerprint::capture(&library, &crate::test_support::never_cancelled).unwrap());
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
            &crate::test_support::never_cancelled
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
        // Check exit/log classification even when the full suite contends for
        // I/O. The separate timeout test exercises its short deadline directly.
        let error = run_track(
            &config,
            None,
            &unit,
            1_000_000,
            Duration::from_secs(15),
            &crate::test_support::never_cancelled,
        )
        .unwrap_err();
        assert!(
            error.contains(message),
            "expected {message}, got {error}; fixture: {body}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn final_log_growth_after_last_poll_is_rejected_before_accepting_exit() {
    use std::cell::Cell;
    for (limit, redirect) in [(MAX_OUTPUT_BYTES, ""), (MAX_LOG_BYTES, ">&2")] {
        let directory = tempfile::tempdir().unwrap();
        let mut config = config(directory.path());
        let bytes = output(&config, None, "0.000 -- 0.500 speaker_00\n");
        executable(
            &mut config,
            &format!(
                "printf '%s' \"$PPID\" > guardian.pid\nprintf '%s' {}\nprintf 'progress 25.00%%\\n' >&2\nwhile [ ! -e release ]; do /bin/sleep 0.01; done\nexec /usr/bin/head -c {} /dev/zero {redirect}",
                shell_quote(std::str::from_utf8(&bytes).unwrap()),
                limit + 1,
            ),
        );
        let unit = directory.path().join("unit");
        std::fs::create_dir(&unit).unwrap();
        wav(&unit);
        let released = Cell::new(false);
        let result = run_track_observed(
            &config,
            None,
            &unit,
            1_000_000,
            Duration::from_secs(15),
            &crate::test_support::never_cancelled,
            &|value| {
                if value != 2500 || released.replace(true) {
                    return;
                }
                // This callback runs after the monitor's size check. Let the
                // engine grow its log, then wait without reaping its guardian
                // so the very next try_wait must observe a completed child.
                let guardian: u32 = std::fs::read_to_string(unit.join("guardian.pid"))
                    .unwrap()
                    .parse()
                    .unwrap();
                std::fs::write(unit.join("release"), b"release owned fixture").unwrap();
                let started = Instant::now();
                loop {
                    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                    let status = unsafe {
                        libc::waitid(
                            libc::P_PID,
                            guardian,
                            &mut info,
                            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                        )
                    };
                    if status != 0 {
                        let error = std::io::Error::last_os_error();
                        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted, "{error}");
                    }
                    if unsafe { info.si_pid() } == guardian as i32 {
                        break;
                    }
                    assert!(started.elapsed() < Duration::from_secs(10));
                    std::thread::sleep(Duration::from_millis(1));
                }
            },
        );
        assert!(released.get(), "the engine never reached its final write");
        assert!(
            result.as_ref().is_err_and(|error| error.contains("上限")),
            "final {redirect:?} log overflow was not rejected: {result:?}"
        );
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
    let proof = |name| {
        Fingerprint::capture(
            Path::new(&std::env::var(name).unwrap()),
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    };
    let config = SherpaConfig {
        executable: proof("MON_SHERPA_CLI"),
        segmentation_model: proof("MON_SHERPA_SEGMENTATION"),
        embedding_model: proof("MON_SHERPA_EMBEDDING"),
        runtime_libraries: std::env::var("MON_SHERPA_RUNTIME_LIBRARY")
            .ok()
            .map(|path| {
                Fingerprint::capture(Path::new(&path), &crate::test_support::never_cancelled)
                    .unwrap()
            })
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
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let started = Instant::now();
        let result = run_standalone_track(
            &config,
            num_speakers,
            directory.path(),
            duration,
            Duration::from_secs(300),
            &crate::test_support::never_cancelled,
        )
        .unwrap()
        .unwrap();
        assert!(!result.is_empty(), "use a speech fixture, not silence");
        let timeline = crate::diarization::timeline::Timeline::build(&result, duration).unwrap();
        assert!(!timeline.intervals.is_empty());
        eprintln!("real sherpa native integration: {num_speakers:?}, {} segments, {} ms; CLI SHA {}, segmentation SHA {}, embedding SHA {}, runtime SHAs {:?}", result.len(), started.elapsed().as_millis(), config.executable.sha256, config.segmentation_model.sha256, config.embedding_model.sha256, config.runtime_libraries.iter().map(|p| &p.sha256).collect::<Vec<_>>());
    }
}

#[cfg(unix)]
#[test]
fn native_progress_streams_before_exit_without_completing_or_checkpointing_inference() {
    use std::cell::RefCell;
    let directory = tempfile::tempdir().unwrap();
    let mut config = config(directory.path());
    let bytes = output(&config, None, "0.000 -- 0.500 speaker_00\n");
    executable(&mut config, &format!(
        "printf 'progress 25.00%%\\n' >&2\n/bin/sleep 0.4\nprintf 'progress 12.50%%\\rprogress 75.' >&2\n/bin/sleep 0.2\nprintf '00%%\\r\\nprogress 100.00%%\\n' >&2\nprintf '%s' {}\ntouch finished",
        shell_quote(std::str::from_utf8(&bytes).unwrap())
    ));
    let unit = directory.path().join("unit");
    std::fs::create_dir(&unit).unwrap();
    wav(&unit);
    let seen = RefCell::new(Vec::new());
    let result = run_track_observed(
        &config,
        None,
        &unit,
        1_000_000,
        Duration::from_secs(3),
        &|| false,
        &|value| {
            if value == 2500 {
                assert!(!unit.join("finished").exists());
            }
            seen.borrow_mut().push(value);
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.len(), 1);
    let values = seen.borrow();
    assert_eq!(values.first(), Some(&0));
    assert!(values.contains(&2500));
    assert_eq!(values.last(), Some(&10000));
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(std::fs::read_to_string(unit.join("stderr.log"))
        .unwrap()
        .contains("progress 25.00%"));
}

#[cfg(unix)]
#[test]
fn native_progress_100_cannot_authorize_failed_output_and_can_still_be_cancelled() {
    use std::cell::Cell;
    for cancel in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut config = config(directory.path());
        executable(
            &mut config,
            "printf 'progress 100.00%%\\n' >&2\n/bin/sleep 0.3\nexit 42",
        );
        let unit = directory.path().join("unit");
        std::fs::create_dir(&unit).unwrap();
        wav(&unit);
        let stop = Cell::new(false);
        let seen = Cell::new(false);
        let result = run_track_observed(
            &config,
            None,
            &unit,
            1_000_000,
            Duration::from_secs(3),
            &|| stop.get(),
            &|value| {
                if value == 10000 {
                    seen.set(true);
                    stop.set(cancel);
                }
            },
        );
        assert!(seen.get());
        if cancel {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
    }
}
