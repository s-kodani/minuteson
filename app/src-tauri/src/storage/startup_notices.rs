//! Presentation receipts only. They never authorize model execution or updates.
use super::*;
use std::collections::BTreeMap;

const KEY: &str = "startup_notice_receipts";
impl Storage {
    pub(crate) fn startup_notice_receipts(&self) -> Result<BTreeMap<String, String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db)
    }
    pub(crate) fn acknowledge_startup_notice(
        &self,
        id: &str,
        revision: &str,
    ) -> Result<(), String> {
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let mut receipts = load(&tx)?;
        receipts.insert(id.to_owned(), revision.to_owned());
        if receipts.len() > 128 {
            return Err("起動時案内の既読記録が上限を超えています".into());
        }
        let json = serde_json::to_string(&receipts).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=app_settings.lock_version+1,updated_at=excluded.updated_at", params![KEY,json,hash,now_ms()?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}
fn load(db: &Connection) -> Result<BTreeMap<String, String>, String> {
    let row: Option<(Option<String>, String)> = db.query_row("SELECT CASE WHEN length(CAST(value_json AS BLOB))<=32768 THEN value_json END,content_hash FROM app_settings WHERE key=?1", [KEY], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    let Some((json, hash)) = row else {
        return Ok(BTreeMap::new());
    };
    let json = json.ok_or("起動時案内の記録が容量上限を超えています")?;
    if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
        return Err("起動時案内の記録が変更されています".into());
    }
    let receipts: BTreeMap<String, String> =
        serde_json::from_str(&json).map_err(|_| "起動時案内の記録が不正です")?;
    if receipts.len() > 128
        || receipts.iter().any(|(id, revision)| {
            id.is_empty() || id.len() > 128 || !crate::model_download_ipc::hash(revision)
        })
    {
        return Err("起動時案内の記録が不正です".into());
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_are_bounded_and_hash_bound() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE app_settings(key TEXT,value_json TEXT,content_hash TEXT);")
            .unwrap();
        assert!(load(&db).unwrap().is_empty());
        db.execute("INSERT INTO app_settings VALUES(?1,'{}','changed')", [KEY])
            .unwrap();
        assert!(load(&db).is_err());
    }
}
