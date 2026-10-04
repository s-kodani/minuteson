//! Native FLAC storage and bounded random access, shared by playback and AI.
//! No subprocess, whole-meeting PCM allocation, or persistent decoded copy.
use super::{PcmFormat, VerifiedAudio};
use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::error::Verify;
use flacenc::source::{Fill, FrameBuf};
use md5::{Digest, Md5};
use sha2::Sha256;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_FLAC};
use symphonia::core::errors::Error;
use symphonia::core::formats::Packet;
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::probe::Hint;

const BLOCK: usize = 4096;
const MAX_SECONDS: u64 = 14_400;
const MAX_METADATA: u64 = 16 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 4 * 1024 * 1024;
const MAX_PACKETS: usize = 2_000_000;

struct FrameIndex {
    offset: u64,
    bytes: usize,
    start: u64,
    frames: u64,
}

// The application does not use artwork, comments, cues or a file-supplied
// seek table. Expose only STREAMINFO and audio to the demuxer. Bounding an
// outer metadata block alone cannot bound allocations from nested lengths
// (including base64 pictures inside comments). Do not parse those fields.
struct EssentialFlacSource {
    file: File,
    header: Vec<u8>,
    audio_offset: u64,
    size: u64,
    position: u64,
}

impl Read for EssentialFlacSource {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let count = if self.position < 42 {
            let start = self.position as usize;
            let count = output.len().min(42 - start);
            output[..count].copy_from_slice(&self.header[start..start + count]);
            count
        } else if self.position >= self.size {
            0
        } else {
            let count = (output.len() as u64).min(self.size - self.position) as usize;
            positional_read(
                &self.file,
                self.audio_offset + (self.position - 42),
                &mut output[..count],
            )?
        };
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for EssentialFlacSource {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.size) + i128::from(n),
        };
        self.position = u64::try_from(position).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid FLAC seek")
        })?;
        Ok(self.position)
    }
}

impl MediaSource for EssentialFlacSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.size)
    }
}

pub(super) struct FlacAudio {
    file: File,
    index: Vec<FrameIndex>,
    decoder: Box<dyn Decoder>,
    rate: u32,
    channels: u32,
    cached_start: u64,
    cached: Vec<f32>,
}

impl FlacAudio {
    pub(super) fn open(
        file: &mut File,
        size: u64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(PcmFormat, Self), String> {
        // Validate lengths before a metadata parser can allocate an image/tag.
        let mut byte_offset = check_metadata(file, size, cancelled)?;
        let mut header = read_at(file, 0, 42)?;
        header[4] = 0x80; // STREAMINFO is the last block of this virtual source.
        let source = MediaSourceStream::new(
            Box::new(EssentialFlacSource {
                file: file.try_clone().map_err(|e| e.to_string())?,
                header,
                audio_offset: byte_offset,
                size: (size - byte_offset) + 42,
                position: 0,
            }),
            MediaSourceStreamOptions::default(),
        );
        let mut hint = Hint::new();
        hint.with_extension("flac");
        let mut reader = symphonia::default::get_probe()
            .format(&hint, source, &Default::default(), &Default::default())
            .map_err(|e| format!("FLAC形式を確認できません: {e}"))?
            .format;
        if reader.tracks().len() != 1 {
            return Err("FLACは単一音声トラックである必要があります".into());
        }
        let parameters = reader.tracks()[0].codec_params.clone();
        let rate = parameters.sample_rate.ok_or("FLACの周波数がありません")?;
        let channels = parameters
            .channels
            .ok_or("FLACのチャネルがありません")?
            .count() as u32;
        if parameters.codec != CODEC_TYPE_FLAC
            || !(8_000..=384_000).contains(&rate)
            || !(1..=8).contains(&channels)
            || !matches!(parameters.bits_per_sample, Some(4..=32))
            || parameters
                .n_frames
                .is_some_and(|n| n == 0 || n > u64::from(rate) * MAX_SECONDS)
        {
            return Err("FLACの形式または時間が対応範囲外です".into());
        }
        let mut decoder = symphonia::default::get_codecs()
            .make(&parameters, &DecoderOptions { verify: true })
            .map_err(|e| e.to_string())?;
        let mut frames = 0_u64;
        let mut index = Vec::new();
        loop {
            check_cancelled(cancelled)?;
            let packet = match reader.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(format!("FLACの読出しに失敗しました: {e}")),
            };
            if packet.track_id() != reader.tracks()[0].id || packet.ts() != frames {
                return Err("FLACの音声区間に欠損または重複があります".into());
            }
            if packet.data.len() > MAX_PACKET_BYTES || index.len() >= MAX_PACKETS {
                return Err("FLACの区間数または区間サイズが上限を超えています".into());
            }
            // Demuxers may resynchronize past damaged bytes. Require that
            // every encoded byte belongs to the contiguous verified packets.
            let bytes = read_at(file, byte_offset, packet.data.len())?;
            if bytes.as_slice() != packet.data.as_ref() || !valid_crc(&bytes) {
                return Err("FLACの音声バイト列またはCRCが一致しません".into());
            }
            index.push(FrameIndex {
                offset: byte_offset,
                bytes: bytes.len(),
                start: frames,
                frames: packet.dur(),
            });
            byte_offset = byte_offset
                .checked_add(bytes.len() as u64)
                .ok_or("FLAC位置が範囲外です")?;
            let audio = decoder
                .decode(&packet)
                .map_err(|e| format!("FLACの音声が破損しています: {e}"))?;
            if audio.spec().rate != rate
                || audio.spec().channels.count() as u32 != channels
                || audio.frames() == 0
                || audio.frames() as u64 != packet.dur()
            {
                return Err("FLACの区間と形式が一致しません".into());
            }
            frames = frames
                .checked_add(audio.frames() as u64)
                .ok_or("FLAC時間が範囲外です")?;
            if frames > u64::from(rate) * MAX_SECONDS {
                return Err("FLACは4時間以内にしてください".into());
            }
        }
        if frames == 0 || byte_offset != size || parameters.n_frames.is_some_and(|n| n != frames) {
            return Err("FLACの実フレーム数が保存記録と一致しません".into());
        }
        if decoder.finalize().verify_ok == Some(false) {
            return Err("FLACのPCM検証ハッシュが一致しません".into());
        }
        check_cancelled(cancelled)?;
        // Full-stream MD5 belongs only to the initial validation. A seeking
        // decoder must not treat reordered playback as another full stream.
        let decoder = symphonia::default::get_codecs()
            .make(&parameters, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let format = PcmFormat {
            sample_rate: rate,
            channels,
            frames,
            data_offset: 0,
            bytes_per_frame: channels * 4,
            float: true,
            little_endian: true,
        };
        Ok((
            format,
            Self {
                file: file.try_clone().map_err(|e| e.to_string())?,
                index,
                decoder,
                rate,
                channels,
                cached_start: 0,
                cached: vec![],
            },
        ))
    }

    pub(super) fn read_frames(&mut self, first: u64, count: u64) -> Result<Vec<u8>, String> {
        let mut position = first;
        let end = first + count; // VerifiedAudio checked overflow/range.
        let mut output = Vec::with_capacity(count as usize * self.channels as usize * 4);
        while position < end {
            let cached_end =
                self.cached_start + self.cached.len() as u64 / u64::from(self.channels);
            if position < self.cached_start || position >= cached_end {
                // Seek by the fully validated packet index, including a short
                // last block. Do not rely on a demuxer resync or replay a long
                // meeting from the beginning for every preview/AI window.
                let entry = self
                    .index
                    .partition_point(|p| p.start <= position)
                    .checked_sub(1)
                    .and_then(|i| self.index.get(i))
                    .ok_or("FLAC区間がありません")?;
                let bytes = read_at(&self.file, entry.offset, entry.bytes)?;
                if !valid_crc(&bytes) {
                    return Err("FLACの再生区間CRCが一致しません".into());
                }
                let packet = Packet::new_from_boxed_slice(
                    0,
                    entry.start,
                    entry.frames,
                    bytes.into_boxed_slice(),
                );
                self.decoder.reset();
                if packet.ts() > position || position >= packet.ts().saturating_add(packet.dur()) {
                    return Err("FLACの要求区間と移動先が一致しません".into());
                }
                let audio = self.decoder.decode(&packet).map_err(|e| e.to_string())?;
                if audio.spec().rate != self.rate
                    || audio.spec().channels.count() as u32 != self.channels
                    || audio.frames() as u64 != packet.dur()
                {
                    return Err("FLACの再生形式が変更されています".into());
                }
                let mut pcm = SampleBuffer::<f32>::new(audio.frames() as u64, *audio.spec());
                pcm.copy_interleaved_ref(audio);
                self.cached_start = packet.ts();
                self.cached.clear();
                self.cached.extend_from_slice(pcm.samples());
                continue;
            }
            let to = end.min(cached_end);
            let from_sample = ((position - self.cached_start) * u64::from(self.channels)) as usize;
            let to_sample = ((to - self.cached_start) * u64::from(self.channels)) as usize;
            for sample in &self.cached[from_sample..to_sample] {
                if !sample.is_finite() {
                    return Err("FLACに非有限のPCM値があります".into());
                }
                output.extend_from_slice(&sample.to_le_bytes());
            }
            position = to;
        }
        Ok(output)
    }
}

fn check_metadata(
    file: &mut File,
    size: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<u64, String> {
    file.rewind().map_err(|e| e.to_string())?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != b"fLaC" {
        return Err("FLAC署名がありません".into());
    }
    let mut position = 4_u64;
    for index in 0..4096 {
        check_cancelled(cancelled)?;
        let mut header = [0; 4];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as u64;
        position = position
            .checked_add(4 + length)
            .ok_or("FLACメタデータ長が範囲外です")?;
        if position > size
            || position > MAX_METADATA
            || header[0] & 127 == 127
            || (index == 0 && (header[0] & 127 != 0 || length != 34))
            || (index != 0 && header[0] & 127 == 0)
        {
            return Err("FLACメタデータが不正または大きすぎます".into());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        if header[0] & 128 != 0 {
            return Ok(position);
        }
    }
    Err("FLACメタデータが多すぎます".into())
}

fn valid_crc(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && crc::Crc::<u16>::new(&crc::CRC_16_UMTS).checksum(&bytes[..bytes.len() - 2])
            == u16::from_be_bytes(bytes[bytes.len() - 2..].try_into().unwrap())
}

fn read_at(file: &File, offset: u64, count: usize) -> Result<Vec<u8>, String> {
    if count > MAX_PACKET_BYTES {
        return Err("FLAC区間が大きすぎます".into());
    }
    let mut bytes = vec![0; count];
    let mut done = 0;
    while done < count {
        let read = positional_read(file, offset + done as u64, &mut bytes[done..])
            .map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("FLAC音声区間が途中で終了しました".into());
        }
        done += read;
    }
    Ok(bytes)
}
fn positional_read(file: &File, offset: u64, output: &mut [u8]) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_at(output, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        file.seek_read(output, offset)
    }
    #[cfg(not(any(unix, windows)))]
    Err(std::io::Error::other("positional audio reads unavailable"))
}
fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("FLAC音声処理を中断しました".into())
    } else {
        Ok(())
    }
}

#[derive(Debug)]
#[allow(dead_code)] // Metadata is consumed by the upcoming storage/import coordinator.
pub(crate) struct SavedFlac {
    pub size: u64,
    pub sha256: String,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: u32,
}

#[derive(Debug)]
#[allow(dead_code)] // The coordinator must distinguish a published file from no file.
pub(crate) struct FlacWriteError {
    pub published: bool,
    pub message: String,
}

// Streaming writer usable for both captured PCM and imported decoded audio.
// f32 input is explicitly quantized to signed 24-bit PCM. This is lossless
// relative to that PCM, not a claim of bit-identical floating-point storage.
#[allow(dead_code)] // Recording/import orchestration follows this shared codec.
pub(crate) fn write_segment(
    source: &mut VerifiedAudio,
    first: u64,
    frames: u64,
    path: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<SavedFlac, FlacWriteError> {
    write_segment_with_sync(
        source,
        first,
        frames,
        path,
        cancelled,
        progress,
        &|parent| {
            #[cfg(unix)]
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(())
        },
    )
}

fn write_segment_with_sync(
    source: &mut VerifiedAudio,
    first: u64,
    frames: u64,
    path: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
    sync_parent: &impl Fn(&Path) -> Result<(), String>,
) -> Result<SavedFlac, FlacWriteError> {
    let mut published = false;
    let result = (|| -> Result<SavedFlac, String> {
        if frames == 0
            || frames > u64::from(source.format.sample_rate) * MAX_SECONDS
            || first
                .checked_add(frames)
                .is_none_or(|end| end > source.format.frames)
        {
            return Err("FLAC保存区間が不正です".into());
        }
        let parent = path.parent().ok_or("FLAC保存フォルダーがありません")?;
        check_cancelled(cancelled)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".mon-flac-")
            .suffix(".part")
            .tempfile_in(parent)
            .map_err(|e| e.to_string())?;
        let rate = source.format.sample_rate;
        let channels = source.format.channels;
        let mut info =
            StreamInfo::new(rate as usize, channels as usize, 24).map_err(|e| e.to_string())?;
        info.set_block_sizes(BLOCK, BLOCK)
            .map_err(|e| e.to_string())?;
        let config = flacenc::config::Encoder::default()
            .into_verified()
            .map_err(|(_, e)| e.to_string())?;
        let mut fb = FrameBuf::with_size(channels as usize, BLOCK).map_err(|e| e.to_string())?;
        // Fixed-size metadata can be replaced after the last frame is complete.
        let header = stream_header(&info)?;
        temporary.write_all(&header).map_err(|e| e.to_string())?;
        let mut md5 = Md5::new();
        let mut offset = 0;
        let mut frame_number = 0;
        while offset < frames {
            check_cancelled(cancelled)?;
            let count = (frames - offset).min(BLOCK as u64);
            let pcm = source.read_frames(first + offset, count)?;
            let mut samples = Vec::with_capacity(pcm.len() / 4);
            for sample in pcm.chunks_exact(4) {
                let value = f32::from_le_bytes(sample.try_into().unwrap());
                if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
                    return Err("FLAC保存PCMの値が不正です".into());
                }
                let quantized = (f64::from(value) * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                md5.update(&quantized.to_le_bytes()[..3]);
                samples.push(quantized);
            }
            fb.fill_interleaved(&samples).map_err(|e| e.to_string())?;
            let frame = flacenc::encode_fixed_size_frame(&config, &fb, frame_number, &info)
                .map_err(|e| e.to_string())?;
            info.update_frame_info(&frame);
            let mut sink = flacenc::bitsink::ByteSink::new();
            frame.write(&mut sink).map_err(|e| e.to_string())?;
            temporary
                .write_all(sink.as_slice())
                .map_err(|e| e.to_string())?;
            offset += count;
            frame_number += 1;
            progress(offset);
        }
        // Fixed-block STREAMINFO must report the nominal size, excluding the
        // short last block, as required by RFC 9639 section 8.2.
        info.set_block_sizes(BLOCK, BLOCK)
            .map_err(|e| e.to_string())?;
        info.set_md5_digest(&md5.finalize().into());
        info.set_total_samples(frames as usize);
        temporary
            .as_file_mut()
            .rewind()
            .map_err(|e| e.to_string())?;
        temporary
            .write_all(&stream_header(&info)?)
            .map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary
            .as_file_mut()
            .rewind()
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            check_cancelled(cancelled)?;
            let count = temporary.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let sha256 = format!("{:x}", hash.finalize());
        let size = temporary
            .as_file()
            .metadata()
            .map_err(|e| e.to_string())?
            .len();
        // Validate complete decoding, sample count, CRC and MD5 before publishing.
        let validated = VerifiedAudio::open_cancellable(
            temporary.as_file().try_clone().map_err(|e| e.to_string())?,
            size,
            &sha256,
            cancelled,
        )?;
        if validated.format.frames != frames
            || validated.format.sample_rate != rate
            || validated.format.channels != channels
        {
            return Err("FLAC保存結果が入力区間と一致しません".into());
        }
        drop(validated);
        check_cancelled(cancelled)?;
        source.check_unchanged()?;
        temporary
            .persist_noclobber(path)
            .map_err(|e| e.to_string())?;
        published = true;
        sync_parent(parent)?;
        Ok(SavedFlac {
            size,
            sha256,
            frames,
            sample_rate: rate,
            channels,
            bits_per_sample: 24,
        })
    })();
    result.map_err(|message| FlacWriteError { published, message })
}

fn stream_header(info: &StreamInfo) -> Result<Vec<u8>, String> {
    let stream = Stream::with_stream_info(info.clone());
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| e.to_string())?;
    if sink.as_slice().len() != 42 {
        return Err("FLAC保存ヘッダー長が不正です".into());
    }
    Ok(sink.as_slice().to_vec())
}

#[cfg(test)]
mod tests;
