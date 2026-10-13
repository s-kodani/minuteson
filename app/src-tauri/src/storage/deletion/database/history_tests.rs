use super::*;
use crate::storage::{initialize_database, Storage, MIGRATIONS};

fn database(version: usize) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
    for (number, sql) in &MIGRATIONS[..version] {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,1)",
            params![number, digest(sql.as_bytes())],
        )
        .unwrap();
    }
    db
}

fn empty_scope(version: usize) -> Scope {
    let known = known_schemas()
        .unwrap()
        .iter()
        .find(|s| s.version == version)
        .unwrap();
    Scope {
        version: 1,
        meeting_id: uuid::Uuid::new_v4().to_string(),
        schema_hash: known.hash.clone(),
        tables: known
            .targets
            .iter()
            .map(|(name, _)| Table {
                name: (*name).into(),
                rows: u64::from(*name == "meetings"),
                sha256: "a".repeat(64),
            })
            .collect(),
        maintenance_hash: "b".repeat(64),
    }
}

#[test]
fn history_schema_checks_do_not_admit_old_database_initialization() {
    for version in 22..=MIGRATIONS.len() {
        let mut db = database(version);
        let scope = empty_scope(version);
        let original = scope.encode().unwrap();
        scope.validate_format(&db).unwrap();
        verify_rows_absent(&db, &scope, &|| false).unwrap();
        assert_eq!(trusted_history_schema(&db).unwrap(), scope.schema_hash);
        assert_eq!(trusted_schema(&db).is_ok(), version == MIGRATIONS.len());
        assert_eq!(initialize_database(&mut db).is_ok(), version == MIGRATIONS.len());
        scope.validate_format(&db).unwrap();
        verify_rows_absent(&db, &scope, &|| false).unwrap();
        assert_eq!(scope.encode().unwrap(), original);
        assert_eq!(
            scope.validate_current(&db).is_ok(),
            version == MIGRATIONS.len()
        );
        let tx = db.transaction().unwrap();
        assert!(erase_rows(&tx, &scope, &|| false).is_err());
        // A current empty synthetic scope also refuses the claimed meeting row
        // count; no older scope can reach the first UPDATE/DELETE at all.
    }
}

#[test]
fn history_schema_rejects_unknown_backward_or_truncated_scope_despite_rehashed_values() {
    let db = database(MIGRATIONS.len());
    for change in 0..5 {
        let mut scope = empty_scope(27);
        match change {
            0 => scope.schema_hash = "a".repeat(64),
            1 => {
                scope.tables.remove(0);
            }
            2 => scope.tables.reverse(),
            3 => scope.tables[0].name = "app_settings".into(),
            _ => scope.tables.push(Table {
                name: "transcript_edit_recovery".into(),
                rows: 0,
                sha256: "a".repeat(64),
            }),
        }
        assert!(scope.validate_format(&db).is_err());
    }
    assert!(empty_scope(MIGRATIONS.len())
        .validate_format(&database(27))
        .is_err());
}

#[test]
fn history_schema_never_trusts_matching_version_with_changed_structure_or_disabled_guards() {
    for change in 0..5 {
        let db = database(27);
        match change {
            0 => db
                .execute_batch("CREATE TABLE unknown_scope(body TEXT)")
                .unwrap(),
            1 => db
                .execute_batch("DROP TRIGGER maintenance_operation_fixed")
                .unwrap(),
            2 => db.execute_batch("PRAGMA foreign_keys=OFF").unwrap(),
            3 => {
                db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                    .unwrap();
            }
            _ => {
                db.execute(
                    "UPDATE schema_migrations SET checksum=?1 WHERE version=27",
                    ["a".repeat(64)],
                )
                .unwrap();
            }
        }
        assert!(trusted_history_schema(&db).is_err());
        assert!(empty_scope(27).validate_format(&db).is_err());
    }
    let db = database(MIGRATIONS.len());
    db.execute_batch("DELETE FROM schema_migrations WHERE version=28")
        .unwrap();
    assert!(trusted_history_schema(&db).is_err());
}

#[test]
fn history_scope_absence_checks_new_rows_without_authorizing_their_deletion() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let meeting = storage
        .create_meeting("NEW ROWS MUST SURVIVE OLD CONSENT")
        .unwrap()
        .id;
    let mut scope = empty_scope(27);
    scope.meeting_id = meeting.clone();
    let mut db = storage.connection.lock().unwrap();
    assert!(verify_rows_absent(&db, &scope, &|| false).is_err());
    let tx = db.transaction().unwrap();
    assert!(erase_rows(&tx, &scope, &|| false)
        .unwrap_err()
        .contains("旧版"));
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM meetings WHERE id=?1",
            [meeting],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn history_schema_refuses_fk_corruption_and_any_unknown_temp_object() {
    let db = database(27);
    db.execute_batch("PRAGMA foreign_keys=OFF; INSERT INTO vocabulary_scopes VALUES('meeting:00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000001','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,1); PRAGMA foreign_keys=ON;").unwrap();
    assert!(trusted_history_schema(&db).is_err());
    let db = database(27);
    db.execute_batch("CREATE TEMP TABLE unknown_attention(id TEXT)")
        .unwrap();
    assert!(trusted_history_schema(&db).is_err());
}
