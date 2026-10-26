//! Read only the native producer's durably advertised raw prefix. A prefix is
//! recovery evidence, never acquisition closure, final FLAC or a DB receipt.
//! Keep descriptors bounded independently of a long recording's segment count.
use crate::audio::VerifiedAudio;
use crate::audio_clock::{ClockChunk, ClockLayout, ClockSegment};
use crate::native_fs::Metadata;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MAX_SEGMENTS: u32 = 32_768;
const MAX_JSON: u64 = 65 * 1024;
const MAX_TOTAL_JSON: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_MAPS: usize = 65_536;
const SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Microphone,
    System,
}
impl Lane {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::System => "system",
        }
    }
    fn tip_name(self) -> String {
        format!("raw-{}.json", self.name())
    }
    fn name_at(self, sequence: u32, extension: &str) -> String {
        format!("raw-{}-{sequence:08}.{extension}", self.name())
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Status {
    schema_version: u32,
    owner_id: String,
    lane: String,
    sealed_segments: u32,
    sealed_frames: u64,
    #[serde(rename = "lastEntrySHA256")]
    last_entry_sha256: Option<String>,
    producer_closed: bool,
    failure: Option<String>,
}
impl Status {
    /// A bounded decoding envelope, never proof of PCM, closure or consent.
    pub(crate) fn capacity_owner(&self, owner: &str, lane: Lane) -> bool {
        self.schema_version == 2
            && self.owner_id == owner
            && self.lane == lane.name()
            && self.sealed_segments <= MAX_SEGMENTS
            && self.sealed_frames <= SAFE
            && if self.sealed_segments == 0 {
                self.sealed_frames == 0 && self.last_entry_sha256.is_none()
            } else {
                self.sealed_frames > 0 && self.last_entry_sha256.as_deref().is_some_and(valid_hash)
            }
    }
    pub(crate) fn agrees_with_completed(&self, prefix: &Prefix, frames: u64) -> bool {
        frames > 0 && self.agrees_with_closed(prefix, frames)
    }
    pub(crate) fn agrees_with_unreceived(&self, prefix: &Prefix) -> bool {
        self.agrees_with_closed(prefix, 0)
    }
    fn agrees_with_closed(&self, prefix: &Prefix, frames: u64) -> bool {
        self.schema_version == 2
            && self.owner_id == prefix.tip.owner_id
            && self.lane == prefix.tip.lane
            && self.sealed_segments == prefix.tip.sealed_segments
            && self.sealed_frames == prefix.tip.sealed_frames
            && self.sealed_frames == frames
            && self.last_entry_sha256 == prefix.tip.last_entry_sha256
            && self.producer_closed
            && prefix.tip.producer_closed
            && self.failure.is_none()
    }
}

#[derive(Debug, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Tip {
    schema_version: u32,
    owner_id: String,
    lane: String,
    sealed_segments: u32,
    sealed_frames: u64,
    #[serde(rename = "lastEntrySHA256")]
    last_entry_sha256: Option<String>,
    producer_closed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Entry {
    schema_version: u32,
    owner_id: String,
    lane: String,
    pub(crate) sequence: u32,
    audio_name: String,
    pub(crate) source_frame_start: u64,
    pub(crate) frames: u64,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u32,
    pub(crate) capture_origin_nanoseconds: i64,
    pub(crate) clock_segments: Vec<ClockSegment>,
    clock_nanoseconds: Vec<Endpoints>,
    audio_bytes: u64,
    #[serde(rename = "audioSHA256")]
    audio_sha256: String,
    #[serde(rename = "previousEntrySHA256")]
    previous_entry_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Endpoints {
    start_nanoseconds: i64,
    end_nanoseconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    links: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    created: Option<std::time::SystemTime>,
}
impl Stamp {
    fn of(file: &File) -> Result<Self, String> {
        let m = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.uid() != crate::native_fs::current_owner()?
            || m.nlink() != 1
            || m.mode() & 0o077 != 0
        {
            return Err("原音声セグメントの実体・所有・権限が不正です".into());
        }
        Ok(Self::metadata(&m))
    }
    fn metadata(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
            links: m.nlink(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            created: m.created().ok(),
        }
    }
    fn require(&self, file: &File) -> Result<(), String> {
        let actual = Self::of(file)?;
        if *self != actual {
            crate::internal_logs::detail(crate::internal_logs::current_context("recording"), "source_changed", "error", std::time::Duration::ZERO,
                &serde_json::json!({"identityChanged":self.dev!=actual.dev||self.ino!=actual.ino||self.created!=actual.created,
                    "sizeBefore":self.size,"sizeAfter":actual.size,"modifiedBefore":self.modified,"modifiedAfter":actual.modified,
                    "changedBefore":self.changed,"changedAfter":actual.changed}).to_string());
            return Err("原音声セグメントが検証後に変更されました".into());
        }
        Ok(())
    }
}

struct BoundEntry {
    entry: Entry,
    json_stamp: Stamp,
    audio_stamp: Stamp,
}
pub(crate) struct Prefix {
    directory: File,
    tip: Tip,
    tip_stamp: Stamp,
    entries: Vec<BoundEntry>,
}

pub(crate) struct FlacGroup {
    pub(crate) first_entry: usize,
    pub(crate) entries: usize,
    pub(crate) frames: u64,
    pub(crate) clock_segments: Vec<ClockSegment>,
}

/// A bounded physical PCM view over a selected raw range. Its timeline remains
/// in the entries; crossing a file boundary must not invent zero-filled PCM.
pub(crate) struct Source<'a, C: Fn() -> bool> {
    prefix: &'a Prefix,
    check: &'a C,
    first_index: usize,
    end_index: usize,
    first_frame: u64,
    format: crate::audio::PcmFormat,
    current: Option<(usize, VerifiedAudio)>,
}
impl<C: Fn() -> bool> crate::audio::flac::PcmSource for Source<'_, C> {
    fn format(&self) -> &crate::audio::PcmFormat {
        &self.format
    }
    fn read_frames(&mut self, first: u64, frames: u64) -> Result<Vec<u8>, String> {
        if frames == 0
            || frames > crate::audio::PREVIEW_FRAMES
            || first
                .checked_add(frames)
                .is_none_or(|end| end > self.format.frames)
        {
            return Err("原音声セグメントの読出し範囲が不正です".into());
        }
        let mut output = Vec::with_capacity(frames as usize * self.format.channels as usize * 4);
        let mut from = self.first_frame + first;
        let end = from + frames;
        while from < end {
            cancelled(self.check)?;
            let index = self.prefix.entries[self.first_index..self.end_index]
                .partition_point(|bound| bound.entry.source_frame_start <= from)
                .checked_sub(1)
                .ok_or("原音声セグメントの対応先がありません")?
                + self.first_index;
            let entry = &self.prefix.entries[index].entry;
            let local = from - entry.source_frame_start;
            let count = (entry.frames - local).min(end - from);
            if self
                .current
                .as_ref()
                .is_none_or(|(current, _)| *current != index)
            {
                self.current = None;
                self.current = Some((index, self.prefix.open_audio(index, self.check)?));
            }
            self.prefix.verify_entry(index)?;
            output.extend_from_slice(
                &self
                    .current
                    .as_mut()
                    .unwrap()
                    .1
                    .read_raw_recording_frames(local, count)?,
            );
            self.prefix.verify_entry(index)?;
            from += count;
        }
        Ok(output)
    }
    fn verify_unchanged(&self) -> Result<(), String> {
        cancelled(self.check)?;
        for index in self.first_index..self.end_index {
            self.prefix.verify_entry(index)?;
        }
        Ok(())
    }
}

fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("原音声セグメントの検証を中断しました".into())
    } else {
        Ok(())
    }
}
fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn open_at(directory: &File, name: &str) -> Result<Option<File>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if name.is_empty() || name.contains(['/', '\\', ':', '\0']) || matches!(name, "." | "..") {
            return Err("原PCMファイル名が不正です".into());
        }
        let path = crate::windows_files::path(directory)?.join(name);
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(&path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if crate::windows_files::identity(&file)?.2 != 1
            || !file.metadata().map_err(|e| e.to_string())?.is_file()
        {
            return Err("原PCMの実体・リンク数が不正です".into());
        }
        return Ok(Some(file));
    }
    #[cfg(unix)]
    {
        // Names come exclusively from Lane plus a bounded integer. A JSON-supplied
        // audioName is compared with this canonical name, never passed to openat.
        let name = std::ffi::CString::new(name).map_err(|_| "原音声のファイル名が不正です")?;
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error.to_string());
        }
        Ok(Some(unsafe { File::from_raw_fd(descriptor) }))
    }
}
fn required(directory: &File, name: &str) -> Result<File, String> {
    open_at(directory, name)?.ok_or("原音声の確定範囲に不足するファイルがあります".into())
}
fn read_json(
    directory: &File,
    name: &str,
    mut file: File,
    check: &impl Fn() -> bool,
) -> Result<(Vec<u8>, Stamp), String> {
    cancelled(check)?;
    let stamp = Stamp::of(&file)?;
    if stamp.size == 0 || stamp.size > MAX_JSON {
        return Err("原音声セグメントのJSONが容量上限外です".into());
    }
    let mut data = Vec::with_capacity(stamp.size as usize);
    (&mut file)
        .take(MAX_JSON + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 != stamp.size {
        return Err("原音声セグメントのJSONが読込み中に変わりました".into());
    }
    stamp.require(&file)?;
    stamp.require(&required(directory, name)?)?;
    cancelled(check)?;
    Ok((data, stamp))
}

impl Entry {
    fn validate(
        &self,
        owner: &str,
        lane: Lane,
        sequence: u32,
        frames: u64,
        previous_hash: Option<&str>,
    ) -> Result<(), String> {
        if self.schema_version != 2
            || self.owner_id != owner
            || self.lane != lane.name()
            || self.sequence != sequence
            || self.audio_name != lane.name_at(sequence, "caf")
            || self.source_frame_start != frames
            || self.frames == 0
            || self.frames > u64::from(self.sample_rate) * 17 / 4
            || !(8_000..=384_000).contains(&self.sample_rate)
            || !(1..=8).contains(&self.channels)
            || self.capture_origin_nanoseconds < 0
            || self.previous_entry_sha256.as_deref() != previous_hash
            || !valid_hash(&self.audio_sha256)
            || self.audio_bytes != 68 + self.frames * u64::from(self.channels) * 4
            || self.clock_segments.is_empty()
            || self.clock_segments.len() > 1024
            || self.clock_nanoseconds.len() != self.clock_segments.len()
        {
            return Err("原音声セグメントの所属・連番・形式・確定証拠が不正です".into());
        }
        let end = self.clock_segments.last().unwrap().timeline_end_us;
        if self.clock_segments[0].timeline_start_us != 0 || end > SAFE as i64 {
            return Err("原音声セグメントの取得時計の原点または範囲が不正です".into());
        }
        ClockLayout::new(
            &[ClockChunk {
                frames: self.frames as i64,
                start_us: 0,
                end_us: end,
                segments: &self.clock_segments,
            }],
            self.sample_rate,
        )?;
        let mut previous_end = None;
        for (map, clock) in self.clock_segments.iter().zip(&self.clock_nanoseconds) {
            if clock.start_nanoseconds < self.capture_origin_nanoseconds
                || clock.end_nanoseconds <= clock.start_nanoseconds
                || previous_end.is_some_and(|end| end > clock.start_nanoseconds)
                || (clock.start_nanoseconds - self.capture_origin_nanoseconds) / 1000
                    != map.timeline_start_us
                || (clock.end_nanoseconds - self.capture_origin_nanoseconds) / 1000
                    != map.timeline_end_us
            {
                return Err("原音声の測定境界と時刻対応が一致しません".into());
            }
            previous_end = Some(clock.end_nanoseconds);
        }
        self.capture_origin_nanoseconds
            .checked_add(end.checked_mul(1000).ok_or("原音声の時刻が範囲外です")?)
            .ok_or("原音声の時刻が範囲外です")?;
        Ok(())
    }
}

impl Prefix {
    pub(crate) fn lane(&self) -> Lane {
        if self.tip.lane == "microphone" {
            Lane::Microphone
        } else {
            Lane::System
        }
    }
    /// Bind a review to exact durable generations, not just the last entry's
    /// content hash. Replacing identical bytes or closing the producer still
    /// invalidates a previously reviewed generation.
    pub(crate) fn review_hash(&self) -> Result<String, String> {
        fn proof(stamp: &Stamp) -> serde_json::Value {
            serde_json::json!([
                stamp.dev,
                stamp.ino,
                stamp.uid,
                stamp.mode,
                stamp.links,
                stamp.size,
                stamp.modified,
                stamp.changed,
                stamp
                    .created
                    .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|v| (v.as_secs(), v.subsec_nanos()))
            ])
        }
        let directory = crate::native_fs::metadata(&self.directory).map_err(|e| e.to_string())?;
        let entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| serde_json::json!([proof(&entry.json_stamp), proof(&entry.audio_stamp)]))
            .collect();
        Ok(hash(
            &serde_json::to_vec(&serde_json::json!({
                "version":1, "directory":[directory.dev(),directory.ino(),directory.uid()],
                "tip":self.tip, "tipGeneration":proof(&self.tip_stamp), "entries":entries
            }))
            .map_err(|e| e.to_string())?,
        ))
    }
    /// Content evidence only, never a review/consent capability. The complete
    /// validated tip binds closure, counts and the entry/audio hash chain, but
    /// deliberately does not bind transport-dependent inode generations.
    pub(crate) fn content_hash(&self) -> Result<String, String> {
        Ok(hash(
            &serde_json::to_vec(&serde_json::json!({"version":1,"tip":self.tip}))
                .map_err(|e| e.to_string())?,
        ))
    }
    pub(crate) fn first_capture_nanoseconds(&self) -> Result<i64, String> {
        self.entries
            .first()
            .map(|bound| bound.entry.capture_origin_nanoseconds)
            .ok_or("原音声の保存範囲が空です".into())
    }
    pub(crate) fn tip_hash(&self) -> &str {
        self.tip.last_entry_sha256.as_deref().unwrap_or("")
    }
    pub(crate) fn verify_at(
        &self,
        path: &std::path::Path,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let current = crate::input_levels::open_capture_directory(path)?;
        let actual = crate::native_fs::metadata(&current).map_err(|e| e.to_string())?;
        let pinned = crate::native_fs::metadata(&self.directory).map_err(|e| e.to_string())?;
        if !actual.is_dir()
            || actual.dev() != pinned.dev()
            || actual.ino() != pinned.ino()
            || actual.uid() != pinned.uid()
        {
            return Err("原音声セグメントの管理先が検証後に変更されました".into());
        }
        self.verify_unchanged(check)
    }
    /// Group physical PCM into at most one minute per FLAC, preserving every
    /// original affine map. Raw segment count is not the eventual DB chunk count.
    /// Each group still satisfies the existing per-chunk map admission limit.
    pub(crate) fn flac_groups(&self, origin_nanoseconds: i64) -> Result<Vec<FlacGroup>, String> {
        let mut groups = Vec::<FlacGroup>::new();
        for (index, bound) in self.entries.iter().enumerate() {
            let entry = &bound.entry;
            if origin_nanoseconds < 0 || origin_nanoseconds > entry.capture_origin_nanoseconds {
                return Err("原音声の共通時計原点が不正です".into());
            }
            let start_new = groups.last().is_none_or(|group| {
                group.frames + entry.frames > u64::from(entry.sample_rate) * 60
                    || group.clock_segments.len() + entry.clock_segments.len() > 1024
            });
            if start_new {
                groups.push(FlacGroup {
                    first_entry: index,
                    entries: 0,
                    frames: 0,
                    clock_segments: Vec::new(),
                });
            }
            let group = groups.last_mut().unwrap();
            for (segment, clock) in entry.clock_segments.iter().zip(&entry.clock_nanoseconds) {
                let start = (clock.start_nanoseconds - origin_nanoseconds) / 1000;
                let end = (clock.end_nanoseconds - origin_nanoseconds) / 1000;
                if end > SAFE as i64 {
                    return Err("原音声の共通時刻が範囲外です".into());
                }
                group.clock_segments.push(ClockSegment {
                    source_frame_start: group.frames as i64 + segment.source_frame_start,
                    source_frame_end: group.frames as i64 + segment.source_frame_end,
                    timeline_start_us: start,
                    timeline_end_us: end,
                    mapping_json: segment.mapping_json.clone(),
                });
            }
            group.frames += entry.frames;
            group.entries += 1;
        }
        for group in &groups {
            ClockLayout::new(
                &[ClockChunk {
                    frames: group.frames as i64,
                    start_us: group.clock_segments[0].timeline_start_us,
                    end_us: group.clock_segments.last().unwrap().timeline_end_us,
                    segments: &group.clock_segments,
                }],
                self.entries[group.first_entry].entry.sample_rate,
            )?;
        }
        Ok(groups)
    }
    pub(crate) fn source<'a, C: Fn() -> bool>(
        &'a self,
        first: usize,
        count: usize,
        check: &'a C,
    ) -> Result<Source<'a, C>, String> {
        self.verify_unchanged(check)?;
        self.source_range(first, count, check)
    }
    // The publication coordinator verifies the whole prefix before/after a
    // track and again at DB publication. Constructing each group's view must
    // not scan every other group (quadratic work for long recordings).
    pub(crate) fn source_group<'a, C: Fn() -> bool>(
        &'a self,
        group: &FlacGroup,
        check: &'a C,
    ) -> Result<Source<'a, C>, String> {
        self.source_range(group.first_entry, group.entries, check)
    }
    fn source_range<'a, C: Fn() -> bool>(
        &'a self,
        first: usize,
        count: usize,
        check: &'a C,
    ) -> Result<Source<'a, C>, String> {
        let end = first
            .checked_add(count)
            .filter(|end| count > 0 && *end <= self.entries.len())
            .ok_or("原音声セグメントの連結範囲が不正です")?;
        cancelled(check)?;
        for index in first..end {
            cancelled(check)?;
            self.verify_entry(index)?;
        }
        let audio = self.open_audio(first, check)?;
        let mut format = audio.format.clone();
        let first_frame = self.entries[first].entry.source_frame_start;
        let last = &self.entries[end - 1].entry;
        format.frames = last.source_frame_start + last.frames - first_frame;
        Ok(Source {
            prefix: self,
            check,
            first_index: first,
            end_index: end,
            first_frame,
            format,
            current: Some((first, audio)),
        })
    }
    fn verify_entry(&self, index: usize) -> Result<(), String> {
        let bound = &self.entries[index];
        let lane = if self.tip.lane == "microphone" {
            Lane::Microphone
        } else {
            Lane::System
        };
        bound.json_stamp.require(&required(
            &self.directory,
            &lane.name_at(bound.entry.sequence, "json"),
        )?)?;
        bound
            .audio_stamp
            .require(&required(&self.directory, &bound.entry.audio_name)?)
    }
    /// The caller supplies the exact DB-owned, pinned native recording directory
    /// and canonical recording UUID. Absence alone returns None; invalid or
    /// incomplete advertised evidence must never fall back to guessed audio.
    pub(crate) fn read_optional(
        directory: File,
        owner: &str,
        lane: Lane,
        check: &impl Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        if !crate::input_levels::valid_owner(owner) {
            return Err("原音声セグメントの録音IDが不正です".into());
        }
        let m = crate::native_fs::metadata(&directory).map_err(|e| e.to_string())?;
        if !m.is_dir() || m.uid() != crate::native_fs::current_owner()? {
            return Err("原音声セグメントの管理先が不正です".into());
        }
        cancelled(check)?;
        let name = lane.tip_name();
        let Some(file) = open_at(&directory, &name)? else {
            return Ok(None);
        };
        let (bytes, tip_stamp) = read_json(&directory, &name, file, check)?;
        let tip: Tip = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if tip.schema_version != 2
            || tip.owner_id != owner
            || tip.lane != lane.name()
            || tip.sealed_segments > MAX_SEGMENTS
            || tip.sealed_frames > SAFE
            || (tip.sealed_segments == 0
                && (tip.sealed_frames != 0 || tip.last_entry_sha256.is_some()))
            || (tip.sealed_segments > 0
                && (tip.sealed_frames == 0
                    || !tip.last_entry_sha256.as_deref().is_some_and(valid_hash)))
        {
            return Err("原音声の確定範囲・所属・ハッシュが不正です".into());
        }
        let mut entries = Vec::with_capacity(tip.sealed_segments as usize);
        let mut previous_hash = None::<String>;
        let mut frames = 0_u64;
        let mut json_bytes = 0_u64;
        let mut maps = 0_usize;
        let mut previous_end_ns = None;
        let mut format = None;
        for sequence in 1..=tip.sealed_segments {
            cancelled(check)?;
            let json_name = lane.name_at(sequence, "json");
            let (bytes, json_stamp) = read_json(
                &directory,
                &json_name,
                required(&directory, &json_name)?,
                check,
            )?;
            json_bytes = json_bytes
                .checked_add(bytes.len() as u64)
                .filter(|v| *v <= MAX_TOTAL_JSON)
                .ok_or("原音声セグメントの総JSON容量が上限を超えています")?;
            let entry: Entry = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            entry.validate(owner, lane, sequence, frames, previous_hash.as_deref())?;
            // The native writer may seal before four physical seconds when
            // its 128-map entry budget reserves two pending/final boundary
            // slots. Only that bounded density range is an early-seal reason;
            // all PCM, measured endpoints, ownership and chain checks remain.
            if sequence < tip.sealed_segments
                && entry.frames < u64::from(entry.sample_rate) * 4
                && !(126..=128).contains(&entry.clock_segments.len())
            {
                return Err("原音声の途中のセグメントが確定条件を満たしていません".into());
            }
            if format.is_some_and(|f| f != (entry.sample_rate, entry.channels)) {
                return Err("原音声セグメントに未対応の入力形式変更があります".into());
            }
            format = Some((entry.sample_rate, entry.channels));
            if previous_end_ns.is_some_and(|end| end > entry.capture_origin_nanoseconds) {
                return Err("原音声セグメントの取得時刻が重複または逆行しています".into());
            }
            previous_end_ns = Some(entry.clock_nanoseconds.last().unwrap().end_nanoseconds);
            maps = maps
                .checked_add(entry.clock_segments.len())
                .filter(|v| *v <= MAX_TOTAL_MAPS)
                .ok_or("原音声セグメントの総時刻対応数が上限を超えています")?;
            let file = required(&directory, &entry.audio_name)?;
            let audio_stamp = Stamp::of(&file)?;
            if audio_stamp.size != entry.audio_bytes {
                return Err("原音声セグメントの物理サイズが確定記録と一致しません".into());
            }
            let (pcm, size, sha256) = crate::audio::inspect_recorded_caf(
                file.try_clone().map_err(|e| e.to_string())?,
                check,
            )?;
            if size != entry.audio_bytes
                || sha256 != entry.audio_sha256
                || pcm.frames != entry.frames
                || pcm.sample_rate != entry.sample_rate
                || pcm.channels != entry.channels
                || !pcm.is_raw_recording_segment()
            {
                return Err("原音声セグメントの実音声が確定記録と一致しません".into());
            }
            audio_stamp.require(&file)?;
            audio_stamp.require(&required(&directory, &entry.audio_name)?)?;
            frames = frames
                .checked_add(entry.frames)
                .filter(|v| *v <= SAFE)
                .ok_or("原音声セグメントの総フレーム数が範囲外です")?;
            previous_hash = Some(hash(&bytes));
            entries.push(BoundEntry {
                entry,
                json_stamp,
                audio_stamp,
            });
        }
        if frames != tip.sealed_frames || previous_hash != tip.last_entry_sha256 {
            return Err("原音声の確定範囲と実際のセグメント列が一致しません".into());
        }
        let prefix = Self {
            directory,
            tip,
            tip_stamp,
            entries,
        };
        prefix.verify_unchanged(check)?;
        Ok(Some(prefix))
    }
    pub(crate) fn entries(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.entries.iter().map(|e| &e.entry)
    }
    pub(crate) fn frames(&self) -> u64 {
        self.tip.sealed_frames
    }
    pub(crate) fn verify_unchanged(&self, check: &impl Fn() -> bool) -> Result<(), String> {
        cancelled(check)?;
        let lane = if self.tip.lane == "microphone" {
            Lane::Microphone
        } else {
            Lane::System
        };
        self.tip_stamp
            .require(&required(&self.directory, &lane.tip_name())?)?;
        for bound in &self.entries {
            cancelled(check)?;
            bound.json_stamp.require(&required(
                &self.directory,
                &lane.name_at(bound.entry.sequence, "json"),
            )?)?;
            bound
                .audio_stamp
                .require(&required(&self.directory, &bound.entry.audio_name)?)?;
        }
        self.tip_stamp
            .require(&required(&self.directory, &lane.tip_name())?)?;
        Ok(())
    }
    /// Reopen only the requested segment. Do not keep thousands of descriptor
    /// pairs alive, and do not trust a replaced path or just a cached hash.
    pub(crate) fn open_audio(
        &self,
        index: usize,
        check: &impl Fn() -> bool,
    ) -> Result<VerifiedAudio, String> {
        let bound = self
            .entries
            .get(index)
            .ok_or("原音声セグメントの番号が範囲外です")?;
        let file = required(&self.directory, &bound.entry.audio_name)?;
        bound.audio_stamp.require(&file)?;
        let audio = VerifiedAudio::open_cancellable(
            file.try_clone().map_err(|e| e.to_string())?,
            bound.entry.audio_bytes,
            &bound.entry.audio_sha256,
            check,
        )?;
        bound.audio_stamp.require(&file)?;
        bound
            .audio_stamp
            .require(&required(&self.directory, &bound.entry.audio_name)?)?;
        Ok(audio)
    }
}

#[cfg(all(test, unix))]
mod tests;
