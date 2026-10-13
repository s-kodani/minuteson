//! Device-local presentation only; no document, audio or engine mutations.
use super::*;
use std::sync::atomic::Ordering;

const KEY: &str = "ui_preferences";
const MAX_VERSION: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields, rename_all_fields = "camelCase")]
pub(crate) enum UiPreferences {
    V1 {
        schema_version: u8,
        theme: Theme,
    },
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self::V1 {
            schema_version: 1,
            theme: Theme::System,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiPreferencesSnapshot {
    pub(crate) id: String,
    pub(crate) payload: UiPreferences,
    pub(crate) lock_version: i64,
}

fn validate(payload: &UiPreferences) -> Result<(), String> {
    match payload {
        UiPreferences::V1 {
            schema_version: 1, ..
        } => Ok(()),
        _ => Err("テーマ設定の形式版に対応していません".into()),
    }
}
fn load(db: &Connection) -> Result<UiPreferencesSnapshot, String> {
    let row: Option<(Option<String>, String, i64)> = db.query_row(
        "SELECT CASE WHEN length(CAST(value_json AS BLOB))<=4096 THEN value_json END,content_hash,lock_version FROM app_settings WHERE key=?1",
        [KEY], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional().map_err(|e| e.to_string())?;
    let (payload, version) = match row {
        None => (UiPreferences::default(), 0),
        Some((json, hash, version)) => {
            let json = json.ok_or("表示設定が容量上限を超えています")?;
            if format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
                return Err("保存済み表示設定の内容が変更されています".into());
            }
            if !(1..=MAX_VERSION).contains(&version) {
                return Err("表示設定の版番号が不正です".into());
            }
            (
                serde_json::from_str(&json).map_err(|e| e.to_string())?,
                version,
            )
        }
    };
    validate(&payload)?;
    Ok(UiPreferencesSnapshot {
        id: KEY.into(),
        payload,
        lock_version: version,
    })
}

impl Storage {
    pub(crate) fn ui_preferences(&self) -> Result<UiPreferencesSnapshot, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load(&db)
    }

    pub(crate) fn save_ui_preferences(
        &self,
        expected_version: i64,
        payload: UiPreferences,
    ) -> Result<UiPreferencesSnapshot, String> {
        validate(&payload)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.require_data_writable()?;
        if self.maintenance.load(Ordering::Acquire) {
            return Err("データ保守中のため表示設定を保存できません".into());
        }
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let current = load(&tx)?;
        if expected_version != current.lock_version {
            return Err("表示設定が変更されています。保存済み設定を再読込してください".into());
        }
        if current.payload == payload {
            return Ok(current);
        }
        let version = expected_version
            .checked_add(1)
            .filter(|v| *v <= MAX_VERSION)
            .ok_or("表示設定の版番号が範囲外です")?;
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![KEY,json,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(UiPreferencesSnapshot {
            id: KEY.into(),
            payload,
            lock_version: version,
        })
    }
}

#[cfg(test)]
mod tests;
