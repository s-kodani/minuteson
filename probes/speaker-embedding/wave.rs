use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) fn identity(path: &Path) -> Result<Metadata, String> {
    if !path.is_absolute() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err("input must be a canonical absolute regular file".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("input is not a nonempty regular file".into());
    }
    Ok(metadata)
}

pub(super) fn same(before: &Metadata, after: &Metadata) -> Result<(), String> {
    if before.len() != after.len()
        || before.modified().map_err(|e| e.to_string())?
            != after.modified().map_err(|e| e.to_string())?
    {
        return Err("input changed during extraction".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err("input identity changed during extraction".into());
        }
    }
    Ok(())
}

pub(super) fn selected(path: &Path, first: u64, end: u64) -> Result<Vec<f32>, String> {
    if first >= end || end - first > 480_000 {
        return Err("invalid selected PCM window".into());
    }
    let before = identity(path)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    same(&before, &file.metadata().map_err(|e| e.to_string())?)?;
    let mut header = [0; 44];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    let u32_at = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    let u16_at = |offset| u16::from_le_bytes(header[offset..offset + 2].try_into().unwrap());
    let data = u64::from(u32_at(40));
    if &header[0..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || &header[36..40] != b"data"
        || u32_at(16) != 16
        || u16_at(20) != 1
        || u16_at(22) != 1
        || u32_at(24) != 16_000
        || u32_at(28) != 32_000
        || u16_at(32) != 2
        || u16_at(34) != 16
        || data == 0
        || data % 2 != 0
        || data + 44 != before.len()
        || u64::from(u32_at(4)) + 8 != before.len()
        || end > data / 2
    {
        return Err("WAV is not the exact verified 16kHz mono PCM16 run format".into());
    }
    file.seek(SeekFrom::Start(44 + first * 2))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; (end - first) as usize * 2];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    same(&before, &file.metadata().map_err(|e| e.to_string())?)?;
    same(&before, &identity(path)?)?;
    Ok(bytes
        .chunks_exact(2)
        .map(|sample| f32::from(i16::from_le_bytes(sample.try_into().unwrap())) / 32768.0)
        .collect())
}
