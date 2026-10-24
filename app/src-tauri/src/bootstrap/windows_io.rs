//! Descriptor-shaped boundary for the shared relocation state machine. These
//! are real Win32 file handles, not CRT descriptors or fabricated inode values.
use std::{
    ffi::CStr,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, IntoRawHandle},
    },
    path::{Path, PathBuf},
};
pub(crate) type RawFd = isize;
pub(crate) trait AsRawFd {
    fn as_raw_fd(&self) -> RawFd;
}
impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as RawFd
    }
}
pub(crate) trait FromRawFd {
    unsafe fn from_raw_fd(handle: RawFd) -> File;
}
impl FromRawFd for File {
    unsafe fn from_raw_fd(handle: RawFd) -> File {
        unsafe { File::from_raw_handle(handle as *mut _) }
    }
}
pub(crate) trait OsStrExt {
    fn as_bytes(&self) -> Vec<u8>;
}
impl OsStrExt for std::ffi::OsStr {
    fn as_bytes(&self) -> Vec<u8> {
        self.to_string_lossy().as_bytes().to_vec()
    }
}
pub(crate) const O_RDONLY: u32 = 0;
pub(crate) const O_WRONLY: u32 = 1;
pub(crate) const O_RDWR: u32 = 2;
pub(crate) const O_CREAT: u32 = 0x100;
pub(crate) const O_EXCL: u32 = 0x200;
pub(crate) const O_DIRECTORY: u32 = 0x02000000;
pub(crate) const O_NOFOLLOW: u32 = 0x00200000;
pub(crate) const O_CLOEXEC: u32 = 0;
pub(crate) const O_NONBLOCK: u32 = 0;
pub(crate) const RENAME_EXCL: u32 = 4;
pub(crate) const S_IFMT: u32 = 0o170000;
pub(crate) const S_IFDIR: u32 = 0o040000;
pub(crate) const S_IFREG: u32 = 0o100000;
pub(crate) const LOCK_SH: i32 = 1;
pub(crate) const LOCK_EX: i32 = 2;
pub(crate) const LOCK_NB: i32 = 4;
pub(crate) unsafe fn flock(handle: RawFd, flags: i32) -> i32 {
    let file = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(handle as *mut _) });
    let result = if flags & LOCK_EX != 0 {
        file.try_lock()
    } else if flags & LOCK_SH != 0 {
        file.try_lock_shared()
    } else {
        return failed(io::Error::other("unsupported lock operation")) as i32;
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let message = io::Error::other(e.to_string());
            failed(message);
            -1
        }
    }
}
fn failed(error: io::Error) -> RawFd {
    unsafe {
        windows_sys::Win32::Foundation::SetLastError(error.raw_os_error().unwrap_or(13) as u32);
    }
    -1
}
fn error(text: String) -> io::Error {
    io::Error::other(text)
}
unsafe fn text(pointer: *const i8) -> io::Result<String> {
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(io::Error::other)
}
fn parent(handle: RawFd) -> io::Result<PathBuf> {
    let file = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(handle as *mut _) });
    crate::windows_files::path(&file).map_err(error)
}
fn leaf(handle: RawFd, pointer: *const i8) -> io::Result<PathBuf> {
    let name = unsafe { text(pointer) }?;
    if name.is_empty() || name.contains(['/', '\\', ':']) || matches!(name.as_str(), "." | "..") {
        return Err(io::Error::other("invalid relative native component"));
    }
    Ok(parent(handle)?.join(name))
}
fn opened(path: &Path, flags: u32) -> io::Result<File> {
    let _parents = crate::windows_files::parents(
        path.parent()
            .ok_or_else(|| io::Error::other("missing parent"))?,
    )
    .map_err(error)?;
    let mut options = OpenOptions::new();
    let access = match flags & 3 {
        O_WRONLY => 0x40000000,
        O_RDWR => 0xc0000000,
        _ => 0x80000000,
    };
    options
        .read(flags & 3 != O_WRONLY)
        .write(flags & 3 != O_RDONLY)
        .access_mode(access)
        .share_mode(if flags & O_DIRECTORY != 0 { 3 } else { 7 })
        .custom_flags(
            O_NOFOLLOW
                | if flags & O_DIRECTORY != 0 {
                    O_DIRECTORY
                } else {
                    0
                },
        );
    if flags & O_CREAT != 0 {
        if flags & O_EXCL != 0 {
            options.create_new(true);
        } else {
            options.create(true);
        }
    }
    let file = options.open(path)?;
    let (_, _, links) = crate::windows_files::identity(&file).map_err(error)?;
    let m = file.metadata()?;
    if (flags & O_DIRECTORY != 0 && !m.is_dir()) || (!m.is_dir() && (!m.is_file() || links != 1)) {
        return Err(io::Error::other("native object kind/links changed"));
    }
    if flags & (O_CREAT | O_EXCL) == (O_CREAT | O_EXCL) {
        crate::windows_files::set_private(path).map_err(error)?;
    }
    Ok(file)
}
pub(crate) unsafe fn open(pointer: *const i8, flags: u32) -> RawFd {
    match unsafe { text(pointer) }.and_then(|s| opened(Path::new(&s), flags)) {
        Ok(file) => file.into_raw_handle() as RawFd,
        Err(e) => failed(e),
    }
}
pub(crate) unsafe fn openat(directory: RawFd, pointer: *const i8, flags: u32, _mode: u32) -> RawFd {
    match leaf(directory, pointer).and_then(|path| opened(&path, flags)) {
        Ok(file) => file.into_raw_handle() as RawFd,
        Err(e) => failed(e),
    }
}
pub(crate) unsafe fn mkdirat(directory: RawFd, pointer: *const i8, _mode: u32) -> i32 {
    let result = (|| {
        let path = leaf(directory, pointer)?;
        let _pin = crate::windows_files::parents(
            path.parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )
        .map_err(error)?;
        std::fs::create_dir(&path)?;
        crate::windows_files::set_private(&path).map_err(error)
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            failed(e);
            -1
        }
    }
}
pub(crate) unsafe fn renameatx_np(
    source: RawFd,
    from: *const i8,
    target: RawFd,
    to: *const i8,
    flags: u32,
) -> i32 {
    use std::os::windows::ffi::OsStrExt as Wide;
    let result = (|| {
        let from = leaf(source, from)?;
        let to = leaf(target, to)?;
        let _source = crate::windows_files::parents(
            from.parent()
                .ok_or_else(|| io::Error::other("missing source parent"))?,
        )
        .map_err(error)?;
        let _target = crate::windows_files::parents(
            to.parent()
                .ok_or_else(|| io::Error::other("missing target parent"))?,
        )
        .map_err(error)?;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                8 | if flags & RENAME_EXCL == 0 { 1 } else { 0 },
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            failed(e);
            -1
        }
    }
}
