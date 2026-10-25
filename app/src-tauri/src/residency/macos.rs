//! Private inherited socket, no filesystem rendezvous and no PID-name scanning.
use super::{
    policy::{Action, Watch},
    Probe,
};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
const ENTRY: &str = "--mon-residency-watch-v1";
struct Owner {
    socket: UnixStream,
    child: Child,
}
static OWNER: OnceLock<Result<Mutex<Owner>, String>> = OnceLock::new();
static LAST: Mutex<Option<Instant>> = Mutex::new(None);
static PAUSED: AtomicBool = AtomicBool::new(false);
thread_local! {static SESSION_OBSERVER:std::cell::RefCell<Option<Retained<AnyObject>>>=const{std::cell::RefCell::new(None)};}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceSessionDidResignActiveNotification: *mut AnyObject;
    static NSWorkspaceSessionDidBecomeActiveNotification: *mut AnyObject;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const std::ffi::c_void;
    fn CFRunLoopRunInMode(
        mode: *const std::ffi::c_void,
        seconds: f64,
        return_after_source_handled: bool,
    ) -> i32;
    fn CFArrayGetCount(array: *const std::ffi::c_void) -> isize;
    fn CFArrayGetValueAtIndex(
        array: *const std::ffi::c_void,
        index: isize,
    ) -> *const std::ffi::c_void;
    fn CFDictionaryGetValue(
        dict: *const std::ffi::c_void,
        key: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn CFNumberGetValue(
        number: *const std::ffi::c_void,
        kind: isize,
        value: *mut std::ffi::c_void,
    ) -> bool;
    fn CFRelease(value: *const std::ffi::c_void);
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(options: u32, relative: u32) -> *const std::ffi::c_void;
    static kCGWindowOwnerPID: *const std::ffi::c_void;
}
pub(super) fn paused() -> bool {
    PAUSED.load(Ordering::SeqCst)
}
pub(super) fn observation(pid: u32) -> super::policy::Observation {
    use super::policy::Observation;
    if dock_present(pid) {
        return Observation::Present;
    }
    // Public window metadata only; this does not capture screen pixels or
    // request Accessibility/Screen Recording permission.
    unsafe {
        let list = CGWindowListCopyWindowInfo(1 | 16, 0);
        if list.is_null() {
            return Observation::Unknown;
        }
        let mut found = false;
        for index in 0..CFArrayGetCount(list) {
            let dictionary = CFArrayGetValueAtIndex(list, index);
            let value = CFDictionaryGetValue(dictionary, kCGWindowOwnerPID);
            let mut owner = 0i32;
            if !value.is_null()
                && CFNumberGetValue(value, 3, (&mut owner as *mut i32).cast())
                && owner == pid as i32
            {
                found = true;
                break;
            }
        }
        CFRelease(list);
        if found {
            Observation::Present
        } else {
            Observation::Absent
        }
    }
}
extern "C" fn pause(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    PAUSED.store(true, Ordering::SeqCst);
}
extern "C" fn resume(_: &AnyObject, _: Sel, _: *mut AnyObject) {
    PAUSED.store(false, Ordering::SeqCst);
}
pub(super) fn observe_session() {
    SESSION_OBSERVER.with(|slot|{
        if slot.borrow().is_some(){return;}
        let Some(base)=AnyClass::get(c"NSObject")else{return;};
        let class=if let Some(class)=AnyClass::get(c"MONResidencySessionObserver"){class}else{
            let Some(mut builder)=ClassBuilder::new(c"MONResidencySessionObserver",base)else{return;};
            unsafe{builder.add_method(sel!(residencyPause:),pause as extern "C" fn(_,_,_));builder.add_method(sel!(residencyResume:),resume as extern "C" fn(_,_,_));}builder.register()
        };
        let Some(workspace)=AnyClass::get(c"NSWorkspace")else{return;};
        unsafe{
            let observer:Retained<AnyObject>=msg_send![class,new];
            let workspace:Retained<AnyObject>=msg_send![workspace,sharedWorkspace];
            let center:Retained<AnyObject>=msg_send![&*workspace,notificationCenter];
            let _:()=msg_send![&*center,addObserver:&*observer,selector:sel!(residencyPause:),name:NSWorkspaceSessionDidResignActiveNotification,object:std::ptr::null::<AnyObject>()];
            let _:()=msg_send![&*center,addObserver:&*observer,selector:sel!(residencyResume:),name:NSWorkspaceSessionDidBecomeActiveNotification,object:std::ptr::null::<AnyObject>()];
            *slot.borrow_mut()=Some(observer);
        }
    });
}
pub(super) fn dock_present(pid: u32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32).is_some_and(|app| {
        !app.isTerminated() && app.activationPolicy() == NSApplicationActivationPolicy::Regular
    })
}
fn activate(pid: u32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32) {
        let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
}
fn identity(pid: i32) -> Option<(u32, u64, u64)> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    if unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    } != size
    {
        return None;
    }
    Some((info.pbi_uid, info.pbi_start_tvsec, info.pbi_start_tvusec))
}
fn same_executable(pid: i32) -> bool {
    use std::os::unix::ffi::OsStringExt;
    let mut path = vec![0u8; 4096];
    if unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) } <= 0 {
        return false;
    }
    path.truncate(path.iter().position(|c| *c == 0).unwrap_or(path.len()));
    let actual = std::path::PathBuf::from(std::ffi::OsString::from_vec(path)).canonicalize();
    matches!((actual,std::env::current_exe().and_then(|p|p.canonicalize())),(Ok(actual),Ok(expected))if actual==expected)
}
pub(super) fn prepare() {
    OWNER.get_or_init(|| {
        let (socket, receiver) = UnixStream::pair().map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        let fd = receiver.as_raw_fd();
        let parent = std::process::id();
        let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .args([ENTRY, &parent.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().map_err(|e| e.to_string())?;
        drop(receiver);
        Ok(Mutex::new(Owner { socket, child }))
    });
}
pub(super) fn available() -> bool {
    LAST.lock()
        .is_ok_and(|s| s.is_some_and(|t| t.elapsed() < Duration::from_secs(3)))
}
pub(super) fn pulse(probe: &Probe) -> Result<bool, String> {
    let mut owner = OWNER
        .get()
        .ok_or("常駐監視が未準備です")?
        .as_ref()
        .map_err(Clone::clone)?
        .try_lock()
        .map_err(|_| "常駐監視の応答待ちです")?;
    if owner.child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Err("常駐監視が終了したため画面を復帰します".into());
    }
    let mut bytes = serde_json::to_vec(probe).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    owner.socket.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut reply = [0; 1];
    owner
        .socket
        .read_exact(&mut reply)
        .map_err(|e| e.to_string())?;
    *LAST.lock().map_err(|_| "常駐監視の状態を保持できません")? = Some(Instant::now());
    match reply[0] {
        b'D' => Ok(true),
        b'O' => Ok(false),
        _ => Err("メイン画面を復帰しています".into()),
    }
}
pub(super) fn entry() -> Option<i32> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_none_or(|s| s != ENTRY) {
        return None;
    }
    Some(run(&args).map_or(1, |_| 0))
}
fn run(args: &[String]) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt;
    if args.len() != 3 {
        return Err("invalid watch invocation".into());
    }
    let pid = args[2].parse::<i32>().map_err(|e| e.to_string())?;
    if pid <= 1 || unsafe { libc::getppid() } != pid {
        return Err("watch parent changed".into());
    }
    if !same_executable(pid) {
        return Err("watch parent executable changed".into());
    }
    let stamp = identity(pid).ok_or("watch parent unavailable")?;
    if stamp.0 != unsafe { libc::geteuid() } {
        return Err("watch owner changed".into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(3) };
    if !file
        .metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_socket()
    {
        return Err("watch channel is not a socket".into());
    }
    let mut socket = UnixStream::from(std::os::fd::OwnedFd::from(file));
    if !socket.local_addr().map_err(|e| e.to_string())?.is_unnamed()
        || !socket.peer_addr().map_err(|e| e.to_string())?.is_unnamed()
    {
        return Err("watch channel is not private".into());
    }
    unsafe {
        libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    observe_session();
    let started = Instant::now();
    let mut watch = Watch::default();
    let mut data = Vec::new();
    let mut probe = Probe::default();
    let mut last = Instant::now() - Duration::from_secs(10);
    loop {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, true);
        }
        if identity(pid) != Some(stamp) {
            return Ok(());
        }
        let observed = if last.elapsed() < Duration::from_secs(3) && probe.tray_present {
            super::policy::Observation::Present
        } else {
            observation(pid as u32)
        };
        let action = watch.observe(
            started.elapsed().as_millis() as u64,
            observed,
            PAUSED.load(Ordering::SeqCst),
        );
        if action == Action::Restore {
            activate(pid as u32);
        }
        if action == Action::Terminate {
            if identity(pid) == Some(stamp) {
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while identity(pid) == Some(stamp) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if identity(pid) == Some(stamp) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            return Ok(());
        }
        let mut bytes = [0; 4096];
        match socket.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(len) => {
                data.extend_from_slice(&bytes[..len]);
                if data.len() > 4096 {
                    return Err("watch message too large".into());
                }
                if data.ends_with(b"\n") {
                    probe = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
                    last = Instant::now();
                    data.clear();
                    let reply = if observation(pid as u32) == super::policy::Observation::Present
                        || probe.tray_present
                    {
                        b'O'
                    } else if action == Action::Drain {
                        b'D'
                    } else {
                        b'R'
                    };
                    socket.write_all(&[reply]).map_err(|e| e.to_string())?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
