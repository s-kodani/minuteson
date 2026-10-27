use super::*;
use crate::audio::{tests::caf_fixture, VerifiedAudio};

pub(in crate::storage) fn fixture() -> (tempfile::TempDir, Storage, String, Vec<PathBuf>) {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().to_path_buf()).unwrap();
    let meeting = storage.create_meeting("移行しても会議名を保持").unwrap().id;
    let session = Uuid::new_v4().to_string();
    let base = Path::new("meetings").join(&meeting).join("fixture");
    std::fs::create_dir_all(storage.root.join(&base)).unwrap();
    let manifest = base.join("capture.json");
    std::fs::write(storage.root.join(&manifest),serde_json::to_vec(&serde_json::json!({
        "schemaVersion":4,"inputDiagnosticsState":"unavailable","state":"saved","captureMode":"system","backend":"wasapi","targetPIDs":[],"errors":[],"recordingOwnerId":session,
        "microphone":{"frames":43200,"sampleRate":48000,"channels":1,"appendFailures":0,"callbackCount":1},
        "system":{"frames":36000,"sampleRate":48000,"channels":1,"appendFailures":0,"callbackCount":1}
    })).unwrap()).unwrap();
    let manifest = storage
        .register_ready_meeting_asset(&meeting, &manifest, "application/json")
        .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type='recording' WHERE id=?1",
            [&meeting],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,duration_us,audio_manifest_hash,created_at,updated_at) VALUES(?1,?2,'saved','system','{}',?1,1000000,?3,0,0)",params![session,meeting,manifest.content_hash]).unwrap();
        db.execute(
            "UPDATE meetings SET lifecycle='recorded',duration_us=1000000 WHERE id=?1",
            [&meeting],
        )
        .unwrap();
    }
    let mut paths = Vec::new();
    for (kind, value, ranges) in [
        ("mic", 0.25f32, vec![(0i64, 300000i64), (400000, 1000000)]),
        ("system", 0.5f32, vec![(0i64, 250000i64), (500000, 1000000)]),
    ] {
        let frames = ranges
            .iter()
            .map(|(a, b)| (b - a) * 48000 / 1000000)
            .sum::<i64>();
        let pcm: Vec<u8> = (0..frames).flat_map(|_| value.to_le_bytes()).collect();
        let path = base.join(format!("{kind}.caf"));
        std::fs::write(
            storage.root.join(&path),
            caf_fixture(48000, 1, 3, 32, 4, &pcm),
        )
        .unwrap();
        let asset = storage
            .register_ready_meeting_asset(&meeting, &path, "audio/x-caf")
            .unwrap();
        paths.push(storage.root.join(path));
        let track = Uuid::new_v4().to_string();
        let chunk = Uuid::new_v4().to_string();
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'caf',48000,1,'ready',0,0)",params![track,meeting,session,kind]).unwrap();
        db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,1000000,?5,48000,1,'ready',0)",params![chunk,meeting,track,asset.id,frames]).unwrap();
        let mut first = 0;
        for (a, b) in ranges {
            let end = first + (b - a) * 48000 / 1000000;
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES(?1,?2,?3,?4,?5,?6,?7,'{\"schema_version\":1,\"method\":\"linear\"}')",params![Uuid::new_v4().to_string(),track,chunk,first,end,a,b]).unwrap();
            first = end;
        }
        let (a, b) = if kind == "mic" {
            (300000, 400000)
        } else {
            (250000, 500000)
        };
        db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,?3,?4,'capture_timestamp_gap',0)",params![Uuid::new_v4().to_string(),track,a,b]).unwrap();
    }
    (temp, storage, meeting, paths)
}

#[test]
fn one_wav_keeps_available_input_and_silence_without_skipping_time() {
    let (_temp, storage, meeting, originals) = fixture();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    let track = &plan.tracks[0];
    let c = &track.chunks[0];
    assert_eq!(c.frames, 16000);
    assert_eq!(track.end_us, 1000000);
    let path = storage.meeting_mix_path(&meeting, track).unwrap();
    let mut audio =
        VerifiedAudio::open(std::fs::File::open(&path).unwrap(), c.size as u64, &c.hash).unwrap();
    for (frame, expected) in [
        (1600, 0.375f32),
        (4000, 0.125),
        (4400, 0.125),
        (4800, 0.0),
        (5600, 0.0),
        (6399, 0.0),
        (6400, 0.125),
        (7200, 0.125),
        (8000, 0.375),
        (12000, 0.375),
    ] {
        let pcm = audio.read_frames(frame, 1).unwrap();
        assert!((f32::from_le_bytes(pcm[..4].try_into().unwrap()) - expected).abs() < 0.001);
    }
    assert!(originals.iter().all(|p| !p.exists()));
    drop(audio);
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| panic!("must not regenerate"))
        .unwrap();
    assert_eq!(plan, storage.canonical_audio_plan(&meeting).unwrap());
    assert_eq!(
        storage.get_meeting(&meeting).unwrap().unwrap().title,
        "移行しても会議名を保持"
    );
    let mut playback = storage
        .prepare_meeting_audio(&super::super::meeting_playback::MeetingPlaybackRequest {
            meeting_id: meeting.clone(),
            mode: super::super::meeting_playback::MeetingPlaybackMode::Mixed,
            start_reference: None,
        })
        .unwrap();
    assert_eq!(playback.sample_rate, 16000);
    assert_eq!(playback.total_frames, 16000);
    assert!(playback.missing_ranges.is_empty());
    assert_eq!(
        playback.read_analysis_pcm(4800, 1600).unwrap(),
        vec![0; 1600 * 4]
    );
    let interval = storage
        .prepare_meeting_interval_cancellable(
            &super::super::meeting_playback::MeetingIntervalRequest {
                meeting_id: meeting,
                mode: super::super::meeting_playback::MeetingPlaybackMode::Mixed,
                start_us: 300000,
                end_us: 600000,
                expected_duration_us: 1000000,
            },
            &|| false,
        )
        .unwrap();
    assert_eq!(interval.sample_rate, 16000);
    assert_eq!(interval.total_frames, 4800);
    assert!(interval.missing_ranges.is_empty());
}

#[test]
fn source_clock_conversion_is_applied_once_before_publication() {
    let (_temp, storage, meeting, _) = fixture();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("UPDATE track_clock_segments SET timeline_start_us=timeline_start_us*2,timeline_end_us=timeline_end_us*2",[]).unwrap();
        db.execute("UPDATE audio_chunks SET end_us=end_us*2", [])
            .unwrap();
        db.execute(
            "UPDATE audio_gaps SET start_us=start_us*2,end_us=end_us*2",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE meetings SET duration_us=2000000 WHERE id=?1",
            [&meeting],
        )
        .unwrap();
        db.execute(
            "UPDATE recording_sessions SET duration_us=2000000 WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    }
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    assert_eq!(plan.tracks[0].chunks[0].frames, 32000);
    assert_eq!(plan.tracks[0].end_us, 2000000);
    let path = storage.meeting_mix_path(&meeting, &plan.tracks[0]).unwrap();
    let c = &plan.tracks[0].chunks[0];
    let mut audio =
        VerifiedAudio::open(std::fs::File::open(path).unwrap(), c.size as u64, &c.hash).unwrap();
    for (frame, expected) in [
        (3200, 0.375f32),
        (8800, 0.125),
        (11200, 0.0),
        (14400, 0.125),
        (24000, 0.375),
    ] {
        let pcm = audio.read_frames(frame, 1).unwrap();
        assert!((f32::from_le_bytes(pcm[..4].try_into().unwrap()) - expected).abs() < 0.001);
    }
}

#[test]
fn sub_sample_gap_keeps_the_timeline_without_rejecting_valid_audio() {
    let (_temp, storage, meeting, _) = fixture();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("DELETE FROM track_clock_segments", []).unwrap();
        db.execute("DELETE FROM audio_gaps", []).unwrap();
        let mut q = db
            .prepare("SELECT id,track_id,frame_count FROM audio_chunks")
            .unwrap();
        let chunks: Vec<(String, String, i64)> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(q);
        for (chunk, track, frames) in chunks {
            for (first, end, start, last) in [
                (0, frames / 2, 0, 500001),
                (frames / 2, frames, 500021, 1000021),
            ] {
                db.execute("INSERT INTO track_clock_segments VALUES(?1,?2,?3,?4,?5,?6,?7,'{\"schema_version\":1,\"method\":\"linear\"}')",params![Uuid::new_v4().to_string(),track,chunk,first,end,start,last]).unwrap();
            }
            db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,500001,500021,'capture_timestamp_gap',0)",params![Uuid::new_v4().to_string(),track]).unwrap();
        }
        db.execute("UPDATE audio_chunks SET end_us=1000021", [])
            .unwrap();
        db.execute(
            "UPDATE meetings SET duration_us=1000021 WHERE id=?1",
            [&meeting],
        )
        .unwrap();
        db.execute(
            "UPDATE recording_sessions SET duration_us=1000021 WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    }
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let plan = storage.verify_canonical_audio(&meeting, &|| false).unwrap();
    assert_eq!(plan.tracks[0].chunks[0].frames, 16001);
}

#[test]
fn cancelled_publication_retains_originals_and_has_no_canonical_commit() {
    let (_temp, storage, meeting, originals) = fixture();
    assert!(storage
        .publish_single_wav(&meeting, &|| true, &|_, _| {})
        .is_err());
    assert!(originals.iter().all(|p| p.exists()));
    assert!(storage.canonical_audio_plan(&meeting).is_err());
}

#[test]
fn changed_original_is_rejected_before_retirement() {
    let (_temp, storage, meeting, originals) = fixture();
    std::fs::write(&originals[0], b"changed").unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    assert!(originals.iter().all(|p| p.exists()));
}

#[test]
fn canonical_backup_restores_one_audio_without_retired_inputs() {
    let (_temp, storage, meeting, _) = fixture();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let (restored, id, _) = super::super::backup::published_restore_for_rebinding_test(
        &storage,
        target.path(),
        &meeting,
        &[],
    );
    let plan = restored.verify_canonical_audio(&id, &|| false).unwrap();
    assert_eq!(plan.tracks[0].chunks[0].frames, 16000);
    assert!(restored
        .meeting_mix_path(&id, &plan.tracks[0])
        .unwrap()
        .is_file());
    assert_eq!(
        restored.get_meeting(&id).unwrap().unwrap().title,
        "移行しても会議名を保持"
    );
}

#[test]
fn interrupted_pcm_write_resumes_owned_output_and_never_retires_sources_early() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_temp, storage, meeting, originals) = fixture();
    let stop = AtomicBool::new(false);
    assert!(storage
        .publish_single_wav(&meeting, &|| stop.load(Ordering::SeqCst), &|done, _| {
            if done > 0 {
                stop.store(true, Ordering::SeqCst);
            }
        })
        .is_err());
    assert!(originals.iter().all(|p| p.exists()));
    assert!(storage.canonical_audio_plan(&meeting).is_err());
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert!(originals.iter().all(|p| !p.exists()));
    storage.verify_canonical_audio(&meeting, &|| false).unwrap();
}

#[test]
fn single_wav_retirement_preserves_notification_companions_byte_for_byte() {
    let (_temp, storage, meeting, paths) = fixture();
    let directory = paths[0].parent().unwrap();
    let events = directory.join("input-events.json");
    let notices = directory.join("input-notices.json");
    let evidence = b"retained producer history";
    let receipt = b"retained rendered receipt";
    std::fs::write(&events, evidence).unwrap();
    std::fs::write(&notices, receipt).unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert_eq!(std::fs::read(events).unwrap(), evidence);
    assert_eq!(std::fs::read(notices).unwrap(), receipt);
    assert!(paths.iter().all(|p| !p.exists()));
}
