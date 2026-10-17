use super::*;
use crate::storage::backup::ledger::BatchPlan;
use crate::storage::backup::restore_ids::tests::fixture;
use std::cell::Cell;

struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    output: tempfile::TempDir,
    source: VerifiedArchive,
    batch: String,
}
fn fixture_with_archive() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "replay-first");
    fixture(&storage, "replay-second");
    let archive = output.path().join("source.monbackup");
    storage
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(archive).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    Fixture {
        _root: root,
        storage,
        output,
        source,
        batch: Uuid::new_v4().to_string(),
    }
}
fn pending<'a>(result: Result<BatchPreparation<'a>, String>) -> RestoreIds<'a> {
    match result.unwrap() {
        BatchPreparation::Pending(ids) => ids,
        BatchPreparation::AlreadyPublished { .. } => panic!("unexpected terminal batch"),
    }
}
fn mappings(ids: &RestoreIds<'_>) -> Vec<IdentityMap> {
    let mut all = vec![];
    ids.visit_mappings(&crate::test_support::never_cancelled, |map| {
        all.push(map);
        Ok(())
    })
    .unwrap();
    all
}
fn reserve(f: &Fixture, db: &mut Connection) -> Vec<IdentityMap> {
    let ids = pending(RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &f.batch,
        db,
        &crate::test_support::never_cancelled,
    ));
    assert!(!ids.reused());
    let maps = mappings(&ids);
    let mut tx = db.transaction().unwrap();
    BatchPlan::new(&ids, &crate::test_support::never_cancelled)
        .unwrap()
        .reserve(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.commit().unwrap();
    maps
}
fn error(f: &Fixture, db: &Connection) -> String {
    RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &f.batch,
        db,
        &crate::test_support::never_cancelled,
    )
    .err()
    .expect("must reject, not mint replacement IDs")
}
fn rehash(db: &Connection, batch: &str) {
    let mut hash = Sha256::new();
    let mut count = 0_i64;
    let mut statement=db.prepare("SELECT domain,source_scope,source_key,target_key FROM restore_identity_maps WHERE batch_id=?1 ORDER BY domain,source_scope,source_key").unwrap();
    let mut rows = statement.query(params![batch]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let map = IdentityMap {
            domain: row.get(0).unwrap(),
            scope: row.get(1).unwrap(),
            source: row.get(2).unwrap(),
            target: row.get(3).unwrap(),
        };
        hash.update(serde_json::to_vec(&map).unwrap());
        hash.update(b"\n");
        count += 1;
    }
    db.execute(
        "UPDATE restore_batches SET mapping_hash=?1,mapping_count=?2 WHERE batch_id=?3",
        params![format!("{:x}", hash.finalize()), count, batch],
    )
    .unwrap();
}

#[test]
fn persisted_ids_replay_after_closing_source_destination_and_private_registry() {
    let f = fixture_with_archive();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let existing = target.create_meeting("既存会議を維持").unwrap();
    let before = f.storage.list_meetings().unwrap();
    let original = reserve(&f, &mut target.connection.lock().unwrap());
    assert!(original.iter().any(|m| m.target.starts_with("manual:")));
    assert!(original.iter().any(|m| m.target.starts_with("split:")));
    let Fixture {
        output,
        source,
        batch,
        ..
    } = f;
    drop(source);
    drop(target);
    let source = archive::read(
        &mut File::open(output.path().join("source.monbackup")).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let mut db = target.connection.lock().unwrap();
    let ids = pending(RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &db,
        &crate::test_support::never_cancelled,
    ));
    assert!(ids.reused());
    assert_eq!(mappings(&ids), original);
    let stage = ids.stage.path().to_path_buf();
    let mut tx = db.transaction().unwrap();
    assert_eq!(
        BatchPlan::new(&ids, &crate::test_support::never_cancelled)
            .unwrap()
            .reserve(&mut tx, &crate::test_support::never_cancelled)
            .unwrap(),
        BatchState::Prepared
    );
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM restore_identity_maps", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        original.len() as i64
    );
    assert_eq!(
        source
            .database
            .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        source
            .database
            .query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before.len() as i64
    );
    drop(ids);
    assert!(!stage.exists());
    drop(db);
    assert_eq!(target.list_meetings().unwrap(), vec![existing]);
}

#[test]
fn published_batch_is_terminal_even_when_its_meetings_have_been_deleted() {
    let f = fixture_with_archive();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    reserve(&f, &mut db);
    // Ledger policy only: publication itself is a later atomic operation.
    db.execute(
        "UPDATE restore_batches SET state='published',published_at=1",
        [],
    )
    .unwrap();
    for _ in 0..2 {
        match RestoreIds::prepare_for_batch(
            &f.source,
            f.output.path(),
            &f.batch,
            &db,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
        {
            BatchPreparation::AlreadyPublished {
                batch_id,
                archive_id,
            } => {
                assert_eq!(batch_id, f.batch);
                assert_eq!(archive_id, f.source.manifest.archive_id);
            }
            BatchPreparation::Pending(_) => panic!("published batch must never append again"),
        }
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn explicit_new_batch_uses_distinct_ids_and_reserves_other_pending_batches() {
    let f = fixture_with_archive();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    let first = reserve(&f, &mut db);
    let batch = Uuid::new_v4().to_string();
    let ids = pending(RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &batch,
        &db,
        &crate::test_support::never_cancelled,
    ));
    assert!(!ids.reused());
    for map in mappings(&ids) {
        assert!(!first.iter().any(|old| old.target == map.target));
    }
    for map in &first {
        assert!(ids
            .database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1)",
                params![map.target],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
    let mut tx = db.transaction().unwrap();
    BatchPlan::new(&ids, &crate::test_support::never_cancelled)
        .unwrap()
        .reserve(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM restore_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn replay_rejects_missing_or_changed_maps_even_if_corruption_rehashes_the_header() {
    let f = fixture_with_archive();
    for change in [
        "missing",
        "extra",
        "scope",
        "domain",
        "prefix",
        "same-target",
        "source-id",
        "wrong-alias",
    ] {
        let mut db = portable::schema(MIGRATIONS.len()).unwrap();
        let original = reserve(&f, &mut db);
        // Simulate corruption only in an owned destination fixture, not source.
        db.execute_batch("DROP TRIGGER restore_map_fixed; DROP TRIGGER restore_map_retained; DROP TRIGGER restore_batch_fixed;").unwrap();
        let meeting = original.iter().find(|m| m.domain == "meetings").unwrap();
        match change {
            "missing" => {
                db.execute(
                    "DELETE FROM restore_identity_maps WHERE domain='meetings' AND source_key=?1",
                    params![meeting.source],
                )
                .unwrap();
            }
            "extra" => {
                db.execute("INSERT INTO restore_identity_maps VALUES(?1,'operation',?2,'unused-history',?3)",params![f.batch,meeting.source,Uuid::new_v4().to_string()]).unwrap();
            }
            "scope" => {
                db.execute(
                    "UPDATE restore_identity_maps SET source_scope=?1 WHERE domain='meetings'",
                    params![Uuid::new_v4().to_string()],
                )
                .unwrap();
            }
            "domain" => {
                db.execute("UPDATE restore_identity_maps SET domain='app_settings' WHERE domain='meetings'",[]).unwrap();
            }
            "prefix" => {
                db.execute("UPDATE restore_identity_maps SET target_key=?1 WHERE domain='speakers' AND source_key LIKE 'manual:%'",params![Uuid::new_v4().to_string()]).unwrap();
            }
            "same-target" => {
                db.execute(
                    "UPDATE restore_identity_maps SET target_key=?1 WHERE domain='meetings'",
                    params![meeting.target],
                )
                .unwrap();
            }
            "source-id" => {
                db.execute("UPDATE restore_identity_maps SET target_key=source_key WHERE domain='meetings'",[]).unwrap();
            }
            "wrong-alias" => {
                db.execute("UPDATE restore_identity_maps SET target_key=?1 WHERE domain='speaker_lineage' AND source_key LIKE 'manual:%'",params![format!("manual:{}",Uuid::new_v4())]).unwrap();
            }
            _ => unreachable!(),
        }
        rehash(&db, &f.batch);
        let failure = error(&f, &db);
        assert!(!failure.is_empty(), "{change}");
        assert_eq!(
            db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn replay_rejects_live_destination_id_collision_and_archive_fingerprint_change() {
    let mut f = fixture_with_archive();
    let root = tempfile::tempdir().unwrap();
    let target = Storage::open(root.path().into()).unwrap();
    let before = target.create_meeting("既存を変更しない").unwrap();
    let mut db = target.connection.lock().unwrap();
    let original = reserve(&f, &mut db);
    let meeting = original.iter().find(|m| m.domain == "meetings").unwrap();
    db.execute_batch("DROP TRIGGER restore_map_fixed; DROP TRIGGER restore_batch_fixed;")
        .unwrap();
    db.execute(
        "UPDATE restore_identity_maps SET target_key=?1 WHERE domain='meetings' AND source_key=?2",
        params![before.id, meeting.source],
    )
    .unwrap();
    rehash(&db, &f.batch);
    assert!(error(&f, &db).contains("競合"));
    f.source.manifest.created_at += 1;
    assert!(error(&f, &db).contains("アーカイブ"));
    drop(db);
    assert_eq!(target.list_meetings().unwrap(), vec![before]);
}

#[test]
fn replay_checks_digest_count_and_orphan_header_before_assigning_ids() {
    let f = fixture_with_archive();
    for change in ["sha", "count", "huge-count", "header"] {
        let mut db = portable::schema(MIGRATIONS.len()).unwrap();
        reserve(&f, &mut db);
        db.execute_batch("DROP TRIGGER restore_batch_fixed; DROP TRIGGER restore_batch_retained; PRAGMA foreign_keys=OFF;").unwrap();
        match change {
            "sha" => {
                db.execute(
                    "UPDATE restore_batches SET mapping_hash=?1",
                    params!["f".repeat(64)],
                )
                .unwrap();
            }
            "count" => {
                db.execute(
                    "UPDATE restore_batches SET mapping_count=mapping_count+1",
                    [],
                )
                .unwrap();
            }
            "huge-count" => {
                db.execute(
                    "UPDATE restore_batches SET mapping_count=9223372036854775807",
                    [],
                )
                .unwrap();
            }
            "header" => {
                db.execute("DELETE FROM restore_batches", []).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(!error(&f, &db).is_empty(), "{change}");
    }
}

#[test]
fn replay_cancellation_preserves_reservation_and_cleans_only_owned_registry() {
    let f = fixture_with_archive();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    let original = reserve(&f, &mut db);
    let before = std::fs::read_dir(f.output.path()).unwrap().count();
    let calls = Cell::new(0);
    assert!(
        RestoreIds::prepare_for_batch(&f.source, f.output.path(), &f.batch, &db, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 200
        })
        .is_err()
    );
    assert!(calls.get() > 200);
    assert_eq!(std::fs::read_dir(f.output.path()).unwrap().count(), before);
    assert!(
        RestoreIds::prepare_for_batch(&f.source, f.output.path(), &f.batch, &db, &|| true).is_err()
    );
    let ids = pending(RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &f.batch,
        &db,
        &crate::test_support::never_cancelled,
    ));
    assert_eq!(mappings(&ids), original);
}

#[test]
fn replay_rechecks_destination_maps_if_changed_while_source_is_being_inspected() {
    let f = fixture_with_archive();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    reserve(&f, &mut db);
    let calls = Cell::new(0);
    drop(pending(RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &f.batch,
        &db,
        &|| {
            calls.set(calls.get() + 1);
            false
        },
    )));
    let threshold = calls.get() / 2;
    calls.set(0);
    db.execute_batch("DROP TRIGGER restore_map_fixed;").unwrap();
    let result = RestoreIds::prepare_for_batch(&f.source, f.output.path(), &f.batch, &db, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == threshold {
            db.execute(
                "UPDATE restore_identity_maps SET target_key=?1 WHERE domain='meetings'",
                params![Uuid::new_v4().to_string()],
            )
            .unwrap();
        }
        false
    });
    assert!(calls.get() >= threshold);
    assert!(result.is_err());
}

#[test]
fn replay_rejects_collision_with_a_destination_manual_speaker_existing_only_in_json() {
    let f = fixture_with_archive();
    let root = tempfile::tempdir().unwrap();
    let target = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&target, "destination-manual");
    let draft = target.load_transcript_draft(&meeting).unwrap().unwrap();
    let manual = &draft.payload.manual_speakers[0].id;
    let mut db = target.connection.lock().unwrap();
    reserve(&f, &mut db);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM speakers WHERE id=?1",
            params![manual],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER restore_map_fixed; DROP TRIGGER restore_batch_fixed;")
        .unwrap();
    db.execute("UPDATE restore_identity_maps SET target_key=?1 WHERE domain IN ('speakers','speaker_lineage') AND source_key LIKE 'manual:%'",params![manual]).unwrap();
    rehash(&db, &f.batch);
    assert!(error(&f, &db).contains("競合"));
    drop(db);
    assert_eq!(
        target.load_transcript_draft(&meeting).unwrap().unwrap(),
        draft
    );
}

#[test]
fn published_replay_validates_maps_and_prepared_state_changed_during_replay_is_rejected() {
    let f = fixture_with_archive();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    reserve(&f, &mut db);
    let calls = Cell::new(0);
    drop(pending(RestoreIds::prepare_for_batch(
        &f.source,
        f.output.path(),
        &f.batch,
        &db,
        &|| {
            calls.set(calls.get() + 1);
            false
        },
    )));
    let threshold = calls.get() / 2;
    calls.set(0);
    let result = RestoreIds::prepare_for_batch(&f.source, f.output.path(), &f.batch, &db, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == threshold {
            db.execute(
                "UPDATE restore_batches SET state='published',published_at=1",
                [],
            )
            .unwrap();
        }
        false
    });
    assert!(result.err().unwrap().contains("変更"));
    db.execute_batch("DROP TRIGGER restore_map_fixed;").unwrap();
    db.execute(
        "UPDATE restore_identity_maps SET target_key=?1 WHERE domain='meetings'",
        params![Uuid::new_v4().to_string()],
    )
    .unwrap();
    // A published state never bypasses integrity inspection of the saved map.
    assert!(error(&f, &db).contains("SHA"));
}
