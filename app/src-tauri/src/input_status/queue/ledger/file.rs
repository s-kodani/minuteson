//! One native delivery writer per pinned recording directory. A commit failure,
//! including uncertain post-rename sync, poisons this writer: no optimistic ACK.
#[cfg(unix)]
mod native {
    use super::super::LIMIT;
    use std::ffi::CString;
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;

    const DATA: &str = "input-notices.json";
    const LOCK: &str = "input-notices.lock";
    #[derive(PartialEq, Eq)]
    struct Stamp {
        dev: u64,
        ino: u64,
        size: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }
    fn stamp(m: &Metadata, limit: u64) -> Result<Stamp, String> {
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || m.len() > limit
        {
            return Err("通知台帳の実体・所有・権限・容量が不正です".into());
        }
        Ok(Stamp {
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    fn open(parent: &File, name: &str, create: bool) -> Result<File, std::io::Error> {
        let name = CString::new(name).unwrap();
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC
                    | if create {
                        libc::O_CREAT | libc::O_EXCL
                    } else {
                        0
                    },
                0o600,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn leaf(parent: &File) -> Result<Option<Stamp>, String> {
        match open(parent, DATA, false) {
            Ok(f) => Ok(Some(stamp(
                &f.metadata().map_err(|e| e.to_string())?,
                LIMIT,
            )?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub(in crate::input_status::queue::ledger) struct Store {
        directory: File,
        lock: File,
        lock_stamp: Stamp,
        expected: Option<Stamp>,
        failed: Option<String>,
        #[cfg(test)]
        pub(in crate::input_status::queue::ledger) fail_after_rename: bool,
        #[cfg(test)]
        pub(in crate::input_status::queue::ledger) before_commit_delay: std::time::Duration,
    }
    impl Drop for Store {
        fn drop(&mut self) {
            unsafe {
                libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
    impl Store {
        pub(in crate::input_status::queue::ledger) fn open(
            directory: &File,
        ) -> Result<(Self, Option<Vec<u8>>), String> {
            let directory = directory.try_clone().map_err(|e| e.to_string())?;
            let m = directory.metadata().map_err(|e| e.to_string())?;
            if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
                return Err("通知台帳の保存先の所有が不正です".into());
            }
            let lock = match open(&directory, LOCK, true) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    open(&directory, LOCK, false).map_err(|e| e.to_string())?
                }
                Err(e) => return Err(e.to_string()),
            };
            let lock_stamp = stamp(&lock.metadata().map_err(|e| e.to_string())?, 0)?;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("通知台帳には別の書込み所有者がいます".into());
            }
            let mut store = Self {
                directory,
                lock,
                lock_stamp,
                expected: None,
                failed: None,
                #[cfg(test)]
                fail_after_rename: false,
                #[cfg(test)]
                before_commit_delay: std::time::Duration::ZERO,
            };
            let bytes = match open(&store.directory, DATA, false) {
                Ok(f) => {
                    let before = stamp(&f.metadata().map_err(|e| e.to_string())?, LIMIT)?;
                    let mut bytes = Vec::new();
                    (&f).take(LIMIT + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    if bytes.len() as u64 != before.size
                        || stamp(&f.metadata().map_err(|e| e.to_string())?, LIMIT)? != before
                        || leaf(&store.directory)?.as_ref() != Some(&before)
                    {
                        return Err("通知台帳が読込み中に変更されました".into());
                    }
                    // Reopening is also the reconciliation boundary after an
                    // uncertain earlier sync. Never issue a receipt just by read.
                    f.sync_all().map_err(|e| e.to_string())?;
                    store.expected = Some(before);
                    Some(bytes)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            };
            store.lock.sync_all().map_err(|e| e.to_string())?;
            store.directory.sync_all().map_err(|e| e.to_string())?;
            store.verify()?;
            Ok((store, bytes))
        }
        fn verify_inner(&self) -> Result<(), String> {
            if let Some(error) = &self.failed {
                return Err(error.clone());
            }
            let live_lock = open(&self.directory, LOCK, false).map_err(|e| e.to_string())?;
            if stamp(&live_lock.metadata().map_err(|e| e.to_string())?, 0)? != self.lock_stamp
                || stamp(&self.lock.metadata().map_err(|e| e.to_string())?, 0)? != self.lock_stamp
                || leaf(&self.directory)? != self.expected
            {
                return Err("通知台帳または書込み所有の実体が変更されました".into());
            }
            Ok(())
        }
        pub(in crate::input_status::queue::ledger) fn verify(&mut self) -> Result<(), String> {
            let result = self.verify_inner();
            if let Err(error) = &result {
                self.failed = Some(error.clone());
            }
            result
        }
        pub(in crate::input_status::queue::ledger) fn commit(
            &mut self,
            bytes: &[u8],
        ) -> Result<(), String> {
            self.verify()?;
            #[cfg(test)]
            std::thread::sleep(self.before_commit_delay);
            if bytes.is_empty() || bytes.len() as u64 > LIMIT {
                return Err("通知台帳の保存容量が不正です".into());
            }
            let name = format!(".input-notices-{}.tmp", uuid::Uuid::new_v4());
            let mut staging = open(&self.directory, &name, true).map_err(|e| e.to_string())?;
            let identity = staging.metadata().map_err(|e| e.to_string())?;
            let name_c = CString::new(name.as_str()).unwrap();
            let data_c = CString::new(DATA).unwrap();
            let result = (|| {
                staging.write_all(bytes).map_err(|e| e.to_string())?;
                staging.sync_all().map_err(|e| e.to_string())?;
                let written = stamp(&staging.metadata().map_err(|e| e.to_string())?, LIMIT)?;
                self.verify_inner()?;
                let at_path = open(&self.directory, &name, false).map_err(|e| e.to_string())?;
                if stamp(&at_path.metadata().map_err(|e| e.to_string())?, LIMIT)? != written {
                    return Err("通知台帳の一時実体が変更されました".into());
                }
                let dir = self.directory.as_raw_fd();
                if self.expected.is_none() {
                    // Atomic no-replace initial publication. Never adopt a
                    // newly appeared file merely because its name is expected.
                    if unsafe { libc::linkat(dir, name_c.as_ptr(), dir, data_c.as_ptr(), 0) } != 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    if unsafe { libc::unlinkat(dir, name_c.as_ptr(), 0) } != 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                } else if unsafe { libc::renameat(dir, name_c.as_ptr(), dir, data_c.as_ptr()) } != 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let committed = leaf(&self.directory)?.ok_or("通知台帳の保存先がありません")?;
                if committed.dev != written.dev
                    || committed.ino != written.ino
                    || committed.size != written.size
                    || committed.modified != written.modified
                {
                    return Err("通知台帳の確定実体が一致しません".into());
                }
                self.expected = Some(committed);
                #[cfg(test)]
                if self.fail_after_rename {
                    return Err("synthetic post-rename sync failure".into());
                }
                self.directory.sync_all().map_err(|e| e.to_string())?;
                self.verify_inner()
            })();
            if let Err(error) = &result {
                // Keep uncertain final data; remove only this attempt's still
                // identified staging name. Never scan leftover temporary files.
                if open(&self.directory, &name, false).is_ok_and(|f| {
                    f.metadata()
                        .is_ok_and(|m| m.dev() == identity.dev() && m.ino() == identity.ino())
                }) {
                    unsafe {
                        libc::unlinkat(self.directory.as_raw_fd(), name_c.as_ptr(), 0);
                    }
                }
                self.failed = Some(format!(
                    "通知台帳の保存が未確定です。再起動後に確認してください: {error}"
                ));
                return Err(self.failed.clone().unwrap());
            }
            Ok(())
        }
    }
}
#[cfg(unix)]
pub(super) use native::Store;

#[cfg(not(unix))]
pub(super) struct Store;
#[cfg(not(unix))]
impl Store {
    pub(super) fn open(_: &std::fs::File) -> Result<(Self, Option<Vec<u8>>), String> {
        Err("このOSの通知台帳は未対応です".into())
    }
    pub(super) fn verify(&mut self) -> Result<(), String> {
        Err("このOSの通知台帳は未対応です".into())
    }
    pub(super) fn commit(&mut self, _: &[u8]) -> Result<(), String> {
        Err("このOSの通知台帳は未対応です".into())
    }
}
