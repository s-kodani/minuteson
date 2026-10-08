use super::*;
use crate::input_status::StatusReader;
use std::fs;
use std::os::unix::fs::PermissionsExt;
const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
const RECIPIENT: &str = "ed134978-12fc-4c5e-8e3f-372acbd44ad0";
const OP: &str = "da93a8dc-c860-478d-ae51-9b982f126a4f";

#[test]
fn archived_poll_accounts_commit_latency_without_exposing_a_live_report_age() {
    let (_temp, directory) = directory();
    let mut q = ArchiveQueue::open(&directory, OWNER, Some(retained(2, true)), None)
        .unwrap()
        .unwrap();
    let first = q.poll(Some(retained(2, true)), None, true).unwrap();
    act(&mut q, &first, Action::Receive).unwrap();
    q.store.before_commit_delay = std::time::Duration::from_millis(35);
    let waited = q.poll(None, Some("missing"), true).unwrap();
    assert!(
        waited.notices[0]
            .delivery
            .as_ref()
            .unwrap()
            .lease_age_ms
            .unwrap()
            >= 35
    );
    assert!(serde_json::to_value(&waited)
        .unwrap()
        .get("reportAgeMs")
        .is_none());
}

#[test]
fn retained_channel_cannot_change_sealing_at_the_same_revision_after_reader_recreation() {
    let (_temp, directory) = directory();
    let mut unsealed = retained(2, true);
    unsealed.sealed = false;
    let mut q = ArchiveQueue::open(&directory, OWNER, Some(unsealed.clone()), None)
        .unwrap()
        .unwrap();
    let mut forged = unsealed.clone();
    forged.sealed = true;
    let rejected = q.poll(Some(forged.clone()), None, true).unwrap();
    assert_eq!(rejected.history_issue.as_deref(), Some("invalid"));
    assert!(!q.record.retained_evidence.as_ref().unwrap().sealed);
    forged.report.revision += 1;
    let accepted = q.poll(Some(forged), None, true).unwrap();
    assert!(accepted.history_issue.is_none());
    assert!(q.record.retained_evidence.as_ref().unwrap().sealed);
}
fn publication(seq: u64) -> Publication {
    crate::input_status::queue::tests::publication(seq)
}
fn retained(seq: u64, sealed: bool) -> journal::Record {
    let mut p = publication(seq);
    if sealed {
        p.microphone.state = "stopped".into();
        if let Some(s) = &mut p.system {
            s.state = "stopped".into();
        }
    }
    journal::Record {
        schema_version: 1,
        sealed,
        report: p,
    }
}
fn directory() -> (tempfile::TempDir, File) {
    let temp = tempfile::tempdir().unwrap();
    let file =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    (temp, file)
}
fn act(q: &mut ArchiveQueue, b: &Batch, action: Action) -> Result<Receipt, String> {
    let n = &b.notices[0];
    q.act(
        &n.id,
        RECIPIENT,
        n.delivery.as_ref().unwrap().generation,
        action,
        Some(OP),
        true,
    )
}
fn live(temp: &tempfile::TempDir, sequence: u64, render: bool) {
    let mut p = publication(sequence);
    p.revision = 900;
    fs::write(
        temp.path().join("input-status.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut r = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    let b = r.read(OWNER, Some("main")).unwrap();
    let n = &b.notices[0];
    r.queued_action(
        &b,
        &n.id,
        RECIPIENT,
        "main",
        true,
        1,
        Action::Receive,
        Some(OP),
    )
    .unwrap();
    if render {
        r.queued_action(&b, &n.id, RECIPIENT, "main", true, 1, Action::Render, None)
            .unwrap();
    }
}
#[test]
fn migration_compares_history_not_unrelated_revision_and_suppresses_confirmed_rendering() {
    let (temp, dir) = directory();
    live(&temp, 4, true);
    let source = retained(6, true); // Producer journal revision 7, live revision 900.
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    let b = q.poll(Some(source.clone()), None, true).unwrap();
    assert_eq!(b.notices[0].delivery.as_ref().unwrap().sequence, 2);
    let json = serde_json::to_value(&b).unwrap();
    for key in ["microphone", "system", "reportAgeMs", "sealed"] {
        assert!(json.get(key).is_none());
    }
    let file: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("input-notices.json")).unwrap()).unwrap();
    assert_eq!(file["schemaVersion"], 2);
    assert_eq!(file["liveEvidence"]["revision"], 900);
    assert_eq!(file["retainedEvidence"]["report"]["revision"], 7);
    assert_eq!(file["queue"]["lanes"][0]["seen"], 6);
    drop(q);
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    let next = q.poll(Some(source), None, true).unwrap();
    assert!(q
        .act(
            &b.notices[0].id,
            RECIPIENT,
            b.notices[0].delivery.as_ref().unwrap().generation,
            Action::Receive,
            Some(OP),
            true
        )
        .is_err());
    assert!(
        next.notices[0].delivery.as_ref().unwrap().generation
            > b.notices[0].delivery.as_ref().unwrap().generation
    );
}
#[test]
fn crash_with_behind_missing_or_invalid_producer_preserves_durable_live_events() {
    for (source, issue) in [
        (Some(retained(2, false)), None),
        (None, Some("missing")),
        (None, Some("invalid")),
    ] {
        let (temp, dir) = directory();
        live(&temp, 4, false);
        let mut q = ArchiveQueue::open(&dir, OWNER, source.clone(), issue)
            .unwrap()
            .unwrap();
        let b = q.poll(source, issue, true).unwrap();
        assert_eq!(b.notices[0].delivery.as_ref().unwrap().sequence, 1);
        assert_eq!(b.notices[0].delivery.as_ref().unwrap().generation, 2);
        assert_eq!(q.record.queue.lanes[0].seen, 4);
        assert_eq!(b.history_issue.as_deref(), issue);
        act(&mut q, &b, Action::Receive).unwrap();
        act(&mut q, &b, Action::Render).unwrap();
        act(&mut q, &b, Action::Close).unwrap();
        assert_eq!(
            q.poll(None, Some("missing"), true).unwrap().notices[0]
                .delivery
                .as_ref()
                .unwrap()
                .sequence,
            2
        );
    }
}
#[test]
fn independent_input_sources_merge_actual_events_without_losing_newer_lane() {
    let (temp, dir) = directory();
    let mut a = publication(2);
    let four = publication(4);
    a.microphone = four.microphone.clone();
    a.microphone_history = four.microphone_history.clone();
    a.revision = 900;
    fs::write(
        temp.path().join("input-status.json"),
        serde_json::to_vec(&a).unwrap(),
    )
    .unwrap();
    let mut r = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
    r.read(OWNER, None).unwrap();
    drop(r);
    let mut b = retained(4, false);
    let two = publication(2);
    b.report.microphone = two.microphone;
    b.report.microphone_history = two.microphone_history;
    let q = ArchiveQueue::open(&dir, OWNER, Some(b), None)
        .unwrap()
        .unwrap();
    assert_eq!(q.record.queue.lanes[0].seen, 4);
    assert_eq!(q.record.queue.lanes[1].seen, 4);
    assert_eq!(q.record.live_evidence.unwrap(), a);
}
#[test]
fn sealed_rollback_and_conflicting_overlap_keep_previous_proof_and_publish_issue_not_false_events()
{
    let (_temp, dir) = directory();
    let source = retained(4, true);
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    let b = q.poll(Some(retained(2, false)), None, true).unwrap();
    assert_eq!(b.history_issue.as_deref(), Some("invalid"));
    assert_eq!(q.record.retained_evidence, Some(source));
    let (temp, dir) = directory();
    live(&temp, 4, false);
    let mut changed = retained(4, false);
    changed.report.microphone_history.as_mut().unwrap().events[0]
        .status
        .reason = Some("restart_failed".into());
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(changed), None)
        .unwrap()
        .unwrap();
    let b = q.poll(None, Some("invalid"), true).unwrap();
    assert_eq!(b.history_issue.as_deref(), Some("invalid"));
    assert_eq!(
        b.notices[0]
            .delivery
            .as_ref()
            .unwrap()
            .status
            .reason
            .as_deref(),
        Some("device_disconnected")
    );
}
#[test]
fn absent_legacy_history_does_not_create_files_and_invalid_only_history_is_not_empty_success() {
    let (temp, dir) = directory();
    assert!(ArchiveQueue::open(&dir, OWNER, None, Some("missing"))
        .unwrap()
        .is_none());
    assert!(ArchiveQueue::open(&dir, OWNER, None, Some("invalid")).is_err());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[test]
fn archived_action_requires_visible_owner_generation_receive_nonce_and_actual_render_confirmation()
{
    let (_temp, dir) = directory();
    let source = retained(2, true);
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    assert!(q
        .poll(Some(source.clone()), None, false)
        .unwrap()
        .notices
        .is_empty());
    let b = q.poll(Some(source), None, true).unwrap();
    let n = &b.notices[0];
    assert!(q
        .act(&n.id, RECIPIENT, 1, Action::Receive, Some(OP), false)
        .is_err());
    assert!(q
        .act(&n.id, RECIPIENT, 1, Action::Receive, None, true)
        .is_err());
    assert!(act(&mut q, &b, Action::Render).is_err());
    act(&mut q, &b, Action::Receive).unwrap();
    assert!(act(&mut q, &b, Action::Close).is_err());
    act(&mut q, &b, Action::Render).unwrap();
    act(&mut q, &b, Action::Close).unwrap();
    assert!(act(&mut q, &b, Action::Close).is_ok());
}
#[test]
fn reopen_and_uncertain_sync_never_replay_rendered_event_or_return_optimistic_receipt() {
    let (temp, dir) = directory();
    let source = retained(4, true);
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    let b = q.poll(Some(source.clone()), None, true).unwrap();
    act(&mut q, &b, Action::Receive).unwrap();
    q.store.fail_after_rename = true;
    assert!(act(&mut q, &b, Action::Render).is_err());
    assert!(q.poll(Some(source.clone()), None, true).is_err());
    drop(q);
    let mut q = ArchiveQueue::open(&dir, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        q.poll(Some(source), None, true).unwrap().notices[0]
            .delivery
            .as_ref()
            .unwrap()
            .sequence,
        2
    );
    let original = temp.path().join("input-notices.json");
    fs::rename(&original, temp.path().join("kept")).unwrap();
    fs::write(&original, b"foreign").unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(q.poll(None, Some("missing"), true).is_err());
    assert_eq!(fs::read(original).unwrap(), b"foreign");
}
#[test]
fn archived_record_is_closed_owner_bound_and_cannot_be_reopened_as_live_delivery() {
    let (temp, dir) = directory();
    ArchiveQueue::open(&dir, OWNER, Some(retained(2, true)), None).unwrap();
    let before = fs::read(temp.path().join("input-notices.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert!(decode(&before, RECIPIENT).is_err());
    for field in ["liveEvidence", "retainedEvidence", "historyIssue"] {
        let mut v = value.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(decode(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
    }
    let dup = String::from_utf8(before.clone()).unwrap().replacen(
        "\"schemaVersion\":2",
        "\"schemaVersion\":2,\"schemaVersion\":2",
        1,
    );
    assert!(decode(dup.as_bytes(), OWNER).is_err());
    assert!(super::super::Ledger::open(&dir, OWNER).is_err());
    assert_eq!(
        fs::read(temp.path().join("input-notices.json")).unwrap(),
        before
    );
}
