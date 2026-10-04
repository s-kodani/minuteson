//! Historical identities, not installation paths or verified local models.
use super::*;
use serde::{Deserialize, Serialize};

pub(in crate::storage) const TABLES: &[&str] = &[
    "model_packages",
    "model_files",
    "model_profiles",
    "profile_components",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ModelFileMetadata {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub role: String,
}
impl ModelFileMetadata {
    pub(in crate::storage) fn validate(&self) -> Result<(), String> {
        if self.name.is_empty()
            || self.name.len() > 4096
            || self.name.contains(['/', '\\', '\0'])
            || matches!(self.name.as_str(), "." | "..")
            || self.role.is_empty()
            || self.role.len() > 256
            || self.role.contains('\0')
            || self.size > i64::MAX as u64
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("モデルの履歴メタデータが不正です".into());
        }
        Ok(())
    }
}

pub(in crate::storage) fn historical(db: &Connection) -> Result<Vec<ModelFileMetadata>, String> {
    let invalid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM model_packages p WHERE source_type='historical_snapshot' AND (engine!='unrecorded' OR model_version!='unrecorded' OR quantization IS NOT NULL OR license_ref IS NOT NULL OR (SELECT count(*) FROM model_files f WHERE f.package_id=p.id)!=1 OR EXISTS(SELECT 1 FROM model_files f WHERE f.package_id=p.id AND (expected_size IS NOT NULL OR expected_hash IS NOT NULL))))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if invalid {
        return Err("モデル履歴の未記録条件・ファイル構成が不正です".into());
    }
    let mut query = db.prepare("SELECT p.model_name,p.manifest_json,p.manifest_hash,p.state,f.relative_path,f.role,f.actual_size,f.actual_hash,f.state FROM model_packages p JOIN model_files f ON f.package_id=p.id WHERE p.source_type='historical_snapshot' ORDER BY p.id,f.id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(0).map_err(|e| e.to_string())?;
        let json: String = row.get(1).map_err(|e| e.to_string())?;
        let hash: String = row.get(2).map_err(|e| e.to_string())?;
        let reference: ModelFileMetadata =
            serde_json::from_str(&json).map_err(|e| e.to_string())?;
        reference.validate()?;
        let size: i64 = row.get(6).map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(json.as_bytes())) != hash
            || name != reference.name
            || row.get::<_, String>(4).map_err(|e| e.to_string())? != reference.name
            || row.get::<_, String>(5).map_err(|e| e.to_string())? != reference.role
            || size < 0
            || size as u64 != reference.size
            || row.get::<_, String>(7).map_err(|e| e.to_string())? != reference.sha256
            || row.get::<_, String>(3).map_err(|e| e.to_string())? != "unavailable"
            || row.get::<_, String>(8).map_err(|e| e.to_string())? != "unavailable"
        {
            return Err("モデル履歴の名前・容量・SHA・未配置状態が一致しません".into());
        }
        result.push(reference);
    }
    Ok(result)
}

/// Called inside additive publication's transaction, never promotes availability.
pub(in crate::storage) fn retain(
    db: &Connection,
    references: &[ModelFileMetadata],
    cancelled: &impl Fn() -> bool,
    unused: &impl Fn(&str) -> Result<bool, String>,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("モデル履歴は公開トランザクション内で保存してください".into());
    }
    let mut known: std::collections::BTreeSet<_> = historical(db)?.into_iter().collect();
    for reference in references {
        if cancelled() {
            return Err("モデル履歴の保存を取り消しました".into());
        }
        reference.validate()?;
        if known.contains(reference) {
            continue;
        }
        let manifest = serde_json::to_string(reference).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(manifest.as_bytes()));
        let mint = || -> Result<String, String> {
            loop {
                if cancelled() {
                    return Err("モデル履歴の保存を取り消しました".into());
                }
                let id = Uuid::new_v4().to_string();
                let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM model_packages WHERE id=?1) OR EXISTS(SELECT 1 FROM model_files WHERE id=?1) OR EXISTS(SELECT 1 FROM model_profiles WHERE id=?1)",params![id],|r|r.get(0)).map_err(|e|e.to_string())?;
                if !exists && unused(&id)? {
                    return Ok(id);
                }
            }
        };
        let package = mint()?;
        // Source archive records only these facts. Do not infer an engine,
        // release, quantization, license, or publisher authenticity from a name.
        db.execute("INSERT INTO model_packages(id,engine,model_name,model_version,source_type,manifest_json,manifest_hash,state) VALUES(?1,'unrecorded',?2,'unrecorded','historical_snapshot',?3,?4,'unavailable')",params![package,reference.name,manifest,hash]).map_err(|e|e.to_string())?;
        let file = mint()?;
        db.execute("INSERT INTO model_files(id,package_id,relative_path,role,actual_size,actual_hash,state) VALUES(?1,?2,?3,?4,?5,?6,'unavailable')",params![file,package,reference.name,reference.role,reference.size as i64,reference.sha256]).map_err(|e|e.to_string())?;
        known.insert(reference.clone());
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=24)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(
            db.query_row("SELECT count(*) FROM model_compatibility_checks", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("DROP TABLE model_compatibility_checks; DELETE FROM schema_migrations WHERE version=24;").unwrap();
    }
    if !db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=23)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        return;
    }
    for table in [
        "restore_omitted_assets",
        "profile_components",
        "model_profiles",
        "model_files",
        "model_packages",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch(&format!("DROP TABLE {table};")).unwrap();
    }
    db.execute_batch("DELETE FROM schema_migrations WHERE version=23;")
        .unwrap();
}

#[cfg(test)]
mod tests;
