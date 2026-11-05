//! Clip the native content too: CSS alone leaves an opaque rectangular window.
//! Uses public AppKit/Core Animation APIs, not WKWebView's private drawsBackground key.
use tauri::{AppHandle, Manager};

pub(crate) fn configure(app: &AppHandle) -> Result<(), String> {
    for label in ["recorder", "recording-confirmation"] {
        if let Some(window) = app.get_webview_window(label) {
            round(&window)?;
        }
    }
    Ok(())
}

fn round(window: &tauri::WebviewWindow) -> Result<(), String> {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject, MainThreadMarker};
    let _main = MainThreadMarker::new().ok_or("角丸の設定はメインスレッドで実行します")?;
    let native = window.ns_window().map_err(|error| error.to_string())?;
    // SAFETY: setup runs on AppKit's main thread. Tauri owns the live NSWindow;
    // the returned view/layer/color are retained while sending public setters.
    unsafe {
        let native = &*(native as *const AnyObject);
        let content: Option<Retained<AnyObject>> = msg_send![native, contentView];
        let content = content.ok_or("ウィンドウの内容を取得できません")?;
        let _: () = msg_send![&*content, setWantsLayer: true];
        let layer: Option<Retained<AnyObject>> = msg_send![&*content, layer];
        let layer = layer.ok_or("ウィンドウの描画レイヤーを取得できません")?;
        let _: () = msg_send![&*layer, setCornerRadius: 12.0_f64];
        let _: () = msg_send![&*layer, setMasksToBounds: true];
        let clear: Retained<AnyObject> = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![native, setBackgroundColor: &*clear];
        let _: () = msg_send![native, setOpaque: false];
        let _: () = msg_send![native, invalidateShadow];
    }
    Ok(())
}
