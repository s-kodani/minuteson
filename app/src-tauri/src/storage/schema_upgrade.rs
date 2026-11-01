//! Explicit forward upgrades. Past SQL/checksums remain immutable.
use super::*;
mod snapshot;
pub(super) fn save_snapshot(db: &Connection, name: &str) -> Result<(), String> {
    snapshot::save(db, name)
}

pub(super) fn admits(version: usize) -> bool {
    matches!(version, 34 | 35 | 36)
}
pub(super) fn upgrade(db: &mut Connection) -> Result<(), Box<dyn Error>> {
    let mut version = verify_migration_history(db)?;
    if version == 34 {
        legacy_audio_migration::upgrade_schema(db)?;
        version = 35;
    }
    if version == MIGRATIONS.len() {
        return Ok(());
    }
    if version == 35 {
        snapshot::save(db, "before-followup-intents.sqlite3")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(MIGRATION_36)?;
        tx.execute(
            "INSERT INTO schema_migrations VALUES(36,?1,?2)",
            params![
                format!("{:x}", Sha256::digest(MIGRATION_36.as_bytes())),
                now_ms()?
            ],
        )?;
        tx.commit()?;
        version = 36;
    }
    if version != 36 {
        return Err("非対応の移行元DBです".into());
    }
    snapshot::save(db, "before-pcm-import.sqlite3")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATION_37)?;
    tx.execute(
        "INSERT INTO schema_migrations VALUES(37,?1,?2)",
        params![
            format!("{:x}", Sha256::digest(MIGRATION_37.as_bytes())),
            now_ms()?
        ],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema36_pcm_import_upgrade_preserves_history_and_is_idempotent() {
        let mut db = schema35();
        db.execute_batch(MIGRATION_36).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(36,?1,0)",
            [format!("{:x}", Sha256::digest(MIGRATION_36.as_bytes()))],
        )
        .unwrap();
        let before = db
            .prepare("SELECT checksum FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        upgrade(&mut db).unwrap();
        upgrade(&mut db).unwrap();
        assert_eq!(verify_migration_history(&db).unwrap(), 37);
        let after = db
            .prepare("SELECT checksum FROM schema_migrations WHERE version<=36 ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(before, after);
        let trigger: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='audio_import_ready_assets'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(trigger.contains("'audio/x-caf'"));
        assert_eq!(
            db.query_row("SELECT title FROM meetings WHERE id='retained'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "既存会議"
        );
    }
    fn schema35() -> Connection {
        schema35_at(Connection::open_in_memory().unwrap())
    }
    fn schema35_at(db: Connection) -> Connection {
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);").unwrap();
        for &(version, sql) in &MIGRATIONS[..35] {
            db.execute_batch(sql).unwrap();
            db.execute(
                "INSERT INTO schema_migrations VALUES(?1,?2,0)",
                params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
            )
            .unwrap();
        }
        db.execute("INSERT INTO meetings(id,title,created_at,updated_at) VALUES('retained','既存会議',1,2)",[]).unwrap();
        db
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn file_schema35_storage_open_retains_published_meeting_settings_and_private_snapshot_on_reopen(
    ) {
        use crate::storage::deletion::fs::NativeProof as _;
        #[cfg(unix)]
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        crate::windows_files::set_private(root.path()).unwrap();
        let path = root.path().join("meetings.sqlite3");
        let db = schema35_at(Connection::open(&path).unwrap());
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        #[cfg(windows)]
        crate::windows_files::set_private(&path).unwrap();
        let published_hash = format!("{:x}", Sha256::digest(b"synthetic published analysis"));
        db.execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES('published','retained',1,'engine','committed',?1,'{}','{}',?1,1,2)", [&published_hash]).unwrap();
        db.execute("UPDATE meetings SET lifecycle='recorded',source_type='recording',current_analysis_id='published',lock_version=4 WHERE id='retained'", []).unwrap();
        let settings = r#"{"schemaVersion":1,"theme":"dark","colorTheme":"purple"}"#;
        let settings_hash = format!("{:x}", Sha256::digest(settings.as_bytes()));
        db.execute(
            "INSERT INTO app_settings VALUES('ui_preferences',?1,?2,3,2)",
            params![settings, settings_hash],
        )
        .unwrap();
        let history = |db: &Connection| -> Vec<(i64, String, i64)> {
            db.prepare("SELECT version,checksum,applied_at FROM schema_migrations WHERE version<=35 ORDER BY version")
                .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap()
                .collect::<Result<_, _>>().unwrap()
        };
        let original_history = history(&db);
        assert_eq!(original_history.len(), 35);
        assert_eq!(verify_migration_history(&db).unwrap(), 35);
        // Close the real schema-35 source before normal Storage startup.
        drop(db);
        let unknown = root.path().join("unrelated-retained.bin");
        std::fs::write(&unknown, b"synthetic unrelated file; keep exactly").unwrap();

        let storage = Storage::open(root.path().to_owned()).unwrap();
        let meetings = storage.list_meetings().unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].id, "retained");
        assert_eq!(meetings[0].title, "既存会議");
        assert_eq!(meetings[0].lifecycle, "recorded");
        assert_eq!(meetings[0].lock_version, 4);
        let retained = |db: &Connection, version| {
            assert_eq!(verify_migration_history(db).unwrap(), version);
            assert_eq!(history(db), original_history);
            assert_eq!(
                db.query_row(
                    "SELECT current_analysis_id FROM meetings WHERE id='retained'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "published"
            );
            assert_eq!(db.query_row("SELECT publication_state,content_hash FROM analysis_revisions WHERE id='published'", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap(), ("committed".into(), published_hash.clone()));
            assert_eq!(db.query_row("SELECT value_json,content_hash,lock_version FROM app_settings WHERE key='ui_preferences'", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))).unwrap(), (settings.into(), settings_hash.clone(), 3));
            assert_eq!(
                db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert!(!db
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap());
        };
        retained(&storage.connection.lock().unwrap(), 37);
        let preferences = storage.ui_preferences().unwrap();
        assert_eq!(preferences.lock_version, 3);
        assert_eq!(
            serde_json::to_value(preferences.payload).unwrap(),
            serde_json::from_str::<serde_json::Value>(settings).unwrap()
        );
        storage.managed_anchor.verify().unwrap();

        let snapshot = root.path().join("before-followup-intents.sqlite3");
        let metadata = crate::native_fs::symlink_metadata(&snapshot).unwrap();
        let saved_identity = mon_native::identity::Identity::read(&metadata).unwrap();
        #[cfg(unix)]
        assert_eq!(metadata.mode() & 0o777, 0o600);
        #[cfg(windows)]
        assert!(
            crate::windows_files::security_metadata(
                &crate::windows_files::read(&snapshot, false).unwrap()
            )
            .unwrap()
            .1
        );
        let saved_hash = hash_file(&snapshot).unwrap();
        let before =
            Connection::open_with_flags(&snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        retained(&before, 35);
        assert!(!before
            .prepare("SELECT 1 FROM sqlite_schema WHERE name='post_analysis_intents'")
            .unwrap()
            .exists([])
            .unwrap());
        drop(before);
        drop(storage);

        let reopened = Storage::open(root.path().to_owned()).unwrap();
        assert_eq!(reopened.list_meetings().unwrap(), meetings);
        retained(&reopened.connection.lock().unwrap(), 37);
        reopened.managed_anchor.verify().unwrap();
        assert_eq!(hash_file(&snapshot).unwrap(), saved_hash);
        assert_eq!(
            mon_native::identity::Identity::read(
                &crate::native_fs::symlink_metadata(&snapshot).unwrap()
            )
            .unwrap(),
            saved_identity
        );
        assert_eq!(
            std::fs::read(&unknown).unwrap(),
            b"synthetic unrelated file; keep exactly"
        );
    }
    #[test]
    fn exact_schema35_upgrades_additively_without_opting_in_old_meetings() {
        let mut db = schema35();
        let history: Vec<String> = db
            .prepare("SELECT checksum FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        upgrade(&mut db).unwrap();
        upgrade(&mut db).unwrap();
        assert_eq!(verify_migration_history(&db).unwrap(), 37);
        assert_eq!(
            db.query_row("SELECT title FROM meetings WHERE id='retained'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "既存会議"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM post_analysis_intents", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
        let retained: Vec<String> = db
            .prepare("SELECT checksum FROM schema_migrations WHERE version<=35 ORDER BY version")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(retained, history);
    }
    #[test]
    fn tampered_history_and_conflicting_addition_do_not_modify_existing_data() {
        let mut db = schema35();
        db.execute(
            "UPDATE schema_migrations SET checksum='changed' WHERE version=35",
            [],
        )
        .unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='post_analysis_intents'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
        let mut db = schema35();
        db.execute_batch("CREATE TABLE post_analysis_intents(untrusted TEXT);")
            .unwrap();
        assert!(upgrade(&mut db).is_err());
        assert_eq!(verify_migration_history(&db).unwrap(), 35);
        assert_eq!(
            db.query_row("SELECT title FROM meetings WHERE id='retained'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "既存会議"
        );
    }
}
