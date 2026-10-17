use super::*;

fn interval(meeting: &str) -> MeetingIntervalRequest {
    MeetingIntervalRequest {
        meeting_id: meeting.into(),
        mode: MeetingPlaybackMode::Mic,
        start_us: 123456,
        end_us: 765432,
        expected_duration_us: 1000000,
    }
}

#[test]
fn markdown_interval_summary_is_read_only_and_lists_only_available_original_tracks() {
    let (_dir, storage, meeting) = fixture();
    let summary = storage.meeting_audio_summary(&meeting).unwrap();
    assert_eq!(summary.schema_version, 1);
    assert_eq!(summary.duration_us, 1000000);
    assert_eq!(
        serde_json::to_value(summary.modes).unwrap(),
        serde_json::json!(["mic"])
    );
    add_system(&storage, &meeting);
    assert_eq!(
        serde_json::to_value(storage.meeting_audio_summary(&meeting).unwrap().modes).unwrap(),
        serde_json::json!(["mic", "system", "both"])
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET state='failed' WHERE kind='system'",
            [],
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(storage.meeting_audio_summary(&meeting).unwrap().modes).unwrap(),
        serde_json::json!(["mic"])
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE recording_sessions SET state='recording'", [])
        .unwrap();
    assert!(storage.meeting_audio_summary(&meeting).is_err());
}

#[test]
fn markdown_interval_native_request_never_uses_another_mode_or_changed_duration() {
    let (_dir, storage, meeting) = fixture();
    let mut r = interval(&meeting);
    let audio = storage
        .prepare_meeting_interval_cancellable(&r, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(audio.start_us, r.start_us);
    assert_eq!(audio.end_us, r.end_us);
    for (start, end, duration) in [
        (-1, 1, 1000000),
        (2, 1, 1000000),
        (0, 1000001, 1000000),
        (0, 1000001, 1000001),
        (0, 1, i64::MAX),
    ] {
        r.start_us = start;
        r.end_us = end;
        r.expected_duration_us = duration;
        assert!(storage
            .prepare_meeting_interval_cancellable(&r, &crate::test_support::never_cancelled)
            .is_err());
    }
    r = interval(&meeting);
    r.mode = MeetingPlaybackMode::System;
    assert!(storage
        .prepare_meeting_interval_cancellable(&r, &crate::test_support::never_cancelled)
        .is_err());
    r = interval("other");
    assert!(storage
        .prepare_meeting_interval_cancellable(&r, &crate::test_support::never_cancelled)
        .is_err());
}

#[test]
fn markdown_interval_verifies_all_managed_sources_and_honors_cancellation() {
    let (_dir, storage, meeting) = fixture();
    let r = interval(&meeting);
    assert!(storage
        .prepare_meeting_interval_cancellable(&r, &|| true)
        .is_err());
    let count = Cell::new(0);
    assert!(storage
        .prepare_meeting_interval_cancellable(&r, &|| {
            count.set(count.get() + 1);
            count.get() > 3
        })
        .is_err());
    // Mutating a real registered file is confined to this temporary fixture.
    let path: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE meeting_id=?1 LIMIT 1",
            params![meeting],
            |row| row.get(0),
        )
        .unwrap();
    let file = storage.managed_path(Path::new(&path)).unwrap();
    let mut bytes = std::fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(file, bytes).unwrap();
    assert!(storage
        .prepare_meeting_interval_cancellable(&r, &crate::test_support::never_cancelled)
        .is_err());
}

#[test]
fn markdown_interval_request_rejects_paths_unknown_fields_and_modes() {
    let valid = serde_json::json!({"meetingId":"x","mode":"mic","startUs":1,"endUs":2,"expectedDurationUs":3});
    assert!(serde_json::from_value::<MeetingIntervalRequest>(valid.clone()).is_ok());
    for (key, value) in [
        ("path", serde_json::json!("/tmp/audio.caf")),
        ("analysisId", serde_json::json!("x")),
        ("mode", serde_json::json!("processed")),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(serde_json::from_value::<MeetingIntervalRequest>(invalid).is_err());
    }
}
