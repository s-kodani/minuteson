//! Real owned cutovers and cold admission; only synthetic closed-recording
//! evidence. No device capture, product launch or portability claim.
use super::*;
use crate::{
    analysis_worker::data_maintenance::NoticeStorageLease,
    input_status::archived::{Acknowledge, ArchivedInputs},
    storage::RecordingConfig,
};
use serde_json::{json, Value};

fn row(
    controller: &ArchivedInputs,
    storage: &Storage,
    lease: &NoticeStorageLease,
    owner: &str,
) -> Value {
    let page =
        serde_json::to_value(controller.page(storage, lease, None, &|| true).unwrap()).unwrap();
    assert_eq!(page["schemaVersion"], 2);
    page["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sessionId"] == owner)
        .unwrap()
        .clone()
}
fn notice(row: &Value, input: &str) -> Value {
    row["batch"]["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["input"] == input)
        .unwrap()
        .clone()
}
fn request(
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
fn seed(storage: &Storage) -> (String, String, PathBuf, Vec<u8>, Vec<u8>) {
    let meeting = storage
        .create_meeting("SYNTHETIC CLOSED INPUT COMPANIONS")
        .unwrap()
        .id;
    let owner = storage
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
    storage.fail_recording(&owner).unwrap();
    let directory = storage
        .root()
        .join("meetings")
        .join(&meeting)
        .join("audio")
        .join(&owner);
    std::fs::create_dir_all(&directory).unwrap();
    let history =
        |reason: &str| {
            // The producer retains exactly the latest 64 transitions. Two lost
            // transitions therefore require sequence 66 and a complete 3..=66
            // suffix, not an arbitrarily shortened four-transition history.
            let events: Vec<_> = (3_u64..=66).map(|sequence| {
            let interrupted = sequence % 2 == 1;
            json!({"sequence":sequence,"kind":if interrupted {"interrupted"} else {"recovered"},
                "status":{"state":if interrupted {"disconnected"} else {"connected"},
                    "sourceEpoch":sequence,"interruptions":sequence / 2 + sequence % 2,
                    "reason":if interrupted {Some(reason)} else {None}}})
        }).collect();
            json!({"lastSequence":66,"droppedThrough":2,"events":events})
        };
    let stopped = json!({"state":"stopped","sourceEpoch":66,"interruptions":33,"reason":null});
    let journal = serde_json::to_vec(&json!({"schemaVersion":1,"sealed":true,"report":{
        "schemaVersion":3,"ownerId":owner,"revision":201,"microphone":stopped,"system":stopped,
        "microphoneHistory":history("device_disconnected"),"systemHistory":history("stream_stopped")}})).unwrap();
    let manifest = serde_json::to_vec(&json!({"state":"failed","recordingOwnerId":owner,
        "inputEventJournalPersistence":{"requestedRevision":201,"committedRevision":200,"state":"failed","closed":true}})).unwrap();
    for (name, bytes) in [
        ("input-events.json", &journal),
        ("manifest.json", &manifest),
    ] {
        std::fs::write(directory.join(name), bytes).unwrap();
        std::fs::set_permissions(directory.join(name), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    storage
        .recording_input_history(&meeting, &owner)
        .expect("the synthetic producer suffix must pass the real retained-history reader");
    (meeting, owner, directory, journal, manifest)
}

#[test]
fn closed_input_companions_two_owned_moves_keep_loss_and_diagnostics_and_fence_old_receipts() {
    for rendered in [false, true] {
        let mut f = NativeFixture::new(false, false);
        let (meeting, owner, source, journal, manifest) = seed(f.storage.as_ref().unwrap());
        let admission = AnalysisController::default();
        let original = ArchivedInputs::default();
        let lease = admission.begin_notice_storage().unwrap();
        let initial = row(&original, f.storage.as_ref().unwrap(), &lease, &owner);
        assert_eq!(initial["historySaveIssue"], "failed");
        assert!(initial["issue"].is_null());
        let loss = initial["batch"]["noticeLoss"].clone();
        assert_eq!(loss, json!({"microphone":2,"system":2}));
        assert!(initial["batch"].get("reportAgeMs").is_none());
        let first = notice(&initial, "microphone");
        let recipient = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().to_string();
        original
            .acknowledge(
                f.storage.as_ref().unwrap(),
                &lease,
                request(
                    &meeting,
                    &owner,
                    &first,
                    &recipient,
                    Some(&nonce),
                    "receive",
                ),
                &|| true,
            )
            .unwrap();
        if rendered {
            original
                .acknowledge(
                    f.storage.as_ref().unwrap(),
                    &lease,
                    request(&meeting, &owner, &first, &recipient, None, "render"),
                    &|| true,
                )
                .unwrap();
        }
        let source_ledger = std::fs::read(source.join("input-notices.json")).unwrap();
        drop(lease);
        let maintenance = admission
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        f.select();
        drop(maintenance);

        let (b, storage) = f.open();
        let moved = storage
            .root()
            .join("meetings")
            .join(&meeting)
            .join("audio")
            .join(&owner);
        // The actual copied bytes must be intact BEFORE another writer opens.
        assert_eq!(
            std::fs::read(moved.join("input-notices.json")).unwrap(),
            source_ledger
        );
        assert_eq!(
            std::fs::read(moved.join("input-events.json")).unwrap(),
            journal
        );
        assert_eq!(
            std::fs::read(moved.join("manifest.json")).unwrap(),
            manifest
        );
        let lease = admission.begin_notice_storage().unwrap();
        assert!(original
            .acknowledge(
                &storage,
                &lease,
                request(&meeting, &owner, &first, &recipient, None, "render"),
                &|| true
            )
            .is_err());
        assert_eq!(
            std::fs::read(moved.join("input-notices.json")).unwrap(),
            source_ledger
        );
        let current = ArchivedInputs::default();
        let after = row(&current, &storage, &lease, &owner);
        assert_eq!(after["historySaveIssue"], "failed");
        assert_eq!(after["batch"]["noticeLoss"], loss);
        let pending = notice(&after, "microphone");
        assert_eq!(
            pending["kind"],
            if rendered { "recovered" } else { "interrupted" }
        );
        assert!(pending["recipientId"].is_null());
        if !rendered {
            assert!(
                pending["delivery"]["generation"].as_u64().unwrap()
                    > first["delivery"]["generation"].as_u64().unwrap()
            );
        }
        let before_old_ack = std::fs::read(moved.join("input-notices.json")).unwrap();
        assert!(current
            .acknowledge(
                &storage,
                &lease,
                request(&meeting, &owner, &first, &recipient, None, "render"),
                &|| true
            )
            .is_err());
        assert_eq!(
            std::fs::read(moved.join("input-notices.json")).unwrap(),
            before_old_ack
        );
        let second_recipient = Uuid::new_v4().to_string();
        let second_nonce = Uuid::new_v4().to_string();
        current
            .acknowledge(
                &storage,
                &lease,
                request(
                    &meeting,
                    &owner,
                    &pending,
                    &second_recipient,
                    Some(&second_nonce),
                    "receive",
                ),
                &|| true,
            )
            .unwrap();
        let next_ledger = std::fs::read(moved.join("input-notices.json")).unwrap();
        drop(lease);
        let maintenance = admission
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let next = f
            ._base
            .path()
            .join(format!("input-companions-move-{}", Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&next)
            .unwrap();
        let next = next.canonicalize().unwrap();
        repeated::select_current(&b, &storage, &next);
        drop(maintenance);
        let lease = admission.begin_notice_storage().unwrap();
        assert!(storage.restart_required());
        assert!(current
            .acknowledge(
                &storage,
                &lease,
                request(
                    &meeting,
                    &owner,
                    &pending,
                    &second_recipient,
                    None,
                    "render"
                ),
                &|| true
            )
            .is_err());
        assert_eq!(
            std::fs::read(moved.join("input-notices.json")).unwrap(),
            next_ledger
        );
        drop(lease);
        drop(current);
        drop(storage);
        drop(b);
        drop(original);

        let (_b, storage) = f.open();
        assert_eq!(storage.root(), next);
        let final_dir = next
            .join("meetings")
            .join(&meeting)
            .join("audio")
            .join(&owner);
        assert_eq!(
            std::fs::read(final_dir.join("input-notices.json")).unwrap(),
            next_ledger
        );
        assert_eq!(
            std::fs::read(final_dir.join("input-events.json")).unwrap(),
            journal
        );
        assert_eq!(
            std::fs::read(final_dir.join("manifest.json")).unwrap(),
            manifest
        );
        let lease = admission.begin_notice_storage().unwrap();
        let final_controller = ArchivedInputs::default();
        let final_row = row(&final_controller, &storage, &lease, &owner);
        assert_eq!(final_row["historySaveIssue"], "failed");
        assert_eq!(final_row["batch"]["noticeLoss"], loss);
        let final_notice = notice(&final_row, "microphone");
        assert_eq!(final_notice["kind"], pending["kind"]);
        assert_eq!(notice(&final_row, "system")["kind"], "interrupted");
        assert_eq!(notice(&final_row, "system")["delivery"]["sequence"], 3);
        assert!(
            final_notice["delivery"]["generation"].as_u64().unwrap()
                > pending["delivery"]["generation"].as_u64().unwrap()
        );
        assert!(final_notice["recipientId"].is_null());
        let before_old_ack = std::fs::read(final_dir.join("input-notices.json")).unwrap();
        assert!(final_controller
            .acknowledge(
                &storage,
                &lease,
                request(
                    &meeting,
                    &owner,
                    &pending,
                    &second_recipient,
                    None,
                    "render"
                ),
                &|| true
            )
            .is_err());
        assert_eq!(
            std::fs::read(final_dir.join("input-notices.json")).unwrap(),
            before_old_ack
        );
        let final_recipient = Uuid::new_v4().to_string();
        let final_nonce = Uuid::new_v4().to_string();
        for (action, nonce) in [
            ("receive", Some(final_nonce.as_str())),
            ("render", None),
            ("close", None),
        ] {
            final_controller
                .acknowledge(
                    &storage,
                    &lease,
                    request(
                        &meeting,
                        &owner,
                        &final_notice,
                        &final_recipient,
                        nonce,
                        action,
                    ),
                    &|| true,
                )
                .unwrap();
        }
        let closed = row(&final_controller, &storage, &lease, &owner);
        assert_eq!(closed["historySaveIssue"], "failed");
        assert_eq!(closed["batch"]["noticeLoss"], loss);
        assert_eq!(notice(&closed, "system")["kind"], "interrupted");
        let remaining_microphone: Vec<_> = closed["batch"]["notices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["input"] == "microphone")
            .collect();
        assert_eq!(remaining_microphone.len(), 1);
        assert_eq!(
            remaining_microphone[0]["kind"],
            if rendered { "interrupted" } else { "recovered" }
        );
        assert_eq!(
            remaining_microphone[0]["delivery"]["sequence"],
            if rendered { 5 } else { 4 }
        );
        let retained =
            serde_json::to_value(storage.recording_input_history(&meeting, &owner).unwrap())
                .unwrap();
        let producer: Value = serde_json::from_slice(&journal).unwrap();
        assert_eq!(retained["ownerId"], owner);
        assert_eq!(retained["revision"], 201);
        assert_eq!(retained["sealed"], true);
        assert_eq!(
            retained["microphoneHistory"],
            producer["report"]["microphoneHistory"]
        );
        assert_eq!(
            retained["systemHistory"],
            producer["report"]["systemHistory"]
        );
        assert!(retained.get("microphone").is_none());
        assert!(retained.get("reportAgeMs").is_none());
        assert_eq!(
            std::fs::read(final_dir.join("input-events.json")).unwrap(),
            journal
        );
        assert_eq!(
            std::fs::read(final_dir.join("manifest.json")).unwrap(),
            manifest
        );
        assert_eq!(
            std::fs::read(source.join("input-notices.json")).unwrap(),
            source_ledger
        );
        assert_eq!(
            std::fs::read(source.join("input-events.json")).unwrap(),
            journal
        );
        assert!(!final_dir.join("microphone.caf").exists());
        assert!(!final_dir.join("system.caf").exists());
        assert_eq!(storage.get_meeting(&meeting).unwrap().unwrap().id, meeting);
        storage
            .create_meeting("CURRENT ROOT STILL WRITABLE")
            .unwrap();
    }
}
