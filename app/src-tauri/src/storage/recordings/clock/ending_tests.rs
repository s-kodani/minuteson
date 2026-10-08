use super::tests::{corrected, unpublished, write};
use crate::playback::PlaybackStore;
use crate::storage::meeting_playback::{MeetingPlaybackMode, MeetingPlaybackRequest};
use crate::storage::recordings::{load_session, RecordingConfig};
use crate::storage::Storage;
use rusqlite::params;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn assert_whole_gap_playback(storage: &Storage, meeting: &str, end: i64) {
    let store = PlaybackStore::default();
    for (mode, kind) in [
        (MeetingPlaybackMode::Mic, "mic"),
        (MeetingPlaybackMode::System, "system"),
        (MeetingPlaybackMode::Both, "both"),
    ] {
        let info = store
            .insert_meeting(
                storage
                    .prepare_meeting_audio(&MeetingPlaybackRequest {
                        meeting_id: meeting.into(),
                        mode,
                    })
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            (
                info.start_us,
                info.end_us,
                info.track_kind.as_str(),
                info.alignment.as_str()
            ),
            (0, end, kind, "unverified")
        );
        assert_eq!(
            info.total_frames,
            crate::audio_clock::frame_at(end, 48000).unwrap()
        );
        assert!(!info.missing_ranges.is_empty());
        for gap in &info.missing_ranges {
            assert!(store.read(&info.token, gap.start_frame).is_err());
        }
        let mut first = 0;
        for gap in &info.missing_ranges {
            if gap.start_frame <= first && first < gap.end_frame {
                first = gap.end_frame;
            }
        }
        let packet = store.read(&info.token, first).unwrap();
        let count = u32::from_le_bytes(packet[8..12].try_into().unwrap()) as u64;
        assert!(count > 0);
        assert!(info
            .missing_ranges
            .iter()
            .all(|gap| first + count <= gap.start_frame || first >= gap.end_frame));
    }
}

fn ending(storage: &Storage) -> (String, String, PathBuf, Value) {
    let (meeting, session, output, mut value) = corrected(storage);
    value["schemaVersion"] = json!(3);
    value["recordingOwnerId"] = json!(session);
    value["captureEnd"] = json!({"method":"acquisition_closed","microphoneClosedUs":1050000,"systemClosedUs":1100000,"endUs":1100000});
    write(&output, &value);
    (meeting, session, output, value)
}

fn gaps(storage: &Storage) -> Vec<(String, i64, i64, String, bool)> {
    storage.connection.lock().unwrap().prepare("SELECT t.kind,g.start_us,g.end_us,g.reason_code,g.recovered FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id ORDER BY t.kind,g.start_us,g.end_us").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap()
        .collect::<Result<_,_>>().unwrap()
}

fn originals(output: &Path) -> [Vec<u8>; 3] {
    ["manifest.json", "microphone.caf", "system.caf"]
        .map(|name| std::fs::read(output.join(name)).unwrap())
}

fn meeting_duration(storage: &Storage, meeting: &str) -> i64 {
    storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT duration_us FROM meetings WHERE id=?1",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn capture_ending_preserves_physical_pcm_boundary_gaps_actual_end_and_commit_time_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = ending(&storage);
    let before = originals(&output);
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.duration_us, 1100000);
    assert_eq!(saved.ended_at_utc_ms, Some(1234567891100));
    let expected = vec![
        (
            "mic".into(),
            1000020,
            1100000,
            "capture_stop_gap".into(),
            false,
        ),
        ("system".into(), 0, 5000, "capture_start_gap".into(), false),
        (
            "system".into(),
            1005100,
            1100000,
            "capture_stop_gap".into(),
            false,
        ),
    ];
    assert_eq!(gaps(&storage), expected);
    assert_whole_gap_playback(&storage, &meeting, 1100000);
    let committed: i64 = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT updated_at FROM recording_sessions WHERE id=?1",
            params![session],
            |r| r.get(0),
        )
        .unwrap();
    assert!(committed > saved.ended_at_utc_ms.unwrap());
    assert_eq!(meeting_duration(&storage, &meeting), 1100000);
    // Boundary gaps are outside the fixed analysis input, not appended silence.
    let plan = storage.recorded_audio_plan(&meeting).unwrap();
    for track in plan.tracks {
        assert_eq!(track.chunks[0].frames, 48000);
        assert!(track.end_us < saved.duration_us);
        let mut known = storage
            .prepare_track_audio(&meeting, &track.id, track.start_us, track.end_us)
            .unwrap();
        assert_eq!(
            known.read_pcm(0, 32).unwrap().len(),
            32 * track.channels as usize * 4
        );
        assert!(storage
            .prepare_track_audio(&meeting, &track.id, track.start_us, saved.duration_us)
            .is_err());
        assert!(storage
            .prepare_track_audio(&meeting, &track.id, track.end_us, saved.duration_us)
            .is_err());
    }
    assert_eq!(originals(&output), before);
    assert!(storage.finalize_recording(&session).is_err());
    assert_eq!(
        load_session(&storage.connection.lock().unwrap(), &session)
            .unwrap()
            .unwrap(),
        saved
    );
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    assert!(reopened.finalize_recording(&session).is_err());
    assert_eq!(
        load_session(&reopened.connection.lock().unwrap(), &session)
            .unwrap()
            .unwrap(),
        saved
    );
    assert_eq!(gaps(&reopened), expected);
    assert_whole_gap_playback(&reopened, &meeting, 1100000);
    let after: i64 = reopened
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT updated_at FROM recording_sessions WHERE id=?1",
            params![session],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, committed);
    assert_eq!(originals(&output), before);
}

#[test]
fn capture_ending_rejects_missing_foreign_reversed_or_unbounded_proofs_without_publication() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, good) = ending(&storage);
    let audio = originals(&output);
    let mut cases = Vec::new();
    for field in ["captureEnd", "recordingOwnerId", "captureClock"] {
        let mut value = good.clone();
        value.as_object_mut().unwrap().remove(field);
        cases.push(value);
    }
    for owner in [
        Value::Null,
        json!("foreign"),
        json!(uuid::Uuid::new_v4().to_string()),
    ] {
        let mut value = good.clone();
        value["recordingOwnerId"] = owner;
        cases.push(value);
    }
    for (field, bad) in [
        ("method", json!("wall_clock")),
        ("microphoneClosedUs", json!(1000019)),
        ("systemClosedUs", json!(1005099)),
        ("microphoneClosedUs", json!(-1)),
        ("systemClosedUs", Value::Null),
        ("endUs", json!(0)),
        ("endUs", json!(1099999)),
        ("endUs", json!(1100001)),
        ("endUs", json!(9007199254740992_i64)),
        ("extra", json!(true)),
    ] {
        let mut value = good.clone();
        value["captureEnd"][field] = bad;
        cases.push(value);
    }
    for field in ["method", "microphoneClosedUs", "systemClosedUs", "endUs"] {
        let mut value = good.clone();
        value["captureEnd"].as_object_mut().unwrap().remove(field);
        cases.push(value);
    }
    let mut overflow = good.clone();
    overflow["captureClock"]["originUtcMs"] = json!(8640000000000000_i64);
    cases.push(overflow);
    for (index, value) in cases.iter().enumerate() {
        write(&output, value);
        assert!(
            storage.finalize_recording(&session).is_err(),
            "case {index}"
        );
        unpublished(&storage, &meeting, &session);
        let current = originals(&output);
        assert_eq!((&current[1], &current[2]), (&audio[1], &audio[2]));
    }
    write(&output, &good);
    assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
}

#[test]
fn capture_ending_never_upgrades_legacy_manifests_from_wall_time_other_track_or_supplied_end() {
    for schema in [1, 2] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, session, output, mut value) = ending(&storage);
        value["schemaVersion"] = json!(schema);
        if schema == 1 {
            value.as_object_mut().unwrap().remove("captureClock");
            for input in ["microphone", "system"] {
                value[input]
                    .as_object_mut()
                    .unwrap()
                    .remove("clockSegments");
            }
        }
        write(&output, &value);
        assert!(storage.finalize_recording(&session).is_err());
        unpublished(&storage, &meeting, &session);
        value.as_object_mut().unwrap().remove("captureEnd");
        value["endedAtUTC"] = json!("2099-01-01T00:00:00Z");
        write(&output, &value);
        let saved = storage.finalize_recording(&session).unwrap();
        assert_eq!(
            saved.duration_us,
            if schema == 1 { 1000000 } else { 1005100 }
        );
        assert!(gaps(&storage).is_empty());
    }
}

#[test]
fn capture_ending_interior_and_boundary_gaps_coexist_without_padding_known_runs() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, mut value) = ending(&storage);
    value["microphone"]["clockSegments"][1]["timelineStartUs"] = json!(700010);
    value["microphone"]["clockSegments"][1]["timelineEndUs"] = json!(1200020);
    value["captureEnd"]["microphoneClosedUs"] = json!(1250000);
    value["captureEnd"]["systemClosedUs"] = json!(1300000);
    value["captureEnd"]["endUs"] = json!(1300000);
    write(&output, &value);
    assert_eq!(
        storage.finalize_recording(&session).unwrap().duration_us,
        1300000
    );
    assert_eq!(
        gaps(&storage),
        vec![
            (
                "mic".into(),
                500010,
                700010,
                "capture_timestamp_gap".into(),
                false
            ),
            (
                "mic".into(),
                1200020,
                1300000,
                "capture_stop_gap".into(),
                false
            ),
            ("system".into(), 0, 5000, "capture_start_gap".into(), false),
            (
                "system".into(),
                1005100,
                1300000,
                "capture_stop_gap".into(),
                false
            ),
        ]
    );
    let mic: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT id FROM audio_tracks WHERE kind='mic'", [], |r| {
            r.get(0)
        })
        .unwrap();
    for (first, last) in [(0, 500010), (700010, 1200020)] {
        let mut audio = storage
            .prepare_track_audio(&meeting, &mic, first, last)
            .unwrap();
        assert!(!audio.read_pcm(0, 32).unwrap().is_empty());
    }
    assert!(storage
        .prepare_track_audio(&meeting, &mic, 0, 1300000)
        .is_err());
    assert_whole_gap_playback(&storage, &meeting, 1300000);
}

#[test]
fn capture_ending_boundary_gap_insert_failure_rolls_back_assets_session_and_analysis_flow() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = ending(&storage);
    let before = originals(&output);
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER refuse_end BEFORE INSERT ON audio_gaps WHEN NEW.reason_code='capture_stop_gap' BEGIN SELECT RAISE(ABORT,'end insert failure'); END;").unwrap();
    assert!(storage
        .finalize_recording(&session)
        .unwrap_err()
        .contains("end insert failure"));
    unpublished(&storage, &meeting, &session);
    assert_eq!(originals(&output), before);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_end")
        .unwrap();
    assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
    assert_eq!(gaps(&storage).len(), 3);
}

#[test]
fn capture_ending_does_not_salvage_failed_empty_or_corrupt_capture_as_saved() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, good) = ending(&storage);
    for (input, field, bad) in [
        ("", "state", json!("failed")),
        ("", "errors", json!(["stop failed"])),
        ("microphone", "frames", json!(0)),
        ("system", "appendFailures", json!(1)),
    ] {
        let mut value = good.clone();
        if input.is_empty() {
            value[field] = bad;
        } else {
            value[input][field] = bad;
        }
        write(&output, &value);
        assert!(storage.finalize_recording(&session).is_err());
        unpublished(&storage, &meeting, &session);
    }
    write(&output, &good);
    let path = output.join("system.caf");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..40]).unwrap();
    assert!(storage.finalize_recording(&session).is_err());
    unpublished(&storage, &meeting, &session);
    assert_eq!(std::fs::read(&path).unwrap(), bytes[..40]);
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
}

#[test]
fn capture_ending_additive_restore_preserves_duration_gaps_and_known_pcm_without_owner_rewrite() {
    let root = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, _, value) = ending(&storage);
    storage.finalize_recording(&session).unwrap();
    let archive = destination.path().join("ending.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let prepared = target
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let confirmation = prepared.preview().confirmation_id;
    let receipt = prepared
        .publish(&target, &confirmation, &|| false, &|_| {})
        .unwrap();
    let restored = &receipt.meeting_ids[0];
    assert_ne!(restored, &meeting);
    assert_eq!(meeting_duration(&target, restored), 1100000);
    assert_eq!(gaps(&target), gaps(&storage));
    assert_whole_gap_playback(&target, restored, 1100000);
    let plan = target.recorded_audio_plan(restored).unwrap();
    for track in &plan.tracks {
        let mut audio = target
            .prepare_track_audio(restored, &track.id, track.start_us, track.end_us)
            .unwrap();
        assert!(!audio.read_pcm(0, 32).unwrap().is_empty());
        assert!(target
            .prepare_track_audio(restored, &track.id, track.end_us, 1100000)
            .is_err());
    }
    let db = target.connection.lock().unwrap();
    let relative: String = db
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            params![plan.manifest_asset_id],
            |r| r.get(0),
        )
        .unwrap();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(target.root.join(relative)).unwrap()).unwrap();
    assert_eq!(manifest["captureEnd"], value["captureEnd"]);
    assert_eq!(manifest["recordingOwnerId"], json!(session));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "runs synthetic Swift capture directly into the native-owned output; make check-capture-ending"]
fn swift_generated_capture_ending_owner_and_physical_pcm_finalize_without_rewriting_proofs() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage
        .create_meeting("synthetic acquisition closure integration")
        .unwrap();
    let session = storage
        .prepare_recording(
            &meeting.id,
            &RecordingConfig {
                mode: "system".into(),
                backend: "screencapturekit".into(),
                target_pids: vec![],
                microphone_device_uid: None,
                target_application_identity: None,
            },
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    storage.mark_recording_started(&session.id).unwrap();
    let output = storage
        .root
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    std::fs::create_dir_all(&output).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let generated=std::process::Command::new("/usr/bin/xcrun")
        .args(["swift","test","--package-path"]).arg(repo.join("probes/macos-capture"))
        .args(["--disable-sandbox","--filter","SystemAudioRecoveryTests/testInterruptedStopRecordingExportsOwnerBoundEndAndUnchangedPhysicalCAF"])
        .env("SWIFTPM_MODULECACHE_OVERRIDE","/tmp/mon-swift-module-cache")
        .env("CLANG_MODULE_CACHE_PATH","/tmp/mon-clang-module-cache")
        .env("MON_CAPTURE_END_FIXTURE_DIR",&output).env("MON_CAPTURE_END_FIXTURE_OWNER",&session.id)
        .output().unwrap();
    assert!(
        generated.status.success(),
        "Swift synthetic generation failed: {}\n{}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    let before = originals(&output);
    let manifest: Value = serde_json::from_slice(&before[0]).unwrap();
    assert_eq!(manifest["schemaVersion"], 3);
    assert_eq!(manifest["recordingOwnerId"], session.id);
    assert_eq!(manifest["system"]["systemRecovery"]["interruptions"], 1);
    let saved = storage.finalize_recording(&session.id).unwrap();
    assert_eq!(saved.duration_us, 50000);
    // Shared origin is floor-rounded 10 ns before the fixed UTC reference.
    assert_eq!(manifest["captureClock"]["originUtcMs"], 1699999999999_i64);
    assert_eq!(saved.ended_at_utc_ms, Some(1700000000049));
    assert_eq!(
        gaps(&storage),
        vec![
            ("mic".into(), 10000, 50000, "capture_stop_gap".into(), false),
            ("system".into(), 0, 5000, "capture_start_gap".into(), false),
            (
                "system".into(),
                15000,
                50000,
                "capture_stop_gap".into(),
                false
            ),
        ]
    );
    for track in storage.recorded_audio_plan(&meeting.id).unwrap().tracks {
        assert_eq!(track.chunks[0].frames, 480);
        let mut audio = storage
            .prepare_track_audio(&meeting.id, &track.id, track.start_us, track.end_us)
            .unwrap();
        assert_eq!(audio.total_frames(), 480);
        let value = if track.kind == "mic" {
            0.125_f32
        } else {
            0.25_f32
        };
        for (index, sample) in audio.read_pcm(0, 480).unwrap().chunks_exact(4).enumerate() {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if index % 2 == 0 { value } else { value / 2.0 }
            );
        }
        assert!(storage
            .prepare_track_audio(&meeting.id, &track.id, track.end_us, 50000)
            .is_err());
    }
    assert_eq!(originals(&output), before);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        load_session(&reopened.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap(),
        saved
    );
    assert!(reopened.finalize_recording(&session.id).is_err());
    assert_whole_gap_playback(&reopened, &meeting.id, 50000);
    let store = PlaybackStore::default();
    for (mode, first, count, value) in [
        (MeetingPlaybackMode::Mic, 0, 480, 0.09375_f32),
        (MeetingPlaybackMode::System, 240, 480, 0.1875),
        (MeetingPlaybackMode::Both, 240, 240, 0.140625),
    ] {
        let info = store
            .insert_meeting(
                reopened
                    .prepare_meeting_audio(&MeetingPlaybackRequest {
                        meeting_id: meeting.id.clone(),
                        mode,
                    })
                    .unwrap(),
            )
            .unwrap();
        let packet = store.read(&info.token, first).unwrap();
        assert_eq!(u32::from_le_bytes(packet[8..12].try_into().unwrap()), count);
        // Stereo input is averaged to mono before constant two-input headroom.
        // Unequal, same-sign channels ensure silence or a dropped channel fails.
        assert_eq!(info.channels, 1);
        assert_eq!(packet.len(), 12 + count as usize * 4);
        for sample in packet[12..].chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), value);
        }
    }
    assert_eq!(originals(&output), before);
}
