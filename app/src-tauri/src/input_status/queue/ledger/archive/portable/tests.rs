use super::*;
use crate::input_status::{queue::Action, StatusReader};
use serde_json::{json, Value};

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
const OTHER: &str = "539640c5-f04c-4d91-96f2-36e37008fe1d";
const RECIPIENT: &str = "ed134978-12fc-4c5e-8e3f-372acbd44ad0";
const NONCE: &str = "da93a8dc-c860-478d-ae51-9b982f126a4f";

fn publication(sequence: u64) -> crate::input_status::Publication {
    crate::input_status::queue::tests::publication(sequence)
}
fn retained(sequence: u64, revision: u64, sealed: bool) -> journal::Record {
    let mut report = publication(sequence);
    report.revision = revision;
    if sealed {
        report.microphone.state = "stopped".into();
        report.system.as_mut().unwrap().state = "stopped".into();
    }
    journal::Record {
        schema_version: 1,
        sealed,
        report,
    }
}
fn bytes(value: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn manifest(state: &str, committed: u64, error: bool) -> Vec<u8> {
    bytes(
        &json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"failed","recordingOwnerId":OWNER,
        "inputEventJournalPersistence":{"requestedRevision":201,"committedRevision":committed,"state":state,"closed":true},
        "inputEventJournalError":error.then_some("secret /Users/private/person/recording.json")}),
    )
}

#[test]
fn retained_diagnostic_projection_checks_independent_commit_without_offering_receipts_or_freshness()
{
    for state in ["confirmed", "pending", "failed", "invalid"] {
        for error in [false, true] {
            for source in [
                None,
                Some((true, 201)),
                Some((false, 201)),
                Some((true, 200)),
            ] {
                let journal =
                    source.map(|(sealed, revision)| bytes(&retained(66, revision, sealed)));
                let final_report =
                    manifest(state, if state == "confirmed" { 201 } else { 200 }, error);
                let snapshot =
                    Snapshot::from_evidence(OWNER, journal.as_deref(), None, Some(&final_report))
                        .unwrap()
                        .unwrap();
                let before = snapshot.encode().unwrap();
                let expected = match state {
                    "invalid" => Some("invalid"),
                    "failed" => Some("failed"),
                    "pending" => Some("unconfirmed"),
                    _ if error => Some("unconfirmed"),
                    _ => None,
                };
                let issue = snapshot.history_save_issue();
                if state == "confirmed" && !error && source != Some((true, 201)) {
                    assert!(issue.is_err(), "{state}/{error}/{source:?}");
                } else {
                    assert_eq!(issue.unwrap(), expected, "{state}/{error}/{source:?}");
                }
                let retained = snapshot.retained_history().unwrap();
                assert_eq!(retained.is_some(), source.is_some());
                if let Some(retained) = retained {
                    let dto = serde_json::to_value(retained).unwrap();
                    assert_eq!(dto["ownerId"], OWNER);
                    assert_eq!(dto["revision"], source.unwrap().1);
                    assert_eq!(dto["sealed"], source.unwrap().0);
                    assert!(dto.get("reportAgeMs").is_none());
                    assert!(dto.get("microphone").is_none());
                }
                let (batch, pending) = snapshot.retained_diagnostics(OTHER).unwrap();
                assert_eq!(pending, source.is_some());
                assert_eq!(batch.is_some(), source.is_some());
                if let Some(batch) = batch {
                    let dto = serde_json::to_value(batch).unwrap();
                    assert_eq!(dto["ownerId"], OTHER);
                    assert_eq!(dto["notices"], json!([]));
                    assert_eq!(dto["noticeLoss"], json!({"microphone":2,"system":2}));
                    assert!(dto.get("reportAgeMs").is_none());
                }
                assert_eq!(snapshot.encode().unwrap(), before);
                assert!(snapshot.retained_diagnostics("../foreign").is_err());
            }
        }
    }
    for sealed in [true, false] {
        let source = bytes(&retained(66, 201, sealed));
        let snapshot = Snapshot::from_evidence(OWNER, Some(&source), None, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshot.history_save_issue().unwrap(),
            (!sealed).then_some("unconfirmed")
        );
    }
}
fn value(snapshot: &Snapshot) -> Value {
    serde_json::from_slice(&snapshot.encode().unwrap()).unwrap()
}

#[test]
fn legacy_unknown_and_failed_recording_diagnostics_without_audio_are_not_fabricated_success() {
    assert!(Snapshot::from_evidence(OWNER, None, None, None)
        .unwrap()
        .is_none());
    assert!(Snapshot::from_evidence(OWNER, None, None, Some(br#"{"state":"saved"}"#)).is_err());
    for state in ["confirmed", "pending", "failed", "invalid"] {
        let raw = manifest(state, if state == "confirmed" { 201 } else { 200 }, true);
        let snapshot = Snapshot::from_evidence(OWNER, None, None, Some(&raw))
            .unwrap()
            .unwrap();
        let encoded = snapshot.encode().unwrap();
        let restored = Snapshot::decode(&encoded, OWNER).unwrap();
        assert_eq!(restored.encode().unwrap(), encoded);
        let portable = value(&restored);
        assert!(portable["sourceDelivery"].is_null());
        assert_eq!(portable["finalDiagnostic"]["ownerId"], OWNER);
        assert_eq!(portable["finalDiagnostic"]["persistence"]["state"], state);
        assert_eq!(portable["finalDiagnostic"]["unconfirmedAtStop"], true);
        assert!(portable["delivery"].is_null());
        assert!(portable["sourceJournal"].is_null());
        assert!(!String::from_utf8(encoded).unwrap().contains("/Users/"));
    }
}

#[test]
fn journal_only_preserves_loss_closed_status_and_original_producer_ownership() {
    let raw = bytes(&retained(66, 201, true));
    let snapshot = Snapshot::from_evidence(
        OWNER,
        Some(&raw),
        None,
        Some(&manifest("failed", 200, false)),
    )
    .unwrap()
    .unwrap();
    let portable = value(&snapshot);
    assert!(portable["sourceDelivery"].is_null());
    assert_eq!(
        portable["sourceJournal"],
        serde_json::from_slice::<Value>(&raw).unwrap()
    );
    assert_eq!(
        portable["delivery"]["retainedEvidence"],
        portable["sourceJournal"]
    );
    assert!(portable["delivery"]["liveEvidence"].is_null());
    assert_eq!(portable["delivery"]["queue"]["lanes"][0]["seen"], 66);
    assert_eq!(portable["delivery"]["queue"]["lanes"][0]["lost"], 2);
    assert_eq!(portable["delivery"]["queue"]["lanes"][1]["lost"], 2);
    assert_eq!(
        portable["delivery"]["queue"]["lanes"][0]["pending"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert!(portable.get("reportAgeMs").is_none());
    assert!(portable.get("microphone").is_none());
    assert!(Snapshot::decode(&snapshot.encode().unwrap(), OTHER).is_err());
}

#[cfg(unix)]
#[test]
fn live_writer_can_stay_open_while_codec_removes_claims_without_mutating_original_or_replaying_rendered_heads(
) {
    for rendered in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut p = publication(66);
        p.revision = 900; // Independently versioned live delivery publication.
        std::fs::write(temp.path().join("input-status.json"), bytes(&p)).unwrap();
        let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        let first = reader.read(OWNER, Some("main")).unwrap();
        let notice = &first.notices[0];
        for action in [Some(Action::Receive), rendered.then_some(Action::Render)]
            .into_iter()
            .flatten()
        {
            reader
                .queued_action(
                    &first,
                    &notice.id,
                    RECIPIENT,
                    "main",
                    true,
                    notice.delivery.as_ref().unwrap().generation,
                    action,
                    Some(NONCE),
                )
                .unwrap();
        }
        let source = bytes(&retained(66, 201, true));
        let ledger = std::fs::read(temp.path().join("input-notices.json")).unwrap();
        let snapshot = Snapshot::from_evidence(
            OWNER,
            Some(&source),
            Some(&ledger),
            Some(&manifest("failed", 200, true)),
        )
        .unwrap()
        .unwrap();
        let encoded = snapshot.encode().unwrap();
        let restored = Snapshot::decode(&encoded, OWNER).unwrap();
        assert_eq!(restored.encode().unwrap(), encoded);
        assert_eq!(
            std::fs::read(temp.path().join("input-notices.json")).unwrap(),
            ledger
        );
        // The original writer still owns its old source; the codec neither
        // opens another flock nor installs transferred bytes into that source.
        assert!(reader.read(OWNER, Some("main")).is_ok());
        let portable = value(&restored);
        assert_eq!(portable["sourceDelivery"]["kind"], "live");
        assert_eq!(
            portable["sourceDelivery"]["record"],
            serde_json::from_slice::<Value>(&ledger).unwrap()
        );
        assert_eq!(portable["sourceJournal"]["report"]["revision"], 201);
        assert_eq!(portable["delivery"]["liveEvidence"]["revision"], 900);
        for index in 0..2 {
            let lane = &portable["delivery"]["queue"]["lanes"][index];
            assert_eq!(lane["seen"], 66);
            assert_eq!(lane["lost"], 2);
            assert!(lane["closed"].is_null());
            for pending in lane["pending"].as_array().unwrap() {
                for key in ["target", "recipient", "receiveId"] {
                    assert!(pending[key].is_null());
                }
                assert_eq!(pending["rendered"], false);
            }
        }
        let head = &portable["delivery"]["queue"]["lanes"][0]["pending"][0];
        assert_eq!(head["event"]["sequence"], if rendered { 4 } else { 3 });
        if !rendered {
            assert!(
                head["generation"].as_u64().unwrap() > notice.delivery.as_ref().unwrap().generation
            );
        }
        // Exercise the sanitized queue itself. Even same source-owner IDs do
        // not preserve the old recipient/nonce/generation's render authority.
        let mut queue = restored.delivery.unwrap().queue;
        assert!(queue
            .act(
                OWNER,
                &notice.id,
                RECIPIENT,
                "main",
                notice.delivery.as_ref().unwrap().generation,
                Action::Render,
                Instant::now(),
                None
            )
            .is_err());
    }
}

#[cfg(unix)]
#[test]
fn archived_closed_receipts_are_removed_but_completed_range_and_independent_version_survive() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    let source = retained(4, 201, true);
    let mut queue = super::super::ArchiveQueue::open(&directory, OWNER, Some(source.clone()), None)
        .unwrap()
        .unwrap();
    let first = queue.poll(Some(source.clone()), None, true).unwrap();
    let head = &first.notices[0];
    for action in [Action::Receive, Action::Render, Action::Close] {
        queue
            .act(
                &head.id,
                RECIPIENT,
                head.delivery.as_ref().unwrap().generation,
                action,
                Some(NONCE),
                true,
            )
            .unwrap();
    }
    let ledger = std::fs::read(temp.path().join("input-notices.json")).unwrap();
    assert!(
        !serde_json::from_slice::<Value>(&ledger).unwrap()["queue"]["lanes"][0]["closed"].is_null()
    );
    let snapshot = Snapshot::from_evidence(OWNER, Some(&bytes(&source)), Some(&ledger), None)
        .unwrap()
        .unwrap();
    let portable = value(&snapshot);
    assert_eq!(portable["sourceDelivery"]["kind"], "archived");
    assert_eq!(
        portable["sourceDelivery"]["record"],
        serde_json::from_slice::<Value>(&ledger).unwrap()
    );
    assert_eq!(
        portable["delivery"]["revision"],
        serde_json::from_slice::<Value>(&ledger).unwrap()["revision"]
    );
    assert!(portable["delivery"]["queue"]["lanes"][0]["closed"].is_null());
    assert_eq!(
        portable["delivery"]["queue"]["lanes"][0]["pending"][0]["event"]["sequence"],
        2
    );
    assert_eq!(
        portable["delivery"]["queue"]["lanes"][1]["pending"][0]["event"]["sequence"],
        1
    );
    assert_eq!(
        std::fs::read(temp.path().join("input-notices.json")).unwrap(),
        ledger
    );
}

#[test]
fn immutable_codec_rejects_foreign_owners_duplicated_fields_receipt_injection_missing_nulls_and_oversized_bytes(
) {
    let raw = bytes(&retained(2, 201, true));
    let snapshot = Snapshot::from_evidence(OWNER, Some(&raw), None, None)
        .unwrap()
        .unwrap();
    let encoded = snapshot.encode().unwrap();
    let valid: Value = serde_json::from_slice(&encoded).unwrap();
    let mut cases = Vec::new();
    let mut changed = valid.clone();
    changed["sourceOwnerId"] = json!(OTHER);
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["sourceJournal"]["report"]["ownerId"] = json!(OTHER);
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["delivery"]["ownerId"] = json!(OTHER);
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["delivery"]["queue"]["lanes"][0]["pending"][0]["target"] = json!("main");
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed.as_object_mut().unwrap().remove("finalDiagnostic");
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["delivery"]["queue"]["lanes"][0]
        .as_object_mut()
        .unwrap()
        .remove("closed");
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["extra"] = json!(true);
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["delivery"]["revision"] = json!(0);
    cases.push(bytes(&changed));
    let mut changed = valid.clone();
    changed["sourceJournal"]["sealed"] = json!(false);
    cases.push(bytes(&changed));
    let text = String::from_utf8(encoded.clone()).unwrap();
    cases.push(
        text.replacen(
            "\"schemaVersion\":1",
            "\"schemaVersion\":1,\"schemaVersion\":1",
            1,
        )
        .into_bytes(),
    );
    cases.push(
        text.replacen("\"generation\":1", "\"generation\":1,\"generation\":1", 1)
            .into_bytes(),
    );
    cases.push(vec![b' '; LIMIT + 1]);
    for case in cases {
        assert!(
            Snapshot::decode(&case, OWNER).is_err(),
            "accepted corrupt payload"
        );
    }
    assert_eq!(snapshot.encode().unwrap(), encoded);
    assert!(Snapshot::from_evidence(OTHER, Some(&raw), None, None).is_err());
    let foreign = bytes(
        &json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"failed","recordingOwnerId":OTHER,"inputEventJournalError":"timeout"}),
    );
    assert!(Snapshot::from_evidence(OWNER, None, None, Some(&foreign)).is_err());
    let duplicate = raw.iter().copied().collect::<Vec<_>>();
    let duplicate = String::from_utf8(duplicate).unwrap().replacen(
        "\"sealed\":true",
        "\"sealed\":true,\"sealed\":true",
        1,
    );
    assert!(Snapshot::from_evidence(OWNER, Some(duplicate.as_bytes()), None, None).is_err());
    assert!(Snapshot::from_evidence(OWNER, None, Some(&vec![b' '; 128 * 1024 + 1]), None).is_err());
}

#[test]
fn incompatible_overlapping_history_is_not_silently_joined_by_unrelated_revision_numbers() {
    let live = publication(4);
    let mut queue = Queue::default();
    queue.observe(&live, 0, None, Instant::now());
    let ledger = bytes(&super::super::super::Record {
        schema_version: 1,
        owner_id: OWNER.into(),
        publication: live.clone(),
        queue,
    });
    let mut changed = retained(4, 201, true);
    changed.report.microphone_history.as_mut().unwrap().events[0]
        .status
        .reason = Some("identity_changed".into());
    // Still individually valid (nondecreasing epochs) but the overlap differs.
    let changed = bytes(&changed);
    assert!(journal::parse(&changed, OWNER).is_ok());
    assert!(Snapshot::from_evidence(OWNER, Some(&changed), Some(&ledger), None).is_err());
}

#[test]
fn exhausted_claim_generation_fails_closed_instead_of_wrapping_or_dropping_unrendered_event() {
    let live = publication(2);
    let mut queue = Queue::default();
    queue.observe(&live, 0, Some("main"), Instant::now());
    queue.lanes[0].pending.front_mut().unwrap().generation = crate::input_status::SAFE;
    let ledger = bytes(&super::super::super::Record {
        schema_version: 1,
        owner_id: OWNER.into(),
        publication: live,
        queue,
    });
    assert!(super::super::super::decode(&ledger, OWNER).is_ok());
    assert!(Snapshot::from_evidence(OWNER, None, Some(&ledger), None).is_err());
}

#[cfg(unix)]
#[test]
fn missing_current_journal_preserves_archived_invalid_issue_and_its_old_retained_proof() {
    let temp = tempfile::tempdir().unwrap();
    let directory =
        crate::input_levels::open_capture_directory(&temp.path().canonicalize().unwrap()).unwrap();
    let mut queue =
        super::super::ArchiveQueue::open(&directory, OWNER, Some(retained(4, 201, false)), None)
            .unwrap()
            .unwrap();
    queue.poll(None, Some("invalid"), true).unwrap();
    let original = std::fs::read(temp.path().join("input-notices.json")).unwrap();
    let snapshot = Snapshot::from_evidence(OWNER, None, Some(&original), None)
        .unwrap()
        .unwrap();
    let portable = value(&snapshot);
    assert!(portable["sourceJournal"].is_null());
    assert_eq!(portable["delivery"]["historyIssue"], "invalid");
    assert_eq!(
        portable["delivery"]["retainedEvidence"]["report"]["revision"],
        201
    );
    assert_eq!(
        portable["sourceDelivery"]["record"],
        serde_json::from_slice::<Value>(&original).unwrap()
    );
    assert_eq!(
        Snapshot::decode(&snapshot.encode().unwrap(), OWNER)
            .unwrap()
            .encode()
            .unwrap(),
        snapshot.encode().unwrap()
    );
    assert_eq!(
        std::fs::read(temp.path().join("input-notices.json")).unwrap(),
        original
    );
}

#[test]
fn final_diagnostic_original_bytes_reject_known_duplicates_invalid_states_open_results_and_owner_rebinding(
) {
    for raw in [
        br#"{"state":"failed","recordingOwnerId":"735027d4-a03e-43d3-830a-21e767dfaf88","inputEventJournalError":"timeout","inputEventJournalError":"timeout"}"#.to_vec(),
        manifest("unexpected", 200, false),
        bytes(&json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"failed","recordingOwnerId":OWNER,"inputEventJournalPersistence":{"requestedRevision":201,"committedRevision":200,"state":"confirmed","closed":true}})),
        bytes(&json!({"schemaVersion":4,"inputDiagnosticsState":"reported","state":"failed","recordingOwnerId":OWNER,"inputEventJournalPersistence":{"requestedRevision":201,"committedRevision":200,"state":"failed","closed":false}})),
        bytes(&json!({"state":"recording","recordingOwnerId":OWNER,"inputEventJournalError":"timeout"})),
    ] {
        assert!(Snapshot::from_evidence(OWNER, None, None, Some(&raw)).is_err());
    }
}
