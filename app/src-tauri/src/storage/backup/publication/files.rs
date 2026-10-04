//! Exclusive new paths, no overwrite, and cleanup of only inode-owned entries.
use super::*;

struct Owned {
    path: PathBuf,
    metadata: Metadata,
    directory: bool,
    proof: Option<(u64, String)>,
}
pub(super) struct Installed {
    root: PathBuf,
    owned: Vec<Owned>,
    keep: bool,
}
fn same_inode(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino() && a.file_type() == b.file_type()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    } // Windows cleanup is deliberately not claimed.
}
fn directory(path: &Path) -> Result<(), String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("追加復元の親フォルダーが変更されています".into());
    }
    Ok(())
}
fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
impl Installed {
    fn create_directory(&mut self, path: PathBuf) -> Result<(), String> {
        directory(path.parent().ok_or("追加復元の親パスがありません")?)?;
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|e| e.to_string())?;
        self.owned.push(Owned {
            metadata: std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?,
            path,
            directory: true,
            proof: None,
        });
        Ok(())
    }
    pub(super) fn prepare(
        root: &Path,
        sealed: &SealedWorkspace,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        directory(root)?;
        let mut install = Self {
            root: root.into(),
            owned: vec![],
            keep: false,
        };
        let parent = root.join("meetings");
        match std::fs::symlink_metadata(&parent) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                install.create_directory(parent.clone())?
            }
            Err(e) => return Err(e.to_string()),
            Ok(_) => directory(&parent)?,
        }
        for meeting in read_meetings(sealed.connection())? {
            check_cancelled(check)?;
            if !Uuid::parse_str(&meeting.id).is_ok_and(|id| id.to_string() == meeting.id) {
                return Err("追加復元の会議IDが不正です".into());
            }
            let path = parent.join(meeting.id);
            install.create_directory(path.clone())?;
            install.create_directory(path.join("restored"))?;
        }
        let source_root = sealed.root_path()?;
        let mut query = sealed.connection().prepare("SELECT id,meeting_id,relative_path,media_type,size_bytes,content_hash,state FROM file_assets WHERE state='ready' ORDER BY id").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(check)?;
            let asset = FileAsset {
                id: row.get(0).map_err(|e| e.to_string())?,
                meeting_id: row.get(1).map_err(|e| e.to_string())?,
                relative_path: row.get(2).map_err(|e| e.to_string())?,
                media_type: row.get(3).map_err(|e| e.to_string())?,
                size_bytes: row.get(4).map_err(|e| e.to_string())?,
                content_hash: row.get(5).map_err(|e| e.to_string())?,
                state: row.get(6).map_err(|e| e.to_string())?,
            };
            if asset.relative_path != format!("meetings/{}/restored/{}", asset.meeting_id, asset.id)
                || asset.size_bytes < 0
            {
                return Err("追加復元の管理パスが不正です".into());
            }
            let target = root.join(&asset.relative_path);
            directory(target.parent().ok_or("追加復元の親パスがありません")?)?;
            let source = source_root.join(&asset.relative_path);
            let (mut input, before) = open_source(&source)?;
            let mut output = private_file(&target)?;
            install.owned.push(Owned {
                metadata: output.metadata().map_err(|e| e.to_string())?,
                path: target,
                directory: false,
                proof: Some((asset.size_bytes as u64, asset.content_hash.clone())),
            });
            let proof = stream(
                &mut input,
                Some(&mut output),
                asset.size_bytes as u64,
                check,
            )?;
            if proof != (asset.size_bytes as u64, asset.content_hash)
                || !same_file(&before, &input.metadata().map_err(|e| e.to_string())?)
                || !same_file(
                    &before,
                    &std::fs::symlink_metadata(&source).map_err(|e| e.to_string())?,
                )
            {
                return Err("追加復元中に管理ファイルが変更されています".into());
            }
            output.sync_all().map_err(|e| e.to_string())?;
        }
        install.verify(check)?;
        for entry in install.owned.iter().rev().filter(|entry| entry.directory) {
            sync_directory(&entry.path)?;
        }
        sync_directory(&parent)?;
        sync_directory(root)?;
        Ok(install)
    }
    pub(super) fn verify(&self, check: &impl Fn() -> bool) -> Result<(), String> {
        directory(&self.root)?;
        for entry in &self.owned {
            check_cancelled(check)?;
            directory(entry.path.parent().ok_or("追加復元の親パスがありません")?)?;
            let now = std::fs::symlink_metadata(&entry.path).map_err(|e| e.to_string())?;
            if !same_inode(&entry.metadata, &now) {
                return Err("追加復元の専用保存先が変更されています".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if now.permissions().mode() & 0o077 != 0 || (!entry.directory && now.nlink() != 1) {
                    return Err("追加復元の非公開権限・リンク数が変更されています".into());
                }
            }
            if let Some(proof) = &entry.proof {
                if digest_file(&entry.path, check)? != *proof {
                    return Err("公開前の管理ファイルのSHA・容量が不正です".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn keep(&mut self) {
        self.keep = true;
    }
}
impl Drop for Installed {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for entry in self.owned.iter().rev() {
            // No recursive deletion: substituted ancestors/inodes and unknown
            // files are never removed, even if this leaves a recoverable orphan.
            let safe = directory(&self.root).is_ok()
                && entry.path.parent().is_some_and(|p| directory(p).is_ok())
                && std::fs::symlink_metadata(&entry.path)
                    .is_ok_and(|m| same_inode(&entry.metadata, &m));
            if safe {
                if entry.directory {
                    let _ = std::fs::remove_dir(&entry.path);
                } else {
                    let _ = std::fs::remove_file(&entry.path);
                }
            }
        }
    }
}
