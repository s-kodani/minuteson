use super::{PlaybackMissingRange, PreparedAudio};
use crate::audio::PREVIEW_FRAMES;
use crate::resampling::Filter;
use std::sync::Arc;

pub(crate) const MEETING_RATE: u32 = 48_000;
const MICROS: u128 = 1_000_000;
const DENOMINATOR: u128 = MICROS * MEETING_RATE as u128;
const MAX_SAFE_FRAMES: u128 = (1_u128 << 53) - 1;

struct Source {
    audio: PreparedAudio,
    filter: Option<Filter>,
    frames: u64,
    first_output: u64,
    last_output: u64,
}

// A bounded PCM reader, not a decoded recording in memory. Sources have already
// passed storage membership/format/gap checks. Padding is only outside their
// known extents; holes within a source must never be represented as silence.
pub(crate) struct MeetingAudio {
    sources: Vec<Source>,
    pub(crate) end_us: i64,
    pub(crate) total_frames: u64,
    pub(crate) track_kind: String,
    pub(crate) missing_ranges: Vec<PlaybackMissingRange>,
    pub(crate) cache_key: String,
    cached: Option<Arc<super::mix_cache::CachedMix>>,
}

// A view over validated whole-meeting sources. Keep absolute resampling phase
// and filter halos, but never return PCM outside the requested time interval.
pub(crate) struct MeetingIntervalAudio {
    audio: MeetingAudio,
    first_frame: u64,
    pub(crate) start_us: i64,
    pub(crate) end_us: i64,
    pub(crate) total_frames: u64,
    pub(crate) track_kind: String,
    pub(crate) missing_ranges: Vec<PlaybackMissingRange>,
}
impl MeetingIntervalAudio {
    pub(crate) fn use_cache(&mut self, cache: &super::mix_cache::MixCache) {
        self.audio.use_cache(cache);
    }
    pub(crate) fn new(audio: MeetingAudio, start_us: i64, end_us: i64) -> Result<Self, String> {
        if start_us < 0
            || end_us <= start_us
            || end_us > audio.end_us
            || end_us > 9_007_199_254_740_991
        {
            return Err("Markdownの再生区間が会議音声の範囲外です".into());
        }
        let first_frame = output_frame(start_us)?;
        let last_frame = output_frame(end_us)?;
        let total_frames = last_frame
            .checked_sub(first_frame)
            .filter(|v| *v > 0)
            .ok_or("再生区間に音声フレームがありません")?;
        let mut missing_ranges: Vec<PlaybackMissingRange> = audio
            .missing_ranges
            .iter()
            .filter_map(|gap| {
                let start_frame = gap.start_frame.max(first_frame);
                let end_frame = gap.end_frame.min(last_frame);
                (start_frame < end_frame).then(|| PlaybackMissingRange {
                    track_kind: gap.track_kind.clone(),
                    start_us: gap.start_us.max(start_us),
                    end_us: gap.end_us.min(end_us),
                    start_frame: start_frame - first_frame,
                    end_frame: end_frame - first_frame,
                    reason_code: gap.reason_code.clone(),
                })
            })
            .collect();
        // Clipping can give gaps with different original starts the same
        // relative start. Restore ordering after clipping (also across tracks).
        missing_ranges.sort_by(|a, b| {
            (a.start_frame, a.end_frame, &a.track_kind).cmp(&(
                b.start_frame,
                b.end_frame,
                &b.track_kind,
            ))
        });
        Ok(Self {
            track_kind: audio.track_kind.clone(),
            audio,
            first_frame,
            start_us,
            end_us,
            total_frames,
            missing_ranges,
        })
    }
    pub(crate) fn readable_frames(&self, offset: u64, frames: u64) -> Result<u64, String> {
        if frames == 0
            || offset
                .checked_add(frames)
                .is_none_or(|end| end > self.total_frames)
        {
            return Err("Markdown区間の読出し範囲が不正です".into());
        }
        self.audio
            .readable_frames(self.first_frame + offset, frames)
    }
    pub(crate) fn read_pcm(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        if self.readable_frames(offset, frames)? != frames {
            return Err("Markdown区間の読出しが欠損区間を横断しています".into());
        }
        self.audio.read_pcm(self.first_frame + offset, frames)
    }
}

impl MeetingAudio {
    pub(crate) fn set_cache_key(&mut self, census: &str) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"MON-mix-f32-48000-v1");
        hash.update(census.as_bytes());
        for source in &self.sources {
            for span in &source.audio.spans {
                hash.update(span.audio.cache_stamp()?.as_bytes());
            }
        }
        self.cache_key = format!("{:x}", hash.finalize());
        Ok(())
    }

    pub(crate) fn use_cache(&mut self, cache: &super::mix_cache::MixCache) {
        self.cached = cache.get(&self.cache_key);
    }

    #[cfg(test)]
    pub(crate) fn new(sources: Vec<PreparedAudio>, end_us: i64) -> Result<Self, String> {
        Self::with_missing_ranges(sources, end_us, vec![])
    }

    pub(crate) fn with_missing_ranges(
        sources: Vec<PreparedAudio>,
        end_us: i64,
        mut missing_ranges: Vec<PlaybackMissingRange>,
    ) -> Result<Self, String> {
        if sources.is_empty() || sources.len() > 2 || end_us <= 0 {
            return Err("会議音声の構成が不正です".into());
        }
        if sources.len() == 2
            && !(sources.iter().any(|audio| audio.track_kind == "mic")
                && sources.iter().any(|audio| audio.track_kind == "system"))
        {
            return Err("両方の再生にはマイクとPC音声が一つずつ必要です".into());
        }
        let total_frames = output_frame(end_us)?;
        if missing_ranges.len() > 2 * crate::audio_clock::MAX_CLOCK_GAPS
            || sources.iter().any(|source| {
                missing_ranges
                    .iter()
                    .filter(|gap| gap.track_kind == source.track_kind)
                    .count()
                    > crate::audio_clock::MAX_CLOCK_GAPS
            })
            || missing_ranges.iter().any(|gap| {
                gap.end_frame > total_frames
                    || gap.end_frame <= gap.start_frame
                    || PlaybackMissingRange::new(
                        &gap.track_kind,
                        gap.start_us,
                        gap.end_us,
                        &gap.reason_code,
                        end_us,
                    )
                    .map_or(true, |verified| {
                        verified.start_frame != gap.start_frame
                            || verified.end_frame != gap.end_frame
                    })
                    || !sources
                        .iter()
                        .any(|source| source.track_kind == gap.track_kind)
            })
        {
            return Err("会議音声の欠損構成が不正です".into());
        }
        missing_ranges.sort_by(|left, right| {
            (left.start_frame, left.end_frame, &left.track_kind).cmp(&(
                right.start_frame,
                right.end_frame,
                &right.track_kind,
            ))
        });
        let track_kind = if sources.len() == 2 {
            "both".into()
        } else {
            sources[0].track_kind.clone()
        };
        let mut prepared = Vec::with_capacity(sources.len());
        for audio in sources {
            if !(8_000..=384_000).contains(&audio.sample_rate)
                || !(1..=8).contains(&audio.channels)
                || audio.start_us < 0
                || audio.end_us <= audio.start_us
                || audio.end_us > end_us
                || !matches!(audio.track_kind.as_str(), "mic" | "system" | "imported")
                || audio.spans.is_empty()
                || audio.spans.len() > 1024
            {
                return Err("会議音声の形式・時間が不正です".into());
            }
            let mut frames = 0_u64;
            for span in &audio.spans {
                if span.frames == 0
                    || span.audio.format.sample_rate != audio.sample_rate
                    || span.audio.format.channels != audio.channels
                    || span
                        .first_frame
                        .checked_add(span.frames)
                        .is_none_or(|end| end > span.audio.format.frames)
                {
                    return Err("会議音声のセグメントが不正です".into());
                }
                frames = frames
                    .checked_add(span.frames)
                    .ok_or("会議音声の長さが範囲外です")?;
            }
            // Mapped sources retain full physical spans for their filter halo;
            // only the logical meeting-time output count determines duration.
            if audio.clock.is_some() {
                frames = audio.total_frames();
            }
            // Prepared ranges may round the first/last source frame outward.
            let duration = i128::from(audio.end_us - audio.start_us);
            if (duration * i128::from(audio.sample_rate) - i128::from(frames) * 1_000_000).abs()
                > 2_000_000
            {
                return Err("会議音声の時間とフレーム数が一致しません".into());
            }
            let first_output = output_frame(audio.start_us)?;
            let last_output = output_frame(audio.end_us)?;
            let aligned = audio.sample_rate == MEETING_RATE
                && (audio.start_us as u128 * u128::from(MEETING_RATE)).is_multiple_of(MICROS);
            let filter = (!aligned).then(|| Filter::new(audio.sample_rate, MEETING_RATE));
            prepared.push(Source {
                audio,
                filter,
                frames,
                first_output,
                last_output,
            });
        }
        Ok(Self {
            sources: prepared,
            end_us,
            total_frames,
            track_kind,
            missing_ranges,
            cache_key: String::new(),
            cached: None,
        })
    }

    pub(crate) fn read_pcm(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        if frames == 0
            || frames > PREVIEW_FRAMES
            || offset
                .checked_add(frames)
                .is_none_or(|end| end > self.total_frames)
        {
            return Err("会議音声の読出し範囲が不正です".into());
        }
        if self.readable_frames(offset, frames)? != frames {
            return Err("会議音声の読出しが欠損区間を横断しています".into());
        }
        if let Some(cache) = &self.cached {
            for source in &self.sources {
                for span in &source.audio.spans {
                    span.audio.check_unchanged()?;
                }
            }
            return cache.read(offset, frames);
        }
        let mut mixed = vec![0_f64; frames as usize];
        // Constant headroom for both: no gain jump when one track starts/ends.
        let gain = 1.0 / self.sources.len() as f64;
        for source in &mut self.sources {
            let mut known_start = 0_u64;
            let mut known_end = self.total_frames;
            for gap in self
                .missing_ranges
                .iter()
                .filter(|gap| gap.track_kind == source.audio.track_kind)
            {
                if gap.end_frame <= offset {
                    known_start = known_start.max(gap.end_frame);
                }
                if gap.start_frame >= offset + frames {
                    known_end = known_end.min(gap.start_frame);
                }
            }
            let has_gaps = self
                .missing_ranges
                .iter()
                .any(|gap| gap.track_kind == source.audio.track_kind);
            let first = offset.max(source.first_output);
            let last = (offset + frames).min(source.last_output);
            if first >= last {
                continue;
            }
            if let Some(clock) = &mut source.audio.clock {
                let pcm = clock.read_known(
                    &mut source.audio.spans,
                    source.audio.channels,
                    MEETING_RATE,
                    first,
                    last - first,
                    has_gaps.then_some((known_start, known_end)),
                )?;
                for (index, frame) in pcm
                    .chunks_exact(source.audio.channels as usize * 4)
                    .enumerate()
                {
                    let value: f64 = frame
                        .chunks_exact(4)
                        .map(|sample| f64::from(f32::from_le_bytes(sample.try_into().unwrap())))
                        .sum::<f64>()
                        / f64::from(source.audio.channels);
                    mixed[(first - offset) as usize + index] += value * gain;
                }
                continue;
            }
            let start_us = source.audio.start_us as u128;
            let sample_rate = u128::from(source.audio.sample_rate);
            let position = |output: u64| {
                (u128::from(output) * MICROS - start_us * u128::from(MEETING_RATE)) * sample_rate
            };
            let first_position = position(first);
            let last_position = position(last - 1);
            let half = source
                .filter
                .as_ref()
                .map_or(0, |filter| filter.half as u64);
            let known_first = if has_gaps {
                position(known_start.max(source.first_output)).div_ceil(DENOMINATOR) as u64
            } else {
                0
            };
            let known_last = if has_gaps {
                (position(known_end.min(source.last_output)) / DENOMINATOR) as u64
            } else {
                source.frames
            };
            let known_last = known_last.min(source.frames);
            if known_first >= known_last {
                return Err("欠損の外側に補間可能な保存音声がありません".into());
            }
            let from = ((first_position / DENOMINATOR) as u64)
                .saturating_sub(half)
                .max(known_first)
                .min(source.frames - 1);
            let to = ((last_position / DENOMINATOR) as u64 + half + 1).min(known_last);
            if from >= to {
                return Err("欠損の外側に補間可能な保存音声がありません".into());
            }
            let mut mono = Vec::with_capacity((to - from) as usize);
            let mut cursor = from;
            while cursor < to {
                let count = PREVIEW_FRAMES.min(to - cursor);
                let pcm = source.audio.read_pcm(cursor, count)?;
                for frame in pcm.chunks_exact(source.audio.channels as usize * 4) {
                    let sum: f64 = frame
                        .chunks_exact(4)
                        .map(|sample| f64::from(f32::from_le_bytes(sample.try_into().unwrap())))
                        .sum();
                    mono.push((sum / f64::from(source.audio.channels)) as f32);
                }
                cursor += count;
            }
            for output in first..last {
                let numerator = position(output);
                let value = match &source.filter {
                    Some(filter) => filter.sample_at(&mono, numerator, DENOMINATOR, from),
                    None => {
                        mono[((numerator / DENOMINATOR) as u64)
                            .min(source.frames - 1)
                            .saturating_sub(from) as usize]
                    }
                };
                mixed[(output - offset) as usize] += f64::from(value) * gain;
            }
        }
        Ok(mixed
            .into_iter()
            .flat_map(|sample| (sample.clamp(-1.0, 1.0) as f32).to_le_bytes())
            .collect())
    }

    pub(crate) fn readable_frames(&self, offset: u64, maximum: u64) -> Result<u64, String> {
        let mut frames = maximum;
        for gap in &self.missing_ranges {
            if gap.start_frame <= offset && offset < gap.end_frame {
                return Err(
                    "現在位置の音声は欠損しています。保存済み区間へ移動してください".into(),
                );
            }
            if gap.start_frame > offset {
                frames = frames.min(gap.start_frame - offset);
            }
        }
        Ok(frames)
    }
}

fn output_frame(micros: i64) -> Result<u64, String> {
    if micros < 0 {
        return Err("会議音声の時刻が不正です".into());
    }
    let frame = (micros as u128 * u128::from(MEETING_RATE)).div_ceil(MICROS);
    if frame > MAX_SAFE_FRAMES {
        return Err("会議音声の時間が再生可能範囲外です".into());
    }
    Ok(frame as u64)
}

#[cfg(test)]
mod tests;
