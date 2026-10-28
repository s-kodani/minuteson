//! A narrow restart relay created BEFORE the application joins its kill-on-exit
//! job. It never joins storage, captures audio, runs models or changes job limits.
//! It accepts one request from its pinned parent only, launches that same image
//! only after the parent exits, and otherwise exits when the parent dies.
use super::{pipe, process};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::Write,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{Pipes::*, Threading::CREATE_NO_WINDOW};
const MODE: &str = "--mon-startup-supervisor";
const REQUEST: &[u8] = b"MON_RESTART_AFTER_PARENT_EXIT_V1\n";
static SUPERVISOR: OnceLock<Result<Mutex<Supervisor>, String>> = OnceLock::new();
static LAST_PULSE: Mutex<Option<Instant>> = Mutex::new(None);
pub(super) fn residency_available() -> bool {
    LAST_PULSE
        .lock()
        .is_ok_and(|p| p.is_some_and(|time| time.elapsed() < Duration::from_secs(3)))
}
pub(super) fn pulse(probe: &crate::residency::Probe) -> Result<bool, String> {
    let mut supervisor = SUPERVISOR
        .get()
        .ok_or("常駐監視が未準備です")?
        .as_ref()
        .map_err(Clone::clone)?
        .try_lock()
        .map_err(|_| "常駐監視の応答待ちです")?;
    if supervisor
        .child
        .try_wait()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("常駐監視が終了したため画面を復帰します".into());
    }
    exchange(&supervisor.endpoint, supervisor.child.id(), probe, |_| {})
}
fn transient(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(2 | 109 | 231 | 232 | 233))
}
fn exchange(
    endpoint: &str,
    expected_pid: u32,
    probe: &crate::residency::Probe,
    mut before_read: impl FnMut(&File),
) -> Result<bool, String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let payload = serde_json::to_string(probe).map_err(|e| e.to_string())?;
    loop {
        let mut attempt = || -> Result<Vec<u8>, std::io::Error> {
            let mut file = OpenOptions::new().read(true).write(true).open(endpoint)?;
            let mut pid = 0;
            if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if pid != expected_pid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "常駐監視の接続先が変更されました",
                ));
            }
            file.write_all(format!("MON_RESIDENCY_V1 {payload}\n").as_bytes())?;
            before_read(&file);
            let response = pipe::line_io(&mut file, deadline)?;
            file.write_all(pipe::RECEIVED)?;
            Ok(response)
        };
        match attempt() {
            Ok(response) => {
                let drain = match response.as_slice() {
                    b"DRAIN\n" => true,
                    b"OK\n" => false,
                    _ => return Err("画面を復帰しています".into()),
                };
                *LAST_PULSE
                    .lock()
                    .map_err(|_| "常駐監視の状態を保持できません")? = Some(Instant::now());
                return Ok(drain);
            }
            Err(error) if transient(&error) && Instant::now() < deadline => {
                std::thread::sleep(
                    Duration::from_millis(20)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(error) => {
                return Err(format!(
                    "常駐監視との通信を確認できません。画面を残します: {error}"
                ))
            }
        }
        if Instant::now() >= deadline {
            return Err("常駐監視の応答がありません。画面を残します".into());
        }
    }
}
struct Supervisor {
    child: Child,
    endpoint: String,
    scheduled: bool,
}
impl Supervisor {
    fn spawn() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let (pid, start) = process::current_identity()?;
        let endpoint = format!(r"\\.\pipe\mon-restart-{}", uuid::Uuid::new_v4());
        let child = Command::new(executable)
            .arg(MODE)
            .arg(pid.to_string())
            .arg(start.to_string())
            .arg(&endpoint)
            .arg("--")
            .args(std::env::args_os().skip(1))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            child,
            endpoint,
            scheduled: false,
        })
    }
    fn schedule(&mut self) -> Result<(), String> {
        if self.scheduled {
            return Err("再起動要求は送信済みです。自動で再送しません。".into());
        }
        if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("再起動の監視処理を確認できません。復旧画面は保持しています。".into());
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut file = loop {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.endpoint)
            {
                Ok(file) => break file,
                Err(error)
                    if Instant::now() < deadline
                        && [Some(2), Some(231)].contains(&error.raw_os_error()) =>
                {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => {
                    return Err(format!(
                        "再起動の準備を確認できません。画面は保持しています: {error}"
                    ))
                }
            }
        };
        let mut server = 0;
        if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut server) } == 0
            || server != self.child.id()
        {
            return Err("再起動の監視先が変更されました".into());
        }
        self.scheduled = true;
        file.write_all(REQUEST).map_err(|e| e.to_string())?;
        if pipe::line(&mut file, deadline)? != b"MON_RESTART_ACCEPTED_V1\n" {
            return Err("再起動の受付を確認できません。画面を保持しています。".into());
        }
        Ok(())
    }
}
pub(super) fn prepare() {
    SUPERVISOR.get_or_init(|| Supervisor::spawn().map(Mutex::new));
}
pub(super) fn schedule() -> Result<(), String> {
    SUPERVISOR
        .get()
        .ok_or("再起動の監視処理が準備されていません")?
        .as_ref()
        .map_err(Clone::clone)?
        .try_lock()
        .map_err(|_| "再起動の要求を処理中です")?
        .schedule()
}
pub(super) fn entry() -> Option<i32> {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_none_or(|arg| arg != MODE) {
        return None;
    }
    Some(run(&args).map_or(1, |_| 0))
}
fn run(args: &[OsString]) -> Result<(), String> {
    if args.len() < 6 || args[5] != "--" {
        return Err("invalid supervisor invocation".into());
    }
    let pid = args[2]
        .to_str()
        .ok_or("invalid parent")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let start = args[3]
        .to_str()
        .ok_or("invalid parent")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let executable = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| e.to_string())?;
    let parent = process::Target::open(pid, &executable)?;
    if parent.created() != start {
        return Err("parent generation changed".into());
    }
    let endpoint = args[4].to_str().ok_or("invalid endpoint")?;
    if !endpoint.starts_with(r"\\.\pipe\mon-restart-") || endpoint.len() > 128 {
        return Err("invalid endpoint".into());
    }
    let file = pipe::server(endpoint)?;
    let mut successor = Command::new(executable);
    successor.args(&args[6..]);
    relay(file, parent, successor)
}
fn relay(mut file: File, parent: process::Target, mut successor: Command) -> Result<(), String> {
    use crate::residency::policy::{Action, Watch};
    let started = Instant::now();
    let mut watch = Watch::default();
    let mut probe = crate::residency::Probe::default();
    let mut checked = Instant::now() - Duration::from_secs(1);
    let mut action = Action::Healthy;
    loop {
        if parent.exited() {
            return Ok(());
        }
        if checked.elapsed() >= Duration::from_secs(1) {
            checked = Instant::now();
            let observed = crate::residency::windows::observe(parent.identity().0, &probe);
            let paused = crate::residency::windows::paused();
            #[cfg(test)]
            let (observed, paused) = if std::env::var_os("MON_RESIDENCY_FIXTURE_LOSS").is_some() {
                (crate::residency::policy::Observation::Absent, false)
            } else {
                (observed, paused)
            };
            action = watch.observe(started.elapsed().as_millis() as u64, observed, paused);
            match action {
                Action::Restore => crate::residency::windows::restore(parent.identity().0, &probe),
                Action::Terminate => {
                    let executable = std::env::current_exe()
                        .and_then(|p| p.canonicalize())
                        .map_err(|e| e.to_string())?;
                    parent.terminate_checked(&executable, Duration::from_secs(5))?;
                    return Ok(());
                }
                _ => {}
            }
        }
        if !pipe::connected(&file)? {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let mut client = 0;
        let allowed = unsafe { GetNamedPipeClientProcessId(file.as_raw_handle(), &mut client) }
            != 0
            && parent.identity().0 == client
            && !parent.exited();
        let request = if allowed {
            pipe::line(&mut file, Instant::now() + Duration::from_millis(500)).ok()
        } else {
            None
        };
        if let Some(payload) = request
            .as_ref()
            .and_then(|r| r.strip_prefix(b"MON_RESIDENCY_V1 "))
        {
            if let Ok(next) = serde_json::from_slice::<crate::residency::Probe>(payload) {
                probe = next;
                let reply = match action {
                    Action::Healthy => b"OK\n".as_slice(),
                    Action::Drain => b"DRAIN\n".as_slice(),
                    _ => b"RESTORE\n".as_slice(),
                };
                let _ = pipe::reply(&mut file, reply);
            }
        } else if request.as_ref().is_some_and(|request| request == REQUEST) {
            file.write_all(b"MON_RESTART_ACCEPTED_V1\n")
                .map_err(|e| e.to_string())?;
            if !parent.wait_exit(Duration::from_secs(10)) {
                return Err("parent did not exit; restart cancelled".into());
            }
            successor.spawn().map_err(|e| e.to_string())?;
            return Ok(());
        }
        unsafe {
            DisconnectNamedPipe(file.as_raw_handle());
        }
    }
}

#[cfg(test)]
pub(super) fn test_prepare(command: &mut Command) -> Result<(), String> {
    // Test parent invokes the same authenticated relay through its test harness,
    // with an explicit successor helper instead of launching production UI.
    let (pid, start) = process::current_identity()?;
    let endpoint = format!(r"\\.\pipe\mon-restart-{}", uuid::Uuid::new_v4());
    let child = command
        .env("MON_STARTUP_SUPERVISOR_PARENT", pid.to_string())
        .env("MON_STARTUP_SUPERVISOR_START", start.to_string())
        .env("MON_STARTUP_SUPERVISOR_PIPE", &endpoint)
        .spawn()
        .map_err(|e| e.to_string())?;
    SUPERVISOR
        .set(Ok(Mutex::new(Supervisor {
            child,
            endpoint,
            scheduled: false,
        })))
        .map_err(|_| "duplicate fixture supervisor")?;
    Ok(())
}
#[cfg(test)]
pub(super) fn test_relay(args: Vec<OsString>) -> Result<(), String> {
    let pid = std::env::var("MON_STARTUP_SUPERVISOR_PARENT")
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let start = std::env::var("MON_STARTUP_SUPERVISOR_START")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let endpoint = std::env::var("MON_STARTUP_SUPERVISOR_PIPE").unwrap();
    let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
    let parent = process::Target::open(pid, &exe)?;
    if parent.created() != start {
        return Err("changed fixture parent".into());
    }
    let mut successor = Command::new(exe);
    successor
        .args(args)
        .env("MON_STARTUP_FIXTURE_MODE", "restart-successor");
    relay(pipe::server(&endpoint)?, parent, successor)
}
#[cfg(test)]
pub(super) fn test_endpoint() -> String {
    SUPERVISOR
        .get()
        .unwrap()
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .endpoint
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Private in-process endpoints exercise the real Win32 pipe lifecycle,
    // never an installed MON or its storage. Relay peer authentication is also
    // covered by the existing child-process startup_instance tests.
    fn fixture(
        count: usize,
        mut respond: impl FnMut(&mut File, usize) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let endpoint = format!(r"\\.\pipe\mon-restart-test-{}", uuid::Uuid::new_v4());
        let mut file = pipe::server(&endpoint).unwrap();
        let server = std::thread::spawn(move || {
            for index in 0..count {
                let deadline = Instant::now() + Duration::from_secs(4);
                while !pipe::connected(&file).unwrap() {
                    assert!(Instant::now() < deadline, "client never connected");
                    std::thread::sleep(Duration::from_millis(5));
                }
                let request = pipe::line(&mut file, deadline).unwrap();
                assert!(request.starts_with(b"MON_RESIDENCY_V1 "));
                respond(&mut file, index);
                unsafe {
                    DisconnectNamedPipe(file.as_raw_handle());
                }
            }
        });
        (endpoint, server)
    }

    #[test]
    fn delayed_reader_keeps_buffered_reply_until_receipt() {
        let (endpoint, server) = fixture(2, |file, _| pipe::reply(file, b"OK\n").unwrap());
        assert!(
            !exchange(&endpoint, std::process::id(), &Default::default(), |file| {
                let deadline = Instant::now() + Duration::from_millis(500);
                loop {
                    let mut available = 0;
                    assert_ne!(
                        unsafe {
                            PeekNamedPipe(
                                file.as_raw_handle(),
                                std::ptr::null_mut(),
                                0,
                                std::ptr::null_mut(),
                                &mut available,
                                std::ptr::null_mut(),
                            )
                        },
                        0
                    );
                    if available > 0 {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                std::thread::sleep(Duration::from_millis(300));
            })
            .unwrap()
        );
        assert!(!exchange(&endpoint, std::process::id(), &Default::default(), |_| {}).unwrap());
        server.join().unwrap();
    }

    #[test]
    fn dropped_connection_retries_only_the_idempotent_pulse() {
        let (endpoint, server) = fixture(2, |file, index| {
            if index == 1 {
                pipe::reply(file, b"DRAIN\n").unwrap();
            }
        });
        assert!(exchange(&endpoint, std::process::id(), &Default::default(), |_| {}).unwrap());
        server.join().unwrap();
    }

    #[test]
    fn absent_or_unresponsive_monitor_has_a_single_bounded_deadline() {
        let endpoint = format!(r"\\.\pipe\mon-restart-absent-{}", uuid::Uuid::new_v4());
        let started = Instant::now();
        assert!(exchange(&endpoint, std::process::id(), &Default::default(), |_| {}).is_err());
        assert!(started.elapsed() < Duration::from_millis(1500));
        let (endpoint, server) = fixture(1, |_, _| std::thread::sleep(Duration::from_millis(1200)));
        let started = Instant::now();
        assert!(exchange(&endpoint, std::process::id(), &Default::default(), |_| {}).is_err());
        assert!(started.elapsed() < Duration::from_millis(1500));
        server.join().unwrap();
    }

    #[test]
    fn a_different_server_identity_is_not_retried() {
        let endpoint = format!(r"\\.\pipe\mon-restart-peer-{}", uuid::Uuid::new_v4());
        let _server = pipe::server(&endpoint).unwrap();
        let started = Instant::now();
        let error = exchange(
            &endpoint,
            std::process::id() + 1,
            &Default::default(),
            |_| {},
        )
        .unwrap_err();
        assert!(error.contains("接続先が変更"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn availability_and_abandoned_clients_are_not_accepted_connections() {
        let endpoint = format!(r"\\.\pipe\mon-restart-abandoned-{}", uuid::Uuid::new_v4());
        let server = pipe::server(&endpoint).unwrap();
        assert!(!pipe::connected(&server).unwrap());
        assert!(!pipe::connected(&server).unwrap());
        let abandoned = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&endpoint)
            .unwrap();
        drop(abandoned);
        assert!(!pipe::connected(&server).unwrap());
        // Disconnect resets the instance; the next nonblocking connect makes
        // it available again, without claiming that a new client exists.
        assert!(!pipe::connected(&server).unwrap());
        let _next = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&endpoint)
            .unwrap();
        assert!(pipe::connected(&server).unwrap());
    }
}
