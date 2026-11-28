//! MP3の音声範囲だけをデコードし、不要なタグはメタデータパーサーに渡さない。
use super::{check_cancelled, AudioInfo};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
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

const APE_BLOCK_SIZE: usize = 32;
const APE_HAS_HEADER: u32 = 1 << 31;
const APE_IS_HEADER: u32 = 1 << 29;
const APE_READ_ONLY: u32 = 1;
const APE_MAX_KEY_BYTES: u64 = 255;
const APE_MIN_ITEM_BYTES: u64 = 11;

fn read_at<const N: usize>(file: &mut File, offset: u64) -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn apev2_start(
    file: &mut File,
    first: u64,
    end: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<u64>, String> {
    let block_size = APE_BLOCK_SIZE as u64;
    let Some(footer_at) = end.checked_sub(block_size) else {
        return Ok(None);
    };
    let footer = read_at::<APE_BLOCK_SIZE>(file, footer_at)?;
    if &footer[..8] != b"APETAGEX" {
        return Ok(None);
    }
    let field = |offset| u32::from_le_bytes(footer[offset..offset + 4].try_into().unwrap());
    let version = field(8);
    if version != 2000 {
        return Err(format!(
            "MP3の末尾APEタグのバージョン {version} は未対応です"
        ));
    }
    let flags = field(20);
    if flags & !(APE_HAS_HEADER | APE_READ_ONLY) != 0 || footer[24..] != [0; 8] {
        return Err("MP3のAPEv2フッターのフラグまたは予約領域が不正です".into());
    }
    // タグ長にはフッターを含み、オプションのヘッダーは含まない。
    let length = u64::from(field(12));
    if length < block_size {
        return Err("MP3のAPEv2タグ長が不正です".into());
    }
    let has_header = flags & APE_HAS_HEADER != 0;
    let header_size = if has_header { block_size } else { 0 };
    let start = end
        .checked_sub(length + header_size)
        .filter(|&start| start >= first)
        .ok_or("MP3のAPEv2タグ長が音声範囲の先頭を越えています")?;
    if has_header {
        let mut expected = footer;
        expected[20..24].copy_from_slice(&(flags | APE_IS_HEADER).to_le_bytes());
        if read_at::<APE_BLOCK_SIZE>(file, start)? != expected {
            return Err("MP3のAPEv2ヘッダーとフッターが一致しません".into());
        }
    }
    validate_apev2_items(
        file,
        start + header_size,
        length - block_size,
        field(16),
        cancelled,
    )?;
    Ok(Some(start))
}

fn validate_apev2_items(
    file: &mut File,
    start: u64,
    mut remaining: u64,
    count: u32,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let read_error = |_| "MP3のAPEv2項目を読み取れません";
    let length_error = "MP3のAPEv2項目の長さまたは個数が不正です";
    if u64::from(count) * APE_MIN_ITEM_BYTES > remaining {
        return Err(length_error.into());
    }
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(start)).map_err(read_error)?;
    for _ in 0..count {
        check_cancelled(cancelled)?;
        if remaining < APE_MIN_ITEM_BYTES {
            return Err(length_error.into());
        }
        let mut fields = [0; 8];
        reader.read_exact(&mut fields).map_err(read_error)?;
        let value_length = u64::from(u32::from_le_bytes(fields[..4].try_into().unwrap()));
        let flags = u32::from_le_bytes(fields[4..].try_into().unwrap());
        if flags & !0b111 != 0 || flags & 0b110 == 0b110 {
            return Err("MP3のAPEv2項目のフラグが不正です".into());
        }
        let key_space = remaining
            .checked_sub(8 + value_length)
            .filter(|&space| space >= 3)
            .ok_or(length_error)?;
        let mut key_length = 0;
        loop {
            if key_length == key_space.min(APE_MAX_KEY_BYTES + 1) {
                return Err("MP3のAPEv2項目のキーが不正です".into());
            }
            let mut byte = [0];
            reader.read_exact(&mut byte).map_err(read_error)?;
            if byte[0] == 0 && key_length >= 2 {
                break;
            }
            if !(0x20..=0x7e).contains(&byte[0]) {
                return Err("MP3のAPEv2項目のキーが不正です".into());
            }
            key_length += 1;
        }
        // 項目の値は解釈せず、検証した長さだけ進める。
        reader
            .seek_relative(value_length as i64)
            .map_err(read_error)?;
        remaining -= 8 + key_length + 1 + value_length;
    }
    if remaining != 0 {
        return Err(length_error.into());
    }
    Ok(())
}

fn audio_range(file: &mut File, cancelled: &impl Fn() -> bool) -> Result<(u64, u64), String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut first = 0;
    for _ in 0..16 {
        let h = read_at::<10>(file, first)?;
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
    let id3_start = size.checked_sub(128);
    let id3_start = match id3_start {
        Some(start) if &read_at::<3>(file, start)? == b"TAG" => Some(start),
        _ => None,
    };
    let ape_at_eof = apev2_start(file, first, size, cancelled);
    let end = match id3_start {
        None => ape_at_eof?.unwrap_or(size),
        Some(id3) => match ape_at_eof {
            // APEv2の値に偶然TAGがあっても、末尾全体を検証できる構成を使う。
            Ok(Some(ape)) if ape <= id3 => ape,
            Err(error) if super::is_cancellation_error(&error) => return Err(error),
            // ID3v1の値の中に現れたAPETAGEXは、APEタグの開始位置にしない。
            _ => apev2_start(file, first, id3, cancelled)?.unwrap_or(id3),
        },
    };
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
        let remaining = length - offset;
        if remaining < 4 {
            return Err("MP3がフレームの途中で終了しています".into());
        }
        let mut h = [0; 4];
        reader
            .read_exact(&mut h)
            .map_err(|_| "MP3がフレームの途中で終了しています")?;
        if h == *b"LYRI" && remaining >= 11 {
            let mut suffix = [0; 7];
            reader.read_exact(&mut suffix).map_err(|e| e.to_string())?;
            if &suffix == b"CSBEGIN" {
                return Err("MP3の末尾タグ Lyrics3 は未対応です".into());
            }
        }
        if offset > 0 && (h[0] != 255 || h[1] & 0xe0 != 0xe0) {
            return Err("MP3のフレーム間または末尾に未対応のデータがあります".into());
        }
        let (rate, channels, frames, bytes, tag_offset) = frame_header(h)?;
        if format.is_some_and(|f| f != (rate, channels)) || bytes as u64 > remaining {
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
    let (first, length) = audio_range(&mut file, cancelled)?;
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
        .map(|path| super::VoiceWavWriter::new(path, rate, channels))
        .transpose()?;
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
            let mut pcm = Vec::with_capacity(last - first);
            for value in &buffer.samples()[first..last] {
                if !value.is_finite() {
                    return Err("MP3に非有限のPCM値があります".into());
                }
                if output.is_some() {
                    pcm.push(*value);
                }
            }
            if let Some(file) = &mut output {
                file.push(&pcm)?;
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
    if let Some(file) = output {
        file.finish()?;
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
#[cfg(test)]
mod tests {
    use super::*;

    fn mp3_frames() -> Vec<u8> {
        // MPEG-1 Layer III、44.1 kHz、128 kbps の無音フレーム。
        let mut frame = [0; 417];
        frame[..4].copy_from_slice(&[255, 251, 144, 0]);
        frame.repeat(3)
    }

    fn apev2_tag(with_header: bool, value: &[u8]) -> Vec<u8> {
        let mut item = Vec::new();
        item.extend((value.len() as u32).to_le_bytes());
        item.extend(0_u32.to_le_bytes());
        item.extend_from_slice(b"Title\0");
        item.extend_from_slice(value);
        let block = |flags: u32| {
            let mut block = [0; 32];
            block[..8].copy_from_slice(b"APETAGEX");
            block[8..12].copy_from_slice(&2000_u32.to_le_bytes());
            block[12..16].copy_from_slice(&(32 + item.len() as u32).to_le_bytes());
            block[16..20].copy_from_slice(&1_u32.to_le_bytes());
            block[20..24].copy_from_slice(&flags.to_le_bytes());
            block
        };
        let mut tag = Vec::new();
        if with_header {
            tag.extend(block(0xa000_0000));
        }
        tag.extend_from_slice(&item);
        tag.extend(block(if with_header { 0x8000_0000 } else { 0 }));
        tag
    }

    fn id3v1_tag() -> [u8; 128] {
        let mut tag = [0; 128];
        tag[..3].copy_from_slice(b"TAG");
        tag
    }

    #[test]
    fn mp3_trailing_tags_preserve_inspection_pcm_and_source() {
        use crate::audio_import::{import, inspect};
        use crate::local_engine::Fingerprint;
        use crate::storage::Storage;

        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().join("meetings")).unwrap();
        let frames = mp3_frames();
        let plain = root.path().join("plain.mp3");
        let cancelled = &crate::test_support::never_cancelled;
        std::fs::write(&plain, &frames).unwrap();
        // MP3は共通デコーダーを使うため、この検証用のファイルは起動されない。
        let helper = Fingerprint::capture(&plain, cancelled).unwrap();
        let inspect_and_import = |input: &Path, name| -> Result<(AudioInfo, Vec<u8>), String> {
            let selected = inspect(input, &helper, root.path(), cancelled)?;
            let info = selected.info.clone();
            let imported = import(
                &storage,
                selected,
                &helper,
                name,
                &uuid::Uuid::new_v4().to_string(),
                cancelled,
                &|_, _| {},
            )?;
            assert_eq!(imported.state, "ready");
            let wav = storage
                .audio_import_directory(&imported)?
                .join("recording.wav");
            Ok((info, std::fs::read(wav).unwrap()))
        };
        let (expected, expected_wav) = inspect_and_import(&plain, "plain").unwrap();
        assert_eq!(expected.frames, 3 * 1152);

        let id3v1 = id3v1_tag();
        let mut id3_with_magic = id3v1;
        id3_with_magic[96..104].copy_from_slice(b"APETAGEX");
        let mut value_with_magic = [b'a'; 96];
        value_with_magic[..3].copy_from_slice(b"TAG");
        for (name, tags) in [
            ("id3v1", id3v1.to_vec()),
            ("id3v1-with-ape-magic", id3_with_magic.to_vec()),
            ("apev2", apev2_tag(false, b"test")),
            (
                "apev2-id3v1",
                [apev2_tag(false, b"test"), id3v1.to_vec()].concat(),
            ),
            ("apev2-header", apev2_tag(true, b"test")),
            (
                "apev2-header-id3v1",
                [apev2_tag(true, b"test"), id3v1.to_vec()].concat(),
            ),
            ("apev2-with-id3-magic", apev2_tag(false, &value_with_magic)),
        ] {
            let bytes = [frames.as_slice(), &tags].concat();
            let input = root.path().join(format!("{name}.mp3"));
            std::fs::write(&input, &bytes).unwrap();
            let (info, wav) =
                inspect_and_import(&input, name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(info, expected, "{name}");
            assert_eq!(wav, expected_wav, "{name}");
            assert_eq!(std::fs::read(input).unwrap(), bytes, "{name}");
        }
        assert_eq!(std::fs::read(plain).unwrap(), frames);
    }

    #[test]
    fn mp3_rejects_invalid_apev2_footer_header_and_item_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input.mp3");
        let id3v2 = b"ID3\x04\0\0\0\0\0\0";
        let frames = mp3_frames();
        let tag = apev2_tag(false, b"test");
        let footer = tag.len() - 32;
        for (offset, value, error) in [
            (footer + 8, 1000, "バージョン"),
            (footer + 12, 31, "タグ長が不正"),
            (
                footer + 12,
                (frames.len() + tag.len() + 1) as u32,
                "先頭を越え",
            ),
            (footer + 12, u32::MAX, "先頭を越え"),
            (footer + 16, 0, "項目の長さまたは個数"),
            (footer + 20, 0x4000_0000, "フッター"),
            (footer + 20, 0x8000_0000, "ヘッダーとフッター"),
            (footer + 24, 1, "予約領域"),
            (0, u32::MAX, "項目の長さ"),
        ] {
            let mut invalid = tag.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            let bytes = [id3v2.as_slice(), &frames, &invalid, &id3v1_tag()].concat();
            std::fs::write(&path, &bytes).unwrap();
            let actual = decode(&path, None, &crate::test_support::never_cancelled).unwrap_err();
            assert!(actual.contains(error), "{offset}: {actual}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn mp3_rejects_truncated_audio_and_unsupported_trailers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input.mp3");
        let frames = mp3_frames();
        let ape = apev2_tag(true, b"test");
        let id3 = id3v1_tag();
        let lyrics = b"LYRICSBEGININD0000200000021LYRICS200";
        for (audio, suffix, error) in [
            (&frames[..frames.len() - 1], ape.clone(), "音声が切れ"),
            (&frames[..420], ape.clone(), "フレームの途中"),
            (&frames[..], b"junk".to_vec(), "未対応のデータ"),
            (
                &frames[..],
                [b"junk".as_slice(), &ape].concat(),
                "未対応のデータ",
            ),
            (
                &frames[..],
                [id3.as_slice(), &ape].concat(),
                "未対応のデータ",
            ),
            (&frames[..], [lyrics.as_slice(), &id3].concat(), "Lyrics3"),
        ] {
            let bytes = [audio, &suffix].concat();
            std::fs::write(&path, &bytes).unwrap();
            let actual = decode(&path, None, &crate::test_support::never_cancelled).unwrap_err();
            assert!(actual.contains(error), "{actual}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

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
        assert!(crate::audio::wave::streaming_header(u64::MAX).is_err());
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
            let wav = root.path().join(format!("{rate}.wav"));
            assert_eq!(
                decode(&mp3, Some(&wav), &crate::test_support::never_cancelled).unwrap(),
                inspected
            );
            assert!(decode(&mp3, Some(&wav), &crate::test_support::never_cancelled).is_err());
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
            let proof = Fingerprint::capture(&wav, &crate::test_support::never_cancelled).unwrap();
            let mut audio =
                VerifiedAudio::open(File::open(&wav).unwrap(), proof.size, &proof.sha256).unwrap();
            audio.require_wav().unwrap();
            let mut converter =
                mon_core::media::voice::VoiceConverter::new(rate, channels).unwrap();
            let mut compact = Vec::new();
            let mut sink = |pcm: &[i16], _, _| {
                compact.extend_from_slice(pcm);
                Ok(())
            };
            let mut first = 0;
            for bytes in expected.chunks(4096 * channels as usize * 4) {
                let samples: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                let last = first + samples.len() as u64 / u64::from(channels);
                converter
                    .push(
                        &samples,
                        (first * 1000000000 / u64::from(rate)) as i64,
                        (last * 1000000000 / u64::from(rate)) as i64,
                        &mut sink,
                    )
                    .unwrap();
                first = last;
            }
            converter.finish(&mut sink).unwrap();
            assert_eq!(audio.format.frames as usize, compact.len());
            assert_eq!(
                (audio.format.sample_rate, audio.format.channels),
                (16000, 1)
            );
            assert_eq!(proof.size, 80 + audio.format.frames * 2);
            let mut offset = 0;
            while offset < audio.format.frames {
                let n = (audio.format.frames - offset).min(4096);
                let samples = audio.read_frames(offset, n).unwrap();
                for (i, sample) in samples.chunks_exact(4).enumerate() {
                    let actual = f32::from_le_bytes(sample.try_into().unwrap());
                    let expected = f32::from(compact[offset as usize + i]) / 32768.0;
                    assert!(
                        (actual - expected).abs() < 0.001,
                        "{rate}: {actual} != {expected}"
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
