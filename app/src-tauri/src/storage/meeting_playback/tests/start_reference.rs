use super::*;

fn utterance_request(storage: &Storage, meeting: &str) -> MeetingPlaybackRequest {
    let analysis = storage.load_current_analysis(meeting).unwrap().unwrap();
    let utterance = &analysis.utterances[0];
    let mut r = request(meeting, MeetingPlaybackMode::Mic);
    r.start_reference = Some(MeetingPlaybackStartReference::Utterance {
        analysis_id: analysis.id.clone(),
        utterance_id: utterance.id.clone(),
        expected_content_hash: analysis.content_hash.clone(),
        start_us: utterance.start_us,
        end_us: utterance.end_us,
    });
    r
}

#[test]
fn referenced_playback_keeps_full_timeline_and_pcm_after_reference_end() {
    let (_dir, storage, meeting) = fixture();
    add_system(&storage, &meeting);
    for interval in [false, true] {
        let mut r = utterance_request(&storage, &meeting);
        r.mode = MeetingPlaybackMode::Both;
        if interval {
            r.start_reference = Some(MeetingPlaybackStartReference::Interval {
                start_us: 123456,
                end_us: 500000,
                expected_duration_us: 1750000,
            });
        }
        let store = PlaybackStore::default();
        let info = store
            .insert_meeting(storage.prepare_meeting_audio(&r).unwrap())
            .unwrap();
        assert_eq!(info.start_us, 0);
        assert_eq!(info.end_us, 1750000);
        assert_eq!(info.total_frames, 84000);
        assert_eq!(info.track_kind, "both");
        // The utterance ends at 0.75s, but the same token reads PC audio at 1.5s.
        assert_eq!(sample(&store.read(&info.token, 72000).unwrap(), 0), 0.25);
    }
}

#[test]
fn referenced_playback_refuses_forged_times_hashes_versions_and_changed_duration() {
    let (_dir, storage, meeting) = fixture();
    let mut valid = serde_json::json!({
        "meetingId":meeting, "mode":"mic", "startReference": {
            "kind":"interval", "startUs":1, "endUs":500000, "expectedDurationUs":1000000
        }
    });
    for (key, value) in [
        ("startUs", -1),
        ("endUs", 0),
        ("endUs", 1000001),
        ("expectedDurationUs", 1000001),
    ] {
        let mut invalid = valid.clone();
        invalid["startReference"][key] = value.into();
        let r: MeetingPlaybackRequest = serde_json::from_value(invalid).unwrap();
        assert!(storage.prepare_meeting_audio(&r).is_err());
    }
    valid["startReference"]["path"] = "/tmp/audio.caf".into();
    assert!(serde_json::from_value::<MeetingPlaybackRequest>(valid).is_err());
    for field in ["time", "hash", "analysis", "utterance", "meeting"] {
        let mut r = utterance_request(&storage, &meeting);
        if let Some(MeetingPlaybackStartReference::Utterance {
            analysis_id,
            utterance_id,
            expected_content_hash,
            start_us,
            ..
        }) = &mut r.start_reference
        {
            match field {
                "time" => *start_us += 1,
                "hash" => *expected_content_hash = "0".repeat(64),
                "analysis" => *analysis_id = "other".into(),
                "utterance" => *utterance_id = "other".into(),
                "meeting" => r.meeting_id = "other".into(),
                _ => unreachable!(),
            }
        }
        assert!(storage.prepare_meeting_audio(&r).is_err(), "{field}");
    }
}

#[test]
fn referenced_playback_rechecks_analysis_after_audio_preparation_and_honors_cancel() {
    let (_dir, storage, meeting) = fixture();
    let r = utterance_request(&storage, &meeting);
    assert!(storage
        .prepare_meeting_audio_cancellable(&r, &|| true)
        .is_err());
    let count = Cell::new(0);
    let result = storage.prepare_meeting_audio_cancellable(&r, &|| {
        count.set(count.get() + 1);
        if count.get() == 3 {
            storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE meetings SET current_analysis_id=NULL, previous_analysis_id=NULL WHERE id=?1",
                    params![meeting],
                )
                .unwrap();
        }
        false
    });
    assert!(count.get() >= 3);
    assert!(result.is_err());
}
