//! Window metadata is available before storage admission and never opens data.
use mon_contracts::{Capabilities, Platform, WindowEnvironment, WindowRole};
use tauri::WebviewWindow;

#[tauri::command]
pub(crate) fn get_window_environment(
    window: WebviewWindow,
) -> Result<WindowEnvironment, mon_contracts::AppError> {
    let result: Result<WindowEnvironment, String> = (|| {
        let role: WindowRole = window.label().parse()?;
        let platform = if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        };
        Ok(WindowEnvironment {
            schema_version: 1,
            platform,
            role,
            capabilities: Capabilities {
                voice_identity: cfg!(any(target_os = "macos", windows)),
            },
        })
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_window_environment", message))
}
