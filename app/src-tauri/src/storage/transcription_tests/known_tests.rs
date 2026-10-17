use super::*;
use crate::storage::transcription::known::KnownInput;
use sha2::{Digest, Sha256};

pub(crate) fn split_fixture(root: &Path) -> (Storage, String, WhisperConfig) {
    let (storage, meeting, config) = fixture(root, 10);
    let path = storage
        .root
        .join("meetings")
        .join(&meeting)
        .join("audio/mic.caf");
    let pcm: Vec<u8> = (0..160000)
        .flat_map(|index| if index < 80000 { 0.25_f32 } else { -0.5_f32 }.to_le_bytes())
        .collect();
    let caf = caf_fixture(16000, 1, 3, 32, 4, &pcm);
    std::fs::write(path, &caf).unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute("UPDATE file_assets SET size_bytes=?1,content_hash=?2 WHERE id=(SELECT file_asset_id FROM audio_chunks WHERE id='chunk')",params![caf.len() as i64,format!("{:x}",Sha256::digest(&caf))]).unwrap();
    db.execute_batch("UPDATE audio_chunks SET end_us=20000000; UPDATE meetings SET duration_us=20000000; UPDATE recording_sessions SET duration_us=20000000;").unwrap();
    for (id, first, end, start, last) in [
        ("left", 0, 80000, 0, 5000000),
        ("right", 80000, 160000, 15000000, 20000000),
    ] {
        db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track','chunk',?2,?3,?4,?5,'{\"schema_version\":1,\"method\":\"linear\"}')",params![id,first,end,start,last]).unwrap();
    }
    db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',5000000,15000000,'capture_timestamp_gap',0)",[]).unwrap();
    drop(db);
    (storage, meeting, config)
}

fn pause_first(storage: &Storage, job: &JobSnapshot) {
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(
        storage,
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
}

fn read_wav(storage: &Storage, asset: &str) -> Vec<u8> {
    let relative: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            params![asset],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::read(storage.root.join(relative)).unwrap()
}

#[test]
fn known_runs_publish_original_meeting_times_and_exact_nonzero_pcm_without_gap_padding() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
    let source = std::fs::read(
        storage
            .root
            .join("meetings")
            .join(&meeting)
            .join("audio/mic.caf"),
    )
    .unwrap();
    let job = enqueue(&storage, &meeting, "known", config, "standard").unwrap();
    let fixed = storage.fixed_job_input(&job.id).unwrap().0;
    assert_eq!(fixed.parameters["schemaVersion"], 3);
    assert_eq!(job.total_units, Some(2));
    assert_eq!(fixed.parameters["units"][0]["runIndex"], 0);
    assert_eq!(fixed.parameters["units"][1]["runIndex"], 1);
    assert_eq!(
        fixed.parameters["knownInput"]["runs"][1]["firstFrame"],
        240000
    );
    storage.acquire_next_job().unwrap().unwrap();
    let candidate = process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let view = storage
        .load_visible_analysis(&meeting, &candidate)
        .unwrap()
        .unwrap();
    let times: Vec<_> = view
        .utterances
        .iter()
        .map(|u| (u.start_us, u.end_us))
        .collect();
    assert_eq!(times, vec![(2500000, 3000000), (17500000, 18000000)]);
    let checkpoint = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    for (index, value) in [8192_i16, -16384].into_iter().enumerate() {
        let wav = read_wav(
            &storage,
            checkpoint["outputs"][index]["wavAssetId"].as_str().unwrap(),
        );
        assert_eq!(wav.len(), 44 + 80000 * 2);
        assert!(wav[44..]
            .chunks_exact(2)
            .all(|sample| i16::from_le_bytes(sample.try_into().unwrap()) == value));
    }
    assert_eq!(
        std::fs::read(
            storage
                .root
                .join("meetings")
                .join(&meeting)
                .join("audio/mic.caf")
        )
        .unwrap(),
        source
    );
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    assert!(storage
        .prepare_track_audio(&meeting, "track", 0, 20000000)
        .is_err());
}

#[test]
fn known_runs_pause_reopen_and_retry_reuse_exact_durable_units() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, mut config) = split_fixture(root.path());
    let calls = root.path().join("calls");
    let body = std::fs::read_to_string(&config.executable.path).unwrap();
    std::fs::write(
        &config.executable.path,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\n{}",
            calls.display(),
            body.strip_prefix("#!/bin/sh\n").unwrap()
        ),
    )
    .unwrap();
    config.executable = Fingerprint::capture(
        &config.executable.path,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let job = enqueue(&storage, &meeting, "known-retry", config, "standard").unwrap();
    pause_first(&storage, &job);
    let saved = storage.fixed_job_input(&job.id).unwrap();
    let failed = storage
        .finish_job(&job.id, 1, "failed", Some("fixture"))
        .unwrap();
    let retry = RetryRequest {
        meeting_id: meeting.clone(),
        job_id: job.id.clone(),
        operation_id: "retry-known".into(),
        expected_spec_hash: failed.spec_hash,
    }
    .enqueue(&storage)
    .unwrap();
    assert_eq!(retry.completed_units, 1);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, retry.id);
    let candidate = process(
        &storage,
        &retry.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(std::fs::read(calls).unwrap(), b"xx");
    assert_eq!(storage.fixed_job_input(&job.id).unwrap(), saved);
    assert_eq!(
        storage.fixed_job_input(&retry.id).unwrap().1.unwrap()["outputs"][0],
        saved.1.unwrap()["outputs"][0]
    );
    assert_eq!(
        storage
            .load_visible_analysis(&meeting, &candidate)
            .unwrap()
            .unwrap()
            .utterances[1]
            .start_us,
        17500000
    );
}

#[test]
fn known_runs_reject_missing_partial_recovered_or_changed_gap_proofs() {
    for case in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting, config) = split_fixture(root.path());
        let query = match case {
            0 => "DELETE FROM audio_gaps",
            1 => "UPDATE audio_gaps SET end_us=14999999",
            2 => "UPDATE audio_gaps SET recovered=1",
            _ => "UPDATE audio_gaps SET start_us=5000001",
        };
        storage
            .connection
            .lock()
            .unwrap()
            .execute(query, [])
            .unwrap();
        assert!(
            enqueue(&storage, &meeting, "unproved", config, "standard").is_err(),
            "case {case}"
        );
        assert!(storage.list_jobs(&meeting).unwrap().is_empty());
    }
    for query in [
        "UPDATE audio_gaps SET reason_code='changed'",
        "UPDATE audio_gaps SET start_us=4999999",
        "UPDATE meetings SET duration_us=21000000",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting, config) = split_fixture(root.path());
        let job = enqueue(&storage, &meeting, "changed-known", config, "standard").unwrap();
        pause_first(&storage, &job);
        let fixed = storage.fixed_job_input(&job.id).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(query, [])
            .unwrap();
        let failed = storage
            .finish_job(&job.id, 1, "failed", Some("fixture"))
            .unwrap();
        assert!(RetryRequest {
            meeting_id: meeting.clone(),
            job_id: job.id.clone(),
            operation_id: "no-retry".into(),
            expected_spec_hash: failed.spec_hash
        }
        .enqueue(&storage)
        .is_err());
        assert_eq!(storage.fixed_job_input(&job.id).unwrap(), fixed);
        assert_eq!(storage.list_jobs(&meeting).unwrap().len(), 1);
    }
}

#[test]
fn known_runs_refuse_corrupt_source_even_outside_current_unit_and_observe_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    let job = enqueue(&storage, &meeting, "known-corrupt", config, "standard").unwrap();
    let (source, known) = storage.recorded_analysis_input(&meeting).unwrap();
    let run = &known.unwrap().runs[0];
    assert!(storage
        .prepare_analysis_run(&meeting, &source.tracks[0], run, &|| true)
        .is_err());
    let path = storage
        .root
        .join("meetings")
        .join(&meeting)
        .join("audio/mic.caf");
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(path, bytes).unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 0);
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
}

#[test]
fn continuous_map_labels_constrain_analysis_filter_halo_and_do_not_leak_other_run() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = split_fixture(root.path());
    storage.connection.lock().unwrap().execute_batch("DELETE FROM track_clock_segments; UPDATE audio_chunks SET end_us=11000000; UPDATE meetings SET duration_us=11000000; UPDATE recording_sessions SET duration_us=11000000; UPDATE audio_gaps SET start_us=4400000,end_us=6600000; INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES ('drift','track','chunk',0,160000,0,11000000,'{\"schema_version\":1,\"method\":\"linear\"}');").unwrap();
    let (source, known) = storage.recorded_analysis_input(&meeting).unwrap();
    let known = known.unwrap();
    for (index, value) in [(0, 0.25_f32), (1, -0.5_f32)] {
        let run = &known.runs[index];
        let mut audio = storage
            .prepare_analysis_run(
                &meeting,
                &source.tracks[0],
                run,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        for first in [0, audio.total_frames() - 64] {
            assert!(audio
                .read_pcm(first, 64)
                .unwrap()
                .chunks_exact(4)
                .all(
                    |sample| (f32::from_le_bytes(sample.try_into().unwrap()) - value).abs() < 1e-6
                ));
        }
    }
}

#[test]
fn fractional_gap_frames_remain_on_meeting_lattice_and_are_not_synthesized() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = fixture(root.path(), 10);
    storage.connection.lock().unwrap().execute_batch("UPDATE meetings SET duration_us=10000000; INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',5000001,5000002,'dropped',0);").unwrap();
    let (source, known) = storage.recorded_analysis_input(&meeting).unwrap();
    let known = known.unwrap();
    assert_eq!(
        (known.runs[0].end_frame, known.runs[1].first_frame),
        (80000, 80001)
    );
    assert_eq!(known.runs[1].start_us, 5000002);
    for run in &known.runs {
        let audio = storage
            .prepare_analysis_run(
                &meeting,
                &source.tracks[0],
                run,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        assert_eq!(audio.total_frames(), run.end_frame - run.first_frame);
    }
}

#[test]
fn known_run_snapshot_cannot_reorder_change_track_or_derived_frame_bounds() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    let job = enqueue(&storage, &meeting, "sealed-known", config, "standard").unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&job.id).unwrap();
    for case in 0..4 {
        let mut bad = spec.clone();
        match case {
            0 => bad.parameters["knownInput"]["runs"][0]["endFrame"] = serde_json::json!(80001),
            1 => bad.parameters["units"][1]["runIndex"] = serde_json::json!(0),
            2 => bad.parameters["knownInput"]["gaps"][0]["trackIndex"] = serde_json::json!(99),
            _ => bad.parameters["knownInput"]["runs"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1),
        }
        assert!(
            crate::transcription::retry_preflight(
                &storage,
                &job,
                &bad,
                checkpoint.as_ref(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "case {case}"
        );
    }
    let known: KnownInput = serde_json::from_value(spec.parameters["knownInput"].clone()).unwrap();
    let source: RecordedAudioPlan =
        serde_json::from_value(spec.parameters["source"].clone()).unwrap();
    known.validate(&source).unwrap();
    assert_eq!(storage.fixed_job_input(&job.id).unwrap().0, spec);
}

#[test]
fn known_runs_merge_overlapping_labels_and_reject_an_entirely_unavailable_track() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = fixture(root.path(), 10);
    storage.connection.lock().unwrap().execute_batch("UPDATE meetings SET duration_us=10000000; INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('a','track',3000000,5000000,'dropped',0),('b','track',4000000,7000000,'dropped',0);").unwrap();
    let (_, known) = storage.recorded_analysis_input(&meeting).unwrap();
    assert_eq!(
        known
            .unwrap()
            .runs
            .iter()
            .map(|run| (run.start_us, run.end_us))
            .collect::<Vec<_>>(),
        vec![(0, 3000000), (7000000, 10000000)]
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE audio_gaps SET start_us=0,end_us=10000000", [])
        .unwrap();
    assert!(storage.recorded_analysis_input(&meeting).is_err());
}

#[test]
fn continuous_jobs_keep_schema_two_and_no_optional_run_fields() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = fixture(root.path(), 4);
    let job = enqueue(&storage, &meeting, "legacy-shape", config, "standard").unwrap();
    let spec = storage.fixed_job_input(&job.id).unwrap().0;
    assert_eq!(spec.parameters["schemaVersion"], 2);
    assert!(spec.parameters.get("knownInput").is_none());
    assert!(spec.parameters["units"][0].get("runIndex").is_none());
    assert_eq!(
        spec.parameters["source"],
        serde_json::to_value(storage.recorded_audio_plan(&meeting).unwrap()).unwrap()
    );
}

#[test]
fn known_runs_additive_restore_keeps_times_and_fixed_ranges_without_running_engine() {
    for partial in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let target_root = tempfile::tempdir().unwrap();
        let (storage, meeting, mut config) = split_fixture(root.path());
        let calls = root.path().join("restore-calls");
        let body = std::fs::read_to_string(&config.executable.path).unwrap();
        std::fs::write(
            &config.executable.path,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\n{}",
                calls.display(),
                body.strip_prefix("#!/bin/sh\n").unwrap()
            ),
        )
        .unwrap();
        config.executable = Fingerprint::capture(
            &config.executable.path,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let job = enqueue(&storage, &meeting, "known-restore", config, "standard").unwrap();
        if partial {
            pause_first(&storage, &job);
        } else {
            storage.acquire_next_job().unwrap().unwrap();
            process(
                &storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .unwrap()
            .unwrap();
        }
        let before = std::fs::read(&calls).unwrap();
        let saved = storage.fixed_job_input(&job.id).unwrap();
        let archive = destination.path().join("known.monbackup");
        storage
            .create_backup_archive(&archive, &crate::test_support::never_cancelled)
            .unwrap();
        let target = Storage::open(target_root.path().into()).unwrap();
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
        let new_meeting = &receipt.meeting_ids[0];
        assert_ne!(new_meeting, &meeting);
        let rebuilt = target.list_jobs(new_meeting).unwrap();
        assert_eq!(rebuilt.len(), 1);
        let fixed = target.fixed_job_input(&rebuilt[0].id).unwrap();
        assert_eq!(
            fixed.0.parameters["knownInput"],
            saved.0.parameters["knownInput"]
        );
        assert_eq!(fixed.0.parameters["units"], saved.0.parameters["units"]);
        assert_ne!(
            fixed.0.parameters["source"]["tracks"][0]["id"],
            saved.0.parameters["source"]["tracks"][0]["id"]
        );
        let (source, known) = target.recorded_analysis_input(new_meeting).unwrap();
        assert_eq!(
            serde_json::to_value(known.unwrap()).unwrap(),
            saved.0.parameters["knownInput"]
        );
        assert_eq!(
            fixed.0.parameters["source"],
            serde_json::to_value(source).unwrap()
        );
        assert_eq!(
            fixed.1.unwrap()["outputs"].as_array().unwrap().len(),
            if partial { 1 } else { 2 }
        );
        assert_eq!(std::fs::read(&calls).unwrap(), before);
        assert_eq!(storage.fixed_job_input(&job.id).unwrap(), saved);
        if !partial {
            let candidate = rebuilt[0].output_analysis_id.as_ref().unwrap();
            let view = target
                .load_visible_analysis(new_meeting, candidate)
                .unwrap()
                .unwrap();
            assert_eq!(
                view.utterances
                    .iter()
                    .map(|u| (u.start_us, u.end_us))
                    .collect::<Vec<_>>(),
                vec![(2500000, 3000000), (17500000, 18000000)]
            );
        }
    }
}

#[test]
fn known_runs_do_not_hide_a_missing_chunk_or_asset_state() {
    for query in [
        "UPDATE audio_chunks SET state='missing'",
        "UPDATE file_assets SET state='missing' WHERE media_type='audio/x-caf'",
        "UPDATE audio_chunks SET sequence=1",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting, config) = split_fixture(root.path());
        storage
            .connection
            .lock()
            .unwrap()
            .execute(query, [])
            .unwrap();
        assert!(enqueue(&storage, &meeting, "no-hidden-files", config, "standard").is_err());
        assert!(storage.list_jobs(&meeting).unwrap().is_empty());
    }
}

#[test]
fn failed_companion_input_is_not_silently_removed_from_fixed_analysis_source() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    storage.connection.lock().unwrap().execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('system-track',?1,'session','system','caf',16000,1,'failed',0,0)",params![meeting]).unwrap();
    assert!(enqueue(&storage, &meeting, "failed-companion", config, "standard").is_err());
    assert!(storage.list_jobs(&meeting).unwrap().is_empty());
}

#[test]
fn paired_inputs_keep_independent_runs_formats_and_original_timestamps() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    let relative = PathBuf::from("meetings")
        .join(&meeting)
        .join("audio/system.caf");
    let pcm: Vec<u8> = (0..160000)
        .flat_map(|_| {
            [0.125_f32, 0.375]
                .into_iter()
                .flat_map(|value| value.to_le_bytes())
        })
        .collect();
    std::fs::write(
        storage.root.join(&relative),
        caf_fixture(8000, 2, 3, 32, 4, &pcm),
    )
    .unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting, &relative, "audio/x-caf")
        .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('system-track',?1,'session','system','caf',8000,2,'ready',0,0)",params![meeting]).unwrap();
        db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES ('system-chunk',?1,'system-track',?2,0,0,20000000,160000,8000,2,'ready',0)",params![meeting,asset.id]).unwrap();
    }
    let job = enqueue(&storage, &meeting, "paired-known", config, "standard").unwrap();
    assert_eq!(job.total_units, Some(3));
    storage.acquire_next_job().unwrap().unwrap();
    let candidate = process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let view = storage
        .load_visible_analysis(&meeting, &candidate)
        .unwrap()
        .unwrap();
    let mut times: Vec<_> = view
        .utterances
        .iter()
        .map(|u| (u.track_id.as_str(), u.start_us, u.end_us))
        .collect();
    times.sort();
    assert_eq!(
        times,
        vec![
            ("system-track", 2500000, 3000000),
            ("track", 2500000, 3000000),
            ("track", 17500000, 18000000)
        ]
    );
    let checkpoint = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    let wav = read_wav(
        &storage,
        checkpoint["outputs"][2]["wavAssetId"].as_str().unwrap(),
    );
    assert_eq!(wav.len(), 44 + 320000 * 2);
    assert!(wav[44..]
        .chunks_exact(2)
        .all(|sample| i16::from_le_bytes(sample.try_into().unwrap()) == 8192));
}

#[test]
fn publication_transaction_rechecks_gap_snapshot_and_rejects_cross_gap_utterances() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config) = split_fixture(root.path());
    let job = enqueue(&storage, &meeting, "publication-known", config, "standard").unwrap();
    let spec = storage.fixed_job_input(&job.id).unwrap().0;
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|snapshot| {
            if snapshot.completed_units == 2 && snapshot.state == "running" {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute("UPDATE audio_gaps SET reason_code='changed'", [])
                    .unwrap();
            }
        }
    )
    .is_err());
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 2);
    let input = AnalysisInput {
        audio_manifest_hash: spec.input_hash.clone(),
        profile_snapshot: spec.profile_snapshot.clone(),
        config: serde_json::json!({"jobId":job.id,"audioCoverage":{"mode":"known_runs","runCount":2,"missingRangeCount":1,"timeBase":"meeting"}}),
        speakers: vec![],
        segments: vec![],
        utterances: vec![UtteranceInput {
            track_id: "track".into(),
            start_us: 2500000,
            end_us: 3000000,
            text: "known".into(),
            speaker_keys: vec![],
            confidence: None,
        }],
    };
    assert!(storage
        .finish_transcription_candidate(&meeting, &job.id, 2, input.clone())
        .is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_gaps SET reason_code='capture_timestamp_gap'",
            [],
        )
        .unwrap();
    let mut crossed = input;
    crossed.utterances[0].start_us = 4500000;
    crossed.utterances[0].end_us = 15500000;
    assert!(storage
        .finish_transcription_candidate(&meeting, &job.id, 2, crossed)
        .is_err());
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
    assert_eq!(storage.fixed_job_input(&job.id).unwrap().0, spec);
}
