use super::*;

fn restored() -> (
    Fixture,
    tempfile::TempDir,
    tempfile::TempDir,
    Storage,
    String,
    String,
) {
    let f = fixture();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("restored-delivery.monbackup");
    f.storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let archive = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, owner) =
        restore(&archive, output.path(), root.path(), &f.meeting, &f.session);
    (f, output, root, storage, meeting, owner)
}
fn page(
    controller: &ArchivedInputs,
    storage: &Storage,
    lease: &crate::analysis_worker::data_maintenance::NoticeStorageLease,
) -> Value {
    serde_json::to_value(controller.page(storage, lease, None, &|| true).unwrap()).unwrap()
}
fn microphone(page: &Value) -> Value {
    page["recordings"][0]["batch"]["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["input"] == "microphone")
        .unwrap()
        .clone()
}
fn request(
    meeting: &str,
    owner: &str,
    notice: &Value,
    recipient: &str,
    nonce: &str,
    action: &str,
) -> crate::input_status::archived::Acknowledge {
    serde_json::from_value(
        json!({"meetingId":meeting,"sessionId":owner,"noticeId":notice["id"],
        "recipientId":recipient,"generation":notice["delivery"]["generation"],"action":action,
        "receiveId":(action=="receive").then_some(nonce)}),
    )
    .unwrap()
}
fn ack(
    controller: &ArchivedInputs,
    storage: &Storage,
    lease: &crate::analysis_worker::data_maintenance::NoticeStorageLease,
    meeting: &str,
    owner: &str,
    notice: &Value,
    recipient: &str,
    nonce: &str,
    action: &str,
) -> Value {
    serde_json::to_value(
        controller
            .acknowledge(
                storage,
                lease,
                request(meeting, owner, notice, recipient, nonce, action),
                &|| true,
            )
            .unwrap(),
    )
    .unwrap()
}
fn delivery_text(storage: &Storage, owner: &str) -> String {
    storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state_json FROM recording_input_delivery WHERE session_id=?1",
            params![owner],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn restored_delivery_is_in_actual_meeting_deletion_scope_and_old_review_cannot_erase_new_progress()
{
    use crate::storage::deletion::operations::Prepared;

    let (source, _output, _root, storage, meeting, owner) = restored();
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    let keep = storage.create_meeting("KEEP UNRELATED MEETING").unwrap();
    let controller = ArchivedInputs::default();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let notice = microphone(&page(&controller, &storage, &lease));
    let mut stale =
        Prepared::fresh(&storage, &meeting, &crate::test_support::never_cancelled).unwrap();
    assert!(stale.preview().execute_allowed);
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &notice,
        &Uuid::new_v4().to_string(),
        &Uuid::new_v4().to_string(),
        "receive",
    );
    let progress = delivery_text(&storage, &owner);
    drop(lease);
    let maintenance = admission
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(stale
        .execute(
            &storage,
            &maintenance,
            &stale.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .is_err());
    assert_eq!(delivery_text(&storage, &owner), progress);
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
    assert!(storage.get_meeting(&meeting).unwrap().is_some());

    let mut current =
        Prepared::fresh(&storage, &meeting, &crate::test_support::never_cancelled).unwrap();
    let confirmation = current.preview().confirmation_id;
    let receipt = current
        .execute(
            &storage,
            &maintenance,
            &confirmation,
            true,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted && !receipt.retained && !receipt.restart_required);
    assert!(storage.get_meeting(&meeting).unwrap().is_none());
    assert_eq!(storage.get_meeting(&keep.id).unwrap(), Some(keep));
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM recording_input_delivery WHERE session_id=?1",
                [&owner],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read(source.directory.join("input-events.json")).unwrap(),
        source.journal
    );
    assert_eq!(
        std::fs::read(source.directory.join("manifest.json")).unwrap(),
        source.manifest
    );
}

#[test]
fn restored_delivery_actual_apis_share_one_writer_and_read_back_lost_receive_render_close_replies()
{
    let (source, _output, root, storage, meeting, owner) = restored();
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let first = ArchivedInputs::default();
    let second = ArchivedInputs::default();
    let p = page(&first, &storage, &lease);
    assert!(p["recordings"][0]["issue"].is_null());
    let notice = microphone(&p);
    assert!(notice["id"].as_str().unwrap().starts_with(&owner));
    assert_eq!(notice["delivery"]["sequence"], 3);
    assert_eq!(
        microphone(&page(&second, &storage, &lease))["delivery"]["generation"],
        notice["delivery"]["generation"]
    );
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    let received = ack(
        &first, &storage, &lease, &meeting, &owner, &notice, &recipient, &nonce, "receive",
    );
    assert_eq!(received["ownerId"], owner);
    assert_eq!(received["delivery"]["rendered"], false);
    let durable = delivery_text(&storage, &owner);
    // Discard the first response; another controller reads the durable winner.
    let read = microphone(&page(&second, &storage, &lease));
    assert_eq!(read["recipientId"], recipient);
    assert_eq!(read["delivery"]["receiveId"], nonce);
    ack(
        &second, &storage, &lease, &meeting, &owner, &notice, &recipient, &nonce, "receive",
    );
    assert_eq!(delivery_text(&storage, &owner), durable);
    ack(
        &first, &storage, &lease, &meeting, &owner, &notice, &recipient, &nonce, "render",
    );
    assert_eq!(
        microphone(&page(&second, &storage, &lease))["delivery"]["rendered"],
        true
    );
    let closed = ack(
        &second, &storage, &lease, &meeting, &owner, &notice, &recipient, &nonce, "close",
    );
    let replay = ack(
        &first, &storage, &lease, &meeting, &owner, &notice, &recipient, &nonce, "close",
    );
    assert_eq!(replay["noticeId"], closed["noticeId"]);
    assert_eq!(replay["delivery"]["rendered"], true);
    assert_eq!(
        microphone(&page(&first, &storage, &lease))["delivery"]["sequence"],
        4
    );
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
    assert!(storage.closed_input_directory(&meeting, &owner).is_err());
    assert!(!root
        .path()
        .join(format!("meetings/{meeting}/audio/{owner}"))
        .exists());
    assert_eq!(
        std::fs::read(source.directory.join("input-events.json")).unwrap(),
        source.journal
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM recording_input_delivery", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    drop(lease);
    drop(first);
    drop(second);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    let lease = admission.begin_notice_storage().unwrap();
    assert_eq!(
        microphone(&page(&ArchivedInputs::default(), &reopened, &lease))["delivery"]["sequence"],
        4
    );
    assert!(ArchivedInputs::default()
        .acknowledge(
            &reopened,
            &lease,
            request(&meeting, &owner, &notice, &recipient, &nonce, "close"),
            &|| true
        )
        .is_err());
}

#[test]
fn restored_delivery_restart_suppresses_rendered_head_and_fences_unrendered_claim() {
    for rendered in [false, true] {
        let (_source, _output, root, storage, meeting, owner) = restored();
        let admission = AnalysisController::default();
        let lease = admission.begin_notice_storage().unwrap();
        let controller = ArchivedInputs::default();
        let notice = microphone(&page(&controller, &storage, &lease));
        let recipient = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().to_string();
        ack(
            &controller,
            &storage,
            &lease,
            &meeting,
            &owner,
            &notice,
            &recipient,
            &nonce,
            "receive",
        );
        if rendered {
            ack(
                &controller,
                &storage,
                &lease,
                &meeting,
                &owner,
                &notice,
                &recipient,
                &nonce,
                "render",
            );
        }
        let before = payload(&storage.connection.lock().unwrap(), &owner);
        drop(lease);
        drop(controller);
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        let lease = admission.begin_notice_storage().unwrap();
        let controller = ArchivedInputs::default();
        let current = microphone(&page(&controller, &storage, &lease));
        assert_eq!(
            current["delivery"]["sequence"],
            if rendered { 4 } else { 3 }
        );
        if !rendered {
            assert!(
                current["delivery"]["generation"].as_u64().unwrap()
                    > notice["delivery"]["generation"].as_u64().unwrap()
            );
        }
        for action in ["receive", "render", "close"] {
            assert!(controller
                .acknowledge(
                    &storage,
                    &lease,
                    request(&meeting, &owner, &notice, &recipient, &nonce, action),
                    &|| true
                )
                .is_err());
        }
        assert_eq!(payload(&storage.connection.lock().unwrap(), &owner), before);
        ack(
            &controller,
            &storage,
            &lease,
            &meeting,
            &owner,
            &current,
            &recipient,
            &Uuid::new_v4().to_string(),
            "receive",
        );
    }
}

#[test]
fn restored_delivery_two_rebackups_keep_display_progress_flat_without_stamping_source_or_old_receipt_authority(
) {
    let (original, output, _root, first, first_meeting, first_owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let first_notice = microphone(&page(&controller, &first, &lease));
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &first,
        &lease,
        &first_meeting,
        &first_owner,
        &first_notice,
        &recipient,
        &nonce,
        "receive",
    );
    ack(
        &controller,
        &first,
        &lease,
        &first_meeting,
        &first_owner,
        &first_notice,
        &recipient,
        &nonce,
        "render",
    );
    let immutable = payload(&first.connection.lock().unwrap(), &first_owner);
    let current = delivery_text(&first, &first_owner);
    let path = output.path().join("after-render.monbackup");
    first
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(delivery_text(&first, &first_owner), current);
    assert_eq!(
        payload(&first.connection.lock().unwrap(), &first_owner),
        immutable
    );
    assert!(first
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT input_delivery_transfer_json FROM recording_sessions WHERE id=?1",
            params![first_owner],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap()
        .is_none());
    let archive = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        archive
            .database
            .query_row("SELECT count(*) FROM recording_input_delivery", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    let second_root = tempfile::tempdir().unwrap();
    let (second, second_meeting, second_owner) = restore(
        &archive,
        output.path(),
        second_root.path(),
        &first_meeting,
        &first_owner,
    );
    let second_notice = microphone(&page(&controller, &second, &lease));
    assert_eq!(second_notice["delivery"]["sequence"], 4);
    for action in ["receive", "render", "close"] {
        assert!(controller
            .acknowledge(
                &second,
                &lease,
                request(
                    &second_meeting,
                    &second_owner,
                    &first_notice,
                    &recipient,
                    &nonce,
                    action
                ),
                &|| true
            )
            .is_err());
    }
    let transfer: String = second
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT input_delivery_transfer_json FROM recording_sessions WHERE id=?1",
            params![second_owner],
            |r| r.get(0),
        )
        .unwrap();
    let second_nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &second,
        &lease,
        &second_meeting,
        &second_owner,
        &second_notice,
        &recipient,
        &second_nonce,
        "receive",
    );
    ack(
        &controller,
        &second,
        &lease,
        &second_meeting,
        &second_owner,
        &second_notice,
        &recipient,
        &second_nonce,
        "render",
    );
    let path = output.path().join("second-render.monbackup");
    second
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        second
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT input_delivery_transfer_json FROM recording_sessions WHERE id=?1",
                params![second_owner],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        transfer
    );
    let archive = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let third_root = tempfile::tempdir().unwrap();
    let (third, third_meeting, third_owner) = restore(
        &archive,
        output.path(),
        third_root.path(),
        &second_meeting,
        &second_owner,
    );
    let third_notice = microphone(&page(&controller, &third, &lease));
    assert_eq!(third_notice["delivery"]["sequence"], 5);
    let final_payload: Value =
        serde_json::from_str(&payload(&third.connection.lock().unwrap(), &third_owner)).unwrap();
    let initial_payload: Value = serde_json::from_str(&immutable).unwrap();
    assert_eq!(final_payload["snapshot"], initial_payload["snapshot"]);
    assert_eq!(final_payload["snapshot"]["sourceOwnerId"], original.session);
    let state: Value = serde_json::from_str(&delivery_text(&third, &third_owner)).unwrap();
    assert_eq!(state["removed"][0]["ownerId"], second_owner);
    assert_eq!(state["removed"][0]["delivery"]["sequence"], 4);
    assert!(state.get("previousTransfer").is_none());
    assert!(serde_json::to_vec(&state).unwrap().len() < 128 * 1024);
    assert!(!third
        .root()
        .join(format!("meetings/{third_meeting}/audio/{third_owner}"))
        .exists());
}

#[test]
fn restored_delivery_corrupt_mutable_state_is_not_reseeded_and_writer_drift_is_fenced() {
    let (_source, _output, _root, storage, meeting, owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let old = microphone(&page(&controller, &storage, &lease));
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_input_delivery SET writer_id=?1 WHERE session_id=?2",
            params![Uuid::new_v4().to_string(), owner],
        )
        .unwrap();
    let failed = page(&controller, &storage, &lease);
    assert_eq!(failed["recordings"][0]["issue"], "deliveryUnavailable");
    assert_eq!(failed["recordings"][0]["batch"]["notices"], json!([]));
    let fresh = microphone(&page(&controller, &storage, &lease));
    assert!(
        fresh["delivery"]["generation"].as_u64().unwrap()
            > old["delivery"]["generation"].as_u64().unwrap()
    );
    let mut corrupt: Value = serde_json::from_str(&delivery_text(&storage, &owner)).unwrap();
    corrupt["sourceHash"] = json!("f".repeat(64));
    let text = serde_json::to_string(&corrupt).unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_input_delivery SET state_json=?1 WHERE session_id=?2",
            params![text, owner],
        )
        .unwrap();
    for _ in 0..2 {
        let failed = page(&controller, &storage, &lease);
        assert_eq!(failed["recordings"][0]["issue"], "deliveryUnavailable");
        assert_eq!(failed["recordings"][0]["batch"]["notices"], json!([]));
        assert_eq!(delivery_text(&storage, &owner), text);
        assert!(controller
            .acknowledge(
                &storage,
                &lease,
                request(
                    &meeting,
                    &owner,
                    &fresh,
                    &Uuid::new_v4().to_string(),
                    &Uuid::new_v4().to_string(),
                    "receive"
                ),
                &|| true
            )
            .is_err());
    }
    let output = tempfile::tempdir().unwrap();
    assert!(storage
        .create_backup_archive(
            &output.path().join("corrupt.monbackup"),
            &crate::test_support::never_cancelled
        )
        .is_err());
}

#[test]
fn restored_delivery_current_reopen_and_maintenance_preserve_original_proof() {
    let (_source, _output, root, storage, meeting, owner) = restored();
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let notice = microphone(&page(&controller, &storage, &lease));
    let text = delivery_text(&storage, &owner);
    storage
        .maintenance
        .store(true, std::sync::atomic::Ordering::Release);
    let failed = page(&controller, &storage, &lease);
    assert_eq!(failed["recordings"][0]["issue"], "deliveryUnavailable");
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            request(
                &meeting,
                &owner,
                &notice,
                &Uuid::new_v4().to_string(),
                &Uuid::new_v4().to_string(),
                "receive"
            ),
            &|| true
        )
        .is_err());
    assert_eq!(delivery_text(&storage, &owner), text);
    storage
        .maintenance
        .store(false, std::sync::atomic::Ordering::Release);
    storage
        .retired
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(controller.page(&storage, &lease, None, &|| true).is_err());
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            request(
                &meeting,
                &owner,
                &notice,
                &Uuid::new_v4().to_string(),
                &Uuid::new_v4().to_string(),
                "receive"
            ),
            &|| true
        )
        .is_err());
    assert_eq!(delivery_text(&storage, &owner), text);
}

#[test]
fn restored_delivery_rechecks_maintenance_after_database_wait_without_publishing_candidate() {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let (_source, _output, _root, storage, meeting, owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    page(&controller, &storage, &lease);
    let before = delivery_text(&storage, &owner);
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);

    // The worker passes the initial admission check, then waits for the DB
    // while holding the destination registry. No timing guess about SQL work.
    let db = storage.connection.lock().unwrap();
    let (observed, result) = std::thread::scope(|scope| {
        let worker =
            scope.spawn(|| storage.restored_input_page(&meeting, &owner, &lease, &|| true));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut observed = false;
        while Instant::now() < deadline {
            if storage.restored_inputs.try_lock().is_err() {
                observed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        storage.maintenance.store(true, Ordering::Release);
        // Always release before assertions/join, including a failed handshake.
        drop(db);
        (observed, worker.join().unwrap())
    });
    storage.maintenance.store(false, Ordering::Release);
    assert!(observed, "worker did not reach the DB wait");
    assert!(matches!(result, Err(ref error) if error.contains("データ操作中")));
    assert_eq!(delivery_text(&storage, &owner), before);
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
    assert!(page(&controller, &storage, &lease)["recordings"][0]["issue"].is_null());
}

#[test]
fn restored_delivery_failed_sql_render_keeps_unrendered_head_and_fences_retry() {
    let (_source, _output, _root, storage, meeting, owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let old = microphone(&page(&controller, &storage, &lease));
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &old,
        &recipient,
        &nonce,
        "receive",
    );
    let before = delivery_text(&storage, &owner);
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    storage.connection.lock().unwrap().execute_batch(
        "CREATE TRIGGER test_restored_delivery_write_failure BEFORE UPDATE ON recording_input_delivery BEGIN SELECT RAISE(ABORT,'test destination write failure'); END;"
    ).unwrap();
    let failure = controller
        .acknowledge(
            &storage,
            &lease,
            request(&meeting, &owner, &old, &recipient, &nonce, "render"),
            &|| true,
        )
        .unwrap_err();
    assert!(failure.contains("test destination write failure"));
    assert_eq!(delivery_text(&storage, &owner), before);
    let failed = page(&controller, &storage, &lease);
    assert_eq!(failed["recordings"][0]["issue"], "deliveryUnavailable");
    assert_eq!(failed["recordings"][0]["batch"]["notices"], json!([]));
    assert_eq!(delivery_text(&storage, &owner), before);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER test_restored_delivery_write_failure;")
        .unwrap();
    let fresh = microphone(&page(&controller, &storage, &lease));
    assert_eq!(fresh["delivery"]["sequence"], old["delivery"]["sequence"]);
    assert_eq!(fresh["delivery"]["rendered"], false);
    assert!(
        fresh["delivery"]["generation"].as_u64().unwrap()
            > old["delivery"]["generation"].as_u64().unwrap()
    );
    for action in ["receive", "render", "close"] {
        assert!(controller
            .acknowledge(
                &storage,
                &lease,
                request(&meeting, &owner, &old, &recipient, &nonce, action),
                &|| true
            )
            .is_err());
    }
    let nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &fresh,
        &recipient,
        &nonce,
        "receive",
    );
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &fresh,
        &recipient,
        &nonce,
        "render",
    );
    assert_eq!(
        microphone(&page(&controller, &storage, &lease))["delivery"]["rendered"],
        true
    );
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
}

#[test]
fn restored_delivery_actual_lru_eviction_preserves_rendered_progress_and_fences_old_claims() {
    let (_source, _output, _root, storage, meeting, owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let old = microphone(&page(&controller, &storage, &lease));
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &old,
        &recipient,
        &nonce,
        "receive",
    );
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &old,
        &recipient,
        &nonce,
        "render",
    );
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    let original_writer: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT writer_id FROM recording_input_delivery WHERE session_id=?1",
            params![owner],
            |r| r.get(0),
        )
        .unwrap();
    let config: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT capture_config_json FROM recording_sessions WHERE id=?1",
            params![owner],
            |r| r.get(0),
        )
        .unwrap();
    // Insert 128 real closed DB-owned imported rows through the same typed
    // binding used by additive restore, one recording per meeting. All
    // uniqueness and ownership constraints stay enabled.
    for _ in 0..128 {
        let other_meeting = storage
            .create_meeting("LRU CLOSED IMPORTED OWNER")
            .unwrap()
            .id;
        let other = Uuid::new_v4().to_string();
        let rebound = Bound::decode(&immutable, &meeting, &owner)
            .unwrap()
            .rebind(&other_meeting, &other)
            .unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE meetings SET source_type='recording' WHERE id=?1",
                params![other_meeting],
            )
            .unwrap();
        storage.connection.lock().unwrap().execute(
            "INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at,input_history_snapshot_json) VALUES(?1,?2,'failed','system',?3,?1,0,0,?4)",
            params![other, other_meeting, config, rebound]).unwrap();
        assert!(storage
            .restored_input_page(&other_meeting, &other, &lease, &|| true)
            .unwrap()
            .is_some());
    }
    let fresh = serde_json::to_value(
        storage
            .restored_input_page(&meeting, &owner, &lease, &|| true)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let next = fresh["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["input"] == "microphone")
        .unwrap();
    assert_eq!(next["delivery"]["sequence"], 4);
    let writer: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT writer_id FROM recording_input_delivery WHERE session_id=?1",
            params![owner],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(
        writer, original_writer,
        "the original owner must really have been evicted"
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM recording_input_delivery", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        129
    );
    for action in ["receive", "render", "close"] {
        assert!(controller
            .acknowledge(
                &storage,
                &lease,
                request(&meeting, &owner, &old, &recipient, &nonce, action),
                &|| true
            )
            .is_err());
    }
    assert_eq!(
        payload(&storage.connection.lock().unwrap(), &owner),
        immutable
    );
}

#[test]
#[ignore = "parent-owned synthetic SQLite checkpoint; invoked only by restored-delivery SIGKILL test"]
fn restored_delivery_committed_checkpoint_child_fixture() {
    let root = PathBuf::from(std::env::var("MON_RESTORED_DELIVERY_TEST_ROOT").unwrap());
    let ready = PathBuf::from(std::env::var("MON_RESTORED_DELIVERY_TEST_READY").unwrap());
    let meeting = std::env::var("MON_RESTORED_DELIVERY_TEST_MEETING").unwrap();
    let owner = std::env::var("MON_RESTORED_DELIVERY_TEST_OWNER").unwrap();
    let phase = std::env::var("MON_RESTORED_DELIVERY_TEST_PHASE").unwrap();
    assert!(matches!(phase.as_str(), "receive" | "render" | "close"));
    let storage = Storage::open(root).unwrap();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let notice = microphone(&page(&controller, &storage, &lease));
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    for action in ["receive", "render", "close"] {
        ack(
            &controller,
            &storage,
            &lease,
            &meeting,
            &owner,
            &notice,
            &recipient,
            &nonce,
            action,
        );
        if phase == action {
            break;
        }
    }
    // The real API has committed before reporting a checkpoint; no fixture
    // writes to the production ledger or fabricates a successful response.
    let checkpoint = serde_json::to_vec(&json!({"notice":notice,"recipient":recipient,"nonce":nonce,
        "state":delivery_text(&storage, &owner),"source":payload(&storage.connection.lock().unwrap(), &owner)})).unwrap();
    let temporary = ready.with_extension("tmp");
    std::fs::write(&temporary, checkpoint).unwrap();
    std::fs::rename(temporary, ready).unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn restored_delivery_sigkill_after_real_commits_recovers_progress_and_fences_old_claims() {
    use std::{
        os::unix::process::ExitStatusExt,
        time::{Duration, Instant},
    };
    for phase in ["receive", "render", "close"] {
        let (_source, output, root, storage, meeting, owner) = restored();
        let immutable = payload(&storage.connection.lock().unwrap(), &owner);
        drop(storage); // The child must own SQLite rather than wait on this connection.
        let ready = output.path().join("delivery-checkpoint.json");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::backup::input_snapshots::tests::delivery::restored_delivery_committed_checkpoint_child_fixture", "--ignored", "--nocapture"])
            .env("MON_RESTORED_DELIVERY_TEST_ROOT", root.path())
            .env("MON_RESTORED_DELIVERY_TEST_READY", &ready)
            .env("MON_RESTORED_DELIVERY_TEST_MEETING", &meeting)
            .env("MON_RESTORED_DELIVERY_TEST_OWNER", &owner)
            .env("MON_RESTORED_DELIVERY_TEST_PHASE", phase)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit())
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("restored delivery commit checkpoint not reached: {phase}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let checkpoint: Value = serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        assert_eq!(
            delivery_text(&storage, &owner),
            checkpoint["state"].as_str().unwrap()
        );
        assert_eq!(
            payload(&storage.connection.lock().unwrap(), &owner),
            immutable
        );
        assert_eq!(checkpoint["source"], immutable);
        let admission = AnalysisController::default();
        let lease = admission.begin_notice_storage().unwrap();
        let controller = ArchivedInputs::default();
        let fresh = microphone(&page(&controller, &storage, &lease));
        assert_eq!(
            fresh["delivery"]["sequence"],
            if phase == "receive" { 3 } else { 4 }
        );
        for action in ["receive", "render", "close"] {
            assert!(controller
                .acknowledge(
                    &storage,
                    &lease,
                    request(
                        &meeting,
                        &owner,
                        &checkpoint["notice"],
                        checkpoint["recipient"].as_str().unwrap(),
                        checkpoint["nonce"].as_str().unwrap(),
                        action
                    ),
                    &|| true
                )
                .is_err());
        }
        assert_eq!(
            payload(&storage.connection.lock().unwrap(), &owner),
            immutable
        );
        assert!(storage.closed_input_directory(&meeting, &owner).is_err());
    }
}

#[test]
fn restored_delivery_transfer_rejects_wrong_bindings_duplicate_missing_and_foreign_proofs() {
    use crate::storage::restored_input::{capture_transfer, Transfer, TRANSFER_MAX_BYTES};
    let (_source, _output, _root, storage, meeting, owner) = restored();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let notice = microphone(&page(&controller, &storage, &lease));
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &notice,
        &recipient,
        &nonce,
        "receive",
    );
    ack(
        &controller,
        &storage,
        &lease,
        &meeting,
        &owner,
        &notice,
        &recipient,
        &nonce,
        "render",
    );
    let immutable = payload(&storage.connection.lock().unwrap(), &owner);
    let source = Bound::decode(&immutable, &meeting, &owner)
        .unwrap()
        .into_snapshot();
    let text = capture_transfer(
        &storage.connection.lock().unwrap(),
        &meeting,
        &owner,
        &source,
    )
    .unwrap()
    .unwrap();
    Transfer::decode(&text, &meeting, &owner, &source).unwrap();
    let other = Uuid::new_v4().to_string();
    assert!(Transfer::decode(&text, &other, &owner, &source).is_err());
    assert!(Transfer::decode(&text, &meeting, &other, &source).is_err());
    assert!(Transfer::decode(
        &text.replacen(
            "\"schemaVersion\":1",
            "\"schemaVersion\":1,\"schemaVersion\":1",
            1
        ),
        &meeting,
        &owner,
        &source
    )
    .is_err());
    let mut invalid = Vec::new();
    for change in [
        "sourceHash",
        "renderedProof",
        "missingState",
        "wrongVersion",
    ] {
        let mut value: Value = serde_json::from_str(&text).unwrap();
        match change {
            "sourceHash" => value["state"]["sourceHash"] = json!("f".repeat(64)),
            "renderedProof" => value["state"]["rendered"] = json!([null, null]),
            "missingState" => {
                value.as_object_mut().unwrap().remove("state");
            }
            "wrongVersion" => value["schemaVersion"] = json!(2),
            _ => unreachable!(),
        }
        let bad = serde_json::to_string(&value).unwrap();
        assert!(
            Transfer::decode(&bad, &meeting, &owner, &source).is_err(),
            "accepted {change}"
        );
        invalid.push((change, value));
    }
    assert!(Transfer::decode(
        &" ".repeat(TRANSFER_MAX_BYTES + 1),
        &meeting,
        &owner,
        &source
    )
    .is_err());
    let config: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT capture_config_json FROM recording_sessions WHERE id=?1",
            params![owner],
            |r| r.get(0),
        )
        .unwrap();
    let other_meeting = storage
        .create_meeting("TRANSFER CLOSED IMPORTED OWNER")
        .unwrap()
        .id;
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET source_type='recording' WHERE id=?1",
            params![other_meeting],
        )
        .unwrap();
    let bound = Bound::decode(&immutable, &meeting, &owner)
        .unwrap()
        .rebind(&other_meeting, &other)
        .unwrap();
    let insert = "INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at,input_history_snapshot_json,input_delivery_transfer_json) VALUES(?1,?2,?3,'system',?4,?1,0,0,?5,?6)";
    let valid = Transfer::decode(&text, &meeting, &owner, &source)
        .unwrap()
        .rebind(&other_meeting, &other, &source)
        .unwrap();
    let db = storage.connection.lock().unwrap();
    assert!(db
        .execute(
            insert,
            params![other, other_meeting, "failed", config, bound, text]
        )
        .is_err());
    assert!(db
        .execute(
            insert,
            params![other, other_meeting, "recording", config, bound, valid]
        )
        .is_err());
    for (change, mut value) in invalid {
        value["meetingId"] = json!(other_meeting);
        value["sessionId"] = json!(other);
        let bad = serde_json::to_string(&value).unwrap();
        match db.execute(
            insert,
            params![other, other_meeting, "failed", config, bound, bad],
        ) {
            Ok(_) => {
                assert!(
                    validate_database(&db, &crate::test_support::never_cancelled).is_err(),
                    "accepted DB {change}"
                );
                db.execute("DELETE FROM recording_sessions WHERE id=?1", params![other])
                    .unwrap();
            }
            Err(_) => assert_eq!(change, "wrongVersion"),
        }
    }
    db.execute(
        insert,
        params![other, other_meeting, "failed", config, bound, valid],
    )
    .unwrap();
    validate_database(&db, &crate::test_support::never_cancelled).unwrap();
    assert!(db
        .execute(
            "UPDATE recording_sessions SET input_delivery_transfer_json=NULL WHERE id=?1",
            params![other]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE recording_sessions SET meeting_id=?1 WHERE id=?2",
            params![Uuid::new_v4().to_string(), other]
        )
        .is_err());
    assert_eq!(payload(&db, &owner), immutable);
}
