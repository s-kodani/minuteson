use super::*;
use crate::audio::tests::caf_fixture;
use crate::playback::{PlaybackPreparations, PlaybackStore};
use std::cell::Cell;

fn fixture() -> (tempfile::TempDir, Storage, String) {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let (meeting, _, _) = crate::storage::playback::tests::fixture(&storage);
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET duration_us = 1000000 WHERE id = ?1",
            params![meeting],
        )
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_sessions SET duration_us = 1000000 WHERE meeting_id = ?1",
            params![meeting],
        )
        .unwrap();
    (directory, storage, meeting)
}

fn request(meeting: &str, mode: MeetingPlaybackMode) -> MeetingPlaybackRequest {
    MeetingPlaybackRequest {
        meeting_id: meeting.into(),
        mode,
    }
}

fn add_system(storage: &Storage, meeting: &str) {
    let relative = PathBuf::from("meetings")
        .join(meeting)
        .join("audio/system.caf");
    let pcm: Vec<u8> = (0..48_000)
        .flat_map(|_| {
            [0.25_f32, 0.75]
                .into_iter()
                .flat_map(|sample| sample.to_le_bytes())
        })
        .collect();
    std::fs::write(
        storage.managed_path(&relative).unwrap(),
        caf_fixture(32_000, 2, 3, 32, 4, &pcm),
    )
    .unwrap();
    let asset = storage
        .register_ready_meeting_asset(meeting, &relative, "audio/x-caf")
        .unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('system-track',?1,'session','system','caf',32000,2,'ready',0,0)", params![meeting]).unwrap();
    db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES ('system-chunk',?1,'system-track',?2,0,250000,1750000,48000,32000,2,'ready',0)", params![meeting, asset.id]).unwrap();
    db.execute(
        "UPDATE meetings SET duration_us = 1750000 WHERE id = ?1",
        params![meeting],
    )
    .unwrap();
    db.execute(
        "UPDATE recording_sessions SET duration_us = 1750000 WHERE meeting_id = ?1",
        params![meeting],
    )
    .unwrap();
}

fn sample(packet: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(packet[12 + offset * 4..16 + offset * 4].try_into().unwrap())
}

#[test]
fn full_meeting_modes_have_one_timeline_and_preserve_original_utterance_playback() {
    let (_directory, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    let store = PlaybackStore::default();
    for (mode, kind, before, overlap, tail) in [
        (MeetingPlaybackMode::Mic, "mic", 0.25, 0.25, 0.0),
        (MeetingPlaybackMode::System, "system", 0.0, 0.5, 0.5),
        (MeetingPlaybackMode::Both, "both", 0.125, 0.375, 0.25),
    ] {
        let audio = storage
            .prepare_meeting_audio(&request(&meeting, mode))
            .unwrap();
        let info = store.insert_meeting(audio).unwrap();
        assert_eq!(info.track_kind, kind);
        assert_eq!(info.total_frames, 84_000);
        assert_eq!(info.end_us, 1_750_000);
        assert_eq!(info.alignment, "unverified");
        let packet = store.read(&info.token, 0).unwrap();
        assert_eq!(sample(&packet, 6000), before);
        assert!((sample(&packet, 18_000) - overlap).abs() < 1e-6);
        let after_boundary = match mode {
            MeetingPlaybackMode::Mic => -0.5,
            MeetingPlaybackMode::System => 0.5,
            MeetingPlaybackMode::Both => 0.0,
            MeetingPlaybackMode::Imported => unreachable!(),
        };
        assert!((sample(&packet, 30_000) - after_boundary).abs() < 1e-6);
        let packet = store.read(&info.token, 72_000).unwrap();
        assert!((sample(&packet, 0) - tail).abs() < 1e-6);
        assert_eq!(packet.len(), 12 + 12_000 * 4);
    }
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Imported))
        .is_err());
    assert!(storage
        .prepare_meeting_audio(&request("other", MeetingPlaybackMode::Mic))
        .is_err());
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let legacy = store
        .insert(
            storage
                .prepare_utterance_audio(&meeting, &analysis.id, &analysis.utterances[0].id)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(legacy.total_frames, 24_000);
    assert_eq!(sample(&store.read(&legacy.token, 0).unwrap(), 0), 0.25);
}

#[test]
fn absent_ambiguous_unready_and_live_sources_do_not_fall_back_to_another_track() {
    let (_directory, storage, meeting) = fixture();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::System))
        .is_err());
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Both))
        .is_err());
    storage.connection.lock().unwrap().execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('duplicate',?1,'session','mic','caf',48000,1,'ready',0,0)", params![meeting]).unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM audio_tracks WHERE id = 'duplicate'", [])
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET state = 'interrupted' WHERE id = 'track'",
            [],
        )
        .unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET state = 'ready' WHERE id = 'track'",
            [],
        )
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_sessions SET state = 'recording' WHERE id = 'session'",
            [],
        )
        .unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
}

#[test]
fn full_source_missing_corrupt_and_internal_gaps_are_not_padded_with_silence() {
    for state in ["missing", "corrupt"] {
        let (_directory, storage, meeting) = fixture();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE audio_chunks SET state = ?1 WHERE id = 'chunk-1'",
                params![state],
            )
            .unwrap();
        assert!(storage
            .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
            .is_err());
    }
    let (_directory, storage, meeting) = fixture();
    storage.connection.lock().unwrap().execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',0,100000,'dropped',0)", []).unwrap();
    let store = PlaybackStore::default();
    let missing = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(missing.missing_ranges.len(), 1);
    assert!(store.read(&missing.token, 0).is_err());
    assert_eq!(sample(&store.read(&missing.token, 4800).unwrap(), 0), 0.25);
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE audio_gaps SET recovered = 1 WHERE id = 'gap'", [])
        .unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_ok());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_chunks SET start_us = 500100 WHERE id = 'chunk-1'",
            [],
        )
        .unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
}

#[test]
fn removed_boundaries_sequence_holes_and_duration_mismatches_are_rejected() {
    for case in 0..4 {
        let (_directory, storage, meeting) = fixture();
        let sql = match case {
            0 => "DELETE FROM audio_chunks WHERE id = 'chunk-0'",
            1 => "DELETE FROM audio_chunks WHERE id = 'chunk-1'",
            2 => "UPDATE meetings SET duration_us = 2000000",
            _ => "UPDATE recording_sessions SET duration_us = 2000000",
        };
        storage.connection.lock().unwrap().execute(sql, []).unwrap();
        assert!(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .is_err(),
            "case {case}"
        );
    }
}

#[test]
fn explicit_missing_leading_audio_is_not_ordinary_offset_padding() {
    let (_directory, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    storage.connection.lock().unwrap().execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','system-track',0,200000,'dropped',0)", []).unwrap();
    let store = PlaybackStore::default();
    for mode in [MeetingPlaybackMode::System, MeetingPlaybackMode::Both] {
        let info = store
            .insert_meeting(
                storage
                    .prepare_meeting_audio(&request(&meeting, mode))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(info.missing_ranges.len(), 1);
        assert!(store.read(&info.token, 0).is_err());
        let after = store.read(&info.token, 9600).unwrap();
        assert_eq!(
            sample(&after, 0),
            if matches!(mode, MeetingPlaybackMode::System) {
                0.0
            } else {
                0.125
            }
        );
        assert!(
            (sample(&after, 2400)
                - if matches!(mode, MeetingPlaybackMode::System) {
                    0.5
                } else {
                    0.375
                })
            .abs()
                < 1e-6
        );
    }
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_ok());
}

#[test]
fn unknown_clock_metadata_is_not_played_as_identity() {
    let (_directory, storage, meeting) = fixture();
    storage.connection.lock().unwrap().execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES ('map','track','chunk-0',0,24000,0,500000,'{}')", []).unwrap();
    let error = storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .err()
        .unwrap();
    assert!(error.contains("時刻補正の形式情報"), "{error}");
}

#[test]
fn corrected_mic_meeting_time_is_used_for_mic_and_both_mix_without_sync_claim() {
    let (_directory, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    {
        let db = storage.connection.lock().unwrap();
        for index in 0..2 {
            db.execute(
                "UPDATE audio_chunks SET start_us=?1,end_us=?2 WHERE id=?3",
                params![
                    index * 550000,
                    (index + 1) * 550000,
                    format!("chunk-{index}")
                ],
            )
            .unwrap();
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track',?2,0,24000,?3,?4,?5)",params![format!("map-{index}"),format!("chunk-{index}"),index*550000,(index+1)*550000,r#"{"schema_version":1,"method":"linear"}"#]).unwrap();
        }
    }
    let store = PlaybackStore::default();
    for (mode, before, after) in [
        (MeetingPlaybackMode::Mic, 0.25, -0.5),
        (MeetingPlaybackMode::Both, 0.375, 0.0),
    ] {
        let audio = storage
            .prepare_meeting_audio(&request(&meeting, mode))
            .unwrap();
        let info = store.insert_meeting(audio).unwrap();
        assert_eq!(info.alignment, "unverified");
        assert_eq!(info.total_frames, 84000);
        let packet = store.read(&info.token, 0).unwrap();
        assert!((sample(&packet, 25000) - before).abs() < 1e-6);
        assert!((sample(&packet, 28000) - after).abs() < 1e-6);
    }
}

#[test]
fn managed_hash_and_track_format_checks_cover_the_whole_source() {
    let (_directory, storage, meeting) = fixture();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET channels = 2 WHERE id = 'track'",
            [],
        )
        .unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET channels = 1 WHERE id = 'track'",
            [],
        )
        .unwrap();
    let path = storage
        .managed_path(&PathBuf::from("meetings").join(&meeting).join("audio/0.caf"))
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
}

#[test]
fn changed_native_census_during_file_verification_invalidates_preparation() {
    let (_directory, storage, meeting) = fixture();
    let baseline = Cell::new(0);
    storage
        .prepare_meeting_audio_cancellable(&request(&meeting, MeetingPlaybackMode::Mic), &|| {
            baseline.set(baseline.get() + 1);
            false
        })
        .unwrap();
    // The first hash block tests the inner source snapshot. The penultimate
    // cancellation boundary is after source return but before the outer census.
    for (boundary, expected) in [
        (5, "保存記録・時刻補正が変更されました"),
        (baseline.get() - 1, "保存記録が変わりました"),
    ] {
        let (_directory, storage, meeting) = fixture();
        let checks = Cell::new(0);
        let error=storage.prepare_meeting_audio_cancellable(&request(&meeting,MeetingPlaybackMode::Mic),&||{
            checks.set(checks.get()+1);
            if checks.get()==boundary { storage.connection.lock().unwrap().execute("UPDATE audio_tracks SET processing_config_json = '{\"timeline_alignment\":\"changed\"}' WHERE id = 'track'", []).unwrap(); }
            false
        }).err().unwrap();
        assert!(error.contains(expected), "boundary {boundary}: {error}");
        assert!(storage
            .prepare_meeting_audio_cancellable(
                &request(&meeting, MeetingPlaybackMode::Mic),
                &|| true
            )
            .is_err());
    }
}

#[test]
fn native_cancel_interrupts_whole_meeting_hashing_for_each_capture_mode() {
    for mode in [
        MeetingPlaybackMode::Mic,
        MeetingPlaybackMode::System,
        MeetingPlaybackMode::Both,
    ] {
        let (_directory, storage, meeting) = fixture();
        add_system(&storage, &meeting);
        let preparations = PlaybackPreparations::default();
        let store = PlaybackStore::default();
        let id = Uuid::new_v4().to_string();
        preparations.reserve(&id).unwrap();
        let guard = preparations.claim(&id).unwrap();
        let checks = Cell::new(0);
        let error = storage
            .prepare_meeting_audio_cancellable(&request(&meeting, mode), &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 6 {
                    preparations.cancel(&id, &store).unwrap();
                }
                guard.cancelled()
            })
            .err()
            .unwrap();
        assert!(error.contains("照合を中断"), "{error}");
        assert_eq!(checks.get(), 6);
        assert!(guard
            .publish(|| panic!("cancelled meeting must not publish"))
            .is_err());
        drop(guard);
        assert!(preparations.claim(&id).is_err());
        assert!(storage
            .prepare_meeting_audio(&request(&meeting, mode))
            .is_ok());
    }
}

#[test]
fn imported_meeting_uses_its_managed_source_and_never_claims_capture_sync() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let meeting = storage.create_meeting("取り込み再生").unwrap();
    let relative = PathBuf::from("meetings")
        .join(&meeting.id)
        .join("import.caf");
    std::fs::create_dir_all(storage.managed_path(relative.parent().unwrap()).unwrap()).unwrap();
    let pcm: Vec<u8> = vec![0.5_f32; 4410]
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    std::fs::write(
        storage.managed_path(&relative).unwrap(),
        caf_fixture(44_100, 1, 3, 32, 4, &pcm),
    )
    .unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting.id, &relative, "audio/x-caf")
        .unwrap();
    let db = storage.connection.lock().unwrap();
    db.execute(
        "UPDATE meetings SET source_type = 'audio_import', lifecycle = 'imported', duration_us = 100000 WHERE id = ?1",
        params![meeting.id],
    )
    .unwrap();
    db.execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,duration_us,created_at,updated_at) VALUES ('import',?1,'ready','example.caf',100000,0,0)", params![meeting.id]).unwrap();
    db.execute("INSERT INTO audio_tracks(id,meeting_id,import_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES ('track',?1,'import','imported','caf',44100,1,'ready','{\"timeline_alignment\":\"source\"}',0,0)", params![meeting.id]).unwrap();
    db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES ('chunk',?1,'track',?2,0,0,100000,4410,44100,1,'ready',0)", params![meeting.id,asset.id]).unwrap();
    drop(db);
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting.id, MeetingPlaybackMode::Imported))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(info.total_frames, 4800);
    assert_eq!(info.track_kind, "imported");
    assert_eq!(info.alignment, "unverified");
    assert!((sample(&store.read(&info.token, 0).unwrap(), 1000) - 0.5).abs() < 1e-6);
}

#[test]
fn request_never_accepts_paths_unknown_fields_or_unknown_modes() {
    assert!(serde_json::from_value::<MeetingPlaybackRequest>(
        serde_json::json!({"meetingId":"x", "mode":"both"})
    )
    .is_ok());
    for value in [
        serde_json::json!({"meetingId":"x", "mode":"processed"}),
        serde_json::json!({"meetingId":"x", "mode":"mic", "path":"/tmp/source.caf"}),
    ] {
        assert!(serde_json::from_value::<MeetingPlaybackRequest>(value).is_err());
    }
}

mod markdown_interval;
mod missing_tests;
