use super::super::super::tests::read;
use super::*;
use std::cell::Cell;

fn fixture(storage: &Storage, operation: &str) -> String {
    let meeting = super::super::super::super::archive::tests::meeting(storage, operation);
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let speaker = format!("manual:{}", Uuid::new_v4());
    draft.payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    draft
        .payload
        .manual_speakers
        .push(crate::storage::drafts::ManualSpeaker {
            id: speaker.clone(),
            label: "手動話者".into(),
        });
    draft
        .payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: analysis.utterances[0].id.clone(),
            effective_text: format!("元IDを本文に保持 {}", analysis.utterances[0].id),
            speaker_ids: vec![speaker],
        });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let global = storage.vocabulary_book(None).unwrap();
    if global.payload.is_empty() {
        storage
            .save_vocabulary_book(
                None,
                global.lock_version,
                vec![VocabularyEntry {
                    id: Uuid::new_v4().to_string(),
                    term: "設計審査".into(),
                    reading: None,
                    note: None,
                    enabled: true,
                }],
            )
            .unwrap();
    }
    meeting
}

fn prepare<'p, 's>(fixed: &FixedData<'p, 's>, output: &Path) -> FixedWorkspace {
    FixedWorkspace::prepare(fixed, output, &crate::test_support::never_cancelled)
        .unwrap()
        .with_speech_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_generation_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_processing_history(fixed, &crate::test_support::never_cancelled)
        .unwrap()
}

#[test]
fn restore_provenance_records_every_scoped_mapping_and_keeps_old_keys_out_of_primary_keys() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = fixture(&storage, "origins-a");
    let b = fixture(&storage, "origins-b");
    let original = storage.load_transcript_draft(&a).unwrap();
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut work = prepare(&fixed, output.path())
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let mut count = 0;
    ids.visit_mappings(&||false,|map| {
        if map.domain=="restore_provenance" {return Ok(());}
        let row:(Option<String>,String,String,String)=work.database.query_row("SELECT meeting_id,entity_key,source_archive,restore_batch FROM restore_provenance WHERE domain=?1 AND source_scope=?2 AND source_key=?3",params![map.domain,map.scope,map.source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        let meeting=if map.domain=="meetings" {Some(map.target.clone())} else if map.scope.is_empty() {None} else {Some(ids.get("meetings","",&map.scope).unwrap())};
        assert_eq!(row,(meeting,map.target.clone(),ids.archive_id.clone(),ids.batch_id.clone()));
        assert_ne!(map.source,map.target);
        count+=1;
        Ok(())
    }).unwrap();
    assert_eq!(
        work.database
            .query_row("SELECT count(*) FROM restore_provenance", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
    assert!(count > 30);
    for old in [&a, &b] {
        assert_eq!(
            work.database
                .query_row(
                    "SELECT count(*) FROM meetings WHERE id=?1",
                    params![old],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    assert!(work.database.query_row("SELECT EXISTS(SELECT 1 FROM restore_provenance WHERE meeting_id IS NULL AND domain='vocabulary_entries')",[],|r|r.get::<_,bool>(0)).unwrap());
    super::super::super::super::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(storage.load_transcript_draft(&a).unwrap(), original);
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
    let stage = work.stage.path().to_path_buf();
    drop(work);
    assert!(!stage.exists());
}

#[test]
fn restore_provenance_survives_another_archive_and_restore_without_copying_local_batch_control() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let old = fixture(&storage, "again");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepare(&fixed, output.path())
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let first = ids.get("meetings", "", &old).unwrap();
    // Test-only access to the owned image: this is not a publication path.
    let owned_connection = Connection::open(work.stage.path().join("fixed.sqlite3")).unwrap();
    let owned = Storage {
        meeting_owners: Default::default(),
        deletion_fence: crate::storage::deletion::fence::Fence::open(&owned_connection).unwrap(),
        #[cfg(any(unix, windows))]
        managed_anchor: crate::storage::deletion::fs::RootAnchor::owned_image(
            work.stage.path(),
            "fixed.sqlite3",
        )
        .unwrap(),
        #[cfg(any(target_os = "macos", windows))]
        _reset_guard: crate::bootstrap::reset::RootLease::acquire(
            &work.stage.path().canonicalize().unwrap(),
            false,
        )
        .unwrap(),
        scratch: None,
        recovery_hold: false,
        #[cfg(any(target_os = "macos", windows))]
        flac_history: None,
        #[cfg(any(unix, windows))]
        native_models_root: work.stage.path().join("models"),
        root: work.stage.path().canonicalize().unwrap(),
        connection: Mutex::new(owned_connection),
        restored_inputs: Mutex::default(),
        feature_vectors: Mutex::default(),
        execution_progress: Mutex::default(),
        post_analysis_drain: Mutex::default(),
        maintenance: std::sync::atomic::AtomicBool::new(false),
        retired: std::sync::atomic::AtomicBool::new(false),
    };
    let plan = super::super::super::super::ledger::BatchPlan::new(
        &ids,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    {
        let mut db = owned.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        plan.reserve(&mut tx, &crate::test_support::never_cancelled)
            .unwrap();
        tx.commit().unwrap();
    }
    let next = read(&owned, output.path());
    assert_eq!(
        next.database
            .query_row("SELECT count(*) FROM restore_batches", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        next.database
            .query_row("SELECT count(*) FROM restore_identity_maps", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(next
        .manifest
        .tables
        .iter()
        .any(|t| t.name == "restore_provenance" && t.row_count > 0));
    let next_ids = RestoreIds::prepare(
        &next,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let next_fixed = FixedData::new(
        &next_ids,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut next_work = prepare(&next_fixed, output.path())
        .with_provenance(&next_fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let second = next_ids.get("meetings", "", &first).unwrap();
    assert_ne!(first, second);
    assert_ne!(old, second);
    let origins:Vec<(String,String,String)>=next_work.database.prepare("SELECT entity_key,source_archive,source_key FROM restore_provenance WHERE meeting_id=?1 AND domain='meetings' ORDER BY source_key").unwrap().query_map(params![second],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert!(origins.contains(&(second.clone(), ids.archive_id.clone(), old)));
    assert!(origins.contains(&(second.clone(), next_ids.archive_id.clone(), first)));
    assert_eq!(origins.len(), 2);
    super::super::super::super::validate_database(
        &mut next_work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(next_work
        .with_provenance(&next_fixed, &crate::test_support::never_cancelled)
        .is_err());
}

#[test]
fn restore_provenance_cancellation_after_private_insert_discards_only_owned_stage() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "cancel-origin");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepare(&fixed, output.path());
    let stage = work.stage.path().to_path_buf();
    let keep = output.path().join("keep.txt");
    std::fs::write(&keep, b"keep").unwrap();
    let calls = Cell::new(0);
    assert!(work
        .with_provenance(&fixed, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 8
        })
        .is_err());
    assert!(!stage.exists());
    assert_eq!(std::fs::read(keep).unwrap(), b"keep");
    assert!(storage.get_meeting(&meeting).unwrap().is_some());
}

#[test]
fn restore_provenance_keeps_duplicate_historical_origins_for_separate_meetings_and_deleted_entity_keys(
) {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = storage.create_meeting("別操作で追加した会議1").unwrap();
    let b = storage.create_meeting("別操作で追加した会議2").unwrap();
    let original = Uuid::new_v4().to_string();
    let deleted = Uuid::new_v4().to_string();
    let archive = Uuid::new_v4().to_string();
    let batch = Uuid::new_v4().to_string();
    {
        let db = storage.connection.lock().unwrap();
        for meeting in [&a.id, &b.id] {
            db.execute(
                "INSERT INTO restore_provenance VALUES(?1,?2,'meetings',?2,?3,'',?4,?5,1)",
                params![
                    Uuid::new_v4().to_string(),
                    meeting,
                    archive,
                    original,
                    batch
                ],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO restore_provenance VALUES(?1,?2,'utterances',?3,?4,?5,?3,?6,1)",
            params![
                Uuid::new_v4().to_string(),
                a.id,
                deleted,
                archive,
                original,
                batch
            ],
        )
        .unwrap();
    }
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut work = prepare(&fixed, output.path())
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let new_a = ids.get("meetings", "", &a.id).unwrap();
    let new_b = ids.get("meetings", "", &b.id).unwrap();
    assert_ne!(new_a, new_b);
    assert_eq!(work.database.query_row("SELECT count(*) FROM restore_provenance WHERE restore_batch=?1 AND domain='meetings' AND source_key=?2",params![batch,original],|r|r.get::<_,i64>(0)).unwrap(),2);
    let new_deleted = ids.get("utterances", &a.id, &deleted).unwrap();
    assert_ne!(new_deleted, deleted);
    assert_eq!(
        work.database
            .query_row(
                "SELECT count(*) FROM utterances WHERE id=?1",
                params![new_deleted],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(work.database.query_row("SELECT count(*) FROM restore_provenance WHERE restore_batch=?1 AND entity_key=?2 AND source_key=?3",params![batch,new_deleted,deleted],|r|r.get::<_,i64>(0)).unwrap(),1);
    super::super::super::super::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
}

#[test]
fn restore_provenance_rejects_unknown_domains_cross_meeting_and_changed_prior_history() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = fixture(&storage, "bad-origin-a");
    let b = fixture(&storage, "bad-origin-b");
    let track = storage.recorded_audio_plan(&a).unwrap().tracks[0]
        .id
        .clone();
    let archive = Uuid::new_v4().to_string();
    let batch = Uuid::new_v4().to_string();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "INSERT INTO restore_provenance VALUES(?1,?2,'audio_tracks',?3,?4,?2,?3,?5,1)",
            params![Uuid::new_v4().to_string(), a, track, archive, batch],
        )
        .unwrap();
    }
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepare(&fixed, output.path());
    let stage = work.stage.path().to_path_buf();
    work.database
        .execute("UPDATE restore_provenance SET source_key='changed'", [])
        .unwrap();
    assert!(work
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .is_err());
    assert!(!stage.exists());
    let filename: String = source
        .database
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Path::new(&filename).starts_with(output.path().canonicalize().unwrap()));
    let writer = Connection::open(filename).unwrap();
    writer
        .execute("UPDATE restore_provenance SET meeting_id=?1", params![b])
        .unwrap();
    assert!(
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
            false
        })
        .is_err()
    );
    writer
        .execute(
            "UPDATE restore_provenance SET meeting_id=?1,domain='injected'",
            params![a],
        )
        .unwrap();
    assert!(
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
            false
        })
        .is_err()
    );
    assert_eq!(
        source
            .database
            .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(storage
        .create_backup_archive(
            &output.path().join("original.monbackup"),
            &crate::test_support::never_cancelled
        )
        .is_ok());
}
