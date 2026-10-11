//! macOS system libarchive, streaming TAR/gzip/bzip2 only. Never use its disk
//! extraction API: destinations are independently validated create-new handles.
//! Archive permissions, ownership, links, devices and scripts are not applied.
use super::{check, Recipe, Source};
use crate::model_placement::fs::Directory;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;

#[link(name = "archive")]
extern "C" {
    fn archive_read_new() -> *mut c_void;
    fn archive_read_support_filter_none(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_gzip(a: *mut c_void) -> c_int;
    fn archive_read_support_filter_bzip2(a: *mut c_void) -> c_int;
    fn archive_read_support_format_tar(a: *mut c_void) -> c_int;
    fn archive_read_open_fd(a: *mut c_void, fd: c_int, block_size: usize) -> c_int;
    fn archive_read_next_header(a: *mut c_void, entry: *mut *mut c_void) -> c_int;
    fn archive_read_data(a: *mut c_void, buffer: *mut c_void, len: usize) -> isize;
    fn archive_read_free(a: *mut c_void) -> c_int;
    fn archive_entry_pathname(entry: *mut c_void) -> *const c_char;
    fn archive_entry_size(entry: *mut c_void) -> i64;
    fn archive_entry_filetype(entry: *mut c_void) -> libc::mode_t;
    fn archive_entry_symlink(entry: *mut c_void) -> *const c_char;
    fn archive_entry_hardlink(entry: *mut c_void) -> *const c_char;
}
struct Reader(*mut c_void);
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                archive_read_free(self.0);
            }
        }
    }
}
fn path(raw: &str, directory: bool) -> Result<&str, String> {
    let normalized = raw.strip_prefix("./").unwrap_or(raw);
    let normalized = if directory {
        normalized.trim_end_matches('/')
    } else {
        normalized
    };
    if directory && matches!(normalized, "" | ".") {
        return Ok("");
    }
    if !crate::model_inspection::relative(normalized) {
        return Err("アーカイブ内に不正なパスがあります".into());
    }
    Ok(normalized)
}

pub(super) fn extract(
    directory: &Directory,
    relative: &str,
    payload: &Directory,
    recipe: &Recipe,
    indices: &[usize],
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check(cancelled)?;
    let first = &recipe.sources[*indices.first().ok_or("展開対象がありません")?];
    let archive = first.archive.as_ref().ok_or("アーカイブ指定がありません")?;
    let mut source = Source::open(directory.path(), relative)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 65_536];
    loop {
        check(cancelled)?;
        let n = source
            .file
            .read(&mut buffer)
            .map_err(|_| "アーカイブを再照合できません")?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .filter(|n| *n <= archive.expected_size)
            .ok_or("アーカイブ容量が一致しません")?;
        digest.update(&buffer[..n]);
    }
    if size != archive.expected_size || format!("{:x}", digest.finalize()) != archive.expected_hash
    {
        return Err("アーカイブの容量・SHAが一致しません".into());
    }
    source.verify()?;
    source
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|_| "アーカイブを読み取れません")?;
    // Source stays alive until Reader is freed. libarchive does not own the fd.
    let reader = Reader(unsafe { archive_read_new() });
    if reader.0.is_null() {
        return Err("アーカイブ読取を開始できません".into());
    }
    unsafe {
        if archive_read_support_filter_none(reader.0) != 0
            || archive_read_support_filter_gzip(reader.0) != 0
            || archive_read_support_filter_bzip2(reader.0) != 0
            || archive_read_support_format_tar(reader.0) != 0
            || archive_read_open_fd(reader.0, source.file.as_raw_fd(), buffer.len()) != 0
        {
            return Err("対応するTARアーカイブを開けません".into());
        }
    }
    let selected: BTreeMap<_, _> = indices
        .iter()
        .map(|index| {
            (
                recipe.sources[*index]
                    .archive
                    .as_ref()
                    .unwrap()
                    .member_path
                    .as_str(),
                *index,
            )
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut extracted = BTreeSet::new();
    let mut expanded = 0_u64;
    let deadline = std::time::Instant::now();
    loop {
        check(cancelled)?;
        if deadline.elapsed().as_secs() > 600 {
            return Err("アーカイブ読取の時間上限を超えました".into());
        }
        source.verify()?;
        payload.verify()?;
        let mut entry = std::ptr::null_mut();
        let status = unsafe { archive_read_next_header(reader.0, &mut entry) };
        if status == 1 {
            break;
        } // ARCHIVE_EOF, no warnings/retry accepted.
        if status != 0 || entry.is_null() {
            return Err("アーカイブのヘッダーを確認できません".into());
        }
        let raw = unsafe { archive_entry_pathname(entry) };
        if raw.is_null() {
            return Err("アーカイブ内ファイル名がありません".into());
        }
        let raw = unsafe { CStr::from_ptr(raw) }
            .to_str()
            .map_err(|_| "アーカイブ内ファイル名が不正です")?;
        let kind = unsafe { archive_entry_filetype(entry) };
        let name = path(raw, kind == 0o040000)?;
        if seen.len() >= 10_000 || !seen.insert(name.to_lowercase()) {
            return Err("アーカイブの重複パスまたは項目数上限を検出しました".into());
        }
        let length = u64::try_from(unsafe { archive_entry_size(entry) })
            .map_err(|_| "アーカイブ内容量が不正です")?;
        expanded = expanded
            .checked_add(length)
            .filter(|n| *n <= super::super::model_download_recipe::MAX_TOTAL)
            .ok_or("展開容量の上限は32GiBです")?;
        let selected_index = selected.get(name).copied();
        let mut destination = if let Some(index) = selected_index {
            if kind != 0o100000
                || unsafe {
                    !archive_entry_symlink(entry).is_null()
                        || !archive_entry_hardlink(entry).is_null()
                }
                || length != recipe.manifest.files[index].expected_size.unwrap()
            {
                return Err(
                    "選択したアーカイブ内ファイルが通常ファイル・確認容量と一致しません".into(),
                );
            }
            Some(payload.create(&recipe.manifest.files[index].relative_path)?)
        } else {
            None
        };
        // Drain even unselected entries in bounded chunks, with cancellation and
        // declared-byte checks. Do not let next_header implicitly skip a bomb.
        let mut count = 0_u64;
        let mut digest = Sha256::new();
        loop {
            check(cancelled)?;
            if deadline.elapsed().as_secs() > 600 {
                return Err("アーカイブ読取の時間上限を超えました".into());
            }
            let n =
                unsafe { archive_read_data(reader.0, buffer.as_mut_ptr().cast(), buffer.len()) };
            if n < 0 {
                return Err("アーカイブ本文を確認できません".into());
            }
            if n == 0 {
                break;
            }
            count = count
                .checked_add(n as u64)
                .filter(|n| *n <= length)
                .ok_or("アーカイブ本文容量が一致しません")?;
            if let Some((file, parent)) = &mut destination {
                parent.verify()?;
                file.write_all(&buffer[..n as usize])
                    .map_err(|_| "選択ファイルを書き込めません")?;
                digest.update(&buffer[..n as usize]);
            }
        }
        if let Some(index) = selected_index {
            if count != length
                || format!("{:x}", digest.finalize())
                    != *recipe.manifest.files[index].expected_hash.as_ref().unwrap()
            {
                return Err("展開したファイルの容量・SHAが一致しません".into());
            }
            let (file, parent) = destination.unwrap();
            file.sync_all()
                .map_err(|_| "展開したファイルを確定できません")?;
            parent.sync()?;
            extracted.insert(index);
        }
    }
    if extracted.len() != indices.len() {
        return Err("必要なファイルがアーカイブ内にありません".into());
    }
    source.verify()?;
    directory.verify()?;
    payload.sync()?;
    Ok(())
}
