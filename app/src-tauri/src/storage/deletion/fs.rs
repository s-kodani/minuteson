//! Existing-object, descriptor-relative reads. No creation, unlink or adoption.
pub(super) use mon_native::identity::{Identity, Proof};
use std::ffi::{CStr, CString};
use std::fs::{File, Metadata};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[cfg(all(test, unix))]
mod sqlite_lock_tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn startup_binding_reads_old_device_history_without_changing_current_proofs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("synthetic.db"), b"synthetic").unwrap();
        let anchor = RootAnchor::owned_image(temp.path(), "synthetic.db").unwrap();
        let (root, database) = anchor.identity().unwrap();
        let mut old_root = root.clone();
        old_root.device += 100;
        let mut old_database = database.clone();
        old_database.device += 100;
        let db = rusqlite::Connection::open_in_memory().unwrap();
        assert!(!anchor
            .matches_history_root(&db, "test", &old_root, &old_database)
            .unwrap());
        let anchor = anchor.with_startup_binding((
            old_root.registered().unwrap(),
            old_database.registered().unwrap(),
        ));
        assert!(anchor
            .matches_history_root(&db, "test", &old_root, &old_database)
            .unwrap());
        assert_eq!(anchor.identity().unwrap(), (root, database));
        assert!(anchor.root.verify_identity(&old_root).is_err());
        old_database.inode += 1;
        assert!(!anchor
            .matches_history_root(&db, "test", &old_root, &old_database)
            .unwrap());
        std::fs::create_dir(temp.path().join("child")).unwrap();
        let child = Directory::root(&temp.path().join("child").canonicalize().unwrap()).unwrap();
        let mut copied_child = child.identity.clone();
        copied_child.inode += 1;
        let mut old_root = old_root.registered().unwrap();
        old_root.1 += 1;
        let anchor = anchor.with_startup_binding((old_root, old_database.registered().unwrap()));
        assert!(anchor
            .verify_history_directory(&db, "test", "child", &child, &copied_child)
            .is_ok());
        assert!(anchor
            .verify_history_directory(&db, "test", "other", &child, &copied_child)
            .is_err());
        assert!(child.verify_identity(&copied_child).is_err());
    }

    #[test]
    fn control_identity_verification_does_not_release_posix_locks() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("synthetic.db"),
            b"synthetic database identity",
        )
        .unwrap();
        let shm = temp.path().join("synthetic.db-shm");
        std::fs::write(&shm, b"synthetic control identity").unwrap();
        let anchor = RootAnchor::owned_image(temp.path(), "synthetic.db").unwrap();
        let held = &anchor
            .controls
            .iter()
            .find(|(name, _)| name.ends_with("-shm"))
            .unwrap()
            .1
            .as_ref()
            .unwrap()
            .0;
        // Like SQLite's dead-man-switch, this lock must survive identity reads.
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_RDLCK;
        lock.l_whence = libc::SEEK_SET as libc::c_short;
        lock.l_start = 128;
        lock.l_len = 1;
        assert_eq!(
            unsafe { libc::fcntl(held.as_raw_fd(), libc::F_SETLK, &lock) },
            0
        );
        let probe = || {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::deletion::fs::sqlite_lock_tests::external_control_lock_fixture",
                    "--ignored",
                ])
                .env("MON_CONTROL_LOCK_TEST_FILE", &shm)
                .output()
                .unwrap();
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed")
            );
        };
        probe();
        for _ in 0..3 {
            anchor.verify().unwrap();
            probe();
        }
    }

    #[test]
    #[ignore = "subprocess-only lock observation on parent-owned synthetic control file"]
    fn external_control_lock_fixture() {
        let path = std::env::var_os("MON_CONTROL_LOCK_TEST_FILE").expect("parent-owned fixture");
        let held = File::open(path).unwrap();
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_WRLCK;
        lock.l_whence = libc::SEEK_SET as libc::c_short;
        lock.l_start = 128;
        lock.l_len = 1;
        assert_eq!(
            unsafe { libc::fcntl(held.as_raw_fd(), libc::F_GETLK, &mut lock) },
            0
        );
        assert_eq!(
            lock.l_type,
            libc::F_RDLCK,
            "identity verification released the owner's lock"
        );
    }

    #[test]
    fn metadata_only_control_checks_still_reject_replacement_links_and_new_files() {
        for mutation in ["replacement", "symlink", "hardlink", "new-control"] {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(temp.path().join("synthetic.db"), b"database").unwrap();
            let wal = temp.path().join("synthetic.db-wal");
            std::fs::write(&wal, b"original control").unwrap();
            let anchor = RootAnchor::owned_image(temp.path(), "synthetic.db").unwrap();
            anchor.verify().unwrap();
            match mutation {
                "replacement" => {
                    std::fs::rename(&wal, temp.path().join("retained.wal")).unwrap();
                    std::fs::write(&wal, b"replacement control").unwrap();
                }
                "symlink" => {
                    let retained = temp.path().join("retained.wal");
                    std::fs::rename(&wal, &retained).unwrap();
                    std::os::unix::fs::symlink(&retained, &wal).unwrap();
                }
                "hardlink" => std::fs::hard_link(&wal, temp.path().join("other.wal")).unwrap(),
                "new-control" => {
                    std::fs::write(temp.path().join("synthetic.db-journal"), b"new").unwrap()
                }
                _ => unreachable!(),
            }
            assert!(
                anchor.verify().is_err(),
                "accepted changed control: {mutation}"
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn same_persistent(a: &Identity, b: &Identity) -> bool {
    a.same_persistent(b)
}
pub(crate) trait NativeProof: Sized {
    fn read(m: &Metadata) -> Result<Self, String>;
}
impl NativeProof for Identity {
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
impl NativeProof for Proof {
    fn read(m: &Metadata) -> Result<Self, String> {
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
    pub(super) fn remove_reserved_tree(
        &self,
        leaf: &str,
        expected: &Identity,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if !leaf.starts_with(".mon-delete-") {
            return Err("削除用予約名ではありません".into());
        }
        let child = self.child(leaf)?.ok_or("削除対象がありません")?;
        child.verify_identity(expected)?;
        self.remove_tree_child_native(&name(leaf)?, &child, check, 0)?;
        self.sync()
    }
    fn remove_tree_child_native(
        &self,
        leaf: &CStr,
        child: &Self,
        check: &impl Fn() -> bool,
        depth: usize,
    ) -> Result<(), String> {
        // Native names may not be UTF-8; recurse through the same pinned path.
        // The public root name remains repository-generated ASCII.
        if depth > 128 {
            return Err("削除対象のフォルダーが深すぎます".into());
        }
        for entry in std::fs::read_dir(&child.path).map_err(|e| e.to_string())? {
            if check() {
                return Err("フォルダー削除を停止しました".into());
            }
            child.verify()?;
            let leaf_name = entry.map_err(|e| e.to_string())?.file_name();
            let raw = CString::new(leaf_name.as_bytes()).map_err(|e| e.to_string())?;
            let metadata = std::fs::symlink_metadata(child.path.join(&leaf_name))
                .map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                let opened = file(unsafe {
                    libc::openat(
                        child.file.as_raw_fd(),
                        raw.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                })?;
                let sub = Self::from_file(
                    opened,
                    child.path.join(&leaf_name),
                    Some(child.identity.device),
                )?;
                child.remove_tree_child_native(&raw, &sub, check, depth + 1)?;
            } else if unsafe { libc::unlinkat(child.file.as_raw_fd(), raw.as_ptr(), 0) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        self.remove_empty_child(leaf, child, check)
    }
    fn remove_empty_child(
        &self,
        leaf: &CStr,
        child: &Self,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if check() {
            return Err("フォルダー削除を停止しました".into());
        }
        self.verify()?;
        child.verify()?;
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
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
    #[cfg(target_os = "macos")]
    startup_binding: Option<((u64, u64, u64), (u64, u64, u64))>,
}
impl RootAnchor {
    #[cfg(target_os = "macos")]
    pub(in crate::storage) fn with_startup_binding(
        mut self,
        binding: ((u64, u64, u64), (u64, u64, u64)),
    ) -> Self {
        self.startup_binding = Some(binding);
        self
    }
    // Historical read/reconfirmation only. Current identity(), file proofs,
    // removal writers and unlink checks continue to compare every field.
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    pub(in crate::storage) fn verify_sqlite_header(&self) -> Result<(), String> {
        use std::os::unix::fs::FileExt;
        self.verify()?;
        let mut header = [0_u8; 100];
        self.database
            .read_exact_at(&mut header, 0)
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
        let database_identity = Identity::read(&database.metadata().map_err(|e| e.to_string())?)?;
        Ok(Self {
            root,
            database,
            database_identity,
            database_name,
            controls: vec![],
            #[cfg(target_os = "macos")]
            inspection_history: None,
            #[cfg(target_os = "macos")]
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
            // Opening and then closing another descriptor for SQLite's -shm
            // file releases this process's POSIX locks, including SQLite's
            // dead-man-switch. Verify the retained descriptor and path using
            // metadata only; never reopen a live SQLite control file here.
            match expected {
                Some((held, identity)) => {
                    self.root.verify_file(name, held)?;
                    if Identity::read(&held.metadata().map_err(|e| e.to_string())?)? != *identity {
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
        #[cfg(target_os = "macos")]
        if self.matches_startup_binding(root, database, &current)? {
            return Ok(true);
        }
        #[cfg(target_os = "macos")]
        if let Some(history) = &self.inspection_history {
            return history.matches_root(db, operation, root, database, &current);
        }
        let _ = (db, operation);
        Ok(false)
    }
    pub(super) fn verify_current_history(&self) -> Result<(), String> {
        self.verify()?;
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
        if self.startup_binding.is_some()
            && (same_persistent(&directory.identity, expected)
                || (self.startup_copy() && directory.path == self.root.path.join(relative_path)))
        {
            return directory.verify();
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
        if self.matches_startup_binding(root, database, &current)? {
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
        if self.matches_startup_binding(root, database, &current)? {
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
