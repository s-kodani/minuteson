//! Windows dialogs stay behind application commands and their ownership checks.
//! No generic dialog/filesystem permissions are exposed to the WebViews.
use std::{path::PathBuf, sync::mpsc};
use tauri::WebviewWindow;
use tauri_plugin_dialog::{DialogExt, FilePath};

fn local_path(selected: Option<FilePath>) -> Result<Option<PathBuf>, String> {
    selected
        .map(|path| {
            let path = path.into_path().map_err(|e| e.to_string())?;
            if !path.is_absolute() {
                return Err("ローカルの絶対パスを選択してください".into());
            }
            Ok(path)
        })
        .transpose()
}

async fn receive(receiver: mpsc::Receiver<Option<FilePath>>) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        local_path(
            receiver
                .recv()
                .map_err(|_| "ファイル選択の応答がありません")?,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) async fn save(
    window: &WebviewWindow,
    filename: &str,
    title: &str,
    extension: &str,
) -> Result<Option<PathBuf>, String> {
    let (send, receiver) = mpsc::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(title)
        .set_file_name(filename)
        .add_filter(title, &[extension])
        .save_file(move |value| {
            let _ = send.send(value);
        });
    receive(receiver).await
}

pub(crate) async fn open_audio(window: &WebviewWindow) -> Result<Option<PathBuf>, String> {
    let (send, receiver) = mpsc::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title("音声を取り込む")
        .add_filter("音声", &["wav", "mp3", "m4a", "flac"])
        .pick_file(move |value| {
            let _ = send.send(value);
        });
    receive(receiver).await
}

pub(crate) async fn open_data(
    window: &WebviewWindow,
    folder: bool,
) -> Result<Option<PathBuf>, String> {
    let (send, receiver) = mpsc::channel();
    let dialog = window.dialog().file().set_parent(window);
    if folder {
        dialog
            .set_title("会議保存先の親フォルダーを選択")
            .pick_folder(move |value| {
                let _ = send.send(value);
            });
    } else {
        dialog
            .set_title("バックアップから追加復元")
            .add_filter("MinutesOnバックアップ", &["monbackup"])
            .pick_file(move |value| {
                let _ = send.send(value);
            });
    }
    receive(receiver).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_not_an_error_and_relative_paths_are_rejected() {
        assert_eq!(local_path(None).unwrap(), None);
        assert!(local_path(Some(FilePath::Path("relative.wav".into()))).is_err());
        let path = PathBuf::from(r"C:\会議 記録\音声.wav");
        assert_eq!(
            local_path(Some(FilePath::Path(path.clone()))).unwrap(),
            Some(path)
        );
    }
}

/// Directory selection is exposed only through an owned application command.
pub(crate) async fn pick_directory(
    window: &WebviewWindow,
    title: &str,
) -> Result<Option<PathBuf>, String> {
    let (send, receiver) = mpsc::channel();
    window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(title)
        .pick_folder(move |value| {
            let _ = send.send(value);
        });
    receive(receiver).await
}
