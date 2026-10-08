use super::*;
use crate::input_status::{queue::Action, StatusReader};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
const RECIPIENT: &str = "ed134978-12fc-4c5e-8e3f-372acbd44ad0";
const OP: &str = "da93a8dc-c860-478d-ae51-9b982f126a4f";
fn publication(sequence: u64) -> Publication {
    super::super::tests::publication(sequence)
}
fn fixture(sequence: u64) -> (tempfile::TempDir, StatusReader) {
    let temp = tempfile::tempdir().unwrap();
    let reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    fs::write(
        temp.path().join("input-status.json"),
        serde_json::to_vec(&publication(sequence)).unwrap(),
    )
    .unwrap();
    (temp, reader)
}
fn action(
    reader: &mut StatusReader,
    snapshot: &crate::input_status::Snapshot,
    index: usize,
    action: Action,
) -> Result<crate::input_status::Receipt, String> {
    let n = &snapshot.notices[index];
    reader.queued_action(
        snapshot,
        &n.id,
        RECIPIENT,
        &n.target,
        true,
        n.delivery.as_ref().unwrap().generation,
        action,
        Some(OP),
    )
}
fn disk(temp: &tempfile::TempDir) -> Value {
    serde_json::from_slice(&fs::read(temp.path().join("input-notices.json")).unwrap()).unwrap()
}

#[test]
fn pin_is_read_only_and_durable_receive_is_not_rendering_restart_fences_old_generation() {
    let (temp, mut reader) = fixture(2);
    let first = reader.read(OWNER, Some("main")).unwrap();
    action(&mut reader, &first, 0, Action::Receive).unwrap();
    let saved = disk(&temp);
    assert_eq!(
        saved["queue"]["lanes"][0]["pending"][0]["recipient"],
        RECIPIENT
    );
    assert_eq!(saved["queue"]["lanes"][0]["pending"][0]["rendered"], false);
    assert!(saved["queue"]["lanes"][0]["pending"][0]
        .get("claimed")
        .is_none());
    drop(reader);
    let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    let next = restarted.read(OWNER, Some("main")).unwrap();
    assert_eq!(next.notices[0].delivery.as_ref().unwrap().generation, 2);
    assert!(next.notices[0].recipient_id.is_none());
    assert!(action(&mut restarted, &first, 0, Action::Render).is_err());
    assert!(action(&mut restarted, &first, 0, Action::Receive).is_err());
    action(&mut restarted, &next, 0, Action::Receive).unwrap();
    action(&mut restarted, &next, 0, Action::Render).unwrap();
    assert_eq!(
        disk(&temp)["queue"]["lanes"][0]["pending"][0]["rendered"],
        true
    );
}

#[test]
fn rendered_and_closed_events_are_suppressed_after_restart_old_ack_is_not_reused() {
    for close in [false, true] {
        let (temp, mut reader) = fixture(4);
        let first = reader.read(OWNER, Some("main")).unwrap();
        action(&mut reader, &first, 0, Action::Receive).unwrap();
        action(&mut reader, &first, 0, Action::Render).unwrap();
        if close {
            action(&mut reader, &first, 0, Action::Close).unwrap();
        }
        drop(reader);
        let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        let next = restarted.read(OWNER, Some("main")).unwrap();
        assert_eq!(next.notices[0].delivery.as_ref().unwrap().sequence, 2);
        assert!(action(&mut restarted, &first, 0, Action::Close).is_err());
        assert!(disk(&temp)["queue"]["lanes"][0]["closed"].is_null());
    }
}

#[test]
fn offered_but_unreceived_generation_is_also_fenced_and_second_writer_is_rejected() {
    let (temp, mut reader) = fixture(2);
    let first = reader.read(OWNER, Some("main")).unwrap();
    let mut other = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    assert!(other.read(OWNER, Some("recorder")).is_err());
    drop(reader);
    let next = other.read(OWNER, Some("recorder")).unwrap();
    assert_eq!(next.notices[0].delivery.as_ref().unwrap().generation, 2);
    assert!(action(&mut other, &first, 0, Action::Receive).is_err());
}

#[test]
fn uncertain_post_rename_failure_never_returns_receipt_or_readback_from_poisoned_writer() {
    let (temp, mut reader) = fixture(2);
    let first = reader.read(OWNER, Some("main")).unwrap();
    action(&mut reader, &first, 0, Action::Receive).unwrap();
    reader.ledger.as_mut().unwrap().store.fail_after_rename = true;
    assert!(action(&mut reader, &first, 0, Action::Render).is_err());
    assert!(!reader.queue.lanes[0].pending.front().unwrap().rendered);
    assert!(reader.read(OWNER, Some("main")).is_err());
    assert!(action(&mut reader, &first, 0, Action::Close).is_err());
    // The potentially committed candidate is retained. A new writer syncs and
    // reconciles it, then removes the proven rendered event without issuing any
    // old-incarnation receipt or making historical status fresh.
    assert_eq!(
        disk(&temp)["queue"]["lanes"][0]["pending"][0]["rendered"],
        true
    );
    drop(reader);
    let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    assert_eq!(
        restarted.read(OWNER, Some("main")).unwrap().notices[0]
            .delivery
            .as_ref()
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn replaced_leaf_and_lock_fail_closed_without_overwriting_foreign_bytes() {
    for name in ["input-notices.json", "input-notices.lock"] {
        let (temp, mut reader) = fixture(2);
        let first = reader.read(OWNER, Some("main")).unwrap();
        let original = temp.path().join(name);
        fs::rename(&original, temp.path().join(format!("kept-{name}"))).unwrap();
        fs::write(&original, b"foreign").unwrap();
        fs::set_permissions(&original, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(action(&mut reader, &first, 0, Action::Receive).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"foreign");
        assert!(reader.queue.lanes[0]
            .pending
            .front()
            .unwrap()
            .recipient
            .is_none());
    }
}

#[test]
fn strict_ledger_rejects_duplicate_missing_unknown_owner_event_and_claim_fields() {
    let (temp, mut reader) = fixture(2);
    reader.read(OWNER, Some("main")).unwrap();
    let baseline = disk(&temp);
    let decode_value = |v: &Value| decode(&serde_json::to_vec(v).unwrap(), OWNER);
    assert!(decode_value(&baseline).is_ok());
    for (path, invalid) in [
        (vec!["schemaVersion"], json!(2)),
        (vec!["ownerId"], json!(RECIPIENT)),
        (vec!["queue", "lanes", "0", "seen"], json!(3)),
        (
            vec!["queue", "lanes", "0", "pending", "0", "generation"],
            json!(0),
        ),
        (
            vec!["queue", "lanes", "0", "pending", "0", "recipient"],
            json!(RECIPIENT),
        ),
        (
            vec![
                "queue", "lanes", "0", "pending", "0", "event", "status", "reason",
            ],
            json!("stream_stopped"),
        ),
        (
            vec!["queue", "lanes", "0", "pending", "1", "target"],
            json!("main"),
        ),
    ] {
        let mut v = baseline.clone();
        let mut at = &mut v;
        for key in path {
            at = if let Ok(index) = key.parse::<usize>() {
                &mut at[index]
            } else {
                &mut at[key]
            };
        }
        *at = invalid;
        assert!(decode_value(&v).is_err());
    }
    let mut v = baseline.clone();
    v["queue"]["lanes"][0]["pending"][0]
        .as_object_mut()
        .unwrap()
        .remove("recipient");
    assert!(decode_value(&v).is_err());
    let mut v = baseline.clone();
    v["extra"] = json!(true);
    assert!(decode_value(&v).is_err());
    let text = serde_json::to_string(&baseline).unwrap().replacen(
        "\"schemaVersion\":1",
        "\"schemaVersion\":1,\"schemaVersion\":1",
        1,
    );
    assert!(decode(text.as_bytes(), OWNER).is_err());
}

#[test]
fn private_regular_bounded_ledger_and_lock_are_required_symlinks_and_hardlinks_are_retained() {
    for kind in ["symlink", "hardlink", "public", "oversize", "malformed"] {
        let (temp, mut reader) = fixture(2);
        reader.read(OWNER, Some("main")).unwrap();
        drop(reader);
        let leaf = temp.path().join("input-notices.json");
        let kept = temp.path().join("kept");
        fs::rename(&leaf, &kept).unwrap();
        match kind {
            "symlink" => symlink(&kept, &leaf).unwrap(),
            "hardlink" => fs::hard_link(&kept, &leaf).unwrap(),
            "public" => {
                fs::copy(&kept, &leaf).unwrap();
                fs::set_permissions(&leaf, fs::Permissions::from_mode(0o644)).unwrap();
            }
            "oversize" => {
                fs::write(&leaf, vec![b' '; LIMIT as usize + 1]).unwrap();
                fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
            }
            _ => {
                fs::write(&leaf, b"{}").unwrap();
                fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let before = fs::read(&leaf).unwrap();
        let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        assert!(restarted.read(OWNER, Some("main")).is_err());
        assert_eq!(fs::read(&leaf).unwrap(), before);
        assert!(kept.is_file());
    }
}

#[test]
fn heartbeat_does_not_rewrite_delivery_and_loss_floor_survives_restart() {
    use std::os::unix::fs::MetadataExt;
    let (temp, mut reader) = fixture(100);
    let first = reader.read(OWNER, Some("main")).unwrap();
    let baseline = fs::metadata(temp.path().join("input-notices.json")).unwrap();
    let mut p = publication(100);
    p.revision += 1;
    fs::write(
        temp.path().join("input-status.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    reader.read(OWNER, Some("main")).unwrap();
    assert_eq!(
        fs::metadata(temp.path().join("input-notices.json"))
            .unwrap()
            .ino(),
        baseline.ino()
    );
    assert_eq!(disk(&temp)["queue"]["lanes"][0]["lost"], 36);
    drop(reader);
    let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    let next = restarted.read(OWNER, Some("main")).unwrap();
    assert_eq!(
        next.notices[0].delivery.as_ref().unwrap().sequence,
        first.notices[0].delivery.as_ref().unwrap().sequence
    );
    assert_eq!(
        serde_json::to_value(next.notice_loss).unwrap()["microphone"],
        36
    );
}

#[test]
fn producer_rollback_is_rejected_against_disk_and_history_alone_is_not_live_telemetry() {
    let (temp, mut reader) = fixture(4);
    reader.read(OWNER, Some("main")).unwrap();
    drop(reader);
    let before = fs::read(temp.path().join("input-notices.json")).unwrap();
    fs::write(
        temp.path().join("input-status.json"),
        serde_json::to_vec(&publication(2)).unwrap(),
    )
    .unwrap();
    let mut restarted = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    assert!(restarted.read(OWNER, Some("main")).is_err());
    fs::remove_file(temp.path().join("input-status.json")).unwrap();
    assert!(restarted.read(OWNER, Some("main")).is_err());
    assert_eq!(
        fs::read(temp.path().join("input-notices.json")).unwrap(),
        before
    );
}

#[test]
fn repeated_restarts_strictly_advance_offered_generation_and_exhaustion_never_wraps() {
    let (temp, mut reader) = fixture(2);
    reader.read(OWNER, Some("main")).unwrap();
    drop(reader);
    for generation in 2..=4 {
        let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        let snapshot = reader.read(OWNER, Some("main")).unwrap();
        assert_eq!(
            snapshot.notices[0].delivery.as_ref().unwrap().generation,
            generation
        );
    }
    let mut stored = disk(&temp);
    stored["queue"]["lanes"][0]["pending"][0]["generation"] = json!(SAFE);
    fs::write(
        temp.path().join("input-notices.json"),
        serde_json::to_vec(&stored).unwrap(),
    )
    .unwrap();
    let before = fs::read(temp.path().join("input-notices.json")).unwrap();
    let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    assert!(reader.read(OWNER, Some("main")).is_err());
    assert_eq!(
        fs::read(temp.path().join("input-notices.json")).unwrap(),
        before
    );
}

#[test]
fn pinned_directory_survives_path_rename_without_writing_replacement_directory() {
    let (temp, mut reader) = fixture(2);
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    let renamed = temp.path().with_extension("retained-notice-ledger");
    fs::rename(temp.path(), &renamed).unwrap();
    fs::create_dir(temp.path()).unwrap();
    fs::write(temp.path().join("input-notices.json"), b"FOREIGN").unwrap();
    action(&mut reader, &snapshot, 0, Action::Receive).unwrap();
    assert_eq!(
        fs::read(temp.path().join("input-notices.json")).unwrap(),
        b"FOREIGN"
    );
    let kept: Value =
        serde_json::from_slice(&fs::read(renamed.join("input-notices.json")).unwrap()).unwrap();
    assert_eq!(
        kept["queue"]["lanes"][0]["pending"][0]["recipient"],
        RECIPIENT
    );
    drop(reader);
    // The fixture owns the renamed directory too; no product data is involved.
    fs::remove_file(temp.path().join("input-notices.json")).unwrap();
    fs::remove_dir(temp.path()).unwrap();
    fs::rename(&renamed, temp.path()).unwrap();
}

#[test]
fn lock_links_and_nonempty_lock_are_rejected_without_adopting_a_writer() {
    for kind in ["symlink", "hardlink", "nonempty"] {
        let (temp, mut reader) = fixture(2);
        reader.read(OWNER, Some("main")).unwrap();
        drop(reader);
        let lock = temp.path().join("input-notices.lock");
        let kept = temp.path().join("kept-lock");
        fs::rename(&lock, &kept).unwrap();
        match kind {
            "symlink" => symlink(&kept, &lock).unwrap(),
            "hardlink" => fs::hard_link(&kept, &lock).unwrap(),
            _ => {
                fs::write(&lock, b"FOREIGN").unwrap();
                fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let before = fs::read(&lock).unwrap();
        let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        assert!(reader.read(OWNER, Some("main")).is_err());
        assert_eq!(fs::read(&lock).unwrap(), before);
        assert_eq!(fs::read(&kept).unwrap(), b"");
    }
}

#[test]
fn persistence_latency_cannot_freshen_connection_or_delivery_offer() {
    use std::time::{Duration, SystemTime};
    let (temp, mut reader) = fixture(2);
    reader.read(OWNER, Some("main")).unwrap();
    reader.ledger.as_mut().unwrap().store.before_commit_delay = Duration::from_millis(300);
    let path = temp.path().join("input-status.json");
    fs::write(&path, serde_json::to_vec(&publication(4)).unwrap()).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_millis(800)),
        )
        .unwrap();
    let snapshot = reader.read(OWNER, Some("main")).unwrap();
    assert!(snapshot.report_age_ms >= 1100);
    assert!(!snapshot.connected());
    assert!(!snapshot.system_connected());
    assert!(snapshot.notices.is_empty());
    // The producer event is retained, not fabricated as a fresh publication.
    assert_eq!(disk(&temp)["queue"]["lanes"][0]["seen"], 4);
}
