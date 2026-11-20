use super::*;

pub(crate) fn maximum_prefix() -> Prefix {
    // This tests admission-capacity geometry independently of disk size. The
    // production reader validates audio/receipts before this grouping runs.
    let stamp = Stamp {
        dev: 0,
        ino: 0,
        uid: 0,
        mode: 0,
        links: 1,
        size: 0,
        modified: (0, 0),
        changed: (0, 0),
        created: None,
    };
    let mut entries = Vec::new();
    let mut maps_total = 0_u32;
    for index in 0..MAX_SEGMENTS {
        let count = if index < 256 { 128_u32 } else { 1 };
        maps_total += count;
        let origin = i64::from(index) * 4_000_000_000;
        let frames = 64000_u64;
        let maps = (0..count)
            .map(|part| ClockSegment {
                source_frame_start: i64::from(part) * 64000 / i64::from(count),
                source_frame_end: i64::from(part + 1) * 64000 / i64::from(count),
                timeline_start_us: i64::from(part) * 4_000_000 / i64::from(count),
                timeline_end_us: i64::from(part + 1) * 4_000_000 / i64::from(count),
                mapping_json: "{\"schema_version\":1,\"method\":\"linear\"}".into(),
            })
            .collect::<Vec<_>>();
        let times = maps
            .iter()
            .map(|map| Endpoints {
                start_nanoseconds: origin + map.timeline_start_us * 1000,
                end_nanoseconds: origin + map.timeline_end_us * 1000,
            })
            .collect();
        entries.push(BoundEntry {
            entry: Entry {
                schema_version: 3,
                sample_encoding: Some("pcm16".into()),
                owner_id: "735027d4-a03e-43d3-830a-21e767dfaf88".into(),
                lane: "microphone".into(),
                sequence: index + 1,
                audio_name: format!("raw-microphone-{:08}.caf", index + 1),
                source_frame_start: u64::from(index) * frames,
                frames,
                sample_rate: 16000,
                channels: 1,
                capture_origin_nanoseconds: origin,
                clock_segments: maps,
                clock_nanoseconds: times,
                audio_bytes: 68 + frames * 2,
                audio_sha256: "a".repeat(64),
                previous_entry_sha256: None,
            },
            json_stamp: stamp.clone(),
            audio_stamp: stamp.clone(),
        });
    }
    assert!(maps_total <= MAX_TOTAL_MAPS as u32);
    Prefix {
        directory: tempfile::tempfile().unwrap(),
        tip: Tip {
            schema_version: 2,
            owner_id: "735027d4-a03e-43d3-830a-21e767dfaf88".into(),
            lane: "microphone".into(),
            sealed_segments: MAX_SEGMENTS,
            sealed_frames: u64::from(MAX_SEGMENTS) * 64000,
            last_entry_sha256: Some("a".repeat(64)),
            producer_closed: true,
        },
        tip_stamp: stamp,
        entries,
        truncated: false,
    }
}

#[test]
fn completed_grouping_fits_maximum_recording_and_dense_clocks_without_audio_allocation() {
    let prefix = maximum_prefix();
    let maps_total = prefix
        .entries
        .iter()
        .map(|entry| entry.entry.clock_segments.len())
        .sum::<usize>();
    let groups = prefix.flac_groups_with_seconds(0, 240).unwrap();
    assert!(groups.len() <= 1024);
    assert_eq!(
        groups.iter().map(|group| group.frames).sum::<u64>(),
        prefix.frames()
    );
    assert_eq!(
        groups
            .iter()
            .map(|group| group.clock_segments.len())
            .sum::<usize>(),
        maps_total
    );
    assert!(groups
        .iter()
        .all(|group| group.frames <= 240 * 16000 && group.clock_segments.len() <= 1024));
}
