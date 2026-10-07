use super::*;
use crate::storage::recordings::{load_session, tests::saved_probe_fixture};
use crate::storage::Storage;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn segment(first: i64, last: i64, start: i64, end: i64) -> Value {
    json!({"sourceFrameStart":first,"sourceFrameEnd":last,"timelineStartUs":start,"timelineEndUs":end,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"})
}

fn corrected(storage: &Storage) -> (String, String, PathBuf, Value) {
    let (meeting, session, output, mut value) = saved_probe_fixture(storage);
    value["schemaVersion"] = json!(2);
    value["captureClock"] = json!({"domain":"mach_continuous_time","originUtcMs":1234567890000_i64,"uncertaintyNs":1000000});
    value["microphone"]["clockSegments"] = json!([
        segment(0, 24000, 0, 500010),
        segment(24000, 48000, 500010, 1000020),
    ]);
    value["system"]["clockSegments"] = json!([segment(0, 48000, 5000, 1005100)]);
    write(&output, &value);
    (meeting, session, output, value)
}

fn write(output: &Path, value: &Value) {
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

fn unpublished(storage: &Storage, meeting: &str, session: &str) {
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
    assert!(storage
        .prepare_meeting_audio(&crate::storage::meeting_playback::MeetingPlaybackRequest {
            meeting_id: meeting,
            mode: crate::storage::meeting_playback::MeetingPlaybackMode::Both
        })
        .is_err());
}

#[test]
fn corrected_capture_clock_or_gap_insert_failure_rolls_back_all_publication_and_can_retry() {
    for table in ["track_clock_segments", "audio_gaps"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, session, output, mut value) = corrected(&storage);
        with_gap(&mut value);
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
fn corrected_capture_legacy_manifest_stays_nominal_and_unverified_without_invented_maps() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, _, _) = saved_probe_fixture(&storage);
    let before = load_session(&storage.connection.lock().unwrap(), &session)
        .unwrap()
        .unwrap()
        .started_at_utc_ms;
    assert_eq!(
        storage
            .finalize_recording(&session)
            .unwrap()
            .started_at_utc_ms,
        before
    );
    for track in storage.recorded_audio_plan(&meeting).unwrap().tracks {
        assert_eq!((track.start_us, track.end_us), (0, 1000000));
        assert!(track.chunks[0].clock_segments.is_empty());
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
        assert_eq!(config, "{\"timeline_alignment\":\"unverified\"}");
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
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let neighbor = target.create_meeting("既存会議を保持").unwrap();
    let prepared = target
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let confirmation = prepared.preview().confirmation_id;
    let receipt = prepared
        .publish(&target, &confirmation, &|| false, &|_| {})
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
