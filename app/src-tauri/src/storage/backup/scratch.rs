//! Local ownership journal for private backup/restore construction, not portable
//! content. Record newly-created inodes before writing payloads. Never adopt an
//! existing path, scan a name pattern for deletion, or recursively remove a tree.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, Weak};

#[cfg(target_os = "macos")]
pub(in crate::storage::backup) mod binding;
#[cfg(target_os = "macos")]
pub(in crate::storage::backup) mod history;
pub(super) mod review;
#[cfg(target_os = "macos")]
pub(in crate::storage::backup) mod transfer;

fn local_version() -> i64 {
    if cfg!(target_os = "macos") {
        4
    } else {
        2
    }
}
fn history_schema(db: &Connection) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    db.execute_batch(history::SCHEMA)
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    db.execute_batch(binding::SCHEMA)
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let _ = db;
    Ok(())
}

const LIMIT: usize = 200_000;
const SCHEMA: &str = "
CREATE TABLE metadata(id INTEGER PRIMARY KEY CHECK(id=1),proof TEXT NOT NULL,hash TEXT NOT NULL);
CREATE TABLE areas(id TEXT PRIMARY KEY,path TEXT UNIQUE NOT NULL,proof TEXT NOT NULL,hash TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('active','attention','recovered')),error TEXT,entry_count INTEGER NOT NULL CHECK(entry_count BETWEEN 1 AND 200000));
CREATE TABLE entries(area TEXT NOT NULL REFERENCES areas(id),path TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('directory','file')),proof TEXT NOT NULL,hash TEXT NOT NULL,removed INTEGER NOT NULL DEFAULT 0 CHECK(removed IN (0,1)),PRIMARY KEY(area,path));
CREATE TRIGGER metadata_fixed BEFORE UPDATE ON metadata BEGIN SELECT RAISE(ABORT,'scratch root is immutable'); END;
CREATE TRIGGER metadata_retained BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'scratch root is retained'); END;
CREATE TRIGGER areas_fixed BEFORE UPDATE ON areas WHEN NEW.id!=OLD.id OR NEW.path!=OLD.path OR NEW.proof!=OLD.proof OR NEW.hash!=OLD.hash OR NEW.entry_count<OLD.entry_count BEGIN SELECT RAISE(ABORT,'scratch area is immutable'); END;
CREATE TRIGGER areas_retained BEFORE DELETE ON areas BEGIN SELECT RAISE(ABORT,'scratch area is retained'); END;
CREATE TRIGGER entries_fixed BEFORE UPDATE ON entries WHEN NEW.area!=OLD.area OR NEW.path!=OLD.path OR NEW.kind!=OLD.kind OR NEW.proof!=OLD.proof OR NEW.hash!=OLD.hash OR NEW.removed<OLD.removed BEGIN SELECT RAISE(ABORT,'scratch ownership is immutable'); END;
CREATE TRIGGER entries_retained BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT,'scratch ownership is retained'); END;
";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    birth_seconds: u64,
    birth_nanos: u32,
}
impl Identity {
    fn read(m: &Metadata) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let birth = m
                .created()
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            Ok(Self {
                device: m.dev(),
                inode: m.ino(),
                birth_seconds: birth.as_secs(),
                birth_nanos: birth.subsec_nanos(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = m;
            Err("Windowsの一時領域回収は保留中です".into())
        }
    }
    fn matches(&self, m: &Metadata) -> bool {
        Self::read(m).is_ok_and(|v| v == *self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    area: String,
    path: String,
    kind: String,
    identity: Identity,
}

pub(in crate::storage) struct Registry {
    root: PathBuf,
    root_identity: Identity,
    journal_root: PathBuf,
    journal_identity: Identity,
    file_identity: Identity,
    expected_schema: Vec<(String, String, String)>,
    db: Mutex<Connection>,
}
#[derive(Clone)]
struct Active {
    registry: Option<Weak<Registry>>,
    area: String,
}
impl Active {
    fn owner(&self) -> Result<Option<Arc<Registry>>, String> {
        self.registry
            .as_ref()
            .map(|weak| {
                weak.upgrade()
                    .ok_or_else(|| "一時領域の所有者が失われています".to_owned())
            })
            .transpose()
    }
}
static REGISTRIES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Registry>>>> = OnceLock::new();
static STAGES: OnceLock<Mutex<BTreeMap<PathBuf, Active>>> = OnceLock::new();
fn stages() -> &'static Mutex<BTreeMap<PathBuf, Active>> {
    STAGES.get_or_init(Mutex::default)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn sync(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn regular(m: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        m.is_file() && m.nlink() == 1 && m.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        m.is_file()
    }
}
fn directory(path: &Path) -> Result<Metadata, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_dir() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err("一時領域の親が変更されています".into());
    }
    Ok(m)
}
fn private_directory(path: &Path) -> Result<Metadata, String> {
    let m = directory(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o077 != 0 {
            return Err("一時領域の非公開権限が変更されています".into());
        }
    }
    Ok(m)
}
fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
    db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name").map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())
}
fn active(path: &Path) -> Result<Option<(PathBuf, Active)>, String> {
    let all = stages()
        .lock()
        .map_err(|_| "一時領域の所有ロックを取得できません")?;
    Ok(all
        .iter()
        .filter(|(p, _)| path.starts_with(p))
        .max_by_key(|(p, _)| p.components().count())
        .map(|(p, a)| (p.clone(), a.clone())))
}

impl Registry {
    #[cfg(target_os = "macos")]
    pub(in crate::storage) fn binding_changed(root: &Path, journal: &Path) -> Result<bool, String> {
        use std::os::unix::ffi::OsStrExt;
        let path = journal.join(".mon-scratch.sqlite3");
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.to_string()),
        };
        if !regular(&metadata) {
            return Err("一時領域台帳の実体・権限が不正です".into());
        }
        if let Some(current) = REGISTRIES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?
            .get(root)
            .and_then(Weak::upgrade)
        {
            current.check_root()?;
            return Ok(false);
        }
        // Immutable read of metadata only: never recover, checkpoint or edit
        // the old journal while deciding whether a fresh generation is needed.
        let encoded = path
            .as_os_str()
            .as_bytes()
            .iter()
            .map(|b| format!("%{b:02X}"))
            .collect::<String>();
        let db = Connection::open_with_flags(
            format!("file:{encoded}?mode=ro&immutable=1"),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        let (proof, digest): (String, String) = db
            .query_row(
                "SELECT proof,hash FROM metadata WHERE id=1 AND length(CAST(proof AS BLOB))<=1024",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if hash(proof.as_bytes()) != digest {
            return Err("一時領域の親証拠が変更されています".into());
        }
        let saved: Identity = serde_json::from_str(&proof).map_err(|e| e.to_string())?;
        Ok(saved != Identity::read(&directory(root)?)?)
    }
    pub(in crate::storage) fn open(root: &Path) -> Result<Arc<Self>, String> {
        Self::open_inner(root, None)
    }
    // A recovered root gets a new journal, while stages and history remain
    // relative to the actual meeting root. Old journal bytes stay untouched.
    #[cfg(target_os = "macos")]
    pub(in crate::storage) fn open_at(root: &Path, journal: &Path) -> Result<Arc<Self>, String> {
        Self::open_inner(root, Some(journal))
    }
    fn open_inner(root: &Path, journal: Option<&Path>) -> Result<Arc<Self>, String> {
        let mut all = REGISTRIES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        // A PreparedRestore may still own its private copies after a Storage
        // handle is reopened in this process. Do not recover that live owner.
        if let Some(current) = all.get(root).and_then(Weak::upgrade) {
            if journal.is_some_and(|path| path != current.journal_root) {
                return Err("別の一時領域台帳が使用中です".into());
            }
            current.check_root()?;
            {
                let db = current
                    .db
                    .lock()
                    .map_err(|_| "一時領域台帳ロックを取得できません")?;
                current.check_schema(&db)?;
            }
            return Ok(current);
        }
        let root_identity = Identity::read(&directory(root)?)?;
        let journal_root = journal.unwrap_or(root);
        let journal_identity = Identity::read(&directory(journal_root)?)?;
        let path = journal_root.join(".mon-scratch.sqlite3");
        let new = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut o = OpenOptions::new();
                o.read(true).write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
                }
                o.open(&path)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
                sync(journal_root)?;
                true
            }
            Ok(m) if regular(&m) => false,
            _ => return Err("一時領域台帳が通常の非公開ファイルではありません".into()),
        };
        let file_identity =
            Identity::read(&std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?)?;
        let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
        expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        expected
            .execute_batch(review::SCHEMA)
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        expected
            .execute_batch(history::SCHEMA)
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        expected
            .execute_batch(binding::SCHEMA)
            .map_err(|e| e.to_string())?;
        let expected_schema = schema(&expected)?;
        let mut db = Connection::open(&path).map_err(|e| e.to_string())?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        // Inspect existing SQL before changing its journaling settings. Even an
        // empty file without a saved root proof is not adopted as ours.
        if !new && (version != local_version() || schema(&db)? != expected_schema) {
            return Err("一時領域台帳のSQL定義が不正です。既存ファイルを保持します".into());
        }
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
        )
        .map_err(|e| e.to_string())?;
        if new {
            let proof = serde_json::to_string(&root_identity).map_err(|e| e.to_string())?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            tx.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            tx.execute_batch(review::SCHEMA)
                .map_err(|e| e.to_string())?;
            history_schema(&tx)?;
            tx.execute(
                "INSERT INTO metadata VALUES(1,?1,?2)",
                params![proof, hash(proof.as_bytes())],
            )
            .map_err(|e| e.to_string())?;
            tx.pragma_update(None, "user_version", local_version())
                .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        if schema(&db)? != expected_schema {
            return Err("一時領域台帳のSQL定義が不正です。既存ファイルを保持します".into());
        }
        let (json, sha): (String, String) = db
            .query_row("SELECT proof,hash FROM metadata WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        if json.len() > 1024 || hash(json.as_bytes()) != sha {
            return Err("一時領域の親証拠が変更されています".into());
        }
        let saved: Identity = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if saved != root_identity {
            return Err("一時領域の保存先実体が変更されています。コピーを保持します".into());
        }
        let registry = Arc::new(Self {
            root: root.into(),
            root_identity,
            journal_root: journal_root.into(),
            journal_identity,
            file_identity,
            expected_schema,
            db: Mutex::new(db),
        });
        registry.recover()?;
        all.retain(|_, v| v.strong_count() > 0);
        all.insert(root.into(), Arc::downgrade(&registry));
        Ok(registry)
    }
    fn check_root(&self) -> Result<(), String> {
        let path = self.journal_root.join(".mon-scratch.sqlite3");
        let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !regular(&m)
            || !self.file_identity.matches(&m)
            || !self.root_identity.matches(&directory(&self.root)?)
            || !self
                .journal_identity
                .matches(&directory(&self.journal_root)?)
        {
            return Err("一時領域台帳または保存先が差し替えられています".into());
        }
        Ok(())
    }
    pub(in crate::storage) fn stage(self: &Arc<Self>) -> Result<Stage, String> {
        self.check_root()?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        self.check_schema(&db)?;
        let id = Uuid::new_v4().to_string();
        let path = self.root.join(format!(".mon-stage-{id}"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|e| e.to_string())?;
        let proof = Proof {
            area: id.clone(),
            path: format!(".mon-stage-{id}"),
            kind: "directory".into(),
            identity: Identity::read(&private_directory(&path)?)?,
        };
        let json = serde_json::to_string(&proof).map_err(|e| e.to_string())?;
        sync(&self.root)?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO areas VALUES(?1,?2,?3,?4,'active',NULL,1)",
            params![id, proof.path, json, hash(json.as_bytes())],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO entries(area,path,kind,proof,hash) VALUES(?1,?2,'directory',?3,?4)",
            params![id, proof.path, json, hash(json.as_bytes())],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        stages()
            .lock()
            .map_err(|_| "一時領域の所有ロックを取得できません")?
            .insert(
                path.clone(),
                Active {
                    registry: Some(Arc::downgrade(self)),
                    area: id.clone(),
                },
            );
        Ok(Stage {
            path,
            temporary: None,
            owner: Some((self.clone(), id)),
        })
    }
    #[cfg(target_os = "macos")]
    pub(in crate::storage) fn require_no_attention(&self) -> Result<(), String> {
        self.check_root()?;
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        self.check_schema(&db)?;
        let attention: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM areas WHERE state='attention')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if attention {
            return Err("読取用一時コピーに所有不明・変更済みの対象があります。元のDBと残ったコピーを保持して確認してください".into());
        }
        Ok(())
    }
    fn relative(&self, path: &Path) -> Result<String, String> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| "一時領域が保存先外です")?;
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("一時領域パスが不正です".into());
        }
        let value = relative.to_str().ok_or("一時領域パスが不正です")?;
        if value.is_empty() || value.len() > 1024 {
            return Err("一時領域パスが長すぎます".into());
        }
        Ok(value.into())
    }
    fn check_schema(&self, db: &Connection) -> Result<(), String> {
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version != local_version() || schema(db)? != self.expected_schema {
            return Err("一時領域台帳のSQL定義が変更されています。コピーを保持します".into());
        }
        Ok(())
    }
    fn area(&self, db: &Connection, id: &str) -> Result<(Proof, usize), String> {
        self.check_schema(db)?;
        let (path, json, sha, count): (String, String, String, u32) = db
            .query_row(
                "SELECT path,proof,hash,entry_count FROM areas WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        if !Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
            || path != format!(".mon-stage-{id}")
            || !(1..=LIMIT as u32).contains(&count)
            || json.len() > 4096
            || hash(json.as_bytes()) != sha
        {
            return Err("一時領域の所有証拠が不正です".into());
        }
        let root: Proof = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if root.area != id || root.path != path || root.kind != "directory" {
            return Err("一時領域の所有証拠が不正です".into());
        }
        Ok((root, count as usize))
    }
    fn proof(
        &self,
        id: &str,
        p: String,
        kind: String,
        json: String,
        sha: String,
        removed: bool,
    ) -> Result<(Proof, bool), String> {
        if json.len() > 4096
            || hash(json.as_bytes()) != sha
            || self.relative(&self.root.join(&p))? != p
            || !Path::new(&p).starts_with(format!(".mon-stage-{id}"))
            || !matches!(kind.as_str(), "directory" | "file")
        {
            return Err("一時領域の対象証拠が変更されています".into());
        }
        let proof: Proof = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if proof.area != id
            || proof.path != p
            || proof.kind != kind
            || proof.identity.birth_nanos >= 1_000_000_000
        {
            return Err("一時領域の実体証拠が不正です".into());
        }
        Ok((proof, removed))
    }
    // Indexed ancestor reads keep payload registration proportional to path
    // depth, not to every previously copied asset in this area.
    fn lineage(
        &self,
        db: &Connection,
        id: &str,
        path: &Path,
        include_leaf: bool,
    ) -> Result<(BTreeMap<String, (Proof, bool)>, usize), String> {
        let (root, count) = self.area(db, id)?;
        let relative = self.relative(path)?;
        if !Path::new(&relative).starts_with(&root.path) {
            return Err("一時領域の外側は登録しません".into());
        }
        let mut entries = BTreeMap::new();
        let mut current = self.root.clone();
        let parts: Vec<_> = Path::new(&relative).components().collect();
        for component in parts.iter().take(parts.len() - usize::from(!include_leaf)) {
            current.push(component.as_os_str());
            let key = self.relative(&current)?;
            let (kind, json, sha, removed) = db
                .query_row(
                    "SELECT kind,proof,hash,removed FROM entries WHERE area=?1 AND path=?2",
                    params![id, key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .map_err(|e| e.to_string())?;
            entries.insert(key.clone(), self.proof(id, key, kind, json, sha, removed)?);
        }
        if entries
            .get(&root.path)
            .and_then(|(p, _)| serde_json::to_string(p).ok())
            != serde_json::to_string(&root).ok()
        {
            return Err("一時領域の親証拠が欠損しています".into());
        }
        Ok((entries, count))
    }
    fn load(&self, db: &Connection, id: &str) -> Result<BTreeMap<String, (Proof, bool)>, String> {
        let (root, count) = self.area(db, id)?;
        let path = &root.path;
        let mut entries = BTreeMap::new();
        let mut query = db
            .prepare("SELECT path,kind,proof,hash,removed FROM entries WHERE area=?1 ORDER BY path")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query(params![id]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            if entries.len() >= LIMIT {
                return Err("一時領域の回収対象が安全上限を超えています".into());
            }
            let (p, kind, json, sha, removed): (String, String, String, String, bool) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                row.get(3).map_err(|e| e.to_string())?,
                row.get(4).map_err(|e| e.to_string())?,
            );
            let proof = self.proof(id, p.clone(), kind, json, sha, removed)?;
            entries.insert(p, proof);
        }
        if entries
            .get(path)
            .and_then(|(p, _)| serde_json::to_string(p).ok())
            != serde_json::to_string(&root).ok()
            || entries.len() != count
        {
            return Err("一時領域の親証拠が欠損しています".into());
        }
        Ok(entries)
    }
    fn parents(
        &self,
        path: &Path,
        entries: &BTreeMap<String, (Proof, bool)>,
    ) -> Result<(), String> {
        self.check_root()?;
        let relative = self.relative(path)?;
        let mut parent = self.root.clone();
        let parts: Vec<_> = Path::new(&relative).components().collect();
        for component in &parts[..parts.len() - 1] {
            parent.push(component.as_os_str());
            let key = self.relative(&parent)?;
            let (proof, removed) = entries
                .get(&key)
                .ok_or("一時領域の親の所有記録がありません")?;
            if *removed
                || proof.kind != "directory"
                || !proof.identity.matches(&private_directory(&parent)?)
            {
                return Err("一時領域の親実体が変更されています".into());
            }
        }
        Ok(())
    }
    fn record(&self, id: &str, path: &Path, m: &Metadata, kind: &str) -> Result<(), String> {
        self.check_root()?;
        let relative = self.relative(path)?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let (entries, count) = self.lineage(&db, id, path, false)?;
        self.parents(path, &entries)?;
        if count >= LIMIT {
            return Err("一時領域の実体は新規作成した場合だけ登録できます".into());
        }
        let current = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let identity = Identity::read(m)?;
        if !identity.matches(&current)
            || (kind == "file" && !regular(&current))
            || (kind == "directory" && private_directory(path).is_err())
        {
            return Err("新規一時ファイルの実体が変更されています".into());
        }
        let proof = Proof {
            area: id.into(),
            path: relative.clone(),
            kind: kind.into(),
            identity,
        };
        let json = serde_json::to_string(&proof).map_err(|e| e.to_string())?;
        sync(path.parent().ok_or("一時領域の親がありません")?)?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if tx.execute("UPDATE areas SET entry_count=entry_count+1 WHERE id=?1 AND state!='recovered' AND entry_count<?2",params![id,LIMIT as i64]).map_err(|e|e.to_string())? != 1 {
            return Err("回収済み・上限到達の一時領域は追加登録しません".into());
        }
        tx.execute(
            "INSERT INTO entries(area,path,kind,proof,hash) VALUES(?1,?2,?3,?4,?5)",
            params![id, relative, kind, json, hash(json.as_bytes())],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    fn owned_directory(&self, id: &str, path: &Path) -> Result<(), String> {
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let (entries, _) = self.lineage(&db, id, path, true)?;
        self.parents(path, &entries)?;
        let (p, removed) = entries
            .get(&self.relative(path)?)
            .ok_or("既存フォルダーは一時領域へ採用しません")?;
        if *removed || p.kind != "directory" || !p.identity.matches(&private_directory(path)?) {
            return Err("一時領域の既存親が変更されています".into());
        }
        Ok(())
    }
    fn check_creation(&self, id: &str, path: &Path) -> Result<(), String> {
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let (entries, count) = self.lineage(&db, id, path, false)?;
        if count >= LIMIT {
            return Err("一時領域の登録上限に達しています".into());
        }
        self.parents(path, &entries)
    }
    fn cleanup(&self, id: &str, scope: &Path) -> Result<(), String> {
        let evidence = self.scratch_evidence(id)?;
        if self.retained("scratch", id, &evidence.intent_hash)? {
            return Err("保持して確認済みの領域は回収しません".into());
        }
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let result = (|| {
            self.check_root()?;
            let entries = self.load(&db, id)?;
            let scope = self.relative(scope)?;
            if !entries.contains_key(&scope) {
                return Err("回収範囲の所有記録がありません".into());
            }
            let mut ordered: Vec<_> = entries
                .values()
                .filter(|(p, removed)| !*removed && Path::new(&p.path).starts_with(&scope))
                .collect();
            ordered.sort_by(|(a, _), (b, _)| {
                b.path.len().cmp(&a.path.len()).then(b.path.cmp(&a.path))
            });
            let mut changed = false;
            for (proof, _) in ordered {
                let path = self.root.join(&proof.path);
                let m = match std::fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Ok(m) => Some(m),
                    _ => {
                        changed = true;
                        continue;
                    }
                };
                if let Some(m) = m {
                    if self.parents(&path, &entries).is_err()
                        || !proof.identity.matches(&m)
                        || (proof.kind == "file" && !regular(&m))
                        || (proof.kind == "directory" && private_directory(&path).is_err())
                    {
                        changed = true;
                        continue;
                    }
                    let remove = if proof.kind == "file" {
                        std::fs::remove_file(&path)
                    } else {
                        std::fs::remove_dir(&path)
                    };
                    if remove.is_err() {
                        changed = true;
                        continue;
                    }
                    sync(path.parent().ok_or("一時領域の親がありません")?)?;
                }
                db.execute(
                    "UPDATE entries SET removed=1 WHERE area=?1 AND path=?2",
                    params![id, proof.path],
                )
                .map_err(|e| e.to_string())?;
            }
            if changed {
                return Err("所有不明・変更済みの一時ファイルを保持しています".into());
            }
            if scope == format!(".mon-stage-{id}") {
                db.execute(
                    "UPDATE areas SET state='recovered',error=NULL WHERE id=?1",
                    params![id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = &result {
            // A replaced root/journal or executable SQL cannot be used even to
            // record attention: SQLite would write through an untrusted path.
            self.check_root()?;
            self.check_schema(&db)?;
            db.execute(
                "UPDATE areas SET state='attention',error=?1 WHERE id=?2",
                params![error, id],
            )
            .map_err(|e| e.to_string())?;
        }
        result
    }
    fn recover(&self) -> Result<(), String> {
        let pending = {
            let db = self
                .db
                .lock()
                .map_err(|_| "一時領域台帳ロックを取得できません")?;
            self.check_root()?;
            self.check_schema(&db)?;
            let mut q = db
                .prepare(
                    "SELECT id,path FROM areas WHERE state!='recovered' ORDER BY id LIMIT 200001",
                )
                .map_err(|e| e.to_string())?;
            let result = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        };
        if pending.len() > LIMIT {
            return Err("一時領域の回収件数が安全上限を超えています".into());
        }
        for (id, path) in pending {
            let intent = self.scratch_evidence(&id)?;
            if self.retained("scratch", &id, &intent.intent_hash)? {
                continue;
            }
            if let Err(error) = self.cleanup(&id, &self.root.join(path)) {
                // Ownership uncertainty is non-blocking only after its durable
                // attention record succeeds. Never swallow journal I/O failure.
                let db = self
                    .db
                    .lock()
                    .map_err(|_| "一時領域台帳ロックを取得できません")?;
                let retained: bool = db
                    .query_row(
                        "SELECT state='attention' AND error=?1 FROM areas WHERE id=?2",
                        params![error, id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !retained {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

pub(in crate::storage) struct Stage {
    path: PathBuf,
    temporary: Option<tempfile::TempDir>,
    owner: Option<(Arc<Registry>, String)>,
}
impl Stage {
    pub(in crate::storage) fn path(&self) -> &Path {
        &self.path
    }
    pub(in crate::storage) fn create(parent: &Path) -> Result<Self, String> {
        let context = active(parent)?;
        let resolved = context
            .as_ref()
            .map(|(_, a)| a.owner())
            .transpose()?
            .flatten();
        if let Some(registry) = &resolved {
            registry.owned_directory(&context.as_ref().unwrap().1.area, parent)?;
        }
        let mut builder = tempfile::Builder::new();
        builder.prefix(".mon-archive-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let temporary = builder.tempdir_in(parent).map_err(|e| e.to_string())?;
        let path = temporary.path().canonicalize().map_err(|e| e.to_string())?;
        let owner =
            match resolved.map(|registry| (registry, context.as_ref().unwrap().1.area.clone())) {
                Some((registry, id)) => {
                    let _ = temporary.keep();
                    registry.record(&id, &path, &directory(&path)?, "directory")?;
                    Some((registry, id))
                }
                None => {
                    stages()
                        .lock()
                        .map_err(|_| "一時領域所有ロックを取得できません")?
                        .insert(
                            path.clone(),
                            Active {
                                registry: None,
                                area: String::new(),
                            },
                        );
                    return Ok(Self {
                        path,
                        temporary: Some(temporary),
                        owner: None,
                    });
                }
            };
        let (registry, id) = owner.as_ref().unwrap();
        stages()
            .lock()
            .map_err(|_| "一時領域所有ロックを取得できません")?
            .insert(
                path.clone(),
                Active {
                    registry: Some(Arc::downgrade(registry)),
                    area: id.clone(),
                },
            );
        Ok(Self {
            path,
            temporary: None,
            owner,
        })
    }
    #[cfg(test)]
    pub(in crate::storage::backup) fn abandon(mut self) -> PathBuf {
        stages().lock().unwrap().remove(&self.path);
        if let Some(t) = self.temporary.take() {
            let _ = t.keep();
        }
        self.owner = None;
        self.path.clone()
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        stages()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.path);
        if let Some((registry, id)) = &self.owner {
            let _ = registry.cleanup(id, &self.path);
        }
    }
}
pub(in crate::storage) fn record_file(path: &Path, file: &File) -> Result<(), String> {
    if let Some((_, a)) = active(path)? {
        if let Some(registry) = a.owner()? {
            registry.record(
                &a.area,
                path,
                &file.metadata().map_err(|e| e.to_string())?,
                "file",
            )?;
        }
    }
    Ok(())
}
pub(in crate::storage) fn check_creation(path: &Path) -> Result<(), String> {
    if let Some((_, context)) = active(path)? {
        if let Some(registry) = context.owner()? {
            registry.check_creation(&context.area, path)?;
        }
    }
    Ok(())
}
pub(super) fn directories(path: &Path) -> Result<(), String> {
    let (base, context) = active(path)?.ok_or("復元ディレクトリは専用一時領域内に限ります")?;
    let owner = context.owner()?;
    if let Some(r) = &owner {
        r.owned_directory(&context.area, &base)?;
    }
    let mut current = base.clone();
    for c in path
        .strip_prefix(&base)
        .map_err(|e| e.to_string())?
        .components()
    {
        if !matches!(c, Component::Normal(_)) {
            return Err("復元ディレクトリの成分が不正です".into());
        }
        current.push(c.as_os_str());
        if let Some(r) = &owner {
            r.check_creation(&context.area, &current)?;
        }
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        match b.create(&current) {
            Ok(()) => {
                if let Some(r) = &owner {
                    r.record(&context.area, &current, &directory(&current)?, "directory")?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                directory(&current)?;
                if let Some(r) = &owner {
                    r.owned_directory(&context.area, &current)?;
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
