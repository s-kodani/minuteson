use super::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CompletionPreferences {
    pub similarity_threshold_milli: u32,
    pub margin_threshold_milli: u32,
}
impl Default for CompletionPreferences {
    fn default() -> Self {
        Self {
            similarity_threshold_milli: 700,
            margin_threshold_milli: 100,
        }
    }
}
impl CompletionPreferences {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if [self.similarity_threshold_milli, self.margin_threshold_milli]
            .iter()
            .any(|v| *v > 1000 || *v % 10 != 0)
        {
            return Err("閾値は0.00～1.00、0.01刻みで設定してください".into());
        }
        Ok(())
    }
    pub(crate) fn accepts(&self, score: f64, runner_up: Option<f64>, clean: bool) -> bool {
        clean
            && score.is_finite()
            && score + 1e-12 >= self.similarity_threshold_milli as f64 / 1000.
            && runner_up.is_none_or(|next| {
                next.is_finite()
                    && score > next
                    && score - next + 1e-12 >= self.margin_threshold_milli as f64 / 1000.
            })
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub id: String,
    pub payload: CompletionPreferences,
    pub lock_version: i64,
}
impl Storage {
    pub(crate) fn speaker_completion_preferences(&self) -> Result<Snapshot, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&connection)
    }

    pub(crate) fn save_speaker_completion_preferences(
        &self,
        expected_version: i64,
        payload: CompletionPreferences,
    ) -> Result<Snapshot, String> {
        self.require_data_writable()?;
        validate(&payload)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let current = load(&transaction)?;
        if current.lock_version != expected_version {
            return Err(
                "別の画面で未検出発言の話者補完設定が変更されました。再読込してください".into(),
            );
        }
        if current.payload == payload {
            return Ok(current);
        }
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        let version = expected_version
            .checked_add(1)
            .ok_or("未検出発言の話者補完設定の版番号が範囲外です")?;
        transaction.execute("INSERT INTO app_settings (key, value_json, content_hash, lock_version, updated_at) VALUES ('speaker_completion', ?1, ?2, ?3, ?4) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, content_hash = excluded.content_hash, lock_version = excluded.lock_version, updated_at = excluded.updated_at", params![json, hash, version, now_ms()?]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Snapshot {
            id: "speaker_completion".into(),
            payload,
            lock_version: version,
        })
    }
}

pub(super) fn validate(payload: &CompletionPreferences) -> Result<(), String> {
    payload.validate()
}
pub(super) fn load(connection: &Connection) -> Result<Snapshot, String> {
    let row: Option<(String, String, i64)> = connection.query_row("SELECT value_json, content_hash, lock_version FROM app_settings WHERE key = 'speaker_completion'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(|e| e.to_string())?;
    let (payload, lock_version) = if let Some((json, hash, version)) = row {
        if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
            return Err("保存済み未検出発言の話者補完設定の内容が変更されています".into());
        }
        (
            serde_json::from_str(&json).map_err(|e| e.to_string())?,
            version,
        )
    } else {
        (CompletionPreferences::default(), 0)
    };
    validate(&payload)?;
    if lock_version < 0 {
        return Err("未検出発言の話者補完設定の版番号が不正です".into());
    }
    Ok(Snapshot {
        id: "speaker_completion".into(),
        payload,
        lock_version,
    })
}

pub(in crate::storage) fn restore(
    tx: &rusqlite::Transaction<'_>,
    payload: CompletionPreferences,
) -> Result<(), String> {
    payload.validate()?;
    let current = load(tx)?;
    if current.payload == payload {
        return Ok(());
    }
    let version = current
        .lock_version
        .checked_add(1)
        .ok_or("設定の版番号が範囲外です")?;
    let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('speaker_completion',?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![json,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
    Ok(())
}
