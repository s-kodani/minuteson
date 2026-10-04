use super::super::tests::{all_rows, fixture, ids, sealed};
use super::*;

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
fn restore_journal_migrates_v21_and_excludes_all_local_recovery_records_from_backup() {
    let f = fixture();
    let before = all_rows(&f.target.connection.lock().unwrap());
    remove_schema_for_test(&f.target.connection.lock().unwrap());
    assert_eq!(
        verify_migration_history(&f.target.connection.lock().unwrap()).unwrap(),
        21
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
    assert_eq!(source.manifest.tables.len(), 31);
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
    assert_eq!(source.manifest.tables.len(), 31);
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
