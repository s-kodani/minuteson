//! Actual historical SQL and SQLite failures; no installed app or user DB.
use super::*;
use rusqlite::types::Value;

const MEETING: &str = "00000000-0000-4000-8000-000000000001";
const AUDIO: &[u8] = b"synthetic retained audio marker, not playable FLAC";

#[cfg(unix)]
struct OwnedChild(std::process::Child);
#[cfg(unix)]
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Even a parent assertion/observation failure must not orphan this
        // test's parked writer. Never look up or terminate another process.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn legacy(root: &Path, version: usize) -> Connection {
    std::fs::create_dir_all(root).unwrap();
    let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
    for &(number, sql) in &MIGRATIONS[..version] {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
            params![
                number,
                format!("{:x}", Sha256::digest(sql.as_bytes())),
                number * 100
            ],
        )
        .unwrap();
    }
    db.execute("INSERT INTO meetings(id,title,created_at,updated_at,next_analysis_no,next_minutes_no,lock_version) VALUES(?1,'更新前の会議',123,456,3,3,7)", [MEETING]).unwrap();
    for n in 1..=2 {
        let id = format!("analysis-{n}");
        let hash = format!("{n:064}");
        db.execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES(?1,?2,?3,'manual_snapshot','committed',?4,'{}','{}',?4,1,10)",params![id,MEETING,n,hash]).unwrap();
        let document = r#"{"schemaVersion":1,"blocks":[{"type":"paragraph","itemId":null,"content":[{"type":"text","text":"保持する手動追記","bold":false,"italic":false}]}]}"#;
        db.execute("INSERT INTO minutes_revisions(id,meeting_id,input_analysis_id,revision_no,publication_state,generated_document_json,effective_document_json,document_schema_version,input_content_hash,content_hash,model_snapshot_json,template_version,created_at) VALUES(?1,?2,?3,?4,'committed',?5,?5,1,?6,?6,'{}','standard-v1',20)", params![format!("minutes-{n}"),MEETING,id,n,document,hash]).unwrap();
    }
    db.execute("UPDATE meetings SET current_analysis_id='analysis-2',previous_analysis_id='analysis-1',current_minutes_id='minutes-2',previous_minutes_id='minutes-1' WHERE id=?1",[MEETING]).unwrap();
    let payload = serde_json::to_string(&TranscriptDraftPayload::default()).unwrap();
    let hash = format!("{:x}", Sha256::digest(payload.as_bytes()));
    db.execute("INSERT INTO working_drafts(id,meeting_id,target_kind,base_analysis_id,payload_json,schema_version,content_hash,lock_version,created_at,updated_at) VALUES('draft-retained',?1,'transcript','analysis-2',?2,1,?3,4,23,45)",params![MEETING,payload,hash]).unwrap();
    db.execute("INSERT INTO edit_operations(id,draft_id,sequence,operation_kind,payload_json,inverse_json,before_hash,after_hash,created_at) VALUES('edit-retained','draft-retained',1,'fixture_noop','{}','{}',?1,?1,44)",[&hash]).unwrap();
    if version >= 11 {
        db.execute("INSERT INTO app_settings VALUES('retained-update-setting','{\"value\":\"保持する設定\"}',?1,3,99)",[&hash]).unwrap();
    }
    let relative = format!("meetings/{MEETING}/audio/retained.flac");
    let audio = root.join(&relative);
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(audio, AUDIO).unwrap();
    db.execute("INSERT INTO file_assets(id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) VALUES('asset-retained',?1,'meeting',?2,'audio/flac',?3,'sha256',?4,'ready',21,22)",params![MEETING,relative,AUDIO.len() as i64,format!("{:x}",Sha256::digest(AUDIO))]).unwrap();
    db
}

fn rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = db.prepare(sql).unwrap();
    let count = statement.column_count();
    statement
        .query_map([], |row| {
            (0..count)
                .map(|column| row.get(column))
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
fn retained(db: &Connection, version: usize) -> Vec<Vec<Vec<Value>>> {
    let mut result = vec![
        rows(db,"SELECT id,title,created_at,updated_at,next_analysis_no,next_minutes_no,lock_version,current_analysis_id,previous_analysis_id,current_minutes_id,previous_minutes_id FROM meetings ORDER BY id"),
        rows(db,"SELECT * FROM analysis_revisions ORDER BY id"),
        rows(db,"SELECT * FROM minutes_revisions ORDER BY id"),
        rows(db,"SELECT id,meeting_id,target_kind,base_analysis_id,base_minutes_id,payload_json,schema_version,content_hash,lock_version,created_at,updated_at FROM working_drafts ORDER BY id"),
        rows(db,"SELECT * FROM edit_operations ORDER BY id"),
        rows(db,"SELECT * FROM file_assets ORDER BY id"),
        rows(db,&format!("SELECT * FROM schema_migrations WHERE version <= {version} ORDER BY version")),
    ];
    if version >= 11 {
        result.push(rows(db, "SELECT * FROM app_settings ORDER BY key"));
    }
    result
}
fn assert_updated(root: &Path, before: &[Vec<Vec<Value>>], version: usize) {
    for _ in 0..2 {
        let storage = Storage::open(root.to_owned()).unwrap();
        let db = storage.connection.lock().unwrap();
        assert_eq!(retained(&db, version), before);
        assert_eq!(verify_migration_history(&db).unwrap(), MIGRATIONS.len());
        assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
        assert_eq!(
            rows(&db, "PRAGMA integrity_check"),
            vec![vec![Value::Text("ok".into())]]
        );
        assert_eq!(
            rows(
                &db,
                "SELECT active FROM working_drafts WHERE id='draft-retained'"
            ),
            vec![vec![Value::Integer(1)]]
        );
        drop(db);
        assert!(storage.verify_asset("asset-retained").unwrap());
        assert_eq!(
            std::fs::read(root.join(format!("meetings/{MEETING}/audio/retained.flac"))).unwrap(),
            AUDIO
        );
    }
}

#[test]
fn historical_updates_keep_heads_documents_drafts_edits_settings_and_audio_after_two_reopens() {
    for version in [9, 11, 20, 27, 29] {
        let temp = tempfile::tempdir().unwrap();
        let db = legacy(temp.path(), version);
        let before = retained(&db, version);
        drop(db);
        assert_updated(temp.path(), &before, version);
    }
}

#[test]
fn late_sql_failure_rolls_back_every_new_version_then_explicit_retry_keeps_old_data() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = legacy(temp.path(), 20);
    db.execute_batch(
        "ALTER TABLE recording_sessions ADD COLUMN input_delivery_transfer_json TEXT;",
    )
    .unwrap();
    let before_schema = schema(&db);
    let before = retained(&db, 20);
    let error = apply_migrations(&mut db).unwrap_err().to_string();
    assert!(error.contains("段階30"), "{error}");
    assert!(db.is_autocommit());
    assert_eq!(schema(&db), before_schema);
    assert_eq!(retained(&db, 20), before);
    assert_eq!(verify_migration_history(&db).unwrap(), 20);
    // Only this synthetic test's conflicting object is removed, not a repair
    // facility exposed to the product or a blanket removal of unknown data.
    db.execute_batch("ALTER TABLE recording_sessions DROP COLUMN input_delivery_transfer_json;")
        .unwrap();
    drop(db);
    assert_updated(temp.path(), &before, 20);
}

#[test]
fn first_initialization_failure_does_not_leave_a_new_history_table() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE meetings(unknown TEXT); INSERT INTO meetings VALUES('must stay');",
    )
    .unwrap();
    let before = schema(&db);
    assert!(apply_migrations(&mut db).is_err());
    assert!(db.is_autocommit());
    assert_eq!(schema(&db), before);
    assert_eq!(
        rows(&db, "SELECT * FROM meetings"),
        vec![vec![Value::Text("must stay".into())]]
    );
}

#[test]
fn future_changed_or_gapped_history_never_adds_known_new_versions() {
    for defect in ["future", "changed", "gap"] {
        let temp = tempfile::tempdir().unwrap();
        let mut db = legacy(temp.path(), 20);
        match defect {
            "future" => {
                db.execute("INSERT INTO schema_migrations VALUES(31,'future',31)", [])
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
        let before = retained(&db, 40);
        let before_schema = schema(&db);
        assert!(apply_migrations(&mut db).is_err());
        assert!(db.is_autocommit());
        assert_eq!(retained(&db, 40), before);
        assert_eq!(schema(&db), before_schema);
    }
}

#[test]
fn rejected_commit_keeps_the_old_schema_and_history_without_claiming_update_success() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = legacy(temp.path(), 20);
    let before = retained(&db, 20);
    let before_schema = schema(&db);
    db.commit_hook(Some(|| true)).unwrap();
    assert!(apply_migrations(&mut db).is_err());
    assert!(db.is_autocommit());
    assert_eq!(schema(&db), before_schema);
    assert_eq!(retained(&db, 20), before);
    db.commit_hook(None::<fn() -> bool>).unwrap();
    drop(db);
    assert_updated(temp.path(), &before, 20);
}

#[cfg(unix)]
#[test]
#[ignore = "owned child only: parent kills its synthetic uncommitted migration"]
fn migration_update_owned_sigkill_child() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let Ok(root) = std::env::var("MON_MIGRATION_UPDATE_CHILD_ROOT") else {
        return;
    };
    let target: usize = std::env::var("MON_MIGRATION_UPDATE_CHILD_STAGE")
        .unwrap()
        .parse()
        .unwrap();
    let root = PathBuf::from(root);
    let mut db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    let mut inserted = 20;
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Insert {
                table_name: "schema_migrations"
            }
        ) {
            inserted += 1;
            if inserted == target {
                // Publish readiness only after the complete marker is closed;
                // exists() must never expose a created-but-empty signal.
                std::fs::write(root.join("migration-paused.tmp"), target.to_string()).unwrap();
                std::fs::rename(
                    root.join("migration-paused.tmp"),
                    root.join("migration-paused"),
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
    apply_migrations(&mut db).unwrap();
    panic!("owned migration child did not stop at the requested stage");
}

#[cfg(unix)]
#[test]
fn real_sigkill_at_three_uncommitted_stages_keeps_old_schema_then_cold_update_succeeds() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    for stage in [21, 27, 30] {
        let temp = tempfile::tempdir().unwrap();
        let db = legacy(temp.path(), 20);
        let before = retained(&db, 20);
        let before_schema = schema(&db);
        drop(db);
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "storage::migration_update_tests::migration_update_owned_sigkill_child",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MON_MIGRATION_UPDATE_CHILD_ROOT", temp.path())
                .env("MON_MIGRATION_UPDATE_CHILD_STAGE", stage.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        while !temp.path().join("migration-paused").exists() {
            if let Some(status) = child.0.try_wait().unwrap() {
                panic!("owned child ended before stage {stage}: {status}");
            }
            if started.elapsed() > Duration::from_secs(30) {
                child.0.kill().unwrap();
                child.0.wait().unwrap();
                panic!("owned child did not reach stage {stage}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(temp.path().join("migration-paused")).unwrap(),
            stage.to_string()
        );
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        let db = Connection::open(temp.path().join("meetings.sqlite3")).unwrap();
        assert_eq!(schema(&db), before_schema);
        assert_eq!(retained(&db, 20), before);
        assert_eq!(verify_migration_history(&db).unwrap(), 20);
        drop(db);
        assert_updated(temp.path(), &before, 20);
    }
}
