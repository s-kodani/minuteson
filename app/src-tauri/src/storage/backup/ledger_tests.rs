use super::super::restore_ids::tests::fixture;
use super::*;
use std::cell::Cell;

fn read(storage: &Storage, output: &Path) -> archive::VerifiedArchive {
    let path = output.join("ledger.monbackup");
    storage.create_backup_archive(&path, &|| false).unwrap();
    archive::read(&mut File::open(path).unwrap(), output, &|| false).unwrap()
}

#[test]
fn restore_ledger_migrates_v20_without_changing_meetings_audio_edits_or_vocabulary() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let database =
        portable::initialize(Connection::open(root.join("meetings.sqlite3")).unwrap(), 20).unwrap();
    let storage = Storage {
        deletion_fence: crate::storage::deletion::fence::Fence::open(&database).unwrap(),
        #[cfg(unix)]
        managed_anchor: crate::storage::deletion::fs::RootAnchor::owned_image(
            &root,
            "meetings.sqlite3",
        )
        .unwrap(),
        scratch: None,
        #[cfg(unix)]
        native_models_root: root.join("models"),
        root: root.clone(),
        connection: Mutex::new(database),
        maintenance: std::sync::atomic::AtomicBool::new(false),
        retired: std::sync::atomic::AtomicBool::new(false),
    };
    let meeting = fixture(&storage, "v20");
    let original = storage.get_meeting(&meeting).unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap();
    let minutes = storage
        .minutes_versions(&meeting)
        .unwrap()
        .current_id
        .unwrap();
    let content = storage.load_minutes_content(&meeting, &minutes).unwrap();
    let audio = storage.recorded_audio_plan(&meeting).unwrap();
    let vocabulary =
        serde_json::to_value(storage.vocabulary_book(Some(&meeting)).unwrap()).unwrap();
    drop(storage);
    let storage = Storage::open(root).unwrap();
    assert_eq!(storage.get_meeting(&meeting).unwrap(), original);
    assert_eq!(storage.load_transcript_draft(&meeting).unwrap(), draft);
    assert_eq!(
        serde_json::to_value(storage.load_minutes_content(&meeting, &minutes).unwrap()).unwrap(),
        serde_json::to_value(content).unwrap()
    );
    assert_eq!(
        serde_json::to_value(storage.recorded_audio_plan(&meeting).unwrap()).unwrap(),
        serde_json::to_value(audio).unwrap()
    );
    assert_eq!(
        serde_json::to_value(storage.vocabulary_book(Some(&meeting)).unwrap()).unwrap(),
        vocabulary
    );
    let db = storage.connection.lock().unwrap();
    assert_eq!(verify_migration_history(&db).unwrap(), MIGRATIONS.len());
    assert_eq!(portable::tables(&db).unwrap().len(), 32);
    assert_eq!(
        db.query_row("SELECT count(*) FROM restore_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn restore_ledger_reservation_survives_reopen_and_same_plan_never_duplicates_or_touches_existing_meetings(
) {
    let source_root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source_storage = Storage::open(source_root.path().into()).unwrap();
    fixture(&source_storage, "reserve");
    let source = read(&source_storage, output.path());
    let batch = Uuid::new_v4().to_string();
    let ids = RestoreIds::prepare(&source, output.path(), &batch, &|| false).unwrap();
    let plan = BatchPlan::new(&ids, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let existing = target.create_meeting("既存会議").unwrap();
    {
        let mut db = target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        assert_eq!(
            plan.reserve(&mut tx, &|| false).unwrap(),
            BatchState::Prepared
        );
        assert_eq!(
            plan.reserve(&mut tx, &|| false).unwrap(),
            BatchState::Prepared
        );
        tx.commit().unwrap();
    }
    drop(target);
    let target = Storage::open(target_root.path().into()).unwrap();
    assert_eq!(target.list_meetings().unwrap(), vec![existing]);
    let mut db = target.connection.lock().unwrap();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        plan.reserve(&mut tx, &|| false).unwrap(),
        BatchState::Prepared
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM restore_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM restore_identity_maps", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        plan.count
    );
    assert!(tx
        .execute("UPDATE restore_identity_maps SET target_key='changed'", [])
        .is_err());
    assert!(tx.execute("DELETE FROM restore_batches", []).is_err());
    assert!(tx
        .execute("UPDATE restore_batches SET archive_id='changed'", [])
        .is_err());
    // Exercise only the ledger's terminal-state policy, not meeting publication.
    tx.execute(
        "UPDATE restore_batches SET state='published',published_at=1 WHERE batch_id=?1",
        params![batch],
    )
    .unwrap();
    assert_eq!(
        plan.reserve(&mut tx, &|| false).unwrap(),
        BatchState::Published
    );
    assert!(tx
        .execute(
            "UPDATE restore_batches SET state='prepared',published_at=NULL",
            []
        )
        .is_err());
    assert!(tx
        .execute(
            "INSERT INTO restore_identity_maps VALUES(?1,'operation','','other','other-new')",
            params![batch]
        )
        .is_err());
    tx.commit().unwrap();
}

#[test]
fn restore_ledger_rejects_same_batch_with_changed_mapping_archive_or_corrupted_persistent_map() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "different");
    let mut source = read(&storage, output.path());
    let batch = Uuid::new_v4().to_string();
    let ids = RestoreIds::prepare(&source, output.path(), &batch, &|| false).unwrap();
    let plan = BatchPlan::new(&ids, &|| false).unwrap();
    let mut target = portable::schema(MIGRATIONS.len()).unwrap();
    {
        let mut tx = target.transaction().unwrap();
        plan.reserve(&mut tx, &|| false).unwrap();
        tx.commit().unwrap();
    }
    let alternative = RestoreIds::prepare(&source, output.path(), &batch, &|| false).unwrap();
    let other = BatchPlan::new(&alternative, &|| false).unwrap();
    {
        let mut tx = target.transaction().unwrap();
        assert!(other.reserve(&mut tx, &|| false).is_err());
        tx.commit().unwrap();
    }
    target.execute_batch("DROP TRIGGER restore_map_fixed; UPDATE restore_identity_maps SET target_key='changed' WHERE rowid=(SELECT min(rowid) FROM restore_identity_maps);").unwrap();
    {
        let mut tx = target.transaction().unwrap();
        assert!(plan
            .reserve(&mut tx, &|| false)
            .unwrap_err()
            .contains("変更"));
        tx.commit().unwrap();
    }
    drop(other);
    drop(alternative);
    drop(plan);
    drop(ids);
    source.manifest.created_at += 1;
    let ids = RestoreIds::prepare(&source, output.path(), &batch, &|| false).unwrap();
    let changed = BatchPlan::new(&ids, &|| false).unwrap();
    let mut tx = target.transaction().unwrap();
    assert!(changed.reserve(&mut tx, &|| false).is_err());
    tx.commit().unwrap();
}

#[test]
fn restore_ledger_cancellation_rolls_back_its_savepoint_even_if_outer_transaction_commits() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "cancel");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let plan = BatchPlan::new(&ids, &|| false).unwrap();
    let mut target = portable::schema(MIGRATIONS.len()).unwrap();
    let mut tx = target.transaction().unwrap();
    let calls = Cell::new(0);
    assert!(plan
        .reserve(&mut tx, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 4
        })
        .is_err());
    tx.commit().unwrap();
    assert_eq!(
        target
            .query_row("SELECT count(*) FROM restore_batches", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        target
            .query_row("SELECT count(*) FROM restore_identity_maps", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(BatchPlan::new(&ids, &|| true).is_err());
}
