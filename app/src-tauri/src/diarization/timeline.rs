//! Keep temporal overlap distinct from multiple sequential speakers inside
//! one ASR span. No word alignment is available here, so text is never split
//! or assigned to a guessed new boundary.
use super::sherpa::{Segment, MAX_SPEAKERS};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Interval {
    pub start_us: i64,
    pub end_us: i64,
    pub speakers: Vec<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Timeline {
    pub duration_us: i64,
    pub intervals: Vec<Interval>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum Attribution {
    Unknown,
    Assigned {
        speaker: u32,
    },
    // All observed speakers participate in actual temporal overlap.
    Overlap {
        speakers: Vec<u32>,
    },
    // Multiple sequential speakers, or a turn before/after another overlap.
    // Keep text intact and unknown until the user can inspect raw intervals.
    NeedsSplit {
        speakers: Vec<u32>,
        overlaps: Vec<Interval>,
    },
}

impl Timeline {
    pub(crate) fn build(segments: &[Segment], duration_us: i64) -> Result<Self, String> {
        if duration_us <= 0 || segments.len() > 100_000 {
            return Err("話者区間の件数または音声時間が不正です".into());
        }
        let mut events = Vec::with_capacity(segments.len() * 2);
        for segment in segments {
            if segment.start_us < 0
                || segment.end_us <= segment.start_us
                || segment.end_us > duration_us
                || segment.speaker >= MAX_SPEAKERS
            {
                return Err("話者区間の時刻・ラベルが不正です".into());
            }
            events.push((segment.start_us, segment.speaker, 1_i32));
            events.push((segment.end_us, segment.speaker, -1_i32));
        }
        events.sort_unstable();
        let mut active = BTreeMap::<u32, i32>::new();
        let mut intervals: Vec<Interval> = Vec::new();
        let mut previous = 0;
        let mut index = 0;
        // O(n log n + n*k) with k <= 100, not a pairwise overlap matrix.
        while index < events.len() {
            let time = events[index].0;
            if time > previous && !active.is_empty() {
                let speakers: Vec<_> = active.keys().copied().collect();
                if let Some(last) = intervals
                    .last_mut()
                    .filter(|last| last.end_us == previous && last.speakers == speakers)
                {
                    last.end_us = time;
                } else {
                    intervals.push(Interval {
                        start_us: previous,
                        end_us: time,
                        speakers,
                    });
                }
            }
            // Apply all ends/starts at one instant before emitting any next
            // interval. Touching boundaries have zero overlap duration.
            while index < events.len() && events[index].0 == time {
                let (_, speaker, delta) = events[index];
                let count = active.entry(speaker).or_default();
                *count += delta;
                if *count == 0 {
                    active.remove(&speaker);
                }
                index += 1;
            }
            previous = time;
        }
        Ok(Self {
            duration_us,
            intervals,
        })
    }

    pub(crate) fn attribute(&self, start_us: i64, end_us: i64) -> Result<Attribution, String> {
        if start_us < 0 || end_us <= start_us || end_us > self.duration_us {
            return Err("発言と話者区間の照合範囲が不正です".into());
        }
        let first = self
            .intervals
            .partition_point(|interval| interval.end_us <= start_us);
        let mut observed = BTreeSet::new();
        let mut simultaneous = BTreeSet::new();
        let mut overlaps = Vec::new();
        for interval in self.intervals[first..]
            .iter()
            .take_while(|interval| interval.start_us < end_us)
        {
            observed.extend(interval.speakers.iter().copied());
            if interval.speakers.len() > 1 {
                simultaneous.extend(interval.speakers.iter().copied());
                overlaps.push(Interval {
                    start_us: interval.start_us.max(start_us),
                    end_us: interval.end_us.min(end_us),
                    speakers: interval.speakers.clone(),
                });
            }
        }
        let speakers: Vec<_> = observed.iter().copied().collect();
        Ok(match speakers.len() {
            0 => Attribution::Unknown,
            1 => Attribution::Assigned {
                speaker: speakers[0],
            },
            _ if simultaneous == observed => Attribution::Overlap { speakers },
            _ => Attribution::NeedsSplit { speakers, overlaps },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(start_us: i64, end_us: i64, speaker: u32) -> Segment {
        Segment {
            start_us,
            end_us,
            speaker,
        }
    }

    #[test]
    fn touching_turns_are_not_overlap_and_text_boundary_is_not_guessed() {
        let timeline =
            Timeline::build(&[segment(100, 500, 0), segment(500, 900, 1)], 1000).unwrap();
        assert_eq!(
            timeline.attribute(100, 500).unwrap(),
            Attribution::Assigned { speaker: 0 }
        );
        assert_eq!(
            timeline.attribute(100, 900).unwrap(),
            Attribution::NeedsSplit {
                speakers: vec![0, 1],
                overlaps: vec![]
            }
        );
        assert_eq!(timeline.attribute(900, 1000).unwrap(), Attribution::Unknown);
        assert!(timeline.attribute(-1, 100).is_err());
        assert!(timeline.attribute(0, 1001).is_err());
    }

    #[test]
    fn nested_same_speaker_segments_are_not_additional_people() {
        let timeline = Timeline::build(
            &[
                segment(0, 1000, 0),
                segment(100, 500, 0),
                segment(400, 700, 1),
            ],
            1000,
        )
        .unwrap();
        assert_eq!(
            timeline.intervals,
            vec![
                Interval {
                    start_us: 0,
                    end_us: 400,
                    speakers: vec![0]
                },
                Interval {
                    start_us: 400,
                    end_us: 700,
                    speakers: vec![0, 1]
                },
                Interval {
                    start_us: 700,
                    end_us: 1000,
                    speakers: vec![0]
                },
            ]
        );
        assert_eq!(
            timeline.attribute(0, 1000).unwrap(),
            Attribution::Overlap {
                speakers: vec![0, 1]
            }
        );
        assert_eq!(
            timeline.attribute(0, 400).unwrap(),
            Attribution::Assigned { speaker: 0 }
        );
    }

    #[test]
    fn sequential_third_speaker_does_not_get_a_false_overlap_assignment() {
        let timeline = Timeline::build(
            &[
                segment(0, 400, 0),
                segment(200, 500, 1),
                segment(500, 900, 2),
            ],
            1000,
        )
        .unwrap();
        assert_eq!(
            timeline.attribute(100, 800).unwrap(),
            Attribution::NeedsSplit {
                speakers: vec![0, 1, 2],
                overlaps: vec![Interval {
                    start_us: 200,
                    end_us: 400,
                    speakers: vec![0, 1]
                }],
            }
        );
        assert_eq!(
            timeline.attribute(300, 350).unwrap(),
            Attribution::Overlap {
                speakers: vec![0, 1]
            }
        );
    }

    #[test]
    fn empty_and_invalid_intervals_are_handled_without_inventing_people() {
        assert_eq!(
            Timeline::build(&[], 1000)
                .unwrap()
                .attribute(0, 1000)
                .unwrap(),
            Attribution::Unknown
        );
        for segment in [
            segment(-1, 100, 0),
            segment(0, 0, 0),
            segment(0, 1001, 0),
            segment(0, 100, 100),
        ] {
            assert!(Timeline::build(&[segment], 1000).is_err());
        }
    }
}
