use super::Report;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
mod pipe;
mod process;
mod supervisor;
pub(super) fn supervisor_entry() -> Option<i32> {
    supervisor::entry()
}
pub(super) fn prepare_supervisor() {
    supervisor::prepare();
}
pub(super) fn schedule_restart() -> Result<(), String> {
    supervisor::schedule()
}
pub(super) fn restart_available() -> Result<(), String> {
    supervisor::available()
}
pub(super) fn residency_pulse(probe: &crate::residency::Probe) -> Result<bool, String> {
    supervisor::pulse(probe)
}
pub(super) fn residency_available() -> bool {
    supervisor::residency_available()
}
const TIMEOUT: Duration = Duration::from_secs(5);
fn endpoint_for(config: &Path, identifier: &str, sid: &str) -> String {
    let key = format!(
        "{sid}\n{identifier}\n{}",
        config.to_string_lossy().to_lowercase()
    );
    format!(r"\\.\pipe\mon-startup-{:x}", Sha256::digest(key.as_bytes()))
}

pub(super) struct Coordinator {
    config: PathBuf,
    executable: PathBuf,
    title: String,
    endpoint: String,
    guard: Option<File>,
    stop: Arc<AtomicBool>,
    target: Option<process::Target>,
    target_id: Option<String>,
    message: String,
}
impl Coordinator {
    pub(super) fn new(app: &tauri::AppHandle, config: &Path) -> Result<Self, String> {
        crate::native_fs::private_directories(config, true)?;
        let config = config.canonicalize().map_err(|e| e.to_string())?;
        let executable = std::env::current_exe()
            .and_then(|p| p.canonicalize())
            .map_err(|e| e.to_string())?;
        let title = app
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == "main")
            .map(|w| w.title.clone())
            .ok_or("メイン画面の設定がありません")?;
        let endpoint = endpoint_for(
            &config,
            &app.config().identifier,
            &crate::windows_files::current_sid()?,
        );
        Ok(Self {
            config,
            executable,
            title,
            endpoint,
            guard: None,
            stop: Arc::new(AtomicBool::new(false)),
            target: None,
            target_id: None,
            message: String::new(),
        })
    }
    pub(super) fn enter(&mut self, app: &tauri::AppHandle) -> Result<bool, String> {
        let guard = self.lock_file(".startup-instance.lock")?;
        match guard.try_lock() {
            Ok(()) => {
                self.guard = Some(guard);
                // Legacy releases own only the bootstrap lock. Inspect its OS
                // owner before admitting storage or claiming the new endpoint.
                self.discover(".bootstrap.lock")?;
                if self.target.is_some() {
                    if self.open_existing()? {
                        super::exit_secondary(app);
                        return Ok(false);
                    }
                    self.message = "既存のMONを開き直せませんでした。ここから復旧できます。".into();
                }
                pipe::serve(app.clone(), self.endpoint.clone(), self.stop.clone())?;
                Ok(self.target.is_none())
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if self.open_existing()? {
                    super::exit_secondary(app);
                }
                self.message =
                    "既存のMONから画面表示の応答がありません。ここから復旧できます。".into();
                Ok(false)
            }
            Err(e) => Err(format!("起動連携の排他を確認できません: {e}")),
        }
    }
    fn lock_file(&self, name: &str) -> Result<File, String> {
        let path = self.config.join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|e| e.to_string())?;
        let meta = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        use std::os::windows::fs::MetadataExt;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.file_attributes() & 0x400 != 0
            || meta.uid() != crate::native_fs::current_owner()?
            || meta.mode() & 0o077 != 0
        {
            return Err("起動連携ファイルの所有・実体を確認できません".into());
        }
        Ok(file)
    }
    fn set_target(&mut self, target: process::Target) {
        if self.target.as_ref().is_none_or(|old| !old.same(&target)) {
            self.target_id = Some(uuid::Uuid::new_v4().to_string());
        }
        self.target = Some(target);
    }
    fn discover(&mut self, name: &str) -> Result<(), String> {
        for (pid, created) in process::lock_owners(&self.config.join(name))? {
            if pid == std::process::id() {
                continue;
            }
            if let Ok(target) = process::Target::open(pid, &self.executable) {
                if target.created() == created {
                    if self.target.as_ref().is_some_and(|old| !old.same(&target)) {
                        self.target = None;
                        self.target_id = None;
                        return Err("複数の実行主体があり、復旧対象を限定できません。終了操作は行いません。".into());
                    }
                    self.set_target(target);
                }
            }
        }
        Ok(())
    }
    pub(super) fn open_existing(&mut self) -> Result<bool, String> {
        // A connection pins the actual pipe server process before waiting for
        // UI acknowledgement; a live server thread alone is not UI readiness.
        if self.guard.is_none() {
            match pipe::activate(&self.endpoint, &self.executable, TIMEOUT) {
                Ok((target, shown)) => {
                    self.set_target(target);
                    return Ok(shown);
                }
                Err(error) => self.message = error,
            }
        }
        if self.target.is_none() {
            self.discover(".startup-instance.lock")?;
        }
        if self.target.is_none() {
            self.discover(".bootstrap.lock")?;
        }
        if let Some(target) = self.target.as_ref() {
            if !target.exited() && target.restore_legacy(&self.title)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn report(&self) -> Report {
        Report {
            schema_version: 1,
            supported: true,
            target_id: self.target_id.clone(),
            can_restart: self.target_id.is_some() && self.target.is_some(),
            message: self.message.clone(),
        }
    }
    /// True means the original owner recovered and was shown. False means its
    /// process and storage lock are gone, so a fresh admitted process may start.
    pub(super) fn recover(&mut self, expected: &str) -> Result<bool, String> {
        if self.target_id.as_deref() != Some(expected) {
            return Err("復旧対象が変更されました。最新の内容を確認してください。".into());
        }
        let original = self
            .target
            .as_ref()
            .ok_or("復旧対象を確認できません")?
            .identity();
        if self.open_existing()? {
            return Ok(true);
        }
        if self.target_id.as_deref() != Some(expected)
            || self
                .target
                .as_ref()
                .is_none_or(|t| t.identity() != original)
        {
            return Err("復旧対象が変更されました。終了していません。".into());
        }
        // Consume the reviewed token before the first destructive operation.
        // A lost response cannot replay termination with the same consent.
        self.target_id = None;
        let stopped = self
            .target
            .as_ref()
            .unwrap()
            .terminate_checked(&self.executable, TIMEOUT);
        if let Err(error) = stopped {
            self.target_id = Some(uuid::Uuid::new_v4().to_string());
            self.message = error.clone();
            return Err(error);
        }
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let lock = self.lock_file(".bootstrap.lock")?;
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => {
                    return Err(format!(
                        "保存先の排他解放が未確認です。データは保持しています: {e}"
                    ))
                }
            }
        }
        Ok(false)
    }
    pub(super) fn release(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.guard.take();
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests;
