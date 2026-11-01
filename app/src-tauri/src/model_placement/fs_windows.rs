use crate::windows_files as win;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(crate) struct Directory {
    file: File,
    path: PathBuf,
    _parents: Vec<File>,
    modified: u64,
}
impl Directory {
    pub(crate) fn identity(&self) -> Result<super::DirectoryIdentity, String> {
        self.verify()?;
        super::directory_identity(&self.file)
    }
    /// Rename the exact opened child, never replace another destination.
    /// Only ERROR_NOT_SAME_DEVICE permits a copy fallback.
    pub(crate) fn move_child(
        &self,
        source: &str,
        target: &Self,
        name: &str,
        expected: &super::DirectoryIdentity,
    ) -> Result<bool, String> {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
        };
        Self::name(source)?;
        Self::name(name)?;
        self.verify()?;
        target.verify()?;
        let file = OpenOptions::new()
            .access_mode(0x10000 | 0x20000 | 0x80)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(self.path.join(source))
            .map_err(|e| e.to_string())?;
        if &super::directory_identity(&file)? != expected {
            return Err("移動するモデル領域が変更されています".into());
        }
        let destination: Vec<u16> = target
            .path
            .join(name)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + destination.len() * 2;
        let mut buffer = vec![0u64; bytes.div_ceil(8)];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = ((destination.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(
                destination.as_ptr(),
                (*info).FileName.as_mut_ptr(),
                destination.len(),
            );
            if SetFileInformationByHandle(
                file.as_raw_handle(),
                FileRenameInfo,
                info.cast(),
                bytes as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(17) {
                    return Ok(false);
                }
                return Err(error.to_string());
            }
        }
        self.sync()?;
        target.sync()?;
        Ok(true)
    }
    pub(crate) fn open_existing(path: &Path) -> Result<Self, String> {
        let parents = win::parents(path)?;
        let file = win::directory(path)?;
        if !win::security_metadata(&file)?.1 {
            return Err("モデル配置先の権限が非公開ではありません".into());
        }
        let path = win::path(&file)?;
        let modified = file
            .metadata()
            .map_err(|e| e.to_string())?
            .last_write_time();
        Ok(Self {
            file,
            path,
            _parents: parents,
            modified,
        })
    }
    pub(crate) fn root(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("管理領域の親がありません")?;
        let _pins = win::parents(parent)?;
        match std::fs::create_dir(path) {
            Ok(()) => win::set_private(path)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.to_string()),
        }
        Self::open_existing(path)
    }
    fn name(name: &str) -> Result<(), String> {
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.contains(['/', '\\', ':', '\0'])
            || name.ends_with([' ', '.'])
        {
            return Err("管理領域の名前が不正です".into());
        }
        Ok(())
    }
    pub(crate) fn child(&self, name: &str, new_only: bool) -> Result<Self, String> {
        Self::name(name)?;
        self.verify()?;
        let path = self.path.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => win::set_private(&path)?,
            Err(e) if !new_only && e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.to_string()),
        }
        Self::open_existing(&path)
    }
    pub(crate) fn create(&self, relative: &str) -> Result<(File, Self), String> {
        let mut parts = relative.split('/').collect::<Vec<_>>();
        let leaf = parts.pop().ok_or("ファイル名がありません")?;
        Self::name(leaf)?;
        let mut parent = Self::open_existing(&self.path)?;
        for part in parts {
            parent = parent.child(part, false)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(1)
            .custom_flags(0x80000000 | 0x00200000)
            .open(parent.path.join(leaf))
            .map_err(|e| e.to_string())?;
        win::identity(&file)?;
        Ok((file, parent))
    }
    pub(crate) fn available(&self) -> Result<u64, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        self.verify()?;
        let path = self
            .path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut bytes = 0u64;
        if unsafe {
            GetDiskFreeSpaceExW(
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
    pub(crate) fn verify(&self) -> Result<(), String> {
        let current = win::directory(&self.path)?;
        if win::identity(&current)? != win::identity(&self.file)? {
            return Err("管理領域が変更されました".into());
        }
        if !win::security_metadata(&current)?.1 {
            return Err("モデル配置先の非公開権限が変更されました".into());
        }
        Ok(())
    }
    pub(crate) fn verify_unchanged(&self) -> Result<(), String> {
        self.verify()?;
        if self
            .file
            .metadata()
            .map_err(|e| e.to_string())?
            .last_write_time()
            != self.modified
        {
            return Err("読戻し中に管理領域が変更されました".into());
        }
        Ok(())
    }
    pub(crate) fn sync_namespace(
        &self,
    ) -> Result<mon_native::durability::NamespaceDurability, String> {
        self.verify()?;
        let durability = win::sync_directory_state(&self.path)?;
        self.verify()?;
        Ok(durability)
    }
    // Flushed file data and restart recovery are the minimum contract. The
    // actual namespace guarantee is available separately, never fabricated.
    pub(crate) fn sync(&self) -> Result<(), String> {
        self.sync_namespace().map(|_| ())
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
