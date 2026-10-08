use super::*;
use serde_json::{json, Value};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";

pub(crate) fn record(owner: &str, sequence: u64, sealed: bool) -> Value {
    let lane = |name: &str| {
        let status = |seq: u64| {
            let recovered = seq % 2 == 0;
            json!({"state":if recovered {"connected"} else {"disconnected"},
                "sourceEpoch":seq/2+1,"interruptions":seq/2+seq%2,
                "reason":if recovered {Value::Null} else {json!(if name == "microphone" {"device_disconnected"} else {"stream_stopped"})}})
        };
        let floor = sequence.saturating_sub(64);
        let events: Vec<_> = (floor + 1..=sequence)
            .map(|seq| {
                json!({"sequence":seq,
            "kind":if seq%2 == 0 {"recovered"} else {"interrupted"},"status":status(seq)})
            })
            .collect();
        let mut current = status(sequence);
        if sealed {
            current["state"] = json!("stopped");
        }
        (
            current,
            json!({"lastSequence":sequence,"droppedThrough":floor,"events":events}),
        )
    };
    let (microphone, mic_history) = lane("microphone");
    let (system, sys_history) = lane("system");
    json!({"schemaVersion":1,"sealed":sealed,"report":{
        "schemaVersion":3,"ownerId":owner,"revision":sequence+2,
        "microphone":microphone,"system":system,
        "microphoneHistory":mic_history,"systemHistory":sys_history}})
}

fn write(path: &Path, value: &Value) {
    let leaf = path.join("input-events.json");
    fs::write(&leaf, serde_json::to_vec(value).unwrap()).unwrap();
    #[cfg(unix)]
    fs::set_permissions(leaf, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn retained_suffix_and_eviction_floor_are_not_a_live_status_or_display_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let mut reader = Reader::pin(&directory.path().canonicalize().unwrap()).unwrap();
    write(directory.path(), &record(OWNER, 130, true));
    let dto = serde_json::to_value(reader.read(OWNER).unwrap()).unwrap();
    assert_eq!(dto["sealed"], true);
    assert_eq!(dto["microphoneHistory"]["lastSequence"], 130);
    assert_eq!(dto["microphoneHistory"]["droppedThrough"], 66);
    assert_eq!(
        dto["microphoneHistory"]["events"].as_array().unwrap().len(),
        64
    );
    assert_eq!(dto.as_object().unwrap().len(), 6);
    for absent in ["reportAgeMs", "microphone", "system", "notices", "rendered"] {
        assert!(dto.get(absent).is_none());
    }
    // Retained history, however recently rewritten, must never enter the live
    // status file's connected/freshness path.
    let mut live =
        crate::input_status::StatusReader::pin(&directory.path().canonicalize().unwrap()).unwrap();
    assert!(live.read(OWNER, Some("main")).is_err());
    assert!(reader.read(OWNER).is_ok());
}

#[test]
fn retained_format_requires_exact_fields_owner_real_history_and_closed_seal() {
    let base = record(OWNER, 2, true);
    let mut invalid = Vec::new();
    for (key, value) in [
        ("schemaVersion", json!(2)),
        ("sealed", json!("yes")),
        ("extra", json!(true)),
    ] {
        let mut v = base.clone();
        v[key] = value;
        invalid.push(v);
    }
    for key in ["schemaVersion", "sealed", "report"] {
        let mut v = base.clone();
        v.as_object_mut().unwrap().remove(key);
        invalid.push(v);
    }
    for key in ["system", "systemHistory", "microphoneHistory"] {
        let mut v = base.clone();
        v["report"].as_object_mut().unwrap().remove(key);
        invalid.push(v);
    }
    for (key, value) in [
        ("ownerId", json!(uuid::Uuid::new_v4().to_string())),
        ("revision", json!(0)),
        ("schemaVersion", json!(2)),
        ("extra", json!(true)),
    ] {
        let mut v = base.clone();
        v["report"][key] = value;
        invalid.push(v);
    }
    for lane in ["microphone", "system"] {
        let mut v = base.clone();
        v["report"][lane]["state"] = json!("connected");
        invalid.push(v);
        let mut v = base.clone();
        v["report"][format!("{lane}History")]["events"][0]["status"]
            .as_object_mut()
            .unwrap()
            .remove("reason");
        invalid.push(v);
        let mut v = base.clone();
        v["report"][format!("{lane}History")]["events"][1]["sequence"] = json!(1);
        invalid.push(v);
    }
    for value in invalid {
        assert!(
            parse(&serde_json::to_vec(&value).unwrap(), OWNER).is_err(),
            "{value}"
        );
    }
    for (key, value) in [
        ("schemaVersion", "1"),
        ("sealed", "true"),
        ("ownerId", &format!("\"{OWNER}\"")),
        ("reason", "null"),
        ("sequence", "1"),
    ] {
        let bytes = serde_json::to_string(&base).unwrap();
        let needle = format!("\"{key}\":{value}");
        let duplicate = format!("{needle},{needle}");
        let doubled = bytes.replacen(&needle, &duplicate, 1);
        // sequence=1 and reason=null occur in the real event array; test original-byte parsing.
        assert_ne!(bytes, doubled);
        assert!(parse(doubled.as_bytes(), OWNER).is_err());
    }
    assert!(parse(&vec![b' '; LIMIT as usize + 1], OWNER).is_err());
    assert!(parse(&[], OWNER).is_err());
}

#[test]
fn reader_rejects_revision_rewrite_rollback_historical_changes_and_unsealing() {
    for changed in [
        "revision",
        "same-revision",
        "overlap",
        "unseal",
        "sealed-revision",
        "scope",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut reader = Reader::pin(&directory.path().canonicalize().unwrap()).unwrap();
        let original = record(OWNER, 2, matches!(changed, "unseal" | "sealed-revision"));
        write(directory.path(), &original);
        reader.read(OWNER).unwrap();
        let mut next = record(OWNER, 4, changed == "unseal");
        match changed {
            "revision" => next["report"]["revision"] = json!(1),
            "same-revision" => next["report"]["revision"] = original["report"]["revision"].clone(),
            "overlap" => {
                next["report"]["microphoneHistory"]["events"][0]["status"]["reason"] =
                    json!("engine_changed")
            }
            "unseal" => {
                next = original.clone();
                next["sealed"] = json!(false);
                next["report"]["revision"] = json!(100);
            }
            "sealed-revision" => {
                next = original.clone();
                next["report"]["revision"] = json!(100);
            }
            "scope" => {
                next["report"]["system"] = Value::Null;
                next["report"]["systemHistory"] = Value::Null;
            }
            _ => unreachable!(),
        }
        write(directory.path(), &next);
        assert!(reader.read(OWNER).is_err(), "{changed}");
        write(directory.path(), &original);
        reader.read(OWNER).unwrap(); // failed candidate never changes the floor
    }
}

#[cfg(unix)]
#[test]
fn retained_reader_refuses_links_public_files_oversize_and_ancestor_replacement() {
    for kind in ["symlink", "hardlink", "mode", "oversize"] {
        let directory = tempfile::tempdir().unwrap();
        let mut reader = Reader::pin(&directory.path().canonicalize().unwrap()).unwrap();
        write(directory.path(), &record(OWNER, 2, true));
        let leaf = directory.path().join("input-events.json");
        let foreign = directory.path().join("foreign.json");
        match kind {
            "symlink" => {
                fs::rename(&leaf, &foreign).unwrap();
                symlink(&foreign, &leaf).unwrap();
            }
            "hardlink" => fs::hard_link(&leaf, &foreign).unwrap(),
            "mode" => fs::set_permissions(&leaf, fs::Permissions::from_mode(0o644)).unwrap(),
            "oversize" => fs::write(&leaf, vec![b' '; LIMIT as usize + 1]).unwrap(),
            _ => unreachable!(),
        }
        let before = fs::read(&leaf).unwrap();
        assert!(reader.read(OWNER).is_err(), "{kind}");
        assert_eq!(fs::read(&leaf).unwrap(), before);
    }
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("recording");
    fs::create_dir(&output).unwrap();
    write(&output, &record(OWNER, 2, true));
    let mut reader = Reader::pin(&output.canonicalize().unwrap()).unwrap();
    fs::rename(&output, root.path().join("retained")).unwrap();
    fs::create_dir(&output).unwrap();
    write(&output, &record(&uuid::Uuid::new_v4().to_string(), 4, true));
    assert_eq!(
        reader.read(OWNER).unwrap().microphone_history.last_sequence,
        2
    );
    assert!(Reader::pin(&output.canonicalize().unwrap())
        .unwrap()
        .read(OWNER)
        .is_err());
}

#[test]
#[ignore = "requires the isolated Swift producer fixture; make check-input-journal"]
fn swift_generated_input_journal_is_retained_history_not_live_telemetry() {
    let directory =
        std::env::var_os("MON_INPUT_JOURNAL_FIXTURE_DIR").expect("Swift fixture directory");
    let path = Path::new(&directory).canonicalize().unwrap();
    let mut reader = Reader::pin(&path).unwrap();
    let history = reader.read(OWNER).unwrap();
    assert!(history.sealed);
    for lane in [
        &history.microphone_history,
        history.system_history.as_ref().unwrap(),
    ] {
        assert_eq!(
            lane.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(lane.events[0].kind, "interrupted");
        assert_eq!(lane.events[1].kind, "recovered");
    }
    assert!(!path.join("input-status.json").exists());
    assert!(crate::input_status::StatusReader::pin(&path)
        .unwrap()
        .read(OWNER, Some("main"))
        .is_err());
}
