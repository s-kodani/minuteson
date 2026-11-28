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
    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn matches_relocation_history(&self, current: &Self) -> bool {
        mon_native::identity::matches_persistent_identity(
            (self.device, self.inode, self.birth),
            (current.device, current.inode, current.birth),
            cfg!(target_os = "macos"),
        ) && self.uid == current.uid
            && self.mode == current.mode
    }

    #[cfg(any(target_os = "macos", windows))]
    pub(in crate::storage) fn rebase_relocation_device(
        &mut self,
        saved_root: &Self,
        current_root: &Self,
    ) {
        // 配置台帳が証明した同一ボリュームのコピーにだけ、受付時の番号を使う。
        if cfg!(target_os = "macos") && self.device == saved_root.device {
            self.device = current_root.device;
        }
    }

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

#[cfg(all(test, any(target_os = "macos", windows)))]
#[test]
fn relocation_history_identity_preserves_inode_birth_owner_and_mode() {
    let current = Identity {
        device: 1,
        inode: 2,
        uid: 3,
        mode: 0o040700,
        birth: (4, 5),
    };
    let mut saved = Identity {
        device: 6,
        ..current.clone()
    };
    assert_eq!(
        saved.matches_relocation_history(&current),
        cfg!(target_os = "macos")
    );
    assert_ne!(saved, current);
    for changed in [
        Identity {
            inode: 7,
            ..current.clone()
        },
        Identity {
            birth: (4, 8),
            ..current.clone()
        },
        Identity {
            uid: 9,
            ..current.clone()
        },
        Identity {
            mode: 0o040755,
            ..current.clone()
        },
    ] {
        assert!(!saved.matches_relocation_history(&changed));
    }
    let saved_root = saved.clone();
    saved.rebase_relocation_device(&saved_root, &current);
    assert_eq!(saved == current, cfg!(target_os = "macos"));

    let mut other_volume = Identity {
        device: 10,
        ..current.clone()
    };
    let unchanged = other_volume.clone();
    other_volume.rebase_relocation_device(&saved_root, &current);
    assert_eq!(other_volume, unchanged);
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

/// Read-only evidence for the fixed schema4 native manifest. Older macOS
/// publishers created this file with mode 0644 in an owned recording directory.
/// It grants no write, rename or deletion capability and is never used for a
/// new WAV, control record or canonical publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::storage) struct LegacyNativeManifestStamp(Stamp);
impl LegacyNativeManifestStamp {
    fn read(file: &File) -> Result<Self, String> {
        let m = crate::native_fs::metadata(file).map_err(|error| error.to_string())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != crate::native_fs::current_owner()?
            || m.mode() & 0o022 != 0
        {
            return Err("旧録音manifestの実体・所有・書込み権限が不正です".into());
        }
        Ok(Self(Stamp {
            identity: Identity::read(&m)?,
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }))
    }
    fn require_file(&self, file: &File) -> Result<(), String> {
        if *self != Self::read(file)? {
            return Err("旧録音manifestが検証後に変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage) fn require(&self, directory: &Directory) -> Result<(), String> {
        let (file, _) = directory
            .legacy_native_manifest_file()?
            .ok_or("旧録音manifestが失われています")?;
        self.require_file(&file)?;
        directory.verify()
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
    fn legacy_native_manifest_file(
        &self,
    ) -> Result<Option<(File, LegacyNativeManifestStamp)>, String> {
        self.verify()?;
        // Native publishers historically created the leaf directory under
        // the admitted storage root with umask permissions (often 0755).
        // Read access is compatible; another owner's write access is not.
        if self.identity.mode & 0o022 != 0 {
            return Err("旧録音manifestの管理先を書換え可能な権限があります".into());
        }
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name("manifest.json")?.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let file = descriptor(fd)?;
        let stamp = LegacyNativeManifestStamp::read(&file)?;
        Ok(Some((file, stamp)))
    }
    pub(in crate::storage) fn read_legacy_native_manifest(
        &self,
    ) -> Result<Option<(Vec<u8>, LegacyNativeManifestStamp)>, String> {
        let Some((mut file, stamp)) = self.legacy_native_manifest_file()? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take(crate::capture_manifest::MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        crate::capture_manifest::validate_size(&bytes)?;
        if bytes.len() as u64 != stamp.0.size {
            return Err("旧録音manifestが読取り中に変更されました".into());
        }
        #[derive(Deserialize)]
        struct Version {
            #[serde(rename = "schemaVersion")]
            version: u32,
        }
        let version: Version = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if version.version != crate::current_formats::LEGACY_CAPTURE {
            return Err("読み取り専用の旧録音manifestではありません".into());
        }
        stamp.require_file(&file)?;
        stamp.require(self)?;
        Ok(Some((bytes, stamp)))
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
    pub(in crate::storage) fn remove_empty_child(
        &self,
        leaf: &str,
        expected: &Identity,
    ) -> Result<bool, String> {
        self.verify()?;
        let child = Self::open(&self.path.join(leaf))?;
        if child.identity != *expected {
            return Err("登録音声のフォルダーが差し替えられています".into());
        }
        #[cfg(unix)]
        if unsafe {
            libc::unlinkat(
                self.file.as_raw_fd(),
                name(leaf)?.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        } != 0
        {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                return Ok(false);
            }
            return Err(error.to_string());
        }
        #[cfg(windows)]
        {
            if std::fs::read_dir(&child.path)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
            {
                return Ok(false);
            }
            let native = crate::windows_files::Identity::file(&child.file)?;
            let path = child.path.clone();
            drop(child);
            crate::windows_files::remove_owned(&path, &native, true)?;
        }
        crate::native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        Ok(true)
    }
    pub(in crate::storage) fn remove_owned(
        &self,
        leaf: &str,
        identity: &Identity,
    ) -> Result<(), String> {
        self.verify()?;
        identity.require(&self.required(leaf, false)?)?;
        #[cfg(unix)]
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), name(leaf)?.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        #[cfg(windows)]
        crate::windows_files::remove_owned(
            &self.path.join(leaf),
            &crate::windows_files::Identity {
                device: identity.device,
                inode: identity.inode,
                birth_seconds: identity.birth.0,
                birth_nanos: identity.birth.1,
            },
            false,
        )?;
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
        self.json_bytes(leaf, &bytes, previous)
    }

    /// Capture maps have a separate, explicitly bounded decoding contract.
    /// Ordinary receipts retain the smaller limit above.
    pub(in crate::storage) fn capture_json(
        &self,
        leaf: &str,
        value: &impl Serialize,
        previous: Option<&Stamp>,
    ) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        crate::capture_manifest::validate_size(&bytes)?;
        self.json_bytes(leaf, &bytes, previous)
    }

    fn json_bytes(&self, leaf: &str, bytes: &[u8], previous: Option<&Stamp>) -> Result<(), String> {
        let temporary = format!(".{}.json.part", uuid::Uuid::new_v4());
        let mut file = self.create(&temporary)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
        if let Some(stamp) = previous {
            stamp.require(&self.required(leaf, false)?)?;
        }
        self.rename(&temporary, leaf, previous.is_none())
    }
}

#[cfg(all(test, unix))]
mod legacy_manifest_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn write(path: &Path, bytes: &[u8], mode: u32) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn legacy644_is_readonly_while_new_publication_guards_stay_private() {
        let root = tempfile::tempdir_in("/private/tmp").unwrap();
        let directory = Directory::open(root.path()).unwrap();
        let bytes = br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable","state":"saved"}"#;
        write(&root.path().join("manifest.json"), bytes, 0o644);
        let (actual, stamp) = directory.read_legacy_native_manifest().unwrap().unwrap();
        assert_eq!(actual, bytes);
        stamp.require(&directory).unwrap();
        assert!(directory.required("manifest.json", false).is_err());
        assert!(Stamp::read(&File::open(root.path().join("manifest.json")).unwrap()).is_err());
        for leaf in [
            "single-wav.json",
            "legacy-canonical-wav.json",
            "recording.wav",
        ] {
            write(&root.path().join(leaf), bytes, 0o644);
            assert!(directory.required(leaf, false).is_err(), "{leaf}");
        }
        directory
            .capture_json(
                "new-canonical.json",
                &serde_json::json!({"schemaVersion":5,"inputDiagnosticsState":"unavailable"}),
                None,
            )
            .unwrap();
        directory.required("new-canonical.json", false).unwrap();
        assert_eq!(
            std::fs::metadata(root.path().join("manifest.json"))
                .unwrap()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn legacy_manifest_rejects_writers_links_other_formats_and_writable_parent() {
        for mode in [0o664, 0o646, 0o666] {
            let root = tempfile::tempdir_in("/private/tmp").unwrap();
            write(
                &root.path().join("manifest.json"),
                br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable"}"#,
                mode,
            );
            assert!(Directory::open(root.path())
                .unwrap()
                .read_legacy_native_manifest()
                .is_err());
        }
        for bytes in [
            br#"{"schemaVersion":5,"inputDiagnosticsState":"unavailable"}"#.as_slice(),
            br#"{"schemaVersion":3,"inputDiagnosticsState":"unavailable"}"#,
            br#"{"schemaVersion":4,"schemaVersion":4,"inputDiagnosticsState":"unavailable"}"#,
        ] {
            let root = tempfile::tempdir_in("/private/tmp").unwrap();
            write(&root.path().join("manifest.json"), bytes, 0o644);
            assert!(Directory::open(root.path())
                .unwrap()
                .read_legacy_native_manifest()
                .is_err());
        }
        let root = tempfile::tempdir_in("/private/tmp").unwrap();
        let path = root.path().join("manifest.json");
        write(
            &path,
            br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable"}"#,
            0o644,
        );
        let directory = Directory::open(root.path()).unwrap();
        std::fs::hard_link(&path, root.path().join("other.json")).unwrap();
        assert!(directory.read_legacy_native_manifest().is_err());
        std::fs::remove_file(&path).unwrap();
        symlink("other.json", &path).unwrap();
        assert!(directory.read_legacy_native_manifest().is_err());
        for mode in [0o755, 0o775, 0o777] {
            let root = tempfile::tempdir_in("/private/tmp").unwrap();
            write(
                &root.path().join("manifest.json"),
                br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable"}"#,
                0o644,
            );
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(mode)).unwrap();
            let result = Directory::open(root.path())
                .unwrap()
                .read_legacy_native_manifest();
            assert_eq!(result.is_ok(), mode == 0o755, "{mode:o}");
        }
    }

    #[test]
    fn legacy_manifest_stamp_rejects_equal_bytes_at_another_inode_and_any_generation_change() {
        for change in ["replace", "content", "mode"] {
            let root = tempfile::tempdir_in("/private/tmp").unwrap();
            let path = root.path().join("manifest.json");
            let bytes =
                br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable","state":"saved"}"#;
            write(&path, bytes, 0o644);
            let directory = Directory::open(root.path()).unwrap();
            let (_, stamp) = directory.read_legacy_native_manifest().unwrap().unwrap();
            match change {
                "replace" => {
                    std::fs::rename(&path, root.path().join("retained.json")).unwrap();
                    write(&path, bytes, 0o644);
                }
                "content" => write(
                    &path,
                    br#"{"schemaVersion":4,"inputDiagnosticsState":"unavailable","state":"other"}"#,
                    0o644,
                ),
                _ => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap()
                }
            }
            assert!(stamp.require(&directory).is_err(), "{change}");
        }
    }
}
