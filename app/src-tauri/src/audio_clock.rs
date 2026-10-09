use serde::{Deserialize, Serialize};

const MICROS: u128 = 1_000_000;
const MAX_SAFE_FRAMES: u128 = (1_u128 << 53) - 1;
// Native raw segments retain independent affine fits. Bound a whole track's
// metadata separately from the unchanged 1,024-map per-chunk admission.
pub(crate) const MAX_CLOCK_SEGMENTS: usize = 65_536;
pub(crate) const MAX_CLOCK_MAPPING_BYTES: usize = 4 * 1024 * 1024;

/// Chunk-local frame endpoints. The enclosing chunk's immutable asset proof
/// binds these to samples; database row IDs are not part of the affine transform.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ClockSegment {
    pub source_frame_start: i64,
    pub source_frame_end: i64,
    pub timeline_start_us: i64,
    pub timeline_end_us: i64,
    pub mapping_json: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    schema_version: u32,
    method: Method,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Method {
    Linear,
}

pub(crate) struct ClockChunk<'a> {
    pub frames: i64,
    pub start_us: i64,
    pub end_us: i64,
    pub segments: &'a [ClockSegment],
}

#[derive(Clone, Debug)]
pub(crate) struct Span {
    pub first_frame: u64,
    pub frames: u64,
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ClockLayout {
    pub spans: Vec<Span>,
    pub raw_frames: u64,
    pub start_us: i64,
    pub end_us: i64,
}

pub(crate) fn frame_at(micros: i64, rate: u32) -> Result<u64, String> {
    if micros < 0 || !(8_000..=384_000).contains(&rate) {
        return Err("時刻補正の時間・形式が不正です".into());
    }
    let frame = (micros as u128 * u128::from(rate)).div_ceil(MICROS);
    if frame > MAX_SAFE_FRAMES {
        return Err("時刻補正のフレーム位置が範囲外です".into());
    }
    Ok(frame as u64)
}

impl ClockLayout {
    pub(crate) fn new(chunks: &[ClockChunk<'_>], rate: u32) -> Result<Self, String> {
        if chunks.is_empty() || chunks.len() > 1024 || !(8_000..=384_000).contains(&rate) {
            return Err("時刻補正の音声構成が不正です".into());
        }
        let mut spans = Vec::new();
        let mut raw_frames = 0_u64;
        let mut previous_end = None;
        let mut mapping_bytes = 0_usize;
        for chunk in chunks {
            if chunk.frames <= 0
                || chunk.start_us < 0
                || chunk.end_us <= chunk.start_us
                || previous_end.is_some_and(|end| end > chunk.start_us)
                || chunk.segments.len() > 1024
            {
                return Err("時刻補正の音声区間に欠損・重複または不正な範囲があります".into());
            }
            let mut from = 0;
            let mut time = chunk.start_us;
            if chunk.segments.is_empty() {
                if (i128::from(chunk.end_us - chunk.start_us)
                    - i128::from(chunk.frames) * 1_000_000 / i128::from(rate))
                .abs()
                    > 1
                {
                    return Err("時刻補正のない区間の時間とフレーム数が一致しません".into());
                }
                spans.push(Span {
                    first_frame: raw_frames,
                    frames: chunk.frames as u64,
                    start_us: chunk.start_us,
                    end_us: chunk.end_us,
                });
            } else {
                for segment in chunk.segments {
                    mapping_bytes = mapping_bytes
                        .checked_add(segment.mapping_json.len())
                        .filter(|count| *count <= MAX_CLOCK_MAPPING_BYTES)
                        .ok_or("時刻補正の形式情報の合計容量が上限を超えています")?;
                    if segment.mapping_json.len() > 4096 {
                        return Err("時刻補正の形式情報が容量上限を超えています".into());
                    }
                    let mapping: Mapping = serde_json::from_str(&segment.mapping_json)
                        .map_err(|_| "時刻補正の形式情報が不正です")?;
                    if mapping.schema_version != 1 || !matches!(mapping.method, Method::Linear) {
                        return Err("時刻補正の形式版に対応していません".into());
                    }
                    if segment.source_frame_start != from
                        || segment.source_frame_end <= from
                        || segment.source_frame_end > chunk.frames
                        || segment.timeline_start_us < time
                        || (from == 0 && segment.timeline_start_us != chunk.start_us)
                        || segment.timeline_end_us <= segment.timeline_start_us
                        || segment.timeline_end_us > chunk.end_us
                    {
                        return Err("時刻補正の対応に欠損・重複または範囲外の位置があります".into());
                    }
                    let frames = (segment.source_frame_end - from) as u64;
                    let duration = (segment.timeline_end_us - segment.timeline_start_us) as u128;
                    let input_step = u128::from(frames) * MICROS;
                    let output_step = duration * u128::from(rate);
                    // Explicit admission bound, never silently clamp a corrupt
                    // slope. This bounds each resampling halo/kernel allocation.
                    if input_step > output_step * 64 || output_step > input_step * 64 {
                        return Err("時刻補正の変換比が対応範囲（1/64〜64）外です".into());
                    }
                    spans.push(Span {
                        first_frame: raw_frames + from as u64,
                        frames,
                        start_us: segment.timeline_start_us,
                        end_us: segment.timeline_end_us,
                    });
                    from = segment.source_frame_end;
                    time = segment.timeline_end_us;
                }
                if from != chunk.frames || time != chunk.end_us {
                    return Err("時刻補正の対応が音声区間全体を覆っていません".into());
                }
            }
            if spans.len() > MAX_CLOCK_SEGMENTS {
                return Err("時刻補正の対応区間が多すぎます".into());
            }
            raw_frames = raw_frames
                .checked_add(chunk.frames as u64)
                .filter(|frames| u128::from(*frames) <= MAX_SAFE_FRAMES)
                .ok_or("時刻補正の入力フレーム数が範囲外です")?;
            previous_end = Some(chunk.end_us);
        }
        let layout = Self {
            spans,
            raw_frames,
            start_us: chunks[0].start_us,
            end_us: chunks.last().unwrap().end_us,
        };
        frame_at(layout.start_us, rate)?;
        frame_at(layout.end_us, rate)?;
        Ok(layout)
    }

    pub(crate) fn range(&self, start: i64, end: i64, rate: u32) -> Result<(u64, u64), String> {
        if start < self.start_us || end > self.end_us || end <= start {
            return Err("時刻補正の再生区間が保存範囲外です".into());
        }
        let mut position = start;
        for span in &self.spans {
            if span.end_us <= position {
                continue;
            }
            if span.start_us > position {
                return Err("時刻補正の再生区間に欠損があります".into());
            }
            position = end.min(span.end_us);
            if position == end {
                break;
            }
        }
        if position != end {
            return Err("時刻補正の再生区間に欠損があります".into());
        }
        let first = frame_at(start, rate)?;
        let last = frame_at(end, rate)?;
        if last <= first {
            return Err("時刻補正の再生フレームがありません".into());
        }
        Ok((first, last))
    }

    pub(crate) fn contiguous_frames(&self, index: usize) -> (u64, u64) {
        let mut first = index;
        let mut last = index;
        while first > 0 && self.spans[first - 1].end_us == self.spans[first].start_us {
            first -= 1;
        }
        while last + 1 < self.spans.len()
            && self.spans[last].end_us == self.spans[last + 1].start_us
        {
            last += 1;
        }
        (
            self.spans[first].first_frame,
            self.spans[last].first_frame + self.spans[last].frames,
        )
    }
}

impl Span {
    pub(crate) fn position(&self, output: u64, rate: u32) -> Result<(u128, u128), String> {
        let time = u128::from(output) * MICROS;
        let start = self.start_us as u128 * u128::from(rate);
        let denominator = (self.end_us - self.start_us) as u128 * u128::from(rate);
        let numerator = time
            .checked_sub(start)
            .and_then(|time| time.checked_mul(u128::from(self.frames)))
            .and_then(|position| {
                u128::from(self.first_frame)
                    .checked_mul(denominator)
                    .and_then(|first| first.checked_add(position))
            })
            .ok_or("時刻補正のフレーム計算が範囲外です")?;
        Ok((numerator, denominator))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn segment(first: i64, last: i64, start: i64, end: i64) -> ClockSegment {
        ClockSegment {
            source_frame_start: first,
            source_frame_end: last,
            timeline_start_us: start,
            timeline_end_us: end,
            mapping_json: r#"{"schema_version":1,"method":"linear"}"#.into(),
        }
    }

    fn layout(segments: &[ClockSegment]) -> Result<ClockLayout, String> {
        let layout = ClockLayout::new(
            &[ClockChunk {
                frames: 48000,
                start_us: 100_000,
                end_us: 1_200_000,
                segments,
            }],
            48000,
        )?;
        layout.range(100000, 1200000, 48000)?;
        Ok(layout)
    }

    #[test]
    fn piecewise_mapping_keeps_exact_rational_positions_and_global_slice_grid() {
        let layout = layout(&[
            segment(0, 24000, 100000, 600000),
            segment(24000, 48000, 600000, 1200000),
        ])
        .unwrap();
        assert_eq!(layout.raw_frames, 48000);
        assert_eq!(layout.range(100001, 1200000, 48000).unwrap(), (4801, 57600));
        let (n, d) = layout.spans[1].position(43200, 48000).unwrap();
        assert_eq!(n / d, 36000);
        let (n, d) = layout.spans[0].position(4801, 48000).unwrap();
        assert_eq!(n, d);
    }

    #[test]
    fn missing_unknown_version_and_algorithm_are_not_implicit_identity_maps() {
        for json in [
            "{}",
            r#"{"schema_version":2,"method":"linear"}"#,
            r#"{"schema_version":1,"method":"quadratic"}"#,
            r#"{"schema_version":1,"method":"linear","extra":true}"#,
        ] {
            let mut map = segment(0, 48000, 100000, 1200000);
            map.mapping_json = json.into();
            assert!(layout(&[map]).is_err(), "{json}");
        }
    }

    #[test]
    fn holes_overlaps_reverse_time_and_incomplete_coverage_are_rejected() {
        for maps in [
            vec![segment(1, 48000, 100000, 1200000)],
            vec![segment(0, 47999, 100000, 1200000)],
            vec![segment(0, 48001, 100000, 1200000)],
            vec![segment(0, 48000, 100001, 1200000)],
            vec![segment(0, 48000, 100000, 1199999)],
            vec![
                segment(0, 24000, 100000, 600000),
                segment(23999, 48000, 600000, 1200000),
            ],
            vec![
                segment(0, 24000, 100000, 600000),
                segment(24000, 48000, 600001, 1200000),
            ],
            vec![
                segment(0, 24000, 100000, 600000),
                segment(24000, 48000, 600000, 599999),
            ],
        ] {
            assert!(layout(&maps).is_err());
        }
    }

    #[test]
    fn requested_clock_ranges_must_be_contiguous_and_maps_cover_their_chunk() {
        let map = segment(0, 24000, 0, 550000);
        let chunks = [
            ClockChunk {
                frames: 24000,
                start_us: 0,
                end_us: 550000,
                segments: &[map],
            },
            ClockChunk {
                frames: 24000,
                start_us: 550000,
                end_us: 1050000,
                segments: &[],
            },
        ];
        let layout = ClockLayout::new(&chunks, 48000).unwrap();
        assert_eq!(layout.spans[1].first_frame, 24000);
        assert_eq!(layout.range(0, 1050000, 48000).unwrap(), (0, 50400));
        let missing = ClockLayout::new(
            &[
                ClockChunk {
                    frames: 24000,
                    start_us: 0,
                    end_us: 500000,
                    segments: &[],
                },
                ClockChunk {
                    frames: 24000,
                    start_us: 550001,
                    end_us: 1050001,
                    segments: &[],
                },
            ],
            48000,
        )
        .unwrap();
        assert!(missing.range(0, 1050001, 48000).is_err());
        assert!(missing.range(550001, 1050001, 48000).is_ok());
    }

    #[test]
    fn extreme_slopes_and_unsafe_frame_positions_fail_without_clamping() {
        let fast = [segment(0, 48000, 0, 1)];
        assert!(ClockLayout::new(
            &[ClockChunk {
                frames: 48000,
                start_us: 0,
                end_us: 1,
                segments: &fast
            }],
            48000
        )
        .is_err());
        assert!(frame_at(i64::MAX, 384000).is_err());
        assert!(frame_at(-1, 48000).is_err());
        assert!(frame_at(1, 0).is_err());
    }

    #[test]
    fn map_count_and_metadata_size_are_bounded() {
        let mut map = segment(0, 48000, 100000, 1200000);
        map.mapping_json = " ".repeat(4097);
        assert!(layout(&[map]).is_err());
        assert!(layout(&vec![segment(0, 48000, 100000, 1200000); 1025]).is_err());
    }

    #[test]
    fn whole_track_map_budget_is_separate_from_bounded_chunk_metadata() {
        let maps: Vec<Vec<ClockSegment>> = (0..65)
            .map(|chunk| {
                (0..1024)
                    .map(|frame| {
                        let start = (chunk * 1024 + frame) * 125;
                        segment(frame, frame + 1, start, start + 125)
                    })
                    .collect()
            })
            .collect();
        let chunks: Vec<_> = maps
            .iter()
            .enumerate()
            .map(|(index, segments)| ClockChunk {
                frames: 1024,
                start_us: index as i64 * 128000,
                end_us: (index as i64 + 1) * 128000,
                segments,
            })
            .collect();
        let valid = ClockLayout::new(&chunks[..64], 8000).unwrap();
        assert_eq!(valid.spans.len(), MAX_CLOCK_SEGMENTS);
        assert_eq!(valid.raw_frames, MAX_CLOCK_SEGMENTS as u64);
        assert!(ClockLayout::new(&chunks, 8000).is_err());
        assert!(ClockLayout::new(&chunks[..2], 8000)
            .unwrap()
            .range(0, 256000, 8000)
            .is_ok());
        let mut verbose = maps[..2].to_vec();
        let json = format!(
            "{}{{\"schema_version\":1,\"method\":\"linear\"}}",
            " ".repeat(3000)
        );
        for map in verbose.iter_mut().flatten() {
            map.mapping_json = json.clone();
        }
        let too_large: Vec<_> = verbose
            .iter()
            .enumerate()
            .map(|(index, segments)| ClockChunk {
                frames: 1024,
                start_us: index as i64 * 128000,
                end_us: (index as i64 + 1) * 128000,
                segments,
            })
            .collect();
        assert!(ClockLayout::new(&too_large, 8000)
            .err()
            .unwrap()
            .contains("合計容量"));
    }
}
