use super::*;
use crate::storage::recordings::{load_session, tests::saved_probe_fixture};
use crate::storage::Storage;
use serde_json::{json, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};

fn segment(first: i64, last: i64, start: i64, end: i64) -> Value {
    json!({"sourceFrameStart":first,"sourceFrameEnd":last,"timelineStartUs":start,"timelineEndUs":end,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"})
}

pub(super) fn corrected(storage: &Storage) -> (String, String, PathBuf, Value) {
    let (meeting, session, output, mut value) = saved_probe_fixture(storage);
    value["schemaVersion"] = json!(4);
    value["captureClock"] = json!({"domain":"mach_continuous_time","originUtcMs":1234567890000_i64,"uncertaintyNs":1000000});
    value["microphone"]["clockSegments"] = json!([
        segment(0, 24000, 0, 500010),
        segment(24000, 48000, 500010, 1000020),
    ]);
    value["system"]["clockSegments"] = json!([segment(0, 48000, 5000, 1005100)]);
    value["captureEnd"] = json!({"method":"acquisition_closed","microphoneClosedUs":1000020,"systemClosedUs":1005100,"endUs":1005100});
    crate::storage::recordings::tests::seal_probe_fixture(&output, &mut value);
    write(&output, &value);
    (meeting, session, output, value)
}

pub(super) fn write(output: &Path, value: &Value) {
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

pub(super) fn unpublished(storage: &Storage, meeting: &str, session: &str) {
    assert_eq!(
        storage.get_meeting(meeting).unwrap().unwrap().lifecycle,
        "recording"
    );
    let db = storage.connection.lock().unwrap();
    assert!(matches!(
        load_session(&db, session).unwrap().unwrap().state.as_str(),
        "recording" | "interrupted"
    ));
    for table in [
        "file_assets",
        "audio_tracks",
        "audio_chunks",
        "track_clock_segments",
        "audio_gaps",
        "analysis_flows",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

// The exported Swift fixtures predate a native session. Bind their synthetic
// capture proof to this session before finalizing through the real writer.
fn verify_swift_fixture(
    variable: &str,
    duration: i64,
    utc: i64,
    interrupted: &str,
    gap: (i64, i64),
    frames: (i64, i64),
    samples: &[(&str, u64, u64, f32)],
) {
    let fixtures = PathBuf::from(std::env::var_os(variable).unwrap());
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = saved_probe_fixture(&storage);
    let names = ["manifest.json", "microphone.caf", "system.caf"];
    let originals = names.map(|name| std::fs::read(fixtures.join(name)).unwrap());
    for (name, bytes) in names.iter().zip(&originals) {
        std::fs::write(output.join(name), bytes).unwrap();
    }
    // Check the actual Swift-produced stereo samples before source retirement.
    for &(lane, first, count, expected) in samples {
        let path = output.join(format!("{lane}.caf"));
        let bytes = std::fs::read(&path).unwrap();
        let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
        let mut audio = crate::audio::VerifiedAudio::open(
            std::fs::File::open(path).unwrap(),
            bytes.len() as u64,
            &hash,
        )
        .unwrap();
        for (index, sample) in audio
            .read_frames(first, count)
            .unwrap()
            .chunks_exact(4)
            .enumerate()
        {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if index % 2 == 0 { expected } else { -expected }
            );
        }
    }
    let mut manifest: Value = serde_json::from_slice(&originals[0]).unwrap();
    manifest["recordingOwnerId"] = json!(session);
    let end = |lane: &str| {
        manifest[lane]["clockSegments"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["timelineEndUs"]
            .as_i64()
            .unwrap()
    };
    manifest["captureEnd"] = json!({"method":"acquisition_closed","microphoneClosedUs":end("microphone"),"systemClosedUs":end("system"),"endUs":duration});
    crate::storage::recordings::tests::seal_probe_fixture(&output, &mut manifest);
    write(&output, &manifest);
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, duration);
    assert_eq!(saved.started_at_utc_ms, Some(utc));
    let db = storage.connection.lock().unwrap();
    let source: String = db
        .query_row(
            "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let source: crate::storage::RecordedAudioPlan = serde_json::from_str(&source).unwrap();
    assert_eq!(source.tracks.len(), 2);
    for (kind, count) in [("mic", frames.0), ("system", frames.1)] {
        let track = source.tracks.iter().find(|t| t.kind == kind).unwrap();
        assert_eq!(track.chunks.iter().map(|c| c.frames).sum::<i64>(), count);
    }
    let gaps: Vec<(String, i64, i64, String)> = db.prepare("SELECT t.kind,g.start_us,g.end_us,g.reason_code FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id WHERE t.kind!='processed' ORDER BY t.kind,g.start_us,g.end_us,g.reason_code").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
    let mut expected = vec![(
        interrupted.into(),
        gap.0,
        gap.1,
        "capture_timestamp_gap".into(),
    )];
    for track in &source.tracks {
        if track.start_us > 0 {
            expected.push((
                track.kind.clone(),
                0,
                track.start_us,
                "capture_start_gap".into(),
            ));
        }
        if track.end_us < duration {
            expected.push((
                track.kind.clone(),
                track.end_us,
                duration,
                "capture_stop_gap".into(),
            ));
        }
    }
    expected.sort();
    assert_eq!(gaps, expected);
    drop(db);
    for track in &source.tracks {
        assert!(storage
            .prepare_track_audio(&meeting, &track.id, track.start_us, track.end_us)
            .is_err());
    }
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(
        plan.tracks[0].chunks[0].frames,
        duration * 16000 / 1_000_000
    );
    let mut audio = storage
        .prepare_track_audio(&meeting, &plan.tracks[0].id, 0, duration)
        .unwrap();
    let pcm = audio.read_pcm(0, audio.total_frames()).unwrap();
    // Every Swift input is balanced stereo, so downmixing produces real silence,
    // including the timeline interval where one acquisition lane was missing.
    assert!(pcm
        .chunks_exact(4)
        .all(|b| f32::from_le_bytes(b.try_into().unwrap()) == 0.0));
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(output.join("manifest.json")).unwrap())
            .unwrap(),
        manifest
    );
    assert_eq!(
        names.map(|name| std::fs::read(fixtures.join(name)).unwrap()),
        originals
    );
}

#[test]
#[ignore = "requires actual Swift-generated capture manifest and CAF fixtures; make check-capture-clock"]
fn swift_generated_capture_clock_manifest_finalizes_and_preserves_missing_pcm_time() {
    verify_swift_fixture(
        "MON_CAPTURE_CLOCK_FIXTURE_DIR",
        305000,
        1700000008999,
        "system",
        (55000, 255000),
        (4800, 4800),
        &[
            ("microphone", 0, 4800, 0.125),
            ("system", 0, 2400, 0.25),
            ("system", 2400, 2400, 0.5),
        ],
    );
}

#[test]
#[ignore = "requires MON_MICROPHONE_RECOVERY_FIXTURE_DIR with synthetic Swift recovery CAF files and manifest"]
fn swift_microphone_rebinding_retains_pcm_gap_and_other_track_through_database_finalization() {
    verify_swift_fixture(
        "MON_MICROPHONE_RECOVERY_FIXTURE_DIR",
        40000,
        1700000000000,
        "mic",
        (10000, 30000),
        (960, 1920),
        &[
            ("microphone", 0, 480, 0.25),
            ("microphone", 480, 480, 0.5),
            ("system", 0, 1920, 0.125),
        ],
    );
}

#[test]
#[ignore = "requires synthetic Swift SystemAudioRecorder CAF files; make check-system-recovery"]
fn swift_system_rebinding_retains_pcm_gap_and_continuous_microphone() {
    verify_swift_fixture(
        "MON_SYSTEM_RECOVERY_FIXTURE_DIR",
        40000,
        1700000000000,
        "system",
        (10000, 30000),
        (1920, 960),
        &[
            ("system", 0, 480, 0.25),
            ("system", 480, 480, 0.5),
            ("microphone", 0, 1920, 0.125),
        ],
    );
}

#[test]
fn corrected_capture_finalization_binds_physical_frames_clock_maps_utc_and_pcm_readers() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, value) = corrected(&storage);
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 1005100);
    assert_eq!(saved.started_at_utc_ms, Some(1234567890000));
    let json: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let original: crate::storage::RecordedAudioPlan = serde_json::from_str(&json).unwrap();
    for track in original.tracks {
        let lane = if track.kind == "mic" {
            "microphone"
        } else {
            "system"
        };
        assert_eq!(
            serde_json::to_value(&track.chunks[0].clock_segments).unwrap(),
            value[lane]["clockSegments"]
        );
        assert_eq!(track.chunks[0].frames, 48000);
    }
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].chunks[0].frames, 16082);
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    let started: i64 = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT started_at_utc_ms FROM meetings WHERE id=?1",
            [meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(started, 1234567890000);
}

#[test]
fn corrected_capture_invalid_or_incomplete_clock_proofs_never_publish_or_modify_audio() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, original) = corrected(&storage);
    let audio =
        ["microphone.caf", "system.caf"].map(|name| std::fs::read(output.join(name)).unwrap());
    let mut cases = Vec::new();
    for (key, bad) in [
        ("domain", json!("mach_absolute_time")),
        ("originUtcMs", json!(-1)),
        ("originUtcMs", json!(8640000000000001_i64)),
        ("uncertaintyNs", json!(-1)),
        ("extra", json!(true)),
    ] {
        let mut value = original.clone();
        value["captureClock"][key] = bad;
        cases.push(value);
    }
    for schema in [0, 1, 3] {
        let mut value = original.clone();
        value["schemaVersion"] = json!(schema);
        cases.push(value);
    }
    let mut value = original.clone();
    value.as_object_mut().unwrap().remove("captureClock");
    cases.push(value);
    for key in ["microphone", "system"] {
        for bad in [
            Value::Null,
            json!([]),
            json!([segment(1, 48000, 0, 1000000)]),
            json!([segment(0, 47999, 0, 1000000)]),
            json!([segment(0, 48001, 0, 1000000)]),
            json!([segment(0, 48000, -1, 1000000)]),
            json!([segment(0, 48000, 0, i64::MAX)]),
            json!([segment(0, 48000, 0, 1)]),
            json!(vec![segment(0, 48000, 0, 1000000); 1025]),
        ] {
            let mut value = original.clone();
            value[key]["clockSegments"] = bad;
            cases.push(value);
        }
        for mapping in [
            "{}".to_owned(),
            "{\"schema_version\":2,\"method\":\"linear\"}".to_owned(),
            "x".repeat(4097),
        ] {
            let mut value = original.clone();
            value[key]["clockSegments"][0]["mappingJson"] = json!(mapping);
            cases.push(value);
        }
        let mut value = original.clone();
        value[key].as_object_mut().unwrap().remove("clockSegments");
        cases.push(value);
    }
    let mut value = original.clone();
    value["microphone"]["clockSegments"][0]["timelineStartUs"] = json!(1);
    cases.push(value);
    let mut value = original.clone();
    value["microphone"]["clockSegments"][1]["timelineStartUs"] = json!(500009);
    cases.push(value);
    for (index, value) in cases.iter().enumerate() {
        write(&output, value);
        assert!(
            storage.finalize_recording(&session).is_err(),
            "case {index}"
        );
        unpublished(&storage, &meeting, &session);
        assert_eq!(
            ["microphone.caf", "system.caf"].map(|name| std::fs::read(output.join(name)).unwrap()),
            audio
        );
    }
    write(&output, &original);
    assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
}

fn with_gap(value: &mut Value) {
    value["captureEnd"] = json!({"method":"acquisition_closed","microphoneClosedUs":4000000,"systemClosedUs":4000000,"endUs":4000000});
    value["microphone"]["clockSegments"] = json!([
        segment(0, 24000, 0, 500000),
        segment(24000, 48000, 3500000, 4000000),
    ]);
}

#[test]
fn corrected_capture_gaps_are_saved_as_missing_time_and_only_known_runs_are_playable() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, mut value) = corrected(&storage);
    with_gap(&mut value);
    crate::storage::recordings::tests::seal_probe_fixture(&output, &mut value);
    write(&output, &value);
    assert_eq!(
        storage.finalize_recording(&session).unwrap().duration_us,
        4000000
    );
    let db = storage.connection.lock().unwrap();
    let (track, first, last, reason, recovered): (String, i64, i64, String, i64) = db
        .query_row(
            "SELECT track_id,start_us,end_us,reason_code,recovered FROM audio_gaps",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        (first, last, reason.as_str(), recovered),
        (500000, 3500000, "capture_timestamp_gap", 0)
    );
    drop(db);
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    let mut full = storage
        .prepare_meeting_audio(&crate::storage::meeting_playback::MeetingPlaybackRequest {
            meeting_id: meeting,
            mode: crate::storage::meeting_playback::MeetingPlaybackMode::Mixed,
            start_reference: None,
        })
        .unwrap();
    assert!(full.missing_ranges.is_empty());
    assert_eq!(full.total_frames, 64000);
    assert_eq!(
        full.read_analysis_pcm(16000, 16000).unwrap(),
        vec![0; 16000 * 4]
    );
}

#[test]
fn corrected_capture_clock_or_gap_insert_failure_rolls_back_all_publication_and_can_retry() {
    for table in ["track_clock_segments", "audio_gaps"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, session, output, mut value) = corrected(&storage);
        with_gap(&mut value);
        crate::storage::recordings::tests::seal_probe_fixture(&output, &mut value);
        write(&output, &value);
        storage.connection.lock().unwrap().execute_batch(&format!("CREATE TRIGGER refuse_clock BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'clock insert failure'); END;")).unwrap();
        assert!(storage
            .finalize_recording(&session)
            .unwrap_err()
            .contains("clock insert failure"));
        unpublished(&storage, &meeting, &session);
        assert!(output.join("microphone.caf").is_file());
        assert!(output.join("system.caf").is_file());
        storage
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_clock")
            .unwrap();
        assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
    }
}

#[test]
fn corrected_capture_restart_uses_the_saved_shared_origin_and_retains_bad_clock_outputs() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, original) = corrected(&storage);
    let mut bad = original.clone();
    bad["system"]["clockSegments"][0]["sourceFrameEnd"] = json!(47999);
    write(&output, &bad);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    unpublished(&reopened, &meeting, &session);
    write(&output, &original);
    assert_eq!(
        reopened.recover_completed_recordings().unwrap(),
        vec![session.clone()]
    );
    let saved = load_session(&reopened.connection.lock().unwrap(), &session)
        .unwrap()
        .unwrap();
    assert_eq!(saved.started_at_utc_ms, Some(1234567890000));
    assert_eq!(saved.duration_us, 1005100);
    reopened
        .verify_canonical_audio(&meeting, &|| false)
        .unwrap();
    let json: String = reopened
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let source: crate::storage::RecordedAudioPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(source.tracks[0].chunks[0].clock_segments.len(), 2);
}

#[test]
fn old_capture_manifest_is_rejected_without_inventing_maps() {
    for version in [1, 2, 3] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, session, output, mut value) = saved_probe_fixture(&storage);
        value["schemaVersion"] = json!(version);
        write(&output, &value);
        assert!(storage.finalize_recording(&session).is_err());
        unpublished(&storage, &meeting, &session);
    }
}

#[test]
fn corrected_capture_real_additive_restore_preserves_maps_shared_clock_and_playback() {
    fn source_plan(storage: &Storage, meeting: &str) -> crate::storage::RecordedAudioPlan {
        let json: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
                [meeting],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&json).unwrap()
    }
    let root = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, _, value) = corrected(&storage);
    storage.finalize_recording(&session).unwrap();
    let original = storage.recorded_audio_plan(&meeting).unwrap();
    let original_source = source_plan(&storage, &meeting);
    let archive = output.path().join("clock.monbackup");
    storage
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let neighbor = target.create_meeting("既存会議を保持").unwrap();
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
    assert_eq!(receipt.meeting_ids.len(), 1);
    assert_eq!(target.get_meeting(&neighbor.id).unwrap(), Some(neighbor));
    let restored = target.recorded_audio_plan(&receipt.meeting_ids[0]).unwrap();
    let restored_source = source_plan(&target, &receipt.meeting_ids[0]);
    assert_eq!(original_source.tracks.len(), 2);
    // Retired input IDs belong to the immutable historical snapshot. The
    // canonical playback plan below is the current, rebased source of audio.
    assert_eq!(original_source, restored_source);
    assert!(restored_source.tracks.iter().all(|track| track
        .chunks
        .iter()
        .all(|chunk| !chunk.clock_segments.is_empty())));
    for (before, after) in original.tracks.iter().zip(&restored.tracks) {
        assert_ne!(before.id, after.id);
        assert_ne!(before.chunks[0].asset_id, after.chunks[0].asset_id);
        assert_eq!(before.chunks[0].hash, after.chunks[0].hash);
        assert_eq!(
            before.chunks[0].clock_segments,
            after.chunks[0].clock_segments
        );
        let mut audio = target
            .prepare_track_audio(
                &receipt.meeting_ids[0],
                &after.id,
                after.start_us,
                after.end_us,
            )
            .unwrap();
        assert_eq!(audio.total_frames(), before.total_frames().unwrap());
        assert_eq!(
            audio.read_pcm(0, 32).unwrap().len(),
            32 * after.channels as usize * 4
        );
    }
    let db = target.connection.lock().unwrap();
    let path: String = db
        .query_row(
            "SELECT f.relative_path FROM file_assets f JOIN recording_sessions r ON r.meeting_id=f.meeting_id AND r.audio_manifest_hash=f.content_hash WHERE r.meeting_id=?1",
            params![receipt.meeting_ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    let config: String = db
        .query_row(
            "SELECT processing_config_json FROM audio_tracks WHERE meeting_id=?1 AND kind='mic'",
            params![receipt.meeting_ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(target.root.join(path)).unwrap()).unwrap();
    assert_eq!(manifest["captureClock"], value["captureClock"]);
    assert_eq!(
        manifest["microphone"]["clockSegments"],
        value["microphone"]["clockSegments"]
    );
    assert_eq!(
        serde_json::from_str::<Value>(&config).unwrap()["capture_clock"],
        value["captureClock"]
    );
}
