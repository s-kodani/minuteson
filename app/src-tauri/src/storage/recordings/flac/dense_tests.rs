use crate::storage::{RecordingConfig, Storage};
use serde_json::Value;

#[test]
#[ignore = "Actual Swift dense raw/CAF capture fixture; synthetic PCM, no devices; make check-recording-raw-prefix"]
fn swift_dense_capture_clock_roundtrip_finalizes_replays_and_restores_over_legacy_capacity() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("高密度の取得時計").unwrap();
    let config = RecordingConfig {
        mode: "system".into(),
        backend: "screencapturekit".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    let session = storage
        .prepare_recording(&meeting.id, &config, "dense-clock")
        .unwrap();
    storage.mark_recording_started(&session.id).unwrap();
    let output = root
        .path()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    std::fs::create_dir_all(&output).unwrap();
    let output = output.canonicalize().unwrap();
    let child = std::process::Command::new("swift")
        .args(["test","--package-path",concat!(env!("CARGO_MANIFEST_DIR"),"/../../probes/macos-capture"),"--disable-sandbox",
            "--filter","RawRecordingSegmentTests/testNativePrefixFixtureExportsBoundManifestForRustWithoutCaptureDevices"])
        .env("SWIFTPM_MODULECACHE_OVERRIDE","/tmp/mon-swift-module-cache")
        .env("CLANG_MODULE_CACHE_PATH","/tmp/mon-clang-module-cache")
        .env("MON_RAW_SEGMENT_FIXTURE_DIR",&output).env("MON_RAW_SEGMENT_FIXTURE_OWNER",&session.id)
        .env("MON_RAW_SEGMENT_FIXTURE_DENSE","1").output().unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    let path = output.join("manifest.json");
    let original = std::fs::read(&path).unwrap();
    assert!(original.len() > crate::capture_manifest::SMALL_MANIFEST_BYTES as usize);
    let good: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        good["microphone"]["clockSegments"]
            .as_array()
            .unwrap()
            .len(),
        2048
    );
    for kind in ["missing-raw", "legacy", "foreign-owner", "clock-drift"] {
        let mut bad = good.clone();
        match kind {
            "missing-raw" => {
                bad["system"].as_object_mut().unwrap().remove("rawSegments");
            }
            "legacy" => {
                bad["schemaVersion"] = serde_json::json!(2);
                bad.as_object_mut().unwrap().remove("captureEnd");
            }
            "foreign-owner" => {
                bad["microphone"]["rawSegments"]["ownerId"] =
                    serde_json::json!(uuid::Uuid::new_v4().to_string())
            }
            "clock-drift" => {
                bad["microphone"]["clockSegments"][0]["timelineEndUs"] = serde_json::json!(9999)
            }
            _ => unreachable!(),
        }
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(storage.finalize_recording(&session.id).is_err(), "{kind}");
        let count: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM file_assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    std::fs::write(&path, &original).unwrap();
    let saved = storage.finalize_recording(&session.id).unwrap();
    assert_eq!(saved.duration_us, 42_000_000);
    fn verify(storage: &Storage, meeting: &str) {
        let source_json: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
                [meeting],
                |r| r.get(0),
            )
            .unwrap();
        let original: crate::storage::RecordedAudioPlan =
            serde_json::from_str(&source_json).unwrap();
        assert_eq!(original.tracks.len(), 2);
        for track in &original.tracks {
            assert_eq!(
                track
                    .chunks
                    .iter()
                    .map(|c| c.clock_segments.len())
                    .sum::<usize>(),
                2048
            );
            assert_eq!(track.chunks.iter().map(|c| c.frames).sum::<i64>(), 983_040);
        }
        let (source, known) = storage.recorded_analysis_input(meeting).unwrap();
        known.unwrap().validate(&source).unwrap();
        assert_eq!(source.tracks.len(), 1);
        let track = &source.tracks[0];
        assert_eq!(track.kind, "processed");
        assert_eq!(track.chunks[0].frames, 672_000);
        assert!(track.chunks[0].clock_segments.is_empty());
        let mut audio = storage
            .prepare_track_audio(meeting, &track.id, 0, 30_000)
            .unwrap();
        let pcm = audio.read_pcm(0, 480).unwrap();
        for (index, sample) in pcm.chunks_exact(4).enumerate() {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if (160..320).contains(&index) {
                    0.0
                } else {
                    -0.125
                }
            );
        }
        use crate::storage::meeting_playback::{MeetingPlaybackMode, MeetingPlaybackRequest};
        let audio = storage
            .prepare_meeting_audio(&MeetingPlaybackRequest {
                start_reference: None,
                meeting_id: meeting.into(),
                mode: MeetingPlaybackMode::Mixed,
            })
            .unwrap();
        let store = crate::playback::PlaybackStore::default();
        let info = store.insert_meeting(audio).unwrap();
        assert!(info.missing_ranges.is_empty());
        assert!(store.read(&info.token, 0).is_ok());
        assert!(store.read(&info.token, 160).is_ok());
        assert!(store.read(&info.token, 320).is_ok());
        store.close(&info.token).unwrap();
    }
    verify(&storage, &meeting.id);
    let archives = tempfile::tempdir().unwrap();
    let archive = archives.path().join("dense.monbackup");
    storage
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let restored_root = tempfile::tempdir().unwrap();
    let restored = Storage::open(restored_root.path().into()).unwrap();
    let existing = restored.create_meeting("保持").unwrap();
    let selection = restored
        .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let receipt = selection
        .publish(
            &restored,
            &selection.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert_ne!(receipt.meeting_ids[0], meeting.id);
    verify(&restored, &receipt.meeting_ids[0]);
    assert!(restored.get_meeting(&existing.id).unwrap().is_some());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    drop(storage);
    let cold = Storage::open(root.path().into()).unwrap();
    assert!(cold.recover_completed_recordings().unwrap().is_empty());
    verify(&cold, &meeting.id);
    assert_eq!(std::fs::read(path).unwrap(), original);
}
