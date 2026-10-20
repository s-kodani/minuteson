//! Shared identity and lifetime guards for local native inference engines.
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
pub(crate) mod resident;
pub(crate) mod verification;

#[cfg(target_os = "macos")]
mod guard_core;
#[cfg(all(test, target_os = "macos"))]
mod guard_tests;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Fingerprint {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

impl Fingerprint {
    pub fn capture(path: &Path, cancelled: &impl Fn() -> bool) -> Result<Self, String> {
        Self::capture_mode(path, cancelled, false)
    }
    pub(crate) fn capture_retained(
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_mode(path, cancelled, true)
    }
    fn capture_mode(
        path: &Path,
        cancelled: &impl Fn() -> bool,
        retain: bool,
    ) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("モデル・実行ファイルには絶対パスが必要です".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if path.to_str().is_none() {
            return Err("パスを文字列として保存できません".into());
        }
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() == 0 {
            return Err("モデル・実行ファイルが空または通常ファイルではありません".into());
        }
        let digest = verification::hash(&path, &mut file, cancelled, retain)?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if crate::audio::reuse::stamp(&before) != crate::audio::reuse::stamp(&after)
            || before.len() != after.len()
            || before.modified().map_err(|e| e.to_string())?
                != after.modified().map_err(|e| e.to_string())?
        {
            return Err("照合中にモデル・実行ファイルが変更されました".into());
        }
        Ok(Self {
            path,
            size: before.len(),
            sha256: digest,
        })
    }

    pub fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if *self != Self::capture_mode(&self.path, cancelled, true)? {
            return Err("固定したモデル・実行ファイルのハッシュが一致しません".into());
        }
        Ok(())
    }
}

pub(crate) struct ManagedChild {
    pub(crate) child: Child,
    #[cfg(target_os = "macos")]
    owner: Option<std::os::unix::net::UnixStream>,
}
impl ManagedChild {
    // Import helpers already have their own parent-PID protocol. This preserves
    // that existing path; native inference uses OwnedCommand instead.
    pub(crate) fn direct(child: Child) -> Self {
        Self {
            child,
            #[cfg(target_os = "macos")]
            owner: None,
        }
    }
}

/// Four inference engines share one headless guardian and one anonymous owner
/// channel per launch. No PID from a receipt, file or later scan is adopted.
pub(crate) struct OwnedCommand {
    pub(crate) command: Command,
    #[cfg(target_os = "macos")]
    receiver: Option<std::os::unix::net::UnixStream>,
    #[cfg(target_os = "macos")]
    owner: Option<std::os::unix::net::UnixStream>,
}
impl OwnedCommand {
    pub(crate) fn new(program: &Path) -> Result<Self, String> {
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self {
                command: Command::new(program),
            })
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::{fd::AsRawFd, unix::process::CommandExt};
            let executable = guard_executable()?;
            let (receiver, owner) =
                std::os::unix::net::UnixStream::pair().map_err(|e| e.to_string())?;
            for stream in [&receiver, &owner] {
                if unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            let fd = receiver.as_raw_fd();
            let mut command = Command::new(executable);
            command.args([
                std::ffi::OsString::from(guard_core::ENTRY),
                std::process::id().to_string().into(),
                guard_core::OWNER_FD.to_string().into(),
                program.as_os_str().to_owned(),
                "--".into(),
            ]);
            // Only async-signal-safe descriptor operations happen after fork;
            // all allocation and parsing runs after exec in the guardian.
            unsafe {
                command.pre_exec(move || {
                    if libc::dup2(fd, guard_core::OWNER_FD) < 0
                        || libc::fcntl(guard_core::OWNER_FD, libc::F_SETFD, 0) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            Ok(Self {
                command,
                receiver: Some(receiver),
                owner: Some(owner),
            })
        }
    }
    pub(crate) fn spawn(&mut self) -> Result<ManagedChild, std::io::Error> {
        #[cfg(target_os = "macos")]
        if self.owner.is_none() || self.receiver.is_none() {
            return Err(std::io::Error::other(
                "engine ownership channel was already consumed",
            ));
        }
        let child = self.command.spawn()?;
        #[cfg(target_os = "macos")]
        {
            self.receiver.take();
            Ok(ManagedChild {
                child,
                owner: self.owner.take(),
            })
        }
        #[cfg(not(target_os = "macos"))]
        Ok(ManagedChild::direct(child))
    }
}

#[cfg(all(not(test), target_os = "macos"))]
fn guard_executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}
#[cfg(all(test, target_os = "macos"))]
fn guard_executable() -> Result<PathBuf, String> {
    guard_tests::executable()
}
#[cfg(target_os = "macos")]
pub(crate) fn guard_entry() -> Option<i32> {
    guard_core::dispatch(&std::env::args_os().skip(1).collect::<Vec<_>>())
}
pub(crate) struct StableFile {
    path: PathBuf,
    metadata: std::fs::Metadata,
}

impl StableFile {
    pub(crate) fn watch(proof: &Fingerprint) -> Result<Self, String> {
        let metadata = std::fs::metadata(&proof.path).map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.len() != proof.size
            || proof.path.canonicalize().map_err(|e| e.to_string())? != proof.path
        {
            return Err("固定したモデル・実行ファイルが変更されています".into());
        }
        Ok(Self {
            path: proof.path.clone(),
            metadata,
        })
    }

    pub(crate) fn verify(&self) -> Result<(), String> {
        let current = std::fs::metadata(&self.path).map_err(|e| e.to_string())?;
        if current.len() != self.metadata.len()
            || current.modified().map_err(|e| e.to_string())?
                != self.metadata.modified().map_err(|e| e.to_string())?
        {
            return Err("実行中にモデル・実行ファイルが変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if current.dev() != self.metadata.dev()
                || current.ino() != self.metadata.ino()
                || current.ctime() != self.metadata.ctime()
                || current.ctime_nsec() != self.metadata.ctime_nsec()
            {
                return Err("実行中にモデル・実行ファイルが差し替えられました".into());
            }
        }
        Ok(())
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        // Dropping std::process::Child alone leaves a native worker alive.
        #[cfg(target_os = "macos")]
        if self.owner.take().is_some() {
            // Do not SIGKILL the guard before it can stop/reap its engine group.
            // EOF also happens without Drop when the MON parent is SIGKILLed.
            let _ = self.child.wait();
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
