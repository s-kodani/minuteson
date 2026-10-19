use super::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct VoiceMatchingPreferences {
    pub similarity_threshold_milli: u32,
    pub margin_threshold_milli: u32,
}
impl Default for VoiceMatchingPreferences {
    fn default() -> Self {
        Self {
            similarity_threshold_milli: 800,
            margin_threshold_milli: 100,
        }
    }
}
impl VoiceMatchingPreferences {
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
    pub(crate) fn policy_version(&self) -> String {
        format!(
            "user_thresholds_v1:{}:{}:solo3000000",
            self.similarity_threshold_milli, self.margin_threshold_milli
        )
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub id: String,
    pub payload: VoiceMatchingPreferences,
    pub lock_version: i64,
}
impl Storage {
    pub(crate) fn voice_matching_preferences(&self) -> Result<Snapshot, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&connection)
    }

    pub(crate) fn save_voice_matching_preferences(
        &self,
        expected_version: i64,
        payload: VoiceMatchingPreferences,
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
            return Err("別の画面で声の自動適用設定が変更されました。再読込してください".into());
        }
        if current.payload == payload {
            return Ok(current);
        }
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        let version = expected_version
            .checked_add(1)
            .ok_or("声の自動適用設定の版番号が範囲外です")?;
        transaction.execute("INSERT INTO app_settings (key, value_json, content_hash, lock_version, updated_at) VALUES ('voice_matching', ?1, ?2, ?3, ?4) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, content_hash = excluded.content_hash, lock_version = excluded.lock_version, updated_at = excluded.updated_at", params![json, hash, version, now_ms()?]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Snapshot {
            id: "voice_matching".into(),
            payload,
            lock_version: version,
        })
    }
}

pub(super) fn validate(payload: &VoiceMatchingPreferences) -> Result<(), String> {
    payload.validate()
}
pub(super) fn load(connection: &Connection) -> Result<Snapshot, String> {
    let row: Option<(String, String, i64)> = connection.query_row("SELECT value_json, content_hash, lock_version FROM app_settings WHERE key = 'voice_matching'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(|e| e.to_string())?;
    let (payload, lock_version) = if let Some((json, hash, version)) = row {
        if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
            return Err("保存済み声の自動適用設定の内容が変更されています".into());
        }
        (
            serde_json::from_str(&json).map_err(|e| e.to_string())?,
            version,
        )
    } else {
        (VoiceMatchingPreferences::default(), 0)
    };
    validate(&payload)?;
    if lock_version < 0 {
        return Err("声の自動適用設定の版番号が不正です".into());
    }
    Ok(Snapshot {
        id: "voice_matching".into(),
        payload,
        lock_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_and_ties() {
        let p = VoiceMatchingPreferences::default();
        assert!(p.accepts(0.9, Some(0.79), true));
        assert!(!p.accepts(0.9, Some(0.85), true));
        assert!(p.accepts(0.9, Some(0.8), true));
        assert!(!p.accepts(0.99, None, false));
        assert!(p.accepts(0.8, None, true));
        let p = VoiceMatchingPreferences {
            similarity_threshold_milli: 0,
            margin_threshold_milli: 0,
        };
        assert!(!p.accepts(0.9, Some(0.9), true));
        assert!(p.accepts(0.3, None, true));
    }
    #[test]
    fn persistent_versioned_settings() {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        assert_eq!(
            s.voice_matching_preferences().unwrap().payload,
            VoiceMatchingPreferences::default()
        );
        let p = VoiceMatchingPreferences {
            similarity_threshold_milli: 700,
            margin_threshold_milli: 50,
        };
        assert_eq!(
            s.save_voice_matching_preferences(0, p)
                .unwrap()
                .lock_version,
            1
        );
        assert!(s.save_voice_matching_preferences(0, p).is_err());
        assert!(s
            .save_voice_matching_preferences(
                1,
                VoiceMatchingPreferences {
                    similarity_threshold_milli: 1001,
                    ..p
                }
            )
            .is_err());
        drop(s);
        let s = Storage::open(root.path().into()).unwrap();
        assert_eq!(s.voice_matching_preferences().unwrap().payload, p);
    }
}

pub(in crate::storage) fn restore(
    tx: &rusqlite::Transaction<'_>,
    payload: VoiceMatchingPreferences,
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
    tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('voice_matching',?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![json,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
    Ok(())
}
