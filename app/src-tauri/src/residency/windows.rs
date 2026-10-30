//! Public Shell/desktop APIs only; never inspect Explorer's private memory.
use super::policy::Observation;
use super::Probe;
use windows_sys::Win32::{
    Foundation::*,
    System::StationsAndDesktops::*,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub(super) fn set_tray_tooltip(
    icon: &tray_icon::TrayIcon,
    guid: &str,
    tooltip: &str,
) -> Result<(), String> {
    let guid = uuid::Uuid::parse_str(guid).map_err(|e| e.to_string())?;
    let data = tooltip_notification(icon.window_handle(), guid, tooltip);
    // tray-icon 0.25.1 omits NIF_GUID in set_tooltip, so its HWND/uID update
    // cannot address our GUID-registered icon. Keep the same Shell identity
    // used for registration, presence checks and removal.
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
        return Err(format!(
            "常駐アイコンの状態を更新できません: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn tooltip_notification(hwnd: HWND, guid: uuid::Uuid, tooltip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uFlags: NIF_GUID | NIF_TIP,
        guidItem: windows_sys::core::GUID::from_u128(guid.as_u128()),
        ..Default::default()
    };
    let mut length = 0;
    for character in tooltip.chars().take_while(|c| *c != '\0') {
        let mut buffer = [0u16; 2];
        let units = character.encode_utf16(&mut buffer);
        // Reserve the terminator and never split a UTF-16 surrogate pair.
        if length + units.len() >= data.szTip.len() {
            break;
        }
        data.szTip[length..length + units.len()].copy_from_slice(units);
        length += units.len();
    }
    data
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_notification_preserves_guid_and_terminated_unicode() {
        let guid = uuid::Uuid::parse_str("12345678-1234-5678-90ab-cdef12345678").unwrap();
        let tooltip = "MON QA — 録音中 01:02:03";
        let data = tooltip_notification(std::ptr::null_mut(), guid, tooltip);
        assert_eq!(data.uFlags, NIF_GUID | NIF_TIP);
        assert_eq!(data.guidItem.data1, 0x12345678);
        assert_eq!(data.guidItem.data2, 0x1234);
        assert_eq!(data.guidItem.data3, 0x5678);
        assert_eq!(
            data.guidItem.data4,
            [0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78]
        );
        let end = data.szTip.iter().position(|c| *c == 0).unwrap();
        assert_eq!(String::from_utf16(&data.szTip[..end]).unwrap(), tooltip);

        for (tooltip, expected) in [
            (format!("{}😀", "あ".repeat(126)), "あ".repeat(126)),
            ("あ".repeat(200), "あ".repeat(127)),
            ("MON\0ignored".into(), "MON".into()),
        ] {
            let data = tooltip_notification(std::ptr::null_mut(), guid, &tooltip);
            let end = data.szTip.iter().position(|c| *c == 0).unwrap();
            assert_eq!(String::from_utf16(&data.szTip[..end]).unwrap(), expected);
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop for real Shell registration"]
    fn shell_accepts_status_updates_for_guid_registered_tray() {
        let guid = uuid::Uuid::new_v4();
        // Exercise the real Shell registration used by MON, without starting
        // the application, opening storage, or capturing audio.
        for _ in 0..2 {
            let icon = tray_icon::TrayIconBuilder::new()
                .with_guid(guid.as_u128())
                .with_tooltip("MON isolated tray test — 待機中")
                .with_icon(
                    tray_icon::Icon::from_rgba([32, 32, 32, 255].repeat(256), 16, 16).unwrap(),
                )
                .build()
                .unwrap();
            assert!(icon.rect().is_some());
            for status in [
                "録音準備中",
                "録音中 00:00:01",
                "録音を保存中",
                "処理中",
                "待機中",
            ] {
                set_tray_tooltip(
                    &icon,
                    &guid.to_string(),
                    &format!("MON isolated tray test — {status}"),
                )
                .unwrap();
            }
            assert!(
                set_tray_tooltip(&icon, &uuid::Uuid::new_v4().to_string(), "wrong icon").is_err()
            );
            assert!(icon.rect().is_some());
            drop(icon);
        }
    }
}
