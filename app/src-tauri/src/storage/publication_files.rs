//! Descriptor-relative conversion workspace. Never overwrite a final object,
//! follow a link, or truncate a partial without its durable identity receipt.
#[cfg(windows)]
use crate::bootstrap::windows_io as libc;
#[cfg(windows)]
use crate::bootstrap::windows_io::{AsRawFd, FromRawFd};
use crate::native_fs::Metadata;
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::storage) struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    birth: (u64, u32),
}
impl Identity {
    pub(in crate::storage) fn read(metadata: &Metadata) -> Result<Self, String> {
        let birth = metadata
            .created()
            .map_err(|e| e.to_string())?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode(),
            birth: (birth.as_secs(), birth.subsec_nanos()),
        })
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn registered(&self) -> Result<(u64, u64, u64), String> {
        self.validate(self.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32)?;
        Ok((
            self.device,
            self.inode,
            self.birth
                .0
                .checked_mul(1_000_000_000)
                .and_then(|n| n.checked_add(self.birth.1.into()))
                .ok_or("音声保存実体の時刻が範囲外です")?,
        ))
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn validate(&self, directory: bool) -> Result<(), String> {
        let kind = if directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if self.birth.1 >= 1_000_000_000
            || self.mode & libc::S_IFMT as u32 != kind as u32
            || self.uid != crate::native_fs::current_owner()?
        {
            return Err("音声保存実体の所有記録が不正です".into());
        }
        Ok(())
    }
    pub(in crate::storage) fn file(file: &File) -> Result<Self, String> {
        let m = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != crate::native_fs::current_owner()?
            || m.mode() & 0o077 != 0
        {
            return Err("音声保存作業ファイルの実体・所有・権限が不正です".into());
        }
        Self::read(&m)
    }
    pub(in crate::storage) fn require(&self, file: &File) -> Result<(), String> {
        if *self != Self::file(file)? {
            return Err("音声保存作業ファイルが差し替えられています".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::storage) struct Stamp {
    identity: Identity,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    pub(in crate::storage) fn read(file: &File) -> Result<Self, String> {
        let m = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        Ok(Self {
            identity: Identity::file(file)?,
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    pub(in crate::storage) fn require(&self, file: &File) -> Result<(), String> {
        if *self != Self::read(file)? {
            return Err("音声保存確定証拠が検証後に変更されています".into());
        }
        Ok(())
    }
}

pub(in crate::storage) struct Directory {
    pub file: File,
    pub path: PathBuf,
    pub identity: Identity,
}
fn name(value: &str) -> Result<CString, String> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value.contains(['/', '\\'])
    {
        return Err("音声保存作業名が不正です".into());
    }
    CString::new(value).map_err(|e| e.to_string())
}
#[cfg(unix)]
type NativeDescriptor = std::os::fd::RawFd;
#[cfg(windows)]
type NativeDescriptor = crate::bootstrap::windows_io::RawFd;
fn descriptor(fd: NativeDescriptor) -> Result<File, String> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl Directory {
    pub(in crate::storage) fn open(path: &Path) -> Result<Self, String> {
        let file = crate::input_levels::open_capture_directory(path)?;
        let identity =
            Identity::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
        if identity.uid != crate::native_fs::current_owner()? {
            return Err("音声保存管理先の所有者が不正です".into());
        }
        Ok(Self {
            file,
            path: path.to_owned(),
            identity,
        })
    }
    pub(in crate::storage) fn verify(&self) -> Result<(), String> {
        let current = Self::open(&self.path)?;
        if self.identity != current.identity {
            return Err("音声保存管理先が差し替えられています".into());
        }
        Ok(())
    }
    pub(in crate::storage) fn create_child(&self, leaf: &str) -> Result<Self, String> {
        self.verify()?;
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), name(leaf)?.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        crate::native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        let child = Self::open(&self.path.join(leaf))?;
        self.verify()?;
        Ok(child)
    }
    pub(in crate::storage) fn optional(
        &self,
        leaf: &str,
        writable: bool,
    ) -> Result<Option<File>, String> {
        self.verify()?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name(leaf)?.as_ptr(),
                (if writable {
                    libc::O_RDWR
                } else {
                    libc::O_RDONLY
                }) | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let file = descriptor(fd)?;
        Identity::file(&file)?;
        Ok(Some(file))
    }
    pub(in crate::storage) fn required(&self, leaf: &str, writable: bool) -> Result<File, String> {
        self.optional(leaf, writable)?
            .ok_or("音声保存作業の確定証拠が不足しています".into())
    }
    pub(in crate::storage) fn create(&self, leaf: &str) -> Result<File, String> {
        self.verify()?;
        let file = descriptor(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name(leaf)?.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        Identity::file(&file)?;
        Ok(file)
    }
    pub(in crate::storage) fn read_json(
        &self,
        leaf: &str,
    ) -> Result<Option<(Vec<u8>, Stamp)>, String> {
        let Some(mut file) = self.optional(leaf, false)? else {
            return Ok(None);
        };
        let stamp = Stamp::read(&file)?;
        if stamp.size == 0 || stamp.size > 512 * 1024 {
            return Err("音声保存確定記録が容量上限外です".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != stamp.size {
            return Err("音声保存確定記録が読込み中に変わりました".into());
        }
        stamp.require(&file)?;
        stamp.require(&self.required(leaf, false)?)?;
        Ok(Some((bytes, stamp)))
    }
    pub(in crate::storage) fn rename(
        &self,
        from: &str,
        to: &str,
        exclusive: bool,
    ) -> Result<(), String> {
        self.verify()?;
        let from = name(from)?;
        let to = name(to)?;
        #[cfg(any(target_os = "macos", windows))]
        let result = unsafe {
            libc::renameatx_np(
                self.file.as_raw_fd(),
                from.as_ptr(),
                self.file.as_raw_fd(),
                to.as_ptr(),
                if exclusive { libc::RENAME_EXCL } else { 0 },
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.file.as_raw_fd(),
                from.as_ptr(),
                self.file.as_raw_fd(),
                to.as_ptr(),
                if exclusive { libc::RENAME_NOREPLACE } else { 0 },
            ) as i32
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let result = -1;
        if result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        crate::native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        self.verify()
    }
    pub(in crate::storage) fn json(
        &self,
        leaf: &str,
        value: &impl Serialize,
        previous: Option<&Stamp>,
    ) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > 512 * 1024 {
            return Err("音声保存確定記録が大きすぎます".into());
        }
        let temporary = format!(".{}.json.part", uuid::Uuid::new_v4());
        let mut file = self.create(&temporary)?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
        if let Some(stamp) = previous {
            stamp.require(&self.required(leaf, false)?)?;
        }
        self.rename(&temporary, leaf, previous.is_none())
    }
}
