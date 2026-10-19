use super::*;
use serde_json::{json, Value};

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
fn envelope() -> Value {
    let status = |lane| {
        json!({"schemaVersion":2,"ownerId":OWNER,"lane":lane,"sealedSegments":0,
        "sealedFrames":0,"lastEntrySHA256":null,"producerClosed":false,"failure":null})
    };
    json!({"schemaVersion":4,"inputDiagnosticsState":"unavailable","recordingOwnerId":OWNER,"state":"failed","errors":[],
        "microphone":{"rawSegments":status("microphone")},"system":{"rawSegments":status("system")}})
}
fn large(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.resize(SMALL_MANIFEST_BYTES as usize + 1, b' ');
    bytes
}
#[test]
#[cfg(unix)]
fn capture_manifest_capacity_requires_both_native_lanes_without_authorizing_saved_audio() {
    let good = envelope();
    assert!(validate_size(&large(&good)).is_ok());
    for pointer in [
        "/schemaVersion",
        "/recordingOwnerId",
        "/microphone/rawSegments/ownerId",
        "/system/rawSegments/lane",
        "/system/rawSegments/schemaVersion",
        "/system/rawSegments/sealedSegments",
        "/system/rawSegments/sealedFrames",
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(validate_size(&large(&bad)).is_err(), "{pointer}");
    }
    for lane in ["microphone", "system"] {
        let mut bad = good.clone();
        bad[lane].as_object_mut().unwrap().remove("rawSegments");
        assert!(validate_size(&large(&bad)).is_err());
    }
    assert!(validate_size(&large(&json!({"state":"saved"}))).is_err());
    assert!(validate_size(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
    assert!(validate_size(&[]).is_err());
}

#[test]
#[cfg(unix)]
fn capture_manifest_capacity_preserves_duplicate_known_field_rejection() {
    let good = serde_json::to_string(&envelope()).unwrap();
    for duplicate in [
        "\"schemaVersion\":4",
        "\"recordingOwnerId\":\"735027d4-a03e-43d3-830a-21e767dfaf88\"",
    ] {
        let mut bytes = format!("{{{duplicate},{}", &good[1..]).into_bytes();
        bytes.resize(SMALL_MANIFEST_BYTES as usize + 1, b' ');
        assert!(validate_size(&bytes).is_err());
    }
}
