//! In-memory translation only: never rewrite an immutable restore intent or
//! grant ownership to unregistered, missing, or substituted source objects.
use super::*;
use crate::bootstrap::placement::Placement;
use crate::storage::backup::{relocation::NativeSnapshot, scratch::binding::Record};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bound {
    target: Target,
    original: Option<Identity>,
    copied: Option<Identity>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    version: u32,
    operation: String,
    created: i64,
    source_root: Identity,
    root: Identity,
    paths: Vec<Bound>,
}
fn manifest_hash(db: &Connection, id: &str) -> Result<String, String> {
    db.query_row(
        "SELECT target_hash FROM maintenance_operations WHERE id=?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
fn created(db: &Connection, id: &str) -> Result<i64, String> {
    db.query_row(
        "SELECT created_at FROM maintenance_operations WHERE id=?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn effective(
    root: &Path,
    db: &Connection,
    id: &str,
    mut saved: Saved,
) -> Result<Saved, String> {
    let Some(registry) = scratch::Registry::known(root)? else {
        return Ok(saved);
    };
    let key = manifest_hash(db, id)?;
    let Some(record) = registry.restore_binding(id, &key)? else {
        return Ok(saved);
    };
    let p: Proof = serde_json::from_str(&record.proof_json).map_err(|e| e.to_string())?;
    if p.version != 1
        || p.operation != id
        || p.created != created(db, id)?
        || p.paths.len() != saved.targets.len()
        || p.source_root != saved.manifest.root
        || !p
            .root
            .matches(&std::fs::symlink_metadata(root).map_err(|e| e.to_string())?)
    {
        return Err("復元所有対応表の対象・ルートが一致しません".into());
    }
    let cleared = saved.state == "recovered" && saved.targets.values().all(|(_, i)| i.is_none());
    let mut seen = std::collections::BTreeSet::new();
    for b in p.paths {
        if !seen.insert(b.target.path.clone()) {
            return Err("復元所有対応表の対象が重複しています".into());
        }
        let (t, i) = saved
            .targets
            .get_mut(&b.target.path)
            .ok_or("復元所有対応表の対象が欠落しています")?;
        if *t != b.target || (record.scope == "copied" && !cleared && *i != b.original) {
            return Err("復元所有対応表の元所有記録が変更されています".into());
        }
        if b.copied.is_some() && b.original.is_none()
            || b.target.kind == "shared_directory" && (b.original.is_some() || b.copied.is_some())
        {
            return Err("未所有・共有対象に復元回収の対応証拠を付けません".into());
        }
        if record.scope == "copied" && !cleared {
            *i = b.copied;
        }
    }
    saved.manifest.root = p.root;
    Ok(saved)
}
pub(super) fn start_local_scope(root: &Path, db: &Connection, id: &str) -> Result<(), String> {
    let raw = load(db, id)?;
    if raw.state != "recovered" || raw.targets.values().any(|(_, i)| i.is_some()) {
        return Err("復元所有の切替前に旧追加物の回収が完了していません".into());
    }
    let effective = effective(root, db, id, raw)?;
    if !effective
        .manifest
        .root
        .matches(&std::fs::symlink_metadata(root).map_err(|e| e.to_string())?)
    {
        return Err("復元再開の保存先証拠が一致しません".into());
    }
    if let Some(registry) = scratch::Registry::known(root)? {
        registry.use_local_restore_scope(id, &manifest_hash(db, id)?)?;
    }
    Ok(())
}
pub(in crate::storage::backup) fn build(
    snapshot: &NativeSnapshot<'_>,
    placement: &Placement<'_>,
    origin: &str,
    check: &impl Fn() -> bool,
) -> Result<Vec<Record>, String> {
    snapshot.verify(check)?;
    snapshot.verify_source(check)?;
    let db = crate::storage::backup::read_only(&snapshot.image().join("meetings.sqlite3"))?;
    let root = snapshot.source_root();
    let registry = scratch::Registry::known(root)?.ok_or("元の端末内台帳がありません")?;
    let mut q=db.prepare("SELECT id FROM maintenance_operations WHERE kind='restore_addition' AND state NOT IN ('completed','recovered') ORDER BY id LIMIT 2001").map_err(|e|e.to_string())?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if ids.len() > 2000 {
        return Err("復元所有の対応件数が上限を超えています".into());
    }
    let progress = placement.progress()?;
    if progress.operation_id != origin || progress.state != "ready" {
        return Err("配置済みの移動操作が一致しません".into());
    }
    let target = progress.target;
    let mut records = Vec::new();
    let mut total_paths = 0usize;
    let mut total_bytes = 0usize;
    for id in ids {
        check_cancelled(check)?;
        let evidence = recovery_evidence(&db, &id)?;
        if evidence.live || registry.retained("restore_addition", &id, &evidence.intent_hash)? {
            continue;
        }
        let raw = match load(&db, &id) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let original_root = raw.manifest.root.clone();
        let original = raw.targets.clone();
        let source = match effective(root, &db, &id, raw) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !source
            .manifest
            .root
            .matches(&std::fs::symlink_metadata(root).map_err(|e| e.to_string())?)
        {
            continue;
        }
        let mut paths = Vec::new();
        for (path, (t, old)) in original {
            check_cancelled(check)?;
            total_paths += 1;
            if total_paths > MAX_PATHS {
                return Err("復元所有の対応対象が安全上限を超えています".into());
            }
            let source_owner = source.targets.get(&path).and_then(|(_, i)| i.as_ref());
            let copied = if t.kind == "shared_directory" {
                None
            } else if let Some(m) = snapshot.source_metadata(&path)? {
                if source_owner.is_some_and(|i| i.matches(&m)) {
                    let dest = placement
                        .owned_path(&path)?
                        .ok_or("所有対象のコピーが欠落しています")?;
                    Some(Identity::read(&dest)?)
                } else {
                    None
                }
            } else {
                None
            };
            paths.push(Bound {
                target: t,
                original: old,
                copied,
            });
        }
        let proof_json = serde_json::to_string(&Proof {
            version: 1,
            operation: id.clone(),
            created: created(&db, &id)?,
            source_root: original_root,
            root: Identity::read(&std::fs::symlink_metadata(&target).map_err(|e| e.to_string())?)?,
            paths,
        })
        .map_err(|e| e.to_string())?;
        total_bytes = total_bytes
            .checked_add(proof_json.len() + 512)
            .ok_or("復元所有対応表の容量が範囲外です")?;
        if total_bytes > 64 * 1024 * 1024 {
            return Err("復元所有対応表が安全容量を超えています".into());
        }
        records.push(Record {
            operation_id: id.clone(),
            manifest_hash: manifest_hash(&db, &id)?,
            origin_id: origin.into(),
            proof_hash: hash(proof_json.as_bytes()),
            proof_json,
            scope: "copied".into(),
        });
    }
    placement.verify(snapshot, check)?;
    Ok(records)
}

#[cfg(test)]
mod tests;
