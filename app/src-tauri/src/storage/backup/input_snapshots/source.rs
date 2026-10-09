//! Bounded descriptor-relative reads. Keep small metadata/hash witnesses, not
//! thousands of open writer locks or an unbounded copy of original JSON bytes.
use super::*;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;

const FILES: [(&str, u64); 3] = [
    ("input-events.json", 65 * 1024),
    ("input-notices.json", 128 * 1024),
    ("manifest.json", 256 * 1024),
];
#[derive(Clone, PartialEq, Eq)]
struct DirectoryStamp {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    birth: std::time::SystemTime,
}
impl DirectoryStamp {
    fn read(file: &File) -> Result<Self, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
            return Err("入力履歴の保存先の所有が不正です".into());
        }
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            owner: m.uid(),
            mode: m.mode(),
            birth: m.created().map_err(|e| e.to_string())?,
        })
    }
}
#[derive(PartialEq, Eq)]
struct FileStamp {
    identity: DirectoryStamp,
    length: u64,
    links: u64,
    modified: std::time::SystemTime,
    change: i64,
    change_nanos: i64,
}
impl FileStamp {
    fn read(file: &File, name: &str, limit: u64) -> Result<Self, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != unsafe { libc::geteuid() }
            || m.len() == 0
            || m.len() > limit
            || (name != "manifest.json" && m.mode() & 0o077 != 0)
        {
            return Err("入力履歴の所有・共有リンク・権限または容量が不正です".into());
        }
        Ok(Self {
            identity: DirectoryStamp {
                device: m.dev(),
                inode: m.ino(),
                owner: m.uid(),
                mode: m.mode(),
                birth: m.created().map_err(|e| e.to_string())?,
            },
            length: m.len(),
            links: m.nlink(),
            modified: m.modified().map_err(|e| e.to_string())?,
            change: m.ctime(),
            change_nanos: m.ctime_nsec(),
        })
    }
}
fn directory(
    root: &File,
    meeting: &str,
    session: &str,
) -> Result<(Vec<DirectoryStamp>, Option<File>), String> {
    let mut parent = root.try_clone().map_err(|e| e.to_string())?;
    let mut chain = vec![DirectoryStamp::read(&parent)?];
    for name in ["meetings", meeting, "audio", session] {
        let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok((chain, None));
            }
            return Err(error.to_string());
        }
        parent = unsafe { File::from_raw_fd(fd) };
        let stamp = DirectoryStamp::read(&parent)?;
        if stamp.device != chain[0].device {
            return Err("入力履歴の保存先が別媒体へ切り替わっています".into());
        }
        chain.push(stamp);
    }
    Ok((chain, Some(parent)))
}
struct Leaf {
    stamp: FileStamp,
    digest: String,
}
fn read(directory: &File, name: &str, limit: u64) -> Result<Option<(Leaf, Vec<u8>)>, String> {
    let Some(file) = crate::input_levels::open_capture_file_at_optional(directory, name)? else {
        return Ok(None);
    };
    let before = FileStamp::read(&file, name, limit)?;
    if before.identity.device != DirectoryStamp::read(directory)?.device {
        return Err("入力履歴の管理ファイルが別媒体にあります".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let current = crate::input_levels::open_capture_file_at(directory, name)?;
    if bytes.len() as u64 != before.length
        || before != FileStamp::read(&file, name, limit)?
        || before != FileStamp::read(&current, name, limit)?
    {
        return Err("入力履歴が読込み中に変更されました".into());
    }
    Ok(Some((
        Leaf {
            stamp: before,
            digest: format!("{:x}", Sha256::digest(&bytes)),
        },
        bytes,
    )))
}
pub(super) struct Witness {
    meeting: String,
    session: String,
    chain: Vec<DirectoryStamp>,
    leaves: Vec<Option<Leaf>>,
}
impl Witness {
    pub(super) fn verify(&self, root: &File, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let (chain, dir) = directory(root, &self.meeting, &self.session)?;
        if chain != self.chain || dir.is_some() != (self.chain.len() == 5) {
            return Err("入力履歴の保存先または祖先が変更されています".into());
        }
        if let Some(dir) = dir {
            for ((name, limit), old) in FILES.iter().zip(&self.leaves) {
                check_cancelled(cancelled)?;
                let now = read(&dir, name, *limit)?;
                if match (old, now) {
                    (None, None) => false,
                    (Some(old), Some((now, _))) => {
                        old.stamp != now.stamp || old.digest != now.digest
                    }
                    _ => true,
                } {
                    return Err(
                        "バックアップ取得後に入力履歴・通知台帳または停止時診断が変更されています"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
}
pub(super) fn capture(
    root: &File,
    meeting: &str,
    session: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(Witness, Option<PortableInputSnapshot>), String> {
    check_cancelled(cancelled)?;
    let (chain, dir) = directory(root, meeting, session)?;
    let mut leaves = Vec::new();
    let mut buffers = Vec::new();
    if let Some(dir) = &dir {
        for (name, limit) in FILES {
            check_cancelled(cancelled)?;
            let (leaf, data) = match read(dir, name, limit)? {
                Some((leaf, data)) => (Some(leaf), Some(data)),
                None => (None, None),
            };
            leaves.push(leaf);
            buffers.push(data);
        }
    }
    let snapshot = if dir.is_some() {
        PortableInputSnapshot::from_evidence(
            session,
            buffers[0].as_deref(),
            buffers[1].as_deref(),
            buffers[2].as_deref(),
        )?
    } else {
        None
    };
    let witness = Witness {
        meeting: meeting.into(),
        session: session.into(),
        chain,
        leaves,
    };
    witness.verify(root, cancelled)?;
    Ok((witness, snapshot))
}
