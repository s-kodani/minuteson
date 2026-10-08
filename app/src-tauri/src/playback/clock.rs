use super::AudioSpan;
use crate::audio::PREVIEW_FRAMES;
use crate::audio_clock::{frame_at, ClockLayout};
use crate::resampling::Filter;

pub(crate) struct ClockAudio {
    layout: ClockLayout,
    pub(crate) first_output: u64,
    pub(crate) frames: u64,
    cached_filter: Option<((usize, u32), Filter)>,
    known_bounds: Option<(u64, u64)>,
}

impl ClockAudio {
    // Only whole-meeting admission may retain holes. Each actual read still
    // rejects holes and clamps its resampling halo to the containing known run.
    pub(crate) fn for_meeting(layout: ClockLayout, rate: u32) -> Result<Self, String> {
        let first_output = frame_at(layout.start_us, rate)?;
        let last = frame_at(layout.end_us, rate)?;
        if last <= first_output {
            return Err("保存音声の再生フレームがありません".into());
        }
        Ok(Self {
            layout,
            first_output,
            frames: last - first_output,
            cached_filter: None,
            known_bounds: None,
        })
    }

    pub(crate) fn missing_intervals(&self) -> Vec<(i64, i64)> {
        self.layout
            .spans
            .windows(2)
            .filter_map(|pair| {
                (pair[0].end_us < pair[1].start_us).then_some((pair[0].end_us, pair[1].start_us))
            })
            .collect()
    }

    pub(crate) fn new(
        layout: ClockLayout,
        start: i64,
        end: i64,
        rate: u32,
    ) -> Result<Self, String> {
        let (first_output, last) = layout.range(start, end, rate)?;
        Ok(Self {
            layout,
            first_output,
            frames: last - first_output,
            cached_filter: None,
            known_bounds: None,
        })
    }

    // The output lattice is anchored to meeting time, not the requested slice,
    // packet or seek. Whole-meeting playback reads directly at its own rate,
    // avoiding a second conversion through the native-rate preview lattice.
    pub(crate) fn read(
        &mut self,
        spans: &mut [AudioSpan],
        channels: u32,
        rate: u32,
        first: u64,
        frames: u64,
    ) -> Result<Vec<u8>, String> {
        self.read_known(spans, channels, rate, first, frames, self.known_bounds)
    }

    pub(crate) fn restrict_to_known(&mut self, first: u64, end: u64) -> Result<(), String> {
        if end <= first || first < self.first_output || end > self.first_output + self.frames {
            return Err("固定した保存区間のフレームが範囲外です".into());
        }
        self.first_output = first;
        self.frames = end - first;
        self.known_bounds = Some((first, end));
        Ok(())
    }

    // Whole-meeting gap labels can also exclude PCM within a physically
    // continuous map. They constrain filter input, not just emitted output.
    pub(crate) fn read_known(
        &mut self,
        spans: &mut [AudioSpan],
        channels: u32,
        rate: u32,
        first: u64,
        frames: u64,
        known: Option<(u64, u64)>,
    ) -> Result<Vec<u8>, String> {
        let declared_bounds = known
            .map(|(start, end)| {
                Ok::<_, String>((
                    self.boundary_frame(start, rate, true)?,
                    self.boundary_frame(end, rate, false)?,
                ))
            })
            .transpose()?;
        let end = first
            .checked_add(frames)
            .ok_or("時刻補正の読出し範囲が不正です")?;
        if frames == 0
            || frames > PREVIEW_FRAMES
            || !(1..=8).contains(&channels)
            || first < frame_at(self.layout.start_us, rate)?
            || end > frame_at(self.layout.end_us, rate)?
            || known.is_some_and(|(start, last)| start >= last || first < start || end > last)
        {
            return Err("時刻補正の読出し範囲が不正です".into());
        }
        let mut result = Vec::with_capacity(frames as usize * channels as usize * 4);
        let first_index = self
            .layout
            .spans
            .partition_point(|span| first >= frame_at(span.end_us, rate).unwrap_or(u64::MAX));
        let last_index = self
            .layout
            .spans
            .partition_point(|span| end - 1 >= frame_at(span.end_us, rate).unwrap_or(u64::MAX));
        if self
            .layout
            .spans
            .get(first_index..=last_index)
            .is_none_or(|spans| {
                spans
                    .windows(2)
                    .any(|pair| pair[0].end_us != pair[1].start_us)
            })
        {
            return Err("時刻補正の読出し区間に欠損があります".into());
        }
        let mut output = first;
        while output < end {
            let index = self
                .layout
                .spans
                .partition_point(|span| output >= frame_at(span.end_us, rate).unwrap_or(u64::MAX));
            let span = self
                .layout
                .spans
                .get(index)
                .ok_or("時刻補正の対応区間がありません")?;
            let mut last = end.min(frame_at(span.end_us, rate)?).min(output + 1024);
            let input_step = u128::from(span.frames) * 1_000_000;
            let output_step = (span.end_us - span.start_us) as u128 * u128::from(rate);
            let (first_position, denominator) = span.position(output, rate)?;
            let exact = input_step == output_step && first_position.is_multiple_of(denominator);
            if exact {
                result.extend_from_slice(&read_raw(
                    spans,
                    channels,
                    (first_position / denominator) as u64,
                    last - output,
                )?);
            } else {
                if self
                    .cached_filter
                    .as_ref()
                    .is_none_or(|(key, _)| *key != (index, rate))
                {
                    self.cached_filter =
                        Some(((index, rate), Filter::for_step(input_step, output_step)?));
                }
                let filter = &self.cached_filter.as_ref().unwrap().1;
                let (mut run_first, mut run_last) = self.layout.contiguous_frames(index);
                if let Some((first, last)) = declared_bounds {
                    run_first = run_first.max(first);
                    run_last = run_last.min(last);
                }
                if run_first >= run_last {
                    return Err("欠損の外側に補間可能な保存音声がありません".into());
                }
                let step = input_step.div_ceil(output_step).max(1) as u64;
                let bounded = (PREVIEW_FRAMES - filter.half as u64 * 2 - 2) / step;
                last = last.min(output + bounded.max(1));
                let (last_position, _) = span.position(last - 1, rate)?;
                let from = ((first_position / denominator) as u64)
                    .saturating_sub(filter.half as u64)
                    .max(run_first)
                    .min(self.layout.raw_frames - 1);
                let to =
                    ((last_position / denominator) as u64 + filter.half as u64 + 1).min(run_last);
                if from >= to {
                    return Err("欠損の外側に補間可能な保存音声がありません".into());
                }
                let raw = read_raw(spans, channels, from, to - from)?;
                let mut planar = vec![Vec::with_capacity((to - from) as usize); channels as usize];
                for frame in raw.chunks_exact(channels as usize * 4) {
                    for (channel, sample) in frame.chunks_exact(4).enumerate() {
                        planar[channel].push(f32::from_le_bytes(sample.try_into().unwrap()));
                    }
                }
                for frame in output..last {
                    let (position, denominator) = span.position(frame, rate)?;
                    for input in &planar {
                        result.extend_from_slice(
                            &filter
                                .sample_at(input, position, denominator, from)
                                .clamp(-1.0, 1.0)
                                .to_le_bytes(),
                        );
                    }
                }
            }
            output = last;
        }
        Ok(result)
    }

    fn boundary_frame(&self, output: u64, rate: u32, lower: bool) -> Result<u64, String> {
        if output <= frame_at(self.layout.start_us, rate)? {
            return Ok(0);
        }
        if output >= frame_at(self.layout.end_us, rate)? {
            return Ok(self.layout.raw_frames);
        }
        let index = self.layout.spans.partition_point(|span| {
            let end = frame_at(span.end_us, rate).unwrap_or(u64::MAX);
            if lower {
                output >= end
            } else {
                output > end
            }
        });
        let span = self
            .layout
            .spans
            .get(index)
            .ok_or("欠損境界の保存音声がありません")?;
        let (position, denominator) = span.position(output, rate)?;
        let frame = if lower {
            position.div_ceil(denominator)
        } else {
            position / denominator
        };
        Ok((frame as u64).min(span.first_frame + span.frames))
    }
}

#[cfg(test)]
mod tests;

// Bounded physical reads, including the filter halo across file boundaries.
// A missing physical frame is an error, never synthesized silence.
pub(super) fn read_raw(
    spans: &mut [AudioSpan],
    channels: u32,
    offset: u64,
    frames: u64,
) -> Result<Vec<u8>, String> {
    let end = offset
        .checked_add(frames)
        .ok_or("音声の読出し範囲が不正です")?;
    let mut result = Vec::with_capacity(frames as usize * channels as usize * 4);
    let mut span_start = 0_u64;
    for span in spans {
        let span_end = span_start
            .checked_add(span.frames)
            .ok_or("音声フレーム数が範囲外です")?;
        let mut from = offset.max(span_start);
        let to = end.min(span_end);
        while from < to {
            let count = PREVIEW_FRAMES.min(to - from);
            result.extend_from_slice(
                &span
                    .audio
                    .read_frames(span.first_frame + from - span_start, count)?,
            );
            from += count;
        }
        span_start = span_end;
        if span_start >= end {
            break;
        }
    }
    if result.len() != frames as usize * channels as usize * 4 {
        return Err("保存音声の読出しフレームが不足しています".into());
    }
    Ok(result)
}
