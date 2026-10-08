//! Device-local microphone choice. A read never selects a replacement or writes.
use super::*;
const KEY: &str = "capture_microphone";
const MAX_VERSION: i64 = 9_007_199_254_740_991;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Preference {
    schema_version: u8,
    microphone_uid: String,
}

fn load(db: &Connection) -> Result<(Option<String>, i64), String> {
    let row: Option<(Option<String>, String, i64)> = db.query_row(
        "SELECT CASE WHEN length(CAST(value_json AS BLOB))<=4096 THEN value_json END,content_hash,lock_version FROM app_settings WHERE key=?1",
        [KEY], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((json, hash, version)) = row else {
        return Ok((None, 0));
    };
    let json = json.ok_or("前回のマイク設定が容量上限を超えています")?;
    if format!("{:x}", Sha256::digest(json.as_bytes())) != hash
        || !(1..=MAX_VERSION).contains(&version)
    {
        return Err("前回のマイク設定の整合性を確認できません".into());
    }
    let value: Preference = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if value.schema_version != 1 || !super::recordings::valid_microphone_uid(&value.microphone_uid)
    {
        return Err("前回のマイク設定の形式が不正です".into());
    }
    Ok((Some(value.microphone_uid), version))
}

pub(super) fn save(tx: &rusqlite::Transaction<'_>, uid: &str) -> Result<(), String> {
    if !super::recordings::valid_microphone_uid(uid) {
        return Err("マイクの識別情報が不正です".into());
    }
    let (current, version) = load(tx)?;
    if current.as_deref() == Some(uid) {
        return Ok(());
    }
    let version = version
        .checked_add(1)
        .filter(|v| *v <= MAX_VERSION)
        .ok_or("マイク設定の版番号が範囲外です")?;
    let json = serde_json::to_string(&Preference {
        schema_version: 1,
        microphone_uid: uid.into(),
    })
    .map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![KEY,json,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
    Ok(())
}

impl Storage {
    pub(crate) fn selected_microphone_uid(&self) -> Result<Option<String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        Ok(load(&db)?.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(uid: Option<&str>) -> RecordingConfig {
        RecordingConfig {
            mode: "system".into(),
            backend: "screencapturekit".into(),
            target_pids: vec![],
            microphone_device_uid: uid.map(str::to_owned),
            target_application_identity: None,
        }
    }
    #[test]
    fn explicit_microphone_choice_is_atomic_with_recording_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let s = Storage::open(temp.path().into()).unwrap();
        assert_eq!(s.selected_microphone_uid().unwrap(), None);
        let m = s.create_meeting("マイク選択").unwrap();
        let session = s
            .prepare_recording(
                &m.id,
                &config(Some("USB input/名前 with spaces")),
                "mic-start",
            )
            .unwrap();
        assert_eq!(
            s.selected_microphone_uid().unwrap().as_deref(),
            Some("USB input/名前 with spaces")
        );
        assert_eq!(
            s.prepare_recording(
                &m.id,
                &config(Some("USB input/名前 with spaces")),
                "mic-start"
            )
            .unwrap()
            .id,
            session.id
        );
        assert!(s
            .prepare_recording(&m.id, &config(Some("other")), "mic-start")
            .is_err());
        drop(s);
        let s = Storage::open(temp.path().into()).unwrap();
        assert_eq!(
            s.selected_microphone_uid().unwrap().as_deref(),
            Some("USB input/名前 with spaces")
        );
    }
    #[test]
    fn microphone_preference_failure_rolls_back_meeting_and_session() {
        let temp = tempfile::tempdir().unwrap();
        let s = Storage::open(temp.path().into()).unwrap();
        let m = s.create_meeting("原子的な準備").unwrap();
        s.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER mic_save_failure BEFORE INSERT ON app_settings WHEN NEW.key='capture_microphone' BEGIN SELECT RAISE(ABORT,'injected microphone save failure'); END;").unwrap();
        assert!(s
            .prepare_recording(&m.id, &config(Some("usb-123")), "failed-mic")
            .is_err());
        assert_eq!(s.get_meeting(&m.id).unwrap().unwrap(), m);
        assert_eq!(s.selected_microphone_uid().unwrap(), None);
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM recording_sessions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn microphone_preferences_reject_tampering_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let s = Storage::open(temp.path().into()).unwrap();
        let a = s.create_meeting("最初の選択").unwrap();
        let session = s
            .prepare_recording(&a.id, &config(Some("usb-123")), "first-mic")
            .unwrap();
        s.fail_recording(&session.id).unwrap();
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE app_settings SET value_json='{}' WHERE key=?1",
                [KEY],
            )
            .unwrap();
        let b = s.create_meeting("保持する会議").unwrap();
        assert!(s.selected_microphone_uid().is_err());
        assert!(s
            .prepare_recording(&b.id, &config(Some("another")), "new-mic")
            .is_err());
        assert_eq!(s.get_meeting(&b.id).unwrap().unwrap(), b);
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT value_json FROM app_settings WHERE key=?1",
                    [KEY],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "{}"
        );
    }
    #[test]
    fn legacy_recording_conditions_are_not_rewritten_and_uid_validation_is_strict() {
        let legacy = r#"{"mode":"system","backend":"screencapturekit","target_pids":[]}"#;
        let value: RecordingConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(value.microphone_device_uid, None);
        assert_eq!(serde_json::to_string(&value).unwrap(), legacy);
        for uid in ["", " \t", "a\nb", "a\0b", &"x".repeat(1025)] {
            assert!(!super::super::recordings::valid_microphone_uid(uid));
        }
        assert!(super::super::recordings::valid_microphone_uid(
            &"x".repeat(1024)
        ));
    }
}
