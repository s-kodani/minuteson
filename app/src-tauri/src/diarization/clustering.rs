//! Track-local matching across disconnected saved runs, never across meetings.
//! Local numeric labels alone carry no identity. The caller verifies model/run
//! provenance; this reducer checks the supplied feature values and constraints.
//! Actual overlap is a cannot-link constraint. No confidence is made.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

pub(super) const ALGORITHM: &str = "run_centroids_cosine_complete_v1";
const MAX_GROUPS: usize = 4096;
const MAX_FEATURES: usize = 8192;
const MAX_COMPONENTS: usize = 8 * 1024 * 1024;
const MAX_DIMENSION: usize = 4096;
const MAX_SAMPLES: u64 = 16_000 * 7 * 24 * 3600 * 100;
const MAX_SPEAKERS: u32 = 100;
const MAX_OVERLAPS: usize = MAX_GROUPS * (MAX_SPEAKERS as usize - 1) / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LocalSpeaker {
    pub run: usize,
    pub speaker: u32,
}

pub(super) struct Feature {
    pub speaker: LocalSpeaker,
    // Number of real 16 kHz samples used for this embedding, not gap duration.
    pub samples: u64,
    pub vector: Vec<f32>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Mapping {
    pub groups: Vec<(LocalSpeaker, Option<u32>)>,
    pub observed_speakers: u32,
    pub unresolved_groups: usize,
}

pub(super) const WINDOW_SELECTION: &str = "unambiguous_longest_two_center30_v1";
const MAX_TURNS: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Turn {
    pub run: usize,
    pub first: u64,
    pub end: u64,
    pub speaker: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FeatureWindow {
    pub speaker: LocalSpeaker,
    pub first: u64,
    pub end: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Support {
    pub groups: Vec<LocalSpeaker>,
    pub overlaps: Vec<(LocalSpeaker, LocalSpeaker)>,
    pub windows: Vec<FeatureWindow>,
}

/// Frame offsets belong to each independently converted, verified 16 kHz run.
/// No frame can cross a run boundary. Window selection keeps at most two real
/// contiguous spans per local speaker; the native extractor may still return
/// unavailable for short/invalid speech, which is not a matching identity.
pub(super) fn support(
    runs: &[(usize, u64)],
    turns: &[Turn],
    cancelled: &impl Fn() -> bool,
) -> Result<Support, String> {
    check(cancelled)?;
    if runs.len() > 4098
        || runs.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || runs.iter().any(|(run, frames)| {
            *run >= 4098 || *frames == 0 || *frames > u64::from((u32::MAX - 36) / 2)
        })
        || turns.len() > MAX_TURNS
        || turns.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("特徴量の保存区間・話者区間・順序が不正です".into());
    }
    let mut events = BTreeMap::<usize, Vec<(u64, u32, i32)>>::new();
    let mut groups = BTreeSet::new();
    for turn in turns {
        check(cancelled)?;
        let index = runs
            .binary_search_by_key(&turn.run, |(run, _)| *run)
            .map_err(|_| "話者区間の保存区間がありません")?;
        if turn.speaker >= MAX_SPEAKERS || turn.first >= turn.end || turn.end > runs[index].1 {
            return Err("話者区間が検証済み保存音声の外側にあります".into());
        }
        groups.insert(LocalSpeaker {
            run: turn.run,
            speaker: turn.speaker,
        });
        let events = events.entry(turn.run).or_default();
        events.push((turn.first, turn.speaker, 1));
        events.push((turn.end, turn.speaker, -1));
    }
    if groups.len() > MAX_GROUPS {
        return Err("保存区間の話者候補が多すぎます".into());
    }
    let mut overlaps = BTreeSet::new();
    let mut eligible = BTreeMap::<LocalSpeaker, Vec<(u64, u64)>>::new();
    for (run, mut events) in events {
        check(cancelled)?;
        events.sort_unstable();
        let mut active = BTreeMap::<u32, i32>::new();
        let mut previous = 0;
        let mut position = 0;
        while position < events.len() {
            check(cancelled)?;
            let time = events[position].0;
            if time > previous {
                if active.len() == 1 {
                    let speaker = LocalSpeaker {
                        run,
                        speaker: *active.keys().next().unwrap(),
                    };
                    let spans = eligible.entry(speaker).or_default();
                    if let Some(last) = spans.last_mut().filter(|(_, end)| *end == previous) {
                        last.1 = time;
                    } else {
                        spans.push((previous, time));
                    }
                } else if active.len() > 1 {
                    let speakers: Vec<_> = active.keys().copied().collect();
                    for (index, left) in speakers.iter().enumerate() {
                        for right in &speakers[index + 1..] {
                            overlaps.insert((
                                LocalSpeaker {
                                    run,
                                    speaker: *left,
                                },
                                LocalSpeaker {
                                    run,
                                    speaker: *right,
                                },
                            ));
                        }
                    }
                }
            }
            while position < events.len() && events[position].0 == time {
                let (_, speaker, delta) = events[position];
                let count = active.entry(speaker).or_default();
                *count += delta;
                if *count == 0 {
                    active.remove(&speaker);
                }
                position += 1;
            }
            if active.values().any(|count| *count < 0) {
                return Err("話者区間の重複回数が不正です".into());
            }
            previous = time;
        }
        if !active.is_empty() {
            return Err("話者区間の終端が閉じていません".into());
        }
    }
    let mut windows = Vec::new();
    for (speaker, mut spans) in eligible {
        check(cancelled)?;
        spans.sort_unstable_by_key(|(first, end)| (std::cmp::Reverse(end - first), *first));
        spans.truncate(2);
        spans.sort_unstable();
        for (first, end) in spans {
            let count = (end - first).min(30 * 16_000);
            let first = first + (end - first - count) / 2;
            windows.push(FeatureWindow {
                speaker,
                first,
                end: first + count,
            });
        }
    }
    check(cancelled)?;
    Ok(Support {
        groups: groups.into_iter().collect(),
        overlaps: overlaps.into_iter().collect(),
        windows,
    })
}

// Compact heap edges include both slot generations. Updating a cluster must
// never reuse the old distance left in the heap. Min distance, then canonical
// input order, wins; numeric local labels are not an identity shortcut.
#[derive(Clone, Copy)]
struct Edge {
    distance: f64,
    left: u16,
    right: u16,
    left_version: u16,
    right_version: u16,
}
impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Edge {}
impl PartialOrd for Edge {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Edge {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.left.cmp(&self.left))
            .then_with(|| other.right.cmp(&self.right))
            .then_with(|| other.left_version.cmp(&self.left_version))
            .then_with(|| other.right_version.cmp(&self.right_version))
    }
}

// Packed upper triangle, not one dense square matrix per speaker/run.
fn position(n: usize, left: usize, right: usize) -> usize {
    let (left, right) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    debug_assert!(left < right && right < n);
    left * (2 * n - left - 1) / 2 + right - left - 1
}

fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("保存区間の話者対応付けを中断しました".into())
    } else {
        Ok(())
    }
}

fn unit(vector: &[f64]) -> Option<Vec<f64>> {
    let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    (norm.is_finite() && norm > 1e-12).then(|| vector.iter().map(|value| value / norm).collect())
}

/// Inputs come from ONE track and one fixed embedding model. Groups and overlap
/// pairs have a canonical order; the caller retains model/run provenance. This
/// reducer consumes no PCM and neither concatenates nor pads missing audio.
pub(super) fn cluster(
    groups: &[LocalSpeaker],
    features: &[Feature],
    overlaps: &[(LocalSpeaker, LocalSpeaker)],
    requested: Option<u32>,
    threshold_milli: u32,
    cancelled: &impl Fn() -> bool,
) -> Result<Mapping, String> {
    check(cancelled)?;
    if groups.len() > MAX_GROUPS
        || features.len() > MAX_FEATURES
        || overlaps.len() > MAX_OVERLAPS
        || threshold_milli > 2000
        || requested.is_some_and(|count| !(1..=MAX_SPEAKERS).contains(&count))
        || groups.windows(2).any(|pair| pair[0] >= pair[1])
        || groups
            .iter()
            .any(|group| group.run >= 4098 || group.speaker >= MAX_SPEAKERS)
        || overlaps.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("保存区間の話者対応付けの構成・順序が不正です".into());
    }
    let mut forbidden = Vec::with_capacity(overlaps.len());
    for (index, &(left, right)) in overlaps.iter().enumerate() {
        if index % 1024 == 0 {
            check(cancelled)?;
        }
        if left >= right || left.run != right.run {
            return Err("同時発話の制約は同じ保存区間の異なる話者に限定します".into());
        }
        forbidden.push((
            groups
                .binary_search(&left)
                .map_err(|_| "重複話者がありません")?,
            groups
                .binary_search(&right)
                .map_err(|_| "重複話者がありません")?,
        ));
    }
    let mut sums: Vec<Option<Vec<f64>>> = vec![None; groups.len()];
    let mut weights = vec![0_u64; groups.len()];
    let mut dimension = None;
    let mut components = 0_usize;
    let mut total_samples = 0_u64;
    for feature in features {
        check(cancelled)?;
        components = components
            .checked_add(feature.vector.len())
            .ok_or("特徴量が大きすぎます")?;
        total_samples = total_samples
            .checked_add(feature.samples)
            .ok_or("特徴量の音声時間が範囲外です")?;
        if !(1..=MAX_DIMENSION).contains(&feature.vector.len())
            || components > MAX_COMPONENTS
            || feature.samples == 0
            || total_samples > MAX_SAMPLES
            || dimension
                .replace(feature.vector.len())
                .is_some_and(|previous| previous != feature.vector.len())
            || feature.vector.iter().any(|value| !value.is_finite())
        {
            return Err("固定した話者特徴量の次元・音声時間・数値が不正です".into());
        }
        let index = groups
            .binary_search(&feature.speaker)
            .map_err(|_| "特徴量の話者所属が不正です")?;
        let vector: Vec<_> = feature
            .vector
            .iter()
            .map(|value| f64::from(*value))
            .collect();
        let normalized = unit(&vector).ok_or("話者特徴量の長さがゼロです")?;
        let sum = sums[index].get_or_insert_with(|| vec![0.0; vector.len()]);
        weights[index] = weights[index]
            .checked_add(feature.samples)
            .ok_or("話者の音声時間が範囲外です")?;
        for (value, normalized) in sum.iter_mut().zip(normalized) {
            *value += normalized * feature.samples as f64;
        }
    }
    let mut centers = Vec::new();
    let mut native_to_active = vec![None; groups.len()];
    for (index, sum) in sums.into_iter().enumerate() {
        check(cancelled)?;
        // Conflicting feature directions can cancel. Preserve that group as
        // unresolved instead of making a synthetic identity or NaN distance.
        if let Some(vector) = sum.and_then(|sum| {
            unit(
                &sum.iter()
                    .map(|value| value / weights[index] as f64)
                    .collect::<Vec<_>>(),
            )
        }) {
            native_to_active[index] = Some(centers.len());
            centers.push(vector);
        }
    }
    let n = centers.len();
    let mut distances = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for left in 0..n {
        check(cancelled)?;
        for right in left + 1..n {
            if right % 256 == 0 {
                check(cancelled)?;
            }
            let similarity = centers[left]
                .iter()
                .zip(&centers[right])
                .map(|(a, b)| a * b)
                .sum::<f64>();
            distances.push((1.0 - similarity).clamp(0.0, 2.0));
        }
    }
    drop(centers);
    for (left, right) in forbidden {
        if let (Some(left), Some(right)) = (native_to_active[left], native_to_active[right]) {
            distances[position(n, left, right)] = f64::INFINITY;
        }
    }
    let mut active = vec![true; n];
    let mut versions = vec![0_u16; n];
    let mut parents: Vec<_> = (0..n).collect();
    let mut heap = BinaryHeap::with_capacity(distances.len());
    let edge = |left: usize, right: usize, distance, versions: &[u16]| Edge {
        distance,
        left: left as u16,
        right: right as u16,
        left_version: versions[left],
        right_version: versions[right],
    };
    for left in 0..n {
        check(cancelled)?;
        for right in left + 1..n {
            if right % 1024 == 0 {
                check(cancelled)?;
            }
            let distance = distances[position(n, left, right)];
            if distance.is_finite() {
                heap.push(edge(left, right, distance, &versions));
            }
        }
    }
    let valid = |edge: &Edge, active: &[bool], versions: &[u16]| {
        let (left, right) = (usize::from(edge.left), usize::from(edge.right));
        active[left]
            && active[right]
            && versions[left] == edge.left_version
            && versions[right] == edge.right_version
    };
    let mut remaining = n;
    let target = requested.map(|count| (count as usize).min(n));
    while remaining > target.unwrap_or(1) {
        check(cancelled)?;
        let selected = loop {
            let Some(next) = heap.pop() else {
                break None;
            };
            if valid(&next, &active, &versions) {
                break Some(next);
            }
            if heap.len() % 1024 == 0 {
                check(cancelled)?;
            }
        };
        let Some(selected) = selected else {
            if target.is_some() {
                return Err("指定人数へ統合すると同時発話の制約に反します".into());
            }
            break;
        };
        if target.is_none() && selected.distance > f64::from(threshold_milli) / 1000.0 {
            break;
        }
        let (left, right) = (usize::from(selected.left), usize::from(selected.right));
        versions[left] = versions[left]
            .checked_add(1)
            .ok_or("話者統合の世代が範囲外です")?;
        active[right] = false;
        parents[right] = left;
        remaining -= 1;
        for other in 0..n {
            if other % 256 == 0 {
                check(cancelled)?;
            }
            if other == left || !active[other] {
                continue;
            }
            let a = position(n, left, other);
            let b = position(n, right, other);
            // Complete linkage preserves every cannot-link pair transitively.
            // In particular, matching A to B must not later merge B with an
            // overlapping C merely because B/C centroids are close.
            distances[a] = distances[a].max(distances[b]);
            if distances[a].is_finite() {
                heap.push(edge(
                    left.min(other),
                    left.max(other),
                    distances[a],
                    &versions,
                ));
            }
        }
        let live_pairs = remaining * remaining.saturating_sub(1) / 2;
        if heap.len() > (2 * live_pairs + n).max(1024) {
            check(cancelled)?;
            heap.retain(|edge| valid(edge, &active, &versions));
            check(cancelled)?;
        }
    }
    if remaining > MAX_SPEAKERS as usize {
        return Err("特徴量から得た話者数が保存上限を超えています".into());
    }
    let labels: BTreeMap<_, _> = active
        .iter()
        .enumerate()
        .filter_map(|(index, active)| active.then_some(index))
        .enumerate()
        .map(|(label, index)| (index, label as u32))
        .collect();
    let mut mapped = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        check(cancelled)?;
        let label = native_to_active[index].map(|mut root| {
            while parents[root] != root {
                root = parents[root];
            }
            labels[&root]
        });
        mapped.push((*group, label));
    }
    check(cancelled)?;
    Ok(Mapping {
        unresolved_groups: mapped.iter().filter(|(_, label)| label.is_none()).count(),
        groups: mapped,
        observed_speakers: remaining as u32,
    })
}

#[cfg(test)]
#[path = "clustering_tests.rs"]
mod tests;
