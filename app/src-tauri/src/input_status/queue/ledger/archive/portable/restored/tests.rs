use super::*;
use crate::input_status::{journal, queue::ledger};
use serde_json::{json, Value};

const SOURCE: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
const DESTINATION: &str = "539640c5-f04c-4d91-96f2-36e37008fe1d";
const NEXT: &str = "57e9ae3a-e22f-4c37-898b-2a6d40cc2658";
const RECIPIENT: &str = "ed134978-12fc-4c5e-8e3f-372acbd44ad0";
const NONCE: &str = "da93a8dc-c860-478d-ae51-9b982f126a4f";

fn source(sequence: u64) -> Snapshot {
    let mut report = crate::input_status::queue::tests::publication(sequence);
    report.microphone.state = "stopped".into();
    report.system.as_mut().unwrap().state = "stopped".into();
    let journal = journal::Record {
        schema_version: 1,
        sealed: true,
        report,
    };
    Snapshot::from_evidence(
        SOURCE,
        Some(&serde_json::to_vec(&journal).unwrap()),
        None,
        None,
    )
    .unwrap()
    .unwrap()
}
fn offered(source: &Snapshot) -> (State, Vec<crate::input_status::Notice>) {
    let state = State::initial(source, DESTINATION).unwrap().unwrap();
    let (state, batch) = state.polled(source, true).unwrap();
    (state, batch.notices)
}
fn act(
    state: &State,
    source: &Snapshot,
    notice: &crate::input_status::Notice,
    action: Action,
) -> (State, Receipt) {
    state
        .acted(
            source,
            &notice.id,
            RECIPIENT,
            notice.delivery.as_ref().unwrap().generation,
            action,
            matches!(action, Action::Receive).then_some(NONCE),
            true,
        )
        .unwrap()
}

#[test]
fn restored_delivery_uses_logical_receipts_without_retagging_or_weakening_native_evidence() {
    let source = source(66);
    let unchanged = source.encode().unwrap();
    let (state, notices) = offered(&source);
    assert_eq!(notices.len(), 2);
    assert!(notices[0].id.starts_with(DESTINATION));
    assert_eq!(notices[0].delivery.as_ref().unwrap().sequence, 3);
    let (received, receipt) = act(&state, &source, &notices[0], Action::Receive);
    assert_eq!(receipt.owner_id, DESTINATION);
    assert!(!receipt.delivery.as_ref().unwrap().rendered);
    let (rendered, _) = act(&received, &source, &notices[0], Action::Render);
    let (closed, done) = act(&rendered, &source, &notices[0], Action::Close);
    let (replayed, reply) = closed
        .acted(
            &source,
            &done.notice_id,
            RECIPIENT,
            done.delivery.as_ref().unwrap().generation,
            Action::Close,
            None,
            true,
        )
        .unwrap();
    assert_eq!(done.notice_id, reply.notice_id);
    assert_eq!(done.recipient_id, reply.recipient_id);
    assert_eq!(
        closed.encode(&source).unwrap(),
        replayed.encode(&source).unwrap()
    );
    assert_eq!(source.encode().unwrap(), unchanged);
    assert_eq!(source.source_owner_id(), SOURCE);
    assert_eq!(
        serde_json::to_value(source.retained_history().unwrap().unwrap()).unwrap()["ownerId"],
        SOURCE
    );
    let producer = super::super::super::bounds(source.delivery.as_ref().unwrap()).unwrap();
    ledger::validate_queue(&producer, &closed.queue, DESTINATION).unwrap();
    assert!(ledger::validate_queue(&producer, &closed.queue, SOURCE).is_err());
    let native = ledger::Record {
        schema_version: 1,
        owner_id: DESTINATION.into(),
        publication: producer,
        queue: closed.queue.clone(),
    };
    assert!(ledger::decode(&serde_json::to_vec(&native).unwrap(), DESTINATION).is_err());
}

#[test]
fn restored_render_is_durable_across_cold_restart_and_two_flat_handoffs() {
    let source = source(6);
    let (state, notices) = offered(&source);
    let (received, _) = act(&state, &source, &notices[0], Action::Receive);
    let (rendered, proof) = act(&received, &source, &notices[0], Action::Render);
    let durable_proof = rendered.rendered[0].clone().unwrap();
    let disk = rendered.encode(&source).unwrap();
    let cold = State::decode(&disk, &source, DESTINATION).unwrap();
    assert!(cold.queue.lanes[0]
        .pending
        .front()
        .unwrap()
        .claimed
        .is_none());
    assert!(cold
        .acted(
            &source,
            &proof.notice_id,
            RECIPIENT,
            proof.delivery.as_ref().unwrap().generation,
            Action::Render,
            None,
            true
        )
        .is_err());
    let cold = cold.fenced(&source, DESTINATION).unwrap();
    assert_eq!(cold.removed[0].as_ref(), Some(&durable_proof));
    assert_eq!(
        cold.queue.lanes[0].pending.front().unwrap().event.sequence,
        2
    );
    assert!(cold.queue.lanes[0].closed.is_none());
    assert!(cold
        .acted(
            &source,
            &proof.notice_id,
            RECIPIENT,
            proof.delivery.as_ref().unwrap().generation,
            Action::Close,
            None,
            true
        )
        .is_err());
    let next = cold.fenced(&source, NEXT).unwrap();
    let bytes = next.encode(&source).unwrap();
    let next = State::decode(&bytes, &source, NEXT)
        .unwrap()
        .fenced(&source, SOURCE)
        .unwrap();
    assert_eq!(next.removed[0].as_ref(), Some(&durable_proof));
    let (_, batch) = next.polled(&source, true).unwrap();
    let mic = batch
        .notices
        .iter()
        .find(|n| n.input == "microphone")
        .unwrap();
    assert_eq!(mic.delivery.as_ref().unwrap().sequence, 2);
    let value: Value = serde_json::from_slice(&next.encode(&source).unwrap()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 7);
    assert!(value.get("sourceJournal").is_none());
    assert!(value.get("previousTransfer").is_none());
    assert!(bytes.len() < 16 * 1024);
}

#[test]
fn restored_unrendered_claim_is_fenced_and_old_generation_cannot_close_or_render() {
    let source = source(6);
    let (state, notices) = offered(&source);
    let (received, proof) = act(&state, &source, &notices[0], Action::Receive);
    let cold = State::decode(&received.encode(&source).unwrap(), &source, DESTINATION)
        .unwrap()
        .fenced(&source, DESTINATION)
        .unwrap();
    let (cold, next) = cold.polled(&source, true).unwrap();
    let mic = next
        .notices
        .iter()
        .find(|n| n.input == "microphone")
        .unwrap();
    assert_eq!(
        mic.delivery.as_ref().unwrap().sequence,
        proof.delivery.as_ref().unwrap().sequence
    );
    assert!(
        mic.delivery.as_ref().unwrap().generation > proof.delivery.as_ref().unwrap().generation
    );
    for action in [Action::Receive, Action::Render, Action::Close] {
        assert!(cold
            .acted(
                &source,
                &proof.notice_id,
                RECIPIENT,
                proof.delivery.as_ref().unwrap().generation,
                action,
                matches!(action, Action::Receive).then_some(NONCE),
                true
            )
            .is_err());
    }
    assert!(cold
        .acted(
            &source,
            &mic.id,
            RECIPIENT,
            mic.delivery.as_ref().unwrap().generation,
            Action::Render,
            None,
            true
        )
        .is_err());
    let (received, _) = act(&cold, &source, mic, Action::Receive);
    let (rendered, _) = act(&received, &source, mic, Action::Render);
    act(&rendered, &source, mic, Action::Close);
}

#[test]
fn restored_progress_rejects_missing_duplicate_foreign_and_fabricated_suffix_proofs() {
    let source = source(6);
    let (state, notices) = offered(&source);
    let bytes = state.encode(&source).unwrap();
    assert!(State::decode(&bytes, &source, NEXT).is_err());
    assert!(State::decode(&bytes, &self::source(8), DESTINATION).is_err());
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let mut changed = value.clone();
    changed["queue"]["lanes"][0]["pending"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert!(State::decode(&serde_json::to_vec(&changed).unwrap(), &source, DESTINATION).is_err());
    let mut changed = value.clone();
    changed["rendered"] = json!([null]);
    assert!(State::decode(&serde_json::to_vec(&changed).unwrap(), &source, DESTINATION).is_err());
    let mut changed = value.clone();
    changed.as_object_mut().unwrap().remove("rendered");
    assert!(State::decode(&serde_json::to_vec(&changed).unwrap(), &source, DESTINATION).is_err());
    let text = String::from_utf8(bytes).unwrap().replacen(
        "\"revision\":",
        "\"revision\":1,\"revision\":",
        1,
    );
    assert!(State::decode(text.as_bytes(), &source, DESTINATION).is_err());
    assert!(state
        .acted(
            &source,
            &notices[0].id,
            RECIPIENT,
            1,
            Action::Receive,
            Some(NONCE),
            false
        )
        .is_err());
    let mut overflow = state;
    overflow.revision = SAFE;
    let (received, _) = act(
        &State::decode(&serde_json::to_vec(&value).unwrap(), &source, DESTINATION).unwrap(),
        &source,
        &notices[0],
        Action::Receive,
    );
    assert!(overflow
        .acted(
            &source,
            &notices[0].id,
            RECIPIENT,
            1,
            Action::Receive,
            Some(NONCE),
            true
        )
        .is_err());
    let mut corrupt = received;
    corrupt.queue.lanes[0].pending.front_mut().unwrap().rendered = true;
    assert!(corrupt.encode(&source).is_err());
}

#[test]
fn restored_hidden_poll_and_repeated_heartbeat_do_not_create_durable_transitions_or_freshness() {
    let source = source(6);
    let state = State::initial(&source, DESTINATION).unwrap().unwrap();
    let original = state.encode(&source).unwrap();
    let (state, hidden) = state.polled(&source, false).unwrap();
    assert!(hidden.notices.is_empty());
    assert_eq!(state.encode(&source).unwrap(), original);
    let (state, notices) = state.polled(&source, true).unwrap();
    let (_, received) = act(&state, &source, &notices.notices[0], Action::Receive);
    let (state, _) = act(&state, &source, &notices.notices[0], Action::Receive);
    let (state, _) = state.polled(&source, true).unwrap();
    assert_eq!(state.revision(), 3);
    let dto = serde_json::to_value(received).unwrap();
    assert!(dto["delivery"]["leaseAgeMs"].as_u64().unwrap() < 5000);
    let (_, batch) = state.polled(&source, true).unwrap();
    let dto = serde_json::to_value(batch).unwrap();
    assert!(dto.get("reportAgeMs").is_none());
    assert!(dto.get("microphone").is_none());
}
