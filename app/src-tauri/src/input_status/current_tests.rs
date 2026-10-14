use super::*;
use serde_json::{json, Value};
use std::fs;

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";

fn lane(sequence: u64, input: &str) -> Value {
    let interruptions = sequence / 2 + sequence % 2;
    json!({
        "state": if sequence % 2 == 0 { "connected" } else { "disconnected" },
        "sourceEpoch": sequence / 2 + 1,
        "interruptions": interruptions,
        "reason": if sequence % 2 == 0 { Value::Null } else if input == "microphone" {
            json!("device_disconnected")
        } else {
            json!("stream_stopped")
        }
    })
}

fn history(sequence: u64, input: &str) -> Value {
    let events: Vec<Value> = (1..=sequence)
        .map(|n| {
            json!({
                "sequence": n,
                "kind": if n % 2 == 0 { "recovered" } else { "interrupted" },
                "status": lane(n, input)
            })
        })
        .collect();
    json!({"lastSequence": sequence, "droppedThrough": 0, "events": events})
}

fn publication(sequence: u64) -> Value {
    json!({
        "schemaVersion": 3,
        "ownerId": OWNER,
        "revision": sequence + 1,
        "microphone": lane(sequence, "microphone"),
        "system": lane(sequence, "system"),
        "microphoneHistory": history(sequence, "microphone"),
        "systemHistory": history(sequence, "system")
    })
}

fn write(directory: &Path, value: &Value) {
    fs::write(
        directory.join("input-status.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

#[test]
fn current_producer_format_is_required_without_legacy_notification_fallback() {
    let current = publication(0);
    assert!(parse(&serde_json::to_vec(&current).unwrap(), OWNER).is_ok());
    let mut old = current.clone();
    old["schemaVersion"] = json!(2);
    old.as_object_mut().unwrap().remove("microphoneHistory");
    old.as_object_mut().unwrap().remove("systemHistory");
    assert!(parse(&serde_json::to_vec(&old).unwrap(), OWNER).is_err());
    let mut old_with_history = current.clone();
    old_with_history["schemaVersion"] = json!(2);
    assert!(parse(&serde_json::to_vec(&old_with_history).unwrap(), OWNER).is_err());
    for field in ["microphoneHistory", "systemHistory"] {
        let mut missing = current.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(parse(&serde_json::to_vec(&missing).unwrap(), OWNER).is_err());
    }
    let mut unsupported_system = current;
    unsupported_system["system"] = Value::Null;
    unsupported_system["systemHistory"] = Value::Null;
    assert!(parse(&serde_json::to_vec(&unsupported_system).unwrap(), OWNER).is_ok());
}

#[test]
fn current_reader_keeps_publication_age_and_rejects_revision_rewrite() {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), &publication(0));
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    let first = reader.read(OWNER, Some("main")).unwrap();
    assert_eq!(first.schema_version, 4);
    assert!(first.connected());
    assert!(first.notices.is_empty());
    reader.previous.as_mut().unwrap().1 = 2000;
    write(temp.path(), &publication(0));
    let stale = reader.read(OWNER, Some("recorder")).unwrap();
    assert!(!stale.connected());
    assert!(stale.report_age_ms >= 2000);
    assert!(stale.notices.is_empty());
    let mut altered = publication(0);
    altered["microphone"]["reason"] = json!("unknown");
    write(temp.path(), &altered);
    assert!(reader.read(OWNER, Some("main")).is_err());
    write(temp.path(), &publication(2));
    let resumed = reader.read(OWNER, Some("main")).unwrap();
    assert!(resumed.connected());
    assert_eq!(resumed.schema_version, 4);
    assert_eq!(resumed.notices.len(), 2);
    write(temp.path(), &publication(0));
    assert!(reader.read(OWNER, None).is_err());
}

#[cfg(unix)]
#[test]
fn current_reader_rejects_linked_leaves_and_replaced_directory() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("output");
    fs::create_dir(&output).unwrap();
    write(&output, &publication(0));
    let mut reader = StatusReader::pin(&output.canonicalize().unwrap()).unwrap();
    reader.read(OWNER, None).unwrap();
    let old = temp.path().join("old");
    fs::rename(&output, &old).unwrap();
    fs::create_dir(&output).unwrap();
    write(&output, &publication(2));
    assert_eq!(reader.read(OWNER, None).unwrap().revision, 1);
    let leaf = old.join("input-status.json");
    let held = old.join("held.json");
    fs::rename(&leaf, &held).unwrap();
    symlink(&held, &leaf).unwrap();
    assert!(reader.read(OWNER, None).is_err());
    fs::remove_file(&leaf).unwrap();
    fs::hard_link(&held, &leaf).unwrap();
    assert!(reader.read(OWNER, None).is_err());
    let linked_directory = temp.path().join("linked");
    symlink(&old, &linked_directory).unwrap();
    assert!(StatusReader::pin(&linked_directory).is_err());
}

#[test]
fn visible_window_selection_remains_explicit() {
    assert_eq!(notice_target(true, true), Some("main"));
    assert_eq!(notice_target(false, true), Some("recorder"));
    assert_eq!(notice_target(false, false), None);
    assert!(notice_window_visible(Some(true), Some(false)));
    assert!(!notice_window_visible(Some(true), Some(true)));
    assert!(!notice_window_visible(None, Some(false)));
}

#[test]
#[ignore = "Swift-owned fixture; run make check-input-status without audio capture"]
fn swift_generated_input_status_is_owner_bound_and_never_pcm_proof() {
    let path = std::path::PathBuf::from(
        std::env::var("MON_INPUT_STATUS_FIXTURE_DIR").expect("Swift fixture directory"),
    );
    let before = fs::read(path.join("input-status.json")).unwrap();
    let mut reader = StatusReader::pin(&path.canonicalize().unwrap()).unwrap();
    let result = reader.read(OWNER, Some("main")).unwrap();
    assert_eq!(result.schema_version, 4);
    assert_eq!(result.revision, 4);
    assert_eq!(result.microphone.source_epoch, 2);
    assert_eq!(result.microphone.interruptions, 1);
    assert_eq!(result.system.as_ref().unwrap().source_epoch, 2);
    assert_eq!(result.system.as_ref().unwrap().interruptions, 1);
    let fresh = result.report_age_ms <= FRESH_MS;
    assert_eq!(result.connected(), fresh);
    if fresh {
        assert_eq!(result.notices.len(), 2);
        // The durable queue offers the first unrendered interruption before
        // the later recovery; a heartbeat must not skip the earlier event.
        assert!(result
            .notices
            .iter()
            .all(|notice| notice.kind == "interrupted"));
        assert!(result.system_connected());
    } else {
        assert!(result.notices.is_empty());
        assert!(!result.system_connected());
    }
    let other = reader.read(OWNER, Some("recorder")).unwrap();
    if other.report_age_ms <= FRESH_MS {
        assert_eq!(other.notices.len(), result.notices.len());
        for (previous, next) in result.notices.iter().zip(&other.notices) {
            // Offering is not receiving. An unclaimed notice follows the
            // currently visible window with a new generation, but keeps its
            // stable event identity and cannot be counted twice.
            assert_eq!(next.id, previous.id);
            assert_eq!(next.target, "recorder");
            assert!(next.recipient_id.is_none());
            let generation = |notice: &Notice| {
                serde_json::to_value(&notice.delivery).unwrap()["generation"]
                    .as_u64()
                    .unwrap()
            };
            assert_eq!(generation(next), generation(previous) + 1);
        }
    } else {
        assert!(other.notices.is_empty());
    }
    assert_eq!(before, fs::read(path.join("input-status.json")).unwrap());
    assert!(!path.join("microphone.caf").exists());
    assert!(!path.join("system.caf").exists());
}
