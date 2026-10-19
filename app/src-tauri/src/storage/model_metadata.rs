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

pub(in crate::storage) fn retained_files(
    db: &Connection,
) -> Result<Vec<ModelFileMetadata>, String> {
    let mut q = db
        .prepare(
            "SELECT metadata_json,metadata_hash FROM model_reference_files ORDER BY metadata_hash",
        )
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    rows.map(|row| {
        let (json, hash) = row.map_err(|e| e.to_string())?;
        let value: ModelFileMetadata = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        value.validate()?;
        if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
            return Err("モデル参照情報のハッシュが不正です".into());
        }
        Ok(value)
    })
    .collect()
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
    let _ = unused;
    for reference in references {
        if cancelled() {
            return Err("モデル参照情報の保存を取り消しました".into());
        }
        reference.validate()?;
        let json = serde_json::to_string(reference).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        db.execute("INSERT INTO model_reference_files(metadata_hash,metadata_json) VALUES(?1,?2) ON CONFLICT(metadata_hash) DO NOTHING",params![hash,json]).map_err(|e|e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    super::model_profiles::remove_schema_for_test(db);
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
