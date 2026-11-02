//! Native relocation image, not a portable archive or permission to cut over.
//! Preserve every DB row/ID/local condition and every file (even unregistered
//! or unfinished audio) below meetings/ and temp/. Root control journals/models
//! stay at the source; their rebinding is a separate, still-required operation.
use super::*;
#[cfg(windows)]
use crate::bootstrap::windows_io as libc;
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

pub(crate) mod control;

const MAX_ENTRIES: usize = 200_000;
const MAX_ROWS: usize = 2_000_000;
const MAX_MANIFEST: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Table {
    pub name: String,
    pub rows: usize,
    pub hash: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Entry {
    pub relative_path: String,
    pub directory: bool,
    pub size: u64,
    pub hash: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Asset {
    pub id: String,
    pub meeting_id: String,
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
    pub state: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub version: u32,
    pub schema_hash: String,
    pub tables: Vec<Table>,
    pub assets: Vec<Asset>,
    pub entries: Vec<Entry>,
    pub database_size: u64,
    pub database_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity(u64, u64, std::time::SystemTime);
impl Identity {
    fn read(m: &Metadata) -> Result<Self, String> {
        Ok(Self(
            m.dev(),
            m.ino(),
            m.created().map_err(|e| e.to_string())?,
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Proof {
    identity: Identity,
    mode: u32,
    links: u64,
    size: u64,
    modified: std::time::SystemTime,
    change: (i64, i64),
}
impl Proof {
    fn read(m: &Metadata) -> Result<Self, String> {
        Ok(Self {
            identity: Identity::read(m)?,
            mode: m.mode(),
            links: m.nlink(),
            size: m.len(),
            modified: m.modified().map_err(|e| e.to_string())?,
            change: (m.ctime(), m.ctime_nsec()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Observed {
    entry: Entry,
    proof: Proof,
}

// All fields stay private: no deserialized manifest or user success flag can
// construct a verified image. The write fence lives until the image is dropped.
pub(crate) struct NativeSnapshot<'a> {
    stage: scratch::Stage,
    image: PathBuf,
    manifest: Manifest,
    manifest_hash: String,
    manifest_proof: Proof,
    database_proof: Proof,
    source_identity: Identity,
    source_database_proof: Proof,
    stage_identity: Identity,
    image_identity: Identity,
    source: Vec<Observed>,
    copied: Vec<Observed>,
    maintenance: Maintenance<'a>,
}

impl Drop for NativeSnapshot<'_> {
    fn drop(&mut self) {
        // Cancellation, stale confirmation or a bootstrap error can return
        // before verify_source. Never unlock an uncertain root/DB merely
        // because that earlier check ended the operation. Stage cleanup still
        // runs before Maintenance, and is independently restricted to ownership.
        let storage = self.maintenance.storage;
        let valid = (|| -> Result<bool, String> {
            Ok(
                Identity::read(&directory(&storage.root)?)? == self.source_identity
                    && database_proof(&storage.root.join("meetings.sqlite3"))?
                        == self.source_database_proof,
            )
        })();
        if !valid.is_ok_and(|valid| valid) {
            self.maintenance.fail_closed();
        }
    }
}

fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("保存先移動の準備を中断しました。旧保存先は維持します".into())
    } else {
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn directory(path: &Path) -> Result<Metadata, String> {
    let m = crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_dir() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err("移動対象のフォルダーまたは親が変更されています".into());
    }
    Ok(m)
}
fn database_proof(path: &Path) -> Result<Proof, String> {
    let m = crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.nlink() != 1 {
        return Err("旧保存先のDBは単独の通常ファイルである必要があります".into());
    }
    Proof::read(&m)
}
fn relative(path: &str) -> Result<(), String> {
    if path.len() > 1024
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || Path::new(path)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("移動対象の相対パスが不正です".into());
    }
    Ok(())
}

// Type/length framing distinguishes NULL, TEXT, BLOB, numeric types and field
// boundaries. Sorting row digests preserves duplicates without depending on
// rowid or a portable allowlist. This includes all native/local tables.
fn table(db: &Connection, name: &str, cancelled: &impl Fn() -> bool) -> Result<Table, String> {
    let escaped = name.replace('"', "\"\"");
    let mut statement = db
        .prepare(&format!("SELECT * FROM \"{escaped}\""))
        .map_err(|e| e.to_string())?;
    let columns = statement.column_count();
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut hashes = Vec::<[u8; 32]>::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check(cancelled)?;
        if hashes.len() >= MAX_ROWS {
            return Err("移動対象のDB行数が安全上限を超えています".into());
        }
        let mut hash = Sha256::new();
        hash.update((columns as u64).to_le_bytes());
        for index in 0..columns {
            let value = row.get_ref(index).map_err(|e| e.to_string())?;
            match value {
                ValueRef::Null => hash.update([0]),
                ValueRef::Integer(n) => {
                    hash.update([1]);
                    hash.update(n.to_le_bytes());
                }
                ValueRef::Real(n) => {
                    hash.update([2]);
                    hash.update(n.to_bits().to_le_bytes());
                }
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                    hash.update([if matches!(value, ValueRef::Text(_)) {
                        3
                    } else {
                        4
                    }]);
                    hash.update((bytes.len() as u64).to_le_bytes());
                    hash.update(bytes);
                }
            }
        }
        hashes.push(hash.finalize().into());
    }
    hashes.sort_unstable();
    let mut hash = Sha256::new();
    hash.update((hashes.len() as u64).to_le_bytes());
    for row in &hashes {
        hash.update(row);
    }
    Ok(Table {
        name: name.into(),
        rows: hashes.len(),
        hash: format!("{:x}", hash.finalize()),
    })
}
fn census(db: &Connection, cancelled: &impl Fn() -> bool) -> Result<(String, Vec<Table>), String> {
    check(cancelled)?;
    let schema = table(db, "sqlite_schema", cancelled)?;
    let names = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if names.len() > 1024 {
        return Err("移動対象のDB表数が安全上限を超えています".into());
    }
    let tables = names
        .iter()
        .map(|name| table(db, name, cancelled))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((schema.hash, tables))
}

fn inspect_file(
    path: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<(Proof, u64, String), String> {
    directory(path.parent().ok_or("移動ファイルの親がありません")?)?;
    let m = crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.nlink() != 1 {
        return Err("移動対象のリンク・特殊ファイルは採用しません".into());
    }
    let proof = Proof::read(&m)?;
    let (size, hash) = digest_file(path, cancelled)?;
    if Proof::read(&crate::native_fs::symlink_metadata(path).map_err(|e| e.to_string())?)? != proof
    {
        return Err("移動対象の実体が検査中に変更されています".into());
    }
    Ok((proof, size, hash))
}
fn inventory(root: &Path, cancelled: &impl Fn() -> bool) -> Result<Vec<Observed>, String> {
    directory(root)?;
    let mut queue = Vec::new();
    for base in ["meetings", "temp", "voices"] {
        match crate::native_fs::symlink_metadata(root.join(base)) {
            Ok(_) => queue.push(base.to_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut found = Vec::new();
    while let Some(name) = queue.pop() {
        check(cancelled)?;
        relative(&name)?;
        if found.len() >= MAX_ENTRIES || name.split('/').count() > 128 {
            return Err("移動対象のファイル数・深さが安全上限を超えています".into());
        }
        let path = crate::native_fs::join(&root, &name);
        directory(path.parent().ok_or("移動対象の親がありません")?)?;
        let m = crate::native_fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let proof = Proof::read(&m)?;
        let (is_directory, size, hash) = if m.is_dir() {
            directory(&path)?;
            for child in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?;
                let child_name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| "移動対象にはUTF-8のファイル名が必要です")?;
                if queue.len() + found.len() >= MAX_ENTRIES {
                    return Err("移動対象のファイル数が安全上限を超えています".into());
                }
                queue.push(format!("{name}/{child_name}"));
            }
            if Proof::read(&directory(&path)?)? != proof {
                return Err("移動対象のフォルダーが検査中に変更されています".into());
            }
            (true, 0, None)
        } else {
            let (checked, size, hash) = inspect_file(&path, cancelled)?;
            if checked != proof {
                return Err("移動対象のファイルが検査中に変更されています".into());
            }
            (false, size, Some(hash))
        };
        found.push(Observed {
            entry: Entry {
                relative_path: name,
                directory: is_directory,
                size,
                hash,
            },
            proof,
        });
    }
    found.sort_by(|a, b| a.entry.relative_path.cmp(&b.entry.relative_path));
    Ok(found)
}
fn assets(db: &Connection, tree: &[Observed]) -> Result<Vec<Asset>, String> {
    let files: BTreeMap<_, _> = tree
        .iter()
        .map(|n| (n.entry.relative_path.as_str(), &n.entry))
        .collect();
    let mut query = db.prepare("SELECT id,meeting_id,relative_path,size_bytes,content_hash,state FROM file_assets ORDER BY id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if all.len() >= MAX_ENTRIES {
            return Err("移動対象の管理ファイル数が安全上限を超えています".into());
        }
        let asset = Asset {
            id: row.get(0).map_err(|e| e.to_string())?,
            meeting_id: row.get(1).map_err(|e| e.to_string())?,
            relative_path: row.get(2).map_err(|e| e.to_string())?,
            size: u64::try_from(row.get::<_, i64>(3).map_err(|e| e.to_string())?)
                .map_err(|_| "管理ファイルサイズが不正です")?,
            hash: row.get(4).map_err(|e| e.to_string())?,
            state: row.get(5).map_err(|e| e.to_string())?,
        };
        relative(&asset.relative_path)?;
        let parts: Vec<_> = asset.relative_path.split('/').collect();
        if parts.len() < 3 || parts[0] != "meetings" || parts[1] != asset.meeting_id {
            return Err("移動対象の管理ファイルが会議領域外を参照しています".into());
        }
        match (
            asset.state.as_str(),
            files.get(asset.relative_path.as_str()),
        ) {
            ("ready", Some(e))
                if !e.directory && e.size == asset.size && e.hash.as_ref() == Some(&asset.hash) => {
            }
            ("missing", None) => {}
            // Preserve broken/awaiting-delete bytes without changing their DB
            // state or pretending they are verified ready audio.
            ("corrupt" | "delete_pending", Some(e)) if !e.directory => {}
            ("corrupt" | "delete_pending", None) => {}
            _ => return Err("移動対象の管理ファイルと登録状態が一致しません".into()),
        }
        all.push(asset);
    }
    for asset in super::super::voice::audio::assets(db)? {
        let e=files.get(asset.relative_path.as_str()).ok_or("移動対象の登録音声がありません")?;
        if e.directory || e.size!=asset.size_bytes as u64 || e.hash.as_deref()!=Some(&asset.content_hash) {return Err("移動対象の登録音声が一致しません".into());}
        all.push(Asset{id:asset.id,meeting_id:String::new(),relative_path:asset.relative_path,size:e.size,hash:asset.content_hash,state:asset.state});
    }
    all.sort_by(|a,b|a.id.cmp(&b.id));
    Ok(all)
}

fn copy_file(
    source: &Path,
    target: &Path,
    node: &Observed,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    directory(source.parent().ok_or("移動元の親がありません")?)?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(source)
        .map_err(|e| e.to_string())?;
    if Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)? != node.proof {
        return Err("移動元のファイルが確認後に変更されています".into());
    }
    let mut output = private_file(target)?;
    let (_, hash) = stream(&mut input, Some(&mut output), node.entry.size, cancelled)?;
    if Some(&hash) != node.entry.hash.as_ref()
        || Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)?
            != node.proof
        || Proof::read(&crate::native_fs::symlink_metadata(source).map_err(|e| e.to_string())?)?
            != node.proof
    {
        return Err("移動元の内容・実体がコピー中に変更されています".into());
    }
    directory(source.parent().ok_or("移動元の親がありません")?)?;
    crate::native_fs::sync_handle(&output).map_err(|e| e.to_string())
}

impl Storage {
    #[allow(dead_code)] // Controller/cutover integration follows this primitive.
    pub(crate) fn prepare_relocation_snapshot(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<NativeSnapshot<'_>, String> {
        check(cancelled)?;
        let maintenance = self.backup_maintenance()?;
        let source_identity = Identity::read(&directory(&self.root)?)?;
        let source_database_proof = database_proof(&self.root.join("meetings.sqlite3"))?;
        let stage = self.backup_stage()?;
        let image = stage.path().join("native");
        scratch::directories(&image)?;
        let database = image.join("meetings.sqlite3");
        snapshot_database(self, &database, cancelled)?;
        let mut db = read_only(&database)?;
        validate_database(&mut db, cancelled)?;
        let (schema_hash, tables) = census(&db, cancelled)?;
        let source = inventory(&self.root, cancelled)?;
        let assets = assets(&db, &source)?;
        let bytes = source.iter().try_fold(0u64, |sum, n| {
            sum.checked_add(n.entry.size).ok_or("移動容量が範囲外です")
        })?;
        check_capacity(&image, bytes)?;
        for node in &source {
            check(cancelled)?;
            let path = crate::native_fs::join(&image, &node.entry.relative_path);
            if node.entry.directory {
                scratch::directories(&path)?;
            } else {
                copy_file(
                    &crate::native_fs::join(&self.root, &node.entry.relative_path),
                    &path,
                    node,
                    cancelled,
                )?;
            }
        }
        // Synchronize child names before the image root. No source checkpoint,
        // removal, ID remapping, model redaction, or old-root mutation occurs.
        for node in source.iter().rev().filter(|n| n.entry.directory) {
            crate::native_fs::sync_path(&crate::native_fs::join(
                &image,
                &node.entry.relative_path,
            ))?;
        }
        crate::native_fs::sync_path(&&image)?;
        let copied = inventory(&image, cancelled)?;
        if source
            .iter()
            .map(|n| &n.entry)
            .ne(copied.iter().map(|n| &n.entry))
        {
            return Err("移動用コピーの件数・相対パス・ハッシュが一致しません".into());
        }
        let (database_size, database_hash) = digest_file(&database, cancelled)?;
        let manifest = Manifest {
            version: 1,
            schema_hash,
            tables,
            assets,
            entries: source.iter().map(|n| n.entry.clone()).collect(),
            database_size,
            database_hash,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("移動manifestが安全上限を超えています".into());
        }
        let path = stage.path().join("relocation-manifest.json");
        let mut saved = private_file(&path)?;
        saved.write_all(&bytes).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&saved).map_err(|e| e.to_string())?;
        let manifest_proof =
            Proof::read(&crate::native_fs::metadata(&saved).map_err(|e| e.to_string())?)?;
        crate::native_fs::sync_path(&stage.path())?;
        let database_proof = Proof::read(
            &crate::native_fs::symlink_metadata(&database).map_err(|e| e.to_string())?,
        )?;
        let snapshot = NativeSnapshot {
            stage_identity: Identity::read(&directory(stage.path())?)?,
            image_identity: Identity::read(&directory(&image)?)?,
            stage,
            image,
            manifest,
            manifest_hash: digest(&bytes),
            manifest_proof,
            database_proof,
            source_identity,
            source_database_proof,
            source,
            copied,
            maintenance,
        };
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        Ok(snapshot)
    }
}

#[allow(dead_code)]
impl NativeSnapshot<'_> {
    // The future cutover coordinator must call this BEFORE attempting bootstrap
    // publication, after verifying destination ownership/history. This is only
    // a source fence, not proof that a target can be activated. No undo method:
    // publication errors can be ambiguous, so only restart can restore writes.
    pub(crate) fn retire_source_for_restart(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.verify(cancelled)?;
        self.verify_source(cancelled)?;
        let storage = self.maintenance.storage;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        check(cancelled)?;
        if self.maintenance.unresolved.get()
            || !storage.maintenance.load(Ordering::Acquire)
            || !db.is_autocommit()
        {
            return Err("旧保存先の検証済み書込停止を確認できません".into());
        }
        db.flush_prepared_statement_cache();
        storage.retired.store(true, Ordering::Release);
        Ok(())
    }
    pub(crate) fn source_root(&self) -> &Path {
        &self.maintenance.storage.root
    }
    pub(in crate::storage) fn native_models_root(&self) -> &Path {
        &self.maintenance.storage.native_models_root
    }
    pub(crate) fn source_metadata(&self, path: &str) -> Result<Option<Metadata>, String> {
        let Ok(index) = self
            .source
            .binary_search_by(|n| n.entry.relative_path.as_str().cmp(path))
        else {
            return Ok(None);
        };
        let node = &self.source[index];
        let metadata = crate::native_fs::symlink_metadata(self.source_root().join(path))
            .map_err(|e| e.to_string())?;
        if Proof::read(&metadata)? != node.proof {
            return Err("元の所有対象の実体・内容が変更されています".into());
        }
        Ok(Some(metadata))
    }
    // The destination owns/records its newly-created FD before calling this.
    // Only a sealed native-image file can supply bytes, never a caller path.
    pub(crate) fn write_placement_file(
        &self,
        name: &str,
        output: &mut File,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let (proof, size, hash) = if name == "meetings.sqlite3" {
            (
                &self.database_proof,
                self.manifest.database_size,
                &self.manifest.database_hash,
            )
        } else {
            let index = self
                .copied
                .binary_search_by(|n| n.entry.relative_path.as_str().cmp(name))
                .map_err(|_| "未確認の移動ファイルをコピーできません")?;
            let n = &self.copied[index];
            if n.entry.directory {
                return Err("フォルダーを音声としてコピーできません".into());
            }
            (
                &n.proof,
                n.entry.size,
                n.entry
                    .hash
                    .as_ref()
                    .ok_or("移動ファイルのSHAがありません")?,
            )
        };
        let source = crate::native_fs::join(&self.image, name);
        directory(source.parent().ok_or("移動コピーの親がありません")?)?;
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&source)
            .map_err(|e| e.to_string())?;
        if Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)? != *proof {
            return Err("検証済みコピーが配置前に変更されています".into());
        }
        check(cancelled)?;
        output.set_len(0).map_err(|e| e.to_string())?;
        std::io::Seek::seek(output, std::io::SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (_, actual) = stream(&mut input, Some(output), size, cancelled)?;
        if actual != *hash
            || Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)?
                != *proof
            || Proof::read(
                &crate::native_fs::symlink_metadata(&source).map_err(|e| e.to_string())?,
            )? != *proof
        {
            return Err("検証済みコピーの配置中に内容・実体が変更されています".into());
        }
        crate::native_fs::sync_handle(&output).map_err(|e| e.to_string())
    }
    pub(crate) fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub(crate) fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }
    pub(crate) fn image(&self) -> &Path {
        &self.image
    }
    pub(crate) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        check(cancelled)?;
        for (path, identity) in [
            (self.stage.path(), &self.stage_identity),
            (self.image.as_path(), &self.image_identity),
        ] {
            let m = directory(path)?;
            if Identity::read(&m)? != *identity || m.mode() & 0o077 != 0 {
                return Err("移動用領域の実体・非公開権限が変更されています".into());
            }
        }
        let path = self.stage.path().join("relocation-manifest.json");
        let (proof, _, hash) = inspect_file(&path, cancelled)?;
        if proof != self.manifest_proof || hash != self.manifest_hash {
            return Err("移動manifestが変更されています".into());
        }
        let (proof, size, hash) = inspect_file(&self.image.join("meetings.sqlite3"), cancelled)?;
        if proof != self.database_proof
            || (size, hash)
                != (
                    self.manifest.database_size,
                    self.manifest.database_hash.clone(),
                )
        {
            return Err("移動用DBの内容・実体が変更されています".into());
        }
        if inventory(&self.image, cancelled)? != self.copied {
            return Err("移動用ファイルの内容・実体・件数が変更されています".into());
        }
        // Refuse any additional payload at the image root, including SQLite WAL
        // files. Only the sealed single DB image and managed trees belong here.
        for name in std::fs::read_dir(&self.image).map_err(|e| e.to_string())? {
            let name = name.map_err(|e| e.to_string())?.file_name();
            if name != "meetings.sqlite3" && name != "meetings" && name != "temp" && name != "voices" {
                return Err("移動用領域に未確認のファイルがあります".into());
            }
        }
        let mut db = read_only(&self.image.join("meetings.sqlite3"))?;
        validate_database(&mut db, cancelled)?;
        if census(&db, cancelled)?
            != (
                self.manifest.schema_hash.clone(),
                self.manifest.tables.clone(),
            )
            || assets(&db, &self.copied)? != self.manifest.assets
        {
            return Err("移動用DBの件数・内容・管理パスが一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn verify_source(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let storage = self.maintenance.storage;
        check(cancelled)?;
        let bound = (|| -> Result<bool, String> {
            Ok(
                Identity::read(&directory(&storage.root)?)? == self.source_identity
                    && database_proof(&storage.root.join("meetings.sqlite3"))?
                        == self.source_database_proof,
            )
        })();
        if !bound.is_ok_and(|valid| valid) {
            // The open SQLite connection might still address a removed inode
            // or cached pages. Dropping a failed copy must NOT re-enable writes
            // to an uncertain root/DB. Reopening through bootstrap is required.
            self.maintenance.fail_closed();
            return Err("旧保存先またはDBが変更されています。書込停止を維持します".into());
        }
        if inventory(&storage.root, cancelled)? != self.source {
            return Err("旧保存先の実体・ファイルが確認後に変更されています".into());
        }
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if census(&db, cancelled)?
            != (
                self.manifest.schema_hash.clone(),
                self.manifest.tables.clone(),
            )
        {
            self.maintenance.fail_closed();
            return Err("旧保存先のDBが確認後に変更されています".into());
        }
        Ok(())
    }
}

// Content verification only: placement also needs its durable ownership proofs,
// bootstrap binding and a held/rechecked source fence. Not a cutover capability.
pub(crate) fn verify_placed_image(
    root: &Path,
    expected: &Manifest,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    verify_placed_image_with_controls(root, expected, cancelled, &[])
}
pub(crate) fn verify_placed_image_with_controls(
    root: &Path,
    expected: &Manifest,
    cancelled: &impl Fn() -> bool,
    controls: &[&str],
) -> Result<(), String> {
    directory(root)?;
    let copied = inventory(root, cancelled)?;
    if copied.iter().map(|n| &n.entry).ne(expected.entries.iter())
        || digest_file(&root.join("meetings.sqlite3"), cancelled)?
            != (expected.database_size, expected.database_hash.clone())
    {
        return Err("配置先の件数・相対パス・SHAが一致しません".into());
    }
    for name in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let name = name.map_err(|e| e.to_string())?.file_name();
        if name != "meetings.sqlite3"
            && name != "meetings"
            && name != "temp"
            && name != "voices"
            && !controls.iter().any(|s| name == *s)
        {
            return Err("配置先に未確認のファイルがあります".into());
        }
    }
    let mut db = read_only(&root.join("meetings.sqlite3"))?;
    validate_database(&mut db, cancelled)?;
    if census(&db, cancelled)? != (expected.schema_hash.clone(), expected.tables.clone())
        || assets(&db, &copied)? != expected.assets
    {
        return Err("配置先DBの件数・内容・管理参照が一致しません".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
