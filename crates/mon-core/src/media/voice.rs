//! Bounded, packet-independent speech PCM conversion. Native callers own IO;
//! neither source files nor full decoded recordings are retained here.
use super::resampling::Filter;
use std::collections::VecDeque;

pub const RATE: u32 = 16_000;
pub const MAX_INPUT_FRAMES: usize = 96_000;
pub const MAX_OUTPUT_FRAMES: usize = 4096;
pub const POLICY: &str = "voice_pcm16_16000_mono_v1";

pub fn frames(input: u64, rate: u32) -> Option<u64> {
    input
        .checked_mul(u64::from(RATE))?
        .checked_add(u64::from(rate).checked_sub(1)?)?
        .checked_div(u64::from(rate))
}

struct Span {
    // Rational source position in units of 1 / RATE input frames. After a
    // partial drain this anchor is the immutable emitted output boundary.
    first: u128,
    last: u64,
    start: i64,
    end: i64,
}

/// State belongs to one serial writer. Output callbacks borrow PCM only for
/// their duration. A failed callback permanently poisons this converter.
pub struct VoiceConverter {
    rate: u32,
    channels: usize,
    filter: Filter,
    samples: Vec<f32>,
    first: u64,
    received: u64,
    emitted: u64,
    spans: VecDeque<Span>,
    last_end: Option<i64>,
    failed: bool,
    finished: bool,
}

impl VoiceConverter {
    pub fn new(rate: u32, channels: u32) -> Result<Self, &'static str> {
        if !(8000..=384000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err("unsupported voice PCM geometry");
        }
        Ok(Self {
            rate,
            channels: channels as usize,
            filter: Filter::new(rate, RATE),
            samples: Vec::new(),
            first: 0,
            received: 0,
            emitted: 0,
            spans: VecDeque::new(),
            last_end: None,
            failed: false,
            finished: false,
        })
    }

    pub fn push(
        &mut self,
        input: &[f32],
        start: i64,
        end: i64,
        write: &mut impl FnMut(&[i16], i64, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if self.failed
            || self.finished
            || input.is_empty()
            || input.len() % self.channels != 0
            || input.len() / self.channels > MAX_INPUT_FRAMES
            || start < 0
            || end <= start
            || input.iter().any(|v| !v.is_finite())
        {
            self.failed = true;
            return Err("invalid voice PCM input or converter state");
        }
        // Only the un-emitted halo can be adjusted to the next measured
        // boundary. Already written PCM and timestamps are immutable.
        if let Some(previous) = self.spans.back_mut() {
            if start <= previous.start || self.last_end.is_some_and(|n| start < n) {
                self.failed = true;
                return Err("voice PCM capture clock reversed");
            }
            previous.end = start;
        }
        let first = self.received;
        self.received = self
            .received
            .checked_add((input.len() / self.channels) as u64)
            .ok_or("voice PCM frame overflow")?;
        self.spans.push_back(Span {
            first: u128::from(first) * u128::from(RATE),
            last: self.received,
            start,
            end,
        });
        for frame in input.chunks_exact(self.channels) {
            self.samples.push(
                (frame.iter().map(|v| f64::from(*v)).sum::<f64>() / self.channels as f64) as f32,
            );
        }
        self.drain(false, write)
    }

    pub fn finish(
        &mut self,
        write: &mut impl FnMut(&[i16], i64, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if self.failed {
            return Err("voice PCM converter failed");
        }
        if self.finished {
            return Ok(());
        }
        self.drain(true, write)?;
        self.finished = true;
        Ok(())
    }

    fn drain(
        &mut self,
        finalizing: bool,
        write: &mut impl FnMut(&[i16], i64, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let target = frames(self.received, self.rate).ok_or("voice PCM frame overflow")?;
        let mut pcm = Vec::with_capacity(MAX_OUTPUT_FRAMES);
        let mut packet_start = 0;
        let mut packet_end = 0;
        while self.emitted < target {
            let position = u128::from(self.emitted) * u128::from(self.rate);
            let base = (position / u128::from(RATE)) as u64;
            let half = self.filter.half as u64;
            if !finalizing && base + half >= self.received {
                break;
            }
            if self
                .spans
                .front()
                .is_some_and(|s| position >= u128::from(s.last) * u128::from(RATE))
            {
                if !pcm.is_empty() {
                    self.emit(&pcm, packet_start, packet_end, write)?;
                    pcm.clear();
                }
                self.spans.pop_front();
                continue;
            }
            let span = self.spans.front().ok_or("voice PCM timestamp missing")?;
            let timestamp = |pos: u128| {
                let relative = pos.saturating_sub(span.first);
                let width = u128::from(span.last) * u128::from(RATE) - span.first;
                span.start + (relative.min(width) * (span.end - span.start) as u128 / width) as i64
            };
            if pcm.is_empty() {
                packet_start = timestamp(position);
            }
            packet_end = timestamp(position + u128::from(self.rate));
            let value = if self.rate == RATE {
                self.samples[(self.emitted - self.first) as usize]
            } else {
                self.filter
                    .sample_from(&self.samples, self.emitted, self.rate, self.first)
            };
            pcm.push((value.clamp(-1.0, 1.0) * 32767.0).round() as i16);
            self.emitted += 1;
            if pcm.len() == MAX_OUTPUT_FRAMES {
                self.emit(&pcm, packet_start, packet_end, write)?;
                pcm.clear();
            }
        }
        if !pcm.is_empty() {
            self.emit(&pcm, packet_start, packet_end, write)?;
        }
        // A later hardware timestamp may refine only the un-emitted halo.
        // Re-fitting the entire previous callback would move an already
        // published boundary backwards even for sub-frame clock jitter.
        let position = u128::from(self.emitted) * u128::from(self.rate);
        while self
            .spans
            .front()
            .is_some_and(|s| position >= u128::from(s.last) * u128::from(RATE))
        {
            self.spans.pop_front();
        }
        if let Some(span) = self.spans.front_mut() {
            if position > span.first {
                span.first = position;
                span.start = self.last_end.ok_or("voice PCM output anchor missing")?;
            }
        }
        let next = (u128::from(self.emitted) * u128::from(self.rate) / u128::from(RATE)) as u64;
        let keep = next.saturating_sub(self.filter.half as u64);
        let discard = keep
            .saturating_sub(self.first)
            .min(self.samples.len() as u64) as usize;
        self.samples.drain(..discard);
        self.first += discard as u64;
        Ok(())
    }

    fn emit(
        &mut self,
        pcm: &[i16],
        start: i64,
        end: i64,
        write: &mut impl FnMut(&[i16], i64, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if end <= start || self.last_end.is_some_and(|n| start < n) {
            self.failed = true;
            return Err("voice PCM output clock reversed");
        }
        if let Err(error) = write(pcm, start, end) {
            self.failed = true;
            return Err(error);
        }
        self.last_end = Some(end);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn convert(rate: u32, channels: u32, input: &[f32], block: usize) -> Vec<i16> {
        let mut converter = VoiceConverter::new(rate, channels).unwrap();
        let mut result = Vec::new();
        let mut count = 0u64;
        let mut write = |pcm: &[i16], _, _| {
            result.extend_from_slice(pcm);
            Ok(())
        };
        for chunk in input.chunks(block * channels as usize) {
            let first = count;
            count += (chunk.len() / channels as usize) as u64;
            converter
                .push(
                    chunk,
                    (first * 1_000_000_000 / u64::from(rate)) as i64,
                    (count * 1_000_000_000 / u64::from(rate)) as i64,
                    &mut write,
                )
                .unwrap();
        }
        converter.finish(&mut write).unwrap();
        result
    }
    #[test]
    fn packet_boundaries_do_not_change_pcm_or_fractional_tail() {
        for rate in [8000, 16000, 44100, 48000, 96000, 384000] {
            let input: Vec<f32> = (0..10007)
                .flat_map(|n| [((n as f32) * 0.1).sin() * 0.4, 0.2])
                .collect();
            let expected = convert(rate, 2, &input, 10007);
            assert_eq!(expected.len() as u64, frames(10007, rate).unwrap());
            for block in [1, 7, 127, 4096] {
                assert_eq!(
                    convert(rate, 2, &input, block),
                    expected,
                    "rate={rate} block={block}"
                );
            }
        }
    }
    #[test]
    fn opposite_channels_cancel_and_failed_writes_are_terminal() {
        assert!(convert(48000, 2, &[0.5, -0.5].repeat(5000), 257)
            .iter()
            .all(|v| *v == 0));
        let mut converter = VoiceConverter::new(48000, 1).unwrap();
        assert!(converter
            .push(&[0.1; 4096], 0, 85333333, &mut |_, _, _| Err("disk full"))
            .is_err());
        assert!(converter.finish(&mut |_, _, _| Ok(())).is_err());
    }
    #[test]
    fn four_hour_frame_arithmetic_does_not_round_per_packet() {
        for rate in [44100, 48000, 96000, 384000] {
            assert_eq!(frames(u64::from(rate) * 14400, rate), Some(16000 * 14400));
            assert!(frames(u64::from(rate) * 14400 + 1, rate).unwrap() <= 16000 * 14400 + 2);
        }
    }

    #[test]
    fn measured_jitter_refines_only_unwritten_filter_halo() {
        for rate in [16000u32, 44100, 48000] {
            let mut converter = VoiceConverter::new(rate, 2).unwrap();
            let mut saved = 0;
            let mut last = 0;
            let mut write = |pcm: &[i16], start, end| {
                assert!(start >= last && end > start);
                saved += pcm.len();
                last = end;
                Ok(())
            };
            for index in 0..200i64 {
                let start =
                    1_000_000_000 + index * 20_000_000 + if index % 2 == 0 { 0 } else { -3000 };
                converter
                    .push(
                        &vec![0.125; rate as usize / 50 * 2],
                        start,
                        start + 20_000_000,
                        &mut write,
                    )
                    .unwrap();
            }
            converter.finish(&mut write).unwrap();
            assert_eq!(saved, 64000);
            assert_eq!(last, 5_000_000_000 - 3000);
        }
    }

    #[test]
    #[ignore = "four hours of actual 48kHz streaming DSP; run in release"]
    fn four_hours_keep_exact_frame_count_and_bounded_memory() {
        let rate = 48000u32;
        let mut converter = VoiceConverter::new(rate, 2).unwrap();
        let packet = vec![0.125f32; 4096 * 2];
        let total = u64::from(rate) * 14400;
        let mut received = 0;
        let mut saved = 0;
        let mut peak = 0;
        let mut last_end = 0;
        while received < total {
            let count = (total - received).min(4096);
            let start = received * 1_000_000_000 / u64::from(rate);
            received += count;
            let end = received * 1_000_000_000 / u64::from(rate);
            converter
                .push(
                    &packet[..count as usize * 2],
                    start as i64,
                    end as i64,
                    &mut |pcm, _, end| {
                        assert!(pcm.iter().all(|n| *n == 4096));
                        saved += pcm.len() as u64;
                        last_end = end;
                        Ok(())
                    },
                )
                .unwrap();
            peak = peak.max(converter.samples.capacity());
            assert!(converter.spans.len() <= 2);
        }
        converter
            .finish(&mut |pcm, _, end| {
                saved += pcm.len() as u64;
                last_end = end;
                Ok(())
            })
            .unwrap();
        assert_eq!(saved, 16000 * 14400);
        assert_eq!(last_end, 14400 * 1_000_000_000);
        assert!(peak <= 16384, "retained sample capacity {peak}");
    }
}
