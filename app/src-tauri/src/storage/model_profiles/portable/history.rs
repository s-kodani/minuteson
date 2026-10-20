//! Unavailable, immutable source metadata. No local model/profile selection.
use super::*;
use crate::storage::model_metadata::ModelFileMetadata;

const MAX_CATALOGS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct Bundle {
    schema_version: u32,
    catalogs: Vec<Catalog>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryEntry {
    pub id: String,
    pub payload_hash: String,
    pub state: String,
    pub(in crate::storage) catalog: Catalog,
}

fn encoded(catalog: &Catalog) -> Result<(String, String), String> {
    catalog.validate()?;
    let json = serde_json::to_string(catalog).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    Ok((json, hash))
}

fn origin(catalog: &Catalog, id: &str) -> bool {
    catalog.packages.iter().any(|p| p.package_id == id)
        || catalog.profiles.iter().any(|p| p.source_profile_id == id)
}

fn origin_ids(catalog: &Catalog) -> impl Iterator<Item = String> + '_ {
    catalog
        .packages
        .iter()
        .map(|p| p.package_id.clone())
        .chain(catalog.profiles.iter().map(|p| p.source_profile_id.clone()))
}

impl HistoryEntry {
    pub(in crate::storage) fn origin_ids(&self) -> BTreeSet<String> {
        origin_ids(&self.catalog).collect()
    }
}

impl Bundle {
    pub(in crate::storage) fn origin_ids(&self) -> BTreeSet<String> {
        self.catalogs.iter().flat_map(origin_ids).collect()
    }
    pub(in crate::storage) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.catalogs.is_empty()
            || self.catalogs.len() > MAX_CATALOGS
        {
            return Err("持ち運び用モデル履歴の版・件数が不正です".into());
        }
        let mut hashes = BTreeSet::new();
        let mut bytes: usize = 64;
        for catalog in &self.catalogs {
            let (_, hash) = encoded(catalog)?;
            if !hashes.insert(hash) {
                return Err("持ち運び用モデル履歴が重複しています".into());
            }
            budget(catalog, &mut bytes)?;
        }
        Ok(())
    }

    pub(in crate::storage) fn file_references(
        &self,
    ) -> Result<BTreeSet<ModelFileMetadata>, String> {
        self.catalogs
            .iter()
            .map(Catalog::file_references)
            .collect::<Result<Vec<_>, _>>()
            .map(|references| references.into_iter().flatten().collect())
    }

    /// Includes current native selections and every imported source catalog.
    /// Sort/deduplicate exact metadata; never merge unrelated origin UUIDs.
    pub(in crate::storage) fn export(
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let current = super::export(db, cancelled)?;
        let mut catalogs = BTreeMap::new();
        let (_, hash) = encoded(&current)?;
        let mut bytes: usize = 64;
        budget(&current, &mut bytes)?;
        catalogs.insert(hash, current);
        for entry in read(db, cancelled)? {
            if cancelled() {
                return Err("モデル履歴の持ち運び準備を取り消しました".into());
            }
            if !catalogs.contains_key(&entry.payload_hash) {
                budget(&entry.catalog, &mut bytes)?;
                catalogs.insert(entry.payload_hash, entry.catalog);
            }
        }
        let bundle = Self {
            schema_version: 1,
            catalogs: catalogs.into_values().collect(),
        };
        bundle.validate()?;
        if cancelled() {
            return Err("モデル履歴の持ち運び準備を取り消しました".into());
        }
        Ok(bundle)
    }

    /// Append only inside the same transaction as meetings and terminal batch.
    /// Origin IDs stay embedded provenance; fresh local IDs never activate AI.
    pub(in crate::storage) fn retain(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
        unused: &impl Fn(&str) -> Result<bool, String>,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            return Err("モデル構成履歴は追加公開のトランザクション内で保存してください".into());
        }
        self.validate()?;
        let entries = read(db, cancelled)?;
        let mut origin_ids = self.origin_ids();
        for entry in &entries {
            origin_ids.extend(entry.origin_ids());
        }
        if entries.iter().any(|entry| origin_ids.contains(&entry.id)) {
            return Err("モデル構成履歴の新IDが元端末の由来IDと衝突しています".into());
        }
        let mut known = entries
            .into_iter()
            .map(|entry| {
                Ok((
                    entry.payload_hash,
                    serde_json::to_string(&entry.catalog).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut bytes: usize = 64;
        for json in known.values() {
            bytes = bytes
                .checked_add(json.len() + 1)
                .ok_or("モデル構成履歴の容量が範囲外です")?;
        }
        for catalog in &self.catalogs {
            if cancelled() {
                return Err("モデル構成履歴の追加を取り消しました".into());
            }
            let (json, hash) = encoded(catalog)?;
            if let Some(old) = known.get(&hash) {
                if *old != json {
                    return Err("モデル構成履歴の同一SHAの内容が一致しません".into());
                }
                continue;
            }
            if known.len() >= MAX_CATALOGS {
                return Err("モデル構成履歴が件数上限を超えています".into());
            }
            budget(catalog, &mut bytes)?;
            let id = loop {
                if cancelled() {
                    return Err("モデル構成履歴の追加を取り消しました".into());
                }
                let id = Uuid::new_v4().to_string();
                if !origin_ids.contains(&id)
                    && super::super::super::model_placements::unused_identity(db, &id)?
                    && unused(&id)?
                {
                    break id;
                }
            };
            db.execute("INSERT INTO model_catalog_history(id,payload_json,payload_hash,state,created_at) VALUES(?1,?2,?3,'unavailable',?4)", params![id,json,hash,now_ms()?]).map_err(|e| e.to_string())?;
            known.insert(hash, json);
        }
        if cancelled() {
            return Err("モデル構成履歴の追加を取り消しました".into());
        }
        Ok(())
    }
}

pub(in crate::storage) fn read(
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<HistoryEntry>, String> {
    if cancelled() {
        return Err("モデル構成履歴の読込みを取り消しました".into());
    }
    let count: i64 = db
        .query_row("SELECT count(*) FROM model_catalog_history", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    if count > MAX_CATALOGS as i64 {
        return Err("モデル構成履歴が件数上限を超えています".into());
    }
    let mut query = db.prepare("SELECT id,payload_json,payload_hash,state,created_at FROM model_catalog_history ORDER BY payload_hash").map_err(|e| e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut bytes = 64;
    let mut local_ids = BTreeSet::new();
    let mut origins = BTreeSet::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if cancelled() {
            return Err("モデル構成履歴の読込みを取り消しました".into());
        }
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        let json: String = row.get(1).map_err(|e| e.to_string())?;
        let hash: String = row.get(2).map_err(|e| e.to_string())?;
        let state: String = row.get(3).map_err(|e| e.to_string())?;
        let created: i64 = row.get(4).map_err(|e| e.to_string())?;
        if json.len() > MAX_BYTES || !canonical_id(&id) || state != "unavailable" || created < 0 {
            return Err("モデル構成履歴のID・容量・未配置状態が不正です".into());
        }
        let catalog: Catalog = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let (canonical, proof) = encoded(&catalog)?;
        if json != canonical || hash != proof || origin(&catalog, &id) {
            return Err("モデル構成履歴の内容SHA・由来IDが一致しません".into());
        }
        budget(&catalog, &mut bytes)?;
        local_ids.insert(id.clone());
        origins.extend(origin_ids(&catalog));
        entries.push(HistoryEntry {
            id,
            payload_hash: hash,
            state,
            catalog,
        });
    }
    if !local_ids.is_disjoint(&origins) {
        return Err("モデル構成履歴の新IDが元端末の由来IDと衝突しています".into());
    }
    Ok(entries)
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    crate::storage::edit_recovery::remove_schema_for_test(db);
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=27)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(
            db.query_row("SELECT count(*) FROM model_catalog_history", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch(
            "DROP TABLE model_catalog_history; DELETE FROM schema_migrations WHERE version=27;",
        )
        .unwrap();
    }
}

impl Storage {
    #[cfg(test)]
    pub(crate) fn model_catalog_history(&self) -> Result<Vec<HistoryEntry>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        read(&db, &|| false)
    }
}
