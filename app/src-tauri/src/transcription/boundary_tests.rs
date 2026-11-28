use super::{asr::Segment, Unit, UtteranceInput};

fn boundary_utterances(segments: [Segment; 2]) -> Vec<UtteranceInput> {
    const RATE: u64 = 16_000;
    let units = [
        Unit {
            track_index: 0,
            core_first: 0,
            core_end: 120 * RATE,
            window_first: 0,
            window_end: 122 * RATE,
            run_index: None,
        },
        Unit {
            track_index: 0,
            core_first: 120 * RATE,
            core_end: 240 * RATE,
            window_first: 118 * RATE,
            window_end: 242 * RATE,
            run_index: None,
        },
    ];
    units
        .iter()
        .zip(segments)
        .filter_map(|(unit, segment)| unit.utterance(segment, RATE, "processed", 0, 242_000_000))
        .collect()
}

#[test]
fn current_midpoint_ownership_duplicates_boundary_words_when_both_segments_are_kept() {
    // #128 の既知の問題条件を合成出力で記録する。境界の同じ「確認します」が
    // 前窓（中点119.25秒）と次窓（中点120.9秒）の両方から採用される。
    let utterances = boundary_utterances([
        Segment {
            start_us: 118_000_000,
            end_us: 120_500_000,
            text: "では確認します".into(),
        },
        Segment {
            start_us: 1_800_000,
            end_us: 4_000_000,
            text: "確認します。次の議題です".into(),
        },
    ]);
    let published: Vec<_> = utterances
        .iter()
        .map(|u| (u.start_us, u.end_us, u.text.as_str()))
        .collect();
    assert_eq!(
        published,
        [
            (118_000_000, 120_500_000, "では確認します"),
            (119_800_000, 122_000_000, "確認します。次の議題です"),
        ]
    );
}

#[test]
fn current_midpoint_ownership_drops_boundary_words_when_both_segments_are_rejected() {
    // #128 の既知の問題条件を合成出力で記録する。前窓の中点は120.9秒、
    // 次窓の中点は119.25秒なので、両方の窓にある「確認します」が欠落する。
    let utterances = boundary_utterances([
        Segment {
            start_us: 119_800_000,
            end_us: 122_000_000,
            text: "確認します。次の議題です".into(),
        },
        Segment {
            start_us: 0,
            end_us: 2_500_000,
            text: "では確認します".into(),
        },
    ]);
    assert!(utterances.is_empty());
}
