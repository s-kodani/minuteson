//! Durable restore intent and bounded, inode-owned startup cleanup. No bodies.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_PATHS: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    birth_seconds: u64,
    birth_nanos: u32,
}
impl Identity {
    fn read(metadata: &Metadata) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let birth = metadata
                .created()
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                birth_seconds: birth.as_secs(),
                birth_nanos: birth.subsec_nanos(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err("Windowsの復元回収は未検証です".into())
        }
    }
    fn matches(&self, metadata: &Metadata) -> bool {
        Self::read(metadata).is_ok_and(|identity| identity == *self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    path: String,
    kind: String,
    size: Option<i64>,
    hash: Option<String>,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    batch: String,
    archive: String,
    archive_fingerprint: String,
    mapping_hash: String,
    mapping_count: i64,
    root: Identity,
    path_count: usize,
    paths_hash: String,
}
struct Saved {
    manifest: Manifest,
    state: String,
    targets: BTreeMap<String, (Target, Option<Identity>)>,
}
pub(super) struct Journal<'a> {
    db: &'a Connection,
    root: PathBuf,
    id: String,
}
fn hash(text: &[u8]) -> String {
    format!("{:x}", Sha256::digest(text))
}
fn uuid(text: &str) -> bool {
    Uuid::parse_str(text).is_ok_and(|id| id.to_string() == text)
}
fn directory(path: &Path) -> Result<(), String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("復元回収の親フォルダーが変更されています".into());
    }
    Ok(())
}
fn sync(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn target_hash(targets: &[Target]) -> Result<String, String> {
    let mut hash = Sha256::new();
    for target in targets {
        hash.update(serde_json::to_vec(target).map_err(|e| e.to_string())?);
        hash.update(b"\n");
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn targets(sealed: &SealedWorkspace) -> Result<Vec<Target>, String> {
    let mut targets = vec![Target {
        path: "meetings".into(),
        kind: "shared_directory".into(),
        size: None,
        hash: None,
    }];
    for meeting in read_meetings(sealed.connection())? {
        for path in [
            format!("meetings/{}", meeting.id),
            format!("meetings/{}/restored", meeting.id),
        ] {
            targets.push(Target {
                path,
                kind: "directory".into(),
                size: None,
                hash: None,
            });
        }
    }
    let mut query=sealed.connection().prepare("SELECT relative_path,size_bytes,content_hash FROM file_assets WHERE state='ready' ORDER BY relative_path").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if targets.len() >= MAX_PATHS {
            return Err("復元回収対象が安全上限を超えています".into());
        }
        targets.push(Target {
            path: row.get(0).map_err(|e| e.to_string())?,
            kind: "file".into(),
            size: Some(row.get(1).map_err(|e| e.to_string())?),
            hash: Some(row.get(2).map_err(|e| e.to_string())?),
        });
    }
    if targets.len() > MAX_PATHS {
        return Err("復元回収対象が安全上限を超えています".into());
    }
    targets.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(targets)
}
fn check_target(db: &Connection, batch: &str, target: &Target) -> Result<(), String> {
    let parts: Vec<_> = target.path.split('/').collect();
    if target.kind == "shared_directory"
        && parts == ["meetings"]
        && target.size.is_none()
        && target.hash.is_none()
    {
        return Ok(());
    }
    if parts.first() != Some(&"meetings") || parts.len() < 2 || !uuid(parts[1]) {
        return Err("復元回収対象パスが不正です".into());
    }
    let meeting = parts[1];
    let mapped:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM restore_identity_maps WHERE batch_id=?1 AND domain='meetings' AND source_scope='' AND target_key=?2)",params![batch,meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !mapped {
        return Err("復元回収対象がバッチの会議に属していません".into());
    }
    match (target.kind.as_str(), parts.as_slice()) {
        ("directory", [_, _]) | ("directory", [_, _, "restored"])
            if target.size.is_none() && target.hash.is_none() =>
        {
            Ok(())
        }
        ("file", [_, _, "restored", file])
            if uuid(file)
                && target.size.is_some_and(|n| n >= 0)
                && target.hash.as_deref().is_some_and(portable::valid_hash) =>
        {
            let mapped:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM restore_identity_maps a JOIN restore_identity_maps m ON m.batch_id=a.batch_id AND m.domain='meetings' AND m.source_scope='' AND m.source_key=a.source_scope WHERE a.batch_id=?1 AND a.domain='file_assets' AND a.target_key=?2 AND m.target_key=?3)",params![batch,file,meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
            if mapped {
                Ok(())
            } else {
                Err("復元回収ファイルの所属が不正です".into())
            }
        }
        _ => Err("復元回収対象の形式が不正です".into()),
    }
}
fn load(db: &Connection, id: &str) -> Result<Saved, String> {
    let (batch,json,proof,state):(String,String,String,String)=db.query_row("SELECT restore_batch_id,target_manifest_json,target_hash,state FROM maintenance_operations WHERE id=?1 AND kind='restore_addition'",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    if !uuid(id) || json.len() > 65536 || hash(json.as_bytes()) != proof {
        return Err("復元回収manifestが変更されています".into());
    }
    let manifest: Manifest = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if manifest.schema_version != 1
        || manifest.batch != batch
        || !uuid(&batch)
        || !uuid(&manifest.archive)
        || manifest.mapping_count < 0
        || manifest.path_count > MAX_PATHS
        || manifest.root.birth_nanos >= 1_000_000_000
        || !portable::valid_hash(&manifest.paths_hash)
    {
        return Err("復元回収manifest形式が不正です".into());
    }
    let (archive,fingerprint,mapping,count):(String,String,String,i64)=db.query_row("SELECT archive_id,archive_fingerprint,mapping_hash,mapping_count FROM restore_batches WHERE batch_id=?1",params![batch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    if (archive, fingerprint, mapping, count)
        != (
            manifest.archive.clone(),
            manifest.archive_fingerprint.clone(),
            manifest.mapping_hash.clone(),
            manifest.mapping_count,
        )
    {
        return Err("復元回収のバッチ証拠が一致しません".into());
    }
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM maintenance_paths WHERE operation_id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count < 0 || count as usize != manifest.path_count {
        return Err("復元回収対象が欠損しています".into());
    }
    let mut targets = BTreeMap::new();
    let mut query=db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").map_err(|e|e.to_string())?;
    let mut rows = query.query(params![id]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let target = Target {
            path: row.get(0).map_err(|e| e.to_string())?,
            kind: row.get(1).map_err(|e| e.to_string())?,
            size: row.get(2).map_err(|e| e.to_string())?,
            hash: row.get(3).map_err(|e| e.to_string())?,
        };
        check_target(db, &batch, &target)?;
        let identity_json: Option<String> = row.get(4).map_err(|e| e.to_string())?;
        let identity_hash: Option<String> = row.get(5).map_err(|e| e.to_string())?;
        let identity = match (identity_json, identity_hash) {
            (None, None) => None,
            (Some(json), Some(proof)) if json.len() <= 1024 && hash(json.as_bytes()) == proof => {
                let value: Identity = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                if value.birth_nanos >= 1_000_000_000 {
                    return Err("復元回収の実体識別子が不正です".into());
                }
                Some(value)
            }
            _ => return Err("復元回収の実体記録が変更されています".into()),
        };
        targets.insert(target.path.clone(), (target, identity));
    }
    if target_hash(&targets.values().map(|(t, _)| t.clone()).collect::<Vec<_>>())?
        != manifest.paths_hash
    {
        return Err("復元回収対象のSHAが一致しません".into());
    }
    Ok(Saved {
        manifest,
        state,
        targets,
    })
}
impl<'a> Journal<'a> {
    pub(super) fn begin(
        db: &'a Connection,
        root: &Path,
        sealed: &SealedWorkspace,
        ids: &RestoreIds<'_>,
    ) -> Result<Self, String> {
        if !db.is_autocommit() {
            return Err("復元回収記録は公開トランザクション前に保存してください".into());
        }
        directory(root)?;
        let paths = targets(sealed)?;
        let (fingerprint, mapping_hash, mapping_count) = BatchPlan::new(ids, &|| false)?.proof();
        let manifest = Manifest {
            schema_version: 1,
            batch: ids.batch_id.clone(),
            archive: ids.archive_id.clone(),
            archive_fingerprint: fingerprint,
            mapping_hash,
            mapping_count,
            root: Identity::read(&std::fs::symlink_metadata(root).map_err(|e| e.to_string())?)?,
            path_count: paths.len(),
            paths_hash: target_hash(&paths)?,
        };
        let old: Option<String> = db
            .query_row(
                "SELECT id FROM maintenance_operations WHERE restore_batch_id=?1",
                params![ids.batch_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let id = if let Some(id) = old {
            recover_one(root, db, &id)?;
            let saved = load(db, &id)?;
            if saved.state != "recovered" || saved.manifest != manifest {
                return Err("前回の復元追加物の確認・回収が必要です".into());
            }
            id
        } else {
            let id = Uuid::new_v4().to_string();
            let json = serde_json::to_string(&manifest).map_err(|e| e.to_string())?;
            let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let now = now_ms()?;
            tx.execute("INSERT INTO maintenance_operations(id,kind,state,restore_batch_id,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'restore_addition','prepared',?2,?3,?4,'{\"owned\":0}',?5,?5)",params![id,ids.batch_id,json,hash(json.as_bytes()),now]).map_err(|e|e.to_string())?;
            for target in &paths {
                check_target(&tx, &ids.batch_id, target)?;
                tx.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash) VALUES(?1,?2,?3,?4,?5)",params![id,target.path,target.kind,target.size,target.hash]).map_err(|e|e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            id
        };
        db.execute("UPDATE maintenance_operations SET state='installing',error_code=NULL,updated_at=?1 WHERE id=?2 AND state IN ('prepared','recovered')",params![now_ms()?,id]).map_err(|e|e.to_string())?;
        Ok(Self {
            db,
            root: root.into(),
            id,
        })
    }
    pub(super) fn record(&self, path: &Path, metadata: &Metadata) -> Result<(), String> {
        if !self.db.is_autocommit() {
            return Err("復元実体の記録は公開前に確定してください".into());
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| "復元実体は保存先外です")?
            .to_str()
            .ok_or("復元実体パスが不正です")?;
        if relative == "meetings" {
            return Ok(());
        } // Shared parent is never removed.
        let json = serde_json::to_string(&Identity::read(metadata)?).map_err(|e| e.to_string())?;
        sync(path.parent().ok_or("復元実体の親がありません")?)?;
        let tx = rusqlite::Transaction::new_unchecked(self.db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if tx.execute("UPDATE maintenance_paths SET identity_json=?1,identity_hash=?2 WHERE operation_id=?3 AND relative_path=?4 AND identity_json IS NULL",params![json,hash(json.as_bytes()),self.id,relative]).map_err(|e|e.to_string())?!=1 {return Err("復元実体の予約がありません".into());}
        tx.execute("UPDATE maintenance_operations SET progress_json=json_object('owned',(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1 AND identity_json IS NOT NULL)),updated_at=?2 WHERE id=?1",params![self.id,now_ms()?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub(super) fn completed(&self, db: &Connection) -> Result<(), String> {
        if db.execute("UPDATE maintenance_operations SET state='completed',error_code=NULL,updated_at=?1 WHERE id=?2 AND state='installing'",params![now_ms()?,self.id]).map_err(|e|e.to_string())?!=1 {return Err("復元回収記録の同時確定に失敗しました".into());}
        Ok(())
    }
    pub(super) fn recover(&self) -> Result<(), String> {
        recover_one(&self.root, self.db, &self.id)
    }
}
fn attention(db: &Connection, id: &str, code: &str) -> Result<(), String> {
    db.execute("UPDATE maintenance_operations SET state='attention',error_code=?1,updated_at=?2 WHERE id=?3 AND state!='completed'",params![code,now_ms()?,id]).map_err(|e|e.to_string())?;
    Ok(())
}
fn parents(root: &Path, target: &str, saved: &Saved) -> Result<(), String> {
    let mut current = root.to_path_buf();
    let parts: Vec<_> = target.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
            Ok(metadata) => {
                directory(&current)?;
                let relative = current
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("復元回収パスが不正です")?;
                if relative != "meetings"
                    && !saved
                        .targets
                        .get(relative)
                        .and_then(|(_, i)| i.as_ref())
                        .is_some_and(|i| i.matches(&metadata))
                {
                    return Err("復元回収の親実体が変更されています".into());
                }
            }
        }
    }
    Ok(())
}
fn recover_one(root: &Path, db: &Connection, id: &str) -> Result<(), String> {
    if let Some(registry) = scratch::Registry::known(root)? {
        let evidence = recovery_evidence(db, id)?;
        if registry.retained("restore_addition", id, &evidence.intent_hash)? {
            return Ok(());
        }
    }
    if !db.is_autocommit() {
        return Err("公開結果が未確定のため復元追加物を保持します".into());
    }
    let saved = match load(db, id) {
        Ok(saved) => saved,
        Err(_) => return attention(db, id, "restore_manifest_invalid"),
    };
    let published: bool = db
        .query_row(
            "SELECT state='published' FROM restore_batches WHERE batch_id=?1",
            params![saved.manifest.batch],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if published {
        db.execute("UPDATE maintenance_operations SET state='completed',error_code=NULL,updated_at=?1 WHERE id=?2 AND state!='completed'",params![now_ms()?,id]).map_err(|e|e.to_string())?;
        return Ok(()); // Do not stat, hash, repair or delete published assets.
    }
    if saved.state == "completed" {
        return attention(db, id, "restore_batch_state_invalid");
    }
    directory(root)?;
    if !saved
        .manifest
        .root
        .matches(&std::fs::symlink_metadata(root).map_err(|e| e.to_string())?)
    {
        return attention(db, id, "restore_root_changed");
    }
    for (target, _) in saved.targets.values() {
        if target.kind == "directory" && target.path.split('/').count() == 2 {
            let meeting = target
                .path
                .rsplit('/')
                .next()
                .ok_or("復元会議パスが不正です")?;
            if db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                    params![meeting],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
            {
                return attention(db, id, "restore_target_referenced");
            }
        }
        if target.kind == "file"
            && db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_assets WHERE relative_path=?1)",
                    params![target.path],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
        {
            return attention(db, id, "restore_target_referenced");
        }
    }
    // Intent/ownership is already durable. Cleanup can safely replay installing
    // markers after a crash. Clear identities only AFTER unlinks are synced.
    let mut ordered: Vec<_> = saved.targets.values().collect();
    ordered.sort_by(|(a, _), (b, _)| b.path.len().cmp(&a.path.len()).then(b.path.cmp(&a.path)));
    let mut changed = false;
    for (target, identity) in ordered {
        if target.kind == "shared_directory" {
            continue;
        }
        if parents(root, &target.path, &saved).is_err() {
            changed = true;
            continue;
        }
        let path = root.join(&target.path);
        let metadata = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                changed = true;
                continue;
            }
            Ok(metadata) => metadata,
        };
        if !identity
            .as_ref()
            .is_some_and(|identity| identity.matches(&metadata))
            || (target.kind == "file" && !metadata.is_file())
            || (target.kind == "directory" && !metadata.is_dir())
        {
            changed = true;
            continue;
        }
        let removal = if target.kind == "file" {
            std::fs::remove_file(&path)
        } else {
            std::fs::remove_dir(&path)
        };
        if removal.is_err() {
            changed = true;
            continue;
        }
        sync(path.parent().ok_or("復元回収の親がありません")?)?;
    }
    if changed {
        return attention(db, id, "restore_paths_changed");
    }
    sync(root)?;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE maintenance_operations SET state='cleanup',updated_at=?1 WHERE id=?2",
        params![now_ms()?, id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE maintenance_paths SET identity_json=NULL,identity_hash=NULL WHERE operation_id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("UPDATE maintenance_operations SET state='recovered',progress_json='{\"owned\":0}',error_code=NULL,updated_at=?1 WHERE id=?2",params![now_ms()?,id]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
#[cfg(test)]
pub(in crate::storage::backup) fn recover_all(root: &Path, db: &Connection) -> Result<(), String> {
    recover_with_policy(root, db, None)
}
pub(in crate::storage::backup) fn recover_with_policy(
    root: &Path,
    db: &Connection,
    registry: Option<&scratch::Registry>,
) -> Result<(), String> {
    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE kind='restore_addition' AND state NOT IN ('completed','recovered'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !pending {
        return Ok(());
    }
    if schema(db)? != schema(&portable::schema(MIGRATIONS.len())?)? {
        return Err("復元回収前の保存先SQL定義が信頼済み定義と一致しません".into());
    }
    let mut query=db.prepare("SELECT id FROM maintenance_operations WHERE kind='restore_addition' AND state NOT IN ('completed','recovered') ORDER BY created_at,id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        if let Some(registry) = registry {
            let evidence = recovery_evidence(db, &id)?;
            if registry.retained("restore_addition", &id, &evidence.intent_hash)? {
                continue;
            }
        }
        recover_one(root, db, &id)?;
    }
    Ok(())
}

pub(in crate::storage::backup) fn recovery_evidence(
    db: &Connection,
    id: &str,
) -> Result<recovery::Evidence, String> {
    if !uuid(id) || schema(db)? != schema(&portable::schema(MIGRATIONS.len())?)? {
        return Err("復元回収の台帳定義またはIDが不正です".into());
    }
    let (batch,json,sha,state,error,progress,created,updated):(String,String,String,String,Option<String>,String,i64,i64)=db.query_row("SELECT restore_batch_id,target_manifest_json,target_hash,state,error_code,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1 AND kind='restore_addition'",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|e|e.to_string())?;
    if !uuid(&batch)
        || json.len() > 65536
        || sha.len() != 64
        || progress.len() > 4096
        || error.as_ref().is_some_and(|e| e.len() > 8192)
    {
        return Err("復元回収の記録が表示上限を超えています".into());
    }
    let batch_state: String = db
        .query_row(
            "SELECT state FROM restore_batches WHERE batch_id=?1",
            params![batch],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut intent = Sha256::new();
    intent.update(serde_json::to_vec(&(id, batch, json, sha, created)).map_err(|e| e.to_string())?);
    let status = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&state, &error, progress, updated, &batch_state))
                .map_err(|e| e.to_string())?
        )
    );
    let mut paths = std::collections::BTreeSet::new();
    let mut count = 0;
    let mut q=db.prepare("SELECT relative_path,kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path LIMIT 200001").map_err(|e|e.to_string())?;
    let mut rows = q.query(params![id]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > MAX_PATHS {
            return Err("復元回収の対象が表示上限を超えています".into());
        }
        let entry: (
            String,
            String,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = (
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            row.get(3).map_err(|e| e.to_string())?,
            row.get(4).map_err(|e| e.to_string())?,
            row.get(5).map_err(|e| e.to_string())?,
        );
        let parts: Vec<_> = entry.0.split('/').collect();
        // Never offer the shared meetings directory or an invalid path for a
        // recursive inventory. Keeping an invalid manifest still deletes none.
        if entry.1 == "directory" && parts.len() == 2 && parts[0] == "meetings" && uuid(parts[1]) {
            paths.insert(entry.0.clone());
        }
        intent.update(serde_json::to_vec(&entry).map_err(|e| e.to_string())?);
        intent.update(b"\n");
    }
    Ok(recovery::Evidence {
        item: recovery::Item {
            source: recovery::Source::RestoreAddition,
            operation_id: id.into(),
            paths: paths.into_iter().collect(),
            registered_paths: count,
            reason: error.unwrap_or_else(|| "未公開の復元追加物を回収できていません".into()),
            retained: false,
        },
        state,
        intent_hash: format!("{:x}", intent.finalize()),
        status_hash: status,
        live: batch_state == "published",
    })
}
// A transfer preserves even an invalid/explicitly kept operation, but must not
// turn its historical proof into ownership of a newly copied inode.
#[cfg(target_os = "macos")]
pub(in crate::storage::backup) fn source_record_valid(
    root: &Path,
    db: &Connection,
    id: &str,
) -> bool {
    load(db, id)
        .is_ok_and(|s| std::fs::symlink_metadata(root).is_ok_and(|m| s.manifest.root.matches(&m)))
}
pub(in crate::storage::backup) fn recover_selected(
    root: &Path,
    db: &Connection,
    id: &str,
    registry: &scratch::Registry,
) -> Result<(), String> {
    let evidence = recovery_evidence(db, id)?;
    if evidence.live || registry.retained("restore_addition", id, &evidence.intent_hash)? {
        return Err("公開済みまたは保持して確認済みの対象は回収しません".into());
    }
    recover_one(root, db, id)
}

#[cfg(test)]
pub(in crate::storage::backup) fn remove_schema_for_test(db: &Connection) {
    crate::storage::model_metadata::remove_schema_for_test(db);
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=22)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if !present {
        return;
    }
    for table in ["maintenance_paths", "maintenance_operations"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch(&format!("DROP TABLE {table};")).unwrap();
    }
    db.execute_batch("DELETE FROM schema_migrations WHERE version=22;")
        .unwrap();
}

#[cfg(test)]
mod tests;
