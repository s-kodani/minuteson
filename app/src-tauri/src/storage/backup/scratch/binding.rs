//! Local restore ownership translations. The original immutable meeting DB
//! intent is preserved; only verified copied inodes receive translated ownership.
use super::*;

pub(super) const SCHEMA: &str = "
CREATE TABLE relocation_restore_bindings(operation_id TEXT PRIMARY KEY NOT NULL CHECK(length(operation_id)=36),manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),origin_id TEXT NOT NULL REFERENCES relocation_history(operation_id),proof_json TEXT NOT NULL CHECK(json_valid(proof_json) AND length(CAST(proof_json AS BLOB))<=67108864),proof_hash TEXT NOT NULL CHECK(length(proof_hash)=64),scope TEXT NOT NULL CHECK(scope IN ('copied','local')));
CREATE TRIGGER relocation_restore_binding_fixed BEFORE UPDATE ON relocation_restore_bindings WHEN NEW.operation_id!=OLD.operation_id OR NEW.manifest_hash!=OLD.manifest_hash OR NEW.origin_id!=OLD.origin_id OR NEW.proof_json!=OLD.proof_json OR NEW.proof_hash!=OLD.proof_hash OR OLD.scope!='copied' OR NEW.scope!='local' BEGIN SELECT RAISE(ABORT,'restore ownership translation is immutable'); END;
CREATE TRIGGER relocation_restore_binding_retained BEFORE DELETE ON relocation_restore_bindings BEGIN SELECT RAISE(ABORT,'restore ownership translation is retained'); END;
";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::storage::backup) struct Record {
    pub operation_id: String,
    pub manifest_hash: String,
    pub origin_id: String,
    pub proof_json: String,
    pub proof_hash: String,
    pub scope: String,
}
fn validate(db: &Connection, r: &Record) -> Result<(), String> {
    if r.proof_json.len() > 64 * 1024 * 1024
        || !Uuid::parse_str(&r.operation_id).is_ok_and(|u| u.to_string() == r.operation_id)
        || !Uuid::parse_str(&r.origin_id).is_ok_and(|u| u.to_string() == r.origin_id)
        || !portable::valid_hash(&r.manifest_hash)
        || hash(r.proof_json.as_bytes()) != r.proof_hash
        || !matches!(r.scope.as_str(), "copied" | "local")
        || !db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM relocation_history WHERE operation_id=?1)",
                params![r.origin_id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?
    {
        return Err("復元所有対応表が欠落・変更されています".into());
    }
    Ok(())
}
pub(super) fn load(db: &Connection) -> Result<Vec<Record>, String> {
    let (n, bytes): (i64,i64) = db.query_row("SELECT count(*),coalesce(sum(length(CAST(proof_json AS BLOB))+512),0) FROM relocation_restore_bindings",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if n > 2000 || !(0..=64 * 1024 * 1024).contains(&bytes) {
        return Err("復元所有対応表が安全上限を超えています".into());
    }
    let mut q=db.prepare("SELECT operation_id,manifest_hash,origin_id,proof_json,proof_hash,scope FROM relocation_restore_bindings ORDER BY operation_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok(Record {
                operation_id: r.get(0)?,
                manifest_hash: r.get(1)?,
                origin_id: r.get(2)?,
                proof_json: r.get(3)?,
                proof_hash: r.get(4)?,
                scope: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for r in &rows {
        validate(db, r)?;
    }
    Ok(rows)
}
pub(super) fn insert(db: &Connection, r: &Record) -> Result<(), String> {
    db.execute(
        "INSERT INTO relocation_restore_bindings VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            r.operation_id,
            r.manifest_hash,
            r.origin_id,
            r.proof_json,
            r.proof_hash,
            r.scope
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
impl Registry {
    pub(in crate::storage::backup) fn restore_bindings(&self) -> Result<Vec<Record>, String> {
        self.guard()?;
        let db = self.db.lock().map_err(|_| "台帳ロックを取得できません")?;
        load(&db)
    }
    pub(in crate::storage::backup) fn restore_binding(
        &self,
        id: &str,
        key: &str,
    ) -> Result<Option<Record>, String> {
        self.guard()?;
        let db = self.db.lock().map_err(|_| "台帳ロックを取得できません")?;
        let record=db.query_row("SELECT operation_id,manifest_hash,origin_id,proof_json,proof_hash,scope FROM relocation_restore_bindings WHERE operation_id=?1 AND manifest_hash=?2 AND length(CAST(proof_json AS BLOB))<=67108864",params![id,key],|r|Ok(Record{operation_id:r.get(0)?,manifest_hash:r.get(1)?,origin_id:r.get(2)?,proof_json:r.get(3)?,proof_hash:r.get(4)?,scope:r.get(5)?})).optional().map_err(|e|e.to_string())?;
        if let Some(r) = &record {
            validate(&db, r)?;
        }
        Ok(record)
    }
    pub(in crate::storage::backup) fn use_local_restore_scope(
        &self,
        id: &str,
        manifest: &str,
    ) -> Result<(), String> {
        self.guard()?;
        let db = self.db.lock().map_err(|_| "台帳ロックを取得できません")?;
        db.execute("UPDATE relocation_restore_bindings SET scope='local' WHERE operation_id=?1 AND manifest_hash=?2 AND scope='copied'",params![id,manifest]).map_err(|e|e.to_string())?;
        drop(db);
        sync(&self.root)?;
        self.guard()
    }
}
