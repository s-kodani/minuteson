use super::*;

fn payload(theme: Theme) -> UiPreferences {
    UiPreferences::V1 {
        schema_version: 1,
        theme,
    }
}
fn rows(storage: &Storage) -> String {
    storage.connection.lock().unwrap().query_row("SELECT coalesce(json_group_array(json_array(key,value_json,content_hash,lock_version,updated_at)),'[]') FROM app_settings", [], |r| r.get(0)).unwrap()
}

fn display(theme: Theme, text_size: u8, follow_playback: bool) -> UiPreferences {
    UiPreferences::V2 {
        schema_version: 2,
        theme,
        text_size,
        follow_playback,
    }
}

#[test]
fn ui_preferences_old_bytes_are_preserved_until_explicit_display_save_and_cold_restart() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage
        .create_meeting("DISPLAY MUST NOT EDIT THIS")
        .unwrap();
    let json = "{\n  \"theme\":\"dark\", \"schemaVersion\":1\n}";
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_settings VALUES(?1,?2,?3,7,1)",
            params![KEY, json, format!("{:x}", Sha256::digest(json.as_bytes()))],
        )
        .unwrap();
    let before = rows(&storage);
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
    assert_eq!(
        storage
            .save_ui_preferences(7, payload(Theme::Dark))
            .unwrap()
            .lock_version,
        7
    );
    assert_eq!(rows(&storage), before);
    let saved = storage
        .save_ui_preferences(7, display(Theme::Dark, 18, true))
        .unwrap();
    assert_eq!(saved.lock_version, 8);
    assert_eq!(
        storage.get_meeting(&meeting.id).unwrap(),
        Some(meeting.clone())
    );
    assert_eq!(storage.analysis_preferences().unwrap().lock_version, 0);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        display(Theme::Dark, 18, true)
    );
    assert_eq!(storage.get_meeting(&meeting.id).unwrap(), Some(meeting));
    let saved = storage
        .save_ui_preferences(8, display(Theme::Light, 18, true))
        .unwrap();
    assert_eq!(saved.lock_version, 9);
    assert_eq!(saved.payload, display(Theme::Light, 18, true));
    let before = rows(&storage);
    assert!(storage
        .save_ui_preferences(9, payload(Theme::System))
        .unwrap_err()
        .contains("旧形式"));
    assert!(storage
        .save_ui_preferences(8, display(Theme::Dark, 13, false))
        .is_err());
    assert_eq!(rows(&storage), before);
    assert_eq!(
        storage
            .save_ui_preferences(9, display(Theme::Light, 18, true))
            .unwrap()
            .lock_version,
        9
    );
    assert_eq!(rows(&storage), before);
}

#[test]
fn ui_preferences_display_contract_requires_complete_known_fields_and_allowed_size() {
    for json in [
        r#"{"schemaVersion":2,"theme":"system"}"#,
        r#"{"schemaVersion":2,"theme":"system","textSize":15,"followPlayback":null}"#,
        r#"{"schemaVersion":2,"theme":"system","textSize":15,"followPlayback":1}"#,
        r#"{"schemaVersion":2,"theme":"system","textSize":15,"followPlayback":"true"}"#,
        r#"{"schemaVersion":2,"theme":"system","textSize":15.5,"followPlayback":true}"#,
        r#"{"schemaVersion":2,"theme":"system","textSize":15,"followPlayback":true,"extra":null}"#,
        r#"{"schemaVersion":1,"theme":"dark","textSize":null,"followPlayback":null}"#,
        r#"{"schemaVersion":2,"theme":"unknown","textSize":15,"followPlayback":true}"#,
    ] {
        if let Ok(value) = serde_json::from_str::<UiPreferences>(json) {
            assert!(validate(&value).is_err(), "{json}");
        }
    }
    for size in [13, 15, 18, 20] {
        let value = display(Theme::System, size, false);
        validate(&value).unwrap();
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"schemaVersion":2,"theme":"system","textSize":size,"followPlayback":false})
        );
        assert_eq!(
            serde_json::from_value::<UiPreferences>(json).unwrap(),
            value
        );
    }
    for value in [
        display(Theme::System, 0, false),
        display(Theme::Light, 14, true),
        display(Theme::Dark, 255, true),
        UiPreferences::V1 {
            schema_version: 2,
            theme: Theme::System,
        },
        UiPreferences::V2 {
            schema_version: 1,
            theme: Theme::System,
            text_size: 15,
            follow_playback: true,
        },
    ] {
        assert!(validate(&value).is_err());
    }
}

#[test]
fn ui_preferences_invalid_display_records_remain_unmodified_instead_of_defaulting() {
    for json in [
        r#"{"schemaVersion":2,"theme":"dark","textSize":14,"followPlayback":true}"#,
        r#"{"schemaVersion":1,"theme":"dark","textSize":15,"followPlayback":true}"#,
        r#"{"schemaVersion":3,"theme":"dark","textSize":15,"followPlayback":true}"#,
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings VALUES(?1,?2,?3,1,1)",
                params![KEY, json, format!("{:x}", Sha256::digest(json.as_bytes()))],
            )
            .unwrap();
        let before = rows(&storage);
        assert!(storage.ui_preferences().is_err());
        assert!(storage
            .save_ui_preferences(1, display(Theme::Light, 15, false))
            .is_err());
        assert_eq!(rows(&storage), before);
    }
}

#[test]
fn ui_preferences_display_write_failure_keeps_old_theme_and_size_together() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, display(Theme::Dark, 20, true))
        .unwrap();
    let before = rows(&storage);
    storage.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER display_failure BEFORE INSERT ON app_settings WHEN NEW.key='ui_preferences' BEGIN SELECT RAISE(ABORT,'injected display save failure'); END;").unwrap();
    assert!(storage
        .save_ui_preferences(1, display(Theme::Light, 13, false))
        .is_err());
    assert_eq!(rows(&storage), before);
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        display(Theme::Dark, 20, true)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn ui_preferences_native_maintenance_blocks_complete_display_writes_but_keeps_reads() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, display(Theme::Dark, 20, true))
        .unwrap();
    let before = rows(&storage);
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        display(Theme::Dark, 20, true)
    );
    assert!(storage
        .save_ui_preferences(1, display(Theme::Light, 13, false))
        .is_err());
    assert_eq!(rows(&storage), before);
    snapshot.retire_source_for_restart(&|| false).unwrap();
    drop(snapshot);
    assert!(storage
        .save_ui_preferences(1, display(Theme::Light, 13, false))
        .is_err());
    assert_eq!(rows(&storage), before);
}

#[test]
fn ui_preferences_default_is_read_only_and_theme_persists_without_changing_content() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("THEME MUST NOT EDIT THIS").unwrap();
    let initial = storage.ui_preferences().unwrap();
    assert_eq!(initial.id, KEY);
    assert_eq!(initial.lock_version, 0);
    assert_eq!(initial.payload, payload(Theme::System));
    assert_eq!(rows(&storage), "[]");
    assert_eq!(
        storage
            .save_ui_preferences(0, initial.payload)
            .unwrap()
            .lock_version,
        0
    );
    assert_eq!(rows(&storage), "[]");
    let saved = storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    assert_eq!(saved.lock_version, 1);
    assert_eq!(
        storage.get_meeting(&meeting.id).unwrap(),
        Some(meeting.clone())
    );
    assert_eq!(storage.analysis_preferences().unwrap().lock_version, 0);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
    assert_eq!(storage.get_meeting(&meeting.id).unwrap(), Some(meeting));
    assert_eq!(
        storage
            .save_ui_preferences(1, payload(Theme::Light))
            .unwrap()
            .lock_version,
        2
    );
    assert_eq!(
        storage
            .save_ui_preferences(2, payload(Theme::System))
            .unwrap()
            .lock_version,
        3
    );
}

#[test]
fn ui_preferences_stale_writes_idempotent_save_and_version_limit_preserve_rows() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    let before = rows(&storage);
    assert!(storage
        .save_ui_preferences(0, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
    assert_eq!(
        storage
            .save_ui_preferences(1, payload(Theme::Dark))
            .unwrap()
            .lock_version,
        1
    );
    assert_eq!(rows(&storage), before);
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE app_settings SET lock_version=?1 WHERE key=?2",
            params![MAX_VERSION, KEY],
        )
        .unwrap();
    let limit = rows(&storage);
    assert!(storage
        .save_ui_preferences(MAX_VERSION, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), limit);
}

#[test]
fn ui_preferences_corrupt_or_unsupported_saved_settings_are_not_overwritten() {
    for json in [
        r#"{"schemaVersion":2,"theme":"dark"}"#,
        r#"{"schemaVersion":1,"theme":"unknown"}"#,
        r#"{"schemaVersion":1,"theme":"dark","extra":true}"#,
        r#"{"theme":"dark"}"#,
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO app_settings VALUES(?1,?2,?3,1,1)",
                params![KEY, json, hash],
            )
            .unwrap();
        let before = rows(&storage);
        assert!(storage.ui_preferences().is_err());
        assert!(storage
            .save_ui_preferences(1, payload(Theme::Light))
            .is_err());
        assert_eq!(rows(&storage), before);
    }
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE app_settings SET content_hash=?1 WHERE key=?2",
            params!["0".repeat(64), KEY],
        )
        .unwrap();
    let before = rows(&storage);
    assert!(storage.ui_preferences().is_err());
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
}

#[test]
fn ui_preferences_size_limit_and_save_failure_do_not_create_or_replace_settings() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER theme_failure BEFORE INSERT ON app_settings WHEN NEW.key='ui_preferences' BEGIN SELECT RAISE(ABORT,'injected theme save failure'); END;").unwrap();
    assert!(storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .is_err());
    assert_eq!(rows(&storage), "[]");
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER theme_failure")
        .unwrap();
    let json = serde_json::to_string(
        &serde_json::json!({"schemaVersion":1,"theme":"dark","extra":"x".repeat(4096)}),
    )
    .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO app_settings VALUES(?1,?2,?3,1,1)",
            params![KEY, json, format!("{:x}", Sha256::digest(json.as_bytes()))],
        )
        .unwrap();
    let before = rows(&storage);
    assert!(storage.ui_preferences().unwrap_err().contains("容量上限"));
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
}

#[cfg(target_os = "macos")]
#[test]
fn ui_preferences_held_native_snapshot_and_retired_storage_never_accept_theme_writes() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    let before = rows(&storage);
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
    drop(snapshot);
    storage
        .save_ui_preferences(1, payload(Theme::Light))
        .unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    snapshot.retire_source_for_restart(&|| false).unwrap();
    drop(snapshot);
    let before = rows(&storage);
    assert!(storage.restart_required());
    assert!(storage
        .save_ui_preferences(2, payload(Theme::Dark))
        .is_err());
    assert_eq!(rows(&storage), before);
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Light)
    );
}
