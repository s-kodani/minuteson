//! Descriptor-relative, create-new-only placement. No overwrite, unlink,
//! recursive deletion, or symlink traversal, including parent substitution.
use std::fs::{File, Metadata};
use std::path::{Path, PathBuf};

pub(crate) struct Directory {
    file: File,
    path: PathBuf,
    identity: Metadata,
}

#[cfg(unix)]
impl Directory {
    /// Read-only opening: no mkdir, chmod or synchronization of unknown entries.
    pub(crate) fn open_existing(path: &Path) -> Result<Self, String> {
        use std::os::unix::ffi::OsStrExt;
        if !path.is_absolute() || path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("モデル管理領域の読戻し先にリンクがあります".into());
        }
        let name =
            std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::open(
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        Self::from_fd(fd, path.to_path_buf())
    }
    fn name(value: &str) -> Result<std::ffi::CString, String> {
        if value.is_empty() || matches!(value, "." | "..") || value.contains('/') {
            return Err("配置先のパス要素が不正です".into());
        }
        std::ffi::CString::new(value).map_err(|e| e.to_string())
    }
    fn from_fd(fd: i32, path: PathBuf) -> Result<Self, String> {
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::PermissionsExt;
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let identity = file.metadata().map_err(|e| e.to_string())?;
        if !identity.is_dir() {
            return Err("配置先がフォルダーではありません".into());
        }
        // Private model directories; the trusted app-data parent may be public.
        let result = Self {
            file,
            path,
            identity,
        };
        result.verify()?;
        if result.identity.permissions().mode() & 0o077 != 0 {
            return Err("モデル配置先の権限が非公開ではありません".into());
        }
        Ok(result)
    }
    pub(crate) fn root(path: &Path) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        if !path.is_absolute() {
            return Err("モデル管理領域には絶対パスが必要です".into());
        }
        let parent = path
            .parent()
            .ok_or("モデル管理領域の親がありません")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let leaf = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("モデル管理領域名が不正です")?;
        let name = Self::name(leaf)?;
        let parent_c =
            std::ffi::CString::new(parent.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::open(
                parent_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        use std::os::fd::FromRawFd;
        let parent_file = unsafe { File::from_raw_fd(fd) };
        let made = unsafe { libc::mkdirat(parent_file.as_raw_fd(), name.as_ptr(), 0o700) };
        if made < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            return Err(std::io::Error::last_os_error().to_string());
        }
        parent_file.sync_all().map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::openat(
                parent_file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        Self::from_fd(fd, parent.join(leaf))
    }
    pub(crate) fn child(&self, name: &str, new_only: bool) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        self.verify()?;
        let name_c = Self::name(name)?;
        let made = unsafe { libc::mkdirat(self.file.as_raw_fd(), name_c.as_ptr(), 0o700) };
        if made < 0
            && (new_only
                || std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists)
        {
            return Err("モデル配置先に既存物があります。上書きしません".into());
        }
        self.sync()?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        Self::from_fd(fd, self.path.join(name))
    }
    pub(crate) fn create(&self, relative: &str) -> Result<(File, Self), String> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut parts = relative.split('/').collect::<Vec<_>>();
        let leaf = Self::name(parts.pop().ok_or("配置ファイル名がありません")?)?;
        let mut directory = Self::from_fd(
            unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) },
            self.path.clone(),
        )?;
        for part in parts {
            directory = directory.child(part, false)?;
        }
        directory.verify()?;
        let fd = unsafe {
            libc::openat(
                directory.file.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err("モデル配置先の既存ファイルを上書きできません".into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        directory.sync()?;
        Ok((file, directory))
    }
    pub(crate) fn available(&self) -> Result<u64, String> {
        use std::os::fd::AsRawFd;
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::fstatvfs(self.file.as_raw_fd(), info.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let info = unsafe { info.assume_init() };
        u64::try_from(u128::from(info.f_bavail) * u128::from(info.f_frsize))
            .map_err(|_| "モデル空き容量が範囲外です".into())
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let current = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        let opened = self.file.metadata().map_err(|e| e.to_string())?;
        let same = |m: &Metadata| {
            m.is_dir()
                && m.dev() == self.identity.dev()
                && m.ino() == self.identity.ino()
                && m.created().ok() == self.identity.created().ok()
        };
        if !same(&current)
            || !same(&opened)
            || self.path.canonicalize().map_err(|e| e.to_string())? != self.path
        {
            return Err("モデル管理領域の実体・親フォルダーが変更されています".into());
        }
        Ok(())
    }
    pub(crate) fn sync(&self) -> Result<(), String> {
        self.verify()?;
        self.file.sync_all().map_err(|e| e.to_string())
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
