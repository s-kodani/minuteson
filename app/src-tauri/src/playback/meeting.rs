use super::PreparedAudio;
use crate::audio::PREVIEW_FRAMES;
use crate::resampling::Filter;

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
// passed the storage membership/hash/gap checks. Padding is only outside their
// known extents; holes within a source must never be represented as silence.
pub(crate) struct MeetingAudio {
    sources: Vec<Source>,
    pub(crate) end_us: i64,
    pub(crate) total_frames: u64,
    pub(crate) track_kind: String,
}

impl MeetingAudio {
    pub(crate) fn new(sources: Vec<PreparedAudio>, end_us: i64) -> Result<Self, String> {
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
        let mut mixed = vec![0_f64; frames as usize];
        // Constant headroom for both: no gain jump when one track starts/ends.
        let gain = 1.0 / self.sources.len() as f64;
        for source in &mut self.sources {
            let first = offset.max(source.first_output);
            let last = (offset + frames).min(source.last_output);
            if first >= last {
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
            let from = ((first_position / DENOMINATOR) as u64)
                .saturating_sub(half)
                .min(source.frames - 1);
            let to = ((last_position / DENOMINATOR) as u64 + half + 1).min(source.frames);
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
