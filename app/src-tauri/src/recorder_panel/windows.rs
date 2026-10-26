//! Automatic panel visibility must not activate the HWND. Keep the window
//! focusable so a deliberate click can still operate its controls.
use std::ffi::c_void;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsWindow, IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
};

pub(super) fn set_visible(window: *mut c_void, visible: bool) -> Result<(), String> {
    if unsafe { IsWindow(window) } == 0 {
        return Err("録音パネルのウィンドウを確認できません".into());
    }
    // ShowWindow returns the *previous* visibility, not a success flag.
    unsafe { ShowWindow(window, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
    if (unsafe { IsWindowVisible(window) } != 0) != visible {
        return Err("録音パネルの表示状態を変更できません".into());
    }
    Ok(())
}
