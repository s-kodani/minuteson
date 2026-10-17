//! Headless entry shared by MON and the synthetic test executable.
//! No DB, network, model discovery, GUI or signal-handler cleanup is used.
use std::io::{ErrorKind, Read};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::{fs::FileTypeExt, net::UnixStream, process::CommandExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub(super) const ENTRY: &str = "--mon-owned-engine-guard-v1";
pub(super) const OWNER_FD: i32 = 3;

fn owner_alive(owner: &mut UnixStream) -> Result<bool, String> {
    let mut byte = [0];
    match owner.read(&mut byte) {
        Ok(0) => Ok(false),
        Ok(_) => Err("unexpected data on engine ownership channel".into()),
        Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(true),
        Err(e) if e.kind() == ErrorKind::Interrupted => owner_alive(owner),
        Err(e) => Err(e.to_string()),
    }
}

struct Engine {
    child: Child,
    reaped: bool,
}
impl Engine {
    fn exited_without_reaping(&mut self) -> Result<bool, String> {
        // WNOWAIT retains the exact child PID even after its exit. Never kill
        // a process group after try_wait/wait has released that PID for reuse.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            Ok(unsafe { info.si_pid() } == self.child.id() as i32)
        } else {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ECHILD) {
                self.reaped = true;
                Err("engine child ownership was lost; no PID is adopted".into())
            } else if e.kind() == ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(e.to_string())
            }
        }
    }
    fn group_has_no_live_members(&self) -> Result<bool, String> {
        // Darwin killpg can return EPERM for a zombie-only group. Never infer
        // that a live group is gone from EPERM alone. Query only our still-
        // waitable child's group, with a fixed bound and no PID-based signals.
        let mut pids = [0_i32; 4096];
        let capacity = std::mem::size_of_val(&pids) as i32;
        let bytes = unsafe {
            libc::proc_listpids(
                2, /* PROC_PGRP_ONLY */
                self.child.id(),
                pids.as_mut_ptr().cast(),
                capacity,
            )
        };
        if bytes <= 0 || bytes >= capacity || bytes % std::mem::size_of::<i32>() as i32 != 0 {
            return Err("owned engine group could not be fully inspected".into());
        }
        let mut leader_is_zombie = false;
        for pid in &pids[..bytes as usize / std::mem::size_of::<i32>()] {
            if *pid <= 1 {
                return Err("invalid member in owned engine group".into());
            }
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            let read = unsafe {
                libc::proc_pidinfo(
                    *pid,
                    libc::PROC_PIDTBSDINFO,
                    1, // Darwin: include zombies when looking up BSD info.
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            if read != size {
                if read <= 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    continue; // A listed member was reaped. No PID is adopted.
                }
                return Err("owned engine group member could not be inspected".into());
            }
            if info.pbi_pid != *pid as u32 || info.pbi_pgid != self.child.id() {
                continue; // PID was reused outside this still-owned group.
            }
            if info.pbi_status != libc::SZOMB {
                return Ok(false);
            }
            leader_is_zombie |= info.pbi_pid == self.child.id();
        }
        Ok(leader_is_zombie)
    }
    fn stop_group(&mut self) -> Result<(), String> {
        let result = unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH)
            || (error.raw_os_error() == Some(libc::EPERM)
                && self.exited_without_reaping()?
                && self.group_has_no_live_members()?)
        {
            return Ok(());
        }
        Err(error.to_string())
    }
    fn wait(&mut self) -> Result<std::process::ExitStatus, String> {
        match self.child.wait() {
            Ok(status) => {
                self.reaped = true;
                Ok(status)
            }
            Err(e) => {
                if e.raw_os_error() == Some(libc::ECHILD) {
                    self.reaped = true;
                }
                Err(e.to_string())
            }
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        // This exact Child has not been reaped yet, including on error paths.
        if !self.reaped {
            let _ = self.stop_group();
            let _ = self.wait();
        }
    }
}

pub(super) fn dispatch(args: &[std::ffi::OsString]) -> Option<i32> {
    if args.first().is_none_or(|a| a != ENTRY) {
        return None;
    }
    Some(match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("MON engine guard: {e}");
            125
        }
    })
}

fn run(args: &[std::ffi::OsString]) -> Result<i32, String> {
    if args.len() < 5 || args[2].to_str() != Some("3") || args[4] != "--" {
        return Err("invalid private engine guard arguments".into());
    }
    let parent: i32 = args[1]
        .to_str()
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|p| *p > 1 && args[1].to_str() == Some(p.to_string().as_str()))
        .ok_or("invalid engine owner PID")?;
    if unsafe { libc::getppid() } != parent {
        return Err("engine owner is no longer the direct parent".into());
    }
    let program = Path::new(&args[3]);
    if !program.is_absolute() || !program.is_file() {
        return Err("engine path must be an existing absolute file".into());
    }
    if unsafe { libc::fcntl(OWNER_FD, libc::F_GETFD) } < 0 {
        return Err("inherited engine ownership channel is missing".into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(OWNER_FD) };
    if !file
        .metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_socket()
    {
        return Err("engine ownership channel is not a socket".into());
    }
    // The native engine must not inherit the owner's endpoint: its lifetime
    // must not conceal parent death from the guard by keeping the channel open.
    if unsafe { libc::fcntl(OWNER_FD, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut owner = UnixStream::from(OwnedFd::from(file));
    let mut socket_type: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&socket_type) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            OWNER_FD,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut libc::c_int).cast(),
            &mut length,
        )
    } != 0
        || socket_type != libc::SOCK_STREAM
        || !owner.local_addr().map_err(|e| e.to_string())?.is_unnamed()
        || !owner.peer_addr().map_err(|e| e.to_string())?.is_unnamed()
    {
        return Err("engine ownership channel is not an anonymous Unix stream pair".into());
    }
    owner.set_nonblocking(true).map_err(|e| e.to_string())?;
    if !owner_alive(&mut owner)? {
        return Ok(125);
    }
    // A parent may have ignored SIGCHLD. The headless guard must retain its
    // exact child's waitable identity, not let the kernel auto-reap it.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
    let child = Command::new(program)
        .args(&args[5..])
        // Legacy launches already provide /dev/null. Resident workers receive
        // their anonymous request pipe; the independent owner FD still closes
        // on exec, so a worker cannot conceal the parent's death.
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut engine = Engine {
        child,
        reaped: false,
    };
    loop {
        if !owner_alive(&mut owner)? {
            engine.stop_group()?;
            engine.wait()?;
            return Ok(125);
        }
        if engine.exited_without_reaping()? {
            // Stop any remaining members of this still-owned group before
            // releasing the leader PID, even when the leader exited 0.
            engine.stop_group()?;
            let status = engine.wait()?;
            use std::os::unix::process::ExitStatusExt;
            return Ok(status.code().unwrap_or(128 + status.signal().unwrap_or(0)));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
