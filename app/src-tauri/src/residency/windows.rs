//! Public Shell/desktop APIs only; never inspect Explorer's private memory.
use super::policy::Observation;
use super::Probe;
use windows_sys::Win32::{
    Foundation::*,
    System::StationsAndDesktops::*,
    UI::{Shell::*, WindowsAndMessaging::*},
};
pub(crate) fn paused() -> bool {
    let desktop = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if desktop.is_null() {
        return true;
    }
    let mut name = [0u16; 64];
    let mut needed = 0;
    let ok = unsafe {
        GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            std::mem::size_of_val(&name) as u32,
            &mut needed,
        )
    } != 0;
    unsafe {
        CloseDesktop(desktop);
    }
    !ok || String::from_utf16_lossy(
        &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())],
    ) != "Default"
}
fn window_present(hwnd: u64, pid: u32) -> bool {
    if hwnd == 0 {
        return false;
    }
    let hwnd = hwnd as usize as HWND;
    let mut actual = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut actual);
        actual == pid && IsWindowVisible(hwnd) != 0
    }
}
pub(crate) fn observe(pid: u32, probe: &Probe) -> Observation {
    if window_present(probe.main, pid) || window_present(probe.mini, pid) {
        return Observation::Present;
    }
    // Also covers startup before the first registered HWND is delivered.
    struct Find {
        pid: u32,
        found: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, ptr: LPARAM) -> i32 {
        let state = unsafe { &mut *(ptr as *mut Find) };
        if window_present(hwnd as usize as u64, state.pid) {
            state.found = true;
            return 0;
        }
        1
    }
    let mut state = Find { pid, found: false };
    let enumerated = unsafe { EnumWindows(Some(visit), &mut state as *mut _ as isize) };
    if state.found {
        return Observation::Present;
    }
    if enumerated == 0 {
        return Observation::Unknown;
    }
    let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(Some(0)).collect();
    let shell = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    let mut reply = 0;
    if shell.is_null()
        || unsafe {
            SendMessageTimeoutW(
                shell,
                WM_NULL,
                0,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                200,
                &mut reply,
            )
        } == 0
    {
        return Observation::Unknown;
    }
    let Some(guid) = probe
        .tray
        .as_ref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
    else {
        return Observation::Absent;
    };
    let id = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        guidItem: windows_sys::core::GUID::from_u128(guid.as_u128()),
        ..Default::default()
    };
    let mut rect = RECT::default();
    if unsafe { Shell_NotifyIconGetRect(&id, &mut rect) } == 0 {
        Observation::Present
    } else {
        Observation::Absent
    }
}
pub(crate) fn restore(pid: u32, probe: &Probe) {
    for value in [probe.main, probe.mini] {
        let hwnd = value as usize as HWND;
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == pid {
                ShowWindowAsync(hwnd, SW_RESTORE);
                ShowWindowAsync(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
        }
    }
}
