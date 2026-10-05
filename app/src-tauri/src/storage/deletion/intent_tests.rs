use super::{files::Census, intent::*};
use crate::storage::Storage;
use rusqlite::{config::DbConfig, params};
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    file: PathBuf,
    operation: String,
}
impl Fixture {
    fn new() -> Self {
        Self::at(tempfile::tempdir().unwrap())
    }
    fn at(base: tempfile::TempDir) -> Self {
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("private meeting title").unwrap().id;
        let other = storage.create_meeting("KEEP MEETING").unwrap().id;
        let file = asset(
            &storage,
            &meeting,
            "audio/source.flac",
            b"PRIVATE AUDIO PAYLOAD",
        );
        Self {
            base,
            storage,
            meeting,
            other,
            file,
            operation: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn census(&self) -> Census {
        Census::capture(&self.storage, &self.meeting, &|| false).unwrap()
    }
    fn prepare(&self) -> Prepared {
        prepare(&self.storage, &self.operation, &self.census(), &|| false).unwrap()
    }
    fn counts(&self) -> (i64, i64) {
        let db = self.storage.connection.lock().unwrap();
        (
            db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
                r.get(0)
            })
            .unwrap(),
            db.query_row("SELECT count(*) FROM maintenance_paths", [], |r| r.get(0))
                .unwrap(),
        )
    }
    fn untouched(&self) {
        assert_eq!(std::fs::read(&self.file).unwrap(), b"PRIVATE AUDIO PAYLOAD");
        assert_eq!(self.storage.list_meetings().unwrap().len(), 2);
    }
    fn tamper(&self, sql: &str) {
        let saved = read(&self.storage, &self.operation, &|| false).unwrap();
        let db = self.storage.connection.lock().unwrap();
        let _writer = super::fence::write_prepared(&self.storage, &db, &saved, &|| false).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute(sql, params![self.operation]).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
}
fn asset(storage: &Storage, meeting: &str, leaf: &str, bytes: &[u8]) -> PathBuf {
    let path = storage.root().join("meetings").join(meeting).join(leaf);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    storage
        .register_ready_meeting_asset(
            meeting,
            path.strip_prefix(storage.root()).unwrap(),
            "audio/flac",
        )
        .unwrap();
    path
}

#[test]
fn deletion_intent_roundtrips_observations_without_payload_or_other_meeting_targets() {
    let f = Fixture::new();
    let other_file = asset(&f.storage, &f.other, "keep.flac", b"KEEP AUDIO");
    let external = f.base.path().join("external.md");
    std::fs::write(&external, b"KEEP EXTERNAL EXPORT").unwrap();
    let census = f.census();
    let p = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    assert_eq!(p.manifest(), census.manifest());
    assert_eq!(p.scope(), census.scope());
    assert_eq!(p.operation(), f.operation);
    p.verify_live(&f.storage, &|| false).unwrap();
    let loaded = read(&f.storage, &f.operation, &|| false).unwrap();
    assert_eq!(loaded.manifest(), census.manifest());
    assert_eq!(loaded.consent_at(), p.consent_at());
    let db = f.storage.connection.lock().unwrap();
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![f.operation],
            |r| r.get(0),
        )
        .unwrap();
    assert!(json.len() < 65536);
    assert!(!json.contains("PRIVATE AUDIO PAYLOAD"));
    assert!(!json.contains("private meeting title"));
    assert!(!json.contains(&f.other));
    assert!(!json.contains("source.flac"));
    let paths: String = db
        .query_row(
            "SELECT group_concat(relative_path || identity_json) FROM maintenance_paths",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!paths.contains("PRIVATE AUDIO PAYLOAD"));
    assert!(!paths.contains(&f.other));
    assert!(db
        .query_row(
            "SELECT max(length(CAST(identity_json AS BLOB)))<=1024 FROM maintenance_paths",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    drop(db);
    f.untouched();
    assert_eq!(std::fs::read(other_file).unwrap(), b"KEEP AUDIO");
    assert_eq!(std::fs::read(external).unwrap(), b"KEEP EXTERNAL EXPORT");
}

#[test]
fn deletion_intent_uses_separate_paths_for_census_larger_than_small_intent_limit() {
    let f = Fixture::new();
    for index in 0..150 {
        asset(
            &f.storage,
            &f.meeting,
            &format!("many/file-{index:03}.flac"),
            b"small audio",
        );
    }
    let census = f.census();
    assert!(census.encode().unwrap().len() > 65536);
    let p = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    assert_eq!(p.manifest().entries.len(), 154);
    assert_eq!(f.counts(), (1, 154));
    read(&f.storage, &f.operation, &|| false)
        .unwrap()
        .verify_live(&f.storage, &|| false)
        .unwrap();
    f.untouched();
}

#[test]
fn deletion_intent_keeps_missing_changed_unknown_and_producer_hints_distinct() {
    let f = Fixture::new();
    let missing = asset(
        &f.storage,
        &f.meeting,
        "audio/missing.flac",
        b"missing audio",
    );
    std::fs::remove_file(&missing).unwrap();
    std::fs::write(&f.file, b"CHANGED SOURCE").unwrap();
    std::fs::write(f.file.parent().unwrap().join("foreign.bin"), b"UNKNOWN").unwrap();
    let job = uuid::Uuid::new_v4().to_string();
    f.storage.connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES(?1,?2,'minutes_generation','failed',?1,?3,'{}','{}',?3,1,1)",params![job,f.meeting,"a".repeat(64)]).unwrap();
    let dir = f
        .storage
        .root()
        .join("meetings")
        .join(&f.meeting)
        .join("analysis")
        .join(job)
        .join(format!("unit-0-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("prompt.txt"), b"PRIVATE FULL PROMPT").unwrap();
    let census = f.census();
    assert_eq!(
        (
            census.summary().unwrap().missing,
            census.summary().unwrap().changed,
            census.summary().unwrap().unknown,
            census.summary().unwrap().unregistered_with_hint
        ),
        (1, 1, 1, 1)
    );
    let p = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    let loaded = read(&f.storage, &f.operation, &|| false).unwrap();
    assert_eq!(loaded.manifest(), p.manifest());
    assert_eq!(loaded.manifest(), census.manifest());
    loaded.verify_live(&f.storage, &|| false).unwrap();
    assert!(loaded
        .manifest()
        .entries
        .iter()
        .any(|e| e.path.ends_with("foreign.bin")
            && e.stage_hint.is_none()
            && e.registered.is_none()));
    assert!(loaded
        .manifest()
        .entries
        .iter()
        .any(|e| e.path.ends_with("missing.flac") && e.proof.is_none() && e.sha256.is_none()));
    assert_eq!(
        std::fs::read(dir.join("prompt.txt")).unwrap(),
        b"PRIVATE FULL PROMPT"
    );
    assert!(!missing.exists());
}

#[test]
fn deletion_intent_same_operation_retries_keep_timestamp_and_reject_other_census_or_id() {
    let f = Fixture::new();
    let census = f.census();
    let first = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    let again = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    assert_eq!(again.consent_at(), first.consent_at());
    assert_eq!(f.counts(), (1, 3));
    let other = Census::capture(&f.storage, &f.other, &|| false).unwrap();
    assert!(prepare(&f.storage, &f.operation, &other, &|| false).is_err());
    assert!(prepare(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &other,
        &|| false
    )
    .is_err());
    assert!(prepare(&f.storage, "not-an-operation", &census, &|| false).is_err());
    assert!(read(&f.storage, "not-an-operation", &|| false).is_err());
    assert_eq!(f.counts(), (1, 3));
    f.untouched();
}

#[test]
fn deletion_intent_stale_file_or_database_confirmation_writes_no_intent() {
    for database in [false, true] {
        let f = Fixture::new();
        let census = f.census();
        if database {
            let m = f.storage.get_meeting(&f.meeting).unwrap().unwrap();
            f.storage
                .rename_meeting(&f.meeting, "changed meeting", m.lock_version)
                .unwrap();
        } else {
            std::fs::write(f.file.parent().unwrap().join("new.bin"), b"NEW").unwrap();
        }
        assert!(prepare(&f.storage, &f.operation, &census, &|| false).is_err());
        assert_eq!(f.counts(), (0, 0));
        f.untouched();
    }
}

#[test]
fn deletion_intent_cancellation_and_unwind_before_commit_roll_back_every_path() {
    for phase in ["intentStored", "pathStored", "beforeCommit"] {
        let f = Fixture::new();
        let census = f.census();
        let cancelled = Cell::new(false);
        assert!(
            prepare_observed(
                &f.storage,
                &f.operation,
                &census,
                &|| cancelled.get(),
                &|point| {
                    if point == phase {
                        cancelled.set(true)
                    }
                }
            )
            .is_err(),
            "{phase}"
        );
        assert_eq!(f.counts(), (0, 0));
        f.untouched();
        f.prepare().verify_live(&f.storage, &|| false).unwrap();
    }
    // Panic only at the last boundary, after insertion; the transaction must
    // roll back even though the connection mutex becomes poisoned.
    let f = Fixture::new();
    let census = f.census();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepare_observed(&f.storage, &f.operation, &census, &|| false, &|point| {
            if point == "beforeCommit" {
                panic!("injected unwind")
            }
        })
    }))
    .is_err());
    let db = f.storage.connection.lock().unwrap_err().into_inner();
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM maintenance_paths", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE AUDIO PAYLOAD");
}

#[test]
fn deletion_intent_file_change_during_path_persistence_rolls_back_not_adopts() {
    let f = Fixture::new();
    let census = f.census();
    let changed = Cell::new(false);
    assert!(
        prepare_observed(&f.storage, &f.operation, &census, &|| false, &|phase| {
            if phase == "pathStored" && !changed.replace(true) {
                std::fs::write(f.file.parent().unwrap().join("late.bin"), b"LATE UNKNOWN").unwrap();
            }
        })
        .is_err()
    );
    assert_eq!(f.counts(), (0, 0));
    f.untouched();
    assert_eq!(
        std::fs::read(f.file.parent().unwrap().join("late.bin")).unwrap(),
        b"LATE UNKNOWN"
    );
}

#[test]
fn deletion_intent_cold_read_preserves_original_census_and_requires_live_reverification() {
    let f = Fixture::new();
    let census = f.census();
    let p = f.prepare();
    let time = p.consent_at();
    let root = f.storage.root().to_owned();
    let operation = f.operation.clone();
    let file = f.file.clone();
    drop(f.storage);
    let storage = Storage::open(root).unwrap();
    let loaded = read(&storage, &operation, &|| false).unwrap();
    assert_eq!(loaded.consent_at(), time);
    assert_eq!(loaded.manifest(), census.manifest());
    loaded.verify_live(&storage, &|| false).unwrap();
    // A cold read is useful for review even when the live file changed. It
    // must not silently replace the originally consented SHA or identity.
    std::fs::write(&file, b"CHANGED AFTER REOPEN").unwrap();
    let stale = read(&storage, &operation, &|| false).unwrap();
    assert_eq!(stale.manifest(), census.manifest());
    assert!(stale.verify_live(&storage, &|| false).is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"CHANGED AFTER REOPEN");
}

#[test]
fn deletion_intent_saved_rows_reject_missing_extra_hash_content_identity_and_path_tampering() {
    for sql in [
        "INSERT INTO maintenance_paths SELECT operation_id,relative_path||'-extra',kind,size_bytes,content_hash,identity_json,identity_hash FROM maintenance_paths WHERE operation_id=?1 AND kind='file'",
        "DELETE FROM maintenance_paths WHERE operation_id=?1 AND kind='file'",
        "UPDATE maintenance_paths SET relative_path=relative_path||'-changed' WHERE operation_id=?1 AND kind='file'",
        "UPDATE maintenance_paths SET size_bytes=size_bytes+1 WHERE operation_id=?1 AND kind='file'",
        "UPDATE maintenance_paths SET content_hash=printf('%064d',0) WHERE operation_id=?1 AND kind='file'",
        "UPDATE maintenance_paths SET identity_hash=printf('%064d',0) WHERE operation_id=?1",
        "UPDATE maintenance_paths SET identity_json='{}' WHERE operation_id=?1",
        "UPDATE maintenance_paths SET identity_json=NULL,identity_hash=NULL WHERE operation_id=?1",
        "UPDATE maintenance_operations SET target_hash=printf('%064d',0) WHERE id=?1",
        "UPDATE maintenance_operations SET target_manifest_json='{}' WHERE id=?1",
        "UPDATE maintenance_operations SET consent_at=NULL WHERE id=?1",
        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
        "UPDATE maintenance_operations SET state='cleanup' WHERE id=?1",
        "UPDATE maintenance_operations SET created_at=created_at+1 WHERE id=?1",
    ] {
        let f=Fixture::new(); let p=f.prepare(); f.tamper(sql);
        assert!(read(&f.storage,&f.operation,&||false).is_err(),"{sql}");
        assert!(p.verify_live(&f.storage,&||false).is_err(),"{sql}");
        f.untouched();
    }
}

#[test]
fn deletion_intent_existing_sql_guards_keep_confirmation_and_paths_immutable() {
    let f = Fixture::new();
    f.prepare();
    let db = f.storage.connection.lock().unwrap();
    for sql in [
        "UPDATE maintenance_operations SET consent_at=consent_at+1 WHERE id=?1",
        "UPDATE maintenance_operations SET target_hash=printf('%064d',0) WHERE id=?1",
        "DELETE FROM maintenance_operations WHERE id=?1",
        "UPDATE maintenance_paths SET content_hash=printf('%064d',0) WHERE operation_id=?1 AND kind='file'",
        "UPDATE maintenance_paths SET identity_json='{}',identity_hash=printf('%064d',0) WHERE operation_id=?1",
        "DELETE FROM maintenance_paths WHERE operation_id=?1",
    ] { assert!(db.execute(sql,params![f.operation]).is_err(),"{sql}"); }
    drop(db);
    read(&f.storage, &f.operation, &|| false)
        .unwrap()
        .verify_live(&f.storage, &|| false)
        .unwrap();
    f.untouched();
}

#[test]
fn deletion_intent_root_replacement_or_active_work_never_persists_a_new_confirmation() {
    for changed_root in [false, true] {
        let f = Fixture::new();
        let census = f.census();
        if changed_root {
            let root = f.storage.root().to_owned();
            std::fs::rename(&root, f.base.path().join("original")).unwrap();
            std::fs::create_dir(&root).unwrap();
        } else {
            f.storage.connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('active-other',?1,'transcription','running','active-other',?2,'{}','{}',?2,1,1)",params![f.other,"a".repeat(64)]).unwrap();
        }
        assert!(prepare(&f.storage, &f.operation, &census, &|| false).is_err());
        assert_eq!(f.counts(), (0, 0));
    }
}

#[test]
fn deletion_intent_draft_without_files_keeps_zero_path_manifest_durable() {
    let f = Fixture::new();
    let census = Census::capture(&f.storage, &f.other, &|| false).unwrap();
    assert!(census.manifest().entries.is_empty());
    let p = prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    assert!(p.manifest().entries.is_empty());
    assert_eq!(f.counts(), (1, 0));
    read(&f.storage, &f.operation, &|| false)
        .unwrap()
        .verify_live(&f.storage, &|| false)
        .unwrap();
    f.untouched();
}

#[test]
fn deletion_intent_read_rejects_unknown_schema_disabled_guards_and_other_pending_operations() {
    let f = Fixture::new();
    let p = f.prepare();
    {
        let db = f.storage.connection.lock().unwrap();
        let _writer = super::fence::write_prepared(&f.storage, &db, &p, &|| false).unwrap();
        db.execute_batch("CREATE TEMP TABLE untrusted(id TEXT)")
            .unwrap();
        assert!(p.verify_saved(&db, &|| false).is_err());
        db.execute_batch("DROP TABLE temp.untrusted").unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        assert!(p.verify_saved(&db, &|| false).is_err());
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    p.verify_live(&f.storage, &|| false).unwrap();
    {
        let db = f.storage.connection.lock().unwrap();
        let _writer = super::fence::write_prepared(&f.storage, &db, &p, &|| false).unwrap();
        db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES('other-pending',?1,'delete_assets','attention','{}',?2,NULL,'{}',1,1)", params![f.other, "a".repeat(64)]).unwrap();
    }
    // Reading an unchanged receipt remains possible, but it cannot be admitted
    // while a different meeting's maintenance operation remains unresolved.
    read(&f.storage, &f.operation, &|| false).unwrap();
    assert!(p.verify_live(&f.storage, &|| false).is_err());
    f.untouched();
}

#[test]
fn deletion_intent_read_validates_manifest_fields_even_when_outer_hash_is_recomputed() {
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("pathCount", serde_json::json!(200001)),
        ("pathsHash", serde_json::json!("f".repeat(64))),
        ("fileManifestHash", serde_json::json!("F".repeat(64))),
        ("unknownField", serde_json::json!(true)),
    ] {
        let f = Fixture::new();
        let p = f.prepare();
        let db = f.storage.connection.lock().unwrap();
        let writer = super::fence::write_prepared(&f.storage, &db, &p, &|| false).unwrap();
        let json: String = db
            .query_row(
                "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                params![f.operation],
                |r| r.get(0),
            )
            .unwrap();
        let mut value_json: serde_json::Value = serde_json::from_str(&json).unwrap();
        value_json[field] = value;
        let altered = serde_json::to_string(&value_json).unwrap();
        use sha2::{Digest, Sha256};
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",
            params![
                f.operation,
                altered,
                format!("{:x}", Sha256::digest(altered.as_bytes()))
            ],
        )
        .unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        drop(writer);
        drop(db);
        assert!(
            read(&f.storage, &f.operation, &|| false).is_err(),
            "{field}"
        );
        f.untouched();
    }
}

#[test]
#[ignore = "subprocess fixture; invoked by the intent SIGKILL test"]
fn deletion_intent_checkpoint_child_fixture() {
    let base = PathBuf::from(std::env::var("MON_DELETE_INTENT_TEST_BASE").unwrap());
    let phase = std::env::var("MON_DELETE_INTENT_TEST_PHASE").unwrap();
    let f = Fixture::at(tempfile::tempdir_in(&base).unwrap());
    let census = f.census();
    prepare_observed(&f.storage, &f.operation, &census, &|| false, &|point| {
        if point == phase {
            let receipt = serde_json::to_vec(&(
                f.storage.root(),
                &f.operation,
                &f.meeting,
                &f.other,
                census.manifest(),
            ))
            .unwrap();
            std::fs::write(base.join("ready.tmp"), receipt).unwrap();
            std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    })
    .unwrap();
    panic!("checkpoint not reached");
}

#[test]
fn deletion_intent_sigkill_during_paths_before_commit_and_after_commit_is_atomic_and_readable() {
    use std::os::unix::process::ExitStatusExt;
    for phase in ["pathStored", "beforeCommit", "committed"] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::intent_tests::deletion_intent_checkpoint_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_DELETE_INTENT_TEST_BASE", base.path())
            .env("MON_DELETE_INTENT_TEST_PHASE", phase)
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
                panic!("intent checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (root, operation, meeting, other, manifest): (
            PathBuf,
            String,
            String,
            String,
            super::files::Manifest,
        ) = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        assert!(root
            .canonicalize()
            .unwrap()
            .starts_with(base.path().canonicalize().unwrap()));
        let storage = Storage::open(root).unwrap();
        let census = Census::capture_locked(
            &storage,
            &storage.connection.lock().unwrap(),
            &meeting,
            &operation,
            &|| false,
        )
        .unwrap();
        assert_eq!(census.manifest(), &manifest);
        if phase == "committed" {
            read(&storage, &operation, &|| false)
                .unwrap()
                .verify_live(&storage, &|| false)
                .unwrap();
        } else {
            assert!(read(&storage, &operation, &|| false).is_err());
            let db = storage.connection.lock().unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM maintenance_paths", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            drop(db);
            prepare(&storage, &operation, &census, &|| false)
                .unwrap()
                .verify_live(&storage, &|| false)
                .unwrap();
        }
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert_eq!(
            std::fs::read(
                storage
                    .root()
                    .join("meetings")
                    .join(meeting)
                    .join("audio/source.flac")
            )
            .unwrap(),
            b"PRIVATE AUDIO PAYLOAD"
        );
    }
}
