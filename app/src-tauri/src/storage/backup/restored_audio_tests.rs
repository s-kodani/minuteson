//! Owned native databases and synthetic files. Test-only append exercises the
//! receipt boundary; real publication is exercised by publication_tests.
use super::*;
use crate::storage::backup::{
    ledger::BatchPlan,
    publication,
    restore_fixed::{workspace::seal::SealedWorkspace, FixedData},
    restore_ids::RestoreIds,
    restored_audio::Seed,
};
use crate::storage::deletion::audio::restored;
use rusqlite::{config::DbConfig, TransactionBehavior};

#[path = "restored_audio_reconfirmation_tests.rs"]
mod reconfirmation;

#[path = "restored_audio_controller_tests.rs"]
mod controller;

fn append_receipt(
    source: &Storage,
    target: &Storage,
    meeting: &str,
    commit: bool,
) -> (String, String) {
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(source, output.path());
    let ids = RestoreIds::prepare_historical(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let seed = Seed::prepare(&sealed, &ids, &crate::test_support::never_cancelled).unwrap();
    let new = ids.get("meetings", "", meeting).unwrap();
    let op = seed
        .meetings()
        .iter()
        .find(|m| m.source == meeting)
        .unwrap()
        .operation
        .clone();
    // Only owned temporary target paths; never run a source engine or delete.
    let mut q = sealed
        .connection()
        .prepare("SELECT relative_path,state FROM file_assets")
        .unwrap();
    let paths = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (path, state) in paths {
        let dest = target.root.join(&path);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        if state == "ready" {
            std::fs::copy(sealed.root_path().unwrap().join(path), dest).unwrap();
        }
    }
    let mut db = target.connection.lock().unwrap();
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    BatchPlan::new(&ids, &crate::test_support::never_cancelled)
        .unwrap()
        .reserve(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;").unwrap();
    publication::append(
        &tx,
        sealed.connection(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    sealed
        .global_merge(&tx, &crate::test_support::never_cancelled)
        .unwrap()
        .apply(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.execute(
        "UPDATE restore_batches SET state='published',published_at=?1 WHERE batch_id=?2",
        params![crate::storage::now_ms().unwrap(), ids.batch_id],
    )
    .unwrap();
    restored::retain(
        &tx,
        &target.managed_anchor,
        &seed,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(restored::deleted(&tx, &op, &new).is_ok());
    assert!(audio::require_source(&tx, &new).is_err());
    if commit {
        tx.commit().unwrap();
    } else {
        tx.rollback().unwrap();
    }
    (new, op)
}

fn native_fixture() -> (tempfile::TempDir, Storage, String, String) {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let target = Storage::open(root.path().join("target")).unwrap();
    let (new, op) = append_receipt(&source, &target, &meeting, true);
    (root, target, new, op)
}

fn inspect_mutation(target: &Storage, meeting: &str, op: &str, sql: &str, parameter: &str) {
    let mut db = target.connection.lock().unwrap();
    assert!(restored::deleted(&db, op, meeting).is_ok());
    let tx = db.transaction().unwrap();
    // Isolate each corruption in a rollback, preserving the positive baseline
    // and trusted schema without repeatedly building an unrelated fixture.
    tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    assert!(tx.execute(sql, [parameter]).unwrap() > 0);
    tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    assert!(restored::deleted(&tx, op, meeting).is_err(), "{sql}");
    assert_eq!(
        restored::inspect_one(&tx, &target.managed_anchor, op).unwrap(),
        Some("restored_audio_receipt_invalid"),
        "{sql}"
    );
    assert!(
        restored::backup_meeting(
            &tx,
            &target.managed_anchor,
            op,
            &crate::test_support::never_cancelled
        )
        .is_err(),
        "{sql}"
    );
    assert!(audio::require_source(&tx, meeting).is_err(), "{sql}");
    assert!(
        audio::backup_deleted(target, &tx, &crate::test_support::never_cancelled).is_err(),
        "{sql}"
    );
    tx.rollback().unwrap();
    assert!(
        restored::deleted(&db, op, meeting).is_ok(),
        "restored baseline: {sql}"
    );
}

#[test]
fn restored_audio_native_receipt_blocks_source_keeps_documents_and_reexports_without_consent() {
    let (root, mut target, mut meeting, mut op) = native_fixture();
    for round in 0..3 {
        assert!(target.recorded_audio_plan(&meeting).is_err());
        assert!(target.load_current_analysis(&meeting).unwrap().is_some());
        assert!(target.load_transcript_draft(&meeting).unwrap().is_some());
        {
            let db = target.connection.lock().unwrap();
            assert_eq!(
                restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
                None
            );
            let (consent,batch,paths):(Option<i64>,Option<String>,i64)=db.query_row("SELECT consent_at,restore_batch_id,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",[&op],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!((consent, batch, paths), (None, None, 0));
        }
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        assert!(audio::intent::removal::start(
            &target,
            &op,
            &lease,
            &crate::test_support::never_cancelled
        )
        .is_err());
        drop(lease);
        let path = target.root.clone();
        drop(target);
        target = Storage::open(path).unwrap();
        {
            let db = target.connection.lock().unwrap();
            assert_eq!(
                restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
                None
            );
            assert_eq!(
                audio::backup_deleted(&target, &db, &crate::test_support::never_cancelled).unwrap(),
                std::collections::BTreeSet::from([meeting.clone()])
            );
        }
        let output = tempfile::tempdir().unwrap();
        let (archive, receipt) = write(&target, output.path());
        assert_eq!(receipt.deleted_audio_meeting_count, 1);
        assert!(archive.deleted_audio.contains(&meeting));
        assert!(historical_plan(&archive, &meeting).is_ok());
        // Native root/DB and deletion permission are absent in the portable DB.
        assert_eq!(
            archive
                .database
                .query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let next = Storage::open(root.path().join(format!("round-{round}"))).unwrap();
        let (new, new_op) = append_receipt(&target, &next, &meeting, true);
        assert_ne!(new, meeting);
        assert_ne!(new_op, op);
        target = next;
        meeting = new;
        op = new_op;
    }
}

#[test]
fn restored_audio_native_record_rolls_back_with_meeting_and_published_batch() {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let target = Storage::open(root.path().join("rollback")).unwrap();
    let (new, op) = append_receipt(&source, &target, &meeting, false);
    let db = target.connection.lock().unwrap();
    for table in [
        "meetings",
        "restore_batches",
        "restore_identity_maps",
        "maintenance_operations",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert!(restored::deleted(&db, &op, &new).is_err());
}

#[test]
fn restored_audio_native_record_coexists_with_local_deletion_and_available_neighbor() {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let target = Storage::open(root.path().join("mixed")).unwrap();
    let local = tests::meeting(&target, "local native deleted audio");
    deleted(&target, &local);
    let neighbor = tests::meeting(&target, "available native neighbor");
    let before = target.recorded_audio_plan(&neighbor).unwrap();
    let (new, op) = append_receipt(&source, &target, &meeting, true);
    assert_eq!(target.recorded_audio_plan(&neighbor).unwrap(), before);
    assert!(target.recorded_audio_plan(&local).is_err());
    assert!(target.recorded_audio_plan(&new).is_err());
    let db = target.connection.lock().unwrap();
    assert_eq!(
        audio::backup_deleted(&target, &db, &crate::test_support::never_cancelled).unwrap(),
        std::collections::BTreeSet::from([local.clone(), new.clone()])
    );
    assert_eq!(
        restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
        None
    );
    drop(db);
    let output = tempfile::tempdir().unwrap();
    let (archive, receipt) = write(&target, output.path());
    assert_eq!(receipt.deleted_audio_meeting_count, 2);
    assert!(historical_plan(&archive, &local).is_ok());
    assert!(historical_plan(&archive, &new).is_ok());
    assert!(!archive.deleted_audio.contains(&neighbor));
    assert!(crate::storage::transcription::read_audio_plan(&archive.database, &neighbor).is_ok());
}

#[test]
fn restored_audio_native_record_rejects_mutable_flags_journal_or_ledger_corruption() {
    let (_root, target, meeting, op) = native_fixture();
    for sql in [
        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
        "UPDATE maintenance_operations SET consent_at=1 WHERE id=?1",
        "UPDATE maintenance_operations SET error_code='substituted' WHERE id=?1",
        "UPDATE maintenance_operations SET meeting_id=NULL WHERE id=?1",
        "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
        "UPDATE maintenance_operations SET target_hash=lower(hex(zeroblob(32))) WHERE id=?1",
        "UPDATE restore_identity_maps SET target_key='substituted' WHERE target_key=?1",
    ] {
        inspect_mutation(&target, &meeting, &op, sql, &op);
    }
    for sql in [
        "UPDATE audio_chunks SET frame_count=frame_count+1 WHERE meeting_id=?1",
        "UPDATE audio_chunks SET state='ready' WHERE meeting_id=?1",
        "UPDATE audio_tracks SET channels=channels+1 WHERE meeting_id=?1",
        "UPDATE audio_tracks SET state='ready' WHERE meeting_id=?1",
        "UPDATE file_assets SET size_bytes=size_bytes+1 WHERE meeting_id=?1 AND media_type LIKE 'audio/%'",
        "UPDATE file_assets SET state='ready' WHERE meeting_id=?1 AND media_type LIKE 'audio/%'",
    ] {
        inspect_mutation(&target,&meeting,&op,sql,&meeting);
    }
}

#[test]
fn restored_audio_native_startup_and_backup_reject_reappearance_and_parent_alias_without_removal() {
    for alias in [false, true] {
        let (root, target, meeting, op) = native_fixture();
        let path = audio_path(&target, &meeting);
        let outside = root.path().join("unrelated-audio");
        std::fs::write(&outside, b"retain unrelated").unwrap();
        if alias {
            std::os::unix::fs::symlink(&outside, &path).unwrap();
        } else {
            std::fs::write(&path, b"unexpected reappearance").unwrap();
        }
        {
            let db = target.connection.lock().unwrap();
            assert_eq!(
                restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
                Some(if alias {
                    "restored_audio_scope_uninspectable"
                } else {
                    "restored_audio_scope_reappeared"
                })
            );
            assert!(
                audio::backup_deleted(&target, &db, &crate::test_support::never_cancelled).is_err()
            );
        }
        let native = target.root.clone();
        drop(target);
        let target = Storage::open(native).unwrap();
        let db = target.connection.lock().unwrap();
        assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM temp.mon_deletion_startup_attention WHERE operation_id=?1)",[&op],|r|r.get::<_,bool>(0)).unwrap());
        assert_eq!(std::fs::read(&outside).unwrap(), b"retain unrelated");
        assert!(std::fs::symlink_metadata(path).is_ok());
    }
    let (root, target, meeting, op) = native_fixture();
    let parent = target.root.join(format!("meetings/{meeting}/restored"));
    let retained = parent.with_file_name("retained-owned");
    std::fs::rename(&parent, &retained).unwrap();
    let outside = root.path().join("unrelated-dir");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &parent).unwrap();
    let db = target.connection.lock().unwrap();
    assert_eq!(
        restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
        Some("restored_audio_scope_uninspectable")
    );
    assert!(restored::backup_meeting(
        &db,
        &target.managed_anchor,
        &op,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(retained.exists());
    assert!(outside.exists());
}

#[test]
fn restored_audio_seed_requires_intact_seal_omissions_and_historical_metadata() {
    let (_root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (mut archive, _) = write(&source, output.path());
    let ids = RestoreIds::prepare_historical(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    assert!(Seed::prepare(&sealed, &ids, &crate::test_support::never_cancelled).is_ok());
    assert!(Seed::prepare(&sealed, &ids, &|| true).is_err());
    let path = sealed.root_path().unwrap().join("unexpected");
    std::fs::write(&path, b"owned seal attack").unwrap();
    assert!(Seed::prepare(&sealed, &ids, &crate::test_support::never_cancelled).is_err());
    drop(sealed);
    drop(fixed);
    drop(ids);
    archive.manifest.omitted_assets.clear();
    assert!(RestoreIds::prepare_historical(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn restored_audio_native_operation_replays_from_prepared_ledger_after_restart() {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&source, output.path());
    let batch = Uuid::new_v4().to_string();
    let ids = RestoreIds::prepare_historical(
        &archive,
        output.path(),
        &batch,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let seed = Seed::prepare(&sealed, &ids, &crate::test_support::never_cancelled).unwrap();
    let op = seed.meetings()[0].operation.clone();
    let new = ids.get("meetings", "", &meeting).unwrap();
    let target = Storage::open(root.path().join("replay")).unwrap();
    {
        let mut db = target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        BatchPlan::new(&ids, &crate::test_support::never_cancelled)
            .unwrap()
            .reserve(&mut tx, &crate::test_support::never_cancelled)
            .unwrap();
        // A prepared ledger is not permission to persist a native marker.
        assert!(restored::retain(
            &tx,
            &target.managed_anchor,
            &seed,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert_eq!(
            tx.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        tx.commit().unwrap();
    }
    let path = target.root.clone();
    drop(target);
    let target = Storage::open(path).unwrap();
    let db = target.connection.lock().unwrap();
    let replay = RestoreIds::prepare_historical_for_batch(
        &archive,
        output.path(),
        &batch,
        &db,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(replay.get("meetings", "", &meeting).unwrap(), new);
    let reconstructed = FixedData::new(
        &replay,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let seal = SealedWorkspace::prepare(
        &reconstructed,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let replay_seed = Seed::prepare(&seal, &replay, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(replay_seed.meetings()[0].operation, op);
    assert_eq!(
        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(restored::retain(
        &rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate).unwrap(),
        &target.managed_anchor,
        &replay_seed,
        &|| true
    )
    .is_err());
}

#[test]
fn restored_audio_native_rehashed_headers_cannot_change_type_origin_or_fingerprint() {
    let (_root, target, meeting, op) = native_fixture();
    for attack in [
        "version",
        "subtype",
        "unknown",
        "source",
        "batch",
        "archive",
        "fingerprint",
        "tables",
        "parents",
    ] {
        let mut db = target.connection.lock().unwrap();
        let json: String = db
            .query_row(
                "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                [&op],
                |r| r.get(0),
            )
            .unwrap();
        let mut header: Value = serde_json::from_str(&json).unwrap();
        match attack {
            "version" => header["version"] = json!(2),
            "subtype" => header["subtype"] = json!("audioOnlyDeletion"),
            "unknown" => header["executeAllowed"] = json!(true),
            "source" => header["sourceMeeting"] = json!(Uuid::new_v4().to_string()),
            "batch" => header["restoreBatch"] = json!(Uuid::new_v4().to_string()),
            "archive" => header["sourceArchive"] = json!(Uuid::new_v4().to_string()),
            "fingerprint" => header["archiveFingerprint"] = json!("0".repeat(64)),
            "tables" => header["tables"][0]["rows"] = json!(999),
            "parents" => header["parents"] = json!([]),
            _ => unreachable!(),
        }
        // Preserve original field order for each known key. Rehashing the JSON
        // does not skip strict decoding, origin or native state verification.
        let ordered = [
            "version",
            "subtype",
            "operation",
            "meeting",
            "sourceArchive",
            "sourceMeeting",
            "restoreBatch",
            "archiveFingerprint",
            "tables",
            "root",
            "database",
            "parents",
        ];
        let container = |key: &str| -> String {
            db.query_row("SELECT json_extract(target_manifest_json,?1) FROM maintenance_operations WHERE id=?2",params![format!("$.{key}"),op],|r|r.get(0)).unwrap()
        };
        let original: Value = serde_json::from_str(&json).unwrap();
        let unchanged = ordered
            .iter()
            .map(|key| {
                let value = if matches!(*key, "tables" | "root" | "database" | "parents") {
                    container(key)
                } else {
                    serde_json::to_string(&original[*key]).unwrap()
                };
                format!("{}:{value}", serde_json::to_string(key).unwrap())
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            format!("{{{unchanged}}}"),
            json,
            "preserve canonical nested identities"
        );
        let mut members = ordered
            .iter()
            .map(|key| {
                format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    if matches!(*key, "root" | "database")
                        || (*key == "parents" && attack != "parents")
                        || (*key == "tables" && attack != "tables")
                    {
                        container(key)
                    } else {
                        serde_json::to_string(&header[*key]).unwrap()
                    }
                )
            })
            .collect::<Vec<_>>();
        if attack == "unknown" {
            members.push("\"executeAllowed\":true".into());
        }
        let json = format!("{{{}}}", members.join(","));
        let mut hash = Sha256::new();
        hash.update(b"mon/restored-audio-unavailable/v1\0");
        hash.update(json.as_bytes());
        let hash = format!("{:x}", hash.finalize());
        let tx = db.transaction().unwrap();
        tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        tx.execute(
            "UPDATE maintenance_operations SET target_manifest_json=?1,target_hash=?2 WHERE id=?3",
            params![json, hash, op],
        )
        .unwrap();
        tx.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        assert!(restored::deleted(&tx, &op, &meeting).is_err(), "{attack}");
        assert!(audio::require_source(&tx, &meeting).is_err(), "{attack}");
        assert!(
            restored::backup_meeting(
                &tx,
                &target.managed_anchor,
                &op,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{attack}"
        );
        assert_eq!(
            restored::inspect_one(&tx, &target.managed_anchor, &op).unwrap(),
            Some("restored_audio_receipt_invalid"),
            "{attack}"
        );
        tx.rollback().unwrap();
        assert!(
            restored::deleted(&db, &op, &meeting).is_ok(),
            "restore positive baseline: {attack}"
        );
    }
}
