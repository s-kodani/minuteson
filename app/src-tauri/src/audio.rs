use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::time::SystemTime;

pub(crate) mod flac;

pub const PREVIEW_FRAMES: u64 = 65_536;

// CAF's flags are NOT AudioStreamBasicDescription's flags. See Apple's CAF
// specification: bit 0 = floating point, bit 1 = little endian.
// https://developer.apple.com/library/archive/documentation/MusicAudio/Reference/CAFSpec/CAF_spec/CAF_spec.html
#[derive(Clone, Debug)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u64,
    data_offset: u64,
    bytes_per_frame: u32,
    float: bool,
    little_endian: bool,
}

pub struct VerifiedAudio {
    file: File,
    size: u64,
    modified: SystemTime,
    pub format: PcmFormat,
    flac: Option<flac::FlacAudio>,
}

impl VerifiedAudio {
    #[cfg(test)]
    pub fn open(file: File, expected_size: u64, expected_hash: &str) -> Result<Self, String> {
        Self::open_cancellable(file, expected_size, expected_hash, &|| false)
    }

    pub fn open_cancellable(
        mut file: File,
        expected_size: u64,
        expected_hash: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        file.rewind().map_err(|e| e.to_string())?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() != expected_size {
            return Err("音声ファイルのサイズが保存時と一致しません".into());
        }
        let modified = before.modified().map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 65_536];
        loop {
            if cancelled() {
                return Err("音声ファイルの照合を中断しました".into());
            }
            let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        if format!("{:x}", hasher.finalize()) != expected_hash {
            return Err("音声ファイルのハッシュが保存時と一致しません".into());
        }
        file.rewind().map_err(|e| e.to_string())?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic).map_err(|e| e.to_string())?;
        let (format, flac) = if &magic == b"fLaC" {
            let (format, audio) = flac::FlacAudio::open(&mut file, expected_size, cancelled)?;
            (format, Some(audio))
        } else {
            (read_caf_format(&mut file, expected_size)?, None)
        };
        let audio = Self {
            file,
            size: expected_size,
            modified,
            format,
            flac,
        };
        audio.check_unchanged()?;
        Ok(audio)
    }

    fn check_unchanged(&self) -> Result<(), String> {
        let metadata: Metadata = self.file.metadata().map_err(|e| e.to_string())?;
        if metadata.len() != self.size
            || metadata.modified().map_err(|e| e.to_string())? != self.modified
        {
            return Err("再生準備後に音声ファイルが変更されました".into());
        }
        Ok(())
    }

    pub fn read_frames(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        if frames == 0
            || frames > PREVIEW_FRAMES
            || offset
                .checked_add(frames)
                .is_none_or(|end| end > self.format.frames)
        {
            return Err("音声の読出し範囲が不正です".into());
        }
        self.check_unchanged()?;
        if let Some(flac) = &mut self.flac {
            let output = flac.read_frames(offset, frames)?;
            self.check_unchanged()?;
            return Ok(output);
        }
        let format = &self.format;
        let position = format.data_offset + offset * u64::from(format.bytes_per_frame);
        self.file
            .seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut input = vec![0_u8; frames as usize * format.bytes_per_frame as usize];
        self.file
            .read_exact(&mut input)
            .map_err(|e| e.to_string())?;
        self.check_unchanged()?;
        let sample_bytes = (format.bytes_per_frame / format.channels) as usize;
        let mut output = Vec::with_capacity(frames as usize * format.channels as usize * 4);
        for sample in input.chunks_exact(sample_bytes) {
            let value = decode_sample(sample, format)?;
            output.extend_from_slice(&value.to_le_bytes());
        }
        Ok(output)
    }
}

fn read_caf_format(file: &mut File, size: u64) -> Result<PcmFormat, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut header = [0_u8; 8];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"caff" || header[4..6] != [0, 1] {
        return Err("対応するCAF音声ではありません".into());
    }
    let mut position = 8_u64;
    let mut format: Option<PcmFormat> = None;
    let mut data = None;
    for index in 0..4096 {
        if position == size {
            break;
        }
        if size.saturating_sub(position) < 12 {
            return Err("CAFチャンクのヘッダーが破損しています".into());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0_u8; 12];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let length = i64::from_be_bytes(chunk[4..12].try_into().unwrap());
        let payload = position + 12;
        // Ready files have a finalized length; a -1 length is still being written.
        if length < 0 || length as u64 > size - payload {
            return Err("CAFチャンクの長さが不正または未確定です".into());
        }
        if index == 0 && &chunk[..4] != b"desc" {
            return Err("CAF音声形式の記述がありません".into());
        }
        match &chunk[..4] {
            b"desc" => {
                if format.is_some() || length != 32 {
                    return Err("CAF音声形式の記述が不正です".into());
                }
                let mut desc = [0_u8; 32];
                file.read_exact(&mut desc).map_err(|e| e.to_string())?;
                let rate = f64::from_be_bytes(desc[0..8].try_into().unwrap());
                let flags = u32::from_be_bytes(desc[12..16].try_into().unwrap());
                let bytes_per_frame = u32::from_be_bytes(desc[16..20].try_into().unwrap());
                let frames_per_packet = u32::from_be_bytes(desc[20..24].try_into().unwrap());
                let channels = u32::from_be_bytes(desc[24..28].try_into().unwrap());
                let bits = u32::from_be_bytes(desc[28..32].try_into().unwrap());
                let float = flags & 1 != 0;
                if &desc[8..12] != b"lpcm"
                    || flags & !3 != 0
                    || frames_per_packet != 1
                    || !rate.is_finite()
                    || !(8_000.0..=384_000.0).contains(&rate)
                    || rate.fract() != 0.0
                    || !(1..=8).contains(&channels)
                    || bytes_per_frame % channels != 0
                    || (float && !matches!((bits, bytes_per_frame / channels), (32, 4) | (64, 8)))
                    || (!float
                        && !matches!(
                            (bits, bytes_per_frame / channels),
                            (16, 2 | 4) | (24, 3 | 4) | (32, 4)
                        ))
                {
                    return Err("このCAFのPCM形式は再生に対応していません".into());
                }
                format = Some(PcmFormat {
                    sample_rate: rate as u32,
                    channels,
                    frames: 0,
                    data_offset: 0,
                    bytes_per_frame,
                    float,
                    little_endian: flags & 2 != 0,
                });
            }
            b"data" => {
                if data.is_some() || length < 4 {
                    return Err("CAF音声データが不正です".into());
                }
                data = Some((payload + 4, length as u64 - 4));
            }
            _ => {}
        }
        position = payload + length as u64;
    }
    if position != size {
        return Err("CAFチャンクが多すぎます".into());
    }
    let mut format = format.ok_or("CAF音声形式がありません")?;
    let (offset, bytes) = data.ok_or("CAF音声データがありません")?;
    if bytes == 0 || bytes % u64::from(format.bytes_per_frame) != 0 {
        return Err("CAF音声フレームが破損しています".into());
    }
    format.data_offset = offset;
    format.frames = bytes / u64::from(format.bytes_per_frame);
    Ok(format)
}

fn decode_sample(bytes: &[u8], format: &PcmFormat) -> Result<f32, String> {
    let value = if format.float {
        match bytes.len() {
            4 => {
                (if format.little_endian {
                    f32::from_le_bytes(bytes.try_into().unwrap())
                } else {
                    f32::from_be_bytes(bytes.try_into().unwrap())
                }) as f64
            }
            8 => {
                if format.little_endian {
                    f64::from_le_bytes(bytes.try_into().unwrap())
                } else {
                    f64::from_be_bytes(bytes.try_into().unwrap())
                }
            }
            _ => unreachable!(),
        }
    } else {
        let mut packed = [0_u8; 4];
        if format.little_endian {
            packed[4 - bytes.len()..].copy_from_slice(bytes);
        } else {
            for (target, source) in packed[4 - bytes.len()..].iter_mut().zip(bytes.iter().rev()) {
                *target = *source;
            }
        }
        // Unpacked CAF integers are high aligned; packed samples are sign
        // extended by moving their most significant bit to i32's sign bit.
        f64::from(i32::from_le_bytes(packed)) / 2_147_483_648.0
    };
    if !value.is_finite() {
        return Err("音声に非有限のPCM値が含まれています".into());
    }
    Ok(value.clamp(-1.0, 1.0) as f32)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub fn caf_fixture(
        rate: u32,
        channels: u32,
        flags: u32,
        bits: u32,
        bytes_per_sample: u32,
        samples: &[u8],
    ) -> Vec<u8> {
        let mut file = b"caff\0\x01\0\0desc".to_vec();
        file.extend_from_slice(&32_i64.to_be_bytes());
        file.extend_from_slice(&f64::from(rate).to_be_bytes());
        file.extend_from_slice(b"lpcm");
        for value in [flags, channels * bytes_per_sample, 1, channels, bits] {
            file.extend_from_slice(&value.to_be_bytes());
        }
        file.extend_from_slice(b"free");
        file.extend_from_slice(&3_i64.to_be_bytes());
        file.extend_from_slice(&[0; 3]); // CAF chunks have no RIFF-style padding.
        file.extend_from_slice(b"data");
        file.extend_from_slice(&(samples.len() as i64 + 4).to_be_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(samples);
        file
    }

    fn open_fixture(bytes: &[u8]) -> Result<VerifiedAudio, String> {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        file.rewind().unwrap();
        VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(bytes)),
        )
    }

    #[test]
    fn reads_only_requested_float_frames_in_channel_order() {
        let samples: Vec<u8> = [0.1_f32, 0.2, 0.3, 0.4, 0.5, 0.6]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let bytes = caf_fixture(48_000, 2, 3, 32, 4, &samples);
        let mut audio = open_fixture(&bytes).unwrap();
        let result = audio.read_frames(1, 1).unwrap();
        assert_eq!(result, samples[8..16]);
        assert_eq!(audio.format.frames, 3);
        assert!(audio.read_frames(3, 1).is_err());
        assert!(audio.read_frames(0, PREVIEW_FRAMES + 1).is_err());
    }

    #[test]
    fn big_endian_and_packed_or_padded_integers_have_correct_sign() {
        for (bits, width, little, samples) in [
            (16, 2, false, vec![0x80, 0, 0x40, 0]),
            (24, 3, true, vec![0, 0, 0x80, 0, 0, 0x40]),
            (24, 4, false, vec![0x80, 0, 0, 0, 0x40, 0, 0, 0]),
            (16, 4, true, vec![0, 0, 0, 0x80, 0, 0, 0, 0x40]),
        ] {
            let mut audio = open_fixture(&caf_fixture(
                16_000,
                1,
                if little { 2 } else { 0 },
                bits,
                width,
                &samples,
            ))
            .unwrap();
            let pcm = audio.read_frames(0, 2).unwrap();
            assert_eq!(f32::from_le_bytes(pcm[..4].try_into().unwrap()), -1.0);
            assert_eq!(f32::from_le_bytes(pcm[4..].try_into().unwrap()), 0.5);
        }
        let mut audio =
            open_fixture(&caf_fixture(16_000, 1, 1, 64, 8, &0.25_f64.to_be_bytes())).unwrap();
        assert_eq!(audio.read_frames(0, 1).unwrap(), 0.25_f32.to_le_bytes());
    }

    #[test]
    fn malformed_or_unfinalized_caf_and_nonfinite_samples_are_rejected() {
        let mut bytes = caf_fixture(48_000, 1, 3, 32, 4, &0.5_f32.to_le_bytes());
        assert!(open_fixture(&bytes[..bytes.len() - 1]).is_err());
        let data_header = 8 + 12 + 32 + 12 + 3;
        bytes[data_header + 4..data_header + 12].copy_from_slice(&(-1_i64).to_be_bytes());
        assert!(open_fixture(&bytes).is_err());
        let mut audio =
            open_fixture(&caf_fixture(48_000, 1, 3, 32, 4, &f32::NAN.to_le_bytes())).unwrap();
        assert!(audio.read_frames(0, 1).is_err());
        assert!(open_fixture(&caf_fixture(48_000, 0, 3, 32, 4, &[])).is_err());
    }

    #[test]
    fn changed_hash_and_size_are_rejected() {
        let bytes = caf_fixture(48_000, 1, 3, 32, 4, &0.5_f32.to_le_bytes());
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        file.rewind().unwrap();
        assert!(VerifiedAudio::open(
            file.try_clone().unwrap(),
            bytes.len() as u64,
            &"0".repeat(64)
        )
        .is_err());
        file.rewind().unwrap();
        let mut audio = VerifiedAudio::open(
            file.try_clone().unwrap(),
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        file.set_len(bytes.len() as u64 - 1).unwrap();
        assert!(audio.read_frames(0, 1).is_err());
    }

    #[test]
    #[ignore = "requires AVFoundation fixtures generated by CafWriterSmoke.swift"]
    fn avfoundation_generated_caf_matches_all_channel_samples() {
        let directory =
            std::env::var("MON_PREVIEW_FIXTURE_DIR").expect("MON_PREVIEW_FIXTURE_DIR required");
        for name in ["source.caf", "result.caf"] {
            let path = std::path::Path::new(&directory).join(name);
            let bytes = std::fs::read(&path).unwrap();
            let mut audio = VerifiedAudio::open(
                File::open(path).unwrap(),
                bytes.len() as u64,
                &format!("{:x}", Sha256::digest(&bytes)),
            )
            .unwrap();
            assert_eq!(
                (
                    audio.format.frames,
                    audio.format.channels,
                    audio.format.sample_rate
                ),
                (4800, 2, 48000)
            );
            let result = audio.read_frames(0, 4800).unwrap();
            for frame in 0..4800 {
                for channel in 0..2 {
                    let index = (frame * 2 + channel) * 4;
                    let actual = f32::from_le_bytes(result[index..index + 4].try_into().unwrap());
                    let expected =
                        ((frame % 101) as f32 - 50.0) / 100.0 * (channel + 1) as f32 / 2.0;
                    assert!(
                        (actual - expected).abs() < 0.000001,
                        "{name}, frame {frame}, channel {channel}: {actual} != {expected}"
                    );
                }
            }
        }
    }
}
