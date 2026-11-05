//! Native clipping follows physical size and monitor DPI. WebView CSS alone
//! leaves rectangular transparent-window hit regions on Windows.
use std::ffi::c_void;
use tauri::{AppHandle, Manager};
#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(
        window: *mut c_void,
        attribute: u32,
        value: *const c_void,
        bytes: u32,
    ) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateRoundRectRgn(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: i32,
        height: i32,
    ) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowRgn(window: *mut c_void, region: *mut c_void, redraw: i32) -> i32;
    fn GetWindowLongPtrW(window: *mut c_void, index: i32) -> isize;
    fn SetWindowLongPtrW(window: *mut c_void, index: i32, value: isize) -> isize;
    fn SetWindowPos(
        window: *mut c_void,
        after: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
}
fn target(label: &str) -> bool {
    matches!(label, "recorder" | "recording-confirmation")
}
fn apply(window: *mut c_void, size: tauri::PhysicalSize<u32>, scale: f64) -> Result<(), String> {
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    if !scale.is_finite()
        || scale <= 0.0
        || size.width > i32::MAX as u32 - 1
        || size.height > i32::MAX as u32 - 1
    {
        return Err("Windows表示領域が範囲外です".into());
    }
    // Tao retains caption styles for native window behavior. These small,
    // non-resizable cards need neither caption nor non-client resize insets.
    // Recalculate only on an actual style change to avoid resize recursion.
    let mut changed = false;
    for (index, mask) in [(-16, 0x00c4_0000isize), (-20, 0x0002_0301isize)] {
        let style = unsafe { GetWindowLongPtrW(window, index) };
        if style & mask != 0 {
            if unsafe { SetWindowLongPtrW(window, index, style & !mask) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            changed = true;
        }
    }
    if changed && unsafe { SetWindowPos(window, std::ptr::null_mut(), 0, 0, 0, 0, 0x0037) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let corner = 2u32;
    let result = unsafe { DwmSetWindowAttribute(window, 33, (&corner as *const u32).cast(), 4) };
    if result < 0 {
        return Err(format!("Windowsの角丸を設定できません: {result:#x}"));
    }
    // Windows 11 otherwise paints a one-pixel system-colored outline around
    // undecorated cards, independently of the WebView's CSS border.
    let border = 0xffff_fffeu32; // DWMWA_COLOR_NONE
    let result = unsafe { DwmSetWindowAttribute(window, 34, (&border as *const u32).cast(), 4) };
    if result < 0 {
        return Err(format!("Windowsの外枠を解除できません: {result:#x}"));
    }
    let diameter = (24.0 * scale).round().clamp(1.0, 256.0) as i32;
    let region = unsafe {
        CreateRoundRectRgn(
            0,
            0,
            size.width as i32 + 1,
            size.height as i32 + 1,
            diameter,
            diameter,
        )
    };
    if region.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { SetWindowRgn(window, region, 1) } == 0 {
        unsafe {
            DeleteObject(region);
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
pub(crate) fn configure(app: &AppHandle) -> Result<(), String> {
    for label in ["recorder", "recording-confirmation"] {
        if let Some(window) = app.get_webview_window(label) {
            apply(
                window.hwnd().map_err(|e| e.to_string())?.0,
                window.outer_size().map_err(|e| e.to_string())?,
                window.scale_factor().map_err(|e| e.to_string())?,
            )?;
        }
    }
    Ok(())
}
pub(crate) fn event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if target(window.label())
        && matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
        )
    {
        let result = (|| {
            apply(
                window.hwnd().map_err(|e| e.to_string())?.0,
                window.outer_size().map_err(|e| e.to_string())?,
                window.scale_factor().map_err(|e| e.to_string())?,
            )
        })();
        if let Err(error) = result {
            crate::internal_logs::phase("windows_appearance", None, None, "failed", Some(&error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            parameter: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(window: *mut c_void) -> i32;
        fn GetClientRect(
            window: *mut c_void,
            rect: *mut windows_sys::Win32::Foundation::RECT,
        ) -> i32;
        fn GetWindowRect(
            window: *mut c_void,
            rect: *mut windows_sys::Win32::Foundation::RECT,
        ) -> i32;
    }
    #[test]
    #[ignore = "creates isolated hidden native Windows cards; run explicitly on the Windows desktop"]
    fn floating_cards_have_no_native_frame_or_client_insets() {
        let mut config: tauri::Config =
            serde_json::from_str(include_str!("../tauri.qa.conf.json")).unwrap();
        crate::window_chrome::configure(&mut config);
        let mut cards = config.app.windows;
        cards.retain(|window| target(&window.label));
        // Exercise native style changes without application setup, WebView,
        // user data, audio, or a process-wide GUI event loop in the test runner.
        struct Native(*mut c_void);
        impl Drop for Native {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let class = [83u16, 84, 65, 84, 73, 67, 0]; // built-in STATIC window class
        for card in cards {
            let window = Native(unsafe {
                CreateWindowExW(
                    0x20301,
                    class.as_ptr(),
                    std::ptr::null(),
                    0x00cf0000,
                    0,
                    0,
                    540,
                    174,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            });
            assert!(!window.0.is_null());
            for scale in [1.0, 1.5, 2.0] {
                let size = tauri::PhysicalSize::new(
                    (card.width * scale) as u32,
                    (card.height * scale) as u32,
                );
                assert_ne!(
                    unsafe {
                        SetWindowPos(
                            window.0,
                            std::ptr::null_mut(),
                            0,
                            0,
                            size.width as i32,
                            size.height as i32,
                            0x14,
                        )
                    },
                    0
                );
                apply(window.0, size, scale).unwrap();
                let mut inner: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
                let mut outer: windows_sys::Win32::Foundation::RECT = unsafe { std::mem::zeroed() };
                assert_ne!(unsafe { GetClientRect(window.0, &mut inner) }, 0);
                assert_ne!(unsafe { GetWindowRect(window.0, &mut outer) }, 0);
                assert_eq!(
                    (inner.right - inner.left, inner.bottom - inner.top),
                    (outer.right - outer.left, outer.bottom - outer.top),
                    "{} retains native frame insets",
                    card.label
                );
                assert_eq!(unsafe { GetWindowLongPtrW(window.0, -16) } & 0x00c40000, 0);
                assert_eq!(unsafe { GetWindowLongPtrW(window.0, -20) } & 0x20301, 0);
                eprintln!(
                    "{}: native client equals outer, scale={scale}, caption/border styles=none",
                    card.label
                );
            }
        }
    }
}
