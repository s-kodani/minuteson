use crate::native_fs::Metadata;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default)]
pub(crate) struct ExportController(Arc<AtomicBool>);
pub(crate) struct ExportLease(Arc<AtomicBool>);
impl Drop for ExportLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl ExportController {
    pub(crate) fn begin(&self) -> Result<ExportLease, String> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "書き出し操作が進行中です")?;
        Ok(ExportLease(self.0.clone()))
    }
    pub(crate) fn active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    #[cfg(windows)]
    created: std::time::SystemTime,
    size: u64,
    modified: std::time::SystemTime,
    hash: String,
    #[cfg(any(unix, windows))]
    device: u64,
    #[cfg(any(unix, windows))]
    inode: u64,
}
fn fingerprint_with(
    path: &Path,
    maximum: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Fingerprint>, String> {
    let metadata = match crate::native_fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("保存先は通常ファイルにしてください（リンクやフォルダーは不可）".into());
    }
    #[cfg(any(unix, windows))]
    {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("複数の場所を参照するファイルには書き出せません".into());
        }
    }
    if metadata.len() > maximum {
        return Err("置換対象のファイルが大きすぎます".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if !same_file(
        &metadata,
        &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
    ) {
        return Err("保存先が変更されました".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        if cancelled() {
            return Err("書き出しを中断しました".into());
        }
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or("保存先サイズが範囲外です")?;
        if bytes > metadata.len() || bytes > maximum {
            return Err("保存先が変更されました".into());
        }
        hash.update(&buffer[..count]);
    }
    let after = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
    if !same_file(&metadata, &after) || bytes != metadata.len() {
        return Err("保存先が変更されました".into());
    }
    Ok(Some(Fingerprint {
        #[cfg(windows)]
        created: metadata.created().map_err(|e| e.to_string())?,
        size: bytes,
        modified: metadata.modified().map_err(|e| e.to_string())?,
        hash: format!("{:x}", hash.finalize()),
        #[cfg(any(unix, windows))]
        device: {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            metadata.dev()
        },
        #[cfg(any(unix, windows))]
        inode: {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        },
    }))
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(any(unix, windows))]
    {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return false;
        }
    }
    a.is_file() == b.is_file()
        && a.is_dir() == b.is_dir()
        && a.len() == b.len()
        && a.modified().ok() == b.modified().ok()
}
pub(crate) struct Destination {
    #[cfg(windows)]
    _pins: Vec<File>,
    pub path: PathBuf,
    parent: PathBuf,
    parent_metadata: Metadata,
    before: Option<Fingerprint>,
    maximum_existing_bytes: u64,
    managed_root: PathBuf,
    extension: String,
}
#[derive(Clone, Debug)]
pub(crate) struct PublishError {
    pub published: bool,
    pub message: String,
}
impl Destination {
    pub(crate) fn replaces_existing(&self) -> bool {
        self.before.is_some()
    }
    pub(crate) fn validate_backup_owner(&self, root: &Path) -> Result<(), String> {
        if self.extension != "monbackup"
            || self.managed_root != root.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("バックアップの保存先確認が別の保存領域または形式に属しています".into());
        }
        Ok(())
    }
    pub(crate) fn selected(root: &Path, path: &Path) -> Result<Self, String> {
        Self::selected_with(root, path, "md", 64 * 1024 * 1024, &|| false)
    }
    pub(crate) fn selected_backup(
        root: &Path,
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::selected_with(root, path, "monbackup", u64::MAX, cancelled)
    }
    fn selected_with(
        root: &Path,
        path: &Path,
        extension: &str,
        maximum: u64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if !path.is_absolute()
            || path.components().any(|c| matches!(c, Component::ParentDir))
            || !path
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(extension))
        {
            let label = if extension == "md" {
                "Markdown"
            } else {
                "MinutesOnバックアップ"
            };
            return Err(format!("絶対パスの{label}保存先を選択してください"));
        }
        let name = path.file_name().ok_or("保存先ファイル名がありません")?;
        #[cfg(windows)]
        let pins = crate::windows_files::parents(path.parent().ok_or("保存先の親がありません")?)?;
        let parent = path
            .parent()
            .ok_or("保存先フォルダーがありません")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let managed_root = root.canonicalize().map_err(|e| e.to_string())?;
        if parent.starts_with(&managed_root) {
            return Err("アプリ管理データの中へは書き出せません".into());
        }
        let parent_metadata =
            crate::native_fs::symlink_metadata(&parent).map_err(|e| e.to_string())?;
        if !parent_metadata.is_dir() {
            return Err("保存先フォルダーがありません".into());
        }
        let path = parent.join(name);
        let before = fingerprint_with(&path, maximum, cancelled)?;
        Ok(Self {
            #[cfg(windows)]
            _pins: pins,
            path,
            parent,
            parent_metadata,
            before,
            maximum_existing_bytes: maximum,
            managed_root,
            extension: extension.into(),
        })
    }
    pub(crate) fn publish(&self, bytes: &[u8]) -> Result<(), PublishError> {
        self.publish_with(
            |file| file.write_all(bytes).map_err(|e| e.to_string()),
            &|| false,
        )
    }
    pub(crate) fn publish_with(
        &self,
        write: impl FnOnce(&mut File) -> Result<(), String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), PublishError> {
        let mut published = false;
        let result = (|| -> Result<(), String> {
            let mut temporary = tempfile::Builder::new()
                .prefix(".mon-export-")
                .suffix(".part")
                .tempfile_in(&self.parent)
                .map_err(|e| e.to_string())?;
            #[cfg(windows)]
            crate::windows_files::set_private(temporary.path())?;
            write(temporary.as_file_mut())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            // Directory mtime/size changes when our temporary file is created;
            // compare its identity, not those mutable fields.
            let current =
                crate::native_fs::symlink_metadata(&self.parent).map_err(|e| e.to_string())?;
            #[cfg(any(unix, windows))]
            {
                #[cfg(unix)]
                use std::os::unix::fs::MetadataExt;
                if self.parent_metadata.dev() != current.dev()
                    || self.parent_metadata.ino() != current.ino()
                {
                    return Err("保存先フォルダーが変更されました".into());
                }
            }
            if cancelled() {
                return Err("書き出しを中断しました".into());
            }
            if !current.is_dir()
                || fingerprint_with(&self.path, self.maximum_existing_bytes, cancelled)?
                    != self.before
            {
                return Err("保存先のファイルが変更されました。確認し直してください".into());
            }
            if self.before.is_some() {
                temporary.persist(&self.path).map_err(|e| e.to_string())?;
            } else {
                temporary
                    .persist_noclobber(&self.path)
                    .map_err(|e| e.to_string())?;
            }
            published = true;
            crate::native_fs::sync_directory(&self.parent)?;
            Ok(())
        })();
        result.map_err(|message| PublishError { published, message })
    }
}

pub(crate) async fn save_dialog(
    window: &tauri::WebviewWindow,
    filename: String,
) -> Result<Option<PathBuf>, String> {
    save_dialog_with(window, filename, "議事録をMarkdownで保存", "md").await
}

pub(crate) async fn save_dialog_with(
    window: &tauri::WebviewWindow,
    filename: String,
    title: &'static str,
    extension: &'static str,
) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSSavePanel};
                use objc2_foundation::{NSArray, NSString};
                let result = (|| -> Result<Option<PathBuf>, String> {
                    let mtm = MainThreadMarker::new()
                        .ok_or("保存ダイアログはメインスレッドが必要です")?;
                    let panel = NSSavePanel::savePanel(mtm);
                    panel.setTitle(Some(&NSString::from_str(title)));
                    panel.setCanCreateDirectories(true);
                    panel.setAllowsOtherFileTypes(false);
                    #[allow(deprecated)]
                    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                        NSString::from_str(extension),
                    ])));
                    // File-type changes can rewrite the name field. Set the full
                    // default last so a reused panel does not append it twice.
                    panel.setNameFieldStringValue(&NSString::from_str(&filename));
                    let response = panel.runModal();
                    if response == NSModalResponseCancel {
                        return Ok(None);
                    }
                    if response != NSModalResponseOK {
                        return Err("保存ダイアログを完了できませんでした".into());
                    }
                    let url = panel.URL().ok_or("保存先がありません")?;
                    if !url.isFileURL() {
                        return Err("ローカルの保存先を選んでください".into());
                    }
                    let path = url.path().ok_or("保存先パスを読み取れません")?;
                    Ok(Some(PathBuf::from(path.to_string())))
                })();
                let _ = send.send(result);
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            receive
                .recv()
                .map_err(|_| "保存ダイアログの応答を受け取れません".to_owned())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(target_os = "windows")]
    {
        crate::platform_dialog::save(window, &filename, title, extension).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (window, filename, title, extension);
        Err("このOSの保存ダイアログはまだ接続していません".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_excludes_double_export_and_releases_on_failure() {
        let c = ExportController::default();
        let lease = c.begin().unwrap();
        assert!(c.active());
        assert!(c.begin().is_err());
        drop(lease);
        assert!(!c.active());
        assert!(c.begin().is_ok());
    }
    #[test]
    fn publication_is_atomic_and_rejects_changed_or_managed_targets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("a.md");
        let selected = Destination::selected(root.path(), &path).unwrap();
        selected.publish(b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        let selected = Destination::selected(root.path(), &path).unwrap();
        std::fs::write(&path, b"other app").unwrap();
        assert!(selected.publish(b"overwrite").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"other app");
        assert!(Destination::selected(root.path(), &root.path().join("data.md")).is_err());
        assert!(Destination::selected(root.path(), &outside.path().join("data.sqlite3")).is_err());
        let selected = Destination::selected(root.path(), &path).unwrap();
        selected.publish(b"confirmed replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"confirmed replacement");
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn links_and_newly_created_targets_are_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("a.md");
        let selected = Destination::selected(root.path(), &path).unwrap();
        std::fs::write(&path, b"new file").unwrap();
        assert!(selected.publish(b"x").is_err());
        let link = outside.path().join("link.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Destination::selected(root.path(), &link).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::hard_link(&path, outside.path().join("hard.md")).unwrap();
        assert!(Destination::selected(root.path(), &path).is_err());
    }
}
