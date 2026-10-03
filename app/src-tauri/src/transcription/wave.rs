use crate::audio::PREVIEW_FRAMES;
use crate::playback::PreparedAudio;
use std::f64::consts::PI;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const WHISPER_RATE: u32 = 16_000;
const MAX_WINDOW_SECONDS: u64 = 124;
const PHASES: usize = 256;

// Bounded windows, not an entire meeting. Context is converted with the window
// so the low-pass filter sees samples on both sides of ASR unit boundaries.
pub fn write_window(
    audio: &mut PreparedAudio,
    first_frame: u64,
    frames: u64,
    path: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<u64, String> {
    if frames == 0
        || frames > u64::from(audio.sample_rate) * MAX_WINDOW_SECONDS
        || first_frame
            .checked_add(frames)
            .is_none_or(|end| end > audio.total_frames())
    {
        return Err("文字起こし用の音声区間が不正です".into());
    }
    let mut mono = Vec::with_capacity(frames as usize);
    let mut offset = 0;
    while offset < frames {
        check_cancelled(cancelled)?;
        let count = PREVIEW_FRAMES.min(frames - offset);
        let pcm = audio.read_pcm(first_frame + offset, count)?;
        for frame in pcm.chunks_exact(audio.channels as usize * 4) {
            let sum: f64 = frame
                .chunks_exact(4)
                .map(|sample| f64::from(f32::from_le_bytes(sample.try_into().unwrap())))
                .sum();
            mono.push((sum / f64::from(audio.channels)) as f32);
        }
        offset += count;
    }
    let output_frames = (frames * u64::from(WHISPER_RATE) + u64::from(audio.sample_rate) / 2)
        / u64::from(audio.sample_rate);
    if output_frames == 0 {
        return Err("変換後の音声が空です".into());
    }
    check_cancelled(cancelled)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(&wav_header(output_frames as u32))
        .map_err(|e| e.to_string())?;
    let filter = Filter::new(audio.sample_rate);
    for index in 0..output_frames {
        if index % 4096 == 0 {
            check_cancelled(cancelled)?;
        }
        let value = if audio.sample_rate == WHISPER_RATE {
            mono[index as usize]
        } else {
            filter.sample(&mono, index, audio.sample_rate)
        };
        let sample = (value.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        writer
            .write_all(&sample.to_le_bytes())
            .map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    writer.get_ref().sync_all().map_err(|e| e.to_string())?;
    check_cancelled(cancelled)?;
    Ok(output_frames)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("解析の一時停止が要求されました".into())
    } else {
        Ok(())
    }
}

fn wav_header(frames: u32) -> [u8; 44] {
    let mut header = [0; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + frames * 2).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&1_u16.to_le_bytes());
    header[24..28].copy_from_slice(&WHISPER_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&(WHISPER_RATE * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2_u16.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&(frames * 2).to_le_bytes());
    header
}

// Windowed-sinc anti-aliasing: a raw stride or linear interpolation would alias
// speech-band-external content into the 16 kHz input. Normalize each fractional
// phase for DC preservation. Edge extension avoids attenuation at file edges.
struct Filter {
    half: i64,
    kernels: Vec<Vec<f64>>,
}

impl Filter {
    fn new(rate: u32) -> Self {
        let cutoff = (f64::from(WHISPER_RATE) / f64::from(rate)).min(1.0) * 0.95;
        let half = (16.0 / cutoff).ceil() as i64;
        let kernels = (0..=PHASES)
            .map(|phase| {
                let fraction = phase as f64 / PHASES as f64;
                let mut kernel: Vec<f64> = (-half..=half)
                    .map(|tap| {
                        let distance = tap as f64 - fraction;
                        let x = distance * cutoff;
                        let sinc = if x.abs() < 1e-12 {
                            1.0
                        } else {
                            (PI * x).sin() / (PI * x)
                        };
                        let window = if distance.abs() > half as f64 {
                            0.0
                        } else {
                            0.42 + 0.5 * (PI * distance / half as f64).cos()
                                + 0.08 * (2.0 * PI * distance / half as f64).cos()
                        };
                        cutoff * sinc * window
                    })
                    .collect();
                let sum: f64 = kernel.iter().sum();
                for value in &mut kernel {
                    *value /= sum;
                }
                kernel
            })
            .collect();
        Self { half, kernels }
    }

    fn sample(&self, input: &[f32], index: u64, rate: u32) -> f32 {
        let position = index * u64::from(rate);
        let base = (position / u64::from(WHISPER_RATE)) as i64;
        let phase = ((position % u64::from(WHISPER_RATE)) as usize * PHASES
            + WHISPER_RATE as usize / 2)
            / WHISPER_RATE as usize;
        self.kernels[phase]
            .iter()
            .enumerate()
            .map(|(tap, weight)| {
                let source = (base + tap as i64 - self.half).clamp(0, input.len() as i64 - 1);
                f64::from(input[source as usize]) * weight
            })
            .sum::<f64>() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{tests::caf_fixture, VerifiedAudio};
    use crate::playback::AudioSpan;
    use sha2::{Digest, Sha256};

    fn audio(rate: u32, channels: u32, samples: &[f32]) -> PreparedAudio {
        let mut file = tempfile::tempfile().unwrap();
        let pcm: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let bytes = caf_fixture(rate, channels, 3, 32, 4, &pcm);
        file.write_all(&bytes).unwrap();
        let verified = VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        let frames = verified.format.frames;
        PreparedAudio {
            sample_rate: rate,
            channels,
            start_us: 0,
            end_us: (frames * 1_000_000 / u64::from(rate)) as i64,
            track_kind: "mic".into(),
            alignment: "unverified".into(),
            spans: vec![AudioSpan {
                audio: verified,
                first_frame: 0,
                frames,
            }],
        }
    }

    #[test]
    fn mono_wav_has_fixed_rate_length_and_downmix() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.wav");
        let samples: Vec<f32> = (0..48_000).flat_map(|_| [0.25, 0.75]).collect();
        let mut source = audio(48_000, 2, &samples);
        assert_eq!(
            write_window(&mut source, 0, 48_000, &path, &|| false).unwrap(),
            16_000
        );
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 32_000);
        assert_eq!(&bytes[..44], &wav_header(16_000));
        for sample in bytes[44..].chunks_exact(2) {
            assert_eq!(i16::from_le_bytes(sample.try_into().unwrap()), 16384);
        }
        assert!(write_window(&mut source, 0, 48_000, &path, &|| false).is_err());
        assert!(write_window(
            &mut source,
            0,
            48_001,
            &directory.path().join("bad.wav"),
            &|| false
        )
        .is_err());
        assert!(write_window(
            &mut source,
            0,
            48_000,
            &directory.path().join("pause.wav"),
            &|| true
        )
        .is_err());
        assert!(!directory.path().join("pause.wav").exists());
    }

    #[test]
    fn lowpass_preserves_speech_band_and_rejects_aliases() {
        for rate in [8_000, 44_100, 48_000, 96_000] {
            let filter = Filter::new(rate);
            let frequency = if rate == 8_000 { 1_000.0 } else { 12_000.0 };
            let input: Vec<f32> = (0..rate)
                .map(|i| (2.0 * PI * frequency * f64::from(i) / f64::from(rate)).sin() as f32)
                .collect();
            let rms = (1000..15_000)
                .map(|i| f64::from(filter.sample(&input, i, rate)).powi(2))
                .sum::<f64>()
                / 14_000.0;
            if rate == 8_000 {
                assert!((rms.sqrt() - 0.707).abs() < 0.005);
            } else {
                assert!(rms.sqrt() < 0.001, "{rate}: {}", rms.sqrt());
            }
            let speech: Vec<f32> = (0..rate)
                .map(|i| (2.0 * PI * 1_000.0 * f64::from(i) / f64::from(rate)).sin() as f32)
                .collect();
            let rms = (1000..15_000)
                .map(|i| f64::from(filter.sample(&speech, i, rate)).powi(2))
                .sum::<f64>()
                / 14_000.0;
            assert!((rms.sqrt() - 0.707).abs() < 0.005);
        }
    }

    #[test]
    fn exact_rate_keeps_pcm_and_range_across_spans() {
        let mut source = audio(16_000, 1, &[0.25; 160]);
        let other = audio(16_000, 1, &[-0.5; 160]);
        source.spans.extend(other.spans);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.wav");
        assert_eq!(
            write_window(&mut source, 80, 160, &path, &|| false).unwrap(),
            160
        );
        let bytes = std::fs::read(path).unwrap();
        for sample in bytes[44..204].chunks_exact(2) {
            assert_eq!(i16::from_le_bytes(sample.try_into().unwrap()), 8192);
        }
        for sample in bytes[204..].chunks_exact(2) {
            assert_eq!(i16::from_le_bytes(sample.try_into().unwrap()), -16384);
        }
    }
}
