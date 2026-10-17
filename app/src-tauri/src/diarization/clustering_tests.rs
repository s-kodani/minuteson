use super::*;
use std::cell::Cell;

fn key(run: usize, speaker: u32) -> LocalSpeaker {
    LocalSpeaker { run, speaker }
}
fn feature(run: usize, speaker: u32, vector: &[f32]) -> Feature {
    Feature {
        speaker: key(run, speaker),
        samples: 16_000,
        vector: vector.to_vec(),
    }
}
fn labels(result: &Mapping) -> Vec<Option<u32>> {
    result.groups.iter().map(|(_, label)| *label).collect()
}

#[test]
fn swapped_run_local_numbers_follow_embeddings_not_numeric_identity() {
    let groups = [key(0, 0), key(0, 1), key(1, 0), key(1, 1)];
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(0, 1, &[0.0, 1.0]),
        feature(1, 0, &[0.0, 1.0]),
        feature(1, 1, &[1.0, 0.0]),
    ];
    let result = cluster(
        &groups,
        &features,
        &[],
        Some(2),
        500,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(labels(&result), vec![Some(0), Some(1), Some(1), Some(0)]);
    assert_eq!(result.observed_speakers, 2);
    assert_eq!(result.unresolved_groups, 0);
}

#[test]
fn equal_local_numbers_with_different_voices_remain_distinct() {
    let result = cluster(
        &[key(0, 0), key(1, 0)],
        &[feature(0, 0, &[1.0, 0.0]), feature(1, 0, &[0.0, 1.0])],
        &[],
        None,
        500,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(labels(&result), vec![Some(0), Some(1)]);
}

#[test]
fn scaling_vectors_does_not_change_cosine_identity() {
    let result = cluster(
        &[key(0, 0), key(1, 9)],
        &[feature(0, 0, &[0.25, 0.0]), feature(1, 9, &[100.0, 0.0])],
        &[],
        None,
        0,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(labels(&result), vec![Some(0), Some(0)]);
}

#[test]
fn complete_linkage_does_not_chain_distant_voices_through_an_intermediate() {
    let angle = std::f32::consts::PI / 6.0;
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(1, 0, &[angle.cos(), angle.sin()]),
        feature(2, 0, &[(2.0 * angle).cos(), (2.0 * angle).sin()]),
    ];
    let result = cluster(
        &[key(0, 0), key(1, 0), key(2, 0)],
        &features,
        &[],
        None,
        200,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(result.observed_speakers, 2);
    assert_ne!(result.groups[0].1, result.groups[2].1);
}

#[test]
fn overlap_constraints_survive_cross_run_merges() {
    let groups = [key(0, 0), key(0, 1), key(1, 0)];
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(0, 1, &[1.0, 0.0]),
        feature(1, 0, &[1.0, 0.0]),
    ];
    let overlap = [(key(0, 0), key(0, 1))];
    let result = cluster(
        &groups,
        &features,
        &overlap,
        None,
        2000,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(labels(&result), vec![Some(0), Some(1), Some(0)]);
    assert!(cluster(
        &groups,
        &features,
        &overlap,
        Some(1),
        2000,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn requested_count_cuts_one_global_track_clustering_not_each_run() {
    let groups = [key(0, 0), key(0, 1), key(1, 0), key(1, 1)];
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(0, 1, &[0.0, 1.0]),
        feature(1, 0, &[1.0, 0.0]),
        feature(1, 1, &[0.0, 1.0]),
    ];
    for count in 1..=4 {
        let result = cluster(
            &groups,
            &features,
            &[],
            Some(count),
            0,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(result.observed_speakers, count);
    }
    let result = cluster(
        &groups,
        &features,
        &[],
        Some(10),
        500,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(result.observed_speakers, 4); // Never fabricate six people.
}

#[test]
fn unavailable_features_and_cancelling_centroids_remain_unresolved() {
    let groups = [key(0, 0), key(1, 0), key(2, 0)];
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(2, 0, &[1.0, 0.0]),
        feature(2, 0, &[-1.0, 0.0]),
    ];
    let result = cluster(
        &groups,
        &features,
        &[],
        Some(3),
        500,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(labels(&result), vec![Some(0), None, None]);
    assert_eq!(result.unresolved_groups, 2);
    assert_eq!(result.observed_speakers, 1);
    assert_eq!(
        cluster(
            &[],
            &[],
            &[],
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .observed_speakers,
        0
    );
    assert_eq!(
        cluster(
            &groups,
            &[],
            &[],
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .unresolved_groups,
        3
    );
}

#[test]
fn selected_real_sample_counts_weight_each_group_centroid() {
    let groups = [key(0, 0), key(1, 0)];
    let mut longer = feature(0, 0, &[1.0, 0.0]);
    longer.samples *= 100;
    let features = [
        longer,
        feature(0, 0, &[0.0, 1.0]),
        feature(1, 0, &[1.0, 0.0]),
    ];
    assert_eq!(
        cluster(
            &groups,
            &features,
            &[],
            None,
            1,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .observed_speakers,
        1
    );
}

#[test]
fn canonical_ties_are_repeatable_without_guessing_probability() {
    let groups = [key(0, 0), key(1, 0), key(2, 0)];
    let features = [
        feature(0, 0, &[1.0, 0.0]),
        feature(1, 0, &[1.0, 0.0]),
        feature(2, 0, &[1.0, 0.0]),
    ];
    for _ in 0..20 {
        assert_eq!(
            labels(
                &cluster(
                    &groups,
                    &features,
                    &[],
                    Some(2),
                    500,
                    &crate::test_support::never_cancelled
                )
                .unwrap()
            ),
            vec![Some(0), Some(0), Some(1)]
        );
    }
}

#[test]
fn malformed_groups_features_constraints_and_counts_are_rejected() {
    let valid = feature(0, 0, &[1.0, 0.0]);
    for groups in [
        vec![key(0, 0), key(0, 0)],
        vec![key(1, 0), key(0, 0)],
        vec![key(4098, 0)],
        vec![key(0, 100)],
    ] {
        assert!(cluster(
            &groups,
            &[],
            &[],
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    for feature in [
        feature(1, 0, &[1.0, 0.0]),
        feature(0, 0, &[]),
        feature(0, 0, &[0.0, 0.0]),
        feature(0, 0, &[f32::NAN, 0.0]),
        feature(0, 0, &[f32::INFINITY, 1.0]),
        feature(0, 0, &[f32::NEG_INFINITY, 1.0]),
    ] {
        assert!(cluster(
            &[key(0, 0)],
            &[feature],
            &[],
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    assert!(cluster(
        &[key(0, 0)],
        &[valid, feature(0, 0, &[1.0])],
        &[],
        None,
        500,
        &crate::test_support::never_cancelled
    )
    .is_err());
    for count in [0, 101, u32::MAX] {
        assert!(cluster(
            &[],
            &[],
            &[],
            Some(count),
            500,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    assert!(cluster(
        &[],
        &[],
        &[],
        None,
        2001,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let groups = [key(0, 0), key(0, 1), key(1, 0)];
    for overlap in [
        vec![(key(0, 0), key(0, 0))],
        vec![(key(0, 1), key(0, 0))],
        vec![(key(0, 0), key(1, 0))],
        vec![(key(0, 0), key(0, 2))],
        vec![(key(0, 0), key(0, 1)), (key(0, 0), key(0, 1))],
    ] {
        assert!(cluster(
            &groups,
            &[],
            &overlap,
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
}

#[test]
fn oversized_dimensions_counts_and_sample_times_fail_before_pair_allocation() {
    let groups: Vec<_> = (0..MAX_GROUPS + 1).map(|run| key(run, 0)).collect();
    assert!(cluster(
        &groups,
        &[],
        &[],
        None,
        500,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let too_large = feature(0, 0, &vec![1.0; MAX_DIMENSION + 1]);
    assert!(cluster(
        &[key(0, 0)],
        &[too_large],
        &[],
        None,
        500,
        &crate::test_support::never_cancelled
    )
    .is_err());
    for samples in [0, MAX_SAMPLES + 1, u64::MAX] {
        let mut input = feature(0, 0, &[1.0]);
        input.samples = samples;
        assert!(cluster(
            &[key(0, 0)],
            &[input],
            &[],
            None,
            500,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    let features: Vec<_> = (0..MAX_FEATURES + 1)
        .map(|_| feature(0, 0, &[1.0]))
        .collect();
    assert!(cluster(
        &[key(0, 0)],
        &features,
        &[],
        None,
        500,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let features: Vec<_> = (0..2049).map(|_| feature(0, 0, &vec![1.0; 4096])).collect();
    assert!(cluster(
        &[key(0, 0)],
        &features,
        &[],
        None,
        500,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn cancellation_is_checked_during_features_distances_and_merges() {
    let groups: Vec<_> = (0..64).map(|run| key(run, 0)).collect();
    let features: Vec<_> = (0..64).map(|run| feature(run, 0, &[1.0, 0.0])).collect();
    let total = Cell::new(0);
    cluster(&groups, &features, &[], Some(1), 500, &|| {
        total.set(total.get() + 1);
        false
    })
    .unwrap();
    assert!(total.get() > 256);
    for limit in [
        0,
        3,
        total.get() / 4,
        total.get() / 2,
        total.get() * 3 / 4,
        total.get() - 1,
    ] {
        let calls = Cell::new(0);
        assert!(
            cluster(&groups, &features, &[], Some(1), 500, &|| {
                let call = calls.get();
                calls.set(call + 1);
                call >= limit
            })
            .is_err(),
            "cancellation check {limit}"
        );
    }
}

#[test]
fn heap_merges_match_independent_exhaustive_complete_linkage() {
    // Small exhaustive matrices independently scan every live pair at each
    // merge. This catches stale-edge and transitive-constraint errors without
    // making the production path allocate a second matrix or rescan all pairs.
    fn reference(
        features: &[Feature],
        forbidden: &[(usize, usize)],
        requested: Option<u32>,
        threshold: u32,
    ) -> Vec<Option<u32>> {
        let n = features.len();
        let vectors: Vec<Vec<f64>> = features
            .iter()
            .map(|f| {
                let raw: Vec<_> = f.vector.iter().map(|v| f64::from(*v)).collect();
                let norm = raw.iter().map(|v| v * v).sum::<f64>().sqrt();
                raw.iter().map(|v| v / norm).collect()
            })
            .collect();
        let mut distances = vec![vec![f64::INFINITY; n]; n];
        for a in 0..n {
            for b in a + 1..n {
                if !forbidden.contains(&(a, b)) {
                    distances[a][b] = (1.0
                        - vectors[a]
                            .iter()
                            .zip(&vectors[b])
                            .map(|(a, b)| a * b)
                            .sum::<f64>())
                    .clamp(0.0, 2.0);
                    distances[b][a] = distances[a][b];
                }
            }
        }
        let mut active = vec![true; n];
        let mut roots: Vec<_> = (0..n).collect();
        let mut count = n;
        while count > requested.map_or(1, |v| (v as usize).min(n)) {
            let mut best: Option<(f64, usize, usize)> = None;
            for a in 0..n {
                for b in a + 1..n {
                    if active[a] && active[b] && distances[a][b].is_finite() {
                        let next = (distances[a][b], a, b);
                        if best.is_none_or(|best| next < best) {
                            best = Some(next);
                        }
                    }
                }
            }
            let Some((distance, a, b)) = best else {
                break;
            };
            if requested.is_none() && distance > f64::from(threshold) / 1000.0 {
                break;
            }
            active[b] = false;
            for root in &mut roots {
                if *root == b {
                    *root = a;
                }
            }
            count -= 1;
            for c in 0..n {
                if active[c] && c != a {
                    distances[a][c] = distances[a][c].max(distances[b][c]);
                    distances[c][a] = distances[a][c];
                }
            }
        }
        let names: Vec<_> = (0..n).filter(|i| active[*i]).collect();
        roots
            .iter()
            .map(|root| Some(names.iter().position(|name| name == root).unwrap() as u32))
            .collect()
    }
    let mut seed = 8379_u64;
    for trial in 0..40 {
        let groups: Vec<_> = (0..12).map(|i| key(i / 3, (i % 3) as u32)).collect();
        let features: Vec<_> = groups
            .iter()
            .map(|g| {
                let vector: Vec<_> = (0..7)
                    .map(|_| {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                        ((seed >> 32) as i32) as f32 / i32::MAX as f32
                    })
                    .collect();
                feature(g.run, g.speaker, &vector)
            })
            .collect();
        let forbidden: Vec<_> = if trial % 2 == 0 {
            vec![(0, 1), (3, 4), (6, 7), (9, 10)]
        } else {
            vec![]
        };
        let overlaps: Vec<_> = forbidden
            .iter()
            .map(|(a, b)| (groups[*a], groups[*b]))
            .collect();
        for (requested, threshold) in [(None, 350), (None, 1100), (Some(4), 0)] {
            let expected = reference(&features, &forbidden, requested, threshold);
            let actual = cluster(&groups, &features, &overlaps, requested, threshold, &|| {
                false
            })
            .unwrap();
            assert_eq!(
                labels(&actual),
                expected,
                "trial {trial}, requested {requested:?}"
            );
        }
    }
}

#[test]
fn stale_edges_cannot_bypass_complete_linkage_or_overlap_guards() {
    let groups: Vec<_> = (0..160).map(|run| key(run, 0)).collect();
    let features: Vec<_> = (0..160)
        .map(|run| {
            let angle = if run % 2 == 0 {
                0.0
            } else {
                std::f32::consts::FRAC_PI_2
            };
            feature(run, 0, &[angle.cos(), angle.sin()])
        })
        .collect();
    let result = cluster(
        &groups,
        &features,
        &[],
        None,
        200,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(result.observed_speakers, 2);
    for (index, (_, label)) in result.groups.iter().enumerate() {
        assert_eq!(*label, Some((index % 2) as u32));
    }
}

#[test]
fn packed_pairs_are_unique_and_heap_records_remain_bounded() {
    assert_eq!(std::mem::size_of::<Edge>(), 16);
    for n in 2..100 {
        let mut seen = vec![false; n * (n - 1) / 2];
        for left in 0..n {
            for right in left + 1..n {
                let p = position(n, left, right);
                assert_eq!(p, position(n, right, left));
                assert!(!seen[p]);
                seen[p] = true;
            }
        }
        assert!(seen.into_iter().all(|seen| seen));
    }
}

fn turn(run: usize, first: u64, end: u64, speaker: u32) -> Turn {
    Turn {
        run,
        first,
        end,
        speaker,
    }
}

#[test]
fn support_excludes_other_voices_and_keeps_only_actual_overlap_constraints() {
    let turns = [
        turn(0, 0, 1000, 0),
        turn(0, 400, 600, 1),
        turn(1, 0, 1000, 0),
    ];
    let result = support(
        &[(0, 1000), (1, 1000)],
        &turns,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(result.groups, vec![key(0, 0), key(0, 1), key(1, 0)]);
    assert_eq!(result.overlaps, vec![(key(0, 0), key(0, 1))]);
    assert_eq!(
        result.windows,
        vec![
            FeatureWindow {
                speaker: key(0, 0),
                first: 0,
                end: 400
            },
            FeatureWindow {
                speaker: key(0, 0),
                first: 600,
                end: 1000
            },
            FeatureWindow {
                speaker: key(1, 0),
                first: 0,
                end: 1000
            },
        ]
    );
    // Entirely overlapping speaker 1 has no clean embedding window. It is
    // retained as a group, never inferred from the mixed samples or deleted.
    assert!(!result
        .windows
        .iter()
        .any(|window| window.speaker == key(0, 1)));
}

#[test]
fn touching_and_nested_same_voice_intervals_do_not_invent_overlap() {
    let result = support(
        &[(0, 1000)],
        &[
            turn(0, 0, 500, 0),
            turn(0, 100, 300, 0),
            turn(0, 500, 1000, 1),
        ],
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(result.overlaps.is_empty());
    assert_eq!(
        result.windows,
        vec![
            FeatureWindow {
                speaker: key(0, 0),
                first: 0,
                end: 500
            },
            FeatureWindow {
                speaker: key(0, 1),
                first: 500,
                end: 1000
            },
        ]
    );
}

#[test]
fn support_never_joins_gap_separated_runs_or_disjoint_speech_into_one_window() {
    let result = support(
        &[(0, 1000), (3, 1000)],
        &[
            turn(0, 0, 100, 0),
            turn(0, 200, 500, 0),
            turn(0, 600, 1000, 0),
            turn(3, 0, 1000, 0),
        ],
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        result.windows,
        vec![
            FeatureWindow {
                speaker: key(0, 0),
                first: 200,
                end: 500
            },
            FeatureWindow {
                speaker: key(0, 0),
                first: 600,
                end: 1000
            },
            FeatureWindow {
                speaker: key(3, 0),
                first: 0,
                end: 1000
            },
        ]
    );
}

#[test]
fn longest_ties_use_original_time_and_long_windows_are_centered_and_bounded() {
    let result = support(
        &[(0, 2_000_000)],
        &[
            turn(0, 0, 100, 0),
            turn(0, 200, 300, 0),
            turn(0, 400, 500, 0),
            turn(0, 600_000, 2_000_000, 1),
        ],
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        result.windows,
        vec![
            FeatureWindow {
                speaker: key(0, 0),
                first: 0,
                end: 100
            },
            FeatureWindow {
                speaker: key(0, 0),
                first: 200,
                end: 300
            },
            FeatureWindow {
                speaker: key(0, 1),
                first: 1_060_000,
                end: 1_540_000
            },
        ]
    );
}

#[test]
fn empty_support_cancellation_and_out_of_run_frames_fail_closed() {
    assert!(
        support(&[(0, 1000)], &[], &crate::test_support::never_cancelled)
            .unwrap()
            .groups
            .is_empty()
    );
    for runs in [
        vec![(0, 0)],
        vec![(0, u64::MAX)],
        vec![(0, 1000), (0, 1000)],
        vec![(1, 1000), (0, 1000)],
        vec![(4098, 1)],
    ] {
        assert!(support(&runs, &[], &crate::test_support::never_cancelled).is_err());
    }
    for turns in [
        vec![turn(0, 0, 1001, 0)],
        vec![turn(1, 0, 100, 0)],
        vec![turn(0, 100, 100, 0)],
        vec![turn(0, 0, 100, 100)],
        vec![turn(0, 0, 100, 0), turn(0, 0, 100, 0)],
        vec![turn(0, 100, 200, 0), turn(0, 0, 100, 0)],
    ] {
        assert!(support(&[(0, 1000)], &turns, &crate::test_support::never_cancelled).is_err());
    }
    let turns = [turn(0, 0, 1000, 0), turn(0, 400, 600, 1)];
    let count = Cell::new(0);
    support(&[(0, 1000)], &turns, &|| {
        count.set(count.get() + 1);
        false
    })
    .unwrap();
    for limit in [0, 2, count.get() / 2, count.get() - 1] {
        let calls = Cell::new(0);
        assert!(support(&[(0, 1000)], &turns, &|| {
            let n = calls.get();
            calls.set(n + 1);
            n >= limit
        })
        .is_err());
    }
}
