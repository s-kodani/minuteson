use super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::{with_test_database, RecordingConfig},
};
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;

fn fixture(storage: &Storage, closed: bool) -> (String, String, PathBuf) {
    let meeting = storage.create_meeting("前回の録音").unwrap().id;
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
    let output = storage
        .root()
        .join("meetings")
        .join(&meeting)
        .join("audio")
        .join(&session);
    fs::create_dir_all(&output).unwrap();
    if closed {
        storage.fail_recording(&session).unwrap();
    }
    (meeting, session, output)
}
fn history(output: &std::path::Path, owner: &str) -> Vec<u8> {
    let value = json!({"schemaVersion":1,"sealed":true,"report":{
        "schemaVersion":3,"ownerId":owner,"revision":7,
        "microphone":{"state":"stopped","sourceEpoch":2,"interruptions":1,"reason":null},"system":null,
        "microphoneHistory":{"lastSequence":2,"droppedThrough":0,"events":[
            {"sequence":1,"kind":"interrupted","status":{"state":"disconnected","sourceEpoch":1,"interruptions":1,"reason":"device_disconnected"}},
            {"sequence":2,"kind":"recovered","status":{"state":"connected","sourceEpoch":2,"interruptions":1,"reason":null}}
        ]},"systemHistory":null}});
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(output.join("input-events.json"), &bytes).unwrap();
    fs::set_permissions(
        output.join("input-events.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    bytes
}
fn page(
    controller: &ArchivedInputs,
    storage: &Storage,
    lease: &NoticeStorageLease,
    visible: bool,
) -> Value {
    serde_json::to_value(controller.page(storage, lease, None, &|| visible).unwrap()).unwrap()
}
fn ack(
    meeting: &str,
    owner: &str,
    notice: &Value,
    recipient: &str,
    nonce: Option<&str>,
    action: &str,
) -> Acknowledge {
    serde_json::from_value(json!({"meetingId":meeting,"sessionId":owner,"noticeId":notice["id"],
        "recipientId":recipient,"generation":notice["delivery"]["generation"],"action":action,"receiveId":nonce})).unwrap()
}

#[test]
fn database_owned_delivery_is_historical_and_rendered_receipt_survives_process_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, owner, output) = fixture(&storage, true);
    let original = history(&output, &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let first = page(&controller, &storage, &lease, true);
    let batch = &first["recordings"][0]["batch"];
    assert_eq!(batch.as_object().unwrap().len(), 6);
    assert!(batch.get("microphone").is_none());
    assert!(batch.get("reportAgeMs").is_none());
    let notice = &batch["notices"][0];
    assert_eq!(notice["kind"], "interrupted");
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    let received = serde_json::to_value(
        controller
            .acknowledge(
                &storage,
                &lease,
                ack(
                    &meeting,
                    &owner,
                    notice,
                    &recipient,
                    Some(&nonce),
                    "receive",
                ),
                &|| true,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(received["recipientId"], recipient);
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            ack(&meeting, &owner, notice, &recipient, None, "close"),
            &|| true
        )
        .is_err());
    controller
        .acknowledge(
            &storage,
            &lease,
            ack(&meeting, &owner, notice, &recipient, None, "render"),
            &|| true,
        )
        .unwrap();
    drop(controller);
    drop(lease);
    drop(storage);
    let reopened = Storage::open(temp.path().into()).unwrap();
    let next = ArchivedInputs::default();
    let lease = admission.begin_notice_storage().unwrap();
    let after = page(&next, &reopened, &lease, true);
    assert_eq!(
        after["recordings"][0]["batch"]["notices"][0]["kind"],
        "recovered"
    );
    assert!(next
        .acknowledge(
            &reopened,
            &lease,
            ack(&meeting, &owner, notice, &recipient, None, "render"),
            &|| true
        )
        .is_err());
    assert_eq!(
        fs::read(output.join("input-events.json")).unwrap(),
        original
    );
    assert!(!output.join("microphone.caf").exists());
}

#[test]
fn final_writer_failure_survives_archive_restart_without_a_queue_or_an_audio_failure() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (_, owner, output) = fixture(&storage, true);
    let manifest = serde_json::to_vec(&json!({"state":"saved","recordingOwnerId":owner,
        "inputEventJournalPersistence":{"requestedRevision":3,"committedRevision":2,"state":"failed","closed":true}})).unwrap();
    fs::write(output.join("manifest.json"), &manifest).unwrap();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    for _ in 0..2 {
        let controller = ArchivedInputs::default();
        let result = page(&controller, &storage, &lease, true);
        assert_eq!(result["schemaVersion"], 3);
        assert_eq!(result["recordings"][0]["historySaveIssue"], "failed");
        assert!(result["recordings"][0]["batch"].is_null());
        assert!(!output.join("input-notices.json").exists());
    }
    assert_eq!(fs::read(output.join("manifest.json")).unwrap(), manifest);
}

#[test]
fn live_sessions_foreign_pairs_hidden_windows_and_retired_storage_cannot_adopt_writer() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, owner, output) = fixture(&storage, false);
    history(&output, &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    assert!(page(&controller, &storage, &lease, true)["recordings"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(storage.closed_input_directory(&meeting, &owner).is_err());
    assert!(!output.join("input-notices.json").exists());
    storage.fail_recording(&owner).unwrap();
    let hidden = page(&controller, &storage, &lease, false);
    assert!(hidden["recordings"][0]["batch"]["notices"]
        .as_array()
        .unwrap()
        .is_empty());
    let shown = page(&controller, &storage, &lease, true);
    let notice = &shown["recordings"][0]["batch"]["notices"][0];
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    let before = fs::read(output.join("input-notices.json")).unwrap();
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            ack(
                &meeting,
                &owner,
                notice,
                &recipient,
                Some(&nonce),
                "receive"
            ),
            &|| false
        )
        .is_err());
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            ack(
                &Uuid::new_v4().to_string(),
                &owner,
                notice,
                &recipient,
                Some(&nonce),
                "receive"
            ),
            &|| true
        )
        .is_err());
    assert_eq!(fs::read(output.join("input-notices.json")).unwrap(), before);
    admission.require_storage_restart();
    assert!(controller.page(&storage, &lease, None, &|| true).is_err());
    assert_eq!(fs::read(output.join("input-notices.json")).unwrap(), before);
}

#[test]
fn missing_legacy_and_broken_one_owner_do_not_fabricate_empty_success_or_hide_good_history() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (_, legacy, legacy_output) = fixture(&storage, true);
    let (_, broken, broken_output) = fixture(&storage, true);
    let (_, good, good_output) = fixture(&storage, true);
    fs::write(
        broken_output.join("input-events.json"),
        b"private corrupt source",
    )
    .unwrap();
    history(&good_output, &good);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let result = page(&ArchivedInputs::default(), &storage, &lease, true);
    let find = |owner: &str| {
        result["recordings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["sessionId"] == owner)
            .unwrap()
    };
    assert!(find(&legacy)["batch"].is_null());
    assert!(find(&legacy)["issue"].is_null());
    assert!(find(&broken)["batch"].is_null());
    assert_eq!(find(&broken)["issue"], "unavailable");
    assert_eq!(find(&good)["batch"]["notices"].as_array().unwrap().len(), 1);
    assert!(!legacy_output.join("input-notices.lock").exists());
    assert!(!broken_output.join("input-notices.lock").exists());
    assert_eq!(
        fs::read(broken_output.join("input-events.json")).unwrap(),
        b"private corrupt source"
    );
}

#[test]
fn directory_replacement_symlink_and_lost_database_ownership_never_redirect_cached_writer() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, owner, output) = fixture(&storage, true);
    history(&output, &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    page(&controller, &storage, &lease, true);
    let before = fs::read(output.join("input-notices.json")).unwrap();
    let moved = output.with_extension("kept");
    fs::rename(&output, &moved).unwrap();
    fs::create_dir(&output).unwrap();
    history(&output, &owner);
    assert_eq!(
        page(&controller, &storage, &lease, true)["recordings"][0]["issue"],
        "unavailable"
    );
    assert!(!output.join("input-notices.json").exists());
    assert_eq!(fs::read(moved.join("input-notices.json")).unwrap(), before);
    fs::remove_file(output.join("input-events.json")).unwrap();
    fs::remove_dir(&output).unwrap();
    std::os::unix::fs::symlink(&moved, &output).unwrap();
    assert!(storage.closed_input_directory(&meeting, &owner).is_err());
    fs::remove_file(&output).unwrap();
    fs::rename(&moved, &output).unwrap();
    with_test_database(&storage, |db| {
        db.execute(
            "UPDATE recording_sessions SET state='recording' WHERE id=?1",
            [&owner],
        )
        .unwrap()
    });
    assert!(page(&controller, &storage, &lease, true)["recordings"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(fs::read(output.join("input-notices.json")).unwrap(), before);
}

#[test]
fn closed_database_pagination_is_bounded_sorted_and_excludes_active_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let mut expected = Vec::new();
    for _ in 0..35 {
        expected.push(fixture(&storage, true).1);
    }
    fixture(&storage, false);
    expected.sort();
    let (first, cursor) = storage.closed_input_recordings(None).unwrap();
    assert_eq!(first.len(), 32);
    let cursor = cursor.unwrap();
    assert_eq!(first[31].session_id, cursor);
    let (last, end) = storage.closed_input_recordings(Some(&cursor)).unwrap();
    assert_eq!(last.len(), 3);
    assert!(end.is_none());
    let actual = first
        .iter()
        .chain(&last)
        .map(|r| r.session_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert!(storage.closed_input_recordings(Some("../foreign")).is_err());
}

#[test]
fn cache_eviction_is_bounded_and_fences_an_unrendered_old_recipient() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, owner, output) = fixture(&storage, true);
    history(&output, &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let first = page(&controller, &storage, &lease, true);
    let notice = &first["recordings"][0]["batch"]["notices"][0];
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    controller
        .acknowledge(
            &storage,
            &lease,
            ack(
                &meeting,
                &owner,
                notice,
                &recipient,
                Some(&nonce),
                "receive",
            ),
            &|| true,
        )
        .unwrap();
    // Simulate the exact eviction boundary without 128 expensive DB migrations.
    let pinned = storage.closed_input_directory(&meeting, &owner).unwrap();
    {
        let mut entries = controller.0.lock().unwrap();
        let mut active = entries.remove(&owner).unwrap();
        active.touched = Instant::now() - std::time::Duration::from_secs(60);
        // The least recently used owner sorts last: eviction must not depend
        // on UUID order or starve low IDs when more than 128 recordings exist.
        entries.insert("zzzz-oldest".into(), active);
        for n in 1..CACHE_LIMIT {
            entries.insert(
                format!("zz-{n}"),
                Entry {
                    meeting: meeting.clone(),
                    directory: pinned.try_clone().unwrap(),
                    reader: journal::Reader::pin_at(&pinned).unwrap(),
                    queue: None,
                    touched: Instant::now(),
                },
            );
        }
    }
    let later = page(&controller, &storage, &lease, true);
    assert_eq!(controller.0.lock().unwrap().len(), CACHE_LIMIT);
    assert_eq!(
        later["recordings"][0]["batch"]["notices"][0]["delivery"]["generation"],
        2
    );
    assert!(controller
        .acknowledge(
            &storage,
            &lease,
            ack(&meeting, &owner, notice, &recipient, None, "render"),
            &|| true
        )
        .is_err());
}

#[test]
fn response_wait_cannot_freshen_a_claim_and_final_visibility_or_exit_withholds_delivery() {
    use std::{cell::Cell, time::Duration};
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, owner, output) = fixture(&storage, true);
    history(&output, &owner);
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    let first = page(&controller, &storage, &lease, true);
    let notice = &first["recordings"][0]["batch"]["notices"][0];
    let recipient = Uuid::new_v4().to_string();
    let nonce = Uuid::new_v4().to_string();
    controller
        .acknowledge(
            &storage,
            &lease,
            ack(
                &meeting,
                &owner,
                notice,
                &recipient,
                Some(&nonce),
                "receive",
            ),
            &|| true,
        )
        .unwrap();
    let calls = Cell::new(0);
    let slow = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            std::thread::sleep(Duration::from_millis(30));
        }
        true
    };
    let waited =
        serde_json::to_value(controller.page(&storage, &lease, None, &slow).unwrap()).unwrap();
    assert!(
        waited["recordings"][0]["batch"]["notices"][0]["delivery"]["leaseAgeMs"]
            .as_u64()
            .unwrap()
            >= 30
    );
    let calls = Cell::new(0);
    let hidden_at_response = || {
        calls.set(calls.get() + 1);
        calls.get() < 3
    };
    let hidden = serde_json::to_value(
        controller
            .page(&storage, &lease, None, &hidden_at_response)
            .unwrap(),
    )
    .unwrap();
    assert!(hidden["recordings"][0]["batch"]["notices"]
        .as_array()
        .unwrap()
        .is_empty());
    let calls = Cell::new(0);
    let exit_at_response = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            admission.seal_exit().unwrap();
        }
        true
    };
    assert!(controller
        .page(&storage, &lease, None, &exit_at_response)
        .is_err());
    admission.abort_shutdown();
    assert!(lease.validate_active().is_err());
    assert!(controller.page(&storage, &lease, None, &|| true).is_err());
}

#[test]
fn replaced_root_with_no_rows_cannot_return_a_false_empty_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let storage = Storage::open(root.clone()).unwrap();
    let admission = AnalysisController::default();
    let lease = admission.begin_notice_storage().unwrap();
    let controller = ArchivedInputs::default();
    assert!(page(&controller, &storage, &lease, true)["recordings"]
        .as_array()
        .unwrap()
        .is_empty());
    let moved = temp.path().join("kept-data");
    fs::rename(&root, &moved).unwrap();
    fs::create_dir(&root).unwrap();
    assert!(controller.page(&storage, &lease, None, &|| true).is_err());
    assert!(fs::read_dir(&root).unwrap().next().is_none());
    fs::remove_dir(&root).unwrap();
    fs::rename(&moved, &root).unwrap();
}
