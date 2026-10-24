//! Shared identity and lifetime guards for local native inference engines.
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
pub(crate) mod resident;
pub(crate) mod verification;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

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
        let before = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() == 0 {
            return Err("モデル・実行ファイルが空または通常ファイルではありません".into());
        }
        let digest = verification::hash(&path, &mut file, cancelled, retain)?;
        let after = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
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
    #[cfg(target_os = "windows")]
    job: Option<windows::Job>,
    #[cfg(target_os = "macos")]
    owner: Option<std::os::unix::net::UnixStream>,
}

pub(crate) fn native_environment(command: &mut Command) {
    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("SystemRoot") {
            command
                .env("SystemRoot", &root)
                .env("PATH", PathBuf::from(root).join("System32"));
        }
    }
    #[cfg(not(windows))]
    let _ = command;
}
impl ManagedChild {
    // Import helpers already have their own parent-PID protocol. This preserves
    // that existing path; native inference uses OwnedCommand instead.
    pub(crate) fn direct(child: Child) -> Self {
        Self {
            child,
            #[cfg(target_os = "windows")]
            job: None,
            #[cfg(target_os = "macos")]
            owner: None,
        }
    }
}

/// Native inference uses an OS-owned lifetime: a macOS guardian or Windows jobs.
/// No PID from a receipt, file or later scan is adopted.
pub(crate) struct OwnedCommand {
    pub(crate) command: Command,
    #[cfg(target_os = "windows")]
    job: Option<windows::Job>,
    #[cfg(target_os = "macos")]
    receiver: Option<std::os::unix::net::UnixStream>,
    #[cfg(target_os = "macos")]
    owner: Option<std::os::unix::net::UnixStream>,
}
impl OwnedCommand {
    pub(crate) fn new(program: &Path) -> Result<Self, String> {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            windows::initialize().map_err(|e| e.to_string())?;
            let job = windows::Job::new().map_err(|e| e.to_string())?;
            let mut command = Command::new(program);
            command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
            Ok(Self {
                command,
                job: Some(job),
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            // Callers configure the command after construction; enforce this
            // invariant at the actual spawn boundary as well.
            self.command
                .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
            let job = self
                .job
                .take()
                .ok_or_else(|| std::io::Error::other("worker job was already consumed"))?;
            let mut child = self.command.spawn()?;
            if let Err(error) = job.assign_and_resume(&child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            return Ok(ManagedChild {
                child,
                job: Some(job),
            });
        }
        #[cfg(not(target_os = "windows"))]
        let child = self.command.spawn()?;
        #[cfg(target_os = "macos")]
        {
            self.receiver.take();
            Ok(ManagedChild {
                child,
                owner: self.owner.take(),
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
    metadata: crate::native_fs::Metadata,
}

impl StableFile {
    pub(crate) fn watch(proof: &Fingerprint) -> Result<Self, String> {
        let metadata =
            crate::native_fs::symlink_metadata(&proof.path).map_err(|e| e.to_string())?;
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
        let current = crate::native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if current.len() != self.metadata.len()
            || current.modified().map_err(|e| e.to_string())?
                != self.metadata.modified().map_err(|e| e.to_string())?
        {
            return Err("実行中にモデル・実行ファイルが変更されました".into());
        }
        #[cfg(any(unix, windows))]
        {
            #[cfg(unix)]
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
        #[cfg(target_os = "windows")]
        drop(self.job.take());
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
