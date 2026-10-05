//! Durable native-copy ownership, separate from source DB and destination data.
//! No cutover, old-data deletion, automatic cleanup, or adoption of existing files.
use super::*;
use crate::storage::backup::relocation::{
    verify_placed_image_with_controls, Entry, Manifest, NativeSnapshot,
};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::BTreeMap;

mod history;

const SCHEMA: &str = "
CREATE TABLE metadata(id INTEGER PRIMARY KEY CHECK(id=1),intent TEXT NOT NULL CHECK(length(CAST(intent AS BLOB))<=65536),intent_hash TEXT NOT NULL CHECK(length(intent_hash)=64),manifest TEXT NOT NULL CHECK(length(CAST(manifest AS BLOB))<=67108864),manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),path_count INTEGER NOT NULL CHECK(path_count BETWEEN 1 AND 200001),state TEXT NOT NULL CHECK(state IN ('copying','ready')));
CREATE TABLE paths(path TEXT PRIMARY KEY NOT NULL CHECK(length(CAST(path AS BLOB)) BETWEEN 1 AND 1024),directory INTEGER NOT NULL CHECK(directory IN (0,1)),size INTEGER NOT NULL CHECK(size>=0),hash TEXT CHECK(hash IS NULL OR length(hash)=64),identity TEXT CHECK(identity IS NULL OR length(CAST(identity AS BLOB))<=512),identity_hash TEXT CHECK(identity_hash IS NULL OR length(identity_hash)=64),state TEXT NOT NULL CHECK(state IN ('pending','owned','copied')),CHECK((identity IS NULL)=(identity_hash IS NULL)),CHECK((state='pending')=(identity IS NULL)),CHECK((directory=1)=(hash IS NULL)));
CREATE TRIGGER intent_fixed BEFORE UPDATE ON metadata WHEN NEW.id!=OLD.id OR NEW.intent!=OLD.intent OR NEW.intent_hash!=OLD.intent_hash OR NEW.manifest!=OLD.manifest OR NEW.manifest_hash!=OLD.manifest_hash OR NEW.path_count!=OLD.path_count OR (OLD.state='ready' AND NEW.state!='ready') BEGIN SELECT RAISE(ABORT,'placement intent is immutable'); END;
CREATE TRIGGER intent_retained BEFORE DELETE ON metadata BEGIN SELECT RAISE(ABORT,'placement intent is retained'); END;
CREATE TRIGGER paths_sealed BEFORE INSERT ON paths WHEN EXISTS(SELECT 1 FROM metadata) BEGIN SELECT RAISE(ABORT,'placement paths are sealed'); END;
CREATE TRIGGER path_fixed BEFORE UPDATE ON paths WHEN NEW.path!=OLD.path OR NEW.directory!=OLD.directory OR NEW.size!=OLD.size OR NEW.hash IS NOT OLD.hash OR NOT ((OLD.state='pending' AND NEW.state='owned' AND NEW.identity IS NOT NULL) OR (OLD.state='owned' AND NEW.state='copied' AND NEW.identity=OLD.identity AND NEW.identity_hash=OLD.identity_hash)) BEGIN SELECT RAISE(ABORT,'placement ownership is monotonic'); END;
CREATE TRIGGER path_retained BEFORE DELETE ON paths BEGIN SELECT RAISE(ABORT,'placement ownership is retained'); END;
";

#[derive(Clone, PartialEq, Eq)]
struct ControlProof {
    identity: Identity,
    size: u64,
    mode: u32,
    change: (i64, i64),
    modified: std::time::SystemTime,
}
impl ControlProof {
    fn read(m: &Metadata) -> Result<Self, String> {
        if !regular(m, true) || m.len() > 512 * 1024 * 1024 {
            return Err("配置台帳の形式・権限・容量が不正です".into());
        }
        Ok(Self {
            identity: Identity::read(m)?,
            size: m.len(),
            mode: m.mode(),
            change: (m.ctime(), m.ctime_nsec()),
            modified: m.modified().map_err(|e| e.to_string())?,
        })
    }
}
#[derive(Clone)]
struct Node {
    entry: Entry,
    identity: Option<Identity>,
    state: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    pub operation_id: String,
    pub target: PathBuf,
    pub state: String,
    pub expected_paths: usize,
    pub owned_paths: usize,
    pub verified_paths: usize,
}

// Not deserializable and not a cutover capability. A later coordinator must
// recheck source/placement and migrate local recovery receipts before switching.
pub(crate) struct Placement<'a> {
    bootstrap: &'a Bootstrap,
    pending: PendingMove,
    path: PathBuf,
    file: File,
    root: File,
    proof: ControlProof,
    db: Connection,
    expected_schema: Vec<(String, String, String)>,
    manifest: Manifest,
    nodes: BTreeMap<String, Node>,
    uncertain: bool,
    // Keep the OS mutex until after the SQLite connection has closed. On macOS
    // a whole-file Rust lock on the DB itself conflicts with SQLite's locks.
    lock: File,
    lock_path: PathBuf,
    lock_proof: ControlProof,
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("移動先への配置を中断しました。旧保存先と途中コピーを保持します".into())
    } else {
        Ok(())
    }
}
fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
    db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name").map_err(|e|e.to_string())?
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?
        .collect::<Result<_,_>>().map_err(|e|e.to_string())
}
fn cstring(name: &str) -> Result<std::ffi::CString, String> {
    std::ffi::CString::new(name).map_err(|_| "配置対象の名前が不正です".into())
}
fn read_identity(json: &str, digest: &str) -> Result<Identity, String> {
    if json.len() > 512 || hash(json.as_bytes()) != digest {
        return Err("配置所有者の証拠が変更されています".into());
    }
    serde_json::from_str(json).map_err(|e| e.to_string())
}
fn descriptor(fd: i32) -> Result<File, String> {
    if fd < 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl Bootstrap {
    pub(crate) fn pending_placement<'a>(
        &'a self,
        operation: &str,
        snapshot: &NativeSnapshot<'_>,
    ) -> Result<Placement<'a>, String> {
        snapshot.verify(&|| false)?;
        snapshot.verify_source(&|| false)?;
        let pending = {
            let state = self
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.guard(&state)?;
            let record = &state.loaded.as_ref().ok_or("起動設定が未確定です")?.record;
            record.current.verify()?;
            let p = record
                .pending_move
                .clone()
                .ok_or("配置対象の移動記録がありません")?;
            if p.operation_id != operation
                || p.source.path != snapshot.source_root()
                || p.manifest_hash != snapshot.manifest_hash()
            {
                return Err("配置対象の移動記録と検証済みコピーが一致しません".into());
            }
            p
        };
        let target = directory(&pending.target)?;
        local(&pending.target)?;
        if target.mode() & 0o077 != 0 || !pending.target_directory.matches(&target) {
            return Err("確認した移動先の実体・権限が変更されています".into());
        }
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&pending.target)
            .map_err(|e| e.to_string())?;
        if !pending
            .target_directory
            .matches(&root.metadata().map_err(|e| e.to_string())?)
        {
            return Err("配置先が開く途中で変更されています".into());
        }
        let name = format!(".relocation-{}.sqlite3", pending.operation_id);
        let path = self.config.join(&name);
        let name_c = cstring(&name)?;
        self.guard_directory()?;
        let new = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_) => false,
            Err(e) => return Err(e.to_string()),
        };
        if new
            && std::fs::read_dir(&pending.target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("移動先には既存物があります。新しい所有台帳で採用しません".into());
        }
        let flags = libc::O_RDWR
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if new { libc::O_CREAT | libc::O_EXCL } else { 0 };
        let file = descriptor(unsafe {
            libc::openat(self.directory.as_raw_fd(), name_c.as_ptr(), flags, 0o600)
        })?;
        let initial = ControlProof::read(&file.metadata().map_err(|e| e.to_string())?)?;
        let lock_name = format!(".relocation-{}.lock", pending.operation_id);
        let lock_path = self.config.join(&lock_name);
        let lock_name_c = cstring(&lock_name)?;
        let new_lock = match std::fs::symlink_metadata(&lock_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_) => false,
            Err(e) => return Err(e.to_string()),
        };
        if !new && new_lock {
            return Err("配置台帳のロックが欠落しています。再作成しません".into());
        }
        let lock_flags = libc::O_RDWR
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if new_lock {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            };
        let mut lock = descriptor(unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                lock_name_c.as_ptr(),
                lock_flags,
                0o600,
            )
        })?;
        ControlProof::read(&lock.metadata().map_err(|e| e.to_string())?)?;
        lock.try_lock()
            .map_err(|e| format!("配置台帳を別の操作が使用中です: {e}"))?;
        let marker = format!(
            "MON relocation 1\n{}\n{}\n",
            pending.operation_id, pending.manifest_hash
        );
        if new_lock {
            lock.write_all(marker.as_bytes())
                .and_then(|_| lock.sync_all())
                .map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
        } else {
            if lock.metadata().map_err(|e| e.to_string())?.len() != marker.len() as u64 {
                return Err("配置ロックの記録が不正です".into());
            }
            let mut bytes = vec![0u8; marker.len()];
            lock.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            if bytes != marker.as_bytes() {
                return Err("配置ロックの確認対象が一致しません".into());
            }
        }
        let lock_proof = ControlProof::read(&lock.metadata().map_err(|e| e.to_string())?)?;
        if new {
            file.sync_all().map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
        } else if initial.size == 0 {
            return Err("未確定の配置台帳を再初期化しません。移動先を保持します".into());
        }
        let db = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        if !initial
            .identity
            .matches(&std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?)
        {
            return Err("配置台帳が開く途中で差し替えられています".into());
        }
        db.busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA locking_mode=EXCLUSIVE; BEGIN IMMEDIATE; COMMIT;").map_err(|e|e.to_string())?;
        let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
        expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let expected_schema = schema(&expected)?;
        let manifest = snapshot.manifest().clone();
        let manifest_json = serde_json::to_string(&manifest).map_err(|e| e.to_string())?;
        let intent = serde_json::to_string(&pending).map_err(|e| e.to_string())?;
        let mut entries = vec![Entry {
            relative_path: "meetings.sqlite3".into(),
            directory: false,
            size: manifest.database_size,
            hash: Some(manifest.database_hash.clone()),
        }];
        entries.extend(manifest.entries.clone());
        if new {
            db.execute_batch("BEGIN IMMEDIATE;")
                .map_err(|e| e.to_string())?;
            db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            for e in &entries {
                db.execute(
                    "INSERT INTO paths VALUES(?1,?2,?3,?4,NULL,NULL,'pending')",
                    params![
                        e.relative_path,
                        e.directory,
                        i64::try_from(e.size).map_err(|_| "配置サイズが範囲外です")?,
                        e.hash
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
            db.execute(
                "INSERT INTO metadata VALUES(1,?1,?2,?3,?4,?5,'copying')",
                params![
                    intent,
                    hash(intent.as_bytes()),
                    manifest_json,
                    snapshot.manifest_hash(),
                    entries.len() as i64
                ],
            )
            .map_err(|e| e.to_string())?;
            db.execute_batch("PRAGMA user_version=1; COMMIT;")
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            self.directory.sync_all().map_err(|e| e.to_string())?;
        }
        if schema(&db)? != expected_schema
            || db
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?
                != 1
        {
            return Err("配置台帳の形式・版が変更されています".into());
        }
        let integrity: String = db
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err("配置台帳が破損しています".into());
        }
        let count: i64 = db
            .query_row("SELECT count(*) FROM metadata", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let lengths:(i64,i64)=db.query_row("SELECT length(CAST(intent AS BLOB)),length(CAST(manifest AS BLOB)) FROM metadata WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if count != 1 || lengths.0 > 65536 || lengths.1 > 64 * 1024 * 1024 {
            return Err("配置台帳の対象が不正です".into());
        }
        let saved:(String,String,String,String,i64,String)=db.query_row("SELECT intent,intent_hash,manifest,manifest_hash,path_count,state FROM metadata WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e|e.to_string())?;
        if saved.0 != intent
            || saved.1 != hash(intent.as_bytes())
            || saved.2 != manifest_json
            || saved.3 != snapshot.manifest_hash()
            || saved.4 != entries.len() as i64
            || !matches!(saved.5.as_str(), "copying" | "ready")
        {
            return Err("配置台帳と現在の確認対象が一致しません。上書きしません".into());
        }
        let mut nodes = BTreeMap::new();
        for e in entries {
            let (d,size,sha,json,proof,state):(bool,i64,Option<String>,Option<String>,Option<String>,String)=db.query_row("SELECT directory,size,hash,identity,identity_hash,state FROM paths WHERE path=?1",params![e.relative_path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e|e.to_string())?;
            if d != e.directory
                || u64::try_from(size).ok() != Some(e.size)
                || sha != e.hash
                || !matches!(state.as_str(), "pending" | "owned" | "copied")
                || (state == "pending") != json.is_none()
            {
                return Err("配置対象または所有状態が変更されています".into());
            }
            let identity = match (json, proof) {
                (Some(j), Some(h)) => Some(read_identity(&j, &h)?),
                (None, None) => None,
                _ => return Err("配置所有者の証拠が欠損しています".into()),
            };
            nodes.insert(
                e.relative_path.clone(),
                Node {
                    entry: e,
                    identity,
                    state,
                },
            );
        }
        let actual: i64 = db
            .query_row("SELECT count(*) FROM paths", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if actual != nodes.len() as i64 {
            return Err("配置対象の件数が変更されています".into());
        }
        if saved.5 == "ready" && nodes.values().any(|n| n.state != "copied") {
            return Err("配置完了の記録と所有・進行の記録が一致しません".into());
        }
        let proof = ControlProof::read(&file.metadata().map_err(|e| e.to_string())?)?;
        if proof.identity != initial.identity {
            return Err("配置台帳の実体が変更されています".into());
        }
        let result = Placement {
            bootstrap: self,
            pending,
            path,
            file,
            root,
            proof,
            db,
            expected_schema,
            manifest,
            nodes,
            uncertain: false,
            lock,
            lock_path,
            lock_proof,
        };
        result.guard()?;
        Ok(result)
    }
}

impl Placement<'_> {
    fn guard(&self) -> Result<(), String> {
        if self.uncertain {
            return Err("配置台帳の同期が未確定です。旧保存先を保持します".into());
        }
        let state = self
            .bootstrap
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.bootstrap.guard(&state)?;
        let record = &state.loaded.as_ref().ok_or("起動設定がありません")?.record;
        if record.pending_move.as_ref() != Some(&self.pending) {
            return Err("配置確認が失効しています".into());
        }
        record.current.verify()?;
        let target = directory(&self.pending.target)?;
        if !self.pending.target_directory.matches(&target)
            || target.mode() & 0o077 != 0
            || !self
                .pending
                .target_directory
                .matches(&self.root.metadata().map_err(|e| e.to_string())?)
            || ControlProof::read(
                &std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?,
            )? != self.proof
            || ControlProof::read(&self.file.metadata().map_err(|e| e.to_string())?)? != self.proof
            || ControlProof::read(
                &std::fs::symlink_metadata(&self.lock_path).map_err(|e| e.to_string())?,
            )? != self.lock_proof
            || ControlProof::read(&self.lock.metadata().map_err(|e| e.to_string())?)?
                != self.lock_proof
            || schema(&self.db)? != self.expected_schema
        {
            return Err("配置先・台帳の実体または形式が変更されています".into());
        }
        local(&self.pending.target)
    }
    fn committed(&mut self) -> Result<(), String> {
        self.uncertain = true;
        self.file.sync_all().map_err(|e| e.to_string())?;
        self.bootstrap
            .directory
            .sync_all()
            .map_err(|e| e.to_string())?;
        let proof = ControlProof::read(&self.file.metadata().map_err(|e| e.to_string())?)?;
        if proof.identity != self.proof.identity
            || !proof
                .identity
                .matches(&std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?)
        {
            return Err("配置台帳の同期中に実体が変更されています".into());
        }
        self.proof = proof;
        self.uncertain = false;
        self.guard()
    }
    fn metadata_matches(&self, node: &Node, m: &Metadata) -> Result<(), String> {
        if !node.identity.as_ref().is_some_and(|p| p.matches(m))
            || m.mode() & 0o077 != 0
            || (node.entry.directory && !m.is_dir())
            || (!node.entry.directory && !regular(m, true))
        {
            return Err("配置した対象の所有・実体・権限が変更されています".into());
        }
        Ok(())
    }
    fn parent(&self, name: &str) -> Result<(File, String), String> {
        self.guard()?;
        let parts: Vec<_> = name.split('/').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
        {
            return Err("配置パスが不正です".into());
        }
        let mut fd = self.root.try_clone().map_err(|e| e.to_string())?;
        let mut path = String::new();
        for part in &parts[..parts.len() - 1] {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            let node = self
                .nodes
                .get(&path)
                .ok_or("配置先の親が確認対象にありません")?;
            if !node.entry.directory {
                return Err("配置先の親がフォルダーではありません".into());
            }
            let part = cstring(part)?;
            let next = descriptor(unsafe {
                libc::openat(
                    fd.as_raw_fd(),
                    part.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?;
            self.metadata_matches(node, &next.metadata().map_err(|e| e.to_string())?)?;
            self.metadata_matches(node, &directory(&self.pending.target.join(&path))?)?;
            fd = next;
        }
        Ok((fd, parts.last().ok_or("配置名がありません")?.to_string()))
    }
    fn record_owned(&mut self, name: &str, m: &Metadata) -> Result<(), String> {
        self.guard()?;
        let identity = Identity::read(m)?;
        let json = serde_json::to_string(&identity).map_err(|e| e.to_string())?;
        let changes=self.db.execute("UPDATE paths SET identity=?1,identity_hash=?2,state='owned' WHERE path=?3 AND state='pending'",params![json,hash(json.as_bytes()),name]).map_err(|e|e.to_string())?;
        if changes != 1 {
            return Err("配置所有者を原子的に記録できません".into());
        }
        self.committed()?;
        let n = self.nodes.get_mut(name).ok_or("配置対象がありません")?;
        n.identity = Some(identity);
        n.state = "owned".into();
        Ok(())
    }
    fn record_copied(&mut self, name: &str) -> Result<(), String> {
        self.guard()?;
        if self
            .db
            .execute(
                "UPDATE paths SET state='copied' WHERE path=?1 AND state='owned'",
                params![name],
            )
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("配置済みの記録が競合しています".into());
        }
        self.committed()?;
        self.nodes
            .get_mut(name)
            .ok_or("配置対象がありません")?
            .state = "copied".into();
        Ok(())
    }
    fn node_path(&self, node: &Node) -> Result<Metadata, String> {
        let _ = self.parent(&node.entry.relative_path)?;
        let path = self.pending.target.join(&node.entry.relative_path);
        let m = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if node.entry.directory {
            directory(&path)?;
        }
        self.metadata_matches(node, &m)?;
        Ok(m)
    }
    pub(crate) fn progress(&self) -> Result<Progress, String> {
        self.guard()?;
        let state: String = self
            .db
            .query_row("SELECT state FROM metadata WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(Progress {
            operation_id: self.pending.operation_id.clone(),
            target: self.pending.target.clone(),
            state,
            expected_paths: self.nodes.len(),
            owned_paths: self.nodes.values().filter(|n| n.identity.is_some()).count(),
            verified_paths: self.nodes.values().filter(|n| n.state == "copied").count(),
        })
    }
    pub(crate) fn copy(
        &mut self,
        snapshot: &NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Progress, String> {
        check(cancelled)?;
        if snapshot.source_root() != self.pending.source.path
            || snapshot.manifest_hash() != self.pending.manifest_hash
        {
            return Err("続行するコピーの確認対象が一致しません".into());
        }
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        self.guard()?;
        // Space accounting is conservative for a resumed partial copy. Never
        // consume the source reserve to complete a destination with low space.
        let bytes = self
            .nodes
            .values()
            .filter(|n| n.state != "copied")
            .try_fold(0u64, |sum, n| {
                sum.checked_add(n.entry.size).ok_or("配置容量が範囲外です")
            })?;
        if crate::audio_import::available_bytes(&self.pending.target)?
            < bytes
                .checked_add(16 * 1024 * 1024)
                .ok_or("配置容量が範囲外です")?
        {
            return Err("移動先の空き容量が不足しています。旧保存先を保持します".into());
        }
        let mut order = vec!["meetings.sqlite3".to_owned()];
        order.extend(
            self.manifest
                .entries
                .iter()
                .map(|e| e.relative_path.clone()),
        );
        for name in order {
            check(cancelled)?;
            let node = self.nodes.get(&name).ok_or("配置対象がありません")?.clone();
            if node.state == "copied" {
                self.node_path(&node)?;
                if !node.entry.directory {
                    self.check_file(&node, cancelled)?;
                }
                continue;
            }
            let (parent, base) = self.parent(&name)?;
            let base = cstring(&base)?;
            if node.state == "pending" {
                if node.entry.directory {
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), base.as_ptr(), 0o700) } != 0 {
                        return Err("移動先に所有未確認のフォルダーがあります。採用しません".into());
                    }
                    let fd = descriptor(unsafe {
                        libc::openat(
                            parent.as_raw_fd(),
                            base.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    })?;
                    fd.sync_all().map_err(|e| e.to_string())?;
                    parent.sync_all().map_err(|e| e.to_string())?;
                    self.record_owned(&name, &fd.metadata().map_err(|e| e.to_string())?)?;
                } else {
                    let fd = descriptor(unsafe {
                        libc::openat(
                            parent.as_raw_fd(),
                            base.as_ptr(),
                            libc::O_RDWR
                                | libc::O_CREAT
                                | libc::O_EXCL
                                | libc::O_NOFOLLOW
                                | libc::O_CLOEXEC,
                            0o600,
                        )
                    })?;
                    fd.sync_all().map_err(|e| e.to_string())?;
                    parent.sync_all().map_err(|e| e.to_string())?;
                    self.record_owned(&name, &fd.metadata().map_err(|e| e.to_string())?)?;
                }
            }
            let node = self.nodes.get(&name).ok_or("配置対象がありません")?.clone();
            self.node_path(&node)?;
            if !node.entry.directory {
                let mut fd = descriptor(unsafe {
                    libc::openat(
                        parent.as_raw_fd(),
                        base.as_ptr(),
                        libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                    )
                })?;
                self.metadata_matches(&node, &fd.metadata().map_err(|e| e.to_string())?)?;
                // The ownership row was FULL-committed BEFORE payload write or
                // truncating an owned, explicitly resumed partial file.
                snapshot.write_placement_file(&name, &mut fd, cancelled)?;
                self.node_path(&node)?;
                self.check_file(&node, cancelled)?;
            }
            parent.sync_all().map_err(|e| e.to_string())?;
            self.record_copied(&name)?;
        }
        self.verify(snapshot, cancelled)?;
        self.root.sync_all().map_err(|e| e.to_string())?;
        if self.progress()?.state != "ready" {
            self.guard()?;
            self.db
                .execute(
                    "UPDATE metadata SET state='ready' WHERE id=1 AND state='copying'",
                    [],
                )
                .map_err(|e| e.to_string())?;
            self.committed()?;
        }
        self.progress()
    }
    fn check_file(&self, node: &Node, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.node_path(node)?;
        let (parent, base) = self.parent(&node.entry.relative_path)?;
        let base = cstring(&base)?;
        let mut fd = descriptor(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                base.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        })?;
        self.metadata_matches(node, &fd.metadata().map_err(|e| e.to_string())?)?;
        let mut buffer = [0u8; 65536];
        let mut sha = Sha256::new();
        let mut size = 0u64;
        loop {
            check(cancelled)?;
            let n = fd.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .ok_or("配置ファイルが範囲外です")?;
            if size > node.entry.size {
                return Err("配置ファイルのサイズが変更されています".into());
            }
            sha.update(&buffer[..n]);
        }
        self.node_path(node)?;
        self.metadata_matches(node, &fd.metadata().map_err(|e| e.to_string())?)?;
        if size != node.entry.size || Some(format!("{:x}", sha.finalize())) != node.entry.hash {
            return Err("配置済みファイルのサイズ・SHAが一致しません。上書きしません".into());
        }
        Ok(())
    }
    pub(crate) fn verify(
        &self,
        snapshot: &NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let controls = history::allowed(self)?;
        self.verify_native(snapshot, cancelled, &controls)
    }
    fn verify_native(
        &self,
        snapshot: &NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
        controls: &[&str],
    ) -> Result<(), String> {
        check(cancelled)?;
        if snapshot.source_root() != self.pending.source.path
            || snapshot.manifest_hash() != self.pending.manifest_hash
        {
            return Err("配置の確認対象が一致しません".into());
        }
        snapshot.verify_source(cancelled)?;
        self.guard()?;
        for node in self.nodes.values() {
            check(cancelled)?;
            if node.state != "copied" {
                return Err("移動先の配置が未完了です".into());
            }
            self.node_path(node)?;
        }
        verify_placed_image_with_controls(
            &self.pending.target,
            &self.manifest,
            cancelled,
            controls,
        )?;
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        self.guard()
    }
}

#[cfg(test)]
mod tests;
