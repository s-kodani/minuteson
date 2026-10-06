use super::*;

fn payload(theme: Theme) -> UiPreferences {
    UiPreferences {
        schema_version: 1,
        theme,
    }
}
fn rows(storage: &Storage) -> String {
    storage.connection.lock().unwrap().query_row("SELECT coalesce(json_group_array(json_array(key,value_json,content_hash,lock_version,updated_at)),'[]') FROM app_settings", [], |r| r.get(0)).unwrap()
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
