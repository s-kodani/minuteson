//! Read-only evidence of complete PCM frames present in the active CAF files.
//! This is neither a finalized recording nor a power-loss durability receipt.
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Instant, SystemTime};

const SAFE: u64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Track {
    state: &'static str,
    frames: Option<u64>,
    age_ms: Option<u64>,
}
impl Track {
    fn unknown(state: &'static str) -> Self {
        Self {
            state,
            frames: None,
            age_ms: None,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    pub(crate) microphone: Track,
    pub(crate) system: Track,
}
#[derive(Clone, Debug, PartialEq)]
struct Format {
    description: [u8; 32],
    offset: u64,
    bytes_per_frame: u32,
    edit_count: u32,
}
// The live parser does not relax the finalized, hash-verified playback parser.
// CAF permits -1 only for the final data chunk (Apple CAF specification).
fn live_frames(file: &mut File, size: u64) -> Result<(Format, u64), String> {
    let mut head = [0; 8];
    file.read_exact(&mut head).map_err(|e| e.to_string())?;
    if &head[..6] != b"caff\0\x01" {
        return Err("CAFの形式が不正です".into());
    }
    let mut position = 8_u64;
    let mut description = None;
    for index in 0..256 {
        if position > 1024 * 1024 || size.saturating_sub(position) < 12 {
            return Err("CAFの書込み確認範囲が未確定です".into());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0; 12];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let length = i64::from_be_bytes(chunk[4..].try_into().unwrap());
        let payload = position + 12;
        if index == 0 && &chunk[..4] != b"desc" {
            return Err("CAFの音声形式がありません".into());
        }
        if &chunk[..4] == b"data" {
            let (desc, bytes_per_frame) = description.ok_or("CAFの音声形式がありません")?;
            if size.saturating_sub(payload) < 4 || length < -1 || (length != -1 && length < 4) {
                return Err("CAFの音声領域が未確定です".into());
            }
            let bytes = if length == -1 {
                size - payload
            } else {
                length as u64
            };
            if bytes > size - payload {
                return Err("CAFの音声領域が未確定です".into());
            }
            let mut edits = [0; 4];
            file.read_exact(&mut edits).map_err(|e| e.to_string())?;
            let frames = (bytes - 4) / u64::from(bytes_per_frame);
            if frames > SAFE {
                return Err("CAFのフレーム数が範囲外です".into());
            }
            return Ok((
                Format {
                    description: desc,
                    offset: payload + 4,
                    bytes_per_frame,
                    edit_count: u32::from_be_bytes(edits),
                },
                frames,
            ));
        }
        if length < 0 || length as u64 > size - payload {
            return Err("CAFの確認用ヘッダーが未確定です".into());
        }
        if &chunk[..4] == b"desc" {
            if description.is_some() || length != 32 {
                return Err("CAFの音声形式が不正です".into());
            }
            let mut desc = [0; 32];
            file.read_exact(&mut desc).map_err(|e| e.to_string())?;
            let rate = f64::from_be_bytes(desc[..8].try_into().unwrap());
            let flags = u32::from_be_bytes(desc[12..16].try_into().unwrap());
            let bytes = u32::from_be_bytes(desc[16..20].try_into().unwrap());
            let packet = u32::from_be_bytes(desc[20..24].try_into().unwrap());
            let channels = u32::from_be_bytes(desc[24..28].try_into().unwrap());
            let bits = u32::from_be_bytes(desc[28..32].try_into().unwrap());
            if &desc[8..12] != b"lpcm"
                || flags & !3 != 0
                || packet != 1
                || !rate.is_finite()
                || rate.fract() != 0.0
                || !(8000.0..=384000.0).contains(&rate)
                || !(1..=8).contains(&channels)
                || bytes == 0
                || bytes % channels != 0
                || if flags & 1 != 0 {
                    !matches!((bits, bytes / channels), (32, 4) | (64, 8))
                } else {
                    !matches!(
                        (bits, bytes / channels),
                        (16, 2 | 4) | (24, 3 | 4) | (32, 4)
                    )
                }
            {
                return Err("CAFのPCM形式を確認できません".into());
            }
            description = Some((desc, bytes));
        }
        position = payload + length as u64;
    }
    Err("CAFの確認用ヘッダーが上限を超えています".into())
}
fn milliseconds(duration: std::time::Duration) -> Result<u64, String> {
    u64::try_from(duration.as_millis().saturating_add(1))
        .ok()
        .filter(|v| *v <= SAFE)
        .ok_or_else(|| "書込み確認の時刻が範囲外です".into())
}
#[derive(Default)]
struct TrackReader {
    previous: Option<(u64, u64, SystemTime, Format, u64, SystemTime, u64, Instant)>,
    invalidated: bool,
}
impl TrackReader {
    fn read(&mut self, directory: &File, name: &str) -> Track {
        if self.invalidated {
            return Track::unknown("unavailable");
        }
        let result = (|| {
            let mut file = crate::input_levels::open_capture_file_at(directory, name)?;
            let before = file.metadata().map_err(|e| e.to_string())?;
            if !before.is_file() {
                return Err("通常のCAFファイルではありません".into());
            }
            #[cfg(unix)]
            let (device, inode) = {
                use std::os::unix::fs::MetadataExt;
                if before.nlink() != 1 {
                    return Err("CAFが共有されています".into());
                }
                (before.dev(), before.ino())
            };
            #[cfg(not(unix))]
            let (device, inode) = (0, 0);
            let modified = before.modified().map_err(|e| e.to_string())?;
            let created = before.created().map_err(|e| e.to_string())?;
            let age = milliseconds(
                SystemTime::now()
                    .duration_since(modified)
                    .map_err(|_| "書込み確認の時計が逆行しました")?,
            )?;
            if before.len() == 0 && self.previous.is_none() {
                return Ok(Track::unknown("waiting"));
            }
            let (format, frames) = live_frames(&mut file, before.len())?;
            let after = file.metadata().map_err(|e| e.to_string())?;
            if before.len() != after.len()
                || modified != after.modified().map_err(|e| e.to_string())?
            {
                return Err("CAFが確認中に変更されました".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if after.nlink() != 1
                    || after.dev() != device
                    || after.ino() != inode
                    || after.created().map_err(|e| e.to_string())? != created
                {
                    return Err("CAFの実体が確認中に変更されました".into());
                }
            }
            let mut age = age;
            if let Some((
                old_device,
                old_inode,
                old_created,
                old_format,
                old_frames,
                old_modified,
                old_age,
                read_at,
            )) = &self.previous
            {
                if (*old_device, *old_inode) != (device, inode)
                    || created != *old_created
                    || &format != old_format
                    || frames < *old_frames
                {
                    self.invalidated = true;
                    return Err("CAFの所有または音声領域が変更されました".into());
                }
                if modified < *old_modified {
                    return Err("CAFの更新時刻が逆行しました".into());
                }
                if frames == *old_frames || modified == *old_modified {
                    age = age.max(old_age.saturating_add(milliseconds(read_at.elapsed())?));
                }
            }
            self.previous = Some((
                device,
                inode,
                created,
                format,
                frames,
                modified,
                age,
                Instant::now(),
            ));
            if frames == 0 {
                return Ok(Track::unknown("waiting"));
            }
            Ok(Track {
                state: "written",
                frames: Some(frames),
                age_ms: Some(age),
            })
        })();
        result.unwrap_or_else(|_: String| Track::unknown("unavailable"))
    }
}
pub(crate) struct WriteReader {
    directory: File,
    owner: Option<String>,
    revision: u64,
    microphone: TrackReader,
    system: TrackReader,
}
impl WriteReader {
    pub(crate) fn pin(directory: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: crate::input_levels::open_capture_directory(directory)?,
            owner: None,
            revision: 0,
            microphone: Default::default(),
            system: Default::default(),
        })
    }
    pub(crate) fn read(&mut self, directory: &Path, owner: &str) -> Result<Snapshot, String> {
        if !crate::input_levels::valid_owner(owner)
            || self.owner.as_ref().is_some_and(|v| v != owner)
            || directory.canonicalize().map_err(|e| e.to_string())? != directory
            || self.revision >= SAFE
        {
            return Err("書込み確認する録音の対象が変更されました".into());
        }
        self.check_directory(directory)?;
        self.owner = Some(owner.into());
        self.revision += 1;
        let result = Snapshot {
            schema_version: 1,
            owner_id: owner.into(),
            revision: self.revision,
            microphone: self.microphone.read(&self.directory, "microphone.caf"),
            system: self.system.read(&self.directory, "system.caf"),
        };
        self.check_directory(directory)?;
        Ok(result)
    }
    fn check_directory(&self, path: &Path) -> Result<(), String> {
        let held = self.directory.metadata().map_err(|e| e.to_string())?;
        let current = crate::input_levels::open_capture_directory(path)?
            .metadata()
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if held.dev() != current.dev()
                || held.ino() != current.ino()
                || held.created().map_err(|e| e.to_string())?
                    != current.created().map_err(|e| e.to_string())?
            {
                return Err("書込み確認先の実体が変更されました".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
