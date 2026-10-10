use super::*;
use serde_json::{json, Value};
use std::fs;

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";

fn input(epoch: u64, count: u64, connected: bool, lane: &str) -> Value {
    json!({"state":if connected {"connected"} else {"disconnected"},
        "sourceEpoch":epoch,"interruptions":count,
        "reason":if connected {Value::Null} else {json!(if lane == "microphone" {"device_disconnected"} else {"stream_stopped"})}})
}

fn history(sequence: u64, lane: &str) -> Value {
    let floor = sequence.saturating_sub(64);
    let events: Vec<_> = (floor + 1..=sequence)
        .map(|seq| {
            let recovered = seq % 2 == 0;
            let count = seq / 2 + seq % 2;
            json!({"sequence":seq,"kind":if recovered {"recovered"} else {"interrupted"},
                "status":input(if recovered {count+1} else {count},count,recovered,lane)})
        })
        .collect();
    json!({"lastSequence":sequence,"droppedThrough":floor,"events":events})
}

fn publication(sequence: u64) -> Value {
    let count = sequence / 2 + sequence % 2;
    let epoch = sequence / 2 + 1;
    json!({"schemaVersion":3,"ownerId":OWNER,"revision":sequence+1,
        "microphone":input(epoch,count,sequence%2 == 0,"microphone"),
        "system":input(epoch,count,sequence%2 == 0,"system"),
        "microphoneHistory":history(sequence,"microphone"),
        "systemHistory":history(sequence,"system")})
}

fn decode(v: &Value) -> Result<Publication, String> {
    parse(&serde_json::to_vec(v).unwrap(), OWNER)
}

fn write(path: &Path, value: &Value) {
    fs::write(
        path.join("input-status.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

#[test]
fn clock_change_keeps_both_inputs_disconnected_until_new_confirmed_epochs() {
    let mut paused = publication(1);
    for (lane, history) in [
        ("microphone", "microphoneHistory"),
        ("system", "systemHistory"),
    ] {
        paused[lane]["reason"] = json!("clock_changed");
        paused[history]["events"][0]["status"]["reason"] = json!("clock_changed");
    }
    let p = decode(&paused).unwrap();
    assert_eq!(p.microphone.state, "disconnected");
    assert_eq!(p.system.as_ref().unwrap().state, "disconnected");
    for lane in ["microphone", "system"] {
        let mut bad = paused.clone();
        bad[lane]["state"] = json!("connected");
        assert!(decode(&bad).is_err());
    }
    let temp = tempfile::tempdir().unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    write(temp.path(), &publication(0));
    reader.read(OWNER, None).unwrap();
    write(temp.path(), &paused);
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    assert!(!snapshot.connected());
    assert_eq!(snapshot.notices.len(), 2);
}

#[test]
fn clock_change_history_delivers_original_reason_even_when_resume_precedes_heartbeat() {
    let temp = tempfile::tempdir().unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    write(temp.path(), &publication(0));
    reader.read(OWNER, None).unwrap();
    let mut resumed = publication(2);
    for history in ["microphoneHistory", "systemHistory"] {
        resumed[history]["events"][0]["status"]["reason"] = json!("clock_changed");
    }
    write(temp.path(), &resumed);
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    assert!(snapshot.connected());
    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(json["notices"].as_array().unwrap().len(), 2);
    for notice in json["notices"].as_array().unwrap() {
        assert_eq!(notice["kind"], "interrupted");
        assert_eq!(notice["delivery"]["status"]["reason"], "clock_changed");
        assert_eq!(notice["delivery"]["status"]["sourceEpoch"], 1);
    }
}

#[test]
fn event_history_keeps_unobserved_interruption_and_recovery_with_no_legacy_format_changes() {
    let temp = tempfile::tempdir().unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    write(temp.path(), &publication(0));
    assert!(reader.read(OWNER, None).unwrap().notices.is_empty());
    write(temp.path(), &publication(4)); // two whole cycles between reads
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    assert!(snapshot.connected());
    let p = &reader.previous.as_ref().unwrap().0;
    let mic = serde_json::to_value(p.microphone_history.as_ref().unwrap()).unwrap();
    assert_eq!(mic["events"].as_array().unwrap().len(), 4);
    assert_eq!(mic["events"][0]["kind"], "interrupted");
    assert_eq!(mic["events"][1]["kind"], "recovered");
    // This stage stores producer proof, not a claim that the existing latest-
    // status notice API has rendered all four events.
    assert_eq!(serde_json::to_value(snapshot).unwrap()["schemaVersion"], 4);
    assert!(p
        .microphone_history
        .as_ref()
        .unwrap()
        .valid(&p.microphone, "microphone"));
}

#[test]
fn history_requires_closed_fields_explicit_null_and_contiguous_policy_transitions() {
    let base = publication(4);
    decode(&base).unwrap();
    let mut invalids = Vec::new();
    for field in ["microphoneHistory", "systemHistory"] {
        let mut v = base.clone();
        v.as_object_mut().unwrap().remove(field);
        invalids.push(v);
        let mut v = base.clone();
        v[field] = Value::Null;
        invalids.push(v);
        for (key, wrong) in [
            ("lastSequence", json!(SAFE + 1)),
            ("droppedThrough", json!(1)),
            ("extra", json!(true)),
        ] {
            let mut v = base.clone();
            v[field][key] = wrong;
            invalids.push(v);
        }
        for (key, wrong) in [
            ("sequence", json!(2)),
            ("kind", json!("recovered")),
            ("extra", json!(true)),
        ] {
            let mut v = base.clone();
            v[field]["events"][0][key] = wrong;
            invalids.push(v);
        }
        for (key, wrong) in [
            ("interruptions", json!(2)),
            ("sourceEpoch", json!(5)),
            ("reason", json!("unproven")),
            ("state", json!("stopped")),
            ("extra", json!(true)),
        ] {
            let mut v = base.clone();
            v[field]["events"][0]["status"][key] = wrong;
            invalids.push(v);
        }
        let mut v = base.clone();
        v[field]["events"][1]["status"]
            .as_object_mut()
            .unwrap()
            .remove("reason");
        invalids.push(v);
        let mut v = base.clone();
        v[field]["events"][1]["status"]["sourceEpoch"] = json!(1);
        invalids.push(v);
        let mut v = base.clone();
        v[field]["events"].as_array_mut().unwrap().remove(0);
        invalids.push(v);
    }
    for v in invalids {
        assert!(decode(&v).is_err(), "unexpectedly accepted {v}");
    }
    let mut unavailable = publication(2);
    unavailable["system"] = Value::Null;
    unavailable["systemHistory"] = Value::Null;
    decode(&unavailable).unwrap();
    unavailable["systemHistory"] = history(0, "system");
    assert!(decode(&unavailable).is_err());
}

#[test]
fn history_has_fixed_eviction_floor_bounds_and_never_synthesizes_an_omitted_event() {
    for sequence in [0, 1, 2, 63, 64, 65, 66, 200, 4098] {
        let v = publication(sequence);
        let p = decode(&v).unwrap();
        assert_eq!(
            serde_json::to_value(p.microphone_history.unwrap()).unwrap(),
            history(sequence, "microphone")
        );
        assert!(serde_json::to_vec(&v).unwrap().len() < LIMIT as usize);
    }
    let mut v = publication(66);
    v["microphoneHistory"]["droppedThrough"] = json!(0);
    assert!(decode(&v).is_err());
    let mut v = publication(66);
    v["microphoneHistory"]["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({}));
    assert!(decode(&v).is_err());
    let mut v = publication(2);
    v["microphoneHistory"] = history(0, "microphone");
    assert!(decode(&v).is_err());
    v = publication(0);
    v["microphoneHistory"] = history(2, "microphone");
    assert!(decode(&v).is_err());
    // A recovered suffix must end at the exact current binding, not an older
    // or invented connected epoch. Pending retries can legitimately have a
    // later epoch than the first interruption recorded for that episode.
    v = publication(2);
    v["microphone"]["sourceEpoch"] = json!(3);
    assert!(decode(&v).is_err());
    v = publication(1);
    v["microphone"]["sourceEpoch"] = json!(3);
    v["microphone"]["state"] = json!("reconnecting");
    decode(&v).unwrap();
}

#[test]
fn newer_heartbeat_cannot_rewrite_recorded_events_reset_history_or_change_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    write(temp.path(), &publication(2));
    reader.read(OWNER, None).unwrap();
    let mut changed = publication(2);
    changed["revision"] = json!(20);
    changed["microphoneHistory"]["events"][0]["status"]["reason"] = json!("engine_changed");
    assert!(decode(&changed).is_ok());
    write(temp.path(), &changed);
    assert!(reader.read(OWNER, None).is_err());
    assert_eq!(reader.previous.as_ref().unwrap().0.revision, 3);
    let mut legacy = publication(2);
    legacy["schemaVersion"] = json!(2);
    legacy["revision"] = json!(21);
    legacy.as_object_mut().unwrap().remove("microphoneHistory");
    legacy.as_object_mut().unwrap().remove("systemHistory");
    assert!(decode(&legacy).is_ok());
    write(temp.path(), &legacy);
    assert!(reader.read(OWNER, None).is_err());
    write(temp.path(), &publication(200)); // explicit prefix eviction accepted
    reader.read(OWNER, None).unwrap();
    changed = publication(198);
    changed["revision"] = json!(1000);
    write(temp.path(), &changed);
    assert!(reader.read(OWNER, None).is_err());
}

#[test]
fn overlapping_suffix_remains_immutable_and_stale_history_cannot_freshen_current_status() {
    let temp = tempfile::tempdir().unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    write(temp.path(), &publication(64));
    reader.read(OWNER, None).unwrap();
    let mut v = publication(66);
    v["microphoneHistory"]["events"][0]["status"]["reason"] = json!("engine_changed");
    assert!(decode(&v).is_ok());
    write(temp.path(), &v);
    assert!(reader.read(OWNER, None).is_err());
    write(temp.path(), &publication(66));
    reader.read(OWNER, None).unwrap();
    reader.previous.as_mut().unwrap().1 = 2000;
    write(temp.path(), &publication(66));
    let stale = reader.read(OWNER, Some("main")).unwrap();
    assert!(!stale.connected());
    assert!(stale.notices.is_empty());
    assert!(reader
        .previous
        .as_ref()
        .unwrap()
        .0
        .microphone_history
        .is_some());
}

#[test]
#[ignore = "Swift-owned history fixture; run make check-input-history without audio capture"]
fn swift_generated_input_history_retains_both_transitions_without_pcm_or_render_claims() {
    let path = std::path::PathBuf::from(std::env::var("MON_INPUT_HISTORY_FIXTURE_DIR").unwrap());
    // This dedicated Swift fixture predates delivery persistence. Prove these
    // companions are new test-owned outputs before removing them on completion;
    // retain the original producer JSON for the Makefile's existing cleanup.
    for name in ["input-notices.json", "input-notices.lock"] {
        assert_eq!(
            fs::symlink_metadata(path.join(name)).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
    let before = fs::read(path.join("input-status.json")).unwrap();
    let mut reader = StatusReader::pin(&path.canonicalize().unwrap()).unwrap();
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    assert_eq!(snapshot.revision, 7);
    let p = &reader.previous.as_ref().unwrap().0;
    assert_eq!(p.schema_version, 3);
    for (h, reason) in [
        (
            p.microphone_history.as_ref().unwrap(),
            "device_disconnected",
        ),
        (p.system_history.as_ref().unwrap(), "stream_stopped"),
    ] {
        let v = serde_json::to_value(h).unwrap();
        assert_eq!(v["lastSequence"], 2);
        assert_eq!(v["droppedThrough"], 0);
        assert_eq!(v["events"][0]["kind"], "interrupted");
        assert_eq!(v["events"][0]["status"]["reason"], reason);
        assert_eq!(v["events"][1]["kind"], "recovered");
        assert_eq!(v["events"][1]["status"]["sourceEpoch"], 2);
    }
    assert_eq!(before, fs::read(path.join("input-status.json")).unwrap());
    drop(reader);
    for name in ["input-notices.json", "input-notices.lock"] {
        fs::remove_file(path.join(name)).unwrap();
    }
}
