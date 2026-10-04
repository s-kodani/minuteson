use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesEnginePreferences {
    pub executable_path: String,
    pub model_path: String,
    pub threads: u32,
    pub context_tokens: u32,
    pub prediction_tokens: u32,
    pub gpu_layers: u32,
    pub seed: u32,
    pub prompt_budget: usize,
}
impl Default for MinutesEnginePreferences {
    fn default() -> Self {
        Self {
            executable_path: String::new(),
            model_path: String::new(),
            threads: 4,
            context_tokens: 8192,
            prediction_tokens: 2048,
            gpu_layers: 0,
            seed: 1,
            prompt_budget: 5400,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesPreferences {
    pub standard: MinutesEnginePreferences,
    pub high_accuracy: MinutesEnginePreferences,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesPreferencesSnapshot {
    pub id: String,
    pub payload: MinutesPreferences,
    pub lock_version: i64,
}
fn validate(payload: &MinutesPreferences) -> Result<(), String> {
    for engine in [&payload.standard, &payload.high_accuracy] {
        let budget = engine
            .context_tokens
            .checked_sub(engine.prediction_tokens)
            .and_then(|n| n.checked_sub(512))
            .unwrap_or(0) as usize;
        if !(1..=64).contains(&engine.threads)
            || !(2048..=32768).contains(&engine.context_tokens)
            || !(64..=8192).contains(&engine.prediction_tokens)
            || engine.gpu_layers > 256
            || !(2048..=60000).contains(&engine.prompt_budget)
            || engine.prompt_budget > budget
        {
            return Err(
                "議事録設定のスレッド数・コンテキスト・出力上限・入力上限が不正です".into(),
            );
        }
        for path in [&engine.executable_path, &engine.model_path] {
            if path.len() > 4096
                || path.contains('\0')
                || (!path.is_empty() && !Path::new(path).is_absolute())
            {
                return Err("議事録エンジンとGGUFモデルには絶対パスを指定してください".into());
            }
        }
    }
    Ok(())
}
fn load(db: &Connection) -> Result<MinutesPreferencesSnapshot, String> {
    let row:Option<(String,String,i64)>=db.query_row("SELECT value_json,content_hash,lock_version FROM app_settings WHERE key='llama_profiles'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let (payload, lock_version) = if let Some((text, hash, version)) = row {
        if text.len() > 32 * 1024 || format!("{:x}", Sha256::digest(text.as_bytes())) != hash {
            return Err("議事録設定の容量・内容ハッシュが不正です".into());
        }
        let payload = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        validate(&payload)?;
        (payload, version)
    } else {
        (MinutesPreferences::default(), 0)
    };
    Ok(MinutesPreferencesSnapshot {
        id: "llama_profiles".into(),
        payload,
        lock_version,
    })
}
impl Storage {
    pub(crate) fn minutes_preferences(&self) -> Result<MinutesPreferencesSnapshot, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db)
    }
    pub(crate) fn save_minutes_preferences(
        &self,
        expected: i64,
        payload: MinutesPreferences,
    ) -> Result<MinutesPreferencesSnapshot, String> {
        if expected < 0 {
            return Err("議事録設定の版番号が不正です".into());
        }
        validate(&payload)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let current = load(&tx)?;
        if current.lock_version != expected {
            return Err("別の画面で議事録設定が変更されました".into());
        }
        if current.payload == payload {
            return Ok(current);
        }
        let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let version = expected
            .checked_add(1)
            .ok_or("議事録設定の版番号が範囲外です")?;
        tx.execute("INSERT INTO app_settings (key,value_json,content_hash,lock_version,updated_at) VALUES ('llama_profiles',?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![text,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(MinutesPreferencesSnapshot {
            id: current.id,
            payload,
            lock_version: version,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minutes_settings_are_separate_fixed_versioned_and_checked_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let mut payload = storage.minutes_preferences().unwrap().payload;
        payload.standard.executable_path = "/engines/llama-completion".into();
        payload.standard.model_path = "/models/japanese.gguf".into();
        let saved = storage
            .save_minutes_preferences(0, payload.clone())
            .unwrap();
        assert_eq!(saved.lock_version, 1);
        assert_eq!(
            storage
                .save_minutes_preferences(1, payload.clone())
                .unwrap(),
            saved
        );
        assert!(storage
            .save_minutes_preferences(0, payload.clone())
            .is_err());
        assert_eq!(storage.analysis_preferences().unwrap().lock_version, 0);
        for field in [
            "relative",
            "budget",
            "threads",
            "context",
            "prediction",
            "gpu",
        ] {
            let mut bad = payload.clone();
            match field {
                "relative" => bad.standard.model_path = "relative.gguf".into(),
                "budget" => bad.standard.prompt_budget = 6000,
                "threads" => bad.standard.threads = 65,
                "context" => bad.standard.context_tokens = 2048,
                "prediction" => bad.standard.prediction_tokens = 8192,
                _ => bad.standard.gpu_layers = 257,
            }
            assert!(storage.save_minutes_preferences(1, bad).is_err(), "{field}");
        }
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        assert_eq!(storage.minutes_preferences().unwrap(), saved);
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value_json='{}' WHERE key='llama_profiles'",
                [],
            )
            .unwrap();
        assert!(storage.minutes_preferences().is_err());
    }
}
