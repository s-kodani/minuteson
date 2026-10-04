use super::*;
use restore_fixed::FixedData;
use restore_ids::BatchPreparation;
use std::cell::Cell;

pub(super) struct Fixture {
    _source_root: tempfile::TempDir,
    pub(super) output: tempfile::TempDir,
    pub(super) source: archive::VerifiedArchive,
    pub(super) target_root: tempfile::TempDir,
    pub(super) target: Storage,
}
fn entry(term: &str) -> VocabularyEntry {
    VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: term.into(),
        reading: None,
        note: None,
        enabled: true,
    }
}
pub(super) fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let source_storage = Storage::open(root.path().into()).unwrap();
    let first = archive::tests::meeting(&source_storage, "source-one");
    let mut draft = source_storage
        .load_transcript_draft(&first)
        .unwrap()
        .unwrap();
    draft.payload.schema_version = 2;
    draft
        .payload
        .manual_speakers
        .push(crate::storage::drafts::ManualSpeaker {
            id: format!("manual:{}", Uuid::new_v4()),
            label: "復元用手動話者".into(),
        });
    source_storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    archive::tests::meeting(&source_storage, "source-two");
    source_storage
        .save_vocabulary_book(
            None,
            0,
            vec![entry("完全一致"), entry("競合"), entry("新規")],
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("source.monbackup");
    source_storage
        .create_backup_archive(&path, &|| false)
        .unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    archive::tests::meeting(&target, "existing");
    let mut conflict = entry("競合");
    conflict.reading = Some("既存を保持".into());
    target
        .save_vocabulary_book(
            None,
            0,
            vec![entry("完全一致"), conflict, entry("既存のみ")],
        )
        .unwrap();
    Fixture {
        _source_root: root,
        output,
        source,
        target_root,
        target,
    }
}
pub(super) fn ids<'a>(f: &'a Fixture, batch: &str) -> RestoreIds<'a> {
    let ids = match RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        batch,
        &f.target.connection.lock().unwrap(),
        &|| false,
    )
    .unwrap()
    {
        BatchPreparation::Pending(ids) => ids,
        _ => panic!("unexpected published"),
    };
    let mut db = f.target.connection.lock().unwrap();
    let mut tx = db.transaction().unwrap();
    BatchPlan::new(&ids, &|| false)
        .unwrap()
        .reserve(&mut tx, &|| false)
        .unwrap();
    tx.commit().unwrap();
    ids
}
pub(super) fn sealed(f: &Fixture, ids: &RestoreIds<'_>) -> SealedWorkspace {
    let fixed = FixedData::new(ids, f.output.path(), &|| false).unwrap();
    SealedWorkspace::prepare(&fixed, f.output.path(), &|| false).unwrap()
}
fn dump(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut query = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let count = query.column_count();
    query
        .query_map([], |r| (0..count).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
pub(in crate::storage::backup) fn all_rows(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    portable::TABLES
        .iter()
        .chain(["app_settings", "restore_batches", "restore_identity_maps"].iter())
        .map(|table| dump(db, table))
        .collect()
}
fn triggers_enabled(target: &Storage) {
    let db = target.connection.lock().unwrap();
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert!(db
        .execute(
            "UPDATE analysis_revisions SET config_json='{}' WHERE publication_state='committed'",
            []
        )
        .is_err());
}
#[test]
fn restore_publication_adds_sealed_meetings_files_and_global_terms_atomically_and_retries_once() {
    let f = fixture();
    let batch = Uuid::new_v4().to_string();
    let ids = ids(&f, &batch);
    let sealed = sealed(&f, &ids);
    let old = all_rows(&f.target.connection.lock().unwrap());
    let old_schema = schema(&f.target.connection.lock().unwrap()).unwrap();
    let result = f.target.publish_restore(&sealed, &ids, &|| false).unwrap();
    let Publication::Published {
        meetings,
        vocabulary,
    } = result
    else {
        panic!("not published")
    };
    assert_eq!(meetings, 2);
    assert_eq!(vocabulary.added, 1);
    assert_eq!(vocabulary.deduplicated, 1);
    assert_eq!(vocabulary.conflicts.len(), 1);
    assert_eq!(f.target.list_meetings().unwrap().len(), 3);
    {
        let mut db = f.target.connection.lock().unwrap();
        validate_database(&mut db, &|| false).unwrap();
        assert_eq!(schema(&db).unwrap(), old_schema);
        for (index, table) in portable::TABLES.iter().enumerate() {
            if *table == "vocabulary_scopes" {
                continue;
            }
            assert!(
                dump(&db, table).starts_with(&old[index]),
                "changed existing {table}"
            );
        }
        assert_eq!(dump(&db, "app_settings"), old[portable::TABLES.len()]);
        assert_eq!(
            db.query_row(
                "SELECT state FROM restore_batches WHERE batch_id=?1",
                params![batch],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "published"
        );
        for asset in super::super::read_assets(&db).unwrap() {
            assert_eq!(
                digest_file(&f.target.root.join(asset.relative_path), &|| false).unwrap(),
                (asset.size_bytes as u64, asset.content_hash)
            );
        }
    }
    triggers_enabled(&f.target);
    let published = all_rows(&f.target.connection.lock().unwrap());
    assert_eq!(
        f.target.publish_restore(&sealed, &ids, &|| false).unwrap(),
        Publication::AlreadyPublished
    );
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), published);
    match RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &batch,
        &f.target.connection.lock().unwrap(),
        &|| false,
    )
    .unwrap()
    {
        BatchPreparation::AlreadyPublished { batch_id, .. } => assert_eq!(batch_id, batch),
        _ => panic!("replay should not build again"),
    };
}

#[test]
fn restore_publication_new_explicit_batch_adds_again_without_duplicating_global_terms() {
    let f = fixture();
    for iteration in 0..2 {
        let ids = ids(&f, &Uuid::new_v4().to_string());
        let sealed = sealed(&f, &ids);
        let Publication::Published { vocabulary, .. } =
            f.target.publish_restore(&sealed, &ids, &|| false).unwrap()
        else {
            panic!()
        };
        assert_eq!(vocabulary.added, usize::from(iteration == 0));
    }
    assert_eq!(f.target.list_meetings().unwrap().len(), 5);
    assert_eq!(f.target.vocabulary_book(None).unwrap().payload.len(), 4);
    triggers_enabled(&f.target);
}

#[test]
fn restore_publication_cancellation_after_install_or_append_rolls_back_only_owned_additions() {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    // Persist the prepared reservation, as a future controller will do before
    // rebuilding. A failed publication must leave it prepared and unchanged.
    {
        let mut db = f.target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        BatchPlan::new(&ids, &|| false)
            .unwrap()
            .reserve(&mut tx, &|| false)
            .unwrap();
        tx.commit().unwrap();
    }
    let before = all_rows(&f.target.connection.lock().unwrap());
    let new_meetings: Vec<_> = read_meetings(sealed.connection())
        .unwrap()
        .into_iter()
        .map(|m| f.target.root.join("meetings").join(m.id))
        .collect();
    let observed = Cell::new(0);
    let error = f
        .target
        .publish_restore(&sealed, &ids, &|| {
            if new_meetings.iter().all(|p| p.join("restored").is_dir()) {
                observed.set(observed.get() + 1);
                observed.get() > 20
            } else {
                false
            }
        })
        .unwrap_err();
    assert!(error.contains("中断"), "{error}");
    assert!(observed.get() > 20);
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    assert!(new_meetings.iter().all(|p| !p.exists()));
    triggers_enabled(&f.target);
    assert!(matches!(
        f.target.publish_restore(&sealed, &ids, &|| false).unwrap(),
        Publication::Published { .. }
    ));
}

#[test]
fn restore_publication_refuses_existing_or_linked_target_and_keeps_unrelated_files() {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    let meeting = read_meetings(sealed.connection()).unwrap().remove(0).id;
    let collision = f.target.root.join("meetings").join(meeting);
    std::fs::create_dir(&collision).unwrap();
    let unrelated = collision.join("user.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    let before = all_rows(&f.target.connection.lock().unwrap());
    assert!(f.target.publish_restore(&sealed, &ids, &|| false).is_err());
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    triggers_enabled(&f.target);
    #[cfg(unix)]
    {
        let empty = tempfile::tempdir().unwrap();
        let target = Storage::open(empty.path().into()).unwrap();
        std::os::unix::fs::symlink(f.target.root.join("meetings"), target.root.join("meetings"))
            .unwrap();
        assert!(target.publish_restore(&sealed, &ids, &|| false).is_err());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
        assert!(target.list_meetings().unwrap().is_empty());
    }
}

#[test]
fn restore_publication_rejects_wrong_workspace_schema_and_late_json_identity_collision() {
    let f = fixture();
    let other = ids(&f, &Uuid::new_v4().to_string());
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    assert!(f
        .target
        .publish_restore(&sealed, &other, &|| false)
        .unwrap_err()
        .contains("ID対応"));
    {
        let db = f.target.connection.lock().unwrap();
        db.execute_batch("CREATE TRIGGER unknown_restore BEFORE INSERT ON meetings BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    }
    assert!(f
        .target
        .publish_restore(&sealed, &ids, &|| false)
        .unwrap_err()
        .contains("信頼済み"));
    f.target
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER unknown_restore;")
        .unwrap();
    let json:String=sealed.connection().query_row("SELECT payload_json FROM working_drafts WHERE json_array_length(payload_json,'$.manual_speakers')>0 LIMIT 1",[],|r|r.get(0)).unwrap();
    let restored: crate::storage::TranscriptDraftPayload = serde_json::from_str(&json).unwrap();
    let manual = restored.manual_speakers[0].id.clone();
    // A pre-existing draft can introduce a new JSON-only manual identity after
    // preparation. Reuse the incoming manual ID to force an exact collision.
    let meeting = f.target.list_meetings().unwrap().remove(0).id;
    let mut draft = f.target.load_transcript_draft(&meeting).unwrap().unwrap();
    draft.payload.schema_version = 2;
    draft
        .payload
        .manual_speakers
        .push(crate::storage::drafts::ManualSpeaker {
            id: manual.clone(),
            label: "manual".into(),
        });
    f.target
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let before = all_rows(&f.target.connection.lock().unwrap());
    assert!(f
        .target
        .publish_restore(&sealed, &ids, &|| false)
        .unwrap_err()
        .contains("競合"));
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
}

#[test]
fn restore_append_capability_denies_update_delete_ddl_and_restores_triggers_on_error() {
    let db = portable::schema(MIGRATIONS.len()).unwrap();
    {
        let _guard = AppendOnly::begin(&db).unwrap();
        for sql in [
            "UPDATE meetings SET title='x'",
            "DELETE FROM meetings",
            "DROP TABLE meetings",
            "CREATE TABLE extra(id)",
            "INSERT INTO app_settings(key,value_json,lock_version,updated_at) VALUES('x','{}',0,0)",
            "ATTACH DATABASE ':memory:' AS other",
        ] {
            assert!(db.execute_batch(sql).is_err(), "allowed {sql}");
        }
    }
    assert!(db
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    assert_eq!(
        schema(&db).unwrap(),
        schema(&portable::schema(MIGRATIONS.len()).unwrap()).unwrap()
    );
}

#[test]
fn restore_publication_commit_failure_rolls_back_meetings_merge_and_batch_and_removes_new_files() {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    let before = all_rows(&f.target.connection.lock().unwrap());
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = attempts.clone();
    let ready: Vec<(PathBuf, i64)> = sealed
        .connection()
        .prepare("SELECT relative_path,size_bytes FROM file_assets WHERE state='ready'")
        .unwrap()
        .query_map([], |r| {
            Ok((f.target.root.join(r.get::<_, String>(0)?), r.get(1)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    f.target
        .connection
        .lock()
        .unwrap()
        .commit_hook(Some(move || {
            if ready
                .iter()
                .all(|(path, size)| std::fs::metadata(path).is_ok_and(|m| m.len() == *size as u64))
            {
                seen.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        }))
        .unwrap();
    assert!(f.target.publish_restore(&sealed, &ids, &|| false).is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    for meeting in read_meetings(sealed.connection()).unwrap() {
        assert!(!f.target.root.join("meetings").join(meeting.id).exists());
    }
    f.target
        .connection
        .lock()
        .unwrap()
        .commit_hook(None::<fn() -> bool>)
        .unwrap();
    triggers_enabled(&f.target);
    assert!(matches!(
        f.target.publish_restore(&sealed, &ids, &|| false).unwrap(),
        Publication::Published { .. }
    ));
    // An uncertain outcome must keep maintenance fail-closed, even if SQLite
    // has already returned to autocommit and a rollback error was inconclusive.
    let maintenance = f.target.backup_maintenance().unwrap();
    maintenance.fail_closed();
    drop(maintenance);
    assert!(f.target.maintenance.load(Ordering::Acquire));
    assert!(f
        .target
        .create_meeting("uncertain outcome must block writes")
        .is_err());
}

#[test]
fn restore_append_cancel_after_immutable_children_rolls_back_and_preserves_old_rows() {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    let mut db = f.target.connection.lock().unwrap();
    let before = all_rows(&db);
    let old: i64 = db
        .query_row("SELECT count(*) FROM utterances", [], |r| r.get(0))
        .unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;").unwrap();
    let error = append(&tx, sealed.connection(), &|| {
        tx.query_row("SELECT count(*) FROM utterances", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
            > old
    })
    .unwrap_err();
    assert!(error.contains("中断"));
    assert!(tx
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
        .unwrap());
    tx.rollback().unwrap();
    assert_eq!(all_rows(&db), before);
}

#[test]
fn restore_publication_rechecks_new_provenance_ids_across_domains_without_retaining_stale_probe_ids(
) {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    let origin: String = sealed
        .connection()
        .query_row("SELECT id FROM restore_provenance LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    f.target.connection.lock().unwrap().execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at,lock_version) VALUES(?1,'later meeting','draft',0,0,0)",params![origin]).unwrap();
    let before = all_rows(&f.target.connection.lock().unwrap());
    assert!(f
        .target
        .publish_restore(&sealed, &ids, &|| false)
        .unwrap_err()
        .contains("復元由来ID"));
    assert_eq!(all_rows(&f.target.connection.lock().unwrap()), before);
    f.target
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM meetings WHERE id=?1", params![origin])
        .unwrap();
    assert!(matches!(
        f.target.publish_restore(&sealed, &ids, &|| false).unwrap(),
        Publication::Published { .. }
    ));
}

#[cfg(unix)]
#[test]
fn restore_install_detects_late_links_and_cleanup_preserves_outside_and_unknown_files() {
    let f = fixture();
    let ids = ids(&f, &Uuid::new_v4().to_string());
    let sealed = sealed(&f, &ids);
    let outside = tempfile::tempdir().unwrap();
    let install = Installed::prepare(&f.target.root, &sealed, &|| false).unwrap();
    let relative: String = sealed
        .connection()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let target = f.target.root.join(relative);
    let hardlink = outside.path().join("external");
    std::fs::hard_link(&target, &hardlink).unwrap();
    let extra = target.parent().unwrap().join("user.txt");
    std::fs::write(&extra, b"unknown").unwrap();
    assert!(install.verify(&|| false).unwrap_err().contains("リンク数"));
    let bytes = std::fs::read(&hardlink).unwrap();
    drop(install);
    assert!(!target.exists());
    assert_eq!(std::fs::read(&hardlink).unwrap(), bytes);
    assert_eq!(std::fs::read(&extra).unwrap(), b"unknown");
    // Cleanup must also leave a substituted inode at an operation-owned path.
    let clean = tempfile::tempdir().unwrap();
    let target_storage = Storage::open(clean.path().into()).unwrap();
    let install = Installed::prepare(&target_storage.root, &sealed, &|| false).unwrap();
    let relative: String = sealed
        .connection()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let substituted = target_storage.root.join(relative);
    std::fs::rename(&substituted, outside.path().join("owned-moved")).unwrap();
    std::fs::write(&substituted, b"replacement").unwrap();
    assert!(install.verify(&|| false).is_err());
    drop(install);
    assert_eq!(std::fs::read(&substituted).unwrap(), b"replacement");
}
