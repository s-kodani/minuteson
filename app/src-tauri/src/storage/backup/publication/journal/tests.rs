use super::super::tests::{all_rows, fixture, ids, sealed};
use super::*;

#[cfg(target_os = "macos")]
#[test]
fn history_install_preserves_restore_keep_receipt_on_destination_startup() {
    use crate::bootstrap::Bootstrap;
    use std::os::unix::fs::DirBuilderExt;
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let first = paths(&sealed, &f.target.root)[0].clone();
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        std::fs::write(first.join("unknown"), b"retained restore body").unwrap();
        drop(install);
    }
    let item = f
        .target
        .recovery_listing()
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.source == recovery::Source::RestoreAddition)
        .unwrap();
    let mut keep = f
        .target
        .prepare_recovery(
            recovery::Source::RestoreAddition,
            &item.operation_id,
            &|| false,
        )
        .unwrap();
    keep.resolve(&f.target, &keep.preview().confirmation_id, true, &|| false)
        .unwrap();
    drop(keep);
    drop(sealed);
    drop(ids);
    let area = tempfile::tempdir().unwrap();
    let base = area.path().canonicalize().unwrap();
    let config = base.join("config");
    let data = base.join("data");
    let b = Bootstrap::open(&config, &data).unwrap();
    let seed = Storage::open(b.root().into()).unwrap();
    b.initialize(&seed).unwrap();
    drop(seed);
    drop(b);
    // The test prepares a source bootstrap record, not a production cutover.
    crate::bootstrap::set_test_root(&config, &data, &f.target.root);
    let b = Bootstrap::open(&config, &data).unwrap();
    let target = base.join("destination");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let native = f.target.prepare_relocation_snapshot(&|| false).unwrap();
    let op = Uuid::new_v4().to_string();
    b.begin_move(
        &op,
        &Uuid::new_v4().to_string(),
        &target,
        native.manifest_hash(),
        0,
    )
    .unwrap();
    let mut p = b.pending_placement(&op, &native).unwrap();
    p.copy(&native, &|| false).unwrap();
    let control = native.prepare_control_handoff(&|| false).unwrap();
    p.install_history(&control, &|| false).unwrap();
    assert_eq!(
        Connection::open(target.join(".mon-scratch.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT count(*) FROM relocation_restore_bindings",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(p);
    drop(control);
    drop(native);
    let reopened = Storage::open(target.clone()).unwrap();
    assert!(reopened
        .recovery_listing()
        .unwrap()
        .items
        .iter()
        .any(|i| i.operation_id == item.operation_id && i.retained));
    assert_eq!(
        std::fs::read(first.join("unknown")).unwrap(),
        b"retained restore body"
    );
    assert_eq!(
        std::fs::read(
            target
                .join(first.strip_prefix(&f.target.root).unwrap())
                .join("unknown")
        )
        .unwrap(),
        b"retained restore body"
    );
    let old = Connection::open(f.target.root.join(".mon-scratch.sqlite3")).unwrap();
    let new = Connection::open(target.join(".mon-scratch.sqlite3")).unwrap();
    let read = |db: &Connection| -> String {
        db.query_row(
            "SELECT receipt_json FROM recovery_decisions WHERE operation_id=?1",
            params![item.operation_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(read(&old), read(&new));
}

#[cfg(target_os = "macos")]
#[test]
fn control_handoff_keeps_restore_consent_and_historical_root_proofs_without_rewriting_native_db() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let first = paths(&sealed, &f.target.root)[0].clone();
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        std::fs::write(first.join("unknown"), b"kept restoration").unwrap();
        drop(install);
    }
    let item = f
        .target
        .recovery_listing()
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.source == recovery::Source::RestoreAddition)
        .unwrap();
    let mut review = f
        .target
        .prepare_recovery(
            recovery::Source::RestoreAddition,
            &item.operation_id,
            &|| false,
        )
        .unwrap();
    assert_eq!(
        review
            .resolve(&f.target, &review.preview().confirmation_id, true, &|| {
                false
            })
            .unwrap()
            .outcome,
        recovery::Outcome::Retained
    );
    drop(review);
    drop(sealed);
    drop(ids);
    let before = all_rows(&f.target.connection.lock().unwrap());
    let saved = f.target.prepare_relocation_snapshot(&|| false).unwrap();
    let handoff = saved.prepare_control_handoff(&|| false).unwrap();
    handoff.verify(&|| false).unwrap();
    // Inspect the sealed bytes for this test only; production cannot construct
    // a ControlHandoff from deserialized history or consent supplied by a UI.
    let path = saved
        .image()
        .parent()
        .unwrap()
        .join("relocation-control.json");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let history = &data["history"];
    assert!(history["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["source"] == "restore_addition" && d["operation_id"] == item.operation_id));
    assert!(history["restores"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["operation_id"] == item.operation_id
            && r["record_valid"] == true
            && r["state"] == "attention"
            && r["published"] == false));
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    assert_eq!(
        std::fs::read(first.join("unknown")).unwrap(),
        b"kept restoration"
    );
}

#[test]
fn restore_attention_keep_preserves_files_on_restart_and_disables_old_batch_publication_without_live_edits(
) {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    let first = paths(&sealed, &root)[0].clone();
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        std::fs::write(first.join("unknown"), b"keep unknown").unwrap();
        drop(install);
        assert_eq!(state(&db, &batch), "attention");
    }
    let before = all_rows(&f.target.connection.lock().unwrap());
    let listing = f.target.recovery_listing().unwrap();
    let item = listing
        .items
        .iter()
        .find(|i| i.source == recovery::Source::RestoreAddition)
        .unwrap();
    let operation = item.operation_id.clone();
    let mut review = f
        .target
        .prepare_recovery(recovery::Source::RestoreAddition, &operation, &|| false)
        .unwrap();
    let receipt = review
        .resolve(&f.target, &review.preview().confirmation_id, true, &|| {
            false
        })
        .unwrap();
    assert_eq!(receipt.outcome, recovery::Outcome::Retained);
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    assert!(f
        .target
        .publish_restore(&sealed, &ids, &|| false)
        .unwrap_err()
        .contains("保持して確認済み"));
    assert_eq!(
        std::fs::read(first.join("unknown")).unwrap(),
        b"keep unknown"
    );
    std::fs::remove_file(first.join("unknown")).unwrap();
    drop(review);
    drop(ids);
    drop(f.target);
    let reopened = Storage::open(root).unwrap();
    assert!(
        first.is_dir(),
        "keep decision must protect even a now empty area"
    );
    assert_eq!(all_rows(&reopened.connection.lock().unwrap()), before);
    assert!(reopened
        .recovery_listing()
        .unwrap()
        .items
        .iter()
        .any(|i| i.operation_id == operation && i.retained));
}

#[test]
fn restore_attention_confirmed_retry_cannot_follow_changed_parents_or_delete_existing_meetings() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let first = paths(&sealed, &f.target.root)[0].clone();
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        std::fs::write(first.join("unknown"), b"keep").unwrap();
        drop(install);
    }
    let item = f
        .target
        .recovery_listing()
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.source == recovery::Source::RestoreAddition)
        .unwrap();
    let before = all_rows(&f.target.connection.lock().unwrap());
    let mut review = f
        .target
        .prepare_recovery(item.source, &item.operation_id, &|| false)
        .unwrap();
    std::fs::write(first.join("new-unknown"), b"new").unwrap();
    assert!(review
        .resolve(&f.target, &review.preview().confirmation_id, false, &|| {
            false
        })
        .is_err());
    assert!(first.join("unknown").exists());
    let mut refreshed = f
        .target
        .prepare_recovery(item.source, &item.operation_id, &|| false)
        .unwrap();
    assert_eq!(
        refreshed
            .resolve(
                &f.target,
                &refreshed.preview().confirmation_id,
                false,
                &|| false
            )
            .unwrap()
            .outcome,
        recovery::Outcome::Attention
    );
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    std::fs::remove_file(first.join("unknown")).unwrap();
    std::fs::remove_file(first.join("new-unknown")).unwrap();
    let mut final_review = f
        .target
        .prepare_recovery(item.source, &item.operation_id, &|| false)
        .unwrap();
    assert_eq!(
        final_review
            .resolve(
                &f.target,
                &final_review.preview().confirmation_id,
                false,
                &|| false
            )
            .unwrap()
            .outcome,
        recovery::Outcome::Recovered
    );
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    assert!(!first.exists());
}

fn state(db: &Connection, batch: &str) -> String {
    db.query_row(
        "SELECT state FROM maintenance_operations WHERE restore_batch_id=?1",
        params![batch],
        |r| r.get(0),
    )
    .unwrap()
}
fn recorded<'a>(
    f: &super::super::tests::Fixture,
    db: &'a Connection,
    sealed: &SealedWorkspace,
    ids: &RestoreIds<'_>,
) -> Installed<'a> {
    let journal = Journal::begin(db, &f.target.root, sealed, ids).unwrap();
    Installed::prepare_recorded(&f.target.root, sealed, journal, &|| false).unwrap()
}
fn paths(sealed: &SealedWorkspace, root: &Path) -> Vec<PathBuf> {
    read_meetings(sealed.connection())
        .unwrap()
        .into_iter()
        .map(|m| root.join("meetings").join(m.id))
        .collect()
}
#[test]
fn restore_journal_current_reopen_excludes_all_local_recovery_records_from_backup() {
    let f = fixture();
    let before = all_rows(&f.target.connection.lock().unwrap());
    assert_eq!(
        verify_migration_history(&f.target.connection.lock().unwrap()).unwrap(),
        MIGRATIONS.len()
    );
    let root = f.target.root.clone();
    drop(f.target);
    let target = Storage::open(root).unwrap();
    assert_eq!(
        verify_migration_history(&target.connection.lock().unwrap()).unwrap(),
        MIGRATIONS.len()
    );
    assert_eq!(all_rows(&target.connection.lock().unwrap()), before);
    let path = f.output.path().join("v22.monbackup");
    target.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), f.output.path(), &|| false).unwrap();
    assert_eq!(source.manifest.schema_version, MIGRATIONS.len());
    assert_eq!(source.manifest.tables.len(), 33);
    assert!(source
        .manifest
        .tables
        .iter()
        .all(|t| !t.name.starts_with("maintenance")));
}

#[test]
fn restore_journal_reopen_cleans_unpublished_files_and_reuses_the_same_batch_ids() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    let before = all_rows(&f.target.connection.lock().unwrap());
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        assert_eq!(state(&db, &batch), "installing");
        std::mem::forget(install); // Simulate losing the in-memory cleanup owner.
    }
    assert!(paths(&sealed, &root).iter().all(|p| p.exists()));
    drop(ids);
    drop(f.target);
    let target = Storage::open(root.clone()).unwrap();
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "recovered"
    );
    assert!(paths(&sealed, &root).iter().all(|p| !p.exists()));
    assert_eq!(all_rows(&target.connection.lock().unwrap()), before);
    let replay = match RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &batch,
        &target.connection.lock().unwrap(),
        &|| false,
    )
    .unwrap()
    {
        restore_ids::BatchPreparation::Pending(ids) => ids,
        _ => panic!(),
    };
    assert!(matches!(
        target.publish_restore(&sealed, &replay, &|| false).unwrap(),
        Publication::Published { meetings: 2, .. }
    ));
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "completed"
    );
}

#[test]
fn restore_journal_partial_copy_and_repeated_cleanup_remove_only_registered_owned_paths() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    {
        let db = f.target.connection.lock().unwrap();
        let journal = Journal::begin(&db, &root, &sealed, &ids).unwrap();
        let target = targets(&sealed)
            .unwrap()
            .into_iter()
            .find(|t| t.kind == "file")
            .unwrap();
        let path = root.join(target.path);
        for folder in [
            path.parent().unwrap().parent().unwrap(),
            path.parent().unwrap(),
        ] {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(folder).unwrap();
            journal
                .record(folder, &std::fs::symlink_metadata(folder).unwrap())
                .unwrap();
        }
        let mut file = private_file(&path).unwrap();
        journal.record(&path, &file.metadata().unwrap()).unwrap();
        file.write_all(b"partial").unwrap();
        file.sync_all().unwrap();
    }
    drop(f.target);
    let target = Storage::open(root.clone()).unwrap();
    assert!(paths(&sealed, &root).iter().all(|p| !p.exists()));
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "recovered"
    );
    let before = all_rows(&target.connection.lock().unwrap());
    recover_all(&root, &target.connection.lock().unwrap()).unwrap();
    assert_eq!(all_rows(&target.connection.lock().unwrap()), before);
}

#[test]
fn restore_journal_unrecorded_creation_gap_keeps_unknown_material_and_reports_attention() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    {
        let db = f.target.connection.lock().unwrap();
        let _journal = Journal::begin(&db, &root, &sealed, &ids).unwrap();
    }
    let new = paths(&sealed, &root).remove(0);
    std::fs::create_dir(&new).unwrap();
    let unknown = new.join("unknown.txt");
    std::fs::write(&unknown, b"not proven owned").unwrap();
    drop(f.target);
    let target = Storage::open(root.clone()).unwrap();
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "attention"
    );
    assert_eq!(std::fs::read(&unknown).unwrap(), b"not proven owned");
    assert_eq!(target.list_meetings().unwrap().len(), 1);
}

#[test]
fn restore_journal_published_batch_never_reclaims_or_rewrites_its_files_after_reopen() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    f.target.publish_restore(&sealed, &ids, &|| false).unwrap();
    let relative: String = sealed
        .connection()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let path = f.target.root.join(relative);
    std::fs::write(&path, b"published external change").unwrap();
    {
        let db = f.target.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET state='installing' WHERE restore_batch_id=?1",
            params![batch],
        )
        .unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    let before = all_rows(&f.target.connection.lock().unwrap());
    let root = f.target.root.clone();
    drop(f.target);
    let target = Storage::open(root).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"published external change");
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "completed"
    );
    assert_eq!(all_rows(&target.connection.lock().unwrap()), before);
}

#[cfg(unix)]
#[test]
fn restore_journal_substituted_parent_and_unknown_files_are_preserved_without_recursive_deletion() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    {
        let db = f.target.connection.lock().unwrap();
        std::mem::forget(recorded(&f, &db, &sealed, &ids));
    }
    let outside = tempfile::tempdir().unwrap();
    let new = paths(&sealed, &root);
    let moved = outside.path().join("moved");
    std::fs::rename(&new[0], &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &new[0]).unwrap();
    let outside_file = moved.join("outside.txt");
    std::fs::write(&outside_file, b"outside").unwrap();
    let unknown = new[1].join("restored").join("unknown.txt");
    std::fs::write(&unknown, b"unknown").unwrap();
    drop(f.target);
    let target = Storage::open(root.clone()).unwrap();
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "attention"
    );
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside");
    assert_eq!(std::fs::read(&unknown).unwrap(), b"unknown");
    recover_all(&root, &target.connection.lock().unwrap()).unwrap();
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"outside");
    assert_eq!(std::fs::read(&unknown).unwrap(), b"unknown");
}

#[test]
fn restore_journal_rejects_corrupted_manifest_or_identity_before_touching_any_files() {
    for change in ["manifest", "identity", "target"] {
        let f = fixture();
        let batch = Uuid::new_v4().to_string();
        let ids = ids(&f, &batch);
        let sealed = sealed(&f, &ids);
        let root = f.target.root.clone();
        {
            let db = f.target.connection.lock().unwrap();
            std::mem::forget(recorded(&f, &db, &sealed, &ids));
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            match change {
                "manifest" => {
                    db.execute("UPDATE maintenance_operations SET target_manifest_json=json_set(target_manifest_json,'$.mapping_count',0) WHERE restore_batch_id=?1",params![batch]).unwrap();
                }
                "identity" => {
                    db.execute("UPDATE maintenance_paths SET identity_json=json_set(identity_json,'$.inode',0) WHERE identity_json IS NOT NULL",[]).unwrap();
                }
                "target" => {
                    db.execute(
                        "UPDATE maintenance_paths SET content_hash=?1 WHERE kind='file'",
                        params!["a".repeat(64)],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
                .unwrap();
        }
        let before = paths(&sealed, &root);
        drop(f.target);
        let target = Storage::open(root.clone()).unwrap();
        assert_eq!(
            state(&target.connection.lock().unwrap(), &batch),
            "attention",
            "{change}"
        );
        assert!(before.iter().all(|p| p.exists()), "{change}");
    }
}

#[test]
fn restore_journal_drop_after_successful_publication_never_unlinks_committed_assets() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let db = f.target.connection.lock().unwrap();
    let install = recorded(&f, &db, &sealed, &ids);
    let merge = sealed.global_merge(&db, &|| false).unwrap();
    let mut tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate).unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;").unwrap();
    append(&tx, sealed.connection(), &|| false).unwrap();
    merge.apply(&mut tx, &|| false).unwrap();
    tx.execute(
        "UPDATE restore_batches SET state='published',published_at=1 WHERE batch_id=?1",
        params![batch],
    )
    .unwrap();
    install.completed(&tx).unwrap();
    tx.commit().unwrap();
    // Deliberately omit keep(): unwinding after COMMIT must use the ledger.
    drop(install);
    assert!(paths(&sealed, &f.target.root).iter().all(|p| p.exists()));
    assert_eq!(state(&db, &batch), "completed");
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn restore_journal_referenced_unpublished_target_is_not_cleaned_even_with_owned_identity() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let db = f.target.connection.lock().unwrap();
    let install = recorded(&f, &db, &sealed, &ids);
    let meeting = read_meetings(sealed.connection()).unwrap().remove(0).id;
    db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at,lock_version) VALUES(?1,'now referenced','draft',0,0,0)",params![meeting]).unwrap();
    drop(install);
    assert_eq!(state(&db, &batch), "attention");
    assert!(paths(&sealed, &f.target.root).iter().all(|p| p.exists()));
}

#[test]
fn restore_journal_cleanup_commit_failure_retains_markers_and_replays_missing_paths_on_reopen() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let root = f.target.root.clone();
    {
        let db = f.target.connection.lock().unwrap();
        let install = recorded(&f, &db, &sealed, &ids);
        db.commit_hook(Some(|| true)).unwrap();
        drop(install);
        assert_eq!(state(&db, &batch), "installing");
        assert!(paths(&sealed, &root).iter().all(|p| !p.exists()));
        assert!(
            db.query_row(
                "SELECT count(*) FROM maintenance_paths WHERE identity_json IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
        db.commit_hook(None::<fn() -> bool>).unwrap();
    }
    drop(f.target);
    let target = Storage::open(root.clone()).unwrap();
    assert_eq!(
        state(&target.connection.lock().unwrap(), &batch),
        "recovered"
    );
    assert!(paths(&sealed, &root).iter().all(|p| !p.exists()));
}

#[test]
fn restore_journal_recovered_operations_are_not_rewalked_and_are_not_portable() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    {
        let db = f.target.connection.lock().unwrap();
        drop(recorded(&f, &db, &sealed, &ids));
        assert_eq!(state(&db, &batch), "recovered");
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "maintenance_paths",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
        recover_all(&f.target.root, &db).unwrap();
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
    }
    let path = f.output.path().join("after-recovery.monbackup");
    f.target.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), f.output.path(), &|| false).unwrap();
    for table in ["maintenance_operations", "maintenance_paths"] {
        assert_eq!(
            source
                .database
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(source.manifest.tables.len(), 33);
}

#[test]
fn restore_journal_rehashed_wrong_meeting_targets_are_rejected_before_cleanup() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let existing = f.target.list_meetings().unwrap().remove(0).id;
    let db = f.target.connection.lock().unwrap();
    let install = recorded(&f, &db, &sealed, &ids);
    std::mem::forget(install);
    let id: String = db
        .query_row(
            "SELECT id FROM maintenance_operations WHERE restore_batch_id=?1",
            params![batch],
            |r| r.get(0),
        )
        .unwrap();
    let old:String=db.query_row("SELECT relative_path FROM maintenance_paths WHERE operation_id=?1 AND kind='directory' AND length(relative_path)=45 LIMIT 1",params![id],|r|r.get(0)).unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute(
        "UPDATE maintenance_paths SET relative_path=?1 WHERE operation_id=?2 AND relative_path=?3",
        params![format!("meetings/{existing}"), id, old],
    )
    .unwrap();
    let rows:Vec<Target>=db.prepare("SELECT relative_path,kind,size_bytes,content_hash FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").unwrap().query_map(params![id],|r|Ok(Target {path:r.get(0)?,kind:r.get(1)?,size:r.get(2)?,hash:r.get(3)?})).unwrap().collect::<Result<_,_>>().unwrap();
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    let mut manifest: Manifest = serde_json::from_str(&json).unwrap();
    manifest.paths_hash = target_hash(&rows).unwrap();
    let json = serde_json::to_string(&manifest).unwrap();
    db.execute(
        "UPDATE maintenance_operations SET target_manifest_json=?1,target_hash=?2 WHERE id=?3",
        params![json, hash(json.as_bytes()), id],
    )
    .unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    recover_all(&f.target.root, &db).unwrap();
    assert_eq!(state(&db, &batch), "attention");
    assert!(paths(&sealed, &f.target.root).iter().all(|p| p.exists()));
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
