//! Keep a migration image private from its first byte, then publish without
//! replacing any existing name. SQLite never opens the public destination path.
use crate::native_fs::{self, Metadata};
use crate::storage::deletion::fs::NativeProof as _;
use mon_native::identity::Identity;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn ancestors(path: &Path) -> Result<(), String> {
    let owner = native_fs::current_owner()?;
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let m = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if !m.is_dir()
            || (m.uid() != 0 && m.uid() != owner)
            || m.mode() & 0o022 != 0 && m.mode() & (libc::S_ISVTX as u32) == 0
        {
            return Err("移行前DBコピーの上位フォルダーが他の利用者から置換可能です。既存DBを保持して停止しました".into());
        }
    }
    Ok(())
}

struct Parent {
    path: PathBuf,
    file: File,
    identity: Identity,
    #[cfg(windows)]
    _pins: Vec<File>,
}
impl Parent {
    fn open(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("移行前DBコピーには絶対パスが必要です".into());
        }
        #[cfg(unix)]
        if path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("移行前DBコピーの親にリンクがあります".into());
        }
        #[cfg(unix)]
        ancestors(path)?;
        #[cfg(unix)]
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let pins = crate::windows_files::parents(path)?;
        #[cfg(windows)]
        let file = crate::windows_files::directory(path)?;
        #[cfg(windows)]
        let canonical = crate::windows_files::path(&file)?;
        #[cfg(windows)]
        if path.canonicalize().map_err(|e| e.to_string())? != canonical {
            return Err("移行前DBコピーの親の実体が一致しません".into());
        }
        #[cfg(windows)]
        let path = canonical.as_path();
        let m = native_fs::metadata(&file).map_err(|e| e.to_string())?;
        // Readable existing roots are supported; another account may not be
        // able to replace a staging directory inside the selected root.
        if !m.is_dir() || m.uid() != native_fs::current_owner()? || m.mode() & 0o022 != 0 {
            return Err("移行前DBコピーの保存先が所有者以外から書込み可能です。既存DBを保持して停止しました".into());
        }
        #[cfg(windows)]
        if !crate::windows_files::security_metadata(&file)?.1 {
            return Err("移行前DBコピーの保存先が非公開ではありません".into());
        }
        let parent = Self {
            path: path.into(),
            identity: Identity::read(&m)?,
            file,
            #[cfg(windows)]
            _pins: pins,
        };
        parent.verify()?;
        Ok(parent)
    }
    fn verify(&self) -> Result<(), String> {
        #[cfg(unix)]
        ancestors(&self.path)?;
        let opened = native_fs::metadata(&self.file).map_err(|e| e.to_string())?;
        let named = native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if Identity::read(&opened)? != self.identity
            || Identity::read(&named)? != self.identity
            || opened.uid() != native_fs::current_owner()?
            || opened.mode() & 0o022 != 0
            || self.path.canonicalize().map_err(|e| e.to_string())? != self.path
        {
            return Err("移行前DBコピーの親の実体・権限が変更されています".into());
        }
        #[cfg(windows)]
        if !crate::windows_files::security_metadata(&self.file)?.1 {
            return Err("移行前DBコピーの保存先の非公開権限が変更されています".into());
        }
        Ok(())
    }
    fn sync(&self) -> Result<(), String> {
        self.verify()?;
        native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        self.verify()
    }
}
fn private_file(m: &Metadata) -> Result<(), String> {
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != native_fs::current_owner()?
        || m.mode() & 0o077 != 0
    {
        return Err(
            "既存の移行前DBコピーの実体・非公開権限を確認できません。既存物を保持して停止しました"
                .into(),
        );
    }
    Ok(())
}
fn same_file(file: &File, path: &Path, expected: &Identity) -> Result<Metadata, String> {
    let opened = native_fs::metadata(file).map_err(|e| e.to_string())?;
    let named = native_fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    private_file(&opened)?;
    private_file(&named)?;
    if Identity::read(&opened)? != *expected || Identity::read(&named)? != *expected {
        return Err("移行前DBコピーの実体が変更されています".into());
    }
    Ok(opened)
}
fn digest(
    file: &mut File,
    path: &Path,
    identity: &Identity,
    parent: &Parent,
) -> Result<(u64, String), String> {
    parent.verify()?;
    let before = same_file(file, path, identity)?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut bytes = [0u8; 65_536];
    loop {
        parent.verify()?;
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .filter(|s| *s <= before.len())
            .ok_or("移行前DBコピーの容量が変わりました")?;
        hash.update(&bytes[..n]);
    }
    let after = same_file(file, path, identity)?;
    if size != before.len()
        || after.len() != size
        || before.modified().map_err(|e| e.to_string())?
            != after.modified().map_err(|e| e.to_string())?
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err("移行前DBコピーが読取り中に変更されました".into());
    }
    Ok((size, format!("{:x}", hash.finalize())))
}
fn existing(parent: &Parent, path: &Path) -> Result<Option<(File, Identity)>, String> {
    parent.verify()?;
    let m = match native_fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    private_file(&m)?;
    let identity = Identity::read(&m)?;
    #[cfg(unix)]
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let file = crate::windows_files::read(path, false)?;
    same_file(&file, path, &identity)?;
    Ok(Some((file, identity)))
}

pub(super) fn save(db: &Connection, name: &str) -> Result<(), String> {
    save_with(db, name, |_| Ok(()))
}
fn save_with(
    db: &Connection,
    name: &str,
    before_publish: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if !matches!(
        name,
        "before-single-wav.sqlite3" | "before-followup-intents.sqlite3"
    ) {
        return Err("未知の移行前DBコピー名です".into());
    }
    let path: String = db
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if path.is_empty() {
        return Ok(());
    }
    let parent = Parent::open(Path::new(&path).parent().ok_or("DBの親がありません")?)?;
    let destination = parent.path.join(name);
    let retained = existing(&parent, &destination)?;
    let stage = super::super::backup::scratch::Stage::create(&parent.path)?;
    #[cfg(windows)]
    crate::windows_files::set_private(stage.path())?;
    let stage_directory = crate::model_placement::fs::Directory::open_existing(stage.path())?;
    let mut image = tempfile::Builder::new()
        .prefix("migration-")
        .suffix(".sqlite3")
        .tempfile_in(stage.path())
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    crate::windows_files::set_private(image.path())?;
    private_file(&native_fs::metadata(image.as_file()).map_err(|e| e.to_string())?)?;
    super::super::backup::scratch::record_file(image.path(), image.as_file())?;
    let identity =
        Identity::read(&native_fs::metadata(image.as_file()).map_err(|e| e.to_string())?)?;
    // Windows denies delete sharing for this owned leaf while SQLite opens a
    // second read/write handle; the private parent is pinned on both platforms.
    #[cfg(windows)]
    let leaf_pin = OpenOptions::new()
        .access_mode(0x80 | 0x20000)
        .share_mode(3)
        .custom_flags(0x00200000)
        .open(image.path())
        .map_err(|e| e.to_string())?;
    parent.verify()?;
    stage_directory.verify()?;
    db.backup("main", image.path(), None)
        .map_err(|e| e.to_string())?;
    image.as_file().sync_all().map_err(|e| e.to_string())?;
    parent.verify()?;
    stage_directory.verify()?;
    let image_path = image.path().to_owned();
    let expected = digest(image.as_file_mut(), &image_path, &identity, &parent)?;
    if expected.0 < 100 {
        return Err("移行前DBコピーのSQLiteヘッダーがありません".into());
    }
    if let Some((mut file, observed)) = retained {
        if digest(&mut file, &destination, &observed, &parent)? != expected {
            return Err("既存の移行前DBコピーが現在のDBと一致しません。既存物と会議DBを保持して停止しました".into());
        }
        #[cfg(windows)]
        drop(leaf_pin);
        drop(stage_directory);
        drop(image);
        drop(stage);
        return Ok(());
    }
    before_publish(&destination)?;
    parent.verify()?;
    stage_directory.verify()?;
    same_file(image.as_file(), image.path(), &identity)?;
    #[cfg(windows)]
    drop(leaf_pin);
    // No-replace publication is atomic for the name. A late ordinary file,
    // hard link or symlink is retained; it is never passed to SQLite or opened
    // for writing. The owned image retains its 0600/private DACL on publication.
    let published = image.persist_noclobber(&destination).map_err(|e| {
        format!(
            "移行前DBコピーの保存先に既存物があります。上書きせず停止しました: {}",
            e.error
        )
    })?;
    published.sync_all().map_err(|e| e.to_string())?;
    same_file(&published, &destination, &identity)?;
    parent.sync()?;
    drop(published);
    drop(stage_directory);
    drop(stage);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{verify_migration_history, MIGRATIONS};
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn source(root: &Path, version: usize) -> Connection {
        let path = root.join("meetings.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);").unwrap();
        for &(version, sql) in &MIGRATIONS[..version] {
            db.execute_batch(sql).unwrap();
            db.execute(
                "INSERT INTO schema_migrations VALUES(?1,?2,0)",
                rusqlite::params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
            )
            .unwrap();
        }
        db.execute_batch("PRAGMA journal_mode=WAL; INSERT INTO meetings(id,title,created_at,updated_at) VALUES('private-copy','synthetic secret',1,1);").unwrap();
        #[cfg(unix)]
        fs_private(&path);
        db
    }
    #[cfg(unix)]
    fn fs_private(path: &Path) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[test]
    fn private_snapshot_includes_wal_and_reuses_only_the_identical_saved_image() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        #[cfg(windows)]
        crate::windows_files::set_private(root.path()).unwrap();
        let db = source(root.path(), 35);
        let name = "before-followup-intents.sqlite3";
        save(&db, name).unwrap();
        let path = root.path().join(name);
        let before = native_fs::symlink_metadata(&path).unwrap();
        private_file(&before).unwrap();
        assert_eq!(before.mode() & 0o777, 0o600);
        let copy =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(verify_migration_history(&copy).unwrap(), 35);
        assert_eq!(
            copy.query_row(
                "SELECT title FROM meetings WHERE id='private-copy'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "synthetic secret"
        );
        drop(copy);
        save(&db, name).unwrap();
        assert_eq!(
            Identity::read(&native_fs::symlink_metadata(&path).unwrap()).unwrap(),
            Identity::read(&before).unwrap()
        );
        db.execute("UPDATE meetings SET title='newer synthetic text'", [])
            .unwrap();
        assert!(
            save(&db, name).is_err(),
            "a different retained image is never overwritten"
        );
        assert_eq!(verify_migration_history(&db).unwrap(), 35);
    }
    #[test]
    fn unknown_existing_file_is_preserved_and_never_authorizes_migration() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        crate::windows_files::set_private(root.path()).unwrap();
        let db = source(root.path(), 35);
        let path = root.path().join("before-followup-intents.sqlite3");
        std::fs::write(&path, b"unrelated existing file").unwrap();
        #[cfg(unix)]
        fs_private(&path);
        assert!(save(&db, "before-followup-intents.sqlite3").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"unrelated existing file");
        assert_eq!(verify_migration_history(&db).unwrap(), 35);
    }
    #[test]
    #[cfg(unix)]
    fn links_and_nonprivate_copies_are_preserved_without_writing_their_targets() {
        for kind in ["dangling", "symlink", "hardlink", "public"] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let db = source(root.path(), 35);
            let target = outside.path().join("retained.sqlite3");
            let path = root.path().join("before-followup-intents.sqlite3");
            match kind {
                "dangling" => symlink(&target, &path).unwrap(),
                "symlink" => {
                    std::fs::write(&target, b"outside retained bytes").unwrap();
                    symlink(&target, &path).unwrap();
                }
                "hardlink" => {
                    std::fs::write(&target, b"outside retained bytes").unwrap();
                    fs_private(&target);
                    std::fs::hard_link(&target, &path).unwrap();
                }
                "public" => {
                    std::fs::write(&path, b"public retained bytes").unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                save(&db, "before-followup-intents.sqlite3").is_err(),
                "{kind}"
            );
            if kind == "dangling" {
                assert!(!target.exists());
                assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
            }
            if matches!(kind, "symlink" | "hardlink") {
                assert_eq!(std::fs::read(&target).unwrap(), b"outside retained bytes");
            }
            if kind == "public" {
                assert_eq!(std::fs::read(&path).unwrap(), b"public retained bytes");
            }
            assert_eq!(verify_migration_history(&db).unwrap(), 35);
        }
    }
    #[test]
    #[cfg(unix)]
    fn late_destination_replacement_is_not_followed_or_overwritten() {
        for linked in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let db = source(root.path(), 35);
            let target = outside.path().join("retained.sqlite3");
            std::fs::write(&target, b"retained bytes").unwrap();
            assert!(
                save_with(&db, "before-followup-intents.sqlite3", |destination| {
                    if linked {
                        symlink(&target, destination).map_err(|e| e.to_string())?;
                    } else {
                        std::fs::write(destination, b"new foreign bytes")
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
                .is_err()
            );
            assert_eq!(std::fs::read(&target).unwrap(), b"retained bytes");
            if !linked {
                assert_eq!(
                    std::fs::read(root.path().join("before-followup-intents.sqlite3")).unwrap(),
                    b"new foreign bytes"
                );
            }
            assert_eq!(verify_migration_history(&db).unwrap(), 35);
        }
    }
    #[test]
    fn interruption_before_publication_keeps_source_and_allows_a_fresh_retry() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        crate::windows_files::set_private(root.path()).unwrap();
        let db = source(root.path(), 34);
        assert!(save_with(&db, "before-single-wav.sqlite3", |_| Err(
            "synthetic interruption".into()
        ))
        .is_err());
        assert!(!root.path().join("before-single-wav.sqlite3").exists());
        assert_eq!(verify_migration_history(&db).unwrap(), 34);
        save(&db, "before-single-wav.sqlite3").unwrap();
        assert_eq!(verify_migration_history(&db).unwrap(), 34);
    }
    #[test]
    #[cfg(unix)]
    fn shared_nonsticky_ancestor_is_refused_but_owned_sticky_ancestor_is_supported() {
        let area = tempfile::tempdir().unwrap();
        let shared = area.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        let root = shared.join("storage");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db = source(&root, 35);
        assert!(save(&db, "before-followup-intents.sqlite3").is_err());
        assert!(!root.join("before-followup-intents.sqlite3").exists());
        assert_eq!(verify_migration_history(&db).unwrap(), 35);
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
        save(&db, "before-followup-intents.sqlite3").unwrap();
        private_file(
            &native_fs::symlink_metadata(root.join("before-followup-intents.sqlite3")).unwrap(),
        )
        .unwrap();
    }
}
