//! Streaming Windows import for PCM WAV and strictly verified FLAC. The
//! meeting-mix WAV reader intentionally remains restricted to its fixed format.
use super::*;
#[cfg(windows)]
use std::io::BufWriter;
use std::io::SeekFrom;
struct Wave {
    rate: u32,
    channels: u32,
    frames: u64,
    offset: u64,
    bits: u16,
    valid_bits: u16,
    float: bool,
    unsigned: bool,
}
fn wave(file: &mut File) -> Result<Wave, String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut h = [0u8; 12];
    file.read_exact(&mut h).map_err(|e| e.to_string())?;
    if &h[..4] != b"RIFF"
        || &h[8..] != b"WAVE"
        || u64::from(u32::from_le_bytes(h[4..8].try_into().unwrap())) + 8 != size
    {
        return Err("WAVの形式・容量が不正です".into());
    }
    let mut position = 12u64;
    let mut format = None;
    let mut data = None;
    for _ in 0..4096 {
        if position == size {
            break;
        }
        if size - position < 8 {
            return Err("WAVのチャンクが途中で終了しています".into());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let length = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
        let first = position + 8;
        let end = first
            .checked_add(length)
            .filter(|n| *n <= size)
            .ok_or("WAVのチャンク長が不正です")?;
        match &chunk[..4] {
            b"fmt " => {
                if format.is_some() || !(16..=4096).contains(&length) {
                    return Err("WAVの音声形式が不正です".into());
                }
                let mut fmt = vec![0; length as usize];
                file.read_exact(&mut fmt).map_err(|e| e.to_string())?;
                let u16at = |n| u16::from_le_bytes(fmt[n..n + 2].try_into().unwrap());
                let u32at = |n| u32::from_le_bytes(fmt[n..n + 4].try_into().unwrap());
                let mut codec = u16at(0);
                let channels = u32::from(u16at(2));
                let rate = u32at(4);
                let bits = u16at(14);
                let mut valid_bits = bits;
                if codec == 0xfffe {
                    if length < 40
                        || u16at(16) < 22
                        || usize::from(u16at(16)) + 18 > fmt.len()
                        || fmt[26..40] != [0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]
                    {
                        return Err("WAV拡張形式のGUIDが不正です".into());
                    }
                    codec = u16at(24);
                    valid_bits = u16at(18);
                }
                let float = codec == 3;
                if !(8000..=384000).contains(&rate)
                    || !(1..=8).contains(&channels)
                    || !matches!((codec, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64))
                    || valid_bits == 0
                    || valid_bits > bits
                    || (float && valid_bits != bits)
                    || u32::from(u16at(12)) != channels * u32::from(bits / 8)
                    || u32at(8) != rate * channels * u32::from(bits / 8)
                {
                    return Err("WAVは対応するPCM音声形式にしてください".into());
                }
                format = Some((rate, channels, bits, valid_bits, float));
            }
            b"data" => {
                if data.replace((first, length)).is_some() {
                    return Err("WAVに複数の音声領域があります".into());
                }
            }
            _ => {}
        }
        position = end + (length & 1);
        if position > size {
            return Err("WAVのパディングが不正です".into());
        }
    }
    if position != size {
        return Err("WAVのチャンク数が上限を超えています".into());
    }
    let (rate, channels, bits, valid_bits, float) = format.ok_or("WAVの音声形式がありません")?;
    let (offset, bytes) = data.ok_or("WAVに音声がありません")?;
    let block = u64::from(channels) * u64::from(bits / 8);
    if bytes == 0 || bytes % block != 0 {
        return Err("WAV音声がフレームの途中で終了しています".into());
    }
    Ok(Wave {
        rate,
        channels,
        frames: bytes / block,
        offset,
        bits,
        valid_bits,
        float,
        unsigned: bits == 8,
    })
}
impl Wave {
    fn sample(&self, sample: &[u8]) -> Result<f32, String> {
        // WAVEFORMATEXTENSIBLE stores integer valid bits left-aligned in the
        // container. Discard the unused low bits before sign/scale conversion.
        let shift = self.bits - self.valid_bits;
        let value = match (self.float, self.bits) {
            (true, 32) => f32::from_le_bytes(sample.try_into().unwrap()),
            (true, 64) => f64::from_le_bytes(sample.try_into().unwrap()) as f32,
            (false, 8) if self.unsigned => {
                let midpoint = 1u32 << (self.valid_bits - 1);
                (f32::from(sample[0] >> shift) - midpoint as f32) / midpoint as f32
            }
            (false, 16 | 24 | 32) => {
                let integer = match self.bits {
                    16 => i32::from(i16::from_le_bytes(sample.try_into().unwrap())),
                    24 => (i32::from_le_bytes([sample[0], sample[1], sample[2], 0]) << 8) >> 8,
                    32 => i32::from_le_bytes(sample.try_into().unwrap()),
                    _ => unreachable!(),
                };
                (integer >> shift) as f32 / (1u64 << (self.valid_bits - 1)) as f32
            }
            _ => return Err("WAVのPCM形式が不正です".into()),
        };
        if !value.is_finite() {
            return Err("WAVに非有限のPCM値があります".into());
        }
        Ok(value)
    }
}
#[cfg(windows)]
pub(super) fn decode(
    input: &Path,
    output: Option<&Path>,
    cancelled: &impl Fn() -> bool,
) -> Result<AudioInfo, String> {
    let proof = Fingerprint::capture(input, cancelled)?;
    let watch = StableFile::watch(&proof)?;
    let mut file = File::open(input).map_err(|e| e.to_string())?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let mut flac = None;
    let mut wav = None;
    let (rate, channels, frames, format, codec) = if &magic == b"fLaC" {
        let audio = VerifiedAudio::open_cancellable(
            file.try_clone().map_err(|e| e.to_string())?,
            proof.size,
            &proof.sha256,
            cancelled,
        )?;
        let values = (
            audio.format.sample_rate,
            audio.format.channels,
            audio.format.frames,
            "flac",
            "flac",
        );
        flac = Some(audio);
        values
    } else {
        let w = wave(&mut file)?;
        let values = (w.rate, w.channels, w.frames, "wav", "pcm");
        file.seek(SeekFrom::Start(w.offset))
            .map_err(|e| e.to_string())?;
        wav = Some(w);
        values
    };
    let info = AudioInfo {
        schema_version: 1,
        detected_format: format.into(),
        codec: codec.into(),
        sample_rate: rate,
        channels,
        frames,
        duration_us: (frames * 1000000 / u64::from(rate)) as i64,
    };
    info.validate()?;
    let mut writer = output
        .map(|p| {
            let f = File::options()
                .write(true)
                .create_new(true)
                .open(p)
                .map_err(|e| e.to_string())?;
            crate::windows_files::set_private(p)?;
            Ok::<_, String>(BufWriter::new(f))
        })
        .transpose()?;
    if let Some(w) = &mut writer {
        w.write_all(&super::mp3::caf_header(rate, channels, frames)?)
            .map_err(|e| e.to_string())?;
    }
    let mut first = 0;
    while first < frames {
        check_cancelled(cancelled)?;
        let count = (frames - first).min(4096);
        let pcm = if let Some(audio) = &mut flac {
            audio.read_frames(first, count)?
        } else {
            let w = wav.as_ref().unwrap();
            let bytes = usize::from(w.bits / 8);
            let mut raw = vec![0; count as usize * channels as usize * bytes];
            file.read_exact(&mut raw).map_err(|e| e.to_string())?;
            let mut pcm = Vec::with_capacity(count as usize * channels as usize * 4);
            for sample in raw.chunks_exact(bytes) {
                let value = w.sample(sample)?;
                pcm.extend_from_slice(&value.to_le_bytes());
            }
            pcm
        };
        if let Some(w) = &mut writer {
            w.write_all(&pcm).map_err(|e| e.to_string())?;
        }
        first += count;
    }
    if let Some(mut w) = writer {
        w.flush().map_err(|e| e.to_string())?;
        w.get_ref().sync_all().map_err(|e| e.to_string())?;
    }
    watch.verify()?;
    proof.verify(cancelled)?;
    Ok(info)
}

#[cfg(test)]
mod tests;
