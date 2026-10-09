use super::*;
use crate::{
    analysis_worker::AnalysisController, input_status::archived::ArchivedInputs,
    storage::RecordingConfig,
};
use serde_json::{json, Value};
use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    session: String,
    directory: PathBuf,
    journal: Vec<u8>,
    manifest: Vec<u8>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage
        .create_meeting("SYNTHETIC FAILED INPUT BACKUP")
        .unwrap()
        .id;
    let session = storage
        .prepare_recording(
            &meeting,
            &RecordingConfig {
                mode: "system".into(),
                backend: "screencapturekit".into(),
                target_pids: vec![],
                microphone_device_uid: None,
                target_application_identity: None,
            },
            &Uuid::new_v4().to_string(),
        )
        .unwrap()
        .id;
    storage.fail_recording(&session).unwrap();
    let directory = storage
        .root()
        .join("meetings")
        .join(&meeting)
        .join("audio")
        .join(&session);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let report = |lane: &str| {
        let status = |seq: u64| json!({"state":if seq%2==1{"disconnected"}else{"connected"},"sourceEpoch":seq,"interruptions":seq/2+seq%2,"reason":if seq%2==1{Some(if lane=="microphone"{"device_disconnected"}else{"stream_stopped"})}else{None}});
        json!({"lastSequence":66,"droppedThrough":2,"events":(3_u64..=66).map(|seq|json!({"sequence":seq,"kind":if seq%2==1{"interrupted"}else{"recovered"},"status":status(seq)})).collect::<Vec<_>>()})
    };
    let stopped = json!({"state":"stopped","sourceEpoch":66,"interruptions":33,"reason":null});
    let journal=serde_json::to_vec(&json!({"schemaVersion":1,"sealed":true,"report":{"schemaVersion":3,"ownerId":session,"revision":201,"microphone":stopped,"system":stopped,"microphoneHistory":report("microphone"),"systemHistory":report("system")}})).unwrap();
    let manifest=serde_json::to_vec(&json!({"state":"failed","recordingOwnerId":session,"inputEventJournalPersistence":{"requestedRevision":201,"committedRevision":200,"state":"failed","closed":true},"inputEventJournalError":"timeout /Users/private/source/manifest.json"})).unwrap();
    for (name, data) in [
        ("input-events.json", &journal),
        ("manifest.json", &manifest),
    ] {
        std::fs::write(directory.join(name), data).unwrap();
        std::fs::set_permissions(directory.join(name), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    Fixture {
        _root: root,
        storage,
        meeting,
        session,
        directory,
        journal,
        manifest,
    }
}
fn payload(db: &Connection, session: &str) -> String {
    db.query_row(
        "SELECT input_history_snapshot_json FROM recording_sessions WHERE id=?1",
        params![session],
        |r| r.get(0),
    )
    .unwrap()
}
fn restore(
    source: &archive::VerifiedArchive,
    parent: &Path,
    root: &Path,
    meeting: &str,
    session: &str,
) -> (Storage, String, String) {
    let ids =
        restore_ids::RestoreIds::prepare(source, parent, &Uuid::new_v4().to_string(), &|| false)
            .unwrap();
    let new_meeting = ids.get("meetings", "", meeting).unwrap();
    let new_session = ids.get("recording_sessions", meeting, session).unwrap();
    let fixed = restore_fixed::FixedData::new(&ids, parent, &|| false).unwrap();
    let sealed =
        restore_fixed::workspace::seal::SealedWorkspace::prepare(&fixed, parent, &|| false)
            .unwrap();
    let target = Storage::open(root.into()).unwrap();
    let existing = target.create_meeting("EXISTING MEETING UNCHANGED").unwrap();
    target.publish_restore(&sealed, &ids, &|| false).unwrap();
    assert_eq!(target.get_meeting(&existing.id).unwrap().unwrap(), existing);
    (target, new_meeting, new_session)
}

#[test]
fn failed_recording_actual_backup_additive_restore_rebackup_second_restore_preserve_evidence_and_rebind_only_logical_owner(
) {
    for rendered in [false, true] {
        let f = fixture();
        let admission = AnalysisController::default();
        let notices = ArchivedInputs::default();
        let lease = admission.begin_notice_storage().unwrap();
        let page = serde_json::to_value(notices.page(&f.storage, &lease, None, &|| true).unwrap())
            .unwrap();
        let row = &page["recordings"][0];
        assert_eq!(row["historySaveIssue"], "failed");
        let notice = &row["batch"]["notices"][0];
        let recipient = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().to_string();
        for action in [Some("receive"), rendered.then_some("render")]
            .into_iter()
            .flatten()
        {
            let request=serde_json::from_value(json!({"meetingId":f.meeting,"sessionId":f.session,"noticeId":notice["id"],"recipientId":recipient,"generation":notice["delivery"]["generation"],"action":action,"receiveId":if action=="receive"{Some(&nonce)}else{None}})).unwrap();
            notices
                .acknowledge(&f.storage, &lease, request, &|| true)
                .unwrap();
        }
        let original_ledger = std::fs::read(f.directory.join("input-notices.json")).unwrap();
        drop(lease);
        let maintenance = admission
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("inputs.monbackup");
        f.storage.create_backup_archive(&path, &|| false).unwrap();
        let source =
            archive::read(&mut File::open(&path).unwrap(), output.path(), &|| false).unwrap();
        let text = payload(&source.database, &f.session);
        let original: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(original["sessionId"], f.session);
        assert_eq!(original["snapshot"]["sourceOwnerId"], f.session);
        assert_eq!(
            original["snapshot"]["sourceJournal"],
            serde_json::from_slice::<Value>(&f.journal).unwrap()
        );
        assert_eq!(
            original["snapshot"]["sourceDelivery"]["record"],
            serde_json::from_slice::<Value>(&original_ledger).unwrap()
        );
        let inert = &original["snapshot"]["delivery"]["queue"]["lanes"];
        assert_eq!(inert[0]["lost"], 2);
        assert_eq!(inert[1]["lost"], 2);
        assert_eq!(
            inert[0]["pending"][0]["event"]["sequence"],
            if rendered { 4 } else { 3 }
        );
        assert!(inert[0]["pending"][0]["recipient"].is_null());
        assert_eq!(
            original["snapshot"]["finalDiagnostic"]["persistence"]["state"],
            "failed"
        );
        assert_eq!(
            original["snapshot"]["finalDiagnostic"]["unconfirmedAtStop"],
            true
        );
        assert!(!text.contains("/Users/private"));
        // Do not stamp the export into the original live DB or change its files.
        assert!(f
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT input_history_snapshot_json FROM recording_sessions WHERE id=?1",
                params![f.session],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap()
            .is_none());
        assert_eq!(
            std::fs::read(f.directory.join("input-notices.json")).unwrap(),
            original_ledger
        );
        assert_eq!(
            std::fs::read(f.directory.join("input-events.json")).unwrap(),
            f.journal
        );
        assert_eq!(
            std::fs::read(f.directory.join("manifest.json")).unwrap(),
            f.manifest
        );
        let target_root = tempfile::tempdir().unwrap();
        let (target, new_meeting, new_session) = restore(
            &source,
            output.path(),
            target_root.path(),
            &f.meeting,
            &f.session,
        );
        assert_ne!(new_session, f.session);
        assert_ne!(new_meeting, f.meeting);
        let restored: Value =
            serde_json::from_str(&payload(&target.connection.lock().unwrap(), &new_session))
                .unwrap();
        assert_eq!(restored["meetingId"], new_meeting);
        assert_eq!(restored["sessionId"], new_session);
        assert_eq!(restored["snapshot"], original["snapshot"]);
        assert!(!target
            .root()
            .join(format!("meetings/{new_meeting}/audio/{new_session}"))
            .exists());
        assert_eq!(target.closed_input_recordings(None).unwrap().0.len(), 1);
        // No fabricated native producer directory or old acceptance authority.
        assert!(target
            .closed_input_directory(&new_meeting, &new_session)
            .is_err());
        let path2 = output.path().join("rebackup.monbackup");
        target.create_backup_archive(&path2, &|| false).unwrap();
        let again =
            archive::read(&mut File::open(path2).unwrap(), output.path(), &|| false).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&payload(&again.database, &new_session)).unwrap(),
            restored
        );
        let third_root = tempfile::tempdir().unwrap();
        let (third, third_meeting, third_session) = restore(
            &again,
            output.path(),
            third_root.path(),
            &new_meeting,
            &new_session,
        );
        let twice: Value =
            serde_json::from_str(&payload(&third.connection.lock().unwrap(), &third_session))
                .unwrap();
        assert_eq!(twice["meetingId"], third_meeting);
        assert_eq!(twice["sessionId"], third_session);
        assert_eq!(twice["snapshot"], original["snapshot"]);
        drop(maintenance);
        assert!(f.storage.create_meeting("SOURCE STILL WRITABLE").is_ok());
    }
}

#[test]
fn pinned_snapshot_rejects_leaf_changes_equal_byte_replacement_new_files_and_ancestor_replacement_without_rewriting_source(
) {
    for change in ["bytes", "replacement", "new_leaf", "ancestor"] {
        let f = fixture();
        let snapshot = f.storage.prepare_backup_snapshot(&|| false).unwrap();
        match change {
            "bytes" => {
                std::fs::write(f.directory.join("manifest.json"), br#"{"state":"failed"}"#).unwrap()
            }
            "replacement" => {
                std::fs::rename(
                    f.directory.join("input-events.json"),
                    f.directory.join("original-events.json"),
                )
                .unwrap();
                std::fs::write(f.directory.join("input-events.json"), &f.journal).unwrap();
                std::fs::set_permissions(
                    f.directory.join("input-events.json"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            "new_leaf" => {
                std::fs::write(f.directory.join("input-notices.json"), b"{}").unwrap();
            }
            "ancestor" => {
                let meeting_dir = f.directory.parent().unwrap().parent().unwrap();
                let old = meeting_dir.with_file_name(format!("original-{}", f.meeting));
                std::fs::rename(meeting_dir, &old).unwrap();
                std::fs::create_dir(meeting_dir).unwrap();
                std::fs::rename(old.join("audio"), meeting_dir.join("audio")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(snapshot.verify(&|| false).is_err(), "accepted {change}");
        drop(snapshot);
        assert!(f.storage.create_meeting("MAINTENANCE RELEASED").is_ok());
        assert_eq!(
            f.storage.get_meeting(&f.meeting).unwrap().unwrap().id,
            f.meeting
        );
    }
}

#[test]
fn corrupt_or_foreign_companions_and_symlink_or_shared_link_never_publish_a_partial_backup() {
    for change in [
        "journal",
        "foreign",
        "symlink",
        "hardlink",
        "ledger_duplicate",
    ] {
        let f = fixture();
        let journal_path = f.directory.join("input-events.json");
        match change {
            "journal" => std::fs::write(&journal_path, b"{}").unwrap(),
            "foreign" => {
                let mut value: Value = serde_json::from_slice(&f.journal).unwrap();
                value["report"]["ownerId"] = json!(Uuid::new_v4().to_string());
                std::fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "symlink" => {
                std::fs::rename(&journal_path, f.directory.join("foreign.json")).unwrap();
                symlink(f.directory.join("foreign.json"), &journal_path).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&journal_path, f.directory.join("shared.json")).unwrap()
            }
            "ledger_duplicate" => {
                std::fs::write(
                    f.directory.join("input-notices.json"),
                    b"{\"schemaVersion\":2,\"schemaVersion\":2}",
                )
                .unwrap();
                std::fs::set_permissions(
                    f.directory.join("input-notices.json"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("refused.monbackup");
        assert!(
            f.storage.create_backup_archive(&path, &|| false).is_err(),
            "accepted {change}"
        );
        assert!(!path.exists());
        assert!(f
            .storage
            .create_meeting("SOURCE USABLE AFTER FAILURE")
            .is_ok());
    }
}

#[test]
fn restored_binding_is_immutable_and_owner_mismatch_duplicate_and_active_rows_are_rejected() {
    let f = fixture();
    let root = f.storage.managed_anchor.backup_input_root().unwrap();
    let (_, snapshot) = source::capture(&root, &f.meeting, &f.session, &|| false).unwrap();
    let bound = Bound {
        schema_version: 1,
        meeting_id: f.meeting.clone(),
        session_id: f.session.clone(),
        snapshot: snapshot.unwrap(),
    };
    let text = bound.encode().unwrap();
    let other = Uuid::new_v4().to_string();
    assert!(Bound::decode(&text, &f.meeting, &other).is_err());
    assert!(Bound::decode(
        &text.replacen(
            "\"schemaVersion\":1",
            "\"schemaVersion\":1,\"schemaVersion\":1",
            1
        ),
        &f.meeting,
        &f.session
    )
    .is_err());
    let db = f.storage.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE recording_sessions SET input_history_snapshot_json=?1 WHERE id=?2",
            params![text, f.session]
        )
        .is_err());
    // Simulate an app-owned additive INSERT, never disabling live constraints.
    drop(db);
    let meeting = f
        .storage
        .create_meeting("IMPORTED CLOSED OWNER")
        .unwrap()
        .id;
    let session = Uuid::new_v4().to_string();
    let rebound = Bound::decode(&text, &f.meeting, &f.session)
        .unwrap()
        .rebind(&meeting, &session)
        .unwrap();
    let db = f.storage.connection.lock().unwrap();
    // A restored meeting retains its actual recording source type. Use a
    // valid original config so unrelated source/config constraints stay on.
    assert_eq!(
        db.execute(
            "UPDATE meetings SET source_type='recording' WHERE id=?1",
            params![meeting]
        )
        .unwrap(),
        1
    );
    let config: String = db
        .query_row(
            "SELECT capture_config_json FROM recording_sessions WHERE id=?1",
            params![f.session],
            |r| r.get(0),
        )
        .unwrap();
    let insert="INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at,input_history_snapshot_json) VALUES(?1,?2,?3,'system',?5,?1,0,0,?4)";
    assert!(db
        .execute(
            insert,
            params![session, meeting, "preparing", rebound, config]
        )
        .is_err());
    assert!(db
        .execute(insert, params![session, meeting, "failed", text, config])
        .is_err());
    db.execute(insert, params![session, meeting, "failed", rebound, config])
        .unwrap();
    assert!(db
        .execute(
            "UPDATE recording_sessions SET input_history_snapshot_json=NULL WHERE id=?1",
            params![session]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE recording_sessions SET state='recording' WHERE id=?1",
            params![session]
        )
        .is_err());
    validate_database(&db, &|| false).unwrap();
}

#[test]
fn legacy_twenty_eight_schema_has_no_snapshot_column_and_migrates_without_changing_existing_evidence(
) {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
    let db = portable::initialize(db, 28).unwrap();
    assert!(!has_column(&db).unwrap());
    validate_database(&db, &|| false).unwrap();
    drop(db);
    let storage = Storage::open(root.path().into()).unwrap();
    let db = storage.connection.lock().unwrap();
    assert!(has_column(&db).unwrap());
    assert_eq!(
        super::super::super::verify_migration_history(&db).unwrap(),
        29
    );
    drop(db);
    drop(storage);

    // Also migrate a populated old DB. Do not manufacture a successful save
    // or stamp native evidence into the new immutable restored-only column.
    let f = fixture();
    {
        let db = f.storage.connection.lock().unwrap();
        remove_schema_for_test(&db);
        assert!(!has_column(&db).unwrap());
        assert_eq!(
            super::super::super::verify_migration_history(&db).unwrap(),
            28
        );
    }
    let root = f.storage.root().to_owned();
    drop(f.storage);
    let migrated = Storage::open(root).unwrap();
    let db = migrated.connection.lock().unwrap();
    let (state, snapshot): (String, Option<String>) = db
        .query_row(
            "SELECT state,input_history_snapshot_json FROM recording_sessions WHERE id=?1",
            params![f.session],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert!(snapshot.is_none());
    assert_eq!(
        std::fs::read(f.directory.join("input-events.json")).unwrap(),
        f.journal
    );
    assert_eq!(
        std::fs::read(f.directory.join("manifest.json")).unwrap(),
        f.manifest
    );
}
