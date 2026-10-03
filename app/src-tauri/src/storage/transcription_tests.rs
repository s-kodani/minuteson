use super::*;
use crate::audio::tests::caf_fixture;
use crate::transcription::{
    enqueue, process,
    whisper::{Fingerprint, WhisperConfig},
    RetryRequest,
};
use std::os::unix::fs::PermissionsExt;

fn fixture(root: &Path, seconds: i64) -> (Storage, String, WhisperConfig) {
    let storage = Storage::open(root.to_owned()).unwrap();
    let meeting = storage.create_meeting("解析結合テスト").unwrap();
    let relative = PathBuf::from("meetings").join(&meeting.id).join("audio");
    let directory = storage.managed_path(&relative).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET source_type = 'recording', lifecycle = 'recorded' WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
    std::fs::write(directory.join("manifest.json"), b"{\"state\":\"saved\"}").unwrap();
    let manifest = storage
        .register_ready_meeting_asset(
            &meeting.id,
            &relative.join("manifest.json"),
            "application/json",
        )
        .unwrap();
    let pcm: Vec<u8> = (0..16_000 * seconds)
        .flat_map(|_| 0.25_f32.to_le_bytes())
        .collect();
    std::fs::write(
        directory.join("mic.caf"),
        caf_fixture(16_000, 1, 3, 32, 4, &pcm),
    )
    .unwrap();
    let audio = storage
        .register_ready_meeting_asset(&meeting.id, &relative.join("mic.caf"), "audio/x-caf")
        .unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute("INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, audio_manifest_hash, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', ?2, 0, 0)", params![meeting.id, manifest.content_hash]).unwrap();
    db.execute("INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'caf', 16000, 1, 'ready', 0, 0)", params![meeting.id]).unwrap();
    db.execute("INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES ('chunk', ?1, 'track', ?2, 0, 0, ?3, ?4, 16000, 1, 'ready', 0)", params![meeting.id, audio.id, seconds * 1_000_000, seconds * 16_000]).unwrap();
    drop(db);
    let model_path = root.join("model.bin");
    let cli_path = root.join("fake-cli");
    std::fs::write(&model_path, b"model").unwrap();
    let model_path = model_path.canonicalize().unwrap();
    let json = serde_json::json!({"model":{"multilingual":true},"params":{"model":model_path,"language":"ja","translate":false},"transcription":[{"offsets":{"from":2500,"to":3000},"text":"点検日です"}]}).to_string();
    // A protocol fixture, not a speech recognizer or a quality claim.
    std::fs::write(
        &cli_path,
        format!("#!/bin/sh\nprintf '%s' '{json}' > result.json\n"),
    )
    .unwrap();
    std::fs::set_permissions(&cli_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = WhisperConfig {
        executable: Fingerprint::capture(&cli_path, &|| false).unwrap(),
        model: Fingerprint::capture(&model_path, &|| false).unwrap(),
        vad_model: None,
        threads: 1,
        use_gpu: false,
        prompt: "点検日".into(),
    };
    (storage, meeting.id, config)
}

#[test]
fn retry_after_engine_failure_reuses_only_durable_units_and_preserves_edits() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, mut config) = fixture(root.path(), 124);
    let gate = root.path().join("fail-second-unit");
    let calls = root.path().join("cli-calls");
    let original_script = std::fs::read_to_string(&config.executable.path).unwrap();
    let body = original_script.strip_prefix("#!/bin/sh\n").unwrap();
    std::fs::write(&config.executable.path, format!("#!/bin/sh\nprintf x >> '{}'\ncase \"$PWD\" in */unit-1-*) if test -f '{}'; then exit 42; fi;; esac\n{body}", calls.display(), gate.display())).unwrap();
    config.executable = Fingerprint::capture(&config.executable.path, &|| false).unwrap();
    let baseline = enqueue(&storage, &meeting, "baseline", config.clone(), "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    let current = process(&storage, &baseline.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let lock = storage.get_meeting(&meeting).unwrap().unwrap().lock_version;
    storage
        .commit_analysis_candidate(&meeting, &current, lock, false)
        .unwrap();
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &current).unwrap();
    let mut payload = draft.payload.clone();
    payload
        .utterances
        .push(super::super::drafts::UtteranceEdit {
            utterance_id: view.utterances[0].id.clone(),
            effective_text: "保持する手動編集".into(),
            speaker_ids: vec![],
        });
    let edited = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();

    std::fs::write(&gate, b"fail").unwrap();
    let failed = enqueue(&storage, &meeting, "first-attempt", config, "high_accuracy").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(&storage, &failed.id, &|| false, &|_| {}).is_err());
    assert_eq!(storage.get_job(&failed.id).unwrap().completed_units, 1);
    let failed = storage
        .finish_job(&failed.id, 1, "failed", Some("engine_failed"))
        .unwrap();
    let saved = storage.fixed_job_input(&failed.id).unwrap();
    let calls_before = std::fs::read(&calls).unwrap().len();
    assert_eq!(calls_before, 4); // baseline 2 + first unit + failing second unit
    std::fs::remove_file(&gate).unwrap();
    let next = RetryRequest {
        meeting_id: meeting.clone(),
        job_id: failed.id.clone(),
        operation_id: "retry-operation".into(),
        expected_spec_hash: failed.spec_hash.clone(),
    }
    .enqueue(&storage)
    .unwrap();
    assert_eq!(next.completed_units, 1);
    assert_eq!(next.attempt, 2);
    assert_eq!(next.retry_of_job_id.as_deref(), Some(failed.id.as_str()));
    assert_eq!(storage.fixed_job_input(&next.id).unwrap().1, saved.1);
    drop(storage);

    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, next.id);
    let candidate = process(&storage, &next.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(&calls).unwrap().len(), calls_before + 1);
    assert_eq!(storage.get_job(&next.id).unwrap().state, "succeeded");
    let inherited = storage.fixed_job_input(&next.id).unwrap().1.unwrap();
    assert_eq!(
        inherited["outputs"][0],
        saved.1.as_ref().unwrap()["outputs"][0]
    );
    assert_eq!(storage.fixed_job_input(&failed.id).unwrap(), saved);
    assert_eq!(storage.get_job(&failed.id).unwrap(), failed);
    assert_eq!(
        storage.load_current_analysis(&meeting).unwrap().unwrap(),
        view
    );
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        edited
    );
    assert_eq!(
        storage
            .load_visible_analysis(&meeting, &candidate)
            .unwrap()
            .unwrap()
            .utterances
            .len(),
        2
    );
    let versions = storage.analysis_versions(&meeting).unwrap();
    let summary = versions
        .versions
        .iter()
        .find(|item| item.id == candidate)
        .unwrap();
    assert_eq!(summary.config["retryOfJobId"], failed.id);
    assert_eq!(summary.config["attempt"], 2);
}

#[test]
fn retry_preflight_rejects_changed_files_and_checkpoint_without_new_job() {
    for change in ["source", "model", "result", "checkpoint", "meeting", "hash"] {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting, config) = fixture(root.path(), 124);
        let job = enqueue(&storage, &meeting, "source", config, "standard").unwrap();
        storage.acquire_next_job().unwrap().unwrap();
        process(&storage, &job.id, &|| false, &|snapshot| {
            if snapshot.completed_units == 1 && snapshot.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap();
        let source = storage
            .finish_job(&job.id, 1, "failed", Some("engine_failed"))
            .unwrap();
        let mut request = RetryRequest {
            meeting_id: meeting.clone(),
            job_id: source.id.clone(),
            operation_id: "retry".into(),
            expected_spec_hash: source.spec_hash.clone(),
        };
        match change {
            "source" => std::fs::write(
                root.path()
                    .join("meetings")
                    .join(&meeting)
                    .join("audio/mic.caf"),
                b"changed",
            )
            .unwrap(),
            "model" => std::fs::write(root.path().join("model.bin"), b"changed").unwrap(),
            "result" => {
                let cp = storage.fixed_job_input(&source.id).unwrap().1.unwrap();
                let relative: String = storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE id = ?1",
                        params![cp["outputs"][0]["jsonAssetId"].as_str().unwrap()],
                        |row| row.get(0),
                    )
                    .unwrap();
                std::fs::write(root.path().join(relative), b"changed").unwrap();
            }
            "checkpoint" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE job_checkpoints SET checkpoint_json = '{}' WHERE job_id = ?1",
                        params![source.id],
                    )
                    .unwrap();
            }
            "meeting" => {
                request.meeting_id = storage.create_meeting("別会議").unwrap().id;
            }
            "hash" => {
                request.expected_spec_hash = "f".repeat(64);
            }
            _ => unreachable!(),
        }
        assert!(request.enqueue(&storage).is_err(), "{change}");
        assert_eq!(storage.list_jobs(&meeting).unwrap(), vec![source]);
        assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    }
}

#[test]
fn durable_units_pause_resume_and_publish_a_candidate_without_replacing_edits() {
    let directory = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = fixture(directory.path(), 124);
    let job = enqueue(&storage, &meeting, "operation", config.clone(), "standard").unwrap();
    assert_eq!(job.total_units, Some(2));
    assert_eq!(
        enqueue(&storage, &meeting, "operation", config, "standard")
            .unwrap()
            .id,
        job.id
    );
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    let paused = process(&storage, &job.id, &|| false, &|snapshot| {
        if snapshot.completed_units == 1 && snapshot.state == "running" {
            storage.request_pause_for_recording().unwrap();
        }
    })
    .unwrap();
    assert!(paused.is_none());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 1);
    let saved_first = storage.fixed_job_input(&job.id).unwrap().1.unwrap()["outputs"][0].clone();
    drop(storage);
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    storage.resume_job(&job.id).unwrap();
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    let candidate = process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
    assert_eq!(
        storage
            .get_job(&job.id)
            .unwrap()
            .output_analysis_id
            .as_deref(),
        Some(candidate.as_str())
    );
    assert_eq!(
        storage.fixed_job_input(&job.id).unwrap().1.unwrap()["outputs"][0],
        saved_first
    );
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    let meeting_record = storage.get_meeting(&meeting).unwrap().unwrap();
    storage
        .commit_analysis_candidate(&meeting, &candidate, meeting_record.lock_version, false)
        .unwrap();
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    assert_eq!(view.utterances.len(), 2);
    assert_eq!(view.utterances[1].start_us, 120_500_000);
    assert!(view
        .utterances
        .iter()
        .all(|utterance| utterance.attribution_state == "unknown"));
    let draft = storage
        .create_transcript_draft(&meeting, &candidate)
        .unwrap();
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    let mut retry = spec;
    retry.operation_id = "second-analysis".into();
    let second = storage.enqueue_job(retry).unwrap();
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    process(&storage, &second.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(
        storage.load_current_analysis(&meeting).unwrap().unwrap(),
        view
    );
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        draft
    );
}

#[test]
fn changed_source_model_and_checkpointed_output_are_rejected() {
    for change in ["source", "model", "result", "spec"] {
        let directory = tempfile::tempdir().unwrap();
        let (storage, meeting, config) = fixture(directory.path(), 124);
        let job = enqueue(&storage, &meeting, "operation", config, "high_accuracy").unwrap();
        storage
            .acquire_job_kind(Some(JobKind::Transcription))
            .unwrap()
            .unwrap();
        process(&storage, &job.id, &|| false, &|snapshot| {
            if snapshot.completed_units == 1 && snapshot.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap();
        let path = match change {
            "source" => directory
                .path()
                .join("meetings")
                .join(&meeting)
                .join("audio/mic.caf"),
            "model" => directory.path().join("model.bin"),
            "result" => {
                let checkpoint = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
                let asset = checkpoint["outputs"][0]["jsonAssetId"].as_str().unwrap();
                let relative: String = storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE id = ?1",
                        params![asset],
                        |row| row.get(0),
                    )
                    .unwrap();
                directory.path().join(relative)
            }
            "spec" => {
                storage.connection.lock().unwrap().execute_batch("DROP TRIGGER job_spec_fixed; UPDATE processing_jobs SET parameters_json = '{}';").unwrap();
                directory.path().join("unused")
            }
            _ => unreachable!(),
        };
        if change != "spec" {
            std::fs::write(path, b"changed").unwrap();
        }
        storage.resume_job(&job.id).unwrap();
        storage
            .acquire_job_kind(Some(JobKind::Transcription))
            .unwrap()
            .unwrap();
        assert!(
            process(&storage, &job.id, &|| false, &|_| {}).is_err(),
            "{change}"
        );
        assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 1);
        assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    }
}

#[test]
fn user_pause_is_not_resumed_by_recording_and_candidate_completion_is_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = fixture(directory.path(), 4);
    let job = enqueue(&storage, &meeting, "operation", config, "standard").unwrap();
    storage.request_job_pause(&job.id, "recording").unwrap();
    storage.request_job_pause(&job.id, "user").unwrap();
    storage.resume_recording_jobs().unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    storage.resume_job(&job.id).unwrap();
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    let input = AnalysisInput {
        audio_manifest_hash: spec.input_hash,
        profile_snapshot: spec.profile_snapshot,
        config: serde_json::json!({}),
        speakers: vec![],
        segments: vec![],
        utterances: vec![],
    };
    assert!(storage
        .finish_transcription_candidate(&meeting, &job.id, 0, input.clone())
        .is_err());
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    storage
        .save_job_checkpoint(&job.id, 0, 1, serde_json::json!({"outputs":[]}))
        .unwrap();
    let candidate = storage
        .finish_transcription_candidate(&meeting, &job.id, 1, input.clone())
        .unwrap();
    assert_eq!(
        storage.get_job(&job.id).unwrap().output_analysis_id,
        Some(candidate)
    );
    assert!(storage
        .finish_transcription_candidate(&meeting, &job.id, 1, input)
        .is_err());
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
#[ignore = "requires MON_WHISPER_CLI, MON_WHISPER_MODEL and MON_WHISPER_INPUT_CAF"]
fn real_whisper_worker_persists_output_checkpoint_and_candidate() {
    use crate::audio::VerifiedAudio;
    let directory = tempfile::tempdir().unwrap();
    let (storage, meeting, mut config) = fixture(directory.path(), 4);
    let input = PathBuf::from(std::env::var("MON_WHISPER_INPUT_CAF").unwrap());
    let proof = Fingerprint::capture(&input, &|| false).unwrap();
    let audio = VerifiedAudio::open(
        std::fs::File::open(&input).unwrap(),
        proof.size,
        &proof.sha256,
    )
    .unwrap();
    let format = audio.format.clone();
    let duration = (format.frames * 1_000_000 / u64::from(format.sample_rate)) as i64;
    let relative = PathBuf::from("meetings")
        .join(&meeting)
        .join("audio/real.caf");
    std::fs::copy(input, storage.managed_path(&relative).unwrap()).unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting, &relative, "audio/x-caf")
        .unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute(
        "UPDATE audio_tracks SET sample_rate = ?1, channels = ?2 WHERE id = 'track'",
        params![format.sample_rate, format.channels],
    )
    .unwrap();
    db.execute("UPDATE audio_chunks SET file_asset_id = ?1, end_us = ?2, frame_count = ?3, sample_rate = ?4, channels = ?5 WHERE id = 'chunk'", params![asset.id, duration, i64::try_from(format.frames).unwrap(), format.sample_rate, format.channels]).unwrap();
    drop(db);
    config.executable = Fingerprint::capture(
        Path::new(&std::env::var("MON_WHISPER_CLI").unwrap()),
        &|| false,
    )
    .unwrap();
    config.model = Fingerprint::capture(
        Path::new(&std::env::var("MON_WHISPER_MODEL").unwrap()),
        &|| false,
    )
    .unwrap();
    config.threads = 4;
    let job = enqueue(&storage, &meeting, "real-integration", config, "standard").unwrap();
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    let candidate = process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let snapshot = storage.get_job(&job.id).unwrap();
    assert_eq!(snapshot.state, "succeeded");
    assert_eq!(snapshot.completed_units, 1);
    assert_eq!(
        snapshot.output_analysis_id.as_deref(),
        Some(candidate.as_str())
    );
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    let lock = storage.get_meeting(&meeting).unwrap().unwrap().lock_version;
    storage
        .commit_analysis_candidate(&meeting, &candidate, lock, false)
        .unwrap();
    assert!(storage.verify_analysis_content_hash(&candidate).unwrap());
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    assert!(!view.utterances.is_empty());
    assert!(view
        .utterances
        .iter()
        .all(|utterance| utterance.attribution_state == "unknown"));
    eprintln!("real whisper.cpp worker integration: {} verified utterances, 1 durable unit, candidate adopted manually", view.utterances.len());
}
