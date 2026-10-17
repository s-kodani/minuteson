//! Common prompt text is independent of models; each job freezes its own copy.
use super::*;
use std::sync::atomic::Ordering;

const KEY: &str = "minutes_prompt_preferences";
const MAX_VERSION: i64 = 9_007_199_254_740_991;
pub(crate) const MAX_PROMPT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesPromptPreferences {
    pub schema_version: u8,
    pub extract_prompt: String,
    pub merge_prompt: String,
}
impl Default for MinutesPromptPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            extract_prompt: crate::generation::markdown::CONCISE_SYSTEM.into(),
            merge_prompt: crate::generation::markdown::CONCISE_MERGE_SYSTEM.into(),
        }
    }
}
impl MinutesPromptPreferences {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("議事録プロンプト設定の形式版が不正です".into());
        }
        for (name, text) in [
            ("作成用", &self.extract_prompt),
            ("統合用", &self.merge_prompt),
        ] {
            if text.trim().is_empty() || text.contains('\0') || text.len() > MAX_PROMPT_BYTES {
                return Err(format!("{name}プロンプトは空白のみを避け、NUL文字を含めずUTF-8で16KiB以内にしてください"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesPromptPreferencesSnapshot {
    pub id: String,
    pub payload: MinutesPromptPreferences,
    pub lock_version: i64,
    pub defaults: MinutesPromptPreferences,
}

pub(crate) fn load(db: &Connection) -> Result<MinutesPromptPreferencesSnapshot, String> {
    let row: Option<(Option<String>, String, i64)> = db.query_row(
        "SELECT CASE WHEN length(CAST(value_json AS BLOB))<=262144 THEN value_json END,content_hash,lock_version FROM app_settings WHERE key=?1",
        [KEY], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional().map_err(|e| e.to_string())?;
    let (payload, lock_version) = match row {
        None => (MinutesPromptPreferences::default(), 0),
        Some((json, hash, version)) => {
            let json = json.ok_or("議事録プロンプト設定の容量が不正です")?;
            if !(1..=MAX_VERSION).contains(&version)
                || format!("{:x}", Sha256::digest(json.as_bytes())) != hash
            {
                return Err("保存済み議事録プロンプト設定の版・内容が不正です".into());
            }
            (
                serde_json::from_str::<MinutesPromptPreferences>(&json)
                    .map_err(|e| e.to_string())?,
                version,
            )
        }
    };
    payload.validate()?;
    Ok(MinutesPromptPreferencesSnapshot {
        id: KEY.into(),
        payload,
        lock_version,
        defaults: MinutesPromptPreferences::default(),
    })
}
impl Storage {
    pub(crate) fn minutes_prompt_preferences(
        &self,
    ) -> Result<MinutesPromptPreferencesSnapshot, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db)
    }
    pub(crate) fn save_minutes_prompt_preferences(
        &self,
        expected_lock_version: i64,
        payload: MinutesPromptPreferences,
    ) -> Result<MinutesPromptPreferencesSnapshot, String> {
        payload.validate()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        if self.maintenance.load(Ordering::Acquire) {
            return Err("データ保守中のため議事録プロンプトを保存できません".into());
        }
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let current = load(&tx)?;
        if current.lock_version != expected_lock_version {
            return Err(
                "議事録プロンプト設定が変更されています。入力を確認して再読込してください".into(),
            );
        }
        if current.payload == payload {
            return Ok(current);
        }
        let version = expected_lock_version
            .checked_add(1)
            .filter(|v| (1..=MAX_VERSION).contains(v))
            .ok_or("議事録プロンプト設定の版番号が範囲外です")?;
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at", params![KEY,json,hash,version,now_ms()?]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(MinutesPromptPreferencesSnapshot {
            id: KEY.into(),
            payload,
            lock_version: version,
            defaults: current.defaults,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_prompts_persist_reset_independently_and_reject_conflicts_and_invalid_text() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let initial = storage.minutes_prompt_preferences().unwrap();
        assert_eq!(initial.payload, initial.defaults);
        let mut custom = initial.payload.clone();
        custom.extract_prompt = "表にまとめる。\n文章形式は自由。".into();
        custom.merge_prompt = "全下書きを一つの表へ統合する。".into();
        let saved = storage
            .save_minutes_prompt_preferences(0, custom.clone())
            .unwrap();
        assert_eq!(saved.lock_version, 1);
        assert!(storage
            .save_minutes_prompt_preferences(0, custom.clone())
            .is_err());
        for bad_text in [
            " ".into(),
            "a\0b".into(),
            "あ".repeat(MAX_PROMPT_BYTES / 3 + 1),
        ] {
            let mut bad = custom.clone();
            bad.extract_prompt = bad_text;
            assert!(storage.save_minutes_prompt_preferences(1, bad).is_err());
        }
        let mut reset = custom.clone();
        reset.extract_prompt = initial.defaults.extract_prompt;
        let saved = storage.save_minutes_prompt_preferences(1, reset).unwrap();
        assert_eq!(saved.payload.merge_prompt, custom.merge_prompt);
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        assert_eq!(storage.minutes_prompt_preferences().unwrap(), saved);
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value_json='{}' WHERE key=?1",
                [KEY],
            )
            .unwrap();
        assert!(storage.minutes_prompt_preferences().is_err());
    }
}
