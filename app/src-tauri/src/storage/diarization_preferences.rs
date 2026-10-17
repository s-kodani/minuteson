use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiarizationEnginePreferences {
    pub executable_path: String,
    pub segmentation_model_path: String,
    pub embedding_model_path: String,
    pub runtime_library_paths: Vec<String>,
    pub threads: u32,
    pub cluster_threshold_milli: u32,
    pub window_shift_milli: u32,
    pub min_duration_on_ms: u32,
    pub min_duration_off_ms: u32,
    #[serde(default, skip_serializing_if = "crate::diarization::sensitivity::is_standard")]
    pub speech_sensitivity_milli: i32,
}
impl Default for DiarizationEnginePreferences {
    fn default() -> Self {
        Self {
            executable_path: String::new(),
            segmentation_model_path: String::new(),
            embedding_model_path: String::new(),
            runtime_library_paths: vec![],
            threads: 4,
            cluster_threshold_milli: 500,
            window_shift_milli: 100,
            min_duration_on_ms: 0,
            min_duration_off_ms: 0,
            speech_sensitivity_milli: 0,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DiarizationPreferences {
    pub standard: DiarizationEnginePreferences,
    pub high_accuracy: DiarizationEnginePreferences,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiarizationPreferencesSnapshot {
    pub id: String,
    pub payload: DiarizationPreferences,
    pub lock_version: i64,
}
pub(super) fn validate(payload: &DiarizationPreferences) -> Result<(), String> {
    for engine in [&payload.standard, &payload.high_accuracy] {
        if !(1..=64).contains(&engine.threads)
            || engine.runtime_library_paths.len() > 16
            || engine.cluster_threshold_milli > 2000
            || !(1..=1000).contains(&engine.window_shift_milli)
            || engine.min_duration_on_ms > 60000
            || engine.min_duration_off_ms > 60000
            || !crate::diarization::sensitivity::valid(engine.speech_sensitivity_milli)
        {
            return Err("話者分離のスレッド数・閾値・窓移動比率・区間長が不正です".into());
        }
        for path in [
            &engine.executable_path,
            &engine.segmentation_model_path,
            &engine.embedding_model_path,
        ]
        .into_iter()
        .chain(engine.runtime_library_paths.iter())
        {
            if path.len() > 4096
                || path.contains(['\0', '\r', '\n'])
                || (!path.is_empty() && !Path::new(path).is_absolute())
            {
                return Err("話者分離には改行を含まない絶対パスを指定してください".into());
            }
        }
        if engine.runtime_library_paths.iter().any(|p| p.is_empty()) {
            return Err("同梱ランタイムに空のパスは指定できません".into());
        }
    }
    Ok(())
}
pub(super) fn load(db: &Connection) -> Result<DiarizationPreferencesSnapshot, String> {
    let row: Option<(String,String,i64)> = db.query_row("SELECT value_json,content_hash,lock_version FROM app_settings WHERE key='diarization_profiles'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let (payload, lock_version) = if let Some((text, hash, version)) = row {
        if text.len() > 192 * 1024 || format!("{:x}", Sha256::digest(text.as_bytes())) != hash {
            return Err("話者分離設定の容量・内容ハッシュが不正です".into());
        }
        let payload = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        validate(&payload)?;
        (payload, version)
    } else {
        (DiarizationPreferences::default(), 0)
    };
    Ok(DiarizationPreferencesSnapshot {
        id: "diarization_profiles".into(),
        payload,
        lock_version,
    })
}
impl Storage {
    pub(crate) fn diarization_preferences(&self) -> Result<DiarizationPreferencesSnapshot, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db)
    }
    pub(crate) fn save_diarization_preferences(
        &self,
        expected: i64,
        payload: DiarizationPreferences,
    ) -> Result<DiarizationPreferencesSnapshot, String> {
        if expected < 0 {
            return Err("話者分離設定の版番号が不正です".into());
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
            return Err("別の画面で話者分離設定が変更されました。入力を確認してください".into());
        }
        if current.payload == payload {
            return Ok(current);
        }
        let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        if text.len() > 192 * 1024 {
            return Err("話者分離設定が大きすぎます".into());
        }
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let version = expected
            .checked_add(1)
            .ok_or("話者分離設定の版番号が範囲外です")?;
        tx.execute("INSERT INTO app_settings (key,value_json,content_hash,lock_version,updated_at) VALUES ('diarization_profiles',?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![text,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(DiarizationPreferencesSnapshot {
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
    fn settings_are_separate_versioned_and_verified_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let whisper = s.analysis_preferences().unwrap();
        let minutes = s.minutes_preferences().unwrap();
        let mut prefs = s.diarization_preferences().unwrap().payload;
        prefs.standard.executable_path = "/engine/sherpa".into();
        prefs.standard.segmentation_model_path = "/models/seg.onnx".into();
        prefs.standard.embedding_model_path = "/models/emb.onnx".into();
        prefs.standard.runtime_library_paths = vec!["/engine/lib/runtime.dylib".into()];
        let saved = s.save_diarization_preferences(0, prefs.clone()).unwrap();
        assert_eq!(saved.lock_version, 1);
        assert!(s.save_diarization_preferences(0, prefs.clone()).is_err());
        assert_eq!(s.analysis_preferences().unwrap().payload, whisper.payload);
        assert_eq!(s.minutes_preferences().unwrap().payload, minutes.payload);
        drop(s);
        let s = Storage::open(root.path().into()).unwrap();
        assert_eq!(s.diarization_preferences().unwrap().payload, prefs);
        assert_eq!(
            s.save_diarization_preferences(1, prefs)
                .unwrap()
                .lock_version,
            1
        );
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value_json='{}' WHERE key='diarization_profiles'",
                [],
            )
            .unwrap();
        assert!(s.diarization_preferences().is_err());
    }
    #[test]
    fn invalid_settings_cannot_replace_saved_values() {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let original = s.diarization_preferences().unwrap().payload;
        for bad in [
            "path",
            "runtime",
            "count",
            "threads",
            "threshold",
            "window",
            "duration",
        ] {
            let mut p = original.clone();
            match bad {
                "path" => p.standard.embedding_model_path = "relative.onnx".into(),
                "runtime" => p.standard.runtime_library_paths = vec!["/lib\n".into()],
                "count" => p.standard.runtime_library_paths = vec!["/lib".into(); 17],
                "threads" => p.high_accuracy.threads = 0,
                "threshold" => p.standard.cluster_threshold_milli = 2001,
                "window" => p.standard.window_shift_milli = 0,
                "duration" => p.standard.min_duration_on_ms = 60001,
                _ => unreachable!(),
            }
            assert!(s.save_diarization_preferences(0, p).is_err(), "{bad}");
        }
        assert_eq!(s.diarization_preferences().unwrap().payload, original);
        assert_eq!(s.diarization_preferences().unwrap().lock_version, 0);
    }
}
