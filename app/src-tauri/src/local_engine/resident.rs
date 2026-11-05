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

const PROCESS_POLICY: mon_native::process::Policy = mon_native::process::Purpose::Resident.policy();

mod installation;
#[cfg(all(test, windows))]
pub(crate) use installation::prepare_installed;
pub(crate) use installation::{prepare, prepare_job, Snapshot};
#[cfg(all(test, unix))]
pub(crate) mod tests;

#[cfg(test)]
thread_local! { static ADMISSION: RefCell<Option<Snapshot>> = const { RefCell::new(None) }; }

#[cfg(test)]
thread_local! { static INSTALLED_ADMISSION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Explicit admission boundary for tests that exercise installed runtimes
/// through product job preparation instead of a single supplied snapshot.
#[cfg(test)]
pub(crate) struct InstalledAdmission {
    previous: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
#[cfg(test)]
impl InstalledAdmission {
    pub(crate) fn enter() -> Self {
        Self {
            previous: INSTALLED_ADMISSION.with(|cell| cell.replace(true)),
            _thread: std::marker::PhantomData,
        }
    }
}
#[cfg(test)]
impl Drop for InstalledAdmission {
    fn drop(&mut self) {
        INSTALLED_ADMISSION.with(|cell| cell.set(self.previous));
    }
}

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
    generation_key: Option<GenerationKey>,
    active_generation: Option<GenerationKey>,
    startup_path: PathBuf,
}
impl Worker {
    fn launch(job: &Job, directory: &Path, snapshot: &Snapshot) -> Result<Self, String> {
        let proof = snapshot.resolve(&job.root)?;
        proof.verify(&|| false)?;
        let watched = StableFile::watch(&proof)?;
        let mut launch =
            OwnedCommand::for_purpose(&proof.path, mon_native::process::Purpose::Resident)?;
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
            generation_key: None,
            active_generation: None,
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
        self.active_generation = if matches!(kind, "llama" | "llama_prefix") {
            GenerationKey::from_args(&args)
        } else {
            None
        };
        let mut bytes = serde_json::to_vec(&json!({"protocol":1,"id":id,"op":"infer","kind":kind,"args":args,"directory":directory,"stdout":stdout,"stderr":stderr})).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > PROCESS_POLICY.stdout_bytes {
            return Err("推論要求が上限を超えています".into());
        }
        bytes.push(b'\n');
        if !self.initialized {
            self.requires_pcm_ranges = matches!(kind, "whisper" | "sherpa" | "embedding");
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
            > PROCESS_POLICY.stderr_bytes
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
            if self.pending.len() as u64 > PROCESS_POLICY.stdout_bytes {
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
                    if self.requires_pcm_ranges && value["message"] != "canonical_pcm_range_v1" {
                        self.failed = true;
                        return Err("推論ランタイムが共通WAVの範囲読出しに未対応です。更新版のランタイムを選択してください".into());
                    }
                }
                Some("progress") => {}
                Some("done") => {
                    self.watched.verify()?;
                    self.active = None;
                    if let Some(key) = self.active_generation.take() {
                        self.generation_key = Some(key);
                    }
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

// Admission checks account for the complete prospective allocation. Once the
// same job's model/context is resident, charging it against free RAM again
// would count it twice. Tokenizer and estimator requests never set this flag.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GenerationKey {
    model: String,
    context: u32,
    layers: u32,
}
impl GenerationKey {
    fn from_args(args: &[String]) -> Option<Self> {
        let value = |flag: &str| args.windows(2).find(|v| v[0] == flag).map(|v| v[1].clone());
        Some(Self {
            model: value("--model")?,
            context: value("--ctx-size")?.parse().ok()?,
            layers: value("--gpu-layers")?.parse().ok()?,
        })
    }
}
pub(crate) fn generation_ready_for(model: &Path, context: u32, layers: u32) -> bool {
    JOB.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|j| j.worker.as_ref())
            .is_some_and(|worker| {
                !worker.failed
                    && worker.generation_key.as_ref().is_some_and(|key| {
                        Some(key.model.as_str()) == model.to_str()
                            && key.context == context
                            && key.layers == layers
                    })
            })
    })
}

// Native GPU admission clamps its requested upper bound to the model's actual
// layer count. Only the verified successful trial may normalize this key.
pub(crate) fn confirm_generation_layers(layers: u32) {
    JOB.with(|cell| {
        if let Some(key) = cell
            .borrow_mut()
            .as_mut()
            .and_then(|j| j.worker.as_mut())
            .and_then(|w| w.generation_key.as_mut())
        {
            if layers > 0 && layers <= key.layers {
                key.layers = layers;
            }
        }
    });
}

#[cfg(test)]
pub(crate) fn synthetic_enabled() -> bool {
    JOB.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|j| installation::is_synthetic(&j.snapshot))
    })
}

/// A separate metadata-only session. It cannot change the job's cached model,
/// context, placement or protocol state.
pub(crate) struct Estimator {
    worker: Option<Worker>,
    root: PathBuf,
    snapshot: Snapshot,
    directory: tempfile::TempDir,
    sequence: u64,
}
impl Estimator {
    pub(crate) fn open(snapshot: Option<&Snapshot>) -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = JOB
            .with(|cell| cell.borrow().as_ref().map(|j| j.root.clone()))
            .unwrap_or(directory.path().canonicalize().map_err(|e| e.to_string())?);
        let snapshot = match snapshot {
            Some(snapshot) => snapshot.clone(),
            None => prepare(&root, "llama")?.ok_or("メモリ見積もりワーカーがありません")?,
        };
        snapshot.validate()?;
        Ok(Self {
            worker: None,
            root,
            snapshot,
            directory,
            sequence: 0,
        })
    }
    pub(crate) fn run(
        &mut self,
        args: Vec<String>,
        gpu: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<u8>, String> {
        use std::time::{Duration, Instant};
        const OUTPUT_LIMIT: u64 = 32 * 1024;
        let selected = {
            #[cfg(windows)]
            {
                if gpu {
                    self.snapshot.accelerated()?
                } else {
                    self.snapshot.clone()
                }
            }
            #[cfg(not(windows))]
            {
                let _ = gpu;
                self.snapshot.clone()
            }
        };
        let directory = self.directory.path().join(self.sequence.to_string());
        self.sequence += 1;
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        for name in ["memory.json", "memory.log"] {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(directory.join(name))
                .map_err(|e| e.to_string())?;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.runtime_sha256 != selected.sha256)
        {
            self.worker.take();
        }
        if self.worker.is_none() {
            let job = Job {
                root: self.root.clone(),
                snapshot: selected.clone(),
                worker: None,
            };
            self.worker = Some(Worker::launch(&job, &directory, &selected)?);
        }
        let worker = self.worker.as_mut().unwrap();
        let id = worker.request(
            "llama_memory",
            args,
            &directory,
            "memory.json",
            "memory.log",
        )?;
        let started = Instant::now();
        loop {
            if cancelled() {
                self.worker.take();
                return Err("メモリ確認が中断されました".into());
            }
            if started.elapsed() > Duration::from_secs(30) {
                self.worker.take();
                return Err("メモリ見積もりが時間上限を超えました".into());
            }
            for (name, limit) in [("memory.json", OUTPUT_LIMIT), ("memory.log", 1024 * 1024)] {
                let meta =
                    std::fs::symlink_metadata(directory.join(name)).map_err(|e| e.to_string())?;
                if !meta.is_file() || meta.len() > limit {
                    self.worker.take();
                    return Err("メモリ見積もりの出力が上限を超えています".into());
                }
            }
            if self.worker.as_mut().unwrap().poll(id)?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let bytes = std::fs::read(directory.join("memory.json")).map_err(|e| e.to_string())?;
        if bytes.is_empty() || bytes.len() as u64 > OUTPUT_LIMIT {
            return Err("メモリ見積もりの出力が空か上限を超えています".into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod generation_key_tests {
    use super::*;
    #[test]
    fn warm_admission_identity_includes_model_context_and_placement() {
        let args: Vec<String> = [
            "--model",
            "/model.gguf",
            "--ctx-size",
            "40960",
            "--gpu-layers",
            "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let key = GenerationKey::from_args(&args).unwrap();
        assert_eq!(
            key,
            GenerationKey {
                model: "/model.gguf".into(),
                context: 40960,
                layers: 0
            }
        );
        for (index, value) in [(1, "/other.gguf"), (3, "32768"), (5, "16")] {
            let mut other = args.clone();
            other[index] = value.into();
            assert_ne!(Some(key.clone()), GenerationKey::from_args(&other));
        }
        assert!(GenerationKey::from_args(&args[..4]).is_none());
    }
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
