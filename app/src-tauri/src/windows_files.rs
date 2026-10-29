//! Windows handle-bound reads. Reparse points are rejected and pinned parents
//! deny delete sharing so a checked path cannot be redirected during use.
use std::fs::{File, OpenOptions};
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::path::{Component, Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
};
mod security;
#[cfg(test)]
mod tests;
pub(crate) use security::current_sid;
pub(crate) use security::set_private;
pub(crate) fn security_metadata(file: &File) -> Result<(u32, bool), String> {
    security::metadata(file)
}
pub(crate) fn current_owner() -> Result<u32, String> {
    security::current_owner()
}

/// Persistent NTFS identity. Paths, timestamps and hashes alone never grant
/// authority to remove a file. Keep this local; portable archives omit it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub device: u64,
    pub inode: u64,
    pub birth_seconds: u64,
    pub birth_nanos: u32,
}
impl Identity {
    pub(crate) fn file(file: &File) -> Result<Self, String> {
        let (volume, index, _) = identity(file)?;
        let birth = file
            .metadata()
            .map_err(|e| e.to_string())?
            .created()
            .map_err(|e| e.to_string())?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            device: volume.into(),
            inode: index,
            birth_seconds: birth.as_secs(),
            birth_nanos: birth.subsec_nanos(),
        })
    }
    pub(crate) fn at(path: &Path) -> Result<Self, String> {
        let file = open_metadata(path)?;
        Self::file(&file)
    }
}
pub(crate) fn open_metadata(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .access_mode(0x80 | 0x20000)
        .share_mode(7)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    identity(&file)?;
    Ok(file)
}
pub(crate) fn available(path: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    let _parents = parents(path)?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut bytes = 0;
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(bytes)
}
pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    sync_directory_state(path).map(|_| ())
}
pub(crate) fn sync_directory_state(
    path: &Path,
) -> Result<mon_native::durability::NamespaceDurability, String> {
    let _parents = parents(path)?;
    let held = directory(path)?;
    let expected = Identity::file(&held)?;
    // Ordinary users/filesystems may reject directory flushes. Retain the
    // opened read handle and return an explicit weaker namespace guarantee.
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(7)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path);
    let result = match opened {
        Ok(file) => {
            if !file.metadata().map_err(|e| e.to_string())?.is_dir()
                || Identity::file(&file)? != expected
            {
                return Err("同期するフォルダーの実体が変更されています".into());
            }
            file.sync_all()
        }
        Err(error) => Err(error),
    };
    let durability =
        mon_native::durability::windows_directory_sync_result(result).map_err(|e| e.to_string())?;
    if Identity::at(path)? != expected || Identity::file(&held)? != expected {
        return Err("同期するフォルダーの実体が変更されています".into());
    }
    Ok(durability)
}

pub(crate) fn local_ntfs(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };
    let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(input.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let drive = unsafe { GetDriveTypeW(root.as_ptr()) };
    let mut fs = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    let len = fs.iter().position(|c| *c == 0).unwrap_or(fs.len());
    if ok == 0 || !matches!(drive, 2 | 3) || String::from_utf16_lossy(&fs[..len]) != "NTFS" {
        return Err("会議保存先は書込み可能なローカルNTFS領域を選択してください。ネットワーク・未対応形式は使用できません".into());
    }
    Ok(())
}
/// Remove one exact previously-owned object through its handle. Never follows
/// reparse points, traverses unknown trees, or deletes a replacement by name.
pub(crate) fn remove_owned(
    path: &Path,
    expected: &Identity,
    is_directory: bool,
) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfoEx, SetFileInformationByHandle, FILE_DISPOSITION_INFO_EX,
    };
    let _parent = parents(path.parent().ok_or("削除対象の親がありません")?)?;
    let file = OpenOptions::new()
        .access_mode(0x10000 | 0x80)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    let m = file.metadata().map_err(|e| e.to_string())?;
    if Identity::file(&file)? != *expected
        || m.is_dir() != is_directory
        || (!is_directory && identity(&file)?.2 != 1)
    {
        return Err("削除対象の実体・種類・リンク数が変更されています".into());
    }
    // Windows 11 + NTFS: unlink the checked name immediately while a read-only
    // proof handle remains open. Legacy delete-pending would make recovery's
    // post-delete absence check return ACCESS_DENIED until that handle closes.
    let info = FILE_DISPOSITION_INFO_EX { Flags: 0x1 | 0x2 };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfoEx,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub(crate) fn identity(file: &File) -> Result<(u32, u64, u32), String> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if info.dwFileAttributes & 0x400 != 0 {
        return Err("リンク・再解析ポイントは利用できません".into());
    }
    Ok((
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        info.nNumberOfLinks,
    ))
}
pub(crate) fn path(file: &File) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    let n = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if n == 0 || n >= buffer.len() {
        return Err("ファイルの実体パスを取得できません".into());
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..n])))
}
pub(crate) fn directory(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err("通常のフォルダーが必要です".into());
    }
    identity(&file)?;
    Ok(file)
}
pub(crate) fn parents(path: &Path) -> Result<Vec<File>, String> {
    if !path.is_absolute() {
        return Err("絶対パスが必要です".into());
    }
    let mut current = PathBuf::new();
    let mut result = Vec::new();
    for part in path.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => current.push(part),
            Component::Normal(name) => {
                current.push(name);
                result.push(directory(&current)?);
            }
            _ => return Err("不正なパス要素です".into()),
        }
    }
    if result.is_empty() {
        result.push(directory(&current)?);
    }
    Ok(result)
}
pub(crate) fn read(path: &Path, mutable: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(if mutable { 7 } else { 1 })
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() || identity(&file)?.2 != 1 {
        return Err("リンクではない単一所有の通常ファイルが必要です".into());
    }
    Ok(file)
}
pub(crate) fn ordinary(metadata: &std::fs::Metadata) -> bool {
    metadata.file_attributes() & 0x400 == 0
}
