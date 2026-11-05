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
    db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);
        CREATE TABLE meetings(id TEXT PRIMARY KEY,title TEXT NOT NULL);
        INSERT INTO meetings VALUES('unsupported','拒否する旧会議');").unwrap();
    db.execute(
        "INSERT INTO schema_migrations VALUES(?1,'old checksum',0)",
        [version as i64],
    )
    .unwrap();
}

#[test]
fn storage_refuses_old_format_before_wal_recovery_cleanup_and_audio_changes() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("meetings.sqlite3");
    let db = Connection::open(&database).unwrap();
    old(&db, 38);
    drop(db);
    let bytes = std::fs::read(&database).unwrap();
    let audio = temp.path().join("keep.flac");
    std::fs::write(&audio, b"synthetic retained audio").unwrap();
    let error = Storage::open(temp.path().to_owned())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("非対応の会議DB形式"), "{error}");
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
fn unsupported_wal_mode_database_without_sidecars_is_rejected_without_creating_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("旧形式 #100%");
    std::fs::create_dir(&root).unwrap();
    let database = root.join("meetings.sqlite3");
    let db = Connection::open(&database).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    old(&db, 38);
    drop(db);
    let sidecars = [
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        "meetings.sqlite3-journal",
    ];
    for leaf in sidecars {
        assert!(!root.join(leaf).exists(), "{leaf}");
    }
    let bytes = std::fs::read(&database).unwrap();
    assert_eq!(&bytes[18..20], &[2, 2]);
    let error = Storage::open(root.clone()).err().unwrap().to_string();
    assert!(error.contains("記録された形式版: 38"), "{error}");
    assert_eq!(std::fs::read(&database).unwrap(), bytes);
    for leaf in sidecars {
        assert!(!root.join(leaf).exists(), "{leaf}");
    }
}

#[test]
fn unsupported_database_with_committed_wal_is_rejected_without_checkpointing() {
    use rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE;

    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("meetings.sqlite3");
    let wal = root.path().join("meetings.sqlite3-wal");
    let db = Connection::open(&database).unwrap();
    old(&db, 38);
    db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA journal_mode=WAL;")
        .unwrap();
    db.set_db_config(SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .unwrap();
    db.execute(
        "INSERT INTO meetings VALUES('wal-meeting','保持する会議')",
        [],
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(&database).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();
    let error = Storage::open(root.path().to_owned())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("非対応の会議DB形式"), "{error}");
    assert_eq!(std::fs::read(&database).unwrap(), before);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
    assert!(!root.path().join("meetings.sqlite3-shm").exists());
    assert!(!root.path().join(".mon-scratch.sqlite3").exists());
}

#[test]
fn current_initialization_is_atomic_and_repeated_calls_do_not_change_any_rows() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    initialize_database(&mut db).unwrap();
    verify_database_schema(&db).unwrap();
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
fn future_changed_or_missing_current_schema_stamp_is_not_repaired() {
    for defect in ["future", "changed", "missing"] {
        let mut db = Connection::open_in_memory().unwrap();
        initialize_database(&mut db).unwrap();
        match defect {
            "future" => {
                db.execute(
                    "INSERT INTO schema_migrations VALUES(?1,'future',32)",
                    params![SCHEMA_VERSION as i64 + 1],
                )
                .unwrap();
            }
            "changed" => {
                db.execute("UPDATE schema_migrations SET checksum='changed'", [])
                    .unwrap();
            }
            _ => {
                db.execute("DELETE FROM schema_migrations", []).unwrap();
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
    assert!(error.contains("authorized"), "{error}");
    assert!(db.is_autocommit());
    assert!(schema(&db).is_empty());
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    initialize_database(&mut db).unwrap();
    verify_database_schema(&db).unwrap();
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
    verify_database_schema(&db).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "parent-owned synthetic initialization child only"]
fn initialization_sigkill_child() {
    let root =
        PathBuf::from(std::env::var_os("MON_INITIALIZATION_CHILD_ROOT").expect("owned fixture"));
    let mut db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Insert {
                table_name: "schema_migrations"
            }
        ) {
            std::fs::write(root.join("initialization-paused.tmp"), b"ready").unwrap();
            std::fs::rename(
                root.join("initialization-paused.tmp"),
                root.join("initialization-paused"),
            )
            .unwrap();
            loop {
                std::thread::park_timeout(std::time::Duration::from_millis(100));
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
    let temp = tempfile::tempdir().unwrap();
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "storage::initialization_tests::initialization_sigkill_child",
                "--exact",
                "--ignored",
            ])
            .env("MON_INITIALIZATION_CHILD_ROOT", temp.path())
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
    verify_database_schema(&storage.connection.lock().unwrap()).unwrap();
}
