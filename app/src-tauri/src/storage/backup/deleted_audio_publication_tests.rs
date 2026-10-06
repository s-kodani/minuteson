//! Real additive-restore entry points with owned storage and synthetic audio.
use super::*;
use crate::storage::backup::{
    ledger::BatchPlan,
    publication::{self, Publication},
    restore_fixed::{workspace::seal::SealedWorkspace, FixedData},
    restore_ids::{BatchPreparation, RestoreIds},
    restored_audio::{operation_key, Seed},
};
use crate::storage::deletion::audio::restored;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn native_record(target: &Storage, meeting: &str) -> String {
    let db = target.connection.lock().unwrap();
    let op: String = db.query_row("SELECT id FROM maintenance_operations WHERE meeting_id=?1 AND kind='delete_assets' AND state='completed'", [meeting], |r|r.get(0)).unwrap();
    assert!(restored::deleted(&db, &op, meeting).is_ok());
    assert_eq!(
        restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
        None
    );
    assert_eq!(db.query_row("SELECT count(*) FROM restore_omitted_assets WHERE meeting_id=?1 AND reason='recreatable_pcm'", [meeting], |r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM maintenance_paths WHERE operation_id=?1",
            [&op],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    op
}

#[test]
fn deleted_audio_publication_selection_preview_is_read_only_replay_restart_and_three_reexports_keep_history(
) {
    let (root, mut source, mut meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    for round in 0..3 {
        let path = output.path().join(format!("round-{round}.monbackup"));
        let backup = source.create_backup_archive(&path, &|| false).unwrap();
        let target_root = root.path().join(format!("round-{round}"));
        let target = Storage::open(target_root.clone()).unwrap();
        let before = publication::tests::all_rows(&target.connection.lock().unwrap());
        let mut selected = target
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        let preview = selected.preview();
        assert_eq!(preview.deleted_audio_meeting_count, 1);
        assert_eq!(
            preview.deleted_audio_asset_count,
            backup.deleted_audio_asset_count
        );
        assert!(preview.deleted_audio_asset_count >= 2);
        assert_eq!(
            publication::tests::all_rows(&target.connection.lock().unwrap()),
            before
        );
        let refreshed = selected.refresh(&target, &|| false, &|_| {}).unwrap();
        assert_eq!(refreshed.batch_id, preview.batch_id);
        assert_ne!(refreshed.confirmation_id, preview.confirmation_id);
        assert!(selected
            .publish(&target, &preview.confirmation_id, &|| false, &|_| {})
            .is_err());
        let receipt = selected
            .publish(&target, &refreshed.confirmation_id, &|| false, &|_| {})
            .unwrap();
        assert!(!receipt.already_published);
        assert_eq!(receipt.deleted_audio_meeting_count, 1);
        assert_eq!(
            receipt.deleted_audio_asset_count,
            preview.deleted_audio_asset_count
        );
        let new = receipt.meeting_ids[0].clone();
        assert_ne!(new, meeting);
        assert!(target.recorded_audio_plan(&new).is_err());
        assert!(target.load_current_analysis(&new).unwrap().is_some());
        assert!(target.load_transcript_draft(&new).unwrap().is_some());
        assert!(target.acquire_local_inference_job().unwrap().is_none());
        let op = native_record(&target, &new);
        let published = publication::tests::all_rows(&target.connection.lock().unwrap());
        let replay = selected
            .publish(&target, &refreshed.confirmation_id, &|| false, &|_| {
                panic!("terminal replay must not rebuild")
            })
            .unwrap();
        assert!(replay.already_published);
        assert_eq!(replay.meeting_ids, receipt.meeting_ids);
        assert_eq!(
            replay.deleted_audio_asset_count,
            receipt.deleted_audio_asset_count
        );
        assert_eq!(replay.vocabulary_added, None);
        assert_eq!(
            publication::tests::all_rows(&target.connection.lock().unwrap()),
            published
        );
        drop(target);
        let target = Storage::open(target_root).unwrap();
        let replay = selected
            .publish(&target, &refreshed.confirmation_id, &|| false, &|_| {
                panic!("restart replay must not rebuild")
            })
            .unwrap();
        assert!(replay.already_published);
        assert_eq!(replay.meeting_ids, receipt.meeting_ids);
        assert_eq!(native_record(&target, &new), op);
        assert_eq!(target.list_meetings().unwrap().len(), 1);
        drop(selected);
        source = target;
        meeting = new;
    }
}

#[test]
fn deleted_audio_publication_mixed_archive_and_new_batches_preserve_available_neighbor_and_existing_data(
) {
    let (root, source, meeting) = fixture();
    let second = tests::meeting(&source, "second deleted source");
    let available = tests::meeting(&source, "available source");
    deleted(&source, &meeting);
    deleted(&source, &second);
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("mixed.monbackup");
    let backup = source.create_backup_archive(&path, &|| false).unwrap();
    let target = Storage::open(root.path().join("mixed-target")).unwrap();
    let existing = tests::meeting(&target, "keep existing");
    let before = target.recorded_audio_plan(&existing).unwrap();
    let draft = target.load_transcript_draft(&existing).unwrap();
    let mut batches = std::collections::BTreeSet::new();
    let mut meetings = std::collections::BTreeSet::new();
    for _ in 0..2 {
        let selected = target
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        let preview = selected.preview();
        assert_eq!(preview.deleted_audio_meeting_count, 2);
        assert_eq!(
            preview.deleted_audio_asset_count,
            backup.deleted_audio_asset_count
        );
        assert!(batches.insert(preview.batch_id.clone()));
        let receipt = selected
            .publish(&target, &preview.confirmation_id, &|| false, &|_| {})
            .unwrap();
        assert_eq!(receipt.meeting_ids.len(), 3);
        let mut deleted_count = 0;
        for id in receipt.meeting_ids {
            assert!(meetings.insert(id.clone()));
            let source_id: String = target.connection.lock().unwrap().query_row(
                "SELECT source_key FROM restore_identity_maps WHERE batch_id=?1 AND domain='meetings' AND source_scope='' AND target_key=?2",
                params![receipt.batch_id,id], |r| r.get(0),
            ).unwrap();
            if source_id == available {
                assert!(target.recorded_audio_plan(&id).is_ok());
            } else {
                native_record(&target, &id);
                assert!(target.recorded_audio_plan(&id).is_err());
                deleted_count += 1;
            }
        }
        assert_eq!(deleted_count, 2);
        assert_eq!(target.recorded_audio_plan(&existing).unwrap(), before);
        assert_eq!(target.load_transcript_draft(&existing).unwrap(), draft);
        assert!(target.acquire_local_inference_job().unwrap().is_none());
    }
    assert_eq!(target.list_meetings().unwrap().len(), 7);
    assert!(source.recorded_audio_plan(&available).is_ok());
    let (_, backup) = write(&target, output.path());
    assert_eq!(backup.deleted_audio_meeting_count, 4);
}

#[test]
fn deleted_audio_publication_late_cancellation_rolls_back_native_record_and_files_then_reuses_prepared_ids_after_restart(
) {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&source, output.path());
    let batch = Uuid::new_v4().to_string();
    let ids = RestoreIds::prepare(&archive, output.path(), &batch, &|| false).unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let new = ids.get("meetings", "", &meeting).unwrap();
    let op = ids
        .get(
            "operation",
            &meeting,
            &operation_key(&ids.archive_id, &meeting),
        )
        .unwrap();
    let target_root = root.path().join("cancel-target");
    let target = Storage::open(target_root.clone()).unwrap();
    let existing = tests::meeting(&target, "keep on late cancellation");
    let before = target.recorded_audio_plan(&existing).unwrap();
    let inserts = Arc::new(AtomicUsize::new(0));
    let seen = inserts.clone();
    // The first insert is the durable installation journal. The next is the
    // native unavailable marker, after rows and batch were appended in the TX.
    target
        .connection
        .lock()
        .unwrap()
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if table == "maintenance_operations"
                    && action == rusqlite::hooks::Action::SQLITE_INSERT
                {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            },
        ))
        .unwrap();
    let error = target
        .publish_restore(&sealed, &ids, &|| inserts.load(Ordering::SeqCst) >= 2)
        .unwrap_err();
    assert!(
        error.contains("音声利用不可の検証を取り消しました"),
        "{error}"
    );
    assert_eq!(inserts.load(Ordering::SeqCst), 2);
    target
        .connection
        .lock()
        .unwrap()
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    assert_eq!(target.recorded_audio_plan(&existing).unwrap(), before);
    assert!(target.get_meeting(&new).unwrap().is_none());
    assert!(!target.root.join("meetings").join(&new).exists());
    {
        let db = target.connection.lock().unwrap();
        assert!(restored::deleted(&db, &op, &new).is_err());
        assert_eq!(
            db.query_row(
                "SELECT state FROM restore_batches WHERE batch_id=?1",
                [&batch],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "prepared"
        );
    }
    drop(target);
    let target = Storage::open(target_root).unwrap();
    let replay = {
        let db = target.connection.lock().unwrap();
        match RestoreIds::prepare_for_batch(&archive, output.path(), &batch, &db, &|| false)
            .unwrap()
        {
            BatchPreparation::Pending(ids) => ids,
            _ => panic!("cancelled transaction must not publish"),
        }
    };
    assert_eq!(replay.get("meetings", "", &meeting).unwrap(), new);
    assert_eq!(
        replay
            .get(
                "operation",
                &meeting,
                &operation_key(&replay.archive_id, &meeting)
            )
            .unwrap(),
        op
    );
    let fixed = FixedData::new(&replay, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    assert!(matches!(
        target.publish_restore(&sealed, &replay, &|| false).unwrap(),
        Publication::Published { .. }
    ));
    assert_eq!(native_record(&target, &new), op);
    assert_eq!(target.list_meetings().unwrap().len(), 2);
}

#[test]
fn deleted_audio_publication_commit_failure_rolls_back_unavailable_marker_and_preserves_reservation_for_retry(
) {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&source, output.path());
    let ids = RestoreIds::prepare(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &|| false,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let target = Storage::open(root.path().join("commit-target")).unwrap();
    let existing = tests::meeting(&target, "keep on commit failure");
    let before = target.recorded_audio_plan(&existing).unwrap();
    {
        let mut db = target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        BatchPlan::new(&ids, &|| false)
            .unwrap()
            .reserve(&mut tx, &|| false)
            .unwrap();
        tx.commit().unwrap();
    }
    let ready = sealed
        .connection()
        .prepare("SELECT relative_path,size_bytes FROM file_assets WHERE state='ready'")
        .unwrap()
        .query_map([], |r| {
            Ok((
                target.root.join(r.get::<_, String>(0)?),
                r.get::<_, i64>(1)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!ready.is_empty());
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    target
        .connection
        .lock()
        .unwrap()
        .commit_hook(Some(move || {
            if ready
                .iter()
                .all(|(p, n)| std::fs::metadata(p).is_ok_and(|m| m.len() == *n as u64))
            {
                seen.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        }))
        .unwrap();
    assert!(target.publish_restore(&sealed, &ids, &|| false).is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    target
        .connection
        .lock()
        .unwrap()
        .commit_hook(None::<fn() -> bool>)
        .unwrap();
    let new = ids.get("meetings", "", &meeting).unwrap();
    assert!(target.get_meeting(&new).unwrap().is_none());
    assert!(!target.root.join("meetings").join(&new).exists());
    assert_eq!(target.recorded_audio_plan(&existing).unwrap(), before);
    assert_eq!(
        target
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM maintenance_operations WHERE kind='delete_assets'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        target.publish_restore(&sealed, &ids, &|| false).unwrap(),
        Publication::Published { .. }
    ));
    native_record(&target, &new);
}

#[test]
fn deleted_audio_publication_omissions_require_persisted_native_record_not_only_missing_rows() {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&source, output.path());
    let ids = RestoreIds::prepare(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &|| false,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let seed = Seed::prepare(&sealed, &ids, &|| false).unwrap();
    let target = Storage::open(root.path().join("omission-target")).unwrap();
    let mut db = target.connection.lock().unwrap();
    let mut tx = db.transaction().unwrap();
    BatchPlan::new(&ids, &|| false)
        .unwrap()
        .reserve(&mut tx, &|| false)
        .unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
    publication::append(&tx, sealed.connection(), &|| false).unwrap();
    tx.execute(
        "UPDATE restore_batches SET state='published',published_at=?1 WHERE batch_id=?2",
        params![crate::storage::now_ms().unwrap(), ids.batch_id],
    )
    .unwrap();
    assert!(crate::storage::backup::exclusions::retain(&tx, &ids, &|| false).is_err());
    assert_eq!(
        tx.query_row("SELECT count(*) FROM restore_omitted_assets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Missing rows, an otherwise valid published ledger and even a real sealed
    // seed do not stand in for the persisted typed native unavailable marker.
    assert!(restored::verify_retained(&tx, &target.managed_anchor, &seed, &|| false).is_err());
    tx.rollback().unwrap();
}
