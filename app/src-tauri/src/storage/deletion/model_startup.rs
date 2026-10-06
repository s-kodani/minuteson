//! Completed model receipts are inspected before mutable startup recovery.
//! Hints are memory-only; scope, consent and immutable history are never adopted.
use super::{
    intent::removal::{self, renewal},
    Directory,
};
use crate::storage::Storage;
use rusqlite::{params, Connection};
use std::{collections::BTreeMap, path::Path};

const MAX_COMPLETED: usize = 10_000;
const REAPPEARED: &str = "model_completed_scope_reappeared";
const INVALID: &str = "model_completed_receipt_invalid";
const UNINSPECTABLE: &str = "model_completed_scope_uninspectable";

fn absent(receipt: &removal::Receipt, root: &Path) -> Result<bool, String> {
    if root != Path::new(receipt.confirmed().models_root()) {
        return Err("ネイティブのモデル保存先と削除記録が一致しません".into());
    }
    let models = Directory::root(root)?;
    models.verify_identity(&receipt.confirmed().snapshot().models_root)?;
    // Presence only: do not hash a large model or enumerate neighboring models.
    for name in receipt.roots()? {
        if models.optional_kind(&name)?.is_some() {
            return Ok(false);
        }
    }
    models.verify()?;
    Ok(true)
}

pub(in crate::storage) fn inspect(
    db: &Connection,
    anchor: &crate::storage::deletion::fs::RootAnchor,
    root: &Path,
) -> Result<Vec<(String, &'static str)>, String> {
    let mut query = db.prepare("SELECT id,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json END FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' ORDER BY created_at,id LIMIT 10001").map_err(|e| e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut count = 0;
    let mut attention = Vec::new();
    let mut linked: BTreeMap<String, Result<Vec<String>, String>> = BTreeMap::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > MAX_COMPLETED {
            return Err(
                "完了済みモデル削除履歴が起動時検査の上限を超えています。対象を保持します".into(),
            );
        }
        let operation: String = row.get(0).map_err(|e| e.to_string())?;
        if !uuid::Uuid::parse_str(&operation).is_ok_and(|id| id.to_string() == operation) {
            return Err("完了済みモデル削除の操作IDが不正です。対象を保持します".into());
        }
        let result = (|| -> Result<Option<&'static str>, String> {
            let json: Option<String> = row.get(1).map_err(|e| e.to_string())?;
            let progress: Option<String> = row.get(2).map_err(|e| e.to_string())?;
            let json = json.ok_or("モデル削除履歴が容量上限を超えています")?;
            let progress = progress.ok_or("モデル削除進捗が容量上限を超えています")?;
            if super::super::audio_supersession::startup_parent(db, anchor, &operation)?.is_some() {
                return Ok(None);
            }
            if crate::storage::deletion::audio::restored::hinted(db, &operation)? {
                return crate::storage::deletion::audio::restored::inspect_one(
                    db, anchor, &operation,
                );
            }
            if serde_json::from_str::<serde_json::Value>(&json)
                .ok()
                .is_some_and(|v| v["subtype"] == "audioOnlyDeletion")
            {
                return crate::storage::deletion::audio::intent::removal::completion::inspect_one(
                    db, anchor, &operation,
                );
            }
            if progress == "{\"modelReconfirmationRecorded\":true}" {
                let parent = renewal::audit_source(&json)?;
                let audits = linked
                    .entry(parent.clone())
                    .or_insert_with(|| removal::Receipt::load(db, &parent)?.startup_audits(db))
                    .as_ref()
                    .map_err(Clone::clone)?;
                if !audits.contains(&operation) {
                    return Err("元のモデル削除に接続していない確認記録です".into());
                }
                return Ok(None);
            }
            let receipt = removal::Receipt::load(db, &operation)?;
            let (storage_root, database) = anchor.identity()?;
            let snapshot = receipt.confirmed().snapshot();
            if snapshot.storage_root != storage_root || snapshot.database != database {
                return Err("モデル削除記録の保存領域・DB実体が一致しません".into());
            }
            // SQL completed is immutable even while a newly accepted v3 plan
            // is removing. The persistent unfinished fence already covers it.
            if !receipt.completed() {
                return Ok(None);
            }
            if root != Path::new(receipt.confirmed().models_root()) {
                return Err("ネイティブのモデル保存先と削除記録が一致しません".into());
            }
            match absent(&receipt, root) {
                Ok(true) => Ok(None),
                Ok(false) => Ok(Some(REAPPEARED)),
                Err(_) => Ok(Some(UNINSPECTABLE)),
            }
        })();
        match result {
            Ok(None) => {}
            Ok(Some(reason)) => attention.push((operation, reason)),
            Err(_) => attention.push((operation, INVALID)),
        }
    }
    Ok(attention)
}

pub(in crate::storage::deletion::models) fn resolve(
    storage: &Storage,
    db: &Connection,
    receipt: &removal::Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    if !super::super::startup::contains(db, receipt.operation())? {
        return Ok(());
    }
    receipt.verify_saved(db)?;
    receipt.verify_anchor(storage)?;
    if !receipt.completed()
        || check()
        || !absent(receipt, Path::new(receipt.confirmed().models_root()))?
        || check()
    {
        return Err("モデル削除の最終確認後に対象が再出現しました。確認待ちを維持します".into());
    }
    db.execute(
        "DELETE FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
        params![receipt.operation()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "model_startup_tests.rs"]
mod tests;
