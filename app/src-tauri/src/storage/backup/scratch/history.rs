//! Immutable relocation history. Historical scratch inodes remain at the old
//! root; never import them into local areas/entries or grant cleanup authority.
use super::*;

pub(super) const SCHEMA: &str = "
CREATE TABLE relocation_history(operation_id TEXT PRIMARY KEY NOT NULL CHECK(length(operation_id)=36),local_json TEXT NOT NULL CHECK(json_valid(local_json) AND length(CAST(local_json AS BLOB))<=67108864),local_hash TEXT NOT NULL CHECK(length(local_hash)=64),control_hash TEXT NOT NULL CHECK(length(control_hash)=64),native_hash TEXT NOT NULL CHECK(length(native_hash)=64),prior_json TEXT NOT NULL CHECK(json_valid(prior_json) AND length(CAST(prior_json AS BLOB))<=16777216),prior_hash TEXT NOT NULL CHECK(length(prior_hash)=64));
CREATE TRIGGER relocation_history_fixed BEFORE UPDATE ON relocation_history BEGIN SELECT RAISE(ABORT,'relocation history is immutable'); END;
CREATE TRIGGER relocation_history_retained BEFORE DELETE ON relocation_history BEGIN SELECT RAISE(ABORT,'relocation history is retained'); END;
";
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(in crate::storage::backup) struct Origin {
    pub operation_id: String,
    pub local_json: String,
    pub local_hash: String,
    pub control_hash: String,
    pub native_hash: String,
    pub prior_json: String,
    pub prior_hash: String,
}
fn valid(o: &Origin) -> Result<(), String> {
    if !Uuid::parse_str(&o.operation_id).is_ok_and(|u| u.to_string() == o.operation_id)
        || o.local_json.len() > 64 * 1024 * 1024
        || o.prior_json.len() > 16 * 1024 * 1024
        || hash(o.local_json.as_bytes()) != o.local_hash
        || hash(o.prior_json.as_bytes()) != o.prior_hash
        || !portable::valid_hash(&o.control_hash)
        || !portable::valid_hash(&o.native_hash)
    {
        return Err("保存先移動の履歴が欠損・変更されています".into());
    }
    let local: serde_json::Value =
        serde_json::from_str(&o.local_json).map_err(|e| e.to_string())?;
    if local.get("origins").is_some()
        || local.get("source_root").and_then(|p| p.as_str()).is_none()
        || local.get("areas").and_then(|a| a.as_array()).is_none()
        || local.get("decisions").and_then(|a| a.as_array()).is_none()
    {
        return Err("保存先移動の履歴形式が不正です".into());
    }
    Ok(())
}
pub(super) fn load(db: &Connection) -> Result<Vec<Origin>, String> {
    let (count,bytes):(i64,i64)=db.query_row("SELECT count(*),coalesce(sum(length(CAST(local_json AS BLOB))+length(CAST(prior_json AS BLOB))+512),0) FROM relocation_history",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if count > 2000 || !(0..=64 * 1024 * 1024).contains(&bytes) {
        return Err("保存先移動の履歴が安全上限を超えています".into());
    }
    let mut q=db.prepare("SELECT operation_id,local_json,local_hash,control_hash,native_hash,prior_json,prior_hash FROM relocation_history ORDER BY operation_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok(Origin {
                operation_id: r.get(0)?,
                local_json: r.get(1)?,
                local_hash: r.get(2)?,
                control_hash: r.get(3)?,
                native_hash: r.get(4)?,
                prior_json: r.get(5)?,
                prior_hash: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let known: BTreeMap<_, _> = rows
        .iter()
        .map(|o| (o.operation_id.as_str(), o.local_hash.as_str()))
        .collect();
    for o in &rows {
        valid(o)?;
        let refs: Vec<(String, String)> =
            serde_json::from_str(&o.prior_json).map_err(|e| e.to_string())?;
        if refs.len() > 2000 {
            return Err("移動履歴の由来が上限を超えています".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for (id, sha) in refs {
            if id == o.operation_id
                || !seen.insert(id.clone())
                || known.get(id.as_str()) != Some(&sha.as_str())
            {
                return Err("移動履歴の由来が欠落・変更されています".into());
            }
        }
    }
    Ok(rows)
}
pub(super) fn insert(db: &Connection, o: &Origin) -> Result<(), String> {
    valid(o)?;
    db.execute(
        "INSERT INTO relocation_history VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            o.operation_id,
            o.local_json,
            o.local_hash,
            o.control_hash,
            o.native_hash,
            o.prior_json,
            o.prior_hash
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
impl Registry {
    pub(in crate::storage::backup) fn origins(&self) -> Result<Vec<Origin>, String> {
        self.guard()?;
        let db = self
            .db
            .lock()
            .map_err(|_| "端末内台帳ロックを取得できません")?;
        load(&db)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoricalRoot {
    pub operation_id: String,
    pub source_root: String,
    pub scratch_areas: usize,
    pub unresolved_scratch_areas: usize,
    pub kept_scratch_areas: usize,
    pub restore_operations: usize,
    pub remaining_root_objects: usize,
}
impl Storage {
    #[allow(dead_code)] // Main-only settings API and historical-root UI follow.
    pub(crate) fn previous_storage_history(&self) -> Result<Vec<HistoricalRoot>, String> {
        let registry = self.scratch.as_ref().ok_or("端末内台帳がありません")?;
        registry
            .origins()?
            .into_iter()
            .map(|o| {
                let local: serde_json::Value =
                    serde_json::from_str(&o.local_json).map_err(|e| e.to_string())?;
                let areas = local["areas"]
                    .as_array()
                    .ok_or("旧領域の構築履歴が不正です")?;
                let decisions = local["decisions"]
                    .as_array()
                    .ok_or("旧領域の保持履歴が不正です")?;
                Ok(HistoricalRoot {
                    operation_id: o.operation_id,
                    source_root: local["source_root"]
                        .as_str()
                        .ok_or("旧領域の場所が不正です")?
                        .into(),
                    scratch_areas: areas.len(),
                    unresolved_scratch_areas: areas
                        .iter()
                        .filter(|a| a["state"] != "recovered")
                        .count(),
                    kept_scratch_areas: decisions
                        .iter()
                        .filter(|d| d["source"] == "scratch")
                        .count(),
                    restore_operations: local["restores"]
                        .as_array()
                        .ok_or("旧領域の復元履歴が不正です")?
                        .len(),
                    remaining_root_objects: local["remaining_root_objects"]
                        .as_array()
                        .ok_or("旧領域の残存記録が不正です")?
                        .len(),
                })
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
pub(in crate::storage::backup) fn write_image(
    db: &Connection,
    target: &Path,
    operation: &str,
    native_hash: &str,
    control_hash: &str,
    history: &transfer::Bundle,
    deletion_history: Option<&crate::storage::deletion::relocation::Source>,
    deletion_binding: Option<&crate::storage::deletion::relocation::Destination>,
    bindings: &[binding::Record],
) -> Result<(), String> {
    let proof = serde_json::to_string(&Identity::read(&private_directory(target)?)?)
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; BEGIN IMMEDIATE;").map_err(|e|e.to_string())?;
    db.execute_batch(super::SCHEMA).map_err(|e| e.to_string())?;
    db.execute_batch(review::SCHEMA)
        .map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    db.execute_batch(binding::SCHEMA)
        .map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO metadata VALUES(1,?1,?2)",
        params![proof, hash(proof.as_bytes())],
    )
    .map_err(|e| e.to_string())?;
    // Only restore keep receipts have corresponding operations in the copied
    // meeting DB. Scratch consent stays scoped to historical source inodes.
    for d in &history.decisions {
        if d.source == "restore_addition" {
            review::validate_decision(
                &d.source,
                &d.operation_id,
                &d.intent_hash,
                &d.receipt_json,
                &d.receipt_hash,
            )?;
            db.execute(
                "INSERT INTO recovery_decisions VALUES(?1,?2,?3,?4,?5)",
                params![
                    d.source,
                    d.operation_id,
                    d.intent_hash,
                    d.receipt_json,
                    d.receipt_hash
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    for o in &history.origins {
        insert(db, o)?;
    }
    if deletion_history.is_some() != deletion_binding.is_some() {
        return Err("削除履歴とコピー先の所有対応が揃っていません".into());
    }
    let local_json = match deletion_history {
        None => history.local_json()?,
        Some(deletion) => {
            let mut local: serde_json::Value =
                serde_json::from_str(&history.local_json()?).map_err(|e| e.to_string())?;
            local
                .as_object_mut()
                .ok_or("元の移動履歴が不正です")?
                .insert(
                    "deletion_history".into(),
                    serde_json::to_value(deletion).map_err(|e| e.to_string())?,
                );
            local
                .as_object_mut()
                .ok_or("元の移動履歴が不正です")?
                .insert(
                    "deletion_binding".into(),
                    serde_json::to_value(deletion_binding.ok_or("削除履歴のコピー先がありません")?)
                        .map_err(|e| e.to_string())?,
                );
            serde_json::to_string(&local).map_err(|e| e.to_string())?
        }
    };
    let prior_json = serde_json::to_string(
        &history
            .origins
            .iter()
            .map(|o| (&o.operation_id, &o.local_hash))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| e.to_string())?;
    insert(
        db,
        &Origin {
            operation_id: operation.into(),
            local_hash: hash(local_json.as_bytes()),
            local_json,
            control_hash: control_hash.into(),
            native_hash: native_hash.into(),
            prior_hash: hash(prior_json.as_bytes()),
            prior_json,
        },
    )?;
    for r in bindings {
        binding::insert(db, r)?;
    }
    binding::load(db)?;
    db.execute_batch("PRAGMA user_version=4; COMMIT;")
        .map_err(|e| e.to_string())?;
    load(db)?;
    Ok(())
}

pub(in crate::storage::backup) fn verify_image(
    root: &Path,
    path: &Path,
    operation: &str,
    native_hash: &str,
    control_hash: &str,
) -> Result<(), String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(super::SCHEMA)
        .map_err(|e| e.to_string())?;
    expected
        .execute_batch(review::SCHEMA)
        .map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    expected
        .execute_batch(binding::SCHEMA)
        .map_err(|e| e.to_string())?;
    if schema(&db)? != schema(&expected)?
        || db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            != 4
    {
        return Err("移行先の端末内台帳の形式が不正です".into());
    }
    let (n,json,sha):(i64,String,String)=db.query_row("SELECT (SELECT count(*) FROM metadata),proof,hash FROM metadata WHERE id=1 AND length(CAST(proof AS BLOB))<=1024",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if n != 1
        || hash(json.as_bytes()) != sha
        || serde_json::from_str::<Identity>(&json).map_err(|e| e.to_string())?
            != Identity::read(&private_directory(root)?)?
    {
        return Err("移行先台帳のルート証拠が一致しません".into());
    }
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if integrity != "ok" {
        return Err("移行先台帳が破損しています".into());
    }
    let rows = load(&db)?;
    binding::load(&db)?;
    if !rows.iter().any(|o| {
        o.operation_id == operation
            && o.native_hash == native_hash
            && o.control_hash == control_hash
    }) {
        return Err("移行先台帳の確認対象が一致しません".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
