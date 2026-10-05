use super::*;

pub(super) async fn select(window: &WebviewWindow, kind: Kind) -> Result<Option<PathBuf>, String> {
    if kind == Kind::Backup {
        let filename = format!(
            "MON-{}.monbackup",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        return crate::exports::save_dialog_with(
            window,
            filename,
            "バックアップを作成",
            "monbackup",
        )
        .await;
    }
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
                use objc2_foundation::{NSArray, NSString};
                let result = (|| -> Result<_, String> {
                    let mtm = MainThreadMarker::new()
                        .ok_or("バックアップ選択にはメインスレッドが必要です")?;
                    let panel = NSOpenPanel::openPanel(mtm);
                    panel.setTitle(Some(&NSString::from_str("バックアップから追加復元")));
                    panel.setCanChooseFiles(true);
                    panel.setCanChooseDirectories(false);
                    panel.setAllowsMultipleSelection(false);
                    #[allow(deprecated)]
                    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                        NSString::from_str("monbackup"),
                    ])));
                    match panel.runModal() {
                        response if response == NSModalResponseCancel => Ok(None),
                        response if response == NSModalResponseOK => {
                            let url = panel.URL().ok_or("選択したバックアップがありません")?;
                            if !url.isFileURL() {
                                return Err("ローカルのバックアップを選んでください".into());
                            }
                            Ok(Some(PathBuf::from(
                                url.path()
                                    .ok_or("バックアップのパスがありません")?
                                    .to_string(),
                            )))
                        }
                        _ => Err("バックアップ選択を完了できませんでした".into()),
                    }
                })();
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receive
                .recv()
                .map_err(|_| "バックアップ選択の応答がありません".to_string())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("このOSのバックアップ選択は保留中です".into())
    }
}
