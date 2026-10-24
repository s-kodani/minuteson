use super::*;
use std::cell::Cell;

fn fixture(seconds: u64) -> (tempfile::TempDir, whisper::WhisperConfig) {
    let root = tempfile::tempdir().unwrap();
    let frames = seconds * RATE;
    let mut bytes = wave::wav_header(frames as u32).to_vec();
    bytes.resize(44 + frames as usize * 2, 0);
    fs::write(root.path().join("input.wav"), bytes).unwrap();
    let proof = whisper::Fingerprint {
        path: root.path().join("model"),
        size: 1,
        sha256: "a".repeat(64),
    };
    let config = whisper::WhisperConfig {
        executable: proof.clone(),
        model: proof,
        vad_model: None,
        vad_options: Some(super::super::vad::VadOptions::default()),
        threads: 1,
        use_gpu: false,
        prompt: "固定ヒント".into(),
    };
    (root, config)
}
fn result(
    config: &whisper::WhisperConfig,
    dir: &Path,
    duration: i64,
    bad: bool,
) -> Result<Option<Vec<whisper::Segment>>, String> {
    assert!(config.prompt.is_empty());
    let text = if bad {
        "繰り返しを検出するための文章。".repeat(4)
    } else {
        "確認した通常の発言です。".into()
    };
    let value = json!({"model":{"multilingual":true},"params":{"model":config.model.path,"language":"ja","translate":false},"transcription":[{"offsets":{"from":0,"to":duration/1000},"text":text}]});
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(dir.join("result.json"), &bytes).unwrap();
    whisper::parse_output(&bytes, &config.model.path, duration).map(Some)
}

#[test]
fn adaptive_recovery_finishes_all_repetitive_audio_with_a_finite_budget_and_no_invented_text() {
    let (root, config) = fixture(120);
    let mut calls = 0;
    let mut snapshots = vec![];
    let path = run_with(
        &config,
        &config.model.path,
        root.path(),
        0,
        120 * RATE,
        None,
        &|| false,
        &mut |s| {
            snapshots.push(s.clone());
            Ok(())
        },
        &mut |c, p, d, _| {
            calls += 1;
            if d <= 10_000_000 {
                assert!(!c.vad_options.unwrap().enabled);
            }
            result(c, p, d, true)
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls, 30);
    assert_eq!(snapshots.len(), 30);
    let (segments, state) = checked(
        &fs::read(&path).unwrap(),
        &config.model.path,
        120_000_000,
        0,
        120 * RATE,
    )
    .unwrap();
    assert!(segments.is_empty());
    assert_eq!(state.review().len(), 16);
    let mut ranges = state.review();
    ranges.sort();
    assert_eq!(ranges.first().unwrap().0, 0);
    assert_eq!(ranges.last().unwrap().1, 120_000_000);
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].1, pair[1].0);
    }
}
#[test]
fn recovered_leaves_resume_without_repeating_accepted_work_and_reject_altered_results() {
    let (root, config) = fixture(120);
    let paused = Cell::new(false);
    let mut saved = None;
    let mut calls = 0;
    assert!(run_with(
        &config,
        &config.model.path,
        root.path(),
        0,
        120 * RATE,
        None,
        &|| paused.get(),
        &mut |s| {
            saved = Some(s.clone());
            paused.set(true);
            Ok(())
        },
        &mut |c, p, d, _| {
            calls += 1;
            result(c, p, d, false)
        }
    )
    .unwrap()
    .is_none());
    assert_eq!(calls, 1);
    assert_eq!(saved.as_ref().unwrap().leaves.len(), 1);
    let path = run_with(
        &config,
        &config.model.path,
        root.path(),
        0,
        120 * RATE,
        saved,
        &|| false,
        &mut |_| Ok(()),
        &mut |c, p, d, _| {
            calls += 1;
            result(c, p, d, false)
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls, 2);
    let bytes = fs::read(&path).unwrap();
    let (segments, state) =
        checked(&bytes, &config.model.path, 120_000_000, 0, 120 * RATE).unwrap();
    assert_eq!(segments.len(), 2);
    assert!(state.review().is_empty());
    assert!(segments[0].end_us <= segments[1].start_us);
    let mut changed: Value = serde_json::from_slice(&bytes).unwrap();
    changed["transcription"][0]["text"] = json!("偽の本文");
    assert!(checked(
        &serde_json::to_vec(&changed).unwrap(),
        &config.model.path,
        120_000_000,
        0,
        120 * RATE
    )
    .is_err());
    let mut changed = state.clone();
    changed.leaves[0].range.first += 1;
    assert!(changed
        .check(0, 120 * RATE, 120 * RATE, &config.model.path)
        .is_err());
    let mut moved: Value = serde_json::from_slice(&bytes).unwrap();
    let new_model = root.path().join("new-model");
    rebind_json(
        &mut moved,
        &[config.model.path.to_string_lossy().into_owned()],
        &new_model,
    )
    .unwrap();
    moved["params"]["model"] = json!(new_model);
    assert!(checked(
        &serde_json::to_vec(&moved).unwrap(),
        &new_model,
        120_000_000,
        0,
        120 * RATE
    )
    .is_ok());
}
#[test]
fn recovery_never_turns_engine_errors_into_review_markers() {
    let (root, config) = fixture(4);
    let error = run_with(
        &config,
        &config.model.path,
        root.path(),
        0,
        4 * RATE,
        None,
        &|| false,
        &mut |_| panic!("failed engine was checkpointed"),
        &mut |_, _, _, _| Err("engine process failed".into()),
    )
    .unwrap_err();
    assert_eq!(error, "engine process failed");
}

#[test]
#[ignore = "requires an explicitly selected private meeting WAV and local model; writes only to MON_RECOVERY_OUTPUT"]
fn private_meeting_transcription_reaches_the_last_unit() {
    use crate::local_engine::Fingerprint;
    let input = PathBuf::from(std::env::var_os("MON_RECOVERY_WAV").unwrap())
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(std::env::var_os("MON_RECOVERY_OUTPUT").unwrap());
    fs::create_dir_all(&output).unwrap();
    let output = output.canonicalize().unwrap();
    let model = PathBuf::from(std::env::var_os("MON_RECOVERY_MODEL").unwrap())
        .canonicalize()
        .unwrap();
    let vad = PathBuf::from(std::env::var_os("MON_RECOVERY_VAD").unwrap())
        .canonicalize()
        .unwrap();
    let verification = crate::local_engine::verification::Scope::join_or_enter(true).unwrap();
    let base_runtime = crate::local_engine::resident::prepare(&output, "whisper")
        .unwrap()
        .unwrap();
    let snapshot = if base_runtime.gpu.is_some() && !crate::gpu::adapters().is_empty() {
        base_runtime.accelerated().unwrap()
    } else {
        base_runtime
    };
    let _resident_scope =
        crate::local_engine::resident::Scope::enter(&output, Some(&snapshot)).unwrap();
    let config = whisper::WhisperConfig {
        executable: snapshot.resolve(&output).unwrap(),
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        vad_model: Some(Fingerprint::capture(&vad, &|| false).unwrap()),
        vad_options: Some(super::super::vad::VadOptions::default()),
        threads: 8,
        use_gpu: true,
        prompt: String::new(),
    };
    let wav = fs::read(&input).unwrap();
    let frames = (wav.len() as u64 - 44) / 2;
    assert_eq!(wav[..44], wave::wav_header(frames as u32));
    let mut accepted = vec![];
    let mut review = vec![];
    let mut completed = 0;
    let units = frames.div_ceil(120 * RATE);
    for index in 0..units {
        let first = index * 120 * RATE;
        let end = (first + 120 * RATE).min(frames);
        let window_first = first.saturating_sub(2 * RATE);
        let window_end = (end + 2 * RATE).min(frames);
        let directory = output.join(format!("unit-{index}"));
        fs::create_dir(&directory).unwrap();
        let mut bytes = wave::wav_header((window_end - window_first) as u32).to_vec();
        bytes.extend_from_slice(&wav[44 + window_first as usize * 2..44 + window_end as usize * 2]);
        fs::write(directory.join("input.wav"), bytes).unwrap();
        let duration = micros(window_end - window_first);
        let segments = match guard::run(&config, &directory, duration, true, true, None, &|| false)
        {
            Ok(Some(outcome)) => {
                whisper::parse_output(&fs::read(outcome.selected).unwrap(), &model, duration)
                    .unwrap()
            }
            Err(error) if error.starts_with(super::super::QUALITY_FAILURE_PREFIX) => {
                let path = run(
                    &config,
                    &model,
                    &directory,
                    first - window_first,
                    end - window_first,
                    None,
                    &|| false,
                    &mut |state| {
                        fs::write(
                            directory.join("progress.json"),
                            serde_json::to_vec(state).unwrap(),
                        )
                        .map_err(|e| e.to_string())
                    },
                )
                .unwrap()
                .unwrap();
                let (segments, state) = checked(
                    &fs::read(path).unwrap(),
                    &model,
                    duration,
                    first - window_first,
                    end - window_first,
                )
                .unwrap();
                review.extend(state.review().into_iter().map(|(a,b)|json!({"startUs":micros(window_first)+a,"endUs":micros(window_first)+b,"reason":"repetition_after_recovery"})));
                segments
            }
            Err(error) => panic!("unit {index}: {error}"),
            Ok(None) => panic!("unit {index}: unexpectedly cancelled"),
        };
        for mut segment in segments {
            let midpoint = (segment.start_us + segment.end_us) / 2 + micros(window_first);
            if midpoint < micros(first) || midpoint >= micros(end) {
                continue;
            }
            segment.start_us += micros(window_first);
            segment.end_us += micros(window_first);
            accepted.push(segment);
        }
        completed += 1;
        fs::write(output.join("transcription.json"),serde_json::to_vec_pretty(&json!({"source":input,"completedUnits":completed,"totalUnits":units,"utterances":accepted,"reviewRanges":review})).unwrap()).unwrap();
        eprintln!(
            "private meeting: completed {completed}/{units}; accepted={}, review={}",
            accepted.len(),
            review.len()
        );
    }
    assert_eq!(completed, units);
    assert!(!accepted.is_empty());
    verification.finish(&|| false).unwrap();
}

#[test]
#[ignore = "writes ONLY an explicitly authorized MON_RECOVERY_MEETING_ID in MON_RECOVERY_LIVE_ROOT; app must be closed"]
fn private_authorized_meeting_regeneration_commits_the_completed_transcript() {
    let root = PathBuf::from(
        std::env::var_os("MON_RECOVERY_LIVE_ROOT").expect("explicit live root required"),
    )
    .canonicalize()
    .unwrap();
    let meeting = std::env::var("MON_RECOVERY_MEETING_ID").expect("explicit meeting required");
    let report =
        PathBuf::from(std::env::var_os("MON_RECOVERY_REPORT").expect("workspace report required"));
    let store =
        crate::storage::Storage::open_with_models(root.clone(), root.join("models")).unwrap();
    assert!(
        store.load_current_analysis(&meeting).unwrap().is_none(),
        "existing published work requires the normal edit barrier"
    );
    let active = store
        .notification_jobs(&[])
        .unwrap()
        .iter()
        .filter(|n| matches!(n.job.state.as_str(), "queued" | "running" | "pausing"))
        .count();
    assert_eq!(
        active, 0,
        "another job is waiting; do not claim unrelated work"
    );
    let _owner = store.begin_meeting_work(&meeting, "job", None).unwrap();
    let request = crate::storage::analysis_flows::regeneration::Request {
        meeting_id: meeting.clone(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected: store.regeneration_admission(&meeting).unwrap(),
        num_speakers: None,
    };
    store.regenerate_analysis(&request, &|| false).unwrap();
    for kind in ["transcription", "diarization"] {
        let flow = crate::analysis_flow::advance_selected_for_test(&store, &meeting).unwrap();
        let expected = if kind == "transcription" {
            flow.transcription_job.unwrap()
        } else {
            flow.diarization_job.unwrap()
        };
        let job = store.acquire_local_inference_job().unwrap().unwrap();
        assert_eq!(job.id, expected.id);
        assert_eq!(job.meeting_id, meeting);
        assert_eq!(job.kind, kind);
        let result = if kind == "transcription" {
            super::super::process(&store, &job.id, &|| false, &|j| {
                eprintln!(
                    "authorized meeting ASR: {} {}/{:?}",
                    j.state, j.completed_units, j.total_units
                )
            })
        } else {
            crate::diarization::process(&store, &job.id, &|| false, &|j| {
                eprintln!("authorized meeting speaker processing: {}", j.state)
            })
        };
        if let Err(error) = result {
            let current = store.get_job(&job.id).unwrap();
            store
                .finish_job(
                    &job.id,
                    current.completed_units,
                    "failed",
                    Some("private_verification_failed"),
                )
                .unwrap();
            let _ = crate::analysis_flow::advance_selected_for_test(&store, &meeting);
            panic!("{kind}: {error}");
        }
        assert_eq!(store.get_job(&job.id).unwrap().state, "succeeded");
    }
    let flow = crate::analysis_flow::advance_selected_for_test(&store, &meeting).unwrap();
    assert_eq!(flow.stage, "review");
    let analysis = store.load_current_analysis(&meeting).unwrap().unwrap();
    assert!(!analysis.utterances.is_empty());
    fs::write(report,serde_json::to_vec_pretty(&json!({"meetingId":meeting,"analysisId":analysis.id,"utteranceCount":analysis.utterances.len(),"reviewRanges":analysis.review_ranges,"completed":true})).unwrap()).unwrap();
    eprintln!(
        "authorized meeting COMPLETE: {} utterances; {} review ranges",
        analysis.utterances.len(),
        analysis.review_ranges.len()
    );
}
