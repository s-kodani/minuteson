use super::*;
use crate::playback::PlaybackMissingRange;

fn add_gap(storage: &Storage, track: &str, start: i64, end: i64, reason: &str) {
    storage.connection.lock().unwrap().execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES (?1,?2,?3,?4,?5,0)",params![Uuid::new_v4().to_string(),track,start,end,reason]).unwrap();
}
fn mapped_gap(storage: &Storage) {
    let db = storage.connection.lock().unwrap();
    db.execute(
        "UPDATE audio_chunks SET end_us=1200000 WHERE id='chunk-1'",
        [],
    )
    .unwrap();
    for (id, from, to, start, end) in [
        ("a", 0, 12000, 500000, 750000),
        ("b", 12000, 24000, 950000, 1200000),
    ] {
        db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track','chunk-1',?2,?3,?4,?5,'{\"schema_version\":1,\"method\":\"linear\"}')",params![id,from,to,start,end]).unwrap();
    }
    db.execute("UPDATE meetings SET duration_us=1200000", [])
        .unwrap();
    db.execute("UPDATE recording_sessions SET duration_us=1200000", [])
        .unwrap();
}
fn frames(packet: &[u8]) -> u32 {
    u32::from_le_bytes(packet[8..12].try_into().unwrap())
}

#[test]
fn reported_internal_clock_hole_has_short_packets_exact_timeline_and_no_silence() {
    let (_root, storage, meeting) = fixture();
    mapped_gap(&storage);
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    add_gap(&storage, "track", 750000, 950000, "capture_timestamp_gap");
    let store = PlaybackStore::default();
    let mut audio = storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .unwrap();
    assert!(audio.read_pcm(35000, 2000).is_err());
    let info = store.insert_meeting(audio).unwrap();
    assert_eq!(info.total_frames, 57600);
    assert_eq!(info.missing_ranges[0].start_frame, 36000);
    assert_eq!(info.missing_ranges[0].end_frame, 45600);
    let known = store.read(&info.token, 0).unwrap();
    assert_eq!(frames(&known), 36000);
    assert_eq!(sample(&known, 23999), 0.25);
    assert_eq!(sample(&known, 24000), -0.5);
    assert_eq!(sample(&known, 35999), -0.5);
    for at in [36000, 40000, 45599] {
        assert!(store.read(&info.token, at).is_err());
    }
    let after = store.read(&info.token, 45600).unwrap();
    assert_eq!(frames(&after), 12000);
    assert!(after[12..]
        .chunks_exact(4)
        .all(|value| f32::from_le_bytes(value.try_into().unwrap()) == -0.5));
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    assert!(storage
        .prepare_utterance_audio(&meeting, &analysis.id, &analysis.utterances[0].id)
        .is_ok());
    assert!(storage
        .prepare_track_audio(&meeting, "track", 500000, 1200000)
        .is_err());
}

#[test]
fn physical_gap_needs_complete_unrecovered_proof_not_a_recovered_or_partial_label() {
    for case in 0..3 {
        let (_root, storage, meeting) = fixture();
        mapped_gap(&storage);
        add_gap(
            &storage,
            "track",
            750000,
            if case == 1 { 900000 } else { 950000 },
            "capture_timestamp_gap",
        );
        if case == 0 {
            storage
                .connection
                .lock()
                .unwrap()
                .execute("UPDATE audio_gaps SET recovered=1", [])
                .unwrap();
        }
        if case == 2 {
            storage
                .connection
                .lock()
                .unwrap()
                .execute("UPDATE audio_gaps SET start_us=750001", [])
                .unwrap();
        }
        assert!(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .is_err(),
            "case {case}"
        );
    }
    let (_root, storage, meeting) = fixture();
    mapped_gap(&storage);
    add_gap(&storage, "track", 750000, 850000, "dropped");
    add_gap(&storage, "track", 850000, 950000, "dropped");
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_ok());
}

#[test]
fn nominal_chunk_holes_also_need_proof_and_preserve_later_pcm() {
    let (_root, storage, meeting) = fixture();
    storage.connection.lock().unwrap().execute_batch("UPDATE audio_chunks SET start_us=600000,end_us=1100000 WHERE id='chunk-1'; UPDATE meetings SET duration_us=1100000; UPDATE recording_sessions SET duration_us=1100000;").unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    add_gap(&storage, "track", 500000, 600000, "dropped");
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(frames(&store.read(&info.token, 0).unwrap()), 24000);
    assert!(store.read(&info.token, 24000).is_err());
    assert_eq!(sample(&store.read(&info.token, 28800).unwrap(), 0), -0.5);
}

#[test]
fn resampling_filter_halo_does_not_borrow_pcm_across_a_reported_clock_gap() {
    let (_root, storage, meeting) = fixture();
    let before = ["0.caf", "1.caf"].map(|name| {
        std::fs::read(
            storage
                .root
                .join("meetings")
                .join(&meeting)
                .join("audio")
                .join(name),
        )
        .unwrap()
    });
    {
        let db = storage.connection.lock().unwrap();
        for (index, start, end) in [(0, 0, 550000), (1, 650000, 1200000)] {
            db.execute(
                "UPDATE audio_chunks SET start_us=?1,end_us=?2 WHERE id=?3",
                params![start, end, format!("chunk-{index}")],
            )
            .unwrap();
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track',?2,0,24000,?3,?4,'{\"schema_version\":1,\"method\":\"linear\"}')",params![format!("map-{index}"),format!("chunk-{index}"),start,end]).unwrap();
        }
        db.execute_batch("UPDATE meetings SET duration_us=1200000; UPDATE recording_sessions SET duration_us=1200000;").unwrap();
    }
    add_gap(&storage, "track", 550000, 650000, "capture_timestamp_gap");
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    let first = store.read(&info.token, 0).unwrap();
    assert_eq!(frames(&first), 26400);
    assert!(first[12..].chunks_exact(4).all(|value| (f32::from_le_bytes(
        value.try_into().unwrap()
    ) - 0.25)
        .abs()
        < 1e-6));
    let next = store.read(&info.token, 31200).unwrap();
    assert_eq!(frames(&next), 26400);
    assert!(next[12..].chunks_exact(4).all(|value| (f32::from_le_bytes(
        value.try_into().unwrap()
    ) + 0.5)
        .abs()
        < 1e-6));
    let after = ["0.caf", "1.caf"].map(|name| {
        std::fs::read(
            storage
                .root
                .join("meetings")
                .join(&meeting)
                .join("audio")
                .join(name),
        )
        .unwrap()
    });
    assert_eq!(after, before);
}

#[test]
fn tail_duration_requires_gap_proof_and_never_pads_missing_pcm() {
    let (_root, storage, meeting) = fixture();
    storage.connection.lock().unwrap().execute_batch("UPDATE meetings SET duration_us=1200000; UPDATE recording_sessions SET duration_us=1200000;").unwrap();
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
        .is_err());
    add_gap(&storage, "track", 1000000, 1200000, "capture_stop_gap");
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(info.total_frames, 57600);
    assert_eq!(frames(&store.read(&info.token, 0).unwrap()), 48000);
    assert!(store.read(&info.token, 48000).is_err());
    assert!(store.read(&info.token, 57599).is_err());
}

#[test]
fn shared_end_beyond_all_pcm_requires_each_tracks_tail_proof() {
    let (_root, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    storage.connection.lock().unwrap().execute_batch("UPDATE meetings SET duration_us=2000000; UPDATE recording_sessions SET duration_us=2000000;").unwrap();
    add_gap(
        &storage,
        "system-track",
        1750000,
        2000000,
        "capture_stop_gap",
    );
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::System))
        .is_err());
    add_gap(&storage, "track", 1000000, 2000000, "capture_stop_gap");
    assert!(storage
        .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Both))
        .is_ok());
}

#[test]
fn both_blocks_either_missing_input_while_explicit_other_track_remains_audible() {
    let (_root, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    add_gap(&storage, "track", 500000, 700000, "dropped");
    add_gap(&storage, "system-track", 600000, 800000, "dropped");
    let store = PlaybackStore::default();
    let both = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Both))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(both.missing_ranges.len(), 2);
    assert_eq!(frames(&store.read(&both.token, 0).unwrap()), 24000);
    for at in [24000, 28800, 36000, 38399] {
        assert!(store.read(&both.token, at).is_err());
    }
    assert!((sample(&store.read(&both.token, 38400).unwrap(), 0)).abs() < 1e-6);
    let sys = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::System))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(sys.missing_ranges.len(), 1);
    assert!((sample(&store.read(&sys.token, 26400).unwrap(), 0) - 0.5).abs() < 1e-6);
    let mic = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(mic.missing_ranges.len(), 1);
    assert_eq!(sample(&store.read(&mic.token, 36000).unwrap(), 0), -0.5);
}

#[test]
fn subframe_gap_is_conservatively_excluded_without_moving_reported_microseconds() {
    let (_root, storage, meeting) = fixture();
    add_gap(&storage, "track", 100001, 100002, "dropped");
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    let range = &info.missing_ranges[0];
    assert_eq!(
        (
            range.start_us,
            range.end_us,
            range.start_frame,
            range.end_frame
        ),
        (100001, 100002, 4800, 4801)
    );
    assert_eq!(frames(&store.read(&info.token, 0).unwrap()), 4800);
    assert!(store.read(&info.token, 4800).is_err());
    assert_eq!(sample(&store.read(&info.token, 4801).unwrap(), 0), 0.25);
}

#[test]
fn missing_metadata_does_not_hide_missing_corrupt_or_tampered_files() {
    for case in 0..3 {
        let (_root, storage, meeting) = fixture();
        add_gap(&storage, "track", 500000, 1000000, "capture_stop_gap");
        let path = storage
            .root
            .join("meetings")
            .join(&meeting)
            .join("audio/1.caf");
        if case == 0 {
            std::fs::remove_file(&path).unwrap();
        } else if case == 1 {
            storage.connection.lock().unwrap().execute("UPDATE file_assets SET state='corrupt' WHERE id=(SELECT file_asset_id FROM audio_chunks WHERE id='chunk-1')",[]).unwrap();
        } else {
            let mut bytes = std::fs::read(&path).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            std::fs::write(path, bytes).unwrap();
        }
        assert!(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .is_err(),
            "case {case}"
        );
    }
}

#[test]
fn malformed_gap_bounds_and_control_codes_are_rejected_even_if_marked_recovered() {
    for (start, end, reason) in [(0, 1000001, "dropped"), (0, 1000, "bad\nreason")] {
        let (_root, storage, meeting) = fixture();
        add_gap(&storage, "track", start, end, reason);
        storage
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE audio_gaps SET recovered=1", [])
            .unwrap();
        assert!(storage
            .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
            .is_err());
    }
}

#[test]
fn changes_to_gap_proof_during_hashing_invalidate_whole_meeting_admission() {
    let (_root, storage, meeting) = fixture();
    add_gap(&storage, "track", 100000, 200000, "dropped");
    let checks = Cell::new(0);
    let error = storage
        .prepare_meeting_audio_cancellable(&request(&meeting, MeetingPlaybackMode::Mic), &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 5 {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute("UPDATE audio_gaps SET end_us=300000", [])
                    .unwrap();
            }
            false
        })
        .err()
        .unwrap();
    assert!(
        error.contains("変更") || error.contains("変わりました"),
        "{error}"
    );
}

#[test]
fn meeting_constructor_rechecks_derived_gap_coordinates_and_source_membership() {
    for case in 0..4 {
        let (_root, storage, meeting) = fixture();
        let source = storage
            .prepare_track_audio(&meeting, "track", 0, 1000000)
            .unwrap();
        let mut gap = PlaybackMissingRange::new("mic", 100000, 200000, "dropped", 1000000).unwrap();
        match case {
            0 => gap.end_frame += 1,
            1 => gap.track_kind = "system".into(),
            2 => gap.reason_code = "bad\nreason".into(),
            _ => gap.start_us = -1,
        };
        assert!(MeetingAudio::with_missing_ranges(vec![source], 1000000, vec![gap]).is_err());
    }
}

#[test]
fn declared_missing_pcm_in_a_continuous_map_is_not_borrowed_by_resampling() {
    let (_root, storage, meeting) = fixture();
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
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track',?2,0,24000,?3,?4,'{\"schema_version\":1,\"method\":\"linear\"}')",params![format!("map-{index}"),format!("chunk-{index}"),index*550000,(index+1)*550000]).unwrap();
        }
        db.execute_batch("UPDATE meetings SET duration_us=1100000; UPDATE recording_sessions SET duration_us=1100000;").unwrap();
    }
    // At 550 ms the files switch from +0.25 to -0.5. A right-hand
    // missing label must keep the final positive sample from borrowing -0.5.
    add_gap(&storage, "track", 550000, 600000, "dropped");
    let store = PlaybackStore::default();
    let info = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    let pcm = store.read(&info.token, 0).unwrap();
    assert_eq!(u32::from_le_bytes(pcm[8..12].try_into().unwrap()), 26400);
    assert!(pcm[12..].chunks_exact(4).all(|sample| (f32::from_le_bytes(
        sample.try_into().unwrap()
    ) - 0.25)
        .abs()
        < 1e-6));
    assert!(store.read(&info.token, 26400).is_err());
    let known = store.read(&info.token, 28800).unwrap();
    assert!(known[12..]
        .chunks_exact(4)
        .all(|sample| (f32::from_le_bytes(sample.try_into().unwrap()) + 0.5).abs() < 1e-6));
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE audio_gaps SET start_us=0,end_us=550000", [])
        .unwrap();
    let next = store
        .insert_meeting(
            storage
                .prepare_meeting_audio(&request(&meeting, MeetingPlaybackMode::Mic))
                .unwrap(),
        )
        .unwrap();
    assert!(store.read(&next.token, 0).is_err());
    let after = store.read(&next.token, 26400).unwrap();
    assert!(after[12..]
        .chunks_exact(4)
        .all(|sample| (f32::from_le_bytes(sample.try_into().unwrap()) + 0.5).abs() < 1e-6));
}
