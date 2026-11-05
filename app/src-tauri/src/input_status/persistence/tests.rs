use super::*;
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt};
const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
fn report(revision: u64, requested: u64, committed: u64, state: &str) -> Value {
    json!({"schemaVersion":1,"ownerId":OWNER,"revision":revision,"persistence":{
        "requestedRevision":requested,"committedRevision":committed,"state":state,"closed":false}})
}
fn write(dir: &std::path::Path, name: &str, value: &Value) {
    fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn journal_diagnostics_closed_wire_and_independent_revision_domains() {
    let good = report(2, 999, 990, "pending");
    assert!(parse(&serde_json::to_vec(&good).unwrap(), OWNER).is_ok());
    for bad in [
        report(0, 2, 1, "pending"),
        report(2, 2, 3, "confirmed"),
        report(2, 2, 1, "confirmed"),
        report(2, 2, 1, "saved"),
    ] {
        assert!(parse(&serde_json::to_vec(&bad).unwrap(), OWNER).is_err());
    }
    let duplicate = serde_json::to_string(&good).unwrap().replacen(
        "\"revision\":2",
        "\"revision\":2,\"revision\":3",
        1,
    );
    assert!(parse(duplicate.as_bytes(), OWNER).is_err());
    let mut extra = good.clone();
    extra["audioSaved"] = json!(true);
    assert!(parse(&serde_json::to_vec(&extra).unwrap(), OWNER).is_err());
    assert!(parse(
        &serde_json::to_vec(&good).unwrap(),
        "63500000-0000-4000-8000-000000000001"
    )
    .is_err());
}
#[test]
fn journal_diagnostics_readback_cannot_refresh_same_heartbeat_or_clear_invalid_floor() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    let mut reader = Reader::default();
    write(
        temp.path(),
        "input-journal-status.json",
        &report(1, 2, 1, "failed"),
    );
    let first = reader.read(&directory, OWNER).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    write(
        temp.path(),
        "input-journal-status.json",
        &report(1, 2, 1, "failed"),
    );
    assert!(reader.read(&directory, OWNER).unwrap().report_age_ms > first.report_age_ms);
    write(
        temp.path(),
        "input-journal-status.json",
        &report(1, 2, 2, "confirmed"),
    );
    assert!(reader.read(&directory, OWNER).is_err());
    write(
        temp.path(),
        "input-journal-status.json",
        &report(2, 2, 2, "confirmed"),
    );
    assert!(reader.read(&directory, OWNER).is_ok());
    write(
        temp.path(),
        "input-journal-status.json",
        &report(3, 3, 2, "invalid"),
    );
    assert!(reader.read(&directory, OWNER).is_ok());
    write(
        temp.path(),
        "input-journal-status.json",
        &report(4, 3, 3, "confirmed"),
    );
    assert!(reader.read(&directory, OWNER).is_err());
    write(
        temp.path(),
        "input-journal-status.json",
        &report(4, 3, 1, "invalid"),
    );
    assert!(reader.read(&directory, OWNER).is_err());
}
#[test]
fn journal_diagnostics_reject_links_and_oversize_without_modifying_foreign_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    let foreign = temp.path().join("foreign");
    fs::write(&foreign, b"preserve").unwrap();
    let leaf = temp.path().join("input-journal-status.json");
    std::os::unix::fs::symlink(&foreign, &leaf).unwrap();
    assert!(Reader::default().read(&directory, OWNER).is_err());
    fs::remove_file(&leaf).unwrap();
    fs::hard_link(&foreign, &leaf).unwrap();
    assert!(Reader::default().read(&directory, OWNER).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
    fs::remove_file(&leaf).unwrap();
    fs::write(&leaf, vec![b' '; 4097]).unwrap();
    assert!(Reader::default().read(&directory, OWNER).is_err());
}
#[test]
fn journal_diagnostics_stop_errors_are_not_audio_failure_or_optimistic_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    write(
        temp.path(),
        "input-events.json",
        &json!({"schemaVersion":1,"sealed":true,"report":{
        "schemaVersion":3,"ownerId":OWNER,"revision":3,
        "microphone":{"state":"stopped","sourceEpoch":0,"interruptions":0,"reason":null},"system":null,
        "microphoneHistory":{"lastSequence":0,"droppedThrough":0,"events":[]},"systemHistory":null}}),
    );
    for (state, expected) in [
        ("confirmed", None),
        ("pending", Some("unconfirmed")),
        ("failed", Some("failed")),
        ("invalid", Some("invalid")),
    ] {
        let mut p =
            report(1, 3, if state == "confirmed" { 3 } else { 2 }, state)["persistence"].clone();
        p["closed"] = json!(true);
        write(
            temp.path(),
            "manifest.json",
            &json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"saved","recordingOwnerId":OWNER,"inputEventJournalPersistence":p}),
        );
        assert_eq!(archived_issue(&directory, OWNER), expected);
    }
    let confirmed =
        json!({"requestedRevision":3,"committedRevision":3,"state":"confirmed","closed":true});
    write(
        temp.path(),
        "manifest.json",
        &json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"saved","recordingOwnerId":OWNER,"inputEventJournalPersistence":confirmed,"inputEventJournalError":"/private/secret: timed out"}),
    );
    assert_eq!(archived_issue(&directory, OWNER), Some("unconfirmed"));
    write(
        temp.path(),
        "manifest.json",
        &json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"saved","recordingOwnerId":"foreign","inputEventJournalPersistence":confirmed}),
    );
    assert_eq!(archived_issue(&directory, OWNER), Some("unavailable"));
    write(temp.path(), "manifest.json", &json!({"state":"saved"}));
    assert_eq!(archived_issue(&directory, OWNER), Some("unavailable"));
    let mut unsealed: Value =
        serde_json::from_slice(&fs::read(temp.path().join("input-events.json")).unwrap()).unwrap();
    unsealed["sealed"] = json!(false);
    write(temp.path(), "input-events.json", &unsealed);
    assert_eq!(archived_issue(&directory, OWNER), Some("unavailable"));
    fs::write(temp.path().join("manifest.json"),format!("{{\"state\":\"saved\",\"recordingOwnerId\":\"{OWNER}\",\"inputEventJournalError\":\"first\",\"inputEventJournalError\":null}}")).unwrap();
    assert_eq!(archived_issue(&directory, OWNER), Some("unavailable"));
}
#[test]
#[ignore = "requires the dedicated headless Swift-generated journal fixture"]
fn swift_generated_input_journal_status_has_separate_writer_result_and_heartbeat() {
    let path = std::env::var("MON_INPUT_JOURNAL_FIXTURE_DIR").expect("dedicated fixture");
    let data = fs::read(std::path::Path::new(&path).join("input-journal-status.json")).unwrap();
    let report = parse(&data, OWNER).unwrap();
    assert_eq!(report.revision, 123);
    assert_eq!(report.persistence.state, State::Confirmed);
    assert!(report.persistence.closed);
    assert_eq!(
        report.persistence.requested_revision,
        report.persistence.committed_revision
    );
    let history = super::super::journal::parse(
        &fs::read(std::path::Path::new(&path).join("input-events.json")).unwrap(),
        OWNER,
    )
    .unwrap();
    assert_eq!(
        report.persistence.committed_revision,
        history.report.revision
    );
}
