use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EnginePreferences {
    pub executable_path: String,
    pub model_path: String,
    pub vad_model_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vad_options: Option<crate::transcription::vad::VadOptions>,
    pub threads: u32,
    pub use_gpu: bool,
}

impl Default for EnginePreferences {
    fn default() -> Self {
        Self {
            executable_path: String::new(),
            model_path: String::new(),
            vad_model_path: None,
            vad_options: None,
            threads: 4,
            use_gpu: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnalysisPreferences {
    pub standard: EnginePreferences,
    pub high_accuracy: EnginePreferences,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesSnapshot {
    pub id: String,
    pub payload: AnalysisPreferences,
    pub lock_version: i64,
}

impl Storage {
    pub(crate) fn analysis_preferences(&self) -> Result<PreferencesSnapshot, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&connection)
    }

    pub(crate) fn save_analysis_preferences(
        &self,
        expected_version: i64,
        payload: AnalysisPreferences,
    ) -> Result<PreferencesSnapshot, String> {
        validate(&payload)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let current = load(&transaction)?;
        if current.lock_version != expected_version {
            return Err("別の画面で解析設定が変更されました。再読込してください".into());
        }
        if current.payload == payload {
            return Ok(current);
        }
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        let version = expected_version
            .checked_add(1)
            .ok_or("解析設定の版番号が範囲外です")?;
        transaction.execute("INSERT INTO app_settings (key, value_json, content_hash, lock_version, updated_at) VALUES ('whisper_profiles', ?1, ?2, ?3, ?4) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, content_hash = excluded.content_hash, lock_version = excluded.lock_version, updated_at = excluded.updated_at", params![json, hash, version, now_ms()?]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(PreferencesSnapshot {
            id: "whisper_profiles".into(),
            payload,
            lock_version: version,
        })
    }
}

pub(super) fn validate(payload: &AnalysisPreferences) -> Result<(), String> {
    for engine in [&payload.standard, &payload.high_accuracy] {
        if let Some(options) = engine.vad_options { options.validate()?; }
        if !(1..=64).contains(&engine.threads) {
            return Err("スレッド数は1～64で設定してください".into());
        }
        for path in [
            Some(&engine.executable_path),
            Some(&engine.model_path),
            engine.vad_model_path.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if path.len() > 4096
                || (!path.is_empty() && !Path::new(path).is_absolute())
                || path.contains('\0')
            {
                return Err("モデル・実行ファイルには絶対パスを入力してください".into());
            }
        }
    }
    Ok(())
}
pub(super) fn load(connection: &Connection) -> Result<PreferencesSnapshot, String> {
    let row: Option<(String, String, i64)> = connection.query_row("SELECT value_json, content_hash, lock_version FROM app_settings WHERE key = 'whisper_profiles'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(|e| e.to_string())?;
    let (payload, lock_version) = if let Some((json, hash, version)) = row {
        if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
            return Err("保存済み解析設定の内容が変更されています".into());
        }
        (
            serde_json::from_str(&json).map_err(|e| e.to_string())?,
            version,
        )
    } else {
        (AnalysisPreferences::default(), 0)
    };
    validate(&payload)?;
    if lock_version < 0 {
        return Err("解析設定の版番号が不正です".into());
    }
    Ok(PreferencesSnapshot {
        id: "whisper_profiles".into(),
        payload,
        lock_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_versioned_persistent_and_reject_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        let initial = storage.analysis_preferences().unwrap();
        assert_eq!(initial.lock_version, 0);
        let mut payload = initial.payload;
        payload.standard.model_path = "/models/multilingual.bin".into();
        payload.standard.executable_path = "/engines/whisper-cli".into();
        let saved = storage
            .save_analysis_preferences(0, payload.clone())
            .unwrap();
        assert_eq!(saved.lock_version, 1);
        assert!(storage
            .save_analysis_preferences(0, AnalysisPreferences::default())
            .is_err());
        drop(storage);
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        assert_eq!(storage.analysis_preferences().unwrap().payload, payload);
        let mut relative = payload.clone();
        relative.standard.model_path = "relative.bin".into();
        assert!(storage.save_analysis_preferences(1, relative).is_err());
        let mut invalid = payload;
        invalid.high_accuracy.threads = 0;
        assert!(storage.save_analysis_preferences(1, invalid).is_err());
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value_json = '{}' WHERE key = 'whisper_profiles'",
                [],
            )
            .unwrap();
        assert!(storage.analysis_preferences().is_err());
    }
}
