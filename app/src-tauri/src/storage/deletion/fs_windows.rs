//! Windows counterpart of the deletion filesystem capability. Shared removal
//! plans still own consent, staging names and durable progress. Every mutation
//! checks the opened NTFS object; no recursive deletion is exposed here.
use crate::{
    native_fs::{self, Metadata},
    windows_files as win,
};
use serde::{Deserialize, Serialize};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    birth_seconds: u64,
    birth_nanos: u32,
    mode: u32,
}
impl Identity {
    pub(super) fn registered(&self) -> Result<(u64, u64, u64), String> {
        Ok((
            self.device,
            self.inode,
            self.birth_seconds
                .checked_mul(1_000_000_000)
                .and_then(|n| n.checked_add(self.birth_nanos.into()))
                .ok_or("実体の時刻が範囲外です")?,
        ))
    }
    pub(super) fn validate(&self, directory: bool) -> Result<(), String> {
        if self.birth_nanos >= 1_000_000_000
            || self.mode & 0o170000 != if directory { 0o040000 } else { 0o100000 }
        {
            return Err("管理対象のWindows実体記録が不正です".into());
        }
        Ok(())
    }
    fn read(m: &Metadata) -> Result<Self, String> {
        let birth = m
            .created()
            .map_err(|e| e.to_string())?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            birth_seconds: birth.as_secs(),
            birth_nanos: birth.subsec_nanos(),
            mode: m.mode(),
        })
    }
    fn native(&self) -> win::Identity {
        win::Identity {
            device: self.device,
            inode: self.inode,
            birth_seconds: self.birth_seconds,
            birth_nanos: self.birth_nanos,
        }
    }
}
fn same_persistent(a: &Identity, b: &Identity) -> bool {
    a.inode == b.inode
        && a.birth_seconds == b.birth_seconds
        && a.birth_nanos == b.birth_nanos
        && a.mode == b.mode
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Proof {
    pub(super) identity: Identity,
    pub size: u64,
    links: u64,
    modified_seconds: i64,
    modified_nanos: i64,
    change_seconds: i64,
    change_nanos: i64,
}
impl Proof {
    // Folder scope binds the object, not the contents or mutable timestamps.
    pub(super) fn directory_scope(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            size: 0,
            links: 1,
            modified_seconds: 0,
            modified_nanos: 0,
            change_seconds: 0,
            change_nanos: 0,
        }
    }
    pub(super) fn matches_after_rename(&self, old: &Self) -> bool {
        self.identity == old.identity
            && self.size == old.size
            && self.links == old.links
            && self.modified_seconds == old.modified_seconds
            && self.modified_nanos == old.modified_nanos
    }
    pub(super) fn validate(&self, directory: bool) -> Result<(), String> {
        self.identity.validate(directory)?;
        if self.links == 0
            || (!directory && self.links != 1)
            || self.size > i64::MAX as u64
            || !(0..1_000_000_000).contains(&self.modified_nanos)
            || !(0..1_000_000_000).contains(&self.change_nanos)
        {
            return Err("管理対象の変更・容量・リンク記録が不正です".into());
        }
        Ok(())
    }
    pub(super) fn read(m: &Metadata) -> Result<Self, String> {
        Ok(Self {
            identity: Identity::read(m)?,
            size: m.len(),
            links: m.nlink(),
            modified_seconds: m.mtime(),
            modified_nanos: m.mtime_nsec(),
            change_seconds: m.ctime(),
            change_nanos: m.ctime_nsec(),
        })
    }
}
fn name(s: &str) -> Result<(), String> {
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if s.is_empty()
        || s.len() > 255
        || matches!(s, "." | "..")
        || s.contains(['/', '\\', ':', '\0'])
        || s.ends_with([' ', '.'])
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("管理パスの要素が不正です".into());
    }
    Ok(())
}
pub(in crate::storage) struct Directory {
    file: File,
    path: PathBuf,
    pub(super) identity: Identity,
    _parents: Vec<File>,
}
impl Directory {
    pub(super) fn root(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("管理領域の絶対パスまたは親が不正です".into());
        }
        // Native AppData paths omit the extended-length prefix on Windows.
        // Pin/check every parent and reject reparse points before normalizing
        // the opened directory's spelling. No alternate object is adopted.
        let parents = win::parents(path.parent().ok_or("管理領域の親がありません")?)?;
        let file = win::open_metadata(path)?;
        let opened_path = win::path(&file)?;
        if path.canonicalize().map_err(|e| e.to_string())? != opened_path {
            return Err("管理領域の実体・親フォルダーが変更されています".into());
        }
        let path = opened_path;
        let m = native_fs::metadata(&file).map_err(|e| e.to_string())?;
        if !m.is_dir() {
            return Err("通常の管理フォルダーが必要です".into());
        }
        let result = Self {
            file,
            path,
            identity: Identity::read(&m)?,
            _parents: parents,
        };
        result.verify()?;
        Ok(result)
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        let current = native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        let opened = native_fs::metadata(&self.file).map_err(|e| e.to_string())?;
        if !current.is_dir()
            || Identity::read(&current)? != self.identity
            || Identity::read(&opened)? != self.identity
            || self.path.canonicalize().map_err(|e| e.to_string())? != self.path
        {
            return Err("管理領域の実体・親フォルダーが変更されています".into());
        }
        Ok(())
    }
    pub(super) fn verify_identity(&self, id: &Identity) -> Result<(), String> {
        self.verify()?;
        if &self.identity != id {
            Err("確認したフォルダーの実体が変更されています".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn proof(&self) -> Result<Proof, String> {
        self.verify()?;
        Proof::read(&native_fs::metadata(&self.file).map_err(|e| e.to_string())?)
    }
    pub(super) fn child(&self, leaf: &str) -> Result<Option<Self>, String> {
        name(leaf)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        match native_fs::symlink_metadata(self.path.join(leaf)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(_) => {
                let child = Self::root(&self.path.join(leaf))?;
                if child.identity.device != self.identity.device {
                    return Err("別媒体を参照しています".into());
                }
                Ok(Some(child))
            }
        }
    }
    fn optional_regular(&self, leaf: &str) -> Result<Option<File>, String> {
        name(leaf)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        let opened = match OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(self.path.join(leaf))
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        self.verify_file(leaf, &opened)?;
        Ok(Some(opened))
    }
    pub(super) fn regular(&self, leaf: &str) -> Result<File, String> {
        self.optional_regular(leaf)?.ok_or_else(|| {
            format!(
                "管理ファイルが見つかりません: {}",
                self.path.join(leaf).display()
            )
        })
    }
    pub(super) fn protected_regular(&self, leaf: &str) -> Result<File, String> {
        name(leaf)?;
        self.verify()?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(5)
            .custom_flags(0x00200000)
            .open(self.path.join(leaf))
            .map_err(|e| e.to_string())?;
        self.verify_file(leaf, &file)?;
        Ok(file)
    }
    pub(super) fn verify_file(&self, leaf: &str, file: &File) -> Result<(), String> {
        name(leaf)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        let current =
            native_fs::symlink_metadata(self.path.join(leaf)).map_err(|e| e.to_string())?;
        let held = native_fs::metadata(file).map_err(|e| e.to_string())?;
        if !current.is_file()
            || !held.is_file()
            || current.nlink() != 1
            || held.nlink() != 1
            || Identity::read(&current)? != Identity::read(&held)?
            || current.dev() != self.identity.device
        {
            return Err("管理ファイルの実体・親・共有リンクが変更されています".into());
        }
        Ok(())
    }
    pub(super) fn optional_kind(&self, leaf: &str) -> Result<Option<bool>, String> {
        name(leaf)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        match native_fs::symlink_metadata(self.path.join(leaf)) {
            Ok(m) if m.is_dir() => Ok(Some(true)),
            Ok(m) if m.is_file() => Ok(Some(false)),
            Ok(_) => Err("管理領域に未対応の特殊ファイルがあります".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub(super) fn kind(&self, leaf: &str) -> Result<bool, String> {
        self.optional_kind(leaf)?
            .ok_or_else(|| "管理対象が消失しました".into())
    }
    pub(super) fn names(&self, limit: usize) -> Result<Vec<String>, String> {
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        let mut names = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&self.path).map_err(|e| e.to_string())? {
            let text = entry
                .map_err(|e| e.to_string())?
                .file_name()
                .into_string()
                .map_err(|_| "管理対象の名前がUTF-8ではありません")?;
            name(&text)?;
            if !names.insert(text) || names.len() > limit {
                return Err("管理対象の件数・重複が安全上限を超えています".into());
            }
        }
        self.verify()?;
        Ok(names.into_iter().collect())
    }
    pub(super) fn rename_exclusive(&self, source: &str, target: &str) -> Result<(), String> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
        };
        name(source)?;
        name(target)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        let file = OpenOptions::new()
            .access_mode(0x10000 | 0x80)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(self.path.join(source))
            .map_err(|e| e.to_string())?;
        win::identity(&file)?;
        let target: Vec<u16> = self
            .path
            .join(target)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Keep an explicit terminator even when the payload ends exactly at
        // the allocation boundary. FileNameLength excludes the terminator.
        let bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + target.len() * 2;
        let mut buffer = vec![0u64; bytes.div_ceil(8)];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = ((target.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(
                target.as_ptr(),
                (*info).FileName.as_mut_ptr(),
                target.len(),
            );
            if SetFileInformationByHandle(
                file.as_raw_handle(),
                FileRenameInfo,
                info.cast(),
                bytes as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        self.verify()
    }
    pub(super) fn unlink_staged(
        &self,
        leaf: &str,
        directory: bool,
        expected: &Proof,
    ) -> Result<(), String> {
        if !leaf.starts_with(".mon-delete-") {
            return Err("削除用予約名ではありません".into());
        }
        name(leaf)?;
        self.verify()?;
        let now = native_fs::symlink_metadata(self.path.join(leaf)).map_err(|e| e.to_string())?;
        if Proof::read(&now)? != *expected {
            return Err("削除直前に予約対象が変更されています".into());
        }
        win::remove_owned(
            &self.path.join(leaf),
            &expected.identity.native(),
            directory,
        )?;
        self.verify()
    }
    pub(super) fn remove_reserved_tree(
        &self,
        leaf: &str,
        expected: &Identity,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if !leaf.starts_with(".mon-delete-") {
            return Err("削除用予約名ではありません".into());
        }
        name(leaf)?;
        let _pin = win::directory(&self.path)?;
        self.verify()?;
        let path = self.path.join(leaf);
        self.child(leaf)?
            .ok_or("削除対象がありません")?
            .verify_identity(expected)?;
        remove_tree_handle(&path, Some(expected), self.identity.device, check, 0)?;
        self.sync()
    }
    pub(super) fn sync(&self) -> Result<(), String> {
        self.verify()?;
        win::sync_directory(&self.path)
    }
}

// Hold each directory against rename while traversing. Reparse points are
// opened themselves, never traversed, and deleted through that exact handle.
fn remove_tree_handle(
    path: &Path,
    expected: Option<&Identity>,
    device: u64,
    check: &impl Fn() -> bool,
    depth: usize,
) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfoEx, GetFileInformationByHandle, SetFileInformationByHandle,
        BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO_EX,
    };
    if check() {
        return Err("フォルダー削除を停止しました".into());
    }
    if depth > 128 {
        return Err("削除対象のフォルダーが深すぎます".into());
    }
    let file = OpenOptions::new()
        // native_fs::metadata also reads owner/DACL while verifying the root
        // identity. GetSecurityInfo requires READ_CONTROL on this same handle.
        .access_mode(0x10000 | 0x80 | 0x20000)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if u64::from(info.dwVolumeSerialNumber) != device {
        return Err("別媒体のフォルダーは削除しません".into());
    }
    let reparse = info.dwFileAttributes & 0x400 != 0;
    if let Some(expected) = expected {
        if reparse
            || Identity::read(&native_fs::metadata(&file).map_err(|e| e.to_string())?)? != *expected
        {
            return Err("削除対象の実体が変更されています".into());
        }
    }
    if info.dwFileAttributes & 0x10 != 0 && !reparse {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            remove_tree_handle(
                &entry.map_err(|e| e.to_string())?.path(),
                None,
                device,
                check,
                depth + 1,
            )?;
        }
    }
    if check() {
        return Err("フォルダー削除を停止しました".into());
    }
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: 0x1 | 0x2 | 0x10,
    };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfoEx,
            &disposition as *const _ as *const _,
            std::mem::size_of_val(&disposition) as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

// Captured when Storage opens, not when a destructive review starts. A later
// replacement cannot be mistaken for the inode used by the cached SQLite DB.
pub(in crate::storage) struct RootAnchor {
    root: Directory,
    database: File,
    database_identity: Identity,
    database_name: &'static str,
    controls: Vec<(String, Option<(File, Identity)>)>,
    #[cfg(any(target_os = "macos", windows))]
    inspection_history: Option<std::sync::Arc<super::relocation::receiver::Aliases>>,
    #[cfg(any(target_os = "macos", windows))]
    startup_binding: Option<((u64, u64, u64), (u64, u64, u64))>,
}
impl RootAnchor {
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn with_startup_binding(
        mut self,
        binding: ((u64, u64, u64), (u64, u64, u64)),
    ) -> Self {
        self.startup_binding = Some(binding);
        self
    }
    // Historical read/reconfirmation only. Current identity(), file proofs,
    // removal writers and unlink checks continue to compare every field.
    #[cfg(any(target_os = "macos", windows))]
    fn matches_startup_binding(
        &self,
        root: &Identity,
        database: &Identity,
        current: &(Identity, Identity),
    ) -> Result<bool, String> {
        let Some((old_root, old_database)) = self.startup_binding else {
            return Ok(false);
        };
        let r = root.registered()?;
        let d = database.registered()?;
        Ok((r.1, r.2) == (old_root.1, old_root.2)
            && (d.1, d.2) == (old_database.1, old_database.2)
            && root.mode == current.0.mode
            && database.mode == current.1.mode)
    }
    #[cfg(any(target_os = "macos", windows))]
    fn startup_copy(&self) -> bool {
        self.startup_binding.is_some_and(|(root, database)| {
            self.root
                .identity
                .registered()
                .is_ok_and(|now| (now.1, now.2) != (root.1, root.2))
                || self
                    .database_identity
                    .registered()
                    .is_ok_and(|now| (now.1, now.2) != (database.1, database.2))
        })
    }
    pub(super) fn verify_startup_history_directory(
        &self,
        directory: &Directory,
        expected: &Identity,
    ) -> Result<(), String> {
        self.verify()?;
        if directory.verify_identity(expected).is_ok() {
            return Ok(());
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.startup_binding.is_some()
            && (same_persistent(&directory.identity, expected) || self.startup_copy())
        {
            return directory.verify();
        }
        Err("履歴の保存先フォルダーの実体が一致しません".into())
    }
    /// Read-only backup traversal starts from the SAME root inode as SQLite,
    /// never a newly opened path that could have been substituted meanwhile.
    pub(in crate::storage) fn backup_input_root(&self) -> Result<File, String> {
        self.verify()?;
        self.root.file.try_clone().map_err(|e| e.to_string())
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn verify_sqlite_header(&self) -> Result<(), String> {
        self.verify()?;
        let mut header = [0_u8; 100];
        crate::native_fs::read_exact_at(&self.database, &mut header, 0)
            .map_err(|e| format!("登録済みDBのヘッダーを読めません。新しいDBにしません: {e}"))?;
        if &header[..16] != b"SQLite format 3\0"
            || !matches!(header[18], 1 | 2)
            || !matches!(header[19], 1 | 2)
        {
            return Err("登録済みDBのヘッダーが不正です。自動初期化しません".into());
        }
        self.verify()
    }
    pub(in crate::storage) fn directory(path: &Path) -> Result<Directory, String> {
        let root = Directory::root(path)?;
        // Reject aliases before SQLite could follow them and change an
        // external DB or journal while initializing its WAL settings.
        for name in [
            "meetings.sqlite3",
            "meetings.sqlite3-wal",
            "meetings.sqlite3-shm",
            "meetings.sqlite3-journal",
        ] {
            root.optional_regular(name)?;
        }
        Ok(root)
    }
    pub(in crate::storage) fn bind(root: Directory) -> Result<Self, String> {
        Self::bind_leaf(root, "meetings.sqlite3")
    }
    fn bind_leaf(root: Directory, database_name: &'static str) -> Result<Self, String> {
        let database = root.regular(database_name)?;
        let database_identity =
            Identity::read(&native_fs::metadata(&database).map_err(|e| e.to_string())?)?;
        Ok(Self {
            root,
            database,
            database_identity,
            database_name,
            controls: vec![],
            #[cfg(any(target_os = "macos", windows))]
            inspection_history: None,
            #[cfg(any(target_os = "macos", windows))]
            startup_binding: None,
        })
    }
    pub(in crate::storage) fn seal_controls(&mut self) -> Result<(), String> {
        self.root.verify_file(self.database_name, &self.database)?;
        if !self.controls.is_empty() {
            return Err("DB制御ファイルの実体は再採用できません".into());
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let name = format!("{}{suffix}", self.database_name);
            let opened = self.root.optional_regular(&name)?;
            let identity = opened
                .map(|file| {
                    let identity =
                        Identity::read(&native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
                    Ok::<_, String>((file, identity))
                })
                .transpose()?;
            self.controls.push((name, identity));
        }
        self.verify()
    }
    #[cfg(test)]
    pub(in crate::storage) fn owned_image(
        path: &Path,
        database_name: &'static str,
    ) -> Result<Self, String> {
        let mut anchor = Self::bind_leaf(
            Directory::root(&path.canonicalize().map_err(|e| e.to_string())?)?,
            database_name,
        )?;
        anchor.seal_controls()?;
        Ok(anchor)
    }
    pub(in crate::storage) fn verify(&self) -> Result<(), String> {
        self.root.verify_file(self.database_name, &self.database)?;
        if Identity::read(&native_fs::metadata(&self.database).map_err(|e| e.to_string())?)?
            != self.database_identity
        {
            return Err("起動時のDB実体が変更されています".into());
        }
        for (name, expected) in &self.controls {
            // Opening and then closing another descriptor for SQLite's -shm
            // file releases this process's POSIX locks, including SQLite's
            // dead-man-switch. Verify the retained descriptor and path using
            // metadata only; never reopen a live SQLite control file here.
            match expected {
                Some((held, identity)) => {
                    self.root.verify_file(name, held)?;
                    if Identity::read(&native_fs::metadata(&held).map_err(|e| e.to_string())?)?
                        != *identity
                    {
                        return Err("起動時のDB制御ファイルが変更されています".into());
                    }
                }
                None if self.root.optional_kind(name)?.is_none() => {}
                None => return Err("起動時のDB制御ファイルの有無が変更されています".into()),
            }
        }
        Ok(())
    }
    pub(super) fn identity(&self) -> Result<(Identity, Identity), String> {
        self.verify()?;
        Ok((self.root.identity.clone(), self.database_identity.clone()))
    }
    // Read-only history checks only. identity(), file proofs, current census,
    // writer admission and unlink verification deliberately remain unchanged.
    pub(super) fn matches_history_root(
        &self,
        db: &rusqlite::Connection,
        operation: &str,
        root: &Identity,
        database: &Identity,
    ) -> Result<bool, String> {
        let current = self.identity()?;
        if current == (root.clone(), database.clone()) {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.matches_startup_binding(root, database, &current)? {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = &self.inspection_history {
            return history.matches_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
    }
    pub(super) fn verify_current_history(&self) -> Result<(), String> {
        self.verify()?;
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = &self.inspection_history {
            return history.verify_current_history(&self.identity()?);
        }
        Ok(())
    }
    pub(super) fn verify_history_directory(
        &self,
        db: &rusqlite::Connection,
        operation: &str,
        relative_path: &str,
        directory: &Directory,
        expected: &Identity,
    ) -> Result<(), String> {
        self.verify()?;
        if directory.verify_identity(expected).is_ok() {
            return Ok(());
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.startup_binding.is_some()
            && (same_persistent(&directory.identity, expected)
                || (self.startup_copy() && directory.path == self.root.path.join(relative_path)))
        {
            return directory.verify();
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = &self.inspection_history {
            if let Some(copied) = history.directory(db, operation, relative_path, expected)? {
                return directory.verify_identity(copied);
            }
        }
        let _ = (db, operation, relative_path);
        Err("履歴の親フォルダーを別の実体として採用しません".into())
    }
    // Read allowance for an explicitly new census only. identity(), writers
    // and unlink still require the actual current root/database identities.
    pub(super) fn matches_reconfirmation_root(
        &self,
        db: &rusqlite::Connection,
        operation: &str,
        root: &Identity,
        database: &Identity,
    ) -> Result<bool, String> {
        let current = self.identity()?;
        if current == (root.clone(), database.clone()) {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.matches_startup_binding(root, database, &current)? {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = &self.inspection_history {
            return history.matches_reconfirmation_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
    }
    // Only a typed completed model source can capture a NEW review. This is
    // never consulted by the removal writer or an unlink verification.
    pub(super) fn matches_model_reconfirmation_root(
        &self,
        db: &rusqlite::Connection,
        operation: &str,
        root: &Identity,
        database: &Identity,
    ) -> Result<bool, String> {
        let current = self.identity()?;
        if current == (root.clone(), database.clone()) {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.matches_startup_binding(root, database, &current)? {
            return Ok(true);
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(history) = &self.inspection_history {
            return history
                .matches_model_reconfirmation_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
    }
    // Source-side inheritance is derived from this live admitted anchor and
    // the held native snapshot's rows; saved JSON cannot construct it.
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn relocation_record_roots(
        &self,
        db: &rusqlite::Connection,
        operation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<super::relocation::HistoricalRoot>, String> {
        let current = self.identity()?;
        if let Some(history) = &self.inspection_history {
            history.verify_anchor(&self.root.path, &current)?;
            return history.record_roots(db, operation, &current, cancelled);
        }
        Ok(Vec::new())
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn relocation_directory_history(
        &self,
        db: &rusqlite::Connection,
        operations: &[&str],
        actual: &[super::relocation::Directory],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<super::relocation::HistoricalDirectory>, String> {
        let current = self.identity()?;
        if let Some(history) = &self.inspection_history {
            history.verify_anchor(&self.root.path, &current)?;
            return history.directory_history(db, operations, actual, &current, cancelled);
        }
        Ok(Vec::new())
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn with_inspection_history(
        mut self,
        history: std::sync::Arc<super::relocation::receiver::Aliases>,
    ) -> Result<Self, String> {
        history.verify_anchor(&self.root.path, &self.identity()?)?;
        if self.inspection_history.is_some() {
            return Err("起動検査の履歴対応を再採用できません".into());
        }
        self.inspection_history = Some(history);
        Ok(self)
    }
    pub(super) fn directory_ref(&self) -> Result<&Directory, String> {
        self.verify()?;
        Ok(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_confirmation_paths_keep_exact_names_at_every_buffer_alignment() {
        let root = tempfile::tempdir().unwrap();
        for padding in 0..8 {
            let parent = root.path().join(format!("日本語{}", "x".repeat(padding)));
            std::fs::create_dir(&parent).unwrap();
            let parent = parent.canonicalize().unwrap();
            let directory = Directory::root(&parent).unwrap();
            std::fs::write(parent.join("source.flac"), b"owned rename fixture").unwrap();
            let target = format!(".mon-delete-{}-r1-0", uuid::Uuid::new_v4());
            directory.rename_exclusive("source.flac", &target).unwrap();
            assert!(!parent.join("source.flac").exists());
            assert_eq!(directory.names(10).unwrap(), vec![target.clone()]);
            assert_eq!(
                std::fs::read(parent.join(&target)).unwrap(),
                b"owned rename fixture"
            );
            std::fs::write(parent.join("source.flac"), b"keep source").unwrap();
            assert!(directory.rename_exclusive("source.flac", &target).is_err());
            assert_eq!(
                std::fs::read(parent.join("source.flac")).unwrap(),
                b"keep source"
            );
            assert_eq!(
                std::fs::read(parent.join(&target)).unwrap(),
                b"owned rename fixture"
            );
        }
    }
}
