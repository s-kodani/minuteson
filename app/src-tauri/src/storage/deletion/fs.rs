//! Existing-object, descriptor-relative reads. No creation, unlink or adoption.
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::fs::{File, Metadata};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

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
    #[cfg(target_os = "macos")]
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
        let kind = if directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if self.birth_nanos >= 1_000_000_000 || self.mode & libc::S_IFMT as u32 != kind as u32 {
            return Err("管理対象の実体記録の形式が不正です".into());
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
    pub(super) fn matches_after_rename(&self, original: &Self) -> bool {
        self.identity == original.identity
            && self.size == original.size
            && self.links == original.links
            && self.modified_seconds == original.modified_seconds
            && self.modified_nanos == original.modified_nanos
    }
    pub(super) fn validate(&self, directory: bool) -> Result<(), String> {
        self.identity.validate(directory)?;
        if self.links == 0
            || (!directory && self.links != 1)
            || !(0..1_000_000_000).contains(&self.modified_nanos)
            || !(0..1_000_000_000).contains(&self.change_nanos)
            || self.size > i64::MAX as u64
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

fn name(s: &str) -> Result<CString, String> {
    if s.is_empty() || s.len() > 255 || matches!(s, "." | "..") || s.contains(['/', '\\']) {
        return Err("管理パスの要素が不正です".into());
    }
    CString::new(s).map_err(|e| e.to_string())
}
fn file(fd: i32) -> Result<File, String> {
    if fd < 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

pub(in crate::storage) struct Directory {
    file: File,
    path: PathBuf,
    pub(super) identity: Identity,
}
impl Directory {
    pub(super) fn verify_identity(&self, expected: &Identity) -> Result<(), String> {
        self.verify()?;
        if &self.identity != expected {
            return Err("確認したフォルダーの実体が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn rename_exclusive(&self, source: &str, target: &str) -> Result<(), String> {
        self.verify()?;
        let source = name(source)?;
        let target = name(target)?;
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.file.as_raw_fd(),
                source.as_ptr(),
                self.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.file.as_raw_fd(),
                source.as_ptr(),
                self.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
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
    // Only the deterministic reserved name of an armed/staged receipt is
    // removed. Normal target names are NEVER directly passed to unlinkat.
    pub(super) fn unlink_staged(
        &self,
        leaf: &str,
        directory: bool,
        expected: &Proof,
    ) -> Result<(), String> {
        self.verify()?;
        if !leaf.starts_with(".mon-delete-") {
            return Err("削除用予約名ではありません".into());
        }
        let current = std::fs::symlink_metadata(self.path.join(leaf)).map_err(|e| e.to_string())?;
        if Proof::read(&current)? != *expected {
            return Err("削除直前に予約対象が変更されています".into());
        }
        let leaf = name(leaf)?;
        let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), leaf.as_ptr(), flags) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.file.sync_all().map_err(|e| e.to_string())?;
        self.verify()
    }
    pub(super) fn sync(&self) -> Result<(), String> {
        self.verify()?;
        self.file.sync_all().map_err(|e| e.to_string())
    }
    pub(super) fn root(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("管理領域の絶対パスまたは親が不正です".into());
        }
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let opened = file(unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        })?;
        Self::from_file(opened, path.into(), None)
    }
    fn from_file(file: File, path: PathBuf, device: Option<u64>) -> Result<Self, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_dir() || device.is_some_and(|d| d != m.dev()) {
            return Err("管理領域外のフォルダーまたは別媒体を参照しています".into());
        }
        let result = Self {
            file,
            path,
            identity: Identity::read(&m)?,
        };
        result.verify()?;
        Ok(result)
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        let current = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        let opened = self.file.metadata().map_err(|e| e.to_string())?;
        if !current.is_dir()
            || !opened.is_dir()
            || Identity::read(&current)? != self.identity
            || Identity::read(&opened)? != self.identity
            || self.path.canonicalize().map_err(|e| e.to_string())? != self.path
        {
            return Err("管理領域の実体・権限・親フォルダーが変更されています".into());
        }
        Ok(())
    }
    pub(super) fn proof(&self) -> Result<Proof, String> {
        self.verify()?;
        Proof::read(&self.file.metadata().map_err(|e| e.to_string())?)
    }
    pub(super) fn child(&self, leaf: &str) -> Result<Option<Self>, String> {
        self.verify()?;
        let c = name(leaf)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let result = Self::from_file(file(fd)?, self.path.join(leaf), Some(self.identity.device))?;
        self.verify()?;
        Ok(Some(result))
    }
    pub(super) fn regular(&self, leaf: &str) -> Result<File, String> {
        self.optional_regular(leaf)?
            .ok_or_else(|| "管理ファイルが見つかりません".into())
    }
    fn optional_regular(&self, leaf: &str) -> Result<Option<File>, String> {
        self.verify()?;
        let c = name(leaf)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let opened = file(fd)?;
        let m = opened.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.nlink() != 1 || m.dev() != self.identity.device {
            return Err("管理対象は別媒体・共有リンクのない通常ファイルに限ります".into());
        }
        self.verify_file(leaf, &opened)?;
        Ok(Some(opened))
    }
    pub(super) fn verify_file(&self, leaf: &str, opened: &File) -> Result<(), String> {
        self.verify()?;
        let current = std::fs::symlink_metadata(self.path.join(leaf)).map_err(|e| e.to_string())?;
        let m = opened.metadata().map_err(|e| e.to_string())?;
        if !current.is_file()
            || current.nlink() != 1
            || !m.is_file()
            || m.nlink() != 1
            || Identity::read(&current)? != Identity::read(&m)?
            || self
                .path
                .join(leaf)
                .canonicalize()
                .map_err(|e| e.to_string())?
                != self.path.join(leaf)
        {
            return Err("管理ファイルの実体・親・共有リンクが変更されています".into());
        }
        Ok(())
    }
    pub(super) fn kind(&self, leaf: &str) -> Result<bool, String> {
        self.optional_kind(leaf)?
            .ok_or_else(|| "管理対象が消失しました".into())
    }
    pub(super) fn optional_kind(&self, leaf: &str) -> Result<Option<bool>, String> {
        self.verify()?;
        let c = name(leaf)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.file.as_raw_fd(),
                c.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(std::io::Error::last_os_error().to_string());
        }
        let m = unsafe { stat.assume_init() };
        match m.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Ok(Some(true)),
            libc::S_IFREG => Ok(Some(false)),
            _ => Err("管理領域にリンクまたは未対応の特殊ファイルがあります".into()),
        }
    }
    pub(super) fn names(&self, limit: usize) -> Result<Vec<String>, String> {
        self.verify()?;
        // A new open file description is important: dup would share a previous
        // directory enumeration offset and could silently omit entries.
        let dot = CString::new(".").unwrap();
        let opened = file(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                dot.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        use std::os::fd::IntoRawFd;
        let fd = opened.into_raw_fd();
        let dir = unsafe { libc::fdopendir(fd) };
        if dir.is_null() {
            let error = std::io::Error::last_os_error().to_string();
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }
        struct Close(*mut libc::DIR);
        impl Drop for Close {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let close = Close(dir);
        let mut names = std::collections::BTreeSet::new();
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(close.0) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                break;
            }
            let s = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| "管理対象の名前がUTF-8ではありません")?;
            if matches!(s, "." | "..") {
                continue;
            }
            name(s)?;
            if !names.insert(s.to_owned()) {
                return Err("管理対象の列挙が重複しています".into());
            }
            if names.len() > limit {
                return Err("管理対象の件数が安全上限を超えています".into());
            }
        }
        self.verify()?;
        Ok(names.into_iter().collect())
    }
}

// Captured when Storage opens, not when a destructive review starts. A later
// replacement cannot be mistaken for the inode used by the cached SQLite DB.
pub(in crate::storage) struct RootAnchor {
    root: Directory,
    database: File,
    database_identity: Identity,
    database_name: &'static str,
    controls: Vec<(String, Option<(File, Identity)>)>,
    #[cfg(target_os = "macos")]
    inspection_history: Option<std::sync::Arc<super::relocation::receiver::Aliases>>,
}
impl RootAnchor {
    /// Read-only backup traversal starts from the SAME root inode as SQLite,
    /// never a newly opened path that could have been substituted meanwhile.
    pub(in crate::storage) fn backup_input_root(&self) -> Result<File, String> {
        self.verify()?;
        self.root.file.try_clone().map_err(|e| e.to_string())
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
        let database_identity = Identity::read(&database.metadata().map_err(|e| e.to_string())?)?;
        Ok(Self {
            root,
            database,
            database_identity,
            database_name,
            controls: vec![],
            #[cfg(target_os = "macos")]
            inspection_history: None,
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
                    let identity = Identity::read(&file.metadata().map_err(|e| e.to_string())?)?;
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
        if Identity::read(&self.database.metadata().map_err(|e| e.to_string())?)?
            != self.database_identity
        {
            return Err("起動時のDB実体が変更されています".into());
        }
        for (name, expected) in &self.controls {
            let current = self.root.optional_regular(name)?;
            match (expected, current) {
                (Some((held, identity)), Some(current)) => {
                    self.root.verify_file(name, held)?;
                    if Identity::read(&current.metadata().map_err(|e| e.to_string())?)? != *identity
                        || Identity::read(&held.metadata().map_err(|e| e.to_string())?)?
                            != *identity
                    {
                        return Err("起動時のDB制御ファイルが変更されています".into());
                    }
                }
                (None, None) => {}
                _ => return Err("起動時のDB制御ファイルの有無が変更されています".into()),
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
        #[cfg(target_os = "macos")]
        if let Some(history) = &self.inspection_history {
            return history.matches_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
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
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
        if let Some(history) = &self.inspection_history {
            return history
                .matches_model_reconfirmation_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
    }
    // Source-side inheritance is derived from this live admitted anchor and
    // the held native snapshot's rows; saved JSON cannot construct it.
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
