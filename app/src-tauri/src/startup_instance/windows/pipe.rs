//! Same-user local rendezvous. Only activation is accepted over the pipe;
//! termination is exclusively an admitted, reviewed main-window command.
use super::process::{wide, Target};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Pipes::*,
};
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    version: u8,
    pid: u32,
    shown: bool,
}
pub(super) fn server(name: &str) -> Result<File, String> {
    let descriptor = wide(&format!(
        "D:P(A;;GA;;;SY)(A;;GA;;;{})",
        crate::windows_files::current_sid()?
    ));
    let mut sd = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let pipe = unsafe {
        CreateNamedPipeW(
            wide(name).as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            4096,
            4096,
            1000,
            &attributes,
        )
    };
    let error = std::io::Error::last_os_error();
    unsafe {
        LocalFree(sd);
    }
    if pipe == INVALID_HANDLE_VALUE {
        return Err(error.to_string());
    }
    Ok(unsafe { File::from_raw_handle(pipe) })
}
// PIPE_NOWAIT reports success when an instance becomes available, before a
// client connects. Only ERROR_PIPE_CONNECTED establishes a usable connection.
pub(super) fn connected(file: &File) -> Result<bool, String> {
    if unsafe { ConnectNamedPipe(file.as_raw_handle(), std::ptr::null_mut()) } != 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error().map(|code| code as u32) {
        Some(ERROR_PIPE_CONNECTED) => Ok(true),
        Some(ERROR_PIPE_LISTENING) => Ok(false),
        Some(ERROR_NO_DATA | ERROR_BROKEN_PIPE) => {
            unsafe {
                DisconnectNamedPipe(file.as_raw_handle());
            }
            Ok(false)
        }
        _ => Err(error.to_string()),
    }
}
pub(super) const RECEIVED: &[u8] = b"MON_RECEIVED_V1\n";
pub(super) fn reply(file: &mut File, bytes: &[u8]) -> Result<(), std::io::Error> {
    file.write_all(bytes)?;
    // Never FlushFileBuffers here: a suspended client could block the guard
    // indefinitely. Keep unread replies alive until receipt/close or a deadline.
    let _ = line_io(file, Instant::now() + Duration::from_secs(1));
    Ok(())
}
pub(super) fn line(file: &mut File, deadline: Instant) -> Result<Vec<u8>, String> {
    line_io(file, deadline).map_err(|error| error.to_string())
}
pub(super) fn line_io(file: &mut File, deadline: Instant) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    while Instant::now() < deadline {
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if available > 0 {
            if bytes.len() + available as usize > 4096 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "起動連携の応答が上限外です",
                ));
            }
            let mut part = vec![0u8; available as usize];
            let count = file.read(&mut part)?;
            bytes.extend_from_slice(&part[..count]);
            if bytes.ends_with(b"\n") {
                return Ok(bytes);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "MONの画面表示の応答がありません",
    ))
}
pub(super) fn activate(
    name: &str,
    executable: &Path,
    timeout: Duration,
) -> Result<(Target, bool), String> {
    let deadline = Instant::now() + timeout;
    let mut file = loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
            .open(name)
        {
            Ok(file) => break file,
            Err(e)
                if Instant::now() < deadline
                    && [Some(2), Some(231)].contains(&e.raw_os_error()) =>
            {
                std::thread::sleep(Duration::from_millis(30))
            }
            Err(e) => return Err(format!("既存MONとの接続を確認できません: {e}")),
        }
    };
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let target = Target::open(pid, executable)?;
    target.allow_foreground();
    let shown = file.write_all(b"MON_ACTIVATE_V1\n").is_ok()
        && line(&mut file, deadline)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Ack>(&bytes).ok())
            .is_some_and(|ack| ack.version == 1 && ack.pid == pid && ack.shown);
    let _ = file.write_all(RECEIVED);
    Ok((target, shown))
}
pub(super) fn serve(
    app: tauri::AppHandle,
    name: String,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut file = server(&name)?;
    std::thread::Builder::new()
        .name("mon-startup-instance".into())
        .spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let connected = match connected(&file) {
                    Ok(connected) => connected,
                    Err(_) => return,
                };
                if !connected {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                if line(&mut file, Instant::now() + Duration::from_millis(800))
                    .is_ok_and(|line| line == b"MON_ACTIVATE_V1\n")
                {
                    let (sender, receive) = std::sync::mpsc::sync_channel(1);
                    let handle = app.clone();
                    let cancelled = stop.clone();
                    let request_id = uuid::Uuid::new_v4().to_string();
                    let request = request_id.clone();
                    let queued = app
                        .run_on_main_thread(move || {
                            let shown = !cancelled.load(Ordering::SeqCst)
                                && handle.get_webview_window("main").is_some_and(|main| {
                                    crate::residency::restore(&handle).is_ok()
                                        && main.is_visible().unwrap_or(false)
                                        && !main.is_minimized().unwrap_or(true)
                                });
                            if shown {
                                if let Ok(mut pending) = handle
                                    .state::<crate::startup_instance::Instance>()
                                    .activation
                                    .lock()
                                {
                                    *pending = Some((request.clone(), sender));
                                }
                                use tauri::Emitter;
                                if let Some(main) = handle.get_webview_window("main") {
                                    let _ = main.emit(
                                        "startup-activate-requested",
                                        serde_json::json!({"requestId":request}),
                                    );
                                }
                            } else {
                                let _ = sender.send(false);
                            }
                        })
                        .is_ok();
                    let shown = queued
                        && receive
                            .recv_timeout(Duration::from_secs(4))
                            .unwrap_or(false);
                    if let Ok(mut pending) = app
                        .state::<crate::startup_instance::Instance>()
                        .activation
                        .lock()
                    {
                        if pending.as_ref().is_some_and(|(id, _)| *id == request_id) {
                            pending.take();
                        }
                    }
                    if let Ok(mut bytes) = serde_json::to_vec(&Ack {
                        version: 1,
                        pid: std::process::id(),
                        shown,
                    }) {
                        bytes.push(b'\n');
                        let _ = reply(&mut file, &bytes);
                    }
                }
                unsafe {
                    DisconnectNamedPipe(file.as_raw_handle());
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
