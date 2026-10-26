use super::*;
use crate::storage::RecordingConfig;
use serde_json::{json, Value};

fn fixture(storage: &Storage) -> (String, String, std::path::PathBuf) {
    let meeting = storage.create_meeting("通知履歴の合成会議").unwrap().id;
    let owner = storage
        .prepare_recording(
            &meeting,
            &RecordingConfig {
                mode: "system".into(),
                backend: "wasapi".into(),
                target_pids: vec![],
                microphone_device_uid: None,
                target_application_identity: None,
            },
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
        .id;
    let output = storage
        .root()
        .join("meetings")
        .join(&meeting)
        .join("audio")
        .join(&owner);
    std::fs::create_dir_all(&output).unwrap();
    crate::windows_files::set_private(&output).unwrap();
    storage.fail_recording(&owner).unwrap();
    let history = json!({"schemaVersion":1,"sealed":true,"report":{
        "schemaVersion":3,"ownerId":owner,"revision":7,
        "microphone":{"state":"stopped","sourceEpoch":2,"interruptions":1,"reason":null},"system":null,
        "microphoneHistory":{"lastSequence":2,"droppedThrough":0,"events":[
            {"sequence":1,"kind":"interrupted","status":{"state":"disconnected","sourceEpoch":1,"interruptions":1,"reason":"device_disconnected"}},
            {"sequence":2,"kind":"recovered","status":{"state":"connected","sourceEpoch":2,"interruptions":1,"reason":null}}
        ]},"systemHistory":null}});
    let path = output.join("input-events.json");
    std::fs::write(&path, serde_json::to_vec(&history).unwrap()).unwrap();
    crate::windows_files::set_private(&path).unwrap();
    (meeting, owner, output)
}

#[test]
fn scoped_history_pause_keeps_other_meetings_and_never_recreates_deleted_files() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let (meeting, owner, output) = fixture(&storage);
    let (other, _, _) = fixture(&storage);
    let analysis = AnalysisController::default();
    let archive = ArchivedInputs::default();
    let page = serde_json::to_value(
        archive
            .page_scoped(&storage, &analysis, None, &|| true)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page["recordings"].as_array().unwrap().len(), 2);
    assert!(page["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["batch"].is_object()));
    let lease = analysis.begin_meeting_deletion(&storage, &meeting).unwrap();
    archive.evict(Some(&meeting)).unwrap();
    std::fs::remove_dir_all(&output).unwrap();
    let page = serde_json::to_value(
        archive
            .page_scoped(&storage, &analysis, None, &|| true)
            .unwrap(),
    )
    .unwrap();
    let rows = page["recordings"].as_array().unwrap();
    let paused = rows.iter().find(|r| r["meetingId"] == meeting).unwrap();
    assert_eq!(paused["issue"], "paused");
    assert!(paused["batch"].is_null());
    assert!(rows.iter().find(|r| r["meetingId"] == other).unwrap()["batch"].is_object());
    assert!(!output.exists());
    assert!(analysis
        .try_begin_scoped_notice_storage(Some(&meeting))
        .unwrap()
        .is_none());
    storage.release_meeting_deletion(&meeting).unwrap();
    drop(lease);
    let writer = analysis
        .begin_scoped_notice_storage(Some(&meeting))
        .unwrap();
    let request: Acknowledge=serde_json::from_value(json!({"meetingId":meeting,"sessionId":owner,"noticeId":"old-notice","recipientId":uuid::Uuid::new_v4().to_string(),"generation":1,"action":"receive","receiveId":uuid::Uuid::new_v4().to_string()})).unwrap();
    assert!(archive
        .acknowledge(&storage, &writer, request, &|| true)
        .is_err());
    assert!(!output.exists());
}

#[test]
fn delayed_other_meeting_history_does_not_hold_the_cache_registry() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let (meeting, owner, _) = fixture(&storage);
    let (other, _, _) = fixture(&storage);
    let analysis = AnalysisController::default();
    let archive = ArchivedInputs::default();
    archive
        .page_scoped(&storage, &analysis, None, &|| true)
        .unwrap();
    let handle = archive.0.lock().unwrap().get(&owner).unwrap().value.clone();
    let held = handle.lock().unwrap();
    archive.evict(Some(&other)).unwrap();
    let page: Value = serde_json::to_value(
        archive
            .page_scoped(&storage, &analysis, Some(&owner), &|| false)
            .unwrap(),
    )
    .unwrap();
    assert!(page["recordings"].is_array());
    drop(held);
    assert_eq!(
        archive.0.lock().unwrap().get(&owner).unwrap().meeting,
        meeting
    );
}
