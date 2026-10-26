//! The canonical meeting audio: PCM16, mono, 16 kHz. Sizes and offsets are
//! 64-bit; RF64 keeps a long recording in one file without truncation.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub(crate) const RATE: u32 = 16_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub data_offset: u64,
    pub frames: u64,
}

pub(crate) fn header(frames: u64) -> Result<Vec<u8>, String> {
    let bytes = frames.checked_mul(2).ok_or("WAV容量が範囲外です")?;
    let rf64 = bytes > u64::from(u32::MAX) - 36;
    let mut out = Vec::with_capacity(if rf64 { 80 } else { 44 });
    out.extend_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    out.extend_from_slice(&(if rf64 { u32::MAX } else { bytes as u32 + 36 }).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    if rf64 {
        out.extend_from_slice(b"ds64");
        out.extend_from_slice(&28u32.to_le_bytes());
        out.extend_from_slice(
            &bytes
                .checked_add(72)
                .ok_or("RF64容量が範囲外です")?
                .to_le_bytes(),
        );
        out.extend_from_slice(&bytes.to_le_bytes());
        out.extend_from_slice(&frames.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(if rf64 { u32::MAX } else { bytes as u32 }).to_le_bytes());
    Ok(out)
}

pub(crate) fn inspect(file: &mut (impl Read + Seek), size: u64) -> Result<Layout, String> {
    let invalid = || "会議音声WAVの形式・容量が不正です".to_string();
    file.rewind().map_err(|e| e.to_string())?;
    let mut root = [0u8; 12];
    file.read_exact(&mut root).map_err(|e| e.to_string())?;
    let rf64 = &root[..4] == b"RF64";
    if (!rf64 && &root[..4] != b"RIFF") || &root[8..] != b"WAVE" {
        return Err(invalid());
    }
    let root_size = u32::from_le_bytes(root[4..8].try_into().unwrap());
    if (rf64 && root_size != u32::MAX) || (!rf64 && u64::from(root_size) + 8 != size) {
        return Err(invalid());
    }
    let mut position = 12u64;
    let mut format_seen = false;
    let mut extended = None;
    let mut data = None;
    for _ in 0..4096 {
        if position == size {
            break;
        }
        if size.saturating_sub(position) < 8 {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let length32 = u32::from_le_bytes(chunk[4..].try_into().unwrap());
        let mut length = u64::from(length32);
        let payload = position + 8;
        if &chunk[..4] == b"ds64" {
            if !rf64 || extended.is_some() || length != 28 {
                return Err(invalid());
            }
            let mut ds = [0u8; 28];
            file.read_exact(&mut ds).map_err(|e| e.to_string())?;
            let riff = u64::from_le_bytes(ds[..8].try_into().unwrap());
            let bytes = u64::from_le_bytes(ds[8..16].try_into().unwrap());
            let frames = u64::from_le_bytes(ds[16..24].try_into().unwrap());
            if riff.checked_add(8) != Some(size)
                || frames.checked_mul(2) != Some(bytes)
                || ds[24..] != [0; 4]
            {
                return Err(invalid());
            }
            extended = Some(bytes);
        } else if &chunk[..4] == b"fmt " {
            if format_seen || length != 16 {
                return Err(invalid());
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt).map_err(|e| e.to_string())?;
            if fmt[..4] != [1, 0, 1, 0]
                || u32::from_le_bytes(fmt[4..8].try_into().unwrap()) != RATE
                || u32::from_le_bytes(fmt[8..12].try_into().unwrap()) != RATE * 2
                || fmt[12..] != [2, 0, 16, 0]
            {
                return Err(invalid());
            }
            format_seen = true;
        } else if &chunk[..4] == b"data" {
            if data.is_some() || !format_seen {
                return Err(invalid());
            }
            if rf64 {
                if length32 != u32::MAX {
                    return Err(invalid());
                }
                length = extended.ok_or_else(invalid)?;
            }
            if length == 0 || length % 2 != 0 {
                return Err(invalid());
            }
            data = Some(Layout {
                data_offset: payload,
                frames: length / 2,
            });
        }
        if length > size.saturating_sub(payload) {
            return Err(invalid());
        }
        position = payload
            .checked_add(length)
            .and_then(|v| v.checked_add(length % 2))
            .ok_or_else(invalid)?;
    }
    if position != size || !format_seen || (rf64 && extended.is_none()) {
        return Err(invalid());
    }
    data.ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn riff_round_trip_and_truncation() {
        let mut f = tempfile::tempfile().unwrap();
        f.write_all(&header(160).unwrap()).unwrap();
        f.write_all(&[0; 320]).unwrap();
        assert_eq!(
            inspect(&mut f, 364).unwrap(),
            Layout {
                data_offset: 44,
                frames: 160
            }
        );
        assert!(inspect(&mut f, 362).is_err());
        assert!(header(u64::MAX).is_err());
    }
    #[test]
    fn rf64_header_uses_64_bit_sizes_without_recording_or_allocating_pcm() {
        let frames = u64::from(u32::MAX) / 2;
        let h = header(frames).unwrap();
        assert_eq!(h.len(), 80);
        let mut f = std::io::Cursor::new(h);
        assert_eq!(
            inspect(&mut f, frames * 2 + 80).unwrap(),
            Layout {
                data_offset: 80,
                frames
            }
        );
        assert!(inspect(&mut f, frames * 2 + 78).is_err());
    }
}
