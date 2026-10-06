use super::database::*;
use crate::storage::{minutes_ui::tests as minutes_fixture, Storage};
use rusqlite::{config::DbConfig, params, Connection};
use std::cell::Cell;

pub(super) struct Fixture {
    pub(super) _root: tempfile::TempDir,
    pub(super) storage: Storage,
    pub(super) meeting: String,
    pub(super) other: String,
}
impl Fixture {
    pub(super) fn new() -> Self {
        Self::at(tempfile::tempdir().unwrap())
    }
    pub(super) fn at(root: tempfile::TempDir) -> Self {
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = minutes_fixture::meeting(&storage);
        for i in 0..3 {
            let candidate = minutes_fixture::candidate(&storage, &meeting);
            if i < 2 {
                storage
                    .adopt_minutes(minutes_fixture::request(&storage, &meeting, &candidate))
                    .unwrap();
                let draft = storage.create_minutes_draft(&meeting, &candidate).unwrap();
                let mut payload = draft.payload.clone();
                payload
                    .blocks
                    .push(crate::minutes_document::Block::Paragraph {
                        item_id: None,
                        content: vec![crate::minutes_document::Inline::text(
                            "削除対象の手動編集全文",
                        )],
                    });
                storage
                    .save_minutes_draft(&draft.id, draft.lock_version, payload)
                    .unwrap();
            }
        }
        let other = storage.create_meeting("対象外会議を保持").unwrap().id;
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "INSERT INTO app_settings VALUES('delete-test','{\"keep\":true}',?1,1,1)",
                params!["a".repeat(64)],
            )
            .unwrap();
            db.execute(
                "INSERT INTO vocabulary_scopes VALUES('global',NULL,?1,0,1)",
                params!["a".repeat(64)],
            )
            .unwrap();
            db.execute(
                "INSERT INTO vocabulary_scopes VALUES(?1,?2,?3,0,1)",
                params![format!("meeting:{meeting}"), meeting, "a".repeat(64)],
            )
            .unwrap();
            for (id, scope) in [
                ("global-term".to_owned(), "global".to_owned()),
                ("meeting-term".to_owned(), format!("meeting:{meeting}")),
            ] {
                db.execute(
                    "INSERT INTO vocabulary_entries VALUES(?1,?2,0,'点検日',NULL,NULL,1,1,1,1)",
                    params![id, scope],
                )
                .unwrap();
            }
            db.execute("INSERT INTO restore_provenance VALUES('restored',?1,'meeting',?1,'archive','','original','batch',1)",params![meeting]).unwrap();
            // Generic older meeting-associated history for SET NULL checks,
            // not a malformed model-deletion receipt. Completed delete_assets
            // now requires its own canonical proof and startup validation.
            db.execute("INSERT INTO maintenance_operations VALUES('older',?1,'prune_revisions','completed',NULL,'{}',?2,1,'{}',NULL,1,1)", params![meeting,"a".repeat(64)]).unwrap();
            db.execute("INSERT INTO model_packages VALUES('retained-model','fixture','keep','1',NULL,'manual_import','{}',?1,NULL,'ready')", params!["b".repeat(64)]).unwrap();
        }
        Self {
            _root: root,
            storage,
            meeting,
            other,
        }
    }
    fn scope(&self) -> Scope {
        Scope::capture(
            &self.storage.connection.lock().unwrap(),
            &self.meeting,
            "",
            &|| false,
        )
        .unwrap()
    }
    fn consent(&self, scope: &Scope) -> String {
        let operation = uuid::Uuid::new_v4().to_string();
        self.storage.connection.lock().unwrap().execute("INSERT INTO maintenance_operations VALUES(?1,?2,'delete_meeting','cleanup',NULL,?3,?4,1,'{}',NULL,1,1)", params![operation,self.meeting,scope.encode().unwrap(),scope.hash().unwrap()]).unwrap();
        operation
    }
}
fn count(db: &Connection, name: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn deletion_scope_counts_public_previous_candidate_internal_drafts_evidence_and_fixed_jobs_without_copying_text(
) {
    let f = Fixture::new();
    let scope = f.scope();
    let rows = |name| scope.tables.iter().find(|t| t.name == name).unwrap().rows;
    assert_eq!(rows("meetings"), 1);
    assert_eq!(rows("minutes_revisions"), 3);
    assert_eq!(rows("processing_jobs"), 3);
    assert!(rows("analysis_revisions") >= 2);
    assert_eq!(rows("job_checkpoints"), 3);
    assert_eq!(rows("working_drafts"), 3);
    assert_eq!(rows("edit_operations"), 2);
    assert!(rows("evidence_links") > 0);
    assert!(rows("utterances") > 0);
    assert_eq!(rows("vocabulary_entries"), 1);
    let manifest = scope.encode().unwrap();
    assert!(!manifest.contains("削除対象の手動編集全文"));
    assert!(!manifest.contains("点検日"));
    assert!(!manifest.contains("対象外会議を保持"));
    assert_eq!(scope, f.scope());
    assert_eq!(scope, serde_json::from_str(&manifest).unwrap());
}

#[test]
fn deletion_row_phase_clears_heads_then_dependencies_and_keeps_other_meetings_models_and_external_files(
) {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let external = f._root.path().join("external-export.md");
    std::fs::write(
        &external,
        b"external export is not a managed deletion target",
    )
    .unwrap();
    let db = f.storage.connection.lock().unwrap();
    let total: u64 = scope.tables.iter().map(|t| t.rows).sum();
    assert_eq!(
        erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap(),
        total
    );
    assert_eq!(count(&db, "meetings"), 1);
    for name in [
        "analysis_revisions",
        "minutes_revisions",
        "working_drafts",
        "edit_operations",
        "processing_jobs",
        "job_checkpoints",
        "analysis_input_snapshots",
        "speakers",
        "utterances",
        "minutes_items",
        "evidence_links",
        "recording_sessions",
        "audio_tracks",
        "audio_chunks",
        "file_assets",
        "restore_provenance",
    ] {
        assert_eq!(count(&db, name), 0, "{name}");
    }
    assert_eq!(count(&db, "model_packages"), 1);
    assert_eq!(count(&db, "app_settings"), 1);
    assert_eq!(count(&db, "vocabulary_entries"), 1);
    assert_eq!(
        db.query_row("SELECT term FROM vocabulary_entries", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "点検日"
    );
    let terminal: (Option<String>, String, String) = db
        .query_row(
            "SELECT meeting_id,state,target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(terminal, (None, "cleanup".into(), scope.encode().unwrap()));
    assert!(db
        .query_row(
            "SELECT meeting_id FROM maintenance_operations WHERE id='older'",
            [],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap()
        .is_none());
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert_eq!(
        std::fs::read(external).unwrap(),
        b"external export is not a managed deletion target"
    );
    drop(db);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
}

#[test]
fn deletion_changed_title_draft_rows_and_provenance_refuse_confirmed_manifest_before_row_removal() {
    for change in ["title", "draft", "provenance"] {
        let f = Fixture::new();
        let scope = f.scope();
        let operation = f.consent(&scope);
        let db = f.storage.connection.lock().unwrap();
        match change {
            "title" => {
                db.execute(
                    "UPDATE meetings SET title='確認後の編集' WHERE id=?1",
                    params![f.meeting],
                )
                .unwrap();
            }
            "draft" => {
                db.execute("UPDATE working_drafts SET payload_json='{\"changed\":true}' WHERE meeting_id=?1", params![f.meeting]).unwrap();
            }
            "provenance" => {
                db.execute("INSERT INTO restore_provenance VALUES('new',?1,'meeting',?1,'archive','','new-source','new-batch',2)",params![f.meeting]).unwrap();
            }
            _ => unreachable!(),
        }
        let actual = Scope::capture(&db, &f.meeting, &operation, &|| false).unwrap();
        assert!(
            erase_confirmed_rows(&db, &operation, &scope, &|| false).is_err(),
            "{change}"
        );
        assert_eq!(
            actual,
            Scope::capture(&db, &f.meeting, &operation, &|| false).unwrap()
        );
        assert!(db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .unwrap());
    }
}

#[test]
fn deletion_missing_consent_wrong_operation_and_changed_durable_hash_remove_no_rows() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    assert!(
        erase_confirmed_rows(&db, &uuid::Uuid::new_v4().to_string(), &scope, &|| false).is_err()
    );
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET consent_at=NULL WHERE id=?1",
        params![operation],
    )
    .unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    assert!(erase_confirmed_rows(&db, &operation, &scope, &|| false).is_err());
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET consent_at=1,target_hash=?2 WHERE id=?1",
        params![operation, "f".repeat(64)],
    )
    .unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    assert!(erase_confirmed_rows(&db, &operation, &scope, &|| false).is_err());
    assert_eq!(count(&db, "minutes_revisions"), 3);
    assert_eq!(count(&db, "meetings"), 2);
}

#[test]
fn deletion_cancel_after_partial_sql_work_rolls_back_rows_heads_and_receipt_and_restores_guards() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    let cancelled = Cell::new(false);
    let cancel = || {
        let gone = count(&db, "working_drafts") == 0;
        cancelled.set(cancelled.get() || gone);
        gone
    };
    assert!(erase_confirmed_rows(&db, &operation, &scope, &cancel).is_err());
    assert!(cancelled.get());
    assert_eq!(
        Scope::capture(&db, &f.meeting, &operation, &|| false).unwrap(),
        scope
    );
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert!(db
        .execute(
            "DELETE FROM minutes_revisions WHERE publication_state='committed'",
            []
        )
        .is_err());
    assert_eq!(
        db.query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            params![operation],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "cleanup"
    );
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
}

#[test]
fn deletion_unwinding_after_partial_sql_work_cannot_leave_trigger_protection_disabled() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        erase_confirmed_rows(&db, &operation, &scope, &|| {
            if count(&db, "working_drafts") == 0 {
                panic!("synthetic deletion failure");
            }
            false
        })
    }));
    assert!(result.is_err());
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert_eq!(
        Scope::capture(&db, &f.meeting, &operation, &|| false).unwrap(),
        scope
    );
    assert!(db
        .execute(
            "DELETE FROM analysis_revisions WHERE publication_state='committed'",
            []
        )
        .is_err());
}

#[test]
fn deletion_scope_refuses_cross_meeting_retry_references_without_widening_its_targets() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    // Simulate a FK-valid external change that bypassed the application's
    // retry guard; the deletion closure must independently refuse it.
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,retry_of_job_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) SELECT 'other-retry',?1,kind,'cancelled','other-operation',id,input_hash,profile_snapshot_json,parameters_json,spec_hash,0,0 FROM processing_jobs LIMIT 1",params![f.other]).unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("対象外"));
    assert_eq!(count(&db, "meetings"), 2);
}

#[test]
fn deletion_scope_refuses_foreign_schema_missing_guards_and_active_work() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('active',?1,'transcription','running','active',?2,'{}','{}',?2,1,1)",params![f.meeting,"a".repeat(64)]).unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("停止完了"));
    db.execute(
        "UPDATE processing_jobs SET state='cancelled' WHERE state='running'",
        [],
    )
    .unwrap();
    db.execute_batch("DROP TRIGGER minutes_committed_no_delete")
        .unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("保護制約"));
}

fn operation_state(db: &Connection, operation: &str) -> String {
    db.query_row(
        "SELECT state FROM maintenance_operations WHERE id=?1",
        params![operation],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
#[cfg(unix)]
fn deletion_database_purge_removes_unique_text_from_database_and_wal_without_removing_other_rows() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let path = f._root.path().join("meetings.sqlite3");
    let wal = f._root.path().join("meetings.sqlite3-wal");
    let marker = format!("DELETE-PRIVATE-{}-BODY", uuid::Uuid::new_v4());
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE meetings SET title=?2 WHERE id=?1",
        params![f.meeting, marker],
    )
    .unwrap();
    // Include overflow pages and old WAL frames, not just a short title.
    db.execute("UPDATE working_drafts SET payload_json=json_set(payload_json,'$.deletionProbe',?2) WHERE meeting_id=?1", params![f.meeting,marker.repeat(500)]).unwrap();
    assert!(std::fs::read(&wal)
        .unwrap()
        .windows(marker.len())
        .any(|w| w == marker.as_bytes()));
    let other = Scope::capture(&db, &f.other, "", &|| false).unwrap();
    drop(db);
    let scope = f.scope();
    let operation = f.consent(&scope);
    let original = std::fs::metadata(&path).unwrap();
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(operation_state(&db, &operation), "cleanup");
    assert_eq!(
        db.query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(operation_state(&db, &operation), "completed");
    assert_eq!(Scope::capture(&db, &f.other, "", &|| false).unwrap(), other);
    assert_eq!(count(&db, "vocabulary_entries"), 1);
    assert_eq!(count(&db, "model_packages"), 1);
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!((original.dev(), original.ino()), (after.dev(), after.ino()));
    assert_eq!(
        db.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    for file in [&path, &wal] {
        if file.exists() {
            let bytes = std::fs::read(file).unwrap();
            assert!(
                !bytes.windows(marker.len()).any(|w| w == marker.as_bytes()),
                "{}",
                file.display()
            );
        }
    }
    // Lost completion acknowledgement does not repeat row removal or invent
    // another operation, and its checkpoint is safe to retry.
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(count(&db, "meetings"), 1);
}

#[test]
fn deletion_database_purge_resumes_after_cold_reopen_of_committed_row_phase() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    erase_confirmed_rows(
        &f.storage.connection.lock().unwrap(),
        &operation,
        &scope,
        &|| false,
    )
    .unwrap();
    let Fixture {
        _root: root,
        storage,
        meeting,
        other,
    } = f;
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let db = storage.connection.lock().unwrap();
    let _writer = super::fence::write_scope_fixture(&storage, &db, &operation, &scope).unwrap();
    assert_eq!(operation_state(&db, &operation), "cleanup");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM meetings WHERE id=?1",
            params![meeting],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(operation_state(&db, &operation), "completed");
    drop(_writer);
    drop(db);
    assert!(storage.get_meeting(&other).unwrap().is_some());
}

#[test]
fn deletion_database_purge_cancel_after_vacuum_keeps_cleanup_until_successful_retry() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    let checks = Cell::new(0);
    assert!(finish_database_purge(&db, &operation, &scope, &|| {
        checks.set(checks.get() + 1);
        checks.get() == 4
    })
    .is_err());
    assert_eq!(checks.get(), 4);
    assert_eq!(operation_state(&db, &operation), "cleanup");
    assert_eq!(count(&db, "meetings"), 1);
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(operation_state(&db, &operation), "completed");
}

#[test]
fn deletion_database_purge_requires_exact_consent_row_phase_and_no_recreated_meeting() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    assert!(finish_database_purge(&db, &operation, &scope, &|| false).is_err());
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    let mut wrong = scope.clone();
    wrong.maintenance_hash = "f".repeat(64);
    assert!(finish_database_purge(&db, &operation, &wrong, &|| false).is_err());
    assert!(
        finish_database_purge(&db, &uuid::Uuid::new_v4().to_string(), &scope, &|| false).is_err()
    );
    db.execute(
        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
        params![operation],
    )
    .unwrap();
    assert!(finish_database_purge(&db, &operation, &scope, &|| false).is_err());
    db.execute(
        "UPDATE maintenance_operations SET progress_json='{\"databaseDeleted\":true}' WHERE id=?1",
        params![operation],
    )
    .unwrap();
    db.execute("INSERT INTO meetings(id,title,lifecycle,source_type,created_at,updated_at) VALUES(?1,'別の会議に再利用','draft','recording',1,1)",params![f.meeting]).unwrap();
    assert!(finish_database_purge(&db, &operation, &scope, &|| false)
        .unwrap_err()
        .contains("完了記録"));
    assert_eq!(operation_state(&db, &operation), "cleanup");
    assert_eq!(count(&db, "meetings"), 2);
}

#[test]
fn deletion_scope_checks_migration_history_and_disabled_guards_before_mutation() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    db.execute_batch("CREATE TEMP TABLE meetings(id TEXT)")
        .unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("一時オブジェクト"));
    db.execute_batch("DROP TABLE temp.meetings").unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("参照保護"));
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false)
        .unwrap_err()
        .contains("参照保護"));
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    db.execute(
        "UPDATE schema_migrations SET checksum=?1 WHERE version=1",
        params!["f".repeat(64)],
    )
    .unwrap();
    assert!(Scope::capture(&db, &f.meeting, "", &|| false).is_err());
    assert_eq!(count(&db, "meetings"), 2);
}

#[test]
fn deletion_dependency_order_removes_retry_leaves_before_parents() {
    let f = Fixture::new();
    {
        let db = f.storage.connection.lock().unwrap();
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('retry-root',?1,'transcription','cancelled','retry-root',?2,'{}','{}',?2,0,0)",params![f.meeting,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,retry_of_job_id,attempt,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) SELECT 'retry-one',meeting_id,kind,'cancelled','retry-op-one',id,attempt+1,input_hash,profile_snapshot_json,parameters_json,spec_hash,0,0 FROM processing_jobs WHERE id='retry-root'",[]).unwrap();
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,retry_of_job_id,attempt,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) SELECT 'retry-two',meeting_id,kind,'cancelled','retry-op-two',id,attempt+1,input_hash,profile_snapshot_json,parameters_json,spec_hash,0,0 FROM processing_jobs WHERE id='retry-one'",[]).unwrap();
    }
    let scope = f.scope();
    assert_eq!(
        scope
            .tables
            .iter()
            .find(|t| t.name == "processing_jobs")
            .unwrap()
            .rows,
        6
    );
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(count(&db, "processing_jobs"), 0);
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
}

#[test]
fn deletion_database_purge_checkpoint_failure_does_not_complete_cleanup() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    let mut statement = db.prepare("SELECT * FROM meetings").unwrap();
    let mut rows = statement.query([]).unwrap();
    assert!(rows.next().unwrap().is_some());
    // A live read prevents truncating this connection's WAL. Production
    // admission flushes users before the purge; this verifies fail-closed.
    assert!(finish_database_purge(&db, &operation, &scope, &|| false).is_err());
    assert_eq!(operation_state(&db, &operation), "cleanup");
    drop(rows);
    drop(statement);
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(operation_state(&db, &operation), "completed");
}

#[test]
fn deletion_database_purge_refuses_active_other_meeting_work_before_completion() {
    let f = Fixture::new();
    let scope = f.scope();
    let operation = f.consent(&scope);
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('other-active',?1,'transcription','running','other-active',?2,'{}','{}',?2,1,1)", params![f.other,"a".repeat(64)]).unwrap();
    assert!(finish_database_purge(&db, &operation, &scope, &|| false)
        .unwrap_err()
        .contains("停止完了"));
    assert_eq!(operation_state(&db, &operation), "cleanup");
    db.execute(
        "UPDATE processing_jobs SET state='cancelled' WHERE id='other-active'",
        [],
    )
    .unwrap();
    finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
    assert_eq!(count(&db, "processing_jobs"), 1);
}

#[test]
#[cfg(unix)]
#[ignore = "subprocess fixture; invoked by the deletion SIGKILL test"]
fn deletion_checkpoint_child_fixture() {
    let base = std::path::PathBuf::from(std::env::var("MON_DELETE_TEST_BASE").unwrap());
    let phase = std::env::var("MON_DELETE_TEST_PHASE").unwrap();
    let f = Fixture::at(tempfile::tempdir_in(&base).unwrap());
    let scope = f.scope();
    let operation = f.consent(&scope);
    let wait = || -> ! {
        let receipt = serde_json::to_vec(&(f._root.path(), &operation, &scope, &f.other)).unwrap();
        std::fs::write(base.join("ready.tmp"), receipt).unwrap();
        std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    let db = f.storage.connection.lock().unwrap();
    erase_confirmed_rows(&db, &operation, &scope, &|| {
        if phase == "partial_rows" && count(&db, "working_drafts") == 0 {
            wait();
        }
        false
    })
    .unwrap();
    if phase == "rows_committed" {
        wait();
    }
    let checks = Cell::new(0);
    finish_database_purge(&db, &operation, &scope, &|| {
        checks.set(checks.get() + 1);
        if phase == "vacuum_done" && checks.get() == 4 {
            wait();
        }
        false
    })
    .unwrap();
    assert_eq!(phase, "completed");
    wait();
}

#[test]
#[cfg(unix)]
fn deletion_sigkill_during_rows_after_commit_after_vacuum_and_after_completion_recovers_safely() {
    use std::os::unix::process::ExitStatusExt;
    for phase in ["partial_rows", "rows_committed", "vacuum_done", "completed"] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::tests::deletion_checkpoint_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_DELETE_TEST_BASE", base.path())
            .env("MON_DELETE_TEST_PHASE", phase)
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
                panic!("deletion checkpoint was not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (path, operation, scope, other): (std::path::PathBuf, String, Scope, String) =
            serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        assert!(path
            .canonicalize()
            .unwrap()
            .starts_with(base.path().canonicalize().unwrap()));
        let storage = Storage::open(path).unwrap();
        let db = storage.connection.lock().unwrap();
        let _writer = if storage.deletion_fence.pending() {
            Some(super::fence::write_scope_fixture(&storage, &db, &operation, &scope).unwrap())
        } else {
            None
        };
        assert!(db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .unwrap());
        if phase == "partial_rows" {
            assert_eq!(
                Scope::capture(&db, &scope.meeting_id, &operation, &|| false).unwrap(),
                scope
            );
            erase_confirmed_rows(&db, &operation, &scope, &|| false).unwrap();
        } else {
            assert_eq!(count(&db, "meetings"), 1);
            assert_eq!(
                operation_state(&db, &operation),
                if phase == "completed" {
                    "completed"
                } else {
                    "cleanup"
                }
            );
        }
        finish_database_purge(&db, &operation, &scope, &|| false).unwrap();
        assert_eq!(operation_state(&db, &operation), "completed");
        assert_eq!(count(&db, "meetings"), 1);
        drop(_writer);
        drop(db);
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert!(storage.get_meeting(&scope.meeting_id).unwrap().is_none());
    }
}
