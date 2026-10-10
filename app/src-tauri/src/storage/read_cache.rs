//! Private WAL inspection copies live outside the inspected meeting tree.
//! Production callers hold and verify the selected bootstrap's OS lock.
//! The existing inode journal records empty files BEFORE any copied payload.
use super::backup::scratch::{self, Registry, Stage};
use std::fs::File;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

const NAME: &str = ".mon-read-cache";

fn open(config: &Path, create: bool) -> Result<Option<Arc<Registry>>, String> {
    if !config.is_absolute() || config.canonicalize().map_err(|e| e.to_string())? != config {
        return Err("読取用一時領域の起動設定の親が変更されています".into());
    }
    let path = config.join(NAME);
    let new = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !create {
                return Ok(None);
            }
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|e| e.to_string())?;
            File::open(config)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
            true
        }
        Ok(_) => false,
        Err(e) => return Err(e.to_string()),
    };
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("読取用一時領域の実体・非公開権限が不正です。既存対象を保持します".into());
    }
    // An initialization gap contains no payload. Do not adopt that directory,
    // unknown content, or a marker/name pattern as permission to erase files.
    if !new && std::fs::symlink_metadata(path.join(".mon-scratch.sqlite3")).is_err() {
        return Err("読取用一時領域の所有台帳がありません。既存対象を保持します".into());
    }
    let registry = Registry::open(&path)?;
    registry.require_no_attention()?;
    Ok(Some(registry))
}

pub(crate) fn recover(config: &Path) -> Result<(), String> {
    drop(open(config, false)?);
    Ok(())
}

pub(crate) struct Snapshot {
    stage: Stage,
}
impl Snapshot {
    pub(crate) fn create(config: &Path) -> Result<Self, String> {
        let registry = open(config, true)?.ok_or("読取用一時領域がありません")?;
        Ok(Self {
            stage: registry.stage()?,
        })
    }
    pub(crate) fn path(&self) -> &Path {
        self.stage.path()
    }
    pub(crate) fn create_file(&self, name: &str) -> Result<File, String> {
        if !matches!(
            name,
            "meetings.sqlite3" | "meetings.sqlite3-wal" | "meetings.sqlite3-shm"
        ) {
            return Err("読取用コピーへ未知の制御ファイルを採用しません".into());
        }
        let path = self.path().join(name);
        scratch::check_creation(&path)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|e| e.to_string())?;
        scratch::record_file(&path, &file)?;
        File::open(self.path())
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests;
