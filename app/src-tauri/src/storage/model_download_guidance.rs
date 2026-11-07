//! Only new user databases are seeded. A missing row means no introduction.
use super::*;

const KEY: &str = "model_download_guidance";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Guidance {
    pub pending: bool,
}
pub(super) fn seed(db: &Connection) -> Result<(), String> {
    let json = r#"{"pending":true}"#;
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,?4)", params![KEY,json,hash,now_ms()?]).map_err(|e| e.to_string())?;
    Ok(())
}
fn load(db: &Connection) -> Result<Guidance, String> {
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT value_json,content_hash FROM app_settings WHERE key=?1",
            [KEY],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((json, hash)) = row else {
        return Ok(Guidance { pending: false });
    };
    if json.len() > 128 || format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
        return Err("初回モデル案内の保存状態を確認できません".into());
    }
    serde_json::from_str(&json).map_err(|_| "初回モデル案内の保存形式を確認できません".into())
}
impl Storage {
    pub(crate) fn model_download_guidance(&self) -> Result<Guidance, String> {
        load(
            &*self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?,
        )
    }
    pub(crate) fn dismiss_model_download_guidance(&self) -> Result<Guidance, String> {
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let current = load(&tx)?;
        if current.pending {
            let json = r#"{"pending":false}"#;
            let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
            tx.execute("UPDATE app_settings SET value_json=?1,content_hash=?2,lock_version=lock_version+1,updated_at=?3 WHERE key=?4",params![json,hash,now_ms()?,KEY]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Guidance { pending: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_user_initialization_seeds_once_and_existing_empty_databases_are_skipped() {
        let mut new = Connection::open_in_memory().unwrap();
        super::super::initialize_database_with_guidance(&mut new, true).unwrap();
        assert!(load(&new).unwrap().pending);
        new.execute("DELETE FROM app_settings WHERE key=?1", [KEY])
            .unwrap();
        super::super::initialize_database_with_guidance(&mut new, true).unwrap();
        assert!(!load(&new).unwrap().pending);
        let mut portable = Connection::open_in_memory().unwrap();
        super::super::initialize_database(&mut portable).unwrap();
        assert!(!load(&portable).unwrap().pending);
    }
    #[test]
    fn dismissal_survives_reopen_is_idempotent_and_checks_saved_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = Storage::open(dir.path().to_owned()).unwrap();
        assert!(store.model_download_guidance().unwrap().pending);
        store.dismiss_model_download_guidance().unwrap();
        store.dismiss_model_download_guidance().unwrap();
        drop(store);
        let store = Storage::open(dir.path().to_owned()).unwrap();
        assert!(!store.model_download_guidance().unwrap().pending);
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET content_hash=?1 WHERE key=?2",
                params!["0".repeat(64), KEY],
            )
            .unwrap();
        assert!(store.dismiss_model_download_guidance().is_err());
    }
}
