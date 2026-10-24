//! Filesystem metadata obtained through an opened object on Windows. Keep
//! identity checks independent of path spelling and the unstable std metadata
//! file-index APIs. Unix callers keep the original metadata representation.
use std::{fs::File, io, path::Path};
/// Rebuild portable DB paths using native components before appending to an
/// extended-length Windows root (where '/' is a literal component character).
pub(crate) fn join(root: &Path, relative: impl AsRef<Path>) -> std::path::PathBuf {
    let mut result = root.to_path_buf();
    for component in relative.as_ref().components() {
        result.push(component.as_os_str());
    }
    result
}
pub(crate) fn current_owner() -> Result<u32, String> {
    #[cfg(unix)]
    {
        Ok(unsafe { libc::geteuid() })
    }
    #[cfg(windows)]
    {
        crate::windows_files::current_owner()
    }
}
pub(crate) fn private_directories(path: &Path, recursive: bool) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    crate::windows_files::set_private(path)?;
    Ok(())
}
pub(crate) fn sync_handle(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        file.sync_all()
    }
    #[cfg(windows)]
    {
        let path = crate::windows_files::path(file).map_err(io::Error::other)?;
        if file.metadata()?.is_dir() {
            crate::windows_files::sync_directory(&path).map_err(io::Error::other)
        } else {
            sync_file(&path).map_err(io::Error::other)
        }
    }
}
pub(crate) fn read_exact_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(bytes, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < bytes.len() {
            let count = file.seek_read(&mut bytes[done..], offset + done as u64)?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += count;
        }
        Ok(())
    }
}
#[cfg(unix)]
pub(crate) type Metadata = std::fs::Metadata;
#[cfg(windows)]
#[derive(Clone, Debug)]
pub(crate) struct Metadata {
    inner: std::fs::Metadata,
    identity: crate::windows_files::Identity,
    links: u64,
    owner: u32,
    private: bool,
    changed: (i64, i64),
}
#[cfg(windows)]
impl std::ops::Deref for Metadata {
    type Target = std::fs::Metadata;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
#[cfg(windows)]
impl Metadata {
    pub(crate) fn dev(&self) -> u64 {
        self.identity.device
    }
    pub(crate) fn ino(&self) -> u64 {
        self.identity.inode
    }
    pub(crate) fn nlink(&self) -> u64 {
        self.links
    }
    pub(crate) fn uid(&self) -> u32 {
        self.owner
    }
    pub(crate) fn mode(&self) -> u32 {
        let kind = if self.is_dir() {
            0o040000
        } else if self.is_file() {
            0o100000
        } else {
            0
        };
        kind | if self.private {
            if self.is_dir() {
                0o700
            } else {
                0o600
            }
        } else {
            0o777
        }
    }
    pub(crate) fn mtime(&self) -> i64 {
        self.time(self.modified()).0
    }
    pub(crate) fn mtime_nsec(&self) -> i64 {
        self.time(self.modified()).1
    }
    pub(crate) fn ctime(&self) -> i64 {
        self.changed.0
    }
    pub(crate) fn ctime_nsec(&self) -> i64 {
        self.changed.1
    }
    fn time(&self, t: io::Result<std::time::SystemTime>) -> (i64, i64) {
        t.ok()
            .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|v| (v.as_secs() as i64, v.subsec_nanos().into()))
            .unwrap_or((-1, 0))
    }
}
pub(crate) trait MetadataSource {
    fn native_metadata(&self) -> io::Result<Metadata>;
}
impl<T: MetadataSource + ?Sized> MetadataSource for &T {
    fn native_metadata(&self) -> io::Result<Metadata> {
        (**self).native_metadata()
    }
}
impl<T: MetadataSource + ?Sized> MetadataSource for &mut T {
    fn native_metadata(&self) -> io::Result<Metadata> {
        (**self).native_metadata()
    }
}
impl MetadataSource for File {
    fn native_metadata(&self) -> io::Result<Metadata> {
        file_metadata(self)
    }
}
impl<T: MetadataSource + ?Sized> MetadataSource for std::sync::Arc<T> {
    fn native_metadata(&self) -> io::Result<Metadata> {
        self.as_ref().native_metadata()
    }
}
impl MetadataSource for Path {
    fn native_metadata(&self) -> io::Result<Metadata> {
        symlink_metadata(self)
    }
}
impl MetadataSource for std::path::PathBuf {
    fn native_metadata(&self) -> io::Result<Metadata> {
        symlink_metadata(self)
    }
}
pub(crate) fn metadata(value: &(impl MetadataSource + ?Sized)) -> io::Result<Metadata> {
    value.native_metadata()
}
fn file_metadata(file: &File) -> io::Result<Metadata> {
    #[cfg(unix)]
    {
        file.metadata()
    }
    #[cfg(windows)]
    {
        let identity = crate::windows_files::Identity::file(file).map_err(io::Error::other)?;
        let links = crate::windows_files::identity(file)
            .map_err(io::Error::other)?
            .2
            .into();
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        };
        let mut basic = FILE_BASIC_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                &mut basic as *mut _ as *mut _,
                std::mem::size_of_val(&basic) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let ticks = basic
            .ChangeTime
            .checked_sub(116444736000000000)
            .ok_or_else(|| io::Error::other("invalid NTFS change time"))?;
        let (owner, private) =
            crate::windows_files::security_metadata(file).map_err(io::Error::other)?;
        Ok(Metadata {
            inner: file.metadata()?,
            identity,
            links,
            owner,
            private,
            changed: (ticks / 10000000, (ticks % 10000000) * 100),
        })
    }
}
pub(crate) fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    #[cfg(unix)]
    {
        std::fs::symlink_metadata(path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .access_mode(0x80 | 0x20000)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)?;
        metadata(&file)
    }
}
pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        crate::windows_files::sync_directory(path)
    }
}
pub(crate) fn sync_path(path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    if symlink_metadata(path).map_err(|e| e.to_string())?.is_dir() {
        sync_directory(path)
    } else {
        sync_file(path)
    }
}
pub(crate) fn sync_file(path: &Path) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.write(true);
    options
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
pub(crate) fn remove_owned(path: &Path, observed: &Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        if observed.is_dir() {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        }
    }
    #[cfg(windows)]
    {
        crate::windows_files::remove_owned(path, &observed.identity, observed.is_dir())
            .map_err(io::Error::other)
    }
}
