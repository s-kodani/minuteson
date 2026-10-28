//! Metadata is fixed before copy. File readiness records verified placement,
//! not engine compatibility; package readiness is never granted by this module.
use super::*;
use crate::model_placement::{CatalogEntry, PlacementReceipt, PlacementRecord};

fn bytes(record: &PlacementRecord) -> Result<(String, String), String> {
    record.validate()?;
    let json = serde_json::to_string(record).map_err(|e| e.to_string())?;
    if json.len() > 65_536 {
        return Err("モデル配置記録が上限を超えています".into());
    }
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    Ok((json, hash))
}

pub(super) fn unused_identity(db: &Connection, id: &str) -> Result<bool, String> {
    // Canonical native schema identifiers are quoted, values stay parameters.
    // Reserve IDs across domains and pending additive-restore mappings too.
    let tables = db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(|e|e.to_string())?.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    for table in tables {
        for column in ["id", "batch_id", "operation_id", "target_key"] {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
                    params![table, column],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if exists {
                let quoted = table.replace('"', "\"\"");
                let used: bool = db
                    .query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM \"{quoted}\" WHERE \"{column}\"=?1)"),
                        params![id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if used {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}
pub(super) fn load(db: &Connection, id: &str) -> Result<Option<(PlacementRecord, bool)>, String> {
    let result = load_history(db, id)?;
    #[cfg(any(unix, windows))]
    if let Some((record, placed)) = result {
        let removed = deletion::models::intent::removal::deleted(db, id)?;
        return Ok(Some((record, placed && !removed)));
    }
    Ok(result)
}
// Placement and CPU history remain valid even when the local body was removed.
// Deletion compares this original evidence, not its availability projection.
pub(super) fn load_history(
    db: &Connection,
    id: &str,
) -> Result<Option<(PlacementRecord, bool)>, String> {
    let row: Option<(String,String,String,String,String,Option<String>,Option<String>,String,String)> = db.query_row("SELECT manifest_json,manifest_hash,engine,model_name,model_version,quantization,license_ref,source_type,state FROM model_packages WHERE id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|e|e.to_string())?;
    let Some((json, hash, engine, name, version, quantization, license, source_type, state)) = row
    else {
        return Ok(None);
    };
    if json.len() > 65_536
        || format!("{:x}", Sha256::digest(json.as_bytes())) != hash
        || !matches!(source_type.as_str(), "manual_import" | "download")
        || !matches!(
            state.as_str(),
            "preparing" | "unavailable" | "ready" | "incompatible"
        )
    {
        return Err("モデル配置記録の由来・状態・SHAが一致しません".into());
    }
    let record: PlacementRecord = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    record.validate()?;
    let expected_engine =
        serde_json::to_value(record.manifest.engine).map_err(|e| e.to_string())?;
    if record.package_id != id
        || source_type != record.source_type()
        || expected_engine.as_str() != Some(&engine)
        || name != record.manifest.model_name
        || version != record.manifest.model_version
        || quantization != record.manifest.quantization
        || license.as_deref() != Some(&record.manifest.license_ref)
    {
        return Err("モデル配置のパッケージ情報が記録と一致しません".into());
    }
    let mut query = db.prepare("SELECT relative_path,role,expected_size,actual_size,expected_hash,actual_hash,state FROM model_files WHERE package_id=?1 ORDER BY relative_path").map_err(|e|e.to_string())?;
    let mut rows = query.query(params![id]).map_err(|e| e.to_string())?;
    let mut files = record
        .manifest
        .files
        .iter()
        .zip(&record.files)
        .collect::<Vec<_>>();
    files.sort_by(|a, b| a.0.relative_path.cmp(&b.0.relative_path));
    let mut ready = 0_usize;
    let mut pending = 0_usize;
    for (declared, actual) in &files {
        let row = rows
            .next()
            .map_err(|e| e.to_string())?
            .ok_or("モデル配置のファイル行が欠損しています")?;
        let role = serde_json::to_value(actual.role).map_err(|e| e.to_string())?;
        let file_state: String = row.get(6).map_err(|e| e.to_string())?;
        if row.get::<_, String>(0).map_err(|e| e.to_string())? != declared.relative_path
            || role.as_str() != Some(&row.get::<_, String>(1).map_err(|e| e.to_string())?)
            || row.get::<_, Option<i64>>(2).map_err(|e| e.to_string())?
                != declared.expected_size.map(|n| n as i64)
            || row.get::<_, i64>(3).map_err(|e| e.to_string())? != actual.actual_size as i64
            || row.get::<_, Option<String>>(4).map_err(|e| e.to_string())? != declared.expected_hash
            || row.get::<_, String>(5).map_err(|e| e.to_string())? != actual.actual_hash
            || !matches!(file_state.as_str(), "preparing" | "unavailable" | "ready")
        {
            return Err("モデル配置のファイル情報が記録と一致しません".into());
        }
        if file_state == "ready" {
            ready += 1;
        } else {
            pending += 1;
        }
    }
    if rows.next().map_err(|e| e.to_string())?.is_some() || (ready > 0 && pending > 0) {
        return Err("モデル配置のファイル構成・確定状態が不正です".into());
    }
    super::model_checks::validate_package_state(db, &record, pending == 0, &state)?;
    Ok(Some((record, pending == 0)))
}
fn receipt(record: &PlacementRecord, placed: bool) -> Result<PlacementReceipt, String> {
    let total = record.files.iter().try_fold(0_u64, |n, f| {
        n.checked_add(f.actual_size).ok_or("モデル容量が範囲外です")
    })?;
    Ok(PlacementReceipt {
        package_id: record.package_id.clone(),
        review_hash: record.review_hash.clone(),
        total_bytes: total,
        placement_recorded: placed,
        engine_compatibility: "not_verified".into(),
        ready: false,
    })
}
impl Storage {
    pub(crate) fn downloaded_placement_result(
        &self,
        package_id: &str,
        download_id: &str,
        recipe_hash: &str,
        review_hash: &str,
    ) -> Result<Option<PlacementReceipt>, String> {
        if [package_id, download_id]
            .iter()
            .any(|value| !uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == *value))
            || !crate::model_download_ipc::hash(recipe_hash)
            || !crate::model_download_ipc::hash(review_hash)
        {
            return Err("取得・コピー結果の操作IDまたはSHAが不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // History proves a committed copy, not current body availability or CPU
        // compatibility. A later explicit removal must not erase this receipt.
        load_history(&db, package_id)?
            .map(|(record, placed)| {
                if record.review_hash != review_hash
                    || !record
                        .download
                        .as_ref()
                        .is_some_and(|origin| origin.matches(download_id, recipe_hash))
                {
                    return Err("配置操作と取得由来・確認SHAが一致しません".into());
                }
                receipt(&record, placed)
            })
            .transpose()
    }
    pub(crate) fn model_placement_receipt(
        &self,
        expected: &PlacementRecord,
    ) -> Result<Option<PlacementReceipt>, String> {
        expected.validate()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db, &expected.package_id)?
            .map(|(saved, placed)| {
                if saved != *expected {
                    return Err("同じモデル操作IDの確認内容が一致しません".into());
                }
                receipt(&saved, placed)
            })
            .transpose()
    }
    pub(crate) fn begin_model_placement(&self, record: &PlacementRecord) -> Result<(), String> {
        let (json, hash) = bytes(record)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if !unused_identity(&tx, &record.package_id)? {
            return Err("モデル配置IDが予約済みデータと衝突しています".into());
        }
        let engine = serde_json::to_value(record.manifest.engine).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO model_packages(id,engine,model_name,model_version,quantization,source_type,manifest_json,manifest_hash,license_ref,state) VALUES(?1,?2,?3,?4,?5,?9,?6,?7,?8,'preparing')",params![record.package_id,engine.as_str(),record.manifest.model_name,record.manifest.model_version,record.manifest.quantization,json,hash,record.manifest.license_ref,record.source_type()]).map_err(|e|e.to_string())?;
        for (declared, actual) in record.manifest.files.iter().zip(&record.files) {
            let role = serde_json::to_value(actual.role).map_err(|e| e.to_string())?;
            let file_id = loop {
                let id = Uuid::new_v4().to_string();
                if unused_identity(&tx, &id)? {
                    break id;
                }
            };
            tx.execute("INSERT INTO model_files(id,package_id,relative_path,role,expected_size,actual_size,expected_hash,actual_hash,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'preparing')",params![file_id,record.package_id,actual.relative_path,role.as_str(),declared.expected_size.map(|n|n as i64),actual.actual_size as i64,declared.expected_hash,actual.actual_hash]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub(crate) fn finish_model_placement(&self, record: &PlacementRecord) -> Result<(), String> {
        self.set_model_placement(record, true)
    }
    pub(crate) fn interrupt_model_placement(&self, record: &PlacementRecord) -> Result<(), String> {
        self.set_model_placement(record, false)
    }
    fn set_model_placement(&self, record: &PlacementRecord, placed: bool) -> Result<(), String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (saved, already) =
            load_history(&tx, &record.package_id)?.ok_or("モデル配置の意図記録がありません")?;
        if saved != *record {
            return Err("モデル配置の確認内容が変わっています".into());
        }
        if already {
            return Ok(());
        } // Never demote a committed successful copy.
        tx.execute(
            "UPDATE model_files SET state=?1 WHERE package_id=?2",
            params![
                if placed { "ready" } else { "unavailable" },
                record.package_id
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE model_packages SET state='unavailable' WHERE id=?1 AND source_type=?2",
            params![record.package_id, record.source_type()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub(crate) fn list_model_placements(&self) -> Result<Vec<CatalogEntry>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let ids = db.prepare("SELECT id FROM model_packages WHERE source_type IN ('manual_import','download') ORDER BY rowid DESC").map_err(|e|e.to_string())?.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        ids.into_iter()
            .map(|id| {
                let (record, placed) =
                    load_history(&db, &id)?.ok_or("モデル配置記録がありません")?;
                #[cfg(any(unix, windows))]
                let body_removed = deletion::models::intent::removal::deleted(&db, &id)?;
                #[cfg(not(any(unix, windows)))]
                let body_removed = false;
                let receipt = receipt(&record, placed)?;
                let engine =
                    serde_json::to_value(record.manifest.engine).map_err(|e| e.to_string())?;
                let compatibility =
                    super::model_checks::latest(&db, &id)?.map(|check| check.summary);
                Ok(CatalogEntry {
                    backend_id: record.manifest.backend.as_ref().map(|b| b.id.clone()),
                    package_id: id,
                    source_type: record.source_type().into(),
                    download: record.download.as_ref().map(|origin| {
                        let (operation, recipe) = origin.identity();
                        crate::model_placement::DownloadIdentity {
                            operation_id: operation.into(),
                            recipe_hash: recipe.into(),
                        }
                    }),
                    model_name: record.manifest.model_name,
                    model_version: record.manifest.model_version,
                    quantization: record.manifest.quantization,
                    model_files: record
                        .files
                        .iter()
                        .filter(|file| {
                            !matches!(
                                file.role,
                                crate::model_inspection::Role::Executable
                                    | crate::model_inspection::Role::RuntimeLibrary
                                    | crate::model_inspection::Role::Notice
                            )
                        })
                        .map(|file| crate::model_placement::ModelFileIdentity {
                            role: file.role,
                            size: file.actual_size,
                            sha256: file.actual_hash.clone(),
                        })
                        .collect(),
                    engine: engine.as_str().ok_or("モデルエンジンが不正です")?.into(),
                    engine_version: record.manifest.engine_version,
                    total_bytes: receipt.total_bytes,
                    placement_recorded: placed,
                    body_removed,
                    ready: false,
                    review_hash: record.review_hash,
                    compatibility,
                })
            })
            .collect()
    }
}
