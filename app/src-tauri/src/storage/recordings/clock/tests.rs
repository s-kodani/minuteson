use super::*;
use crate::storage::recordings::{load_session, tests::saved_probe_fixture};
use crate::storage::Storage;
use serde_json::{json, Value};
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

#[test]
#[ignore = "requires actual Swift-generated capture manifest and CAF fixtures; make check-capture-clock"]
fn swift_generated_capture_clock_manifest_finalizes_and_preserves_missing_pcm_time() {
    let fixtures = PathBuf::from(
        std::env::var_os("MON_CAPTURE_CLOCK_FIXTURE_DIR")
            .expect("run through make check-capture-clock"),
    );
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = saved_probe_fixture(&storage);
    let names = ["manifest.json", "microphone.caf", "system.caf"];
    let originals = names.map(|name| std::fs::read(fixtures.join(name)).unwrap());
    for (name, bytes) in names.iter().zip(&originals) {
        // Only replace this test's synthetic, unpublished session files.
        std::fs::write(output.join(name), bytes).unwrap();
    }
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 305000);
    assert_eq!(saved.started_at_utc_ms, Some(1700000008999));
    let db = storage.connection.lock().unwrap();
    let (system_track, start, end, reason): (String, i64, i64, String) = db
        .query_row(
            "SELECT track_id,start_us,end_us,reason_code FROM audio_gaps",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (start, end, reason.as_str()),
        (55000, 255000, "capture_timestamp_gap")
    );
    let microphone: String = db
        .query_row("SELECT id FROM audio_tracks WHERE kind='mic'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let physical: Vec<i64> = db
        .prepare("SELECT frame_count FROM audio_chunks ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(physical, vec![4800, 4800]);
    drop(db);
    for (track, first, last, value) in [
        (&microphone, 0, 100000, 0.125_f32),
        (&system_track, 5000, 55000, 0.25),
        (&system_track, 255000, 305000, 0.5),
    ] {
        let mut audio = storage
            .prepare_track_audio(&meeting, track, first, last)
            .unwrap();
        assert_eq!(audio.alignment, "unverified");
        assert_eq!(
            audio.total_frames(),
            if track == &microphone { 4800 } else { 2400 }
        );
        let pcm = audio.read_pcm(0, audio.total_frames()).unwrap();
        for (index, sample) in pcm.chunks_exact(4).enumerate() {
            let actual = f32::from_le_bytes(sample.try_into().unwrap());
            assert_eq!(actual, if index % 2 == 0 { value } else { -value });
        }
    }
    assert!(storage
        .prepare_track_audio(&meeting, &system_track, 5000, 305000)
        .is_err());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
    assert_eq!(
        names.map(|name| std::fs::read(output.join(name)).unwrap()),
        originals
    );
    assert_eq!(
        names.map(|name| std::fs::read(fixtures.join(name)).unwrap()),
        originals
    );
}

#[test]
#[ignore = "requires MON_MICROPHONE_RECOVERY_FIXTURE_DIR with synthetic Swift recovery CAF files and manifest"]
fn swift_microphone_rebinding_retains_pcm_gap_and_other_track_through_database_finalization() {
    let fixtures = PathBuf::from(std::env::var_os("MON_MICROPHONE_RECOVERY_FIXTURE_DIR").unwrap());
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = saved_probe_fixture(&storage);
    let names = ["manifest.json", "microphone.caf", "system.caf"];
    let originals = names.map(|name| std::fs::read(fixtures.join(name)).unwrap());
    for (name, bytes) in names.iter().zip(&originals) {
        std::fs::write(output.join(name), bytes).unwrap();
    }
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 40000);
    assert_eq!(saved.started_at_utc_ms, Some(1700000000000));
    let db = storage.connection.lock().unwrap();
    let mic: String = db
        .query_row("SELECT id FROM audio_tracks WHERE kind='mic'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let system: String = db
        .query_row("SELECT id FROM audio_tracks WHERE kind='system'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let gaps: Vec<(String, i64, i64, String)> = db
        .prepare("SELECT track_id,start_us,end_us,reason_code FROM audio_gaps")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        gaps,
        vec![(mic.clone(), 10000, 30000, "capture_timestamp_gap".into())]
    );
    let frames: Vec<i64> = db
        .prepare("SELECT frame_count FROM audio_chunks ORDER BY frame_count")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(frames, vec![960, 1920]);
    drop(db);
    for (track, start, end, count, value) in [
        (&mic, 0, 10000, 480, 0.25_f32),
        (&mic, 30000, 40000, 480, 0.5),
        (&system, 0, 40000, 1920, 0.125),
    ] {
        let mut audio = storage
            .prepare_track_audio(&meeting, track, start, end)
            .unwrap();
        assert_eq!(audio.total_frames(), count);
        assert_eq!(audio.alignment, "unverified");
        for (index, sample) in audio
            .read_pcm(0, count)
            .unwrap()
            .chunks_exact(4)
            .enumerate()
        {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if index % 2 == 0 { value } else { -value }
            );
        }
    }
    assert!(storage
        .prepare_track_audio(&meeting, &mic, 0, 40000)
        .is_err());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
    assert_eq!(
        names.map(|name| std::fs::read(output.join(name)).unwrap()),
        originals
    );
    assert_eq!(
        names.map(|name| std::fs::read(fixtures.join(name)).unwrap()),
        originals
    );
}

#[test]
#[ignore = "requires synthetic Swift SystemAudioRecorder CAF files; make check-system-recovery"]
fn swift_system_rebinding_retains_pcm_gap_and_continuous_microphone() {
    let fixtures = PathBuf::from(std::env::var_os("MON_SYSTEM_RECOVERY_FIXTURE_DIR").unwrap());
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = saved_probe_fixture(&storage);
    let names = ["manifest.json", "microphone.caf", "system.caf"];
    let originals = names.map(|name| std::fs::read(fixtures.join(name)).unwrap());
    let manifest: Value = serde_json::from_slice(&originals[0]).unwrap();
    assert_eq!(manifest["system"]["systemRecovery"]["state"], "stopped");
    assert_eq!(manifest["system"]["systemRecovery"]["sourceEpoch"], 2);
    assert_eq!(manifest["system"]["systemRecovery"]["interruptions"], 1);
    assert_eq!(manifest["system"]["appendFailures"], 0);
    for (name, bytes) in names.iter().zip(&originals) {
        std::fs::write(output.join(name), bytes).unwrap();
    }
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 40000);
    assert_eq!(saved.started_at_utc_ms, Some(1700000000000));
    let db = storage.connection.lock().unwrap();
    let mic: String = db
        .query_row("SELECT id FROM audio_tracks WHERE kind='mic'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let system: String = db
        .query_row("SELECT id FROM audio_tracks WHERE kind='system'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let gaps: Vec<(String, i64, i64, String)> = db
        .prepare("SELECT track_id,start_us,end_us,reason_code FROM audio_gaps")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        gaps,
        vec![(system.clone(), 10000, 30000, "capture_timestamp_gap".into())]
    );
    let frames: Vec<i64> = db
        .prepare("SELECT frame_count FROM audio_chunks ORDER BY frame_count")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(frames, vec![960, 1920]);
    drop(db);
    for (track, start, end, count, value) in [
        (&system, 0, 10000, 480, 0.25_f32),
        (&system, 30000, 40000, 480, 0.5),
        (&mic, 0, 40000, 1920, 0.125),
    ] {
        let mut audio = storage
            .prepare_track_audio(&meeting, track, start, end)
            .unwrap();
        assert_eq!(audio.total_frames(), count);
        assert_eq!(audio.alignment, "unverified");
        for (index, sample) in audio
            .read_pcm(0, count)
            .unwrap()
            .chunks_exact(4)
            .enumerate()
        {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if index % 2 == 0 { value } else { -value }
            );
        }
    }
    assert!(storage
        .prepare_track_audio(&meeting, &system, 0, 40000)
        .is_err());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
    assert_eq!(
        names.map(|name| std::fs::read(output.join(name)).unwrap()),
        originals
    );
    assert_eq!(
        names.map(|name| std::fs::read(fixtures.join(name)).unwrap()),
        originals
    );
}

#[test]
fn corrected_capture_finalization_binds_physical_frames_clock_maps_utc_and_pcm_readers() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, value) = corrected(&storage);
    let original =
        ["microphone.caf", "system.caf"].map(|name| std::fs::read(output.join(name)).unwrap());
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 1005100);
    assert_eq!(saved.started_at_utc_ms, Some(1234567890000));
    let plan = storage.recorded_audio_plan(&meeting).unwrap();
    assert_eq!(plan.tracks.len(), 2);
    for track in &plan.tracks {
        let key = if track.kind == "mic" {
            "microphone"
        } else {
            "system"
        };
        assert_eq!(
            serde_json::to_value(&track.chunks[0].clock_segments).unwrap(),
            value[key]["clockSegments"]
        );
        assert_eq!(track.chunks[0].frames, 48000);
        assert_eq!(
            track.total_frames().unwrap(),
            if track.kind == "mic" { 48001 } else { 48005 }
        );
        let mut audio = storage
            .prepare_track_audio(&meeting, &track.id, track.start_us, track.end_us)
            .unwrap();
        assert_eq!(audio.alignment, "unverified");
        assert_eq!(audio.total_frames(), track.total_frames().unwrap());
        assert_eq!(
            audio.read_pcm(0, 32).unwrap().len(),
            32 * track.channels as usize * 4
        );
        assert!(storage.verify_asset(&track.chunks[0].asset_id).unwrap());
        let config: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT processing_config_json FROM audio_tracks WHERE id=?1",
                params![track.id],
                |r| r.get(0),
            )
            .unwrap();
        let config: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["timeline_alignment"], "unverified");
        assert_eq!(config["capture_clock"], value["captureClock"]);
    }
    assert_eq!(
        ["microphone.caf", "system.caf"].map(|name| std::fs::read(output.join(name)).unwrap()),
        original
    );
    let started: i64 = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT started_at_utc_ms FROM meetings WHERE id=?1",
            params![meeting],
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
    assert!(storage.recorded_audio_plan(&meeting).is_err());
    assert!(storage
        .prepare_track_audio(&meeting, &track, 0, 4000000)
        .is_err());
    assert!(storage
        .prepare_track_audio(&meeting, &track, 499999, 3500001)
        .is_err());
    for (start, end) in [(0, 500000), (3500000, 4000000)] {
        let mut audio = storage
            .prepare_track_audio(&meeting, &track, start, end)
            .unwrap();
        assert_eq!(audio.total_frames(), 24000);
        assert_eq!(audio.read_pcm(0, 32).unwrap().len(), 128);
    }
    let both = storage
        .prepare_meeting_audio(&crate::storage::meeting_playback::MeetingPlaybackRequest {
            start_reference: None,
            meeting_id: meeting.clone(),
            mode: crate::storage::meeting_playback::MeetingPlaybackMode::Both,
        })
        .unwrap();
    // The current closure also proves the system lane's leading/trailing loss.
    assert_eq!(both.missing_ranges.len(), 3);
    assert_eq!(
        both.missing_ranges
            .iter()
            .filter(|gap| gap.track_kind == "system")
            .count(),
        2
    );
    let store = crate::playback::PlaybackStore::default();
    let audio = storage
        .prepare_meeting_audio(&crate::storage::meeting_playback::MeetingPlaybackRequest {
            start_reference: None,
            meeting_id: meeting.clone(),
            mode: crate::storage::meeting_playback::MeetingPlaybackMode::Mic,
        })
        .unwrap();
    let info = store.insert_meeting(audio).unwrap();
    assert_eq!(
        (info.start_us, info.end_us, info.total_frames),
        (0, 4000000, 192000)
    );
    assert_eq!(info.missing_ranges.len(), 1);
    let gap = &info.missing_ranges[0];
    assert_eq!(
        (gap.track_kind.as_str(), gap.start_frame, gap.end_frame),
        ("mic", 24000, 168000)
    );
    let known = store.read(&info.token, 0).unwrap();
    assert_eq!(u32::from_le_bytes(known[8..12].try_into().unwrap()), 24000);
    for offset in [24000, 100000, 167999] {
        assert!(store.read(&info.token, offset).is_err());
    }
    let after = store.read(&info.token, 168000).unwrap();
    assert_eq!(u32::from_le_bytes(after[8..12].try_into().unwrap()), 24000);
    assert!(storage.recorded_audio_plan(&meeting).is_err());
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
    assert_eq!(
        reopened.recorded_audio_plan(&meeting).unwrap().tracks[0].chunks[0]
            .clock_segments
            .len(),
        2
    );
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
    let root = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, _, value) = corrected(&storage);
    storage.finalize_recording(&session).unwrap();
    let original = storage.recorded_audio_plan(&meeting).unwrap();
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
            "SELECT relative_path FROM file_assets WHERE id=?1",
            params![restored.manifest_asset_id],
            |r| r.get(0),
        )
        .unwrap();
    let config: String = db
        .query_row(
            "SELECT processing_config_json FROM audio_tracks WHERE meeting_id=?1 LIMIT 1",
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
