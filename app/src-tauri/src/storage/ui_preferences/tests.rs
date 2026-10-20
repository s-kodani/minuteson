use super::*;

fn payload(theme: Theme) -> UiPreferences {
    UiPreferences::V1 {
        schema_version: 1,
        theme,
        color_theme: ColorTheme::Monochrome,
    }
}
fn rows(storage: &Storage) -> String {
    storage.connection.lock().unwrap().query_row("SELECT coalesce(json_group_array(json_array(key,value_json,content_hash,lock_version,updated_at)),'[]') FROM app_settings WHERE key='ui_preferences'", [], |r| r.get(0)).unwrap()
}

#[test]
fn ui_preferences_color_is_independent_persisted_and_version_bound() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage
        .create_meeting("color must not edit content")
        .unwrap();
    let purple = UiPreferences::V1 {
        schema_version: 1,
        theme: Theme::Dark,
        color_theme: ColorTheme::Purple,
    };
    let saved = storage.save_ui_preferences(0, purple.clone()).unwrap();
    assert_eq!(saved.lock_version, 1);
    assert!(storage
        .save_ui_preferences(0, payload(Theme::Light))
        .is_err());
    assert_eq!(storage.ui_preferences().unwrap().payload, purple);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(storage.ui_preferences().unwrap().payload, purple);
    assert_eq!(storage.get_meeting(&meeting.id).unwrap(), Some(meeting));
    assert_eq!(
        storage
            .save_ui_preferences(1, payload(Theme::Dark))
            .unwrap()
            .lock_version,
        2
    );
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
}

#[test]
fn ui_preferences_absent_color_defaults_without_rewriting_or_accepting_invalid_color() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let json = r#"{"schemaVersion":1,"theme":"dark"}"#;
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
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
    assert_eq!(rows(&storage), before);
    for color in ["null", "\"blue\"", "true", "1"] {
        let json = format!(r#"{{"schemaVersion":1,"theme":"dark","colorTheme":{color}}}"#);
        assert!(serde_json::from_str::<UiPreferences>(&json).is_err());
    }
}

#[test]
fn ui_preferences_invalid_display_records_remain_unmodified_instead_of_defaulting() {
    for json in [
        r#"{"schemaVersion":2,"theme":"dark","textSize":15,"followPlayback":false}"#,
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
            .save_ui_preferences(1, payload(Theme::Light))
            .is_err());
        assert_eq!(rows(&storage), before);
    }
}

#[test]
fn ui_preferences_theme_write_failure_keeps_saved_theme() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    let before = rows(&storage);
    storage.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER display_failure BEFORE INSERT ON app_settings WHEN NEW.key='ui_preferences' BEGIN SELECT RAISE(ABORT,'injected display save failure'); END;").unwrap();
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn ui_preferences_native_maintenance_blocks_theme_writes_but_keeps_reads() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .save_ui_preferences(0, payload(Theme::Dark))
        .unwrap();
    let before = rows(&storage);
    let snapshot = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        storage.ui_preferences().unwrap().payload,
        payload(Theme::Dark)
    );
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
        .is_err());
    assert_eq!(rows(&storage), before);
    snapshot
        .retire_source_for_restart(&crate::test_support::never_cancelled)
        .unwrap();
    drop(snapshot);
    assert!(storage
        .save_ui_preferences(1, payload(Theme::Light))
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
    let snapshot = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
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
    let snapshot = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    snapshot
        .retire_source_for_restart(&crate::test_support::never_cancelled)
        .unwrap();
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
