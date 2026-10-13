use super::*;

#[test]
fn markdown_interval_preserves_fractional_origin_resampling_phase_and_exact_bounds() {
    let samples: Vec<_> = (0..44100).map(|n| (n as f32 * 0.037).sin() * 0.3).collect();
    let source = || audio(44100, 1, 0, "imported", &samples, 14700);
    let mut whole = MeetingAudio::new(vec![source()], 1_000_000).unwrap();
    let mut interval = MeetingIntervalAudio::new(
        MeetingAudio::new(vec![source()], 1_000_000).unwrap(),
        123456,
        765432,
    )
    .unwrap();
    let first = output_frame(123456).unwrap();
    assert_eq!(interval.total_frames, output_frame(765432).unwrap() - first);
    for (offset, count) in [(0, 1000), (1000, 2048), (interval.total_frames - 2, 2)] {
        assert_eq!(
            interval.read_pcm(offset, count).unwrap(),
            whole.read_pcm(first + offset, count).unwrap()
        );
    }
    assert!(interval.read_pcm(interval.total_frames, 1).is_err());
    assert!(interval.read_pcm(0, interval.total_frames + 1).is_err());
    assert!(interval.read_pcm(u64::MAX, 1).is_err());
}

#[test]
fn markdown_interval_clips_gaps_to_relative_frames_and_never_pads_or_skips_them() {
    let source = audio(48000, 1, 0, "mic", &vec![0.25; 48000], 24000);
    let gap =
        PlaybackMissingRange::new("mic", 200000, 300000, "capture_timestamp_gap", 1000000).unwrap();
    let whole = MeetingAudio::with_missing_ranges(vec![source], 1000000, vec![gap]).unwrap();
    let interval = MeetingIntervalAudio::new(whole, 250001, 700003).unwrap();
    let first = output_frame(250001).unwrap();
    let last = output_frame(700003).unwrap();
    assert_eq!(interval.missing_ranges[0].start_us, 250001);
    assert_eq!(interval.missing_ranges[0].end_us, 300000);
    assert_eq!(interval.missing_ranges[0].start_frame, 0);
    assert_eq!(interval.missing_ranges[0].end_frame, 14400 - first);
    let store = PlaybackStore::default();
    let info = store.insert_interval(interval).unwrap();
    assert_eq!(info.start_us, 250001);
    assert_eq!(info.end_us, 700003);
    assert_eq!(info.total_frames, last - first);
    assert_eq!(info.alignment, "unverified");
    assert!(store.read(&info.token, 0).is_err());
    let packet = store.read(&info.token, 14400 - first).unwrap();
    assert_eq!(packet.len(), 12 + (last - 14400) as usize * 4);
    assert!(values(&packet[12..]).iter().all(|v| *v == 0.25));
}

#[test]
fn markdown_interval_rejects_invalid_ranges_and_ranges_with_no_physical_frame() {
    for (start, end) in [(-1, 10), (10, 10), (10, 0), (0, 1000001), (1, 2)] {
        let source = audio(48000, 1, 0, "mic", &vec![0.0; 48000], 48000);
        assert!(MeetingIntervalAudio::new(
            MeetingAudio::new(vec![source], 1000000).unwrap(),
            start,
            end
        )
        .is_err());
    }
}

#[test]
fn markdown_interval_sorts_overlapping_track_gaps_again_after_clipping() {
    let mic = audio(48000, 1, 0, "mic", &vec![0.25; 48000], 48000);
    let system = audio(48000, 1, 0, "system", &vec![0.5; 48000], 48000);
    let gaps = vec![
        PlaybackMissingRange::new("mic", 0, 500000, "capture_start_gap", 1000000).unwrap(),
        PlaybackMissingRange::new("system", 100000, 300000, "capture_timestamp_gap", 1000000)
            .unwrap(),
    ];
    let whole = MeetingAudio::with_missing_ranges(vec![mic, system], 1000000, gaps).unwrap();
    let interval = MeetingIntervalAudio::new(whole, 200001, 700003).unwrap();
    assert_eq!(interval.missing_ranges[0].track_kind, "system");
    assert_eq!(interval.missing_ranges[1].track_kind, "mic");
    assert!(interval
        .missing_ranges
        .iter()
        .all(|gap| gap.start_frame == 0 && gap.start_us == 200001));
    let store = PlaybackStore::default();
    let info = store.insert_interval(interval).unwrap();
    assert!(store.read(&info.token, 0).is_err());
    assert!(store
        .read(&info.token, 14400 - output_frame(200001).unwrap())
        .is_err());
    assert!(store
        .read(&info.token, 24000 - output_frame(200001).unwrap())
        .is_ok());
}
