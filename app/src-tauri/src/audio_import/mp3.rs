//! Bounded MP3 decode; unused ID3 never reaches a metadata/image parser.
use super::{check_cancelled, AudioInfo};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_MP3};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::probe::Hint;

struct AudioRange {
    file: File,
    first: u64,
    length: u64,
    position: u64,
}
impl Read for AudioRange {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let count = (out.len() as u64).min(self.length.saturating_sub(self.position)) as usize;
        self.file
            .seek(SeekFrom::Start(self.first + self.position))?;
        let count = self.file.read(&mut out[..count])?;
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for AudioRange {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let n = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        if n < 0 || n > i128::from(self.length) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid MP3 seek",
            ));
        }
        self.position = n as u64;
        Ok(self.position)
    }
}
impl MediaSource for AudioRange {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

fn audio_range(file: &mut File) -> Result<(u64, u64), String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut first = 0;
    for _ in 0..16 {
        file.seek(SeekFrom::Start(first))
            .map_err(|e| e.to_string())?;
        let mut h = [0; 10];
        file.read_exact(&mut h).map_err(|e| e.to_string())?;
        if &h[..3] != b"ID3" {
            break;
        }
        if !(2..=4).contains(&h[3]) || h[6..].iter().any(|n| n & 128 != 0) {
            return Err("MP3のID3ヘッダーが不正です".into());
        }
        let length = h[6..].iter().fold(0_u64, |n, b| (n << 7) | u64::from(*b));
        first = first
            .checked_add(10 + length + if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 })
            .ok_or("ID3長が範囲外です")?;
        if first > 16 * 1024 * 1024 || first >= size {
            return Err("MP3のID3長が不正または大きすぎます".into());
        }
    }
    let mut end = size;
    if size >= 128 {
        file.seek(SeekFrom::Start(size - 128))
            .map_err(|e| e.to_string())?;
        let mut h = [0; 3];
        file.read_exact(&mut h).map_err(|e| e.to_string())?;
        if &h == b"TAG" {
            end -= 128;
        }
    }
    if first >= end {
        return Err("MP3音声がありません".into());
    }
    Ok((first, end - first))
}

// MPEG layer III frame geometry, also independently checked by the decoder.
// No resynchronization past missing/partial frames, free bitrate or junk.
fn frame_header(h: [u8; 4]) -> Result<(u32, u32, u64, usize, usize), String> {
    let version = (h[1] >> 3) & 3;
    let layer = (h[1] >> 1) & 3;
    let rate_index = (h[2] >> 2) & 3;
    let bit_index = (h[2] >> 4) as usize;
    if h[0] != 255
        || h[1] & 0xe0 != 0xe0
        || version == 1
        || layer != 1
        || rate_index == 3
        || bit_index == 0
        || bit_index == 15
    {
        return Err("MP3フレームの形式が不正または未対応です".into());
    }
    let rate = [44100, 48000, 32000][rate_index as usize]
        / match version {
            3 => 1,
            2 => 2,
            _ => 4,
        };
    let bitrate = if version == 3 {
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ][bit_index]
    } else {
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bit_index]
    };
    let channels = if h[3] >> 6 == 3 { 1 } else { 2 };
    let frames = if version == 3 { 1152 } else { 576 };
    let length = ((if version == 3 { 144000 } else { 72000 }) * bitrate / rate
        + u32::from((h[2] >> 1) & 1)) as usize;
    let side = if version == 3 {
        if channels == 1 {
            17
        } else {
            32
        }
    } else if channels == 1 {
        9
    } else {
        17
    };
    let tag_offset = 4 + if h[1] & 1 == 0 { 2 } else { 0 } + side;
    if length < 4 || length > 4096 {
        return Err("MP3フレーム長が不正です".into());
    }
    Ok((rate, channels, frames, length, tag_offset))
}
fn scan(
    file: File,
    first: u64,
    length: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<(u32, u32, u64), String> {
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(first))
        .map_err(|e| e.to_string())?;
    let mut offset = 0;
    let mut total = 0;
    let mut format = None;
    let mut discard = 0;
    while offset < length {
        check_cancelled(cancelled)?;
        let mut h = [0; 4];
        reader
            .read_exact(&mut h)
            .map_err(|_| "MP3がフレームの途中で終了しています")?;
        let (rate, channels, frames, bytes, tag_offset) = frame_header(h)?;
        if format.is_some_and(|f| f != (rate, channels)) || bytes as u64 > length - offset {
            return Err("MP3の形式が途中で変わるか音声が切れています".into());
        }
        format = Some((rate, channels));
        if offset == 0 {
            let mut body = vec![0; bytes - 4];
            reader.read_exact(&mut body).map_err(|e| e.to_string())?;
            if body
                .get(tag_offset - 4..tag_offset)
                .is_some_and(|tag| tag == b"Xing" || tag == b"Info")
                || body.get(32..36) == Some(b"VBRI")
            {
                discard = frames;
            }
        } else {
            reader
                .seek_relative(bytes as i64 - 4)
                .map_err(|e| e.to_string())?;
        }
        offset += bytes as u64;
        total += frames;
        if total > u64::from(rate) * 14400 + 8192 {
            return Err("MP3は4時間以内にしてください".into());
        }
    }
    let (rate, channels) = format.ok_or("MP3音声がありません")?;
    Ok((rate, channels, total - discard))
}
pub(super) fn decode(
    input: &Path,
    output: Option<&Path>,
    cancelled: &impl Fn() -> bool,
) -> Result<AudioInfo, String> {
    let mut file = File::open(input).map_err(|e| e.to_string())?;
    let (first, length) = audio_range(&mut file)?;
    let (rate, channels, expected_raw) = scan(
        file.try_clone().map_err(|e| e.to_string())?,
        first,
        length,
        cancelled,
    )?;
    let source = AudioRange {
        file,
        first,
        length,
        position: 0,
    };
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    // Perform checked gapless trimming ourselves. Untrusted encoder padding
    // cannot underflow the demuxer's declared duration calculation.
    let mut reader = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default()),
            &FormatOptions::default(),
            &Default::default(),
        )
        .map_err(|e| e.to_string())?
        .format;
    if reader.tracks().len() != 1 {
        return Err("MP3は単一音声である必要があります".into());
    }
    let p = reader.tracks()[0].codec_params.clone();
    if p.codec != CODEC_TYPE_MP3
        || p.sample_rate != Some(rate)
        || p.channels.map(|c| c.count() as u32) != Some(channels)
    {
        return Err("MP3フレームと復号形式が一致しません".into());
    }
    let delay = u64::from(p.delay.unwrap_or(0));
    let padding = u64::from(p.padding.unwrap_or(0));
    if delay.checked_add(padding).is_none_or(|n| n >= expected_raw)
        || (delay + padding > 0 && p.n_frames != Some(expected_raw))
    {
        return Err("MP3の遅延・末尾パディングが不正です".into());
    }
    let end = expected_raw - padding;
    let mut decoder = symphonia::default::get_codecs()
        .make(&p, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut output = output
        .map(|path| {
            File::options()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    if let Some(file) = &mut output {
        file.write_all(&caf_header(rate, channels, 0)?)
            .map_err(|e| e.to_string())?;
    }
    let mut raw_frames = 0;
    let mut frames = 0;
    loop {
        check_cancelled(cancelled)?;
        let packet = match reader.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        };
        let audio = decoder.decode(&packet).map_err(|e| e.to_string())?;
        if audio.spec().rate != rate
            || audio.spec().channels.count() as u32 != channels
            || audio.frames() > 1152
            || audio.frames() == 0
        {
            return Err("MP3のPCM形式が不正です".into());
        }
        let count = audio.frames() as u64;
        let to = (raw_frames + count).min(end);
        let from = raw_frames.max(delay);
        let mut buffer = SampleBuffer::<f32>::new(count, *audio.spec());
        buffer.copy_interleaved_ref(audio);
        if to > from {
            let first = ((from - raw_frames) * u64::from(channels)) as usize;
            let last = ((to - raw_frames) * u64::from(channels)) as usize;
            let mut pcm = Vec::with_capacity((last - first) * 4);
            for value in &buffer.samples()[first..last] {
                if !value.is_finite() {
                    return Err("MP3に非有限のPCM値があります".into());
                }
                if output.is_some() {
                    pcm.extend_from_slice(&value.to_le_bytes());
                }
            }
            if let Some(file) = &mut output {
                file.write_all(&pcm).map_err(|e| e.to_string())?;
            }
            frames += to - from;
        }
        raw_frames += count;
        if raw_frames > expected_raw {
            return Err("MP3の実フレーム数が保存情報と一致しません".into());
        }
    }
    if raw_frames != expected_raw || frames != end - delay || frames == 0 {
        return Err("MP3の実フレーム数が保存情報と一致しません".into());
    }
    check_cancelled(cancelled)?;
    if let Some(file) = &mut output {
        file.rewind().map_err(|e| e.to_string())?;
        file.write_all(&caf_header(rate, channels, frames)?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    let info = AudioInfo {
        schema_version: 1,
        detected_format: "mp3".into(),
        codec: "mp3".into(),
        sample_rate: rate,
        channels,
        frames,
        duration_us: (frames * 1_000_000 / u64::from(rate)) as i64,
    };
    info.validate()?;
    Ok(info)
}
pub(super) fn caf_header(rate: u32, channels: u32, frames: u64) -> Result<Vec<u8>, String> {
    let length = frames
        .checked_mul(u64::from(channels) * 4)
        .and_then(|n| n.checked_add(4))
        .and_then(|n| i64::try_from(n).ok())
        .ok_or("PCM長が範囲外です")?;
    let mut h = b"caff\0\x01\0\0desc".to_vec();
    h.extend_from_slice(&32_i64.to_be_bytes());
    h.extend_from_slice(&f64::from(rate).to_be_bytes());
    h.extend_from_slice(b"lpcm");
    for n in [3, channels * 4, 1, channels, 32] {
        h.extend_from_slice(&n.to_be_bytes());
    }
    h.extend_from_slice(b"data");
    h.extend_from_slice(&length.to_be_bytes());
    h.extend_from_slice(&[0; 4]);
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_frame_geometry_rejects_free_bitrate_reserved_layer_and_bad_sync() {
        assert_eq!(
            frame_header([255, 251, 144, 0]).unwrap(),
            (44100, 2, 1152, 417, 36)
        );
        assert_eq!(
            frame_header([255, 243, 136, 192]).unwrap(),
            (16000, 1, 576, 288, 13)
        );
        assert_eq!(
            frame_header([255, 227, 136, 192]).unwrap(),
            (8000, 1, 576, 576, 13)
        );
        for h in [
            [0, 251, 144, 0],
            [255, 251, 0, 0],
            [255, 251, 252, 0],
            [255, 233, 144, 0],
        ] {
            assert!(frame_header(h).is_err());
        }
    }

    #[test]
    fn mp3_metadata_and_truncated_frame_lengths_are_bounded_before_decoder() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input.mp3");
        for bytes in [
            b"ID3\x04\0\0\x7f\x7f\x7f\x7f".as_slice(),
            b"ID3\x04\0\0\x80\0\0\0".as_slice(),
            b"ID3\x01\0\0\0\0\0\0".as_slice(),
            b"ID3\x04\0\0\0\0\0\x7f".as_slice(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(decode(&path, None, &crate::test_support::never_cancelled).is_err());
        }
        std::fs::write(&path, [255, 251, 144, 0, 0]).unwrap();
        assert!(scan(
            File::open(&path).unwrap(),
            0,
            5,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(caf_header(384000, 8, u64::MAX).is_err());
        assert!(decode(&path, None, &|| true).is_err());
    }

    #[test]
    #[ignore = "requires MON_IMPORT_FFMPEG; independent MPEG1/2/2.5 and gapless PCM comparison"]
    fn ffmpeg_mp3_variants_preserve_gapless_count_pcm_and_reject_truncation() {
        use crate::audio::VerifiedAudio;
        use crate::local_engine::Fingerprint;
        use std::process::Command;
        let ffmpeg = std::env::var("MON_IMPORT_FFMPEG").unwrap();
        let root = tempfile::tempdir().unwrap();
        for (rate, channels, vbr) in [(44100_u32, 2, true), (16000, 2, false), (8000, 1, true)] {
            let mp3 = root.path().join(format!("{rate}.unrelated"));
            let result = Command::new(&ffmpeg)
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=300:duration=2",
                ])
                .args([
                    "-ar",
                    &rate.to_string(),
                    "-ac",
                    &channels.to_string(),
                    "-c:a",
                    "libmp3lame",
                ])
                .args(if vbr { ["-q:a", "4"] } else { ["-b:a", "64k"] })
                .args(["-f", "mp3"])
                .arg(&mp3)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let inspected = decode(&mp3, None, &crate::test_support::never_cancelled).unwrap();
            assert_eq!(inspected.frames, u64::from(rate) * 2);
            assert_eq!(inspected.channels, channels);
            let caf = root.path().join(format!("{rate}.caf"));
            assert_eq!(
                decode(&mp3, Some(&caf), &crate::test_support::never_cancelled).unwrap(),
                inspected
            );
            assert!(decode(&mp3, Some(&caf), &crate::test_support::never_cancelled).is_err());
            let raw = root.path().join(format!("{rate}.raw"));
            let result = Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&mp3)
                .args(["-f", "f32le"])
                .arg(&raw)
                .output()
                .unwrap();
            assert!(result.status.success());
            let expected = std::fs::read(raw).unwrap();
            assert_eq!(
                expected.len() as u64,
                inspected.frames * u64::from(channels) * 4
            );
            let proof = Fingerprint::capture(&caf, &crate::test_support::never_cancelled).unwrap();
            let mut audio =
                VerifiedAudio::open(File::open(&caf).unwrap(), proof.size, &proof.sha256).unwrap();
            let mut offset = 0;
            while offset < inspected.frames {
                let n = (inspected.frames - offset).min(4096);
                let samples = audio.read_frames(offset, n).unwrap();
                for (i, sample) in samples.chunks_exact(4).enumerate() {
                    let sample = f32::from_le_bytes(sample.try_into().unwrap());
                    let first = (offset as usize * channels as usize + i) * 4;
                    let expected =
                        f32::from_le_bytes(expected[first..first + 4].try_into().unwrap());
                    assert!(
                        (sample - expected).abs() < 0.001,
                        "{rate}: {sample} != {expected}"
                    );
                }
                offset += n;
            }
            let bytes = std::fs::read(&mp3).unwrap();
            std::fs::write(&mp3, &bytes[..bytes.len() - 1]).unwrap();
            assert!(decode(&mp3, None, &crate::test_support::never_cancelled).is_err());
        }
    }
}
