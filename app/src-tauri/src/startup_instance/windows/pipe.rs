//! Same-user transport for the authenticated restart and residency relay.
use super::process::wide;
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Pipes::*,
};
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
        "アプリの画面表示の応答がありません",
    ))
}
