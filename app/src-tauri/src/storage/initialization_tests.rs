//! Current initialization and unsupported database rejection; synthetic data only.
use super::*;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    types::Value,
};

fn rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut query = db.prepare(sql).unwrap();
    let count = query.column_count();
    query
        .query_map([], |row| {
            (0..count)
                .map(|i| row.get(i))
                .collect::<Result<Vec<Value>, _>>()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn schema(db: &Connection) -> Vec<Vec<Value>> {
    rows(
        db,
        "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name",
    )
}
fn old(db: &Connection, version: usize) {
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);").unwrap();
    for &(n, sql) in &MIGRATIONS[..version] {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,0)",
            params![n, format!("{:x}", Sha256::digest(sql.as_bytes()))],
        )
        .unwrap();
    }
    db.execute("INSERT INTO meetings(id,title,created_at,updated_at) VALUES('unsupported','拒否する旧会議',12,34)", []).unwrap();
}

#[test]
fn all_old_versions_are_rejected_without_updating_schema_history_or_meetings() {
    for version in 1..31 {
        let mut db = Connection::open_in_memory().unwrap();
        old(&db, version);
        let before = schema(&db);
        let history = rows(&db, "SELECT * FROM schema_migrations ORDER BY version");
        let meetings = rows(&db, "SELECT * FROM meetings");
        assert!(initialize_database(&mut db)
            .unwrap_err()
            .to_string()
            .contains("非対応の旧DB形式"));
        assert!(db.is_autocommit());
        assert_eq!(schema(&db), before);
        assert_eq!(
            rows(&db, "SELECT * FROM schema_migrations ORDER BY version"),
            history
        );
        assert_eq!(rows(&db, "SELECT * FROM meetings"), meetings);
    }
}

#[test]
fn storage_refuses_old_format_before_wal_recovery_cleanup_and_audio_changes() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("meetings.sqlite3");
    let db = Connection::open(&database).unwrap();
    old(&db, 20);
    drop(db);
    let bytes = std::fs::read(&database).unwrap();
    let audio = temp.path().join("keep.flac");
    std::fs::write(&audio, b"synthetic retained audio").unwrap();
    let error = Storage::open(temp.path().to_owned())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("非対応の旧DB形式"), "{error}");
    assert_eq!(std::fs::read(&database).unwrap(), bytes);
    assert_eq!(std::fs::read(&audio).unwrap(), b"synthetic retained audio");
    for leaf in [
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        ".mon-scratch.sqlite3",
    ] {
        assert!(!temp.path().join(leaf).exists(), "{leaf}");
    }
}

#[test]
fn current_initialization_is_atomic_and_repeated_calls_do_not_change_any_rows() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    initialize_database(&mut db).unwrap();
    assert_eq!(verify_migration_history(&db).unwrap(), MIGRATIONS.len());
    db.execute(
        "INSERT INTO meetings(id,title,created_at,updated_at) VALUES('current','現行会議',12,34)",
        [],
    )
    .unwrap();
    let before = schema(&db);
    let names = rows(
        &db,
        "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name",
    );
    let capture = |db: &Connection| {
        names
            .iter()
            .map(|row| {
                let Value::Text(name) = &row[0] else {
                    panic!("table name")
                };
                rows(
                    db,
                    &format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")),
                )
            })
            .collect::<Vec<_>>()
    };
    let content = capture(&db);
    initialize_database(&mut db).unwrap();
    initialize_database(&mut db).unwrap();
    assert_eq!(schema(&db), before);
    assert_eq!(capture(&db), content);
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    assert_eq!(
        rows(&db, "PRAGMA integrity_check"),
        vec![vec![Value::Text("ok".into())]]
    );
}

#[test]
fn unknown_objects_and_empty_history_are_never_treated_as_new_databases() {
    for sql in ["CREATE TABLE meetings(unknown TEXT); INSERT INTO meetings VALUES('must stay');",
        "CREATE TABLE sqliteXunknown(body TEXT);",
        "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT,applied_at INTEGER);"] {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(sql).unwrap();
        let before = schema(&db);
        assert!(initialize_database(&mut db).is_err());
        assert!(db.is_autocommit());
        assert_eq!(schema(&db), before);
    }
}

#[test]
fn future_changed_or_gapped_current_history_is_not_repaired() {
    for defect in ["future", "changed", "gap"] {
        let mut db = Connection::open_in_memory().unwrap();
        initialize_database(&mut db).unwrap();
        match defect {
            "future" => {
                db.execute(
                    "INSERT INTO schema_migrations VALUES(?1,'future',32)",
                    params![MIGRATIONS.last().unwrap().0 + 1],
                )
                .unwrap();
            }
            "changed" => {
                db.execute(
                    "UPDATE schema_migrations SET checksum='changed' WHERE version=10",
                    [],
                )
                .unwrap();
            }
            _ => {
                db.execute("DELETE FROM schema_migrations WHERE version=10", [])
                    .unwrap();
            }
        }
        let before = schema(&db);
        let history = rows(&db, "SELECT * FROM schema_migrations ORDER BY version");
        assert!(initialize_database(&mut db).is_err());
        assert_eq!(schema(&db), before);
        assert_eq!(
            rows(&db, "SELECT * FROM schema_migrations ORDER BY version"),
            history
        );
    }
}

#[test]
fn late_initialization_sql_failure_rolls_back_all_tables_and_history_then_retry_succeeds() {
    let mut db = Connection::open_in_memory().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::CreateTable {
            table_name: "minutes_markdown",
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }))
    .unwrap();
    let error = initialize_database(&mut db).unwrap_err().to_string();
    assert!(error.contains("段階31"), "{error}");
    assert!(db.is_autocommit());
    assert!(schema(&db).is_empty());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    initialize_database(&mut db).unwrap();
    assert_eq!(verify_migration_history(&db).unwrap(), MIGRATIONS.len());
}

#[test]
fn rejected_initialization_commit_leaves_empty_database_and_explicit_retry_succeeds() {
    let mut db = Connection::open_in_memory().unwrap();
    db.commit_hook(Some(|| true)).unwrap();
    assert!(initialize_database(&mut db).is_err());
    assert!(db.is_autocommit());
    assert!(schema(&db).is_empty());
    db.commit_hook(None::<fn() -> bool>).unwrap();
    initialize_database(&mut db).unwrap();
    assert_eq!(verify_migration_history(&db).unwrap(), MIGRATIONS.len());
}

#[cfg(unix)]
#[test]
#[ignore = "parent-owned synthetic initialization child only"]
fn initialization_sigkill_child() {
    let root =
        PathBuf::from(std::env::var_os("MON_INITIALIZATION_CHILD_ROOT").expect("owned fixture"));
    let target: usize = std::env::var("MON_INITIALIZATION_CHILD_STAGE")
        .unwrap()
        .parse()
        .unwrap();
    let mut db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    let mut inserted = 0;
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Insert {
                table_name: "schema_migrations"
            }
        ) {
            inserted += 1;
            if inserted == target {
                std::fs::write(root.join("initialization-paused.tmp"), target.to_string()).unwrap();
                std::fs::rename(
                    root.join("initialization-paused.tmp"),
                    root.join("initialization-paused"),
                )
                .unwrap();
                loop {
                    std::thread::park_timeout(std::time::Duration::from_millis(100));
                }
            }
        }
        Authorization::Allow
    }))
    .unwrap();
    initialize_database(&mut db).unwrap();
    panic!("owned initialization child did not pause");
}

#[cfg(unix)]
#[test]
fn actual_sigkill_during_new_initialization_never_leaves_partial_schema_and_cold_retry_succeeds() {
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for stage in [1, 17, 31] {
        let temp = tempfile::tempdir().unwrap();
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "storage::initialization_tests::initialization_sigkill_child",
                    "--exact",
                    "--ignored",
                ])
                .env("MON_INITIALIZATION_CHILD_ROOT", temp.path())
                .env("MON_INITIALIZATION_CHILD_STAGE", stage.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        while !temp.path().join("initialization-paused").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "owned child ended early"
            );
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "owned child readiness timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(temp.path().join("initialization-paused")).unwrap(),
            stage.to_string()
        );
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        let db = Connection::open(temp.path().join("meetings.sqlite3")).unwrap();
        assert!(schema(&db).is_empty());
        drop(db);
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("現行形式の初期化再試行").unwrap();
        drop(storage);
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(storage.get_meeting(&meeting.id).unwrap(), Some(meeting));
        assert_eq!(
            verify_migration_history(&storage.connection.lock().unwrap()).unwrap(),
            MIGRATIONS.len()
        );
    }
}

#[test]
fn voice_upgrade_preserves_previous_current_data_and_restores_foreign_keys() {
    let mut db = Connection::open_in_memory().unwrap();
    old(&db, 31);
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('job','unsupported','transcription','queued','operation',?1,'{}','{}',?2,1,1)",params!["a".repeat(64),"b".repeat(64)]).unwrap();
    db.execute(
        "INSERT INTO job_checkpoints VALUES('checkpoint','job',1,0,'{}',?1,1)",
        ["c".repeat(64)],
    )
    .unwrap();
    let jobs = rows(&db, "SELECT * FROM processing_jobs");
    let checkpoints = rows(&db, "SELECT * FROM job_checkpoints");
    let before = rows(&db, "SELECT * FROM meetings ORDER BY id");
    initialize_database(&mut db).unwrap();
    assert_eq!(rows(&db, "SELECT * FROM meetings ORDER BY id"), before);
    assert_eq!(rows(&db, "SELECT * FROM processing_jobs"), jobs);
    assert_eq!(rows(&db, "SELECT * FROM job_checkpoints"), checkpoints);
    assert_eq!(verify_migration_history(&db).unwrap(), 32);
    assert_eq!(
        db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    let trusted = Connection::open_in_memory().unwrap();
    let mut fresh = trusted;
    initialize_database(&mut fresh).unwrap();
    let without_history = |db: &Connection| {
        schema(db)
            .into_iter()
            .filter(|row| row[1] != Value::Text("schema_migrations".into()))
            .collect::<Vec<_>>()
    };
    assert_eq!(without_history(&db), without_history(&fresh));
}

#[test]
fn failed_voice_upgrade_rolls_back_meetings_jobs_schema_and_history() {
    let mut db = Connection::open_in_memory().unwrap();
    old(&db, 31);
    let before = schema(&db);
    let meetings = rows(&db, "SELECT * FROM meetings");
    let history = rows(&db, "SELECT * FROM schema_migrations");
    db.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::CreateIndex {
            index_name: "processing_jobs_state_created",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }))
    .unwrap();
    assert!(initialize_database(&mut db).is_err());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_eq!(schema(&db), before);
    assert_eq!(rows(&db, "SELECT * FROM meetings"), meetings);
    assert_eq!(rows(&db, "SELECT * FROM schema_migrations"), history);
    assert_eq!(
        db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    initialize_database(&mut db).unwrap();
}

#[test]
fn voice_upgrade_refuses_pending_maintenance_without_changing_its_target() {
    let mut db = Connection::open_in_memory().unwrap();
    old(&db, 31);
    db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES('pending','move_storage','prepared','{}',?1,'{}',0,0)",["a".repeat(64)]).unwrap();
    let before = schema(&db);
    let operation = rows(&db, "SELECT * FROM maintenance_operations");
    assert!(initialize_database(&mut db)
        .unwrap_err()
        .to_string()
        .contains("未完了"));
    assert_eq!(schema(&db), before);
    assert_eq!(rows(&db, "SELECT * FROM maintenance_operations"), operation);
    assert_eq!(verify_migration_history(&db).unwrap(), 31);
}
