use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
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
    size: u64,
    modified: std::time::SystemTime,
    hash: String,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
fn fingerprint(path: &Path) -> Result<Option<Fingerprint>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(
            "保存先は通常のMarkdownファイルにしてください（リンクやフォルダーは不可）".into(),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("複数の場所を参照するファイルには書き出せません".into());
        }
    }
    if metadata.len() > 64 * 1024 * 1024 {
        return Err("置換対象のファイルが大きすぎます".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if !same_file(&metadata, &file.metadata().map_err(|e| e.to_string())?) {
        return Err("保存先が変更されました".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > 64 * 1024 * 1024 {
            return Err("保存先が変更されました".into());
        }
        hash.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if !same_file(&metadata, &after) || bytes != metadata.len() {
        return Err("保存先が変更されました".into());
    }
    Ok(Some(Fingerprint {
        size: bytes,
        modified: metadata.modified().map_err(|e| e.to_string())?,
        hash: format!("{:x}", hash.finalize()),
        #[cfg(unix)]
        device: {
            use std::os::unix::fs::MetadataExt;
            metadata.dev()
        },
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        },
    }))
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
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
    pub path: PathBuf,
    parent: PathBuf,
    parent_metadata: Metadata,
    before: Option<Fingerprint>,
}
#[derive(Debug)]
pub(crate) struct PublishError {
    pub published: bool,
    pub message: String,
}
impl Destination {
    pub(crate) fn selected(root: &Path, path: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path.components().any(|c| matches!(c, Component::ParentDir))
            || !path
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("md"))
        {
            return Err("絶対パスのMarkdown保存先を選択してください".into());
        }
        let name = path.file_name().ok_or("保存先ファイル名がありません")?;
        let parent = path
            .parent()
            .ok_or("保存先フォルダーがありません")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if parent.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
            return Err("アプリ管理データの中へは書き出せません".into());
        }
        let parent_metadata = std::fs::metadata(&parent).map_err(|e| e.to_string())?;
        if !parent_metadata.is_dir() {
            return Err("保存先フォルダーがありません".into());
        }
        let path = parent.join(name);
        let before = fingerprint(&path)?;
        Ok(Self {
            path,
            parent,
            parent_metadata,
            before,
        })
    }
    pub(crate) fn publish(&self, bytes: &[u8]) -> Result<(), PublishError> {
        let mut published = false;
        let result = (|| -> Result<(), String> {
            let mut temporary = tempfile::Builder::new()
                .prefix(".mon-export-")
                .suffix(".part")
                .tempfile_in(&self.parent)
                .map_err(|e| e.to_string())?;
            temporary.write_all(bytes).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            // Directory mtime/size changes when our temporary file is created;
            // compare its identity, not those mutable fields.
            let current = std::fs::metadata(&self.parent).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if self.parent_metadata.dev() != current.dev()
                    || self.parent_metadata.ino() != current.ino()
                {
                    return Err("保存先フォルダーが変更されました".into());
                }
            }
            if !current.is_dir() || fingerprint(&self.path)? != self.before {
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
            #[cfg(unix)]
            File::open(&self.parent)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        result.map_err(|message| PublishError { published, message })
    }
}

pub(crate) async fn save_dialog(
    window: &tauri::WebviewWindow,
    filename: String,
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
                    panel.setTitle(Some(&NSString::from_str("議事録をMarkdownで保存")));
                    panel.setNameFieldStringValue(&NSString::from_str(&filename));
                    panel.setCanCreateDirectories(true);
                    panel.setAllowsOtherFileTypes(false);
                    #[allow(deprecated)]
                    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                        NSString::from_str("md"),
                    ])));
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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, filename);
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
