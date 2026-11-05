use super::{
    fence::*,
    files::Census,
    intent::{self, Prepared},
};
use crate::storage::Storage;
use rusqlite::{config::DbConfig, params, Connection};
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    operation: String,
    file: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::at(tempfile::tempdir().unwrap())
    }
    fn at(base: tempfile::TempDir) -> Self {
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage
            .create_meeting("KEEP TARGET UNTIL ACTUAL DELETE")
            .unwrap()
            .id;
        let other = storage.create_meeting("KEEP OTHER MEETING").unwrap().id;
        let file = storage
            .root()
            .join("meetings")
            .join(&meeting)
            .join("audio/source.flac");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"PRIVATE ORIGINAL AUDIO").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                file.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        Self {
            base,
            storage,
            meeting,
            other,
            file,
            operation: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn prepare(&self) -> Prepared {
        intent::prepare(
            &self.storage,
            &self.operation,
            &Census::capture(
                &self.storage,
                &self.meeting,
                &crate::test_support::never_cancelled,
            )
            .unwrap(),
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn state(&self) -> String {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM maintenance_operations WHERE id=?1",
                params![self.operation],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn untouched(&self) {
        assert_eq!(
            std::fs::read(&self.file).unwrap(),
            b"PRIVATE ORIGINAL AUDIO"
        );
        assert_eq!(self.storage.list_meetings().unwrap().len(), 2);
    }
}

#[test]
fn deletion_fence_keeps_theme_reads_but_never_accepts_presentation_writes() {
    use crate::storage::ui_preferences::{Theme, UiPreferences};
    let f = Fixture::new();
    let p = f.prepare();
    assert!(f.storage.data_write_blocked());
    assert_eq!(
        f.storage.ui_preferences().unwrap().payload,
        UiPreferences::default()
    );
    assert!(f
        .storage
        .save_ui_preferences(
            0,
            UiPreferences::V1 {
                schema_version: 1,
                theme: Theme::Dark,
                color_theme: Default::default(),
            }
        )
        .is_err());
    f.untouched();
    drop(p);
    drop(f.storage);
    let storage = Storage::open(f.base.path().join("root")).unwrap();
    assert!(storage.data_write_blocked());
    assert_eq!(storage.ui_preferences().unwrap().lock_version, 0);
    assert!(storage
        .save_ui_preferences(
            0,
            UiPreferences::V1 {
                schema_version: 1,
                theme: Theme::Light,
                color_theme: Default::default(),
            }
        )
        .is_err());
    assert!(f.file.exists());
}

#[test]
fn deletion_fence_arms_after_durable_confirmation_and_keeps_read_views_and_portable_cells_unchanged(
) {
    let f = Fixture::new();
    assert!(!f.storage.data_write_blocked());
    let original = f.storage.get_meeting(&f.meeting).unwrap().unwrap();
    let p = f.prepare();
    assert!(f.storage.data_write_blocked());
    assert!(!f.storage.restart_required());
    assert!(f
        .storage
        .require_data_writable()
        .unwrap_err()
        .contains("未完了の会議削除"));
    assert_eq!(
        f.storage.deleting_meeting_ids().unwrap(),
        vec![f.meeting.clone()]
    );
    assert_eq!(
        f.storage.get_meeting(&f.meeting).unwrap().unwrap(),
        original
    );
    p.verify_live(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(f.storage.create_meeting("must not be added").is_err());
    assert!(f
        .storage
        .rename_meeting(&f.other, "must not change other", 0)
        .is_err());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 2);
    f.untouched();
}

#[test]
fn deletion_fence_blocks_all_database_mutations_pragmas_schema_and_new_attachments() {
    let f = Fixture::new();
    f.prepare();
    let db = f.storage.connection.lock().unwrap();
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        assert!(
            db.execute(&format!("DELETE FROM \"{table}\" WHERE 0"), [])
                .is_err(),
            "{table}"
        );
    }
    for sql in [
        "PRAGMA foreign_keys=OFF",
        "PRAGMA writable_schema=ON",
        "PRAGMA journal_mode=DELETE",
        "PRAGMA secure_delete=OFF",
        "VACUUM",
        "CREATE TEMP TABLE should_not_exist(id TEXT)",
        "CREATE TABLE should_not_exist(id TEXT)",
        "DROP TABLE app_settings",
        "ATTACH ':memory:' AS external",
    ] {
        assert!(db.execute_batch(sql).is_err(), "{sql}");
    }
    assert!(db
        .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
        .unwrap());
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn deletion_fence_invalidates_cached_writes_at_confirmation_and_writes_prepared_inside_privileged_scope(
) {
    let f = Fixture::new();
    {
        let db = f.storage.connection.lock().unwrap();
        let mut q = db
            .prepare_cached("UPDATE meetings SET title='should be denied' WHERE id=?1")
            .unwrap();
        q.execute(params![f.other]).unwrap();
    }
    let p = f.prepare();
    let db = f.storage.connection.lock().unwrap();
    assert!(db
        .prepare_cached("UPDATE meetings SET title='should be denied' WHERE id=?1")
        .is_err());
    let writer =
        write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).unwrap();
    let mut cached = db
        .prepare("UPDATE meetings SET title='escaped write' WHERE id=?1")
        .unwrap();
    drop(writer);
    assert!(cached.execute(params![f.other]).is_err());
    assert_eq!(
        db.query_row(
            "SELECT title FROM meetings WHERE id=?1",
            params![f.other],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "should be denied"
    );
}

#[test]
fn deletion_fence_writer_rolls_back_open_transaction_and_reinstates_guards_on_drop() {
    let f = Fixture::new();
    let p = f.prepare();
    let db = f.storage.connection.lock().unwrap();
    let writer =
        write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    db.execute(
        "UPDATE meetings SET title='uncommitted edit' WHERE id=?1",
        params![f.other],
    )
    .unwrap();
    drop(writer);
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row(
            "SELECT title FROM meetings WHERE id=?1",
            params![f.other],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "KEEP OTHER MEETING"
    );
    assert!(db
        .execute(
            "UPDATE meetings SET title='blocked' WHERE id=?1",
            params![f.other]
        )
        .is_err());
    assert!(f.storage.data_write_blocked());
}

#[test]
fn deletion_fence_writer_unwind_does_not_leave_connection_writable_or_commit_partial_changes() {
    let f = Fixture::new();
    let p = f.prepare();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let db = f.storage.connection.lock().unwrap();
        let _writer =
            write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        db.execute(
            "UPDATE meetings SET title='uncommitted panic' WHERE id=?1",
            params![f.other],
        )
        .unwrap();
        panic!("injected deletion writer panic");
    }))
    .is_err());
    let db = f.storage.connection.lock().unwrap_err().into_inner();
    assert!(db.is_autocommit());
    assert!(f.storage.data_write_blocked());
    assert!(db
        .execute_batch("DELETE FROM app_settings WHERE 0")
        .is_err());
    assert_eq!(
        db.query_row(
            "SELECT title FROM meetings WHERE id=?1",
            params![f.other],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "KEEP OTHER MEETING"
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE ORIGINAL AUDIO");
}

#[test]
fn deletion_fence_guard_failure_stays_fail_closed_until_reopen() {
    let f = Fixture::new();
    let p = f.prepare();
    {
        let db = f.storage.connection.lock().unwrap();
        let writer =
            write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        drop(writer);
        assert!(db
            .execute_batch("UPDATE meetings SET title='blocked' WHERE 0")
            .is_err());
        assert!(db
            .query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .is_err());
    }
    assert!(f.storage.data_write_blocked());
    let root = f.storage.root().to_owned();
    drop(f.storage);
    let storage = Storage::open(root).unwrap();
    intent::read(
        &storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .verify_live(&storage, &crate::test_support::never_cancelled)
    .unwrap();
    assert!(storage.data_write_blocked());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE ORIGINAL AUDIO");
}

#[test]
fn deletion_fence_blocks_backup_move_stage_and_startup_recording_import_recovery() {
    let f = Fixture::new();
    f.prepare();
    assert!(f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    assert!(f.storage.backup_stage().is_err());
    assert!(f.storage.require_data_writable().is_err());
    assert!(f.storage.recover_completed_recordings().unwrap().is_empty());
    assert!(f
        .storage
        .recover_audio_imports_cancellable(&crate::test_support::never_cancelled)
        .unwrap()
        .is_empty());
    f.untouched();
}

#[test]
fn deletion_fence_cold_open_skips_mutable_recovery_preserves_active_rows_and_requires_restart_after_keep(
) {
    for (sql,table,state) in [
        ("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('pending-active',?1,'transcription','running','pending-active',?2,'{}','{\"schemaVersion\":4}',?2,1,1)","processing_jobs","running"),
        ("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES('pending-active',?1,'preparing','system','{}','pending-active',1,1)","recording_sessions","preparing"),
        ("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,created_at,updated_at) VALUES('pending-active',?1,'copying',?2,1,1)","audio_imports","copying"),
        ("INSERT INTO export_records(id,meeting_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,format,destination_path,state,created_at,updated_at) VALUES('pending-active',?1,?2,?2,?2,0,'markdown','/tmp/unused.md','writing',1,1)","export_records","writing"),
    ] {
        let f=Fixture::new();
        let source = match table {
            "recording_sessions" => Some("recording"),
            "audio_imports" => Some("audio_import"),
            _ => None,
        };
        if let Some(source) = source {
            f.storage.connection.lock().unwrap().execute("UPDATE meetings SET source_type=?2 WHERE id=?1",params![f.other,source]).unwrap();
        }
        let p=f.prepare();
        {
            let db=f.storage.connection.lock().unwrap(); let _writer=write_prepared(&f.storage,&db,&p,&||false).unwrap();
            if table == "recording_sessions" {
                db.execute(sql,params![f.other]).unwrap();
            } else {
                db.execute(sql,params![f.other,"a".repeat(64)]).unwrap();
            }
        }
        let root=f.storage.root().to_owned(); drop(f.storage);
        let storage=Storage::open(root.clone()).unwrap();
        assert!(storage.data_write_blocked()); assert!(!storage.restart_required()); assert!(storage.scratch.is_none());
        let before:String=storage.connection.lock().unwrap().query_row(&format!("SELECT state FROM {table} WHERE id='pending-active'"),[],|r|r.get(0)).unwrap(); assert_eq!(before,state);
        assert!(intent::read(&storage,&f.operation,&||false).unwrap().verify_live(&storage,&||false).is_err());
        intent::keep_unstarted(&storage,&p,&||false).unwrap();
        assert!(storage.restart_required()); assert!(storage.data_write_blocked()); assert!(storage.require_data_writable().unwrap_err().contains("再起動"));
        assert!(storage.create_meeting("still blocked").is_err());
        assert!(storage.prepare_backup_snapshot(&||false).is_err());
        intent::keep_unstarted(&storage,&p,&||false).unwrap();
        assert!(storage.deleting_meeting_ids().unwrap().is_empty());
        drop(storage);
        let storage=Storage::open(root).unwrap();
        assert!(!storage.data_write_blocked()); assert!(!storage.restart_required()); assert!(storage.scratch.is_some());
        let after:String=storage.connection.lock().unwrap().query_row(&format!("SELECT state FROM {table} WHERE id='pending-active'"),[],|r|r.get(0)).unwrap(); assert_eq!(after,if table == "audio_imports" { "failed" } else { "interrupted" });
        storage.create_meeting("allowed after ordinary recovery").unwrap();
        assert_eq!(std::fs::read(&f.file).unwrap(),b"PRIVATE ORIGINAL AUDIO");
    }
}

#[test]
fn deletion_fence_keep_unstarted_preserves_changed_files_intent_paths_and_original_consent() {
    let f = Fixture::new();
    let p = f.prepare();
    std::fs::write(&f.file, b"CHANGED OUTSIDE THE APPLICATION").unwrap();
    assert!(p
        .verify_live(&f.storage, &crate::test_support::never_cancelled)
        .is_err());
    intent::keep_unstarted(&f.storage, &p, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(f.state(), "recovered");
    assert!(!f.storage.data_write_blocked());
    assert!(!f.storage.restart_required());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT consent_at FROM maintenance_operations WHERE id=?1",
            params![f.operation],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        p.consent_at()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM maintenance_paths WHERE operation_id=?1",
            params![f.operation],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    assert!(db
        .execute(
            "DELETE FROM maintenance_paths WHERE operation_id=?1",
            params![f.operation]
        )
        .is_err());
    drop(db);
    intent::keep_unstarted(&f.storage, &p, &crate::test_support::never_cancelled).unwrap();
    assert!(intent::read(
        &f.storage,
        &f.operation,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(
        std::fs::read(&f.file).unwrap(),
        b"CHANGED OUTSIDE THE APPLICATION"
    );
    f.storage.create_meeting("ordinary writes resumed").unwrap();
}

#[test]
fn deletion_fence_keep_cancel_before_commit_keeps_receipt_pending_and_write_stopped() {
    for phase in ["updated", "beforeCommit"] {
        let f = Fixture::new();
        let p = f.prepare();
        let cancelled = Cell::new(false);
        assert!(
            intent::keep_observed(
                &f.storage,
                &p,
                &|| cancelled.get(),
                &|point| if phase == point {
                    cancelled.set(true)
                }
            )
            .is_err()
        );
        assert_eq!(f.state(), "prepared");
        assert!(f.storage.data_write_blocked());
        p.verify_live(&f.storage, &crate::test_support::never_cancelled)
            .unwrap();
        f.untouched();
        intent::keep_unstarted(&f.storage, &p, &crate::test_support::never_cancelled).unwrap();
        assert!(!f.storage.data_write_blocked());
    }
}

#[test]
fn deletion_fence_keep_cannot_resolve_a_started_or_corrupted_receipt() {
    for sql in [
        "UPDATE maintenance_operations SET state='cleanup' WHERE id=?1",
        "UPDATE maintenance_operations SET progress_json='{\"filePhase\":\"started\"}' WHERE id=?1",
        "UPDATE maintenance_operations SET target_hash=printf('%064d',0) WHERE id=?1",
    ] {
        let f = Fixture::new();
        let p = f.prepare();
        {
            let db = f.storage.connection.lock().unwrap();
            let _writer =
                write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).unwrap();
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            db.execute(sql, params![f.operation]).unwrap();
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
                .unwrap();
        }
        assert!(
            intent::keep_unstarted(&f.storage, &p, &crate::test_support::never_cancelled).is_err()
        );
        assert!(f.storage.data_write_blocked());
        f.untouched();
    }
}

#[test]
fn deletion_fence_writer_cannot_adopt_another_operation_or_changed_root() {
    let f = Fixture::new();
    let p = f.prepare();
    let other = Fixture::new();
    let foreign = other.prepare();
    {
        let db = f.storage.connection.lock().unwrap();
        assert!(write_prepared(
            &f.storage,
            &db,
            &foreign,
            &crate::test_support::never_cancelled
        )
        .is_err());
        let mut copied = Connection::open(f.base.path().join("copy.sqlite3")).unwrap();
        rusqlite::backup::Backup::new(&db, &mut copied)
            .unwrap()
            .run_to_completion(128, std::time::Duration::from_millis(1), None)
            .unwrap();
        copied.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let copied_fence = Fence::open(&copied).unwrap();
        p.verify_saved(&copied, &crate::test_support::never_cancelled)
            .unwrap();
        assert!(write_prepared(
            &f.storage,
            &copied,
            &p,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(copied_fence.pending());
        assert!(copied
            .execute_batch("DELETE FROM meetings WHERE 0")
            .is_err());
        assert!(db.execute_batch("DELETE FROM meetings WHERE 0").is_err());
    }
    let root = f.storage.root().to_owned();
    std::fs::rename(&root, f.base.path().join("original-root")).unwrap();
    std::fs::create_dir(&root).unwrap();
    {
        let db = f.storage.connection.lock().unwrap();
        assert!(
            write_prepared(&f.storage, &db, &p, &crate::test_support::never_cancelled).is_err()
        );
        assert!(db.execute_batch("DELETE FROM meetings WHERE 0").is_err());
    }
    assert!(f.storage.data_write_blocked());
    assert!(intent::keep_unstarted(&f.storage, &p, &crate::test_support::never_cancelled).is_err());
}

#[test]
fn deletion_fence_unknown_pending_manifest_still_blocks_startup_writes_without_adopting_targets() {
    let f = Fixture::new();
    f.storage.connection.lock().unwrap().execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_meeting','attention','{}',?3,1,'{}',1,1)",params![f.operation,f.meeting,"a".repeat(64)]).unwrap();
    let root = f.storage.root().to_owned();
    drop(f.storage);
    let storage = Storage::open(root).unwrap();
    assert!(storage.data_write_blocked());
    assert!(storage.create_meeting("not allowed").is_err());
    assert!(intent::read(
        &storage,
        &f.operation,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(storage.deleting_meeting_ids().unwrap(), vec![f.meeting]);
    assert_eq!(std::fs::read(f.file).unwrap(), b"PRIVATE ORIGINAL AUDIO");
}

#[test]
fn deletion_fence_pending_older_database_is_not_migrated_or_recovered() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let mut db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    crate::storage::initialize_database(&mut db).unwrap();
    db.execute(
        "UPDATE schema_migrations SET version=38,checksum='prerelease'",
        [],
    )
    .unwrap();
    let meeting = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,'KEEP OLD DATA','draft',1,1)",params![meeting]).unwrap();
    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_meeting','prepared','{}',?3,1,'{}',1,1)",params![uuid::Uuid::new_v4().to_string(),meeting,"a".repeat(64)]).unwrap();
    drop(db);
    assert!(Storage::open(root.clone())
        .err()
        .unwrap()
        .to_string()
        .contains("非対応の会議DB形式"));
    let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT version FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        38
    );
    assert_eq!(
        db.query_row("SELECT title FROM meetings", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "KEEP OLD DATA"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
#[ignore = "subprocess fixture; invoked by the fenced keep SIGKILL test"]
fn deletion_fence_keep_checkpoint_child_fixture() {
    let base = PathBuf::from(std::env::var("MON_DELETE_FENCE_TEST_BASE").unwrap());
    let phase = std::env::var("MON_DELETE_FENCE_TEST_PHASE").unwrap();
    let f = Fixture::at(tempfile::tempdir_in(&base).unwrap());
    let p = f.prepare();
    intent::keep_observed(
        &f.storage,
        &p,
        &crate::test_support::never_cancelled,
        &|point| {
            if phase == point {
                let receipt =
                    serde_json::to_vec(&(f.storage.root(), &f.operation, &f.meeting, &f.other))
                        .unwrap();
                std::fs::write(base.join("ready.tmp"), receipt).unwrap();
                std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        },
    )
    .unwrap();
    panic!("keep checkpoint not reached");
}

#[test]
fn deletion_fence_sigkill_before_and_after_keep_commit_preserves_all_data_and_correct_startup_gate()
{
    use std::os::unix::process::ExitStatusExt;
    for phase in ["beforeCommit", "committed"] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::fence_tests::deletion_fence_keep_checkpoint_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_DELETE_FENCE_TEST_BASE", base.path())
            .env("MON_DELETE_FENCE_TEST_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let ready = base.path().join("ready.json");
        let start = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || start.elapsed() > std::time::Duration::from_secs(20)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("fence keep checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (root, operation, meeting, other): (PathBuf, String, String, String) =
            serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        assert!(root
            .canonicalize()
            .unwrap()
            .starts_with(base.path().canonicalize().unwrap()));
        let storage = Storage::open(root).unwrap();
        assert_eq!(storage.data_write_blocked(), phase == "beforeCommit");
        if phase == "beforeCommit" {
            let p =
                intent::read(&storage, &operation, &crate::test_support::never_cancelled).unwrap();
            p.verify_live(&storage, &crate::test_support::never_cancelled)
                .unwrap();
            intent::keep_unstarted(&storage, &p, &crate::test_support::never_cancelled).unwrap();
            assert!(storage.restart_required());
        } else {
            storage
                .create_meeting("allowed after cold kept receipt")
                .unwrap();
        }
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert!(storage.get_meeting(&meeting).unwrap().is_some());
        assert_eq!(
            std::fs::read(
                storage
                    .root()
                    .join("meetings")
                    .join(meeting)
                    .join("audio/source.flac")
            )
            .unwrap(),
            b"PRIVATE ORIGINAL AUDIO"
        );
    }
}
