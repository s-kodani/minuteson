use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn worker_fixture(
    mode: &str,
    count: Option<u32>,
) -> (tempfile::TempDir, Storage, String, JobSnapshot) {
    worker_fixture_rows(mode, count, "0.000 -- 5.000 speaker_00\n")
}

fn worker_fixture_rows(
    mode: &str,
    count: Option<u32>,
    rows: &str,
) -> (tempfile::TempDir, Storage, String, JobSnapshot) {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = crate::storage::split_audio_fixture(root.path());
    let (source, known) = storage.recorded_analysis_input(&meeting).unwrap();
    let current = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: source.manifest_hash.clone(),
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "prior".into(),
                    label: "元の話者".into(),
                }],
                segments: vec![],
                utterances: [
                    (500_000, 1_000_000),
                    (2_000_000, 3_000_000),
                    (4_000_000, 16_000_000),
                    (17_000_000, 18_000_000),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (start_us, end_us))| UtteranceInput {
                    track_id: "track".into(),
                    start_us,
                    end_us,
                    text: format!("元の本文{i}"),
                    speaker_keys: vec![],
                    confidence: Some(0.8),
                })
                .collect(),
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(&meeting, &current, 0, false)
        .unwrap();
    let original = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &current).unwrap();
    let mut payload = serde_json::to_value(&draft.payload).unwrap();
    payload["speaker_names"] =
        json!([{"speaker_id":original.speakers[0].id,"display_name":"確定した氏名"}]);
    payload["utterances"] = json!([{"utterance_id":original.utterances[0].id,"effective_text":"修正した本文","speaker_ids":[original.speakers[0].id]}]);
    let draft = storage
        .save_transcript_draft(
            &draft.id,
            draft.lock_version,
            serde_json::from_value(payload).unwrap(),
        )
        .unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: "known-worker".into(),
        expected_analysis_id: current,
        expected_content_hash: original.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(crate::storage::DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let library = root.path().join("libsherpa-onnx-c-api.dylib");
    std::fs::copy(
        std::env::var("MON_EMBEDDING_FAKE_LIBRARY")
            .expect("dedicated check must provide fake C API"),
        &library,
    )
    .unwrap();
    let segmentation = root.path().join("segmentation.onnx");
    let embedding = root.path().join("embedding.onnx");
    let cli = root.path().join("cli");
    std::fs::write(&segmentation, b"fixture").unwrap();
    std::fs::write(&embedding, mode).unwrap();
    std::fs::write(&cli, b"fixture").unwrap();
    let mut engine = SherpaConfig {
        executable: Fingerprint::capture(&cli, &crate::test_support::never_cancelled).unwrap(),
        segmentation_model: Fingerprint::capture(
            &segmentation,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        embedding_model: Fingerprint::capture(&embedding, &crate::test_support::never_cancelled)
            .unwrap(),
        runtime_libraries: vec![Fingerprint::capture(
            &library,
            &crate::test_support::never_cancelled,
        )
        .unwrap()],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 500,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
        speech_sensitivity_milli: 0,
    };
    let header = r#"for arg do
case "$arg" in
--segmentation.pyannote-model=*) seg=${arg#*=};;
--embedding.model=*) embed=${arg#*=};;
--segmentation.num-threads=*) threads=${arg#*=};;
--clustering.num-clusters=*) clusters=${arg#*=};;
--clustering.cluster-threshold=*) threshold=${arg#*=};;
--segmentation.pyannote-window-shift-ratio=*) shift_ratio=${arg#*=};;
--min-duration-on=*) duration_on=${arg#*=};;
--min-duration-off=*) duration_off=${arg#*=};;
esac
done
test "$clusters" != 3 || exit 81
printf 'OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model="%s", window_shift_ratio=%s), num_threads=%s, debug=False, provider="cpu"), embedding=SpeakerEmbeddingExtractorConfig(model="%s", num_threads=%s, debug=False, provider="cpu"), clustering=FastClusteringConfig(num_clusters=%s, threshold=%s, compute_confidence=False), min_duration_on=%s, min_duration_off=%s)\nStarted\n' "$seg" "$shift_ratio" "$threads" "$embed" "$threads" "$clusters" "$threshold" "$duration_on" "$duration_off"
"#;
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nprintf x >> {}\n{header}\nprintf '%s' {}\n",
            quote(root.path().join("calls").to_str().unwrap()),
            quote(rows)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    engine.executable = Fingerprint::capture(&cli, &crate::test_support::never_cancelled).unwrap();
    let helper = PathBuf::from(
        std::env::var("MON_EMBEDDING_HELPER").expect("dedicated check must provide signed helper"),
    );
    let job = enqueue(
        &storage,
        request,
        engine,
        "standard",
        vec![TrackSettings {
            track_id: "track".into(),
            num_speakers: count,
        }],
        None,
        source,
        known.unwrap(),
        Some(&helper),
    )
    .unwrap();
    (root, storage, meeting, job)
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_preserves_pcm_times_edits_and_matches_features_not_local_labels() {
    for (mode, count, estimated, unmatched) in [
        ("voice_fixture", Some(3), 2, 0),
        ("same_voice_fixture", None, 1, 0),
        ("same_voice_fixture", Some(3), 2, 0),
        ("notready", Some(3), 0, 2),
    ] {
        let (root, storage, meeting, job) = worker_fixture(mode, count);
        let source_path = root
            .path()
            .join("meetings")
            .join(&meeting)
            .join("audio/mic.caf");
        let source = std::fs::read(&source_path).unwrap();
        let original = storage.load_current_analysis(&meeting).unwrap().unwrap();
        let draft = storage.load_transcript_draft(&meeting).unwrap();
        assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, job.id);
        let id = process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        let candidate = storage
            .load_visible_analysis(&meeting, &id)
            .unwrap()
            .unwrap();
        assert_eq!(
            candidate
                .utterances
                .iter()
                .map(|u| (u.start_us, u.end_us))
                .collect::<Vec<_>>(),
            original
                .utterances
                .iter()
                .map(|u| (u.start_us, u.end_us))
                .collect::<Vec<_>>()
        );
        assert_eq!(candidate.utterances[0].recognized_text, "元の本文0");
        assert_eq!(candidate.utterances[0].effective_text, "修正した本文");
        assert_eq!(
            candidate
                .speakers
                .iter()
                .find(|s| s.display_name.as_deref() == Some("確定した氏名"))
                .unwrap()
                .id,
            candidate.utterances[0].speaker_ids[0]
        );
        assert!(candidate.utterances[2].speaker_ids.is_empty());
        assert_eq!(
            candidate.utterances[1].speaker_ids == candidate.utterances[3].speaker_ids,
            estimated == 1
        );
        let config: String = crate::storage::with_test_database(&storage, |connection| {
            connection
                .query_row(
                    "SELECT config_json FROM analysis_revisions WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap()
        });
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["estimatedSpeakers"], json!([estimated]));
        assert_eq!(config["unmatchedRunGroups"], json!([unmatched]));
        assert_eq!(config["audioCoverage"]["runCount"], 2);
        let times: Vec<(i64, i64)> = crate::storage::with_test_database(&storage, |connection| {
            connection.prepare("SELECT start_us,end_us FROM speaker_segments WHERE analysis_revision_id=?1 ORDER BY start_us").unwrap().query_map([&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect()
        });
        assert_eq!(times, vec![(0, 5_000_000), (15_000_000, 20_000_000)]);
        let checkpoint = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
        for (index, value) in [8192_i16, -16384].into_iter().enumerate() {
            let asset = checkpoint["outputs"][index]["wavAssetId"].as_str().unwrap();
            let relative: String = crate::storage::with_test_database(&storage, |connection| {
                connection
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE id=?1",
                        [asset],
                        |r| r.get(0),
                    )
                    .unwrap()
            });
            let bytes = std::fs::read(root.path().join(relative)).unwrap();
            assert_eq!(bytes.len(), 44 + 80000 * 2);
            assert!(bytes[44..]
                .chunks_exact(2)
                .all(|b| i16::from_le_bytes(b.try_into().unwrap()) == value));
        }
        assert_eq!(
            storage.load_current_analysis(&meeting).unwrap().unwrap(),
            original
        );
        assert_eq!(storage.load_transcript_draft(&meeting).unwrap(), draft);
        assert_eq!(std::fs::read(source_path).unwrap(), source);
        assert_eq!(std::fs::read(root.path().join("calls")).unwrap(), b"xx");
    }
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_pause_retry_reopen_reuses_exact_completed_features() {
    let (root, storage, meeting, job) = worker_fixture("voice_fixture", None);
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|snapshot| {
            if snapshot.completed_units == 1 && snapshot.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        }
    )
    .unwrap()
    .is_none());
    let saved = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    let first_features = std::fs::read_to_string(root.path().join("embedding.log")).unwrap();
    assert_eq!(
        first_features
            .lines()
            .filter(|line| *line == "create")
            .count(),
        1
    );
    let failed = storage
        .finish_job(&job.id, 1, "failed", Some("fixture"))
        .unwrap();
    let retry = super::super::RetryRequest {
        meeting_id: meeting.clone(),
        job_id: job.id.clone(),
        operation_id: "retry-known".into(),
        expected_spec_hash: failed.spec_hash,
    }
    .enqueue(&storage)
    .unwrap();
    assert_eq!(retry.completed_units, 1);
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, retry.id);
    let id = process(
        &storage,
        &retry.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(storage.fixed_job_input(&job.id).unwrap(), saved);
    assert_eq!(
        storage.fixed_job_input(&retry.id).unwrap().1.unwrap()["outputs"][0],
        saved.1.unwrap()["outputs"][0]
    );
    assert_eq!(std::fs::read(root.path().join("calls")).unwrap(), b"xx");
    assert_eq!(
        std::fs::read_to_string(root.path().join("embedding.log"))
            .unwrap()
            .lines()
            .filter(|line| *line == "create")
            .count(),
        2
    );
    assert_eq!(
        storage
            .load_visible_analysis(&meeting, &id)
            .unwrap()
            .unwrap()
            .utterances[3]
            .start_us,
        17_000_000
    );
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_rejects_changed_gap_at_publication_and_changed_model_at_resume() {
    for change_gap in [true, false] {
        let (root, storage, meeting, job) = worker_fixture("voice_fixture", None);
        let original = storage.load_current_analysis(&meeting).unwrap();
        let draft = storage.load_transcript_draft(&meeting).unwrap();
        storage.acquire_next_job().unwrap().unwrap();
        let output = process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|snapshot| {
                if snapshot.state == "running"
                    && snapshot.completed_units == if change_gap { 2 } else { 1 }
                {
                    if change_gap {
                        crate::storage::with_test_database(&storage, |connection| {
                            connection
                                .execute("UPDATE audio_gaps SET reason_code='changed'", [])
                                .unwrap()
                        });
                    } else {
                        storage.request_job_pause(&job.id, "user").unwrap();
                    }
                }
            },
        );
        if change_gap {
            assert!(output.is_err());
        } else {
            assert!(output.unwrap().is_none());
            std::fs::write(root.path().join("embedding.onnx"), b"same_voice_fixture").unwrap();
            storage.resume_job(&job.id).unwrap();
            storage.acquire_next_job().unwrap().unwrap();
            assert!(process(
                &storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .is_err());
            assert_eq!(std::fs::read(root.path().join("calls")).unwrap(), b"x");
        }
        assert_eq!(storage.load_current_analysis(&meeting).unwrap(), original);
        assert_eq!(storage.load_transcript_draft(&meeting).unwrap(), draft);
        assert!(storage
            .get_job(&job.id)
            .unwrap()
            .output_analysis_id
            .is_none());
    }
}

fn fixture() -> (tempfile::TempDir, Storage, String, Parameters, JobSpec) {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = crate::storage::split_audio_fixture(root.path());
    let (source, known) = storage.recorded_analysis_input(&meeting).unwrap();
    let file = |name: &str| {
        let path = root.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        Fingerprint::capture(&path, &crate::test_support::never_cancelled).unwrap()
    };
    let helper = file("helper");
    let library = file("libsherpa-onnx-c-api.dylib");
    let engine = SherpaConfig {
        executable: file("cli"),
        segmentation_model: file("segmentation"),
        embedding_model: file("embedding"),
        runtime_libraries: vec![library.clone()],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 500,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
        speech_sensitivity_milli: 0,
    };
    let parameters = Parameters {
        resident_runtime: None,
        tracks: vec![TrackSettings {
            track_id: source.tracks[0].id.clone(),
            num_speakers: None,
        }],
        source,
        engine,
        known_input: known,
        extractor: Some(embedding::ExtractorConfig { helper, library }),
        matching: Some(Policy::fixed()),
        manual_assignment_hash: "a".repeat(64),
    };
    let request = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: "known-test".into(),
        expected_analysis_id: "parent".into(),
        expected_content_hash: "a".repeat(64),
        expected_meeting_lock_version: 0,
        expected_draft: None,
    };
    let spec = JobSpec {
        meeting_id: meeting.clone(),
        kind: JobKind::Diarization,
        operation_id: request.operation_id.clone(),
        input_analysis_id: Some("fixed-input".into()),
        input_hash: "a".repeat(64),
        profile_snapshot: json!({"profile":"standard"}),
        parameters: json!({"schemaVersion":2,"request":request,"diarization":parameters}),
        total_units: Some(2),
    };
    (root, storage, meeting, parameters, spec)
}

fn result(parameters: &Parameters, spec: &JobSpec, index: usize) -> ResultFile {
    let run = parameters.known_input.as_ref().unwrap().runs[index].clone();
    let duration = (run_frames(parameters, index).unwrap() * 1_000_000 / 16000) as i64;
    let segments = vec![Segment {
        start_us: 0,
        end_us: duration,
        speaker: 0,
    }];
    let features=support(parameters,index,&segments,&||false).unwrap().windows.iter().map(|window|SavedFeature {speaker:window.speaker.speaker,first_frame:window.first,end_frame:window.end,output:embedding::parse(&serde_json::to_vec(&json!({"schemaVersion":1,"engine":"sherpa-onnx","engineVersion":"1.13.8","sampleRate":16000,"firstFrame":window.first,"endFrame":window.end,"dimension":2,"status":"ready","reason":null,"vectorBits":["3f800000","00000000"]})).unwrap(),window.first,window.end).unwrap()}).collect();
    ResultFile {
        schema_version: 2,
        origin_operation_id: "known-test".into(),
        input_analysis_id: spec.input_analysis_id.clone().unwrap(),
        input_hash: spec.input_hash.clone(),
        source_manifest_hash: parameters.source.manifest_hash.clone(),
        track_id: parameters.source.tracks[run.track_index].id.clone(),
        track_start_us: run.start_us,
        duration_us: duration,
        engine: parameters.engine.clone(),
        num_speakers: None,
        segments,
        run: Some(run),
        extractor: parameters.extractor.clone(),
        features: Some(features),
        matching: parameters.matching.clone(),
    }
}

#[test]
fn known_diarization_conditions_and_feature_windows_are_fixed_and_not_local_requested_counts() {
    let (_root, _storage, _meeting, parameters, spec) = fixture();
    assert_eq!(validate(&parameters).unwrap().runs.len(), 2);
    for index in 0..2 {
        validate_result(
            &result(&parameters, &spec, index),
            &spec,
            &parameters,
            index,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    }
    let mut parameters = parameters;
    parameters.matching.as_mut().unwrap().windows = "invented".into();
    assert!(validate(&parameters).is_err());
    parameters.matching = Some(Policy::fixed());
    parameters.extractor = None;
    assert!(validate(&parameters).is_err());
}

#[test]
fn known_diarization_result_refuses_wrong_origin_run_window_vector_and_partial_features() {
    let (_root, _storage, _meeting, parameters, spec) = fixture();
    let valid = result(&parameters, &spec, 1);
    for field in [
        "originOperationId",
        "inputAnalysisId",
        "inputHash",
        "sourceManifestHash",
        "trackId",
        "trackStartUs",
        "durationUs",
        "numSpeakers",
        "run",
        "extractor",
        "features",
        "matching",
    ] {
        let mut bytes = serde_json::to_value(&valid).unwrap();
        bytes[field] = match field {
            "trackStartUs" | "durationUs" => json!(1),
            "numSpeakers" => json!(3),
            "run" | "extractor" | "features" | "matching" => json!(null),
            _ => json!("bad"),
        };
        let changed: ResultFile = serde_json::from_value(bytes).unwrap();
        assert!(
            validate_result(
                &changed,
                &spec,
                &parameters,
                1,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{field}"
        );
    }
    let mut changed = valid.clone();
    changed.features.as_mut().unwrap()[0].first_frame += 1;
    assert!(validate_result(
        &changed,
        &spec,
        &parameters,
        1,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let mut changed = serde_json::to_value(valid).unwrap();
    changed["features"][0]["output"]["vectorBits"][0] = json!("7fc00000");
    assert!(validate_result(
        &serde_json::from_value(changed).unwrap(),
        &spec,
        &parameters,
        1,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn known_diarization_restore_rebases_ids_but_keeps_features_times_and_extractors() {
    let (_root, _storage, _meeting, parameters, old) = fixture();
    let result = result(&parameters, &old, 1);
    let bytes = serde_json::to_vec(&result).unwrap();
    let mut new = old.clone();
    new.meeting_id = "restored-meeting".into();
    new.input_analysis_id = Some("restored-input".into());
    new.input_hash = "b".repeat(64);
    new.parameters["request"]["meetingId"] = json!(new.meeting_id);
    new.parameters["request"]["operationId"] = json!("restored-operation");
    new.parameters["diarization"]["source"]["manifestHash"] = json!("c".repeat(64));
    new.parameters["diarization"]["source"]["tracks"][0]["id"] = json!("restored-track");
    new.parameters["diarization"]["tracks"][0]["trackId"] = json!("restored-track");
    let rebuilt: ResultFile =
        serde_json::from_slice(&restore_result(&bytes, &old, &new).unwrap()).unwrap();
    assert_eq!(rebuilt.features, result.features);
    assert_eq!(rebuilt.run, result.run);
    assert_eq!(rebuilt.extractor, result.extractor);
    assert_eq!(rebuilt.track_id, "restored-track");
    assert_eq!(rebuilt.input_hash, new.input_hash);
    new.parameters["diarization"]["tracks"][0]["numSpeakers"] = json!(3);
    assert!(restore_result(&bytes, &old, &new).is_err());
}

#[test]
fn tiny_turns_keep_local_groups_and_exact_overlap_without_fabricating_feature_samples() {
    let (_root, _storage, _meeting, parameters, _spec) = fixture();
    let selected = support(
        &parameters,
        0,
        &[
            Segment {
                start_us: 0,
                end_us: 10,
                speaker: 0,
            },
            Segment {
                start_us: 1,
                end_us: 2,
                speaker: 1,
            },
        ],
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(selected.groups.len(), 2);
    assert_eq!(
        selected.overlaps,
        vec![(
            clustering::LocalSpeaker { run: 0, speaker: 0 },
            clustering::LocalSpeaker { run: 0, speaker: 1 }
        )]
    );
    assert!(selected.windows.is_empty());
    let selected = support(
        &parameters,
        0,
        &[
            Segment {
                start_us: 0,
                end_us: 5_000_000,
                speaker: 0,
            },
            Segment {
                start_us: 1,
                end_us: 2,
                speaker: 1,
            },
        ],
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(selected.windows.len(), 1);
    assert_eq!(selected.windows[0].first, 1);
    assert_eq!(selected.windows[0].end, 80_000);
    assert_eq!(selected.overlaps.len(), 1);
}

#[test]
fn restore_checks_fixed_conditions_even_before_any_saved_unit_and_rejects_changed_pcm_proofs() {
    let (_root, _storage, _meeting, _parameters, spec) = fixture();
    let before = envelope(&spec).unwrap();
    for field in ["hash", "size", "frames", "sampleRate"] {
        let mut changed = spec.clone();
        changed.parameters["diarization"]["source"]["tracks"][0]["chunks"][0][field] =
            if field == "hash" {
                json!("b".repeat(64))
            } else {
                json!(1)
            };
        assert!(
            envelope(&changed)
                .and_then(|after| validate_restore(&spec, &changed, &before, &after))
                .is_err(),
            "{field}"
        );
    }
    let mut changed = spec.clone();
    changed.parameters["diarization"]["tracks"][0]["numSpeakers"] = json!(3);
    assert!(validate_restore(&spec, &changed, &before, &envelope(&changed).unwrap()).is_err());
}

#[test]
fn aggregate_saved_feature_limits_apply_before_retaining_many_small_results() {
    let (_root, _storage, _meeting, parameters, spec) = fixture();
    let mut result = result(&parameters, &spec, 0);
    result.features.as_mut().unwrap()[0].output.dimension = 4096;
    let mut limits = SavedLimits::default();
    for _ in 0..2048 {
        limits.add(&result).unwrap();
    }
    assert!(limits.add(&result).is_err());
    result.features = Some(vec![]);
    let mut limits = SavedLimits::default();
    for _ in 0..100_000 {
        limits.add(&result).unwrap();
    }
    assert!(limits.add(&result).is_err());
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_additive_restore_preserves_features_and_resumes_without_reextracting_saved_run()
{
    for partial in [false, true] {
        let (root, storage, _meeting, job) = worker_fixture("voice_fixture", None);
        storage.acquire_next_job().unwrap().unwrap();
        process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|snapshot| {
                if partial && snapshot.completed_units == 1 && snapshot.state == "running" {
                    storage.request_job_pause(&job.id, "user").unwrap();
                }
            },
        )
        .unwrap();
        let saved = storage.fixed_job_input(&job.id).unwrap();
        let calls = std::fs::read(root.path().join("calls")).unwrap();
        let features = std::fs::read(root.path().join("embedding.log")).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let archive = destination.path().join("known.monbackup");
        storage
            .create_backup_archive(&archive, &crate::test_support::never_cancelled)
            .unwrap();
        let target_root = tempfile::tempdir().unwrap();
        let target = Storage::open(target_root.path().to_owned()).unwrap();
        let prepared = target
            .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
        let confirmation = prepared.preview().confirmation_id;
        let receipt = prepared
            .publish(
                &target,
                &confirmation,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .unwrap();
        let restored_meeting = &receipt.meeting_ids[0];
        let jobs = target.list_jobs(restored_meeting).unwrap();
        assert_eq!(jobs.len(), 1);
        let restored_job = &jobs[0];
        let rebuilt = target.fixed_job_input(&restored_job.id).unwrap();
        for field in ["knownInput", "matching"] {
            assert_eq!(
                rebuilt.0.parameters["diarization"][field],
                saved.0.parameters["diarization"][field]
            );
        }
        let mut portable_extractor = saved.0.parameters["diarization"]["extractor"].clone();
        for field in ["helper", "library"] {
            portable_extractor[field]["path"] = json!(format!(
                "mon-unavailable:{}",
                portable_extractor[field]["sha256"].as_str().unwrap()
            ));
        }
        assert_eq!(
            rebuilt.0.parameters["diarization"]["extractor"],
            portable_extractor
        );
        assert_ne!(
            rebuilt.0.parameters["diarization"]["source"]["tracks"][0]["id"],
            "track"
        );
        assert_eq!(std::fs::read(root.path().join("calls")).unwrap(), calls);
        assert_eq!(
            std::fs::read(root.path().join("embedding.log")).unwrap(),
            features
        );
        assert_eq!(storage.fixed_job_input(&job.id).unwrap(), saved);
        let rebuilt_parameters = envelope(&rebuilt.0).unwrap().diarization;
        let checkpoint: Checkpoint = serde_json::from_value(rebuilt.1.unwrap()).unwrap();
        for (index, output) in checkpoint.outputs.iter().enumerate() {
            let restored_result = load_output(
                &target,
                &rebuilt.0,
                &rebuilt_parameters,
                output,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let old_checkpoint: Checkpoint =
                serde_json::from_value(saved.1.clone().unwrap()).unwrap();
            let old_parameters = envelope(&saved.0).unwrap().diarization;
            let old_result = load_output(
                &storage,
                &saved.0,
                &old_parameters,
                &old_checkpoint.outputs[index],
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            assert_eq!(restored_result.features, old_result.features);
            assert_eq!(restored_result.run, old_result.run);
        }
        let id = if partial {
            use crate::model_inspection::{
                Engine, PackageFile, PackageManifest, Role as ModelRole, Target,
            };
            let package_source = tempfile::tempdir().unwrap();
            let files: Vec<_> = [
                ("cli", ModelRole::Executable),
                ("segmentation.onnx", ModelRole::Segmentation),
                ("embedding.onnx", ModelRole::SpeakerEmbedding),
                ("libsherpa-onnx-c-api.dylib", ModelRole::RuntimeLibrary),
            ]
            .into_iter()
            .map(|(name, role)| {
                std::fs::copy(root.path().join(name), package_source.path().join(name)).unwrap();
                PackageFile {
                    relative_path: name.into(),
                    role,
                    expected_size: None,
                    expected_hash: None,
                }
            })
            .collect();
            let manifest = PackageManifest {
                schema_version: 1,
                engine: Engine::Sherpa,
                engine_version: "test-only".into(),
                model_name: "saved-run fixture".into(),
                model_version: "1".into(),
                quantization: None,
                license_ref: "test-only".into(),
                target: Target::current().unwrap(),
                files,
            };
            std::fs::write(
                package_source.path().join("mon-model.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            let models_directory = tempfile::tempdir().unwrap();
            let models = models_directory
                .path()
                .canonicalize()
                .unwrap()
                .join("models");
            let package = uuid::Uuid::new_v4().to_string();
            let check = uuid::Uuid::new_v4().to_string();
            let inspected = crate::model_inspection::inspect(
                package_source.path(),
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            crate::model_placement::place(
                &target,
                &models,
                package_source.path(),
                &package,
                &inspected.review_hash,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let report = crate::model_compatibility::check(
                &target,
                &models,
                &check,
                &package,
                &inspected.review_hash,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            assert_eq!(serde_json::to_value(report).unwrap()["state"], "succeeded");
            let before_retry_calls = std::fs::read(root.path().join("calls")).unwrap();
            let preview = crate::model_rebinding::preview(
                &target,
                &models,
                restored_meeting,
                &restored_job.id,
                &restored_job.spec_hash,
                &package,
                &inspected.review_hash,
                &check,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let retry = crate::model_rebinding::apply(
                &target,
                &models,
                &uuid::Uuid::new_v4().to_string(),
                &preview,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            assert_eq!(
                target.fixed_job_input(&retry.id).unwrap().0.parameters,
                rebuilt.0.parameters
            );
            assert_eq!(
                std::fs::read(root.path().join("calls")).unwrap(),
                before_retry_calls
            );
            assert_eq!(target.acquire_next_job().unwrap().unwrap().id, retry.id);
            let id = process(
                &target,
                &retry.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                std::fs::read(root.path().join("calls")).unwrap().len(),
                before_retry_calls.len() + 1
            );
            assert_eq!(
                std::fs::read(root.path().join("embedding.log")).unwrap(),
                features
            );
            assert_eq!(
                std::fs::read_to_string(models.join(package).join("embedding.log"))
                    .unwrap()
                    .lines()
                    .filter(|line| *line == "create")
                    .count(),
                1
            );
            id
        } else {
            restored_job.output_analysis_id.clone().unwrap()
        };
        let candidate = target
            .load_visible_analysis(restored_meeting, &id)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.utterances[3].start_us, 17_000_000);
        assert_eq!(candidate.utterances[0].effective_text, "修正した本文");
        assert!(candidate.utterances[2].speaker_ids.is_empty());
        assert_eq!(
            std::fs::read(root.path().join("calls")).unwrap(),
            if partial { &b"xxx"[..] } else { &b"xx"[..] }
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("embedding.log"))
                .unwrap()
                .lines()
                .filter(|l| *l == "create")
                .count(),
            if partial { 1 } else { 2 }
        );
    }
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_keeps_real_overlap_and_refuses_requested_count_that_would_merge_it() {
    for (mode, count) in [
        ("same_voice_fixture", None),
        ("notready", None),
        ("same_voice_fixture", Some(1)),
    ] {
        let (_root, storage, meeting, job) = worker_fixture_rows(
            mode,
            count,
            "0.000 -- 4.000 speaker_00\n2.000 -- 3.000 speaker_01\n4.000 -- 5.000 speaker_01\n",
        );
        storage.acquire_next_job().unwrap().unwrap();
        let output = process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        );
        if count.is_some() {
            assert!(output.is_err());
            assert!(storage
                .get_job(&job.id)
                .unwrap()
                .output_analysis_id
                .is_none());
            continue;
        }
        let id = output.unwrap().unwrap();
        let candidate = storage
            .load_visible_analysis(&meeting, &id)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.utterances[1].speaker_ids.len(), 2);
    }
}

#[test]
#[ignore = "dedicated signed helper and fake C API; no live recording"]
fn saved_run_worker_automatic_handoff_preserves_existing_text_without_explicit_regeneration() {
    let (root, storage, meeting, unused) = worker_fixture("voice_fixture", None);
    storage
        .finish_job(&unused.id, 0, "cancelled", None)
        .unwrap();
    let original = storage.load_current_analysis(&meeting).unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap();
    let mut whisper = storage.analysis_preferences().unwrap().payload;
    whisper.standard.executable_path = root.path().join("fake-cli").to_str().unwrap().into();
    whisper.standard.model_path = root.path().join("model.bin").to_str().unwrap().into();
    whisper.standard.threads = 1;
    whisper.standard.use_gpu = false;
    storage.save_analysis_preferences(0, whisper).unwrap();
    let mut sherpa = storage.diarization_preferences().unwrap().payload;
    sherpa.standard.executable_path = root.path().join("cli").to_str().unwrap().into();
    sherpa.standard.segmentation_model_path = root
        .path()
        .join("segmentation.onnx")
        .to_str()
        .unwrap()
        .into();
    sherpa.standard.embedding_model_path =
        root.path().join("embedding.onnx").to_str().unwrap().into();
    sherpa.standard.runtime_library_paths = vec![root
        .path()
        .join("libsherpa-onnx-c-api.dylib")
        .to_str()
        .unwrap()
        .into()];
    sherpa.standard.threads = 1;
    sherpa.standard.min_duration_on_ms = 0;
    sherpa.standard.min_duration_off_ms = 0;
    storage.save_diarization_preferences(0, sherpa).unwrap();
    storage.ensure_analysis_flow(&meeting).unwrap();
    crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let transcription = storage.acquire_next_job().unwrap().unwrap();
    assert_eq!(transcription.kind, "transcription");
    crate::transcription::process(
        &storage,
        &transcription.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = storage.analysis_flow(&meeting).unwrap().unwrap();
    assert_eq!(flow.stage, "diarization", "{:?}", flow.error_message);
    let job = storage.acquire_next_job().unwrap().unwrap();
    assert_eq!(job.kind, "diarization");
    assert_eq!(job.total_units, Some(2));
    assert_eq!(
        storage.fixed_job_input(&job.id).unwrap().0.parameters["schemaVersion"],
        2
    );
    let id = super::super::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = storage.analysis_flow(&meeting).unwrap().unwrap();
    assert_eq!(flow.stage, "attention", "{:?}", flow.error_message);
    assert_eq!(
        flow.error_message.as_deref(),
        Some("既存の文字起こしと編集を保持しています。自動処理で上書きしません。")
    );
    let candidate = storage
        .load_visible_analysis(&meeting, &id)
        .unwrap()
        .unwrap();
    assert_eq!(
        candidate
            .utterances
            .iter()
            .map(|u| (u.start_us, u.end_us))
            .collect::<Vec<_>>(),
        vec![(2_500_000, 3_000_000), (17_500_000, 18_000_000)]
    );
    assert_ne!(
        candidate.utterances[0].speaker_ids,
        candidate.utterances[1].speaker_ids
    );
    assert_eq!(storage.load_current_analysis(&meeting).unwrap(), original);
    assert_eq!(storage.load_transcript_draft(&meeting).unwrap(), draft);
}
