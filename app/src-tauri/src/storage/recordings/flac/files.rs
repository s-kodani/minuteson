//! Descriptor-relative conversion workspace. Never overwrite a final object,
//! follow a link, or truncate a partial without its durable identity receipt.
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    birth: (u64, u32),
}
impl Identity {
    fn read(metadata: &Metadata) -> Result<Self, String> {
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
    pub(super) fn file(file: &File) -> Result<Self, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
        {
            return Err("FLAC作業ファイルの実体・所有・権限が不正です".into());
        }
        Self::read(&m)
    }
    pub(super) fn require(&self, file: &File) -> Result<(), String> {
        if *self != Self::file(file)? {
            return Err("FLAC作業ファイルが差し替えられています".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    identity: Identity,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    pub(super) fn read(file: &File) -> Result<Self, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        Ok(Self {
            identity: Identity::file(file)?,
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    pub(super) fn require(&self, file: &File) -> Result<(), String> {
        if *self != Self::read(file)? {
            return Err("FLAC確定証拠が検証後に変更されています".into());
        }
        Ok(())
    }
}

pub(super) struct Directory {
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
        return Err("FLAC作業名が不正です".into());
    }
    CString::new(value).map_err(|e| e.to_string())
}
fn descriptor(fd: i32) -> Result<File, String> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl Directory {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let file = crate::input_levels::open_capture_directory(path)?;
        let identity = Identity::read(&file.metadata().map_err(|e| e.to_string())?)?;
        if identity.uid != unsafe { libc::geteuid() } {
            return Err("FLAC管理先の所有者が不正です".into());
        }
        Ok(Self {
            file,
            path: path.to_owned(),
            identity,
        })
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        let current = Self::open(&self.path)?;
        if self.identity != current.identity {
            return Err("FLAC管理先が差し替えられています".into());
        }
        Ok(())
    }
    pub(super) fn create_child(&self, leaf: &str) -> Result<Self, String> {
        self.verify()?;
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), name(leaf)?.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.file.sync_all().map_err(|e| e.to_string())?;
        let child = Self::open(&self.path.join(leaf))?;
        self.verify()?;
        Ok(child)
    }
    pub(super) fn optional(&self, leaf: &str, writable: bool) -> Result<Option<File>, String> {
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
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let file = descriptor(fd)?;
        Identity::file(&file)?;
        Ok(Some(file))
    }
    pub(super) fn required(&self, leaf: &str, writable: bool) -> Result<File, String> {
        self.optional(leaf, writable)?
            .ok_or("FLAC作業の確定証拠が不足しています".into())
    }
    pub(super) fn create(&self, leaf: &str) -> Result<File, String> {
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
    pub(super) fn read_json(&self, leaf: &str) -> Result<Option<(Vec<u8>, Stamp)>, String> {
        let Some(mut file) = self.optional(leaf, false)? else {
            return Ok(None);
        };
        let stamp = Stamp::read(&file)?;
        if stamp.size == 0 || stamp.size > 512 * 1024 {
            return Err("FLAC確定記録が容量上限外です".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != stamp.size {
            return Err("FLAC確定記録が読込み中に変わりました".into());
        }
        stamp.require(&file)?;
        stamp.require(&self.required(leaf, false)?)?;
        Ok(Some((bytes, stamp)))
    }
    pub(super) fn rename(&self, from: &str, to: &str, exclusive: bool) -> Result<(), String> {
        self.verify()?;
        let from = name(from)?;
        let to = name(to)?;
        #[cfg(target_os = "macos")]
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
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let result = -1;
        if result != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.file.sync_all().map_err(|e| e.to_string())?;
        self.verify()
    }
    pub(super) fn json(
        &self,
        leaf: &str,
        value: &impl Serialize,
        previous: Option<&Stamp>,
    ) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > 512 * 1024 {
            return Err("FLAC確定記録が大きすぎます".into());
        }
        let temporary = format!(".{}.json.part", uuid::Uuid::new_v4());
        let mut file = self.create(&temporary)?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        if let Some(stamp) = previous {
            stamp.require(&self.required(leaf, false)?)?;
        }
        self.rename(&temporary, leaf, previous.is_none())
    }
}
