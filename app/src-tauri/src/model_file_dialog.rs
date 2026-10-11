//! Read-only selection. A native panel grants no execution or placement consent.
use std::path::PathBuf;
use tauri::WebviewWindow;

pub(crate) async fn select(
    window: &WebviewWindow,
    multiple: bool,
) -> Result<Option<Vec<PathBuf>>, String> {
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
                use objc2_foundation::NSString;
                let result = (|| -> Result<_, String> {
                    let mtm =
                        MainThreadMarker::new().ok_or("モデル選択にはメインスレッドが必要です")?;
                    let panel = NSOpenPanel::openPanel(mtm);
                    panel.setTitle(Some(&NSString::from_str("取り込むAIファイルを選択")));
                    panel.setCanChooseFiles(true);
                    panel.setCanChooseDirectories(false);
                    panel.setAllowsMultipleSelection(multiple);
                    panel.setResolvesAliases(false);
                    match panel.runModal() {
                        response if response == NSModalResponseCancel => Ok(None),
                        response if response == NSModalResponseOK => {
                            let urls = panel.URLs();
                            if urls.is_empty() || urls.len() > 16 || (!multiple && urls.len() != 1)
                            {
                                return Err("モデルファイルの選択件数が不正です".into());
                            }
                            let mut paths = Vec::new();
                            for url in urls.iter() {
                                if !url.isFileURL() {
                                    return Err("ローカルのモデルファイルを選んでください".into());
                                }
                                paths.push(PathBuf::from(
                                    url.path()
                                        .ok_or("選択したファイルのパスがありません")?
                                        .to_string(),
                                ));
                            }
                            Ok(Some(paths))
                        }
                        _ => Err("モデルファイルの選択を完了できませんでした".into()),
                    }
                })();
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receive
                .recv()
                .map_err(|_| "モデル選択の応答がありません".to_string())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, multiple);
        Err("このOSのモデルファイル選択は保留中です".into())
    }
}
