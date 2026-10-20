use super::*;
#[test]
fn meeting_limit_preserves_unknown_time_and_bounds_short_input_labels() {
    let mut segments = vec![
        Segment {
            start_us: 0,
            end_us: 2_000_000,
            speaker: 4,
        },
        Segment {
            start_us: 4_000_000,
            end_us: 5_000_000,
            speaker: 8,
        },
    ];
    sherpa::limit_speakers(&mut segments, Some(1));
    assert!(segments.iter().all(|s| s.speaker == 0));
    let timeline = Timeline::build(&segments, 6_000_000).unwrap();
    assert_eq!(
        timeline.attribute(2_000_000, 4_000_000).unwrap(),
        Attribution::Unknown
    );
}

#[test]
fn sparse_native_labels_and_duplicate_turns_still_respect_the_meeting_limit() {
    for n in [1, 2, 100] {
        let mut segments = vec![
            Segment {
                start_us: 0,
                end_us: 2_000_000,
                speaker: 1000,
            },
            Segment {
                start_us: 0,
                end_us: 2_000_000,
                speaker: 1001,
            },
            Segment {
                start_us: 4_000_000,
                end_us: 5_000_000,
                speaker: 1002,
            },
        ];
        let merges = sherpa::limit_speakers(&mut segments, Some(n));
        assert!(segments.iter().all(|s| s.speaker < n));
        assert_eq!(
            merges.len(),
            if n == 1 {
                2
            } else if n == 2 {
                1
            } else {
                0
            }
        );
    }
    let mut segments = vec![];
    assert!(sherpa::limit_speakers(&mut segments, Some(100)).is_empty());
}

#[test]
fn excess_native_identity_is_merged_consistently_with_speaker_order_breaking_ties() {
    let mut segments = vec![
        Segment {
            start_us: 0,
            end_us: 3_000_000,
            speaker: 0,
        },
        Segment {
            start_us: 3_000_000,
            end_us: 4_000_000,
            speaker: 2,
        },
        Segment {
            start_us: 6_000_000,
            end_us: 7_000_000,
            speaker: 2,
        },
        Segment {
            start_us: 7_000_000,
            end_us: 10_000_000,
            speaker: 1,
        },
    ];
    let merges = sherpa::limit_speakers(&mut segments, Some(2));
    assert_eq!(merges.len(), 2);
    assert!(merges
        .iter()
        .all(|m| m.from_speaker == 2 && m.into_speaker == 0));
    assert!(segments[1..3].iter().all(|s| s.speaker == 0));
}
