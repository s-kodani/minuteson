//! A blocking job owns this thread-local scope. Only model state lives in the
//! child; durable unit results and admission remain owned by the scheduler.
use super::{Fingerprint, ManagedChild, OwnedCommand, StableFile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, ExitStatus, Stdio};

mod installation;
pub(crate) use installation::{prepare, prepare_job, Snapshot};
#[cfg(all(test, unix))]
pub(crate) mod tests;

#[cfg(test)]
thread_local! { static ADMISSION: RefCell<Option<Snapshot>> = const { RefCell::new(None) }; }

thread_local! { static JOB: RefCell<Option<Job>> = const { RefCell::new(None) }; }
struct Job {
    root: PathBuf,
    snapshot: Snapshot,
    worker: Option<Worker>,
}
pub(crate) struct Scope;
impl Scope {
    pub(crate) fn enter(root: &Path, snapshot: Option<&Snapshot>) -> Result<Self, String> {
        JOB.with(|cell| {
            if cell.borrow().is_some() {
                return Err("推論セッションが重複しています".into());
            }
            #[cfg(test)]
            if snapshot.is_some_and(installation::is_synthetic) {
                return Ok(Self);
            }
            if let Some(snapshot) = snapshot {
                snapshot.validate()?;
                #[cfg(windows)]
                if snapshot.protocol == 2 {
                    // Native Windows CLI binaries use the same frozen inputs
                    // and Job Object teardown; they do not speak resident v1.
                    snapshot.resolve(root)?;
                    return Ok(Self);
                }
                *cell.borrow_mut() = Some(Job {
                    root: root.to_owned(),
                    snapshot: snapshot.clone(),
                    worker: None,
                });
            }
            Ok(Self)
        })
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        JOB.with(|cell| {
            cell.borrow_mut().take();
        });
    }
}
// GPU selection is the only permitted retry within a job. A failed trial may
// release its worker; a successful trial keeps the loaded model for inference.
pub(crate) fn discard_trial() {
    JOB.with(|cell| {
        if let Some(job) = cell.borrow_mut().as_mut() {
            job.worker.take();
        }
    });
}

struct Worker {
    runtime_sha256: String,
    child: ManagedChild,
    input: ChildStdin,
    output: ChildStdout,
    pending: Vec<u8>,
    watched: StableFile,
    next: u64,
    active: Option<u64>,
    failed: bool,
    initialized: bool,
    requires_pcm_ranges: bool,
    startup_path: PathBuf,
}
impl Worker {
    fn launch(job: &Job, directory: &Path, snapshot: &Snapshot) -> Result<Self, String> {
        let proof = snapshot.resolve(&job.root)?;
        proof.verify(&|| false)?;
        let watched = StableFile::watch(&proof)?;
        let mut launch = OwnedCommand::new(&proof.path)?;
        // Startup diagnostics have their own bounded file. Per-request engine
        // stderr is redirected by the child into each existing unit log.
        let startup = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("worker-startup.log"))
            .map_err(|e| e.to_string())?;
        launch
            .command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(startup)
            .arg("--mon-resident-v1");
        super::native_environment(&mut launch.command);
        let mut child = launch.spawn().map_err(|e| e.to_string())?;
        let input = child
            .child
            .stdin
            .take()
            .ok_or("ワーカーの入力パイプがありません")?;
        let output = child
            .child
            .stdout
            .take()
            .ok_or("ワーカーの出力パイプがありません")?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let fd = output.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(Self {
            runtime_sha256: snapshot.sha256.clone(),
            child,
            input,
            output,
            pending: vec![],
            watched,
            next: 1,
            active: None,
            failed: false,
            initialized: false,
            requires_pcm_ranges: false,
            startup_path: directory.join("worker-startup.log"),
        })
    }
    fn request(
        &mut self,
        kind: &str,
        args: Vec<String>,
        directory: &Path,
        stdout: &str,
        stderr: &str,
    ) -> Result<u64, String> {
        if self.failed || self.active.is_some() {
            return Err("推論ワーカーは停止済みか実行中です".into());
        }
        self.watched.verify()?;
        let id = self.next;
        self.next += 1;
        let mut bytes = serde_json::to_vec(&json!({"protocol":1,"id":id,"op":"infer","kind":kind,"args":args,"directory":directory,"stdout":stdout,"stderr":stderr})).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("推論要求が上限を超えています".into());
        }
        bytes.push(b'\n');
        if !self.initialized {
            self.requires_pcm_ranges=matches!(kind,"whisper"|"sherpa"|"embedding");
            writeln!(self.input, "{}", json!({"protocol":1,"id":id,"op":"init"}))
                .map_err(|e| e.to_string())?;
            self.initialized = true;
        }
        self.input.write_all(&bytes).map_err(|e| e.to_string())?;
        self.active = Some(id);
        Ok(id)
    }
    fn poll(&mut self, id: u64) -> Result<Option<ExitStatus>, String> {
        if self.active != Some(id) || self.failed {
            return Err("推論応答の実行IDが一致しません".into());
        }
        if std::fs::metadata(&self.startup_path)
            .map_err(|e| e.to_string())?
            .len()
            > 4 * 1024 * 1024
        {
            return Err("推論起動ログが上限を超えています".into());
        }
        let mut block = [0; 8192];
        for _ in 0..8 {
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                let mut available = 0;
                if unsafe {
                    windows_sys::Win32::System::Pipes::PeekNamedPipe(
                        self.output.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    if std::io::Error::last_os_error().raw_os_error() == Some(109)
                        && !self.pending.is_empty()
                    {
                        break;
                    }
                    self.failed = true;
                    return Err(std::io::Error::last_os_error().to_string());
                }
                if available == 0 {
                    break;
                }
            }
            match self.output.read(&mut block) {
                Ok(0) => {
                    self.failed = true;
                    return Err("推論ワーカーとの通信が終了しました".into());
                }
                Ok(count) => self.pending.extend_from_slice(&block[..count]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.failed = true;
                    return Err(e.to_string());
                }
            }
            if self.pending.len() > 65536 {
                self.failed = true;
                return Err("推論応答が上限を超えています".into());
            }
        }
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let value: Value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
            if value["protocol"] != 1 || value["id"] != id {
                self.failed = true;
                return Err("推論プロトコル・応答IDが一致しません".into());
            }
            match value["op"].as_str() {
                Some("ready") => {
                    if self.requires_pcm_ranges && value["message"]!="canonical_pcm_range_v1" {
                        self.failed=true;
                        return Err("推論ランタイムが共通WAVの範囲読出しに未対応です。更新版のランタイムを選択してください".into());
                    }
                }
                Some("progress") => {}
                Some("done") => {
                    self.watched.verify()?;
                    self.active = None;
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        return Ok(Some(ExitStatus::from_raw(0)));
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::ExitStatusExt;
                        return Ok(Some(ExitStatus::from_raw(0)));
                    }
                    #[cfg(not(any(unix, windows)))]
                    {
                        return Err("常駐推論はこのOSに対応していません".into());
                    }
                }
                Some("error") => {
                    self.failed = true;
                    return Err(format!(
                        "推論ワーカーが失敗しました: {}",
                        value["message"].as_str().unwrap_or("engine error")
                    ));
                }
                _ => {
                    self.failed = true;
                    return Err("未知の推論応答です".into());
                }
            }
        }
        if let Some(status) = self.child.child.try_wait().map_err(|e| e.to_string())? {
            self.failed = true;
            return Err(format!("推論ワーカーが途中終了しました: {status}"));
        }
        Ok(None)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // ManagedChild closes the independent guardian owner channel and reaps
        // the exact process group, including a worker stuck inside native code.
        let _ = self
            .input
            .write_all(b"{\"protocol\":1,\"id\":0,\"op\":\"shutdown\"}\n");
    }
}

pub(crate) fn enabled() -> bool {
    JOB.with(|cell| cell.borrow().is_some())
}

pub(crate) enum Run {
    Direct(ManagedChild),
    Resident(u64),
}
impl Run {
    pub(crate) fn spawn(
        launch: &mut OwnedCommand,
        kind: &str,
        directory: &Path,
        stdout: &str,
        stderr: &str,
    ) -> Result<Self, String> {
        JOB.with(|cell| {
            let mut current = cell.borrow_mut();
            let Some(job) = current.as_mut() else {
                return launch.spawn().map(Self::Direct).map_err(|e| e.to_string());
            };
            job.snapshot.accepts(kind)?;
            let args = launch
                .command
                .get_args()
                .map(|arg| {
                    arg.to_str()
                        .filter(|value| !value.contains('\0'))
                        .map(str::to_owned)
                        .ok_or("推論引数がUTF-8ではないかNULを含みます".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            // OwnedCommand prepends guardian arguments on macOS. Forward only
            // the native engine's arguments, never its program or guard entry.
            #[cfg(target_os = "macos")]
            let args = args.into_iter().skip(5).collect();
            #[cfg(windows)]
            let selected = {
                let gpu = if kind == "llama_tokenize" {
                    job.worker.as_ref().is_some_and(|worker| {
                        job.snapshot
                            .gpu
                            .as_ref()
                            .is_some_and(|gpu| gpu.sha256 == worker.runtime_sha256)
                    })
                } else if kind == "whisper" {
                    !args.iter().any(|v| v == "--no-gpu")
                } else if kind == "llama" || kind == "llama_prefix" {
                    args.windows(2).any(|v| {
                        matches!(v[0].as_str(), "--n-gpu-layers" | "-ngl" | "--gpu-layers")
                            && v[1].parse::<u32>().is_ok_and(|n| n > 0)
                    })
                } else {
                    false
                };
                if gpu {
                    job.snapshot.accelerated()?
                } else {
                    job.snapshot.clone()
                }
            };
            #[cfg(not(windows))]
            let selected = job.snapshot.clone();
            if job
                .worker
                .as_ref()
                .is_some_and(|worker| worker.runtime_sha256 != selected.sha256)
            {
                return Err("保存したCPU/GPU実行構成が区間間で変更されています".into());
            }
            if job.worker.is_none() {
                job.worker = Some(Worker::launch(job, directory, &selected)?);
            }
            job.worker
                .as_mut()
                .unwrap()
                .request(kind, args, directory, stdout, stderr)
                .map(Self::Resident)
        })
    }
    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        match self {
            Self::Direct(child) => child.child.try_wait().map_err(|e| e.to_string()),
            Self::Resident(id) => JOB.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|j| j.worker.as_mut())
                    .ok_or("推論セッションがありません".to_owned())?
                    .poll(*id)
            }),
        }
    }
}
