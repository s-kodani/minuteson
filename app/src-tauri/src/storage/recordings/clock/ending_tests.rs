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
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&MeetingPlaybackRequest {
                    meeting_id: meeting.into(),
                    mode: MeetingPlaybackMode::Mixed,
                    start_reference: None,
                })
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        (info.start_us, info.end_us, info.track_kind.as_str()),
        (0, end, "mixed")
    );
    assert_eq!(
        info.total_frames,
        crate::audio_clock::frame_at(end, 16000).unwrap()
    );
    assert!(info.missing_ranges.is_empty());
    assert!(store.read(&info.token, 0).is_ok());
}

fn ending(storage: &Storage) -> (String, String, PathBuf, Value) {
    let (meeting, session, output, mut value) = corrected(storage);
    value["schemaVersion"] = json!(4);
    value["recordingOwnerId"] = json!(session);
    value["captureEnd"] = json!({"method":"acquisition_closed","microphoneClosedUs":1050000,"systemClosedUs":1100000,"endUs":1100000});
    write(&output, &value);
    (meeting, session, output, value)
}

fn gaps(storage: &Storage) -> Vec<(String, i64, i64, String, bool)> {
    storage.connection.lock().unwrap().prepare("SELECT t.kind,g.start_us,g.end_us,g.reason_code,g.recovered FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id WHERE t.kind!='processed' ORDER BY t.kind,g.start_us,g.end_us").unwrap()
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

#[cfg(target_os = "macos")]
#[test]
#[ignore = "actual Swift PC-only fixture, no capture devices"]
fn swift_generated_pc_only_capture_publishes_wave_without_inventing_microphone_audio() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("PC-only synthetic capture").unwrap();
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
    let output = root
        .path()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    std::fs::create_dir_all(&output).unwrap();
    let generated = std::process::Command::new("swift")
        .args(["test", "--package-path", concat!(env!("CARGO_MANIFEST_DIR"), "/../../probes/macos-capture"), "--disable-sandbox", "--filter", "CaptureClockFixtureTests/testPCOnlyRecordingExportsActualCAFEmptyMicrophoneProofAndAcquisitionClosures"])
        .env("SWIFTPM_MODULECACHE_OVERRIDE", "/tmp/mon-swift-module-cache")
        .env("CLANG_MODULE_CACHE_PATH", "/tmp/mon-clang-module-cache")
        .env("MON_PC_ONLY_FIXTURE_DIR", &output).env("MON_PC_ONLY_FIXTURE_OWNER", &session.id)
        .output().unwrap();
    assert!(
        generated.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    let manifest = std::fs::read(output.join("manifest.json")).unwrap();
    let saved = storage.finalize_recording(&session.id).unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 100000);
    let db = storage.connection.lock().unwrap();
    let source: String = db
        .query_row(
            "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting.id],
            |r| r.get(0),
        )
        .unwrap();
    let source: crate::storage::RecordedAudioPlan = serde_json::from_str(&source).unwrap();
    assert_eq!(source.tracks.len(), 1);
    assert_eq!(source.tracks[0].kind, "system");
    assert_eq!(
        (source.tracks[0].sample_rate, source.tracks[0].channels),
        (16000, 1)
    );
    // The fixture's 480 input frames at 48 kHz retain 10 ms of audio at 16 kHz.
    assert_eq!(source.tracks[0].chunks[0].frames, 160);
    drop(db);
    let plan = storage
        .verify_canonical_audio(&meeting.id, &|| false)
        .unwrap();
    assert_eq!(plan.tracks[0].chunks[0].frames, 1600);
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(storage.finalize_recording(&session.id).unwrap(), saved);
}

#[test]
fn capture_ending_preserves_physical_pcm_boundary_gaps_actual_end_and_commit_time_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, output, _) = ending(&storage);
    let manifest = std::fs::read(output.join("manifest.json")).unwrap();
    let saved = storage.finalize_recording(&session).unwrap();
    assert_eq!(saved.duration_us, 1100000);
    assert!(saved.ended_at_utc_ms.is_some());
    assert_eq!(meeting_duration(&storage, &meeting), 1100000);
    let expected = gaps(&storage);
    assert_eq!(expected.len(), 3);
    assert_whole_gap_playback(&storage, &meeting, 1100000);
    let mut audio = storage
        .prepare_meeting_audio(&MeetingPlaybackRequest {
            meeting_id: meeting.clone(),
            mode: MeetingPlaybackMode::Mixed,
            start_reference: None,
        })
        .unwrap();
    assert_eq!(audio.read_analysis_pcm(16600, 1000).unwrap(), vec![0; 4000]);
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        manifest
    );
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    assert_eq!(storage.finalize_recording(&session).unwrap(), saved);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    assert_eq!(reopened.finalize_recording(&session).unwrap(), saved);
    assert_eq!(gaps(&reopened), expected);
    assert_whole_gap_playback(&reopened, &meeting, 1100000);
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
        assert!(storage.finalize_recording(&session).is_err());
        unpublished(&storage, &meeting, &session);
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
    crate::storage::recordings::tests::seal_probe_fixture(&output, &mut value);
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
    let mic = storage.canonical_audio_plan(&meeting).unwrap().tracks[0]
        .id
        .clone();
    for (first, last) in [(0, 500010), (700010, 1200020)] {
        let mut audio = storage
            .prepare_track_audio(&meeting, &mic, first, last)
            .unwrap();
        assert!(!audio.read_pcm(0, 32).unwrap().is_empty());
    }
    assert!(storage
        .prepare_track_audio(&meeting, &mic, 0, 1300000)
        .is_ok());
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
    assert!(gaps(&storage).len() >= 3);
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
fn capture_ending_current_backup_preserves_duration_and_canonical_pcm() {
    let root = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, session, _, _) = ending(&storage);
    storage.finalize_recording(&session).unwrap();
    let archive = destination.path().join("ending.monbackup");
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
    let restored = &receipt.meeting_ids[0];
    assert_ne!(restored, &meeting);
    assert_eq!(meeting_duration(&target, restored), 1100000);
    assert!(gaps(&target).is_empty()); // Retired acquisition tracks are excluded.
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
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "runs synthetic Swift clock recovery into native-owned output; make check-capture-ending"]
fn swift_generated_capture_ending_clock_recovery_preserves_sleep_gaps_and_actual_pcm() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage
        .create_meeting("synthetic sleep clock recovery")
        .unwrap();
    let identity = crate::capture_scope::ApplicationIdentity {
        pid: 321,
        bundle_identifier: "com.example.confirmed".into(),
        launch_time_us: 1_700_000_000_000_000,
    };
    let session = storage
        .prepare_recording(
            &meeting.id,
            &RecordingConfig {
                mode: "application".into(),
                backend: "screencapturekit".into(),
                target_pids: vec![identity.pid],
                microphone_device_uid: None,
                target_application_identity: Some(identity),
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
    let generated = std::process::Command::new("/usr/bin/xcrun")
        .args(["swift", "test", "--package-path"]).arg(repo.join("probes/macos-capture"))
        .args(["--disable-sandbox", "--filter", "SystemAudioRecoveryTests/testClockRecoveryWaitsForActualStopAndResumesOnlyNewConfirmedStreamWithoutChangingCAF"])
        .env("SWIFTPM_MODULECACHE_OVERRIDE", "/tmp/mon-swift-module-cache")
        .env("CLANG_MODULE_CACHE_PATH", "/tmp/mon-clang-module-cache")
        .env("MON_CLOCK_RECOVERY_FIXTURE_DIR", &output)
        .env("MON_CLOCK_RECOVERY_FIXTURE_OWNER", &session.id)
        .output().unwrap();
    assert!(
        generated.status.success(),
        "Swift clock recovery failed: {}\n{}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    let before = originals(&output);
    let manifest: Value = serde_json::from_slice(&before[0]).unwrap();
    assert_eq!(manifest["schemaVersion"], 4);
    assert_eq!(manifest["recordingOwnerId"], session.id);
    // Lifecycle history belongs to its separate producer journal/status, never
    // to the audio manifest. Its clock_changed reason is asserted in the Swift
    // source test and input_status's history tests, not invented here.
    assert!(manifest["system"].get("inputEventHistory").is_none());
    assert_eq!(manifest["system"]["systemRecovery"]["sourceEpoch"], 2);
    assert_eq!(manifest["system"]["systemRecovery"]["interruptions"], 1);
    assert_eq!(manifest["system"]["systemRecovery"]["state"], "stopped");
    let saved = storage.finalize_recording(&session.id).unwrap();
    assert_eq!(saved.duration_us, 10_050_000);
    assert_eq!(saved.ended_at_utc_ms, Some(1_700_000_010_059));
    assert_eq!(
        gaps(&storage),
        vec![
            (
                "mic".into(),
                10_000,
                10_040_000,
                "capture_timestamp_gap".into(),
                false
            ),
            (
                "system".into(),
                10_000,
                10_040_000,
                "capture_timestamp_gap".into(),
                false
            ),
        ]
    );
    let plan = storage
        .verify_canonical_audio(&meeting.id, &|| false)
        .unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].chunks[0].frames, 160800);
    let mut audio = storage
        .prepare_track_audio(&meeting.id, &plan.tracks[0].id, 0, 10_050_000)
        .unwrap();
    let mut pcm = Vec::new();
    for first in (0..160800).step_by(crate::audio::PREVIEW_FRAMES as usize) {
        pcm.extend(
            audio
                .read_pcm(first, (160800 - first).min(crate::audio::PREVIEW_FRAMES))
                .unwrap(),
        );
    }
    // Balanced stereo downmixes to silence. The preserved capture gaps above
    // remain provenance, while the canonical WAV covers the whole timeline.
    assert!(pcm
        .chunks_exact(4)
        .all(|b| f32::from_le_bytes(b.try_into().unwrap()) == 0.0));
    assert_whole_gap_playback(&storage, &meeting.id, 10_050_000);
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        before[0]
    );
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        load_session(&reopened.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap(),
        saved
    );
    assert_eq!(meeting_duration(&reopened, &meeting.id), 10_050_000);
    assert_whole_gap_playback(&reopened, &meeting.id, 10_050_000);
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        before[0]
    );
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
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
    assert_eq!(manifest["schemaVersion"], 4);
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
    let plan = storage
        .verify_canonical_audio(&meeting.id, &|| false)
        .unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].chunks[0].frames, 800);
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        before[0]
    );
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        load_session(&reopened.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap(),
        saved
    );
    assert_eq!(reopened.finalize_recording(&session.id).unwrap(), saved);
    assert_whole_gap_playback(&reopened, &meeting.id, 50000);
    let plan = reopened.recorded_audio_plan(&meeting.id).unwrap();
    let mut audio = reopened
        .prepare_track_audio(&meeting.id, &plan.tracks[0].id, 0, 50000)
        .unwrap();
    let pcm = audio.read_pcm(0, 800).unwrap();
    for (frame, sample) in pcm.chunks_exact(4).enumerate() {
        let expected = match frame {
            0..80 => 0.046875,
            80..160 => 0.140625,
            160..240 => 0.09375,
            _ => 0.0,
        };
        assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), expected);
    }
    assert_eq!(
        std::fs::read(output.join("manifest.json")).unwrap(),
        before[0]
    );
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
}
