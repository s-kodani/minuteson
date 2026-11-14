use super::{complete, policy};
use std::future::IntoFuture;
use windows::{
    core::Interface,
    Services::Store::StoreContext,
    Win32::{Foundation::HWND, UI::Shell::IInitializeWithWindow},
};

pub(super) fn is_store_package() -> bool {
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut length = 0;
    unsafe {
        GetCurrentPackageFullName(&mut length, std::ptr::null_mut());
    }
    if length == 0 || length > 32768 {
        return false;
    }
    let mut name = vec![0u16; length as usize];
    if unsafe { GetCurrentPackageFullName(&mut length, name.as_mut_ptr()) } != 0 {
        return false;
    }
    let name = String::from_utf16_lossy(&name[..length.saturating_sub(1) as usize]);
    let identity: serde_json::Value =
        serde_json::from_str(include_str!("../../../windows-store.json")).unwrap();
    name.starts_with(&format!(
        "{}_",
        identity["identityName"].as_str().unwrap_or("invalid")
    ))
}
pub(super) fn request(app: &tauri::AppHandle) {
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        use tauri::Manager;
        let operation = (|| -> windows::core::Result<_> {
            let window = handle
                .get_webview_window("main")
                .ok_or_else(windows::core::Error::from_thread)?;
            let hwnd = window
                .hwnd()
                .map_err(|_| windows::core::Error::from_thread())?;
            let context = StoreContext::GetDefault()?;
            unsafe {
                context
                    .cast::<IInitializeWithWindow>()?
                    .Initialize(HWND(hwnd.0))?;
            }
            context.GetAppAndOptionalStorePackageUpdatesAsync()
        })();
        match operation {
            Ok(operation) => {
                tauri::async_runtime::spawn(async move {
                    let result = match tokio::time::timeout(
                        policy::DEADLINE,
                        operation.clone().into_future(),
                    )
                    .await
                    {
                        Ok(Ok(updates)) => updates
                            .Size()
                            .map(|n| (n > 0, None))
                            .map_err(|e| e.to_string()),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => {
                            let _ = operation.Cancel();
                            Err("Store update check timed out".into())
                        }
                    };
                    complete(&handle, result);
                });
            }
            Err(error) => complete(&handle, Err(error.to_string())),
        }
    });
    if let Err(error) = queued {
        complete(app, Err(error.to_string()));
    }
}
pub(super) async fn open(window: &tauri::WebviewWindow, destination: &str) -> Result<(), String> {
    // Only the controller's fixed Store URI reaches this boundary.
    if destination != policy::STORE_PAGE {
        return Err("unexpected Store URI".into());
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            use windows_sys::Win32::UI::{
                Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL,
            };
            let verb: Vec<u16> = "open\0".encode_utf16().collect();
            let uri: Vec<u16> = policy::STORE_PAGE.encode_utf16().chain(Some(0)).collect();
            let value = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    verb.as_ptr(),
                    uri.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                )
            };
            let _ = send.send(if value as isize > 32 {
                Ok(())
            } else {
                Err("Store URI launch failed".into())
            });
        })
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(policy::DEADLINE, receive)
        .await
        .map_err(|_| "Store URI launch timed out".to_string())?
        .map_err(|_| "Store URI launch response lost".to_string())?
}
