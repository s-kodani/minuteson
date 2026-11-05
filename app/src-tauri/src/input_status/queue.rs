//! Live recorder-owned, ordered delivery. Receiving is not rendering. Leases
//! fence abandoned WebViews; only the confirmed recipient can render/close.
use super::{history::Event, Input, Notice, Publication, Receipt, SAFE};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(crate) const LEASE_MS: u64 = 5000;
mod ledger;
pub(super) use ledger::archive::ArchiveQueue;
pub(crate) use ledger::archive::Batch as ArchivedBatch;
// Immutable transfer codec; archive publication and restored delivery binding
// are separate consumers and must not treat its producer owner as authority.
pub(super) use ledger::Ledger;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Delivery {
    sequence: u64,
    generation: u64,
    lease_age_ms: Option<u64>,
    rendered: bool,
    status: Input,
    receive_id: Option<String>,
}
impl Delivery {
    pub(super) fn account_persistence_time(&mut self, elapsed: Duration) {
        if let Some(age) = &mut self.lease_age_ms {
            *age = age.saturating_add(elapsed.as_millis().try_into().unwrap_or(u64::MAX));
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Loss {
    microphone: u64,
    system: u64,
}

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Receive,
    Render,
    Close,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Pending {
    event: Event,
    generation: u64,
    target: Option<String>,
    recipient: Option<String>,
    // A disk record is never a fresh lease. Restoration explicitly fences the
    // old recipient before this queue may be offered to another WebView.
    #[serde(skip)]
    claimed: Option<Instant>,
    rendered: bool,
    receive_id: Option<String>,
}
impl Pending {
    fn notice(&self, owner: &str, lane: &str, now: Instant) -> Option<Notice> {
        Some(Notice {
            id: format!(
                "{owner}:{lane}:{}:{}",
                self.event.status.interruptions, self.event.kind
            ),
            target: self.target.clone()?,
            kind: self.event.kind.clone(),
            interruptions: self.event.status.interruptions,
            input: lane.into(),
            recipient_id: self.recipient.clone(),
            delivery: Some(Delivery {
                sequence: self.event.sequence,
                generation: self.generation,
                lease_age_ms: self
                    .claimed
                    .map(|at| now.duration_since(at).as_millis() as u64),
                rendered: self.rendered,
                status: self.event.status.clone(),
                receive_id: self.receive_id.clone(),
            }),
        })
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Lane {
    seen: u64,
    lost: u64,
    pending: VecDeque<Pending>,
    closed: Option<Receipt>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Queue {
    lanes: [Lane; 2],
}

impl Queue {
    pub(super) fn observe(
        &mut self,
        p: &Publication,
        age: u64,
        target: Option<&str>,
        now: Instant,
    ) -> Vec<Notice> {
        self.observe_histories(
            &p.owner_id,
            [p.microphone_history.as_ref(), p.system_history.as_ref()],
            age <= super::FRESH_MS,
            target,
            now,
        )
    }
    fn observe_histories(
        &mut self,
        owner: &str,
        histories: [Option<&super::history::History>; 2],
        offer: bool,
        target: Option<&str>,
        now: Instant,
    ) -> Vec<Notice> {
        let mut notices = Vec::new();
        for (index, (name, history)) in [("microphone", histories[0]), ("system", histories[1])]
            .into_iter()
            .enumerate()
        {
            let Some(history) = history else { continue };
            let lane = &mut self.lanes[index];
            if history.dropped_through > lane.seen {
                lane.lost = lane.lost.max(history.dropped_through);
            }
            for event in history.events.iter().filter(|e| e.sequence > lane.seen) {
                lane.pending.push_back(Pending {
                    event: event.clone(),
                    generation: 1,
                    target: None,
                    recipient: None,
                    claimed: None,
                    rendered: false,
                    receive_id: None,
                });
            }
            lane.seen = history.last_sequence;
            while lane.pending.len() > 64 {
                let preserve_head = lane.pending.front().is_some_and(|p| p.claimed.is_some());
                let evicted = lane
                    .pending
                    .remove(if preserve_head { 1 } else { 0 })
                    .unwrap();
                if !evicted.rendered {
                    lane.lost = lane.lost.max(evicted.event.sequence);
                }
            }
            loop {
                let Some(head) = lane.pending.front_mut() else {
                    break;
                };
                if head
                    .claimed
                    .is_some_and(|at| now.duration_since(at) >= Duration::from_millis(LEASE_MS))
                {
                    if head.rendered {
                        let notice = head.notice(owner, name, now).unwrap();
                        lane.closed = Some(receipt(owner, &notice));
                        lane.pending.pop_front();
                        continue;
                    }
                    // Generation exhaustion fails closed. Never wrap to an old
                    // claim or use the recipient UUID alone as an ABA fence.
                    if head.generation == SAFE {
                        break;
                    }
                    head.generation += 1;
                    head.target = None;
                    head.recipient = None;
                    head.claimed = None;
                    head.receive_id = None;
                }
                if offer {
                    if head.recipient.is_none() {
                        if let Some(target) = target.filter(|t| matches!(*t, "main" | "recorder")) {
                            if head.target.as_deref().is_some_and(|prior| prior != target) {
                                if head.generation == SAFE {
                                    break;
                                }
                                head.generation += 1;
                            }
                            head.target = Some(target.into());
                        }
                    }
                    if let Some(notice) = head.notice(owner, name, now) {
                        notices.push(notice);
                    }
                }
                break;
            }
        }
        notices
    }

    pub(super) fn loss(&self) -> Loss {
        Loss {
            microphone: self.lanes[0].lost,
            system: self.lanes[1].lost,
        }
    }

    pub(super) fn act(
        &mut self,
        owner: &str,
        id: &str,
        recipient: &str,
        target: &str,
        generation: u64,
        action: Action,
        now: Instant,
        receive_id: Option<&str>,
    ) -> Result<Receipt, String> {
        if matches!(action, Action::Receive)
            && !receive_id.is_some_and(crate::input_levels::valid_owner)
        {
            return Err("通知受付の操作IDが不正です".into());
        }
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            let name = if index == 0 { "microphone" } else { "system" };
            if !matches!(action, Action::Receive) {
                if let Some(done) = lane.closed.as_ref().filter(|r| {
                    r.notice_id == id
                        && r.target == target
                        && r.recipient_id == recipient
                        && r.delivery
                            .as_ref()
                            .is_some_and(|d| d.generation == generation)
                }) {
                    return Ok(done.clone());
                }
            }
            let Some(head) = lane.pending.front_mut() else {
                continue;
            };
            let Some(notice) = head.notice(owner, name, now) else {
                continue;
            };
            if notice.id != id {
                continue;
            }
            if head.target.as_deref() != Some(target)
                || head.generation != generation
                || generation == 0
                || head
                    .claimed
                    .is_some_and(|at| now.duration_since(at) >= Duration::from_millis(LEASE_MS))
            {
                return Err("通知の宛先・受付世代または期限が変更されました".into());
            }
            match action {
                Action::Receive => {
                    let winner = head.recipient.get_or_insert_with(|| recipient.into());
                    if winner == recipient {
                        head.claimed = Some(now);
                        head.receive_id = receive_id.map(str::to_owned);
                    }
                }
                Action::Render | Action::Close => {
                    if head.recipient.as_deref() != Some(recipient) || head.claimed.is_none() {
                        return Err("この画面は通知の受付先ではありません".into());
                    }
                    if matches!(action, Action::Close) && !head.rendered {
                        return Err("通知の表示を確認できていません".into());
                    }
                    head.rendered = true;
                    head.claimed = Some(now);
                }
            }
            let result = receipt(owner, &head.notice(owner, name, now).unwrap());
            if matches!(action, Action::Close) {
                lane.closed = Some(result.clone());
                lane.pending.pop_front();
            }
            return Ok(result);
        }
        Err("現在の通知キューに該当する通知がありません".into())
    }
}

fn receipt(owner: &str, notice: &Notice) -> Receipt {
    Receipt {
        schema_version: 2,
        owner_id: owner.into(),
        notice_id: notice.id.clone(),
        target: notice.target.clone(),
        recipient_id: notice.recipient_id.clone().unwrap(),
        delivery: notice.delivery.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_status::{parse, StatusReader};
    use serde_json::json;
    const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
    const A: &str = "ed134978-12fc-4c5e-8e3f-372acbd44ad0";
    const B: &str = "539640c5-f04c-4d91-96f2-36e37008fe1d";
    const OP: &str = "da93a8dc-c860-478d-ae51-9b982f126a4f";
    const OP2: &str = "57e9ae3a-e22f-4c37-898b-2a6d40cc2658";
    pub(super) fn publication(sequence: u64) -> Publication {
        let status = |seq: u64, lane: &str| {
            let count = seq / 2 + seq % 2;
            json!({"state":if seq%2==0{"connected"}else{"disconnected"},"sourceEpoch":seq/2+1,"interruptions":count,"reason":if seq%2==0{None}else{Some(if lane=="microphone"{"device_disconnected"}else{"stream_stopped"})}})
        };
        let history = |lane: &str| json!({"lastSequence":sequence,"droppedThrough":sequence.saturating_sub(64),"events":(sequence.saturating_sub(64)+1..=sequence).map(|seq|json!({"sequence":seq,"kind":if seq%2==0{"recovered"}else{"interrupted"},"status":status(seq,lane)})).collect::<Vec<_>>()});
        parse(&serde_json::to_vec(&json!({"schemaVersion":3,"ownerId":OWNER,"revision":sequence+1,"microphone":status(sequence,"microphone"),"system":status(sequence,"system"),"microphoneHistory":history("microphone"),"systemHistory":history("system")})).unwrap(),OWNER).unwrap()
    }
    fn act(
        q: &mut Queue,
        n: &Notice,
        recipient: &str,
        action: Action,
        now: Instant,
    ) -> Result<Receipt, String> {
        q.act(
            OWNER,
            &n.id,
            recipient,
            &n.target,
            n.delivery.as_ref().unwrap().generation,
            action,
            now,
            Some(OP),
        )
    }
    #[test]
    fn fifo_retains_interruption_before_recovery_and_never_confuses_evidence_with_current_state() {
        let p = publication(4);
        let now = Instant::now();
        let mut q = Queue::default();
        assert!(q.observe(&p, 0, None, now).is_empty());
        let heads = q.observe(&p, 0, Some("main"), now);
        assert_eq!(heads.len(), 2);
        assert_eq!(p.microphone.state, "connected");
        assert_eq!(heads[0].kind, "interrupted");
        assert_eq!(
            heads[0].delivery.as_ref().unwrap().status.state,
            "disconnected"
        );
        let first = &heads[0];
        let r = act(&mut q, first, A, Action::Receive, now).unwrap();
        assert!(!r.delivery.unwrap().rendered);
        assert!(act(&mut q, first, A, Action::Close, now).is_err());
        let shown = act(&mut q, first, A, Action::Render, now).unwrap();
        assert!(shown.delivery.unwrap().rendered);
        let unchanged = q.observe(&p, 0, Some("recorder"), now);
        assert_eq!(unchanged[0].id, first.id);
        assert_eq!(unchanged[0].target, "main");
        let done = act(&mut q, first, A, Action::Close, now).unwrap();
        assert_eq!(act(&mut q, first, A, Action::Close, now).unwrap(), done);
        let next = q.observe(&p, 0, Some("recorder"), now);
        assert_eq!(next[0].kind, "recovered");
        assert_eq!(next[0].interruptions, 1);
        assert_eq!(next[1].id, heads[1].id);
        assert!(next[1].recipient_id.is_none());
    }
    #[test]
    fn receiving_is_not_rendering_and_abandoned_claim_changes_generation_before_handoff() {
        let p = publication(2);
        let now = Instant::now();
        let mut q = Queue::default();
        let head = q.observe(&p, 0, Some("main"), now)[0].clone();
        let first = act(&mut q, &head, A, Action::Receive, now).unwrap();
        let loser = act(
            &mut q,
            &head,
            B,
            Action::Receive,
            now + Duration::from_secs(4),
        )
        .unwrap();
        assert_eq!(loser.recipient_id, A);
        assert_eq!(q.lanes[0].pending.front().unwrap().claimed, Some(now));
        assert!(act(&mut q, &head, B, Action::Render, now).is_err());
        let later = now + Duration::from_millis(5000);
        let renewed = q.observe(&p, 0, Some("recorder"), later)[0].clone();
        assert_eq!(renewed.id, head.id);
        assert_eq!(renewed.target, "recorder");
        assert_eq!(renewed.delivery.as_ref().unwrap().generation, 2);
        assert!(renewed.recipient_id.is_none());
        assert!(act(&mut q, &head, A, Action::Render, later).is_err());
        let winner = act(&mut q, &renewed, B, Action::Receive, later).unwrap();
        assert_eq!(winner.recipient_id, B);
        assert!(!first.delivery.unwrap().rendered);
        assert!(act(&mut q, &head, A, Action::Close, later).is_err());
        assert!(
            act(&mut q, &renewed, B, Action::Render, later)
                .unwrap()
                .delivery
                .unwrap()
                .rendered
        );
    }
    #[test]
    fn exact_renewal_nonce_is_read_back_and_active_rendered_notice_waits_until_close_or_abandonment(
    ) {
        let p = publication(2);
        let now = Instant::now();
        let mut q = Queue::default();
        let head = q.observe(&p, 0, Some("main"), now)[0].clone();
        act(&mut q, &head, A, Action::Receive, now).unwrap();
        act(&mut q, &head, A, Action::Render, now).unwrap();
        q.act(
            OWNER,
            &head.id,
            A,
            "main",
            1,
            Action::Receive,
            now + Duration::from_secs(4),
            Some(OP2),
        )
        .unwrap();
        let read = q.observe(&p, 0, Some("main"), now + Duration::from_secs(8));
        assert_eq!(read[0].id, head.id);
        assert_eq!(
            read[0].delivery.as_ref().unwrap().receive_id.as_deref(),
            Some(OP2)
        );
        assert!(read[0].delivery.as_ref().unwrap().rendered);
        let next = q.observe(&p, 0, Some("recorder"), now + Duration::from_secs(10));
        assert_eq!(next[0].kind, "recovered");
        assert_eq!(next[0].delivery.as_ref().unwrap().generation, 1);
        assert!(
            q.act(
                OWNER,
                &head.id,
                A,
                "main",
                1,
                Action::Render,
                now + Duration::from_secs(10),
                None
            )
            .unwrap()
            .delivery
            .unwrap()
            .rendered
        );
    }
    #[test]
    fn bounded_queue_preserves_active_head_and_reports_missing_middle_and_source_prefix() {
        let now = Instant::now();
        let mut q = Queue::default();
        let head = q.observe(&publication(2), 0, Some("main"), now)[0].clone();
        act(&mut q, &head, A, Action::Receive, now).unwrap();
        act(&mut q, &head, A, Action::Render, now).unwrap();
        let read = q.observe(&publication(132), 0, Some("main"), now);
        assert_eq!(read[0].id, head.id);
        assert_eq!(q.lanes[0].pending.len(), 64);
        assert!(q.loss().microphone >= 69);
        act(&mut q, &head, A, Action::Close, now).unwrap();
        let next = q.observe(&publication(132), 0, Some("main"), now);
        assert_eq!(next[0].delivery.as_ref().unwrap().sequence, 70);
        let mut missed = Queue::default();
        missed.observe(&publication(200), 0, None, now);
        assert_eq!(missed.loss().microphone, 136);
        assert_eq!(missed.lanes[0].pending.len(), 64);
        assert!(missed
            .observe(&publication(200), 1001, Some("main"), now)
            .is_empty());
        assert_eq!(missed.loss().system, 136);
    }
    #[test]
    fn native_action_rechecks_owner_freshness_visibility_generation_and_current_source() {
        let temp = tempfile::tempdir().unwrap();
        let p = publication(2);
        let v = json!({"schemaVersion":p.schema_version,"ownerId":p.owner_id,"revision":p.revision,"microphone":p.microphone,"system":p.system,"microphoneHistory":p.microphone_history,"systemHistory":p.system_history});
        std::fs::write(
            temp.path().join("input-status.json"),
            serde_json::to_vec(&v).unwrap(),
        )
        .unwrap();
        let mut reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        let snap = reader.read(OWNER, Some("main")).unwrap();
        let id = &snap.notices[0].id;
        assert_eq!(snap.schema_version, 4);
        for (recipient, target, visible, generation, nonce) in [
            (A, "main", false, 1, Some(OP)),
            (A, "recorder", true, 1, Some(OP)),
            ("wrong", "main", true, 1, Some(OP)),
            (A, "main", true, 2, Some(OP)),
            (A, "main", true, 1, None),
        ] {
            assert!(reader
                .queued_action(
                    &snap,
                    id,
                    recipient,
                    target,
                    visible,
                    generation,
                    Action::Receive,
                    nonce
                )
                .is_err());
        }
        let first = reader
            .queued_action(&snap, id, A, "main", true, 1, Action::Receive, Some(OP))
            .unwrap();
        assert_eq!(first.schema_version, 2);
        let mut stale = snap.clone();
        stale.observed_at = Instant::now() - Duration::from_secs(2);
        assert!(reader
            .queued_action(&stale, id, A, "main", true, 1, Action::Render, None)
            .is_err());
        stale = snap.clone();
        stale.microphone.source_epoch += 1;
        assert!(reader
            .queued_action(&stale, id, A, "main", true, 1, Action::Render, None)
            .is_err());
        let reread = reader.read(OWNER, None).unwrap();
        assert_eq!(reread.notices[0].recipient_id.as_deref(), Some(A));
        assert!(!reread.notices[0].delivery.as_ref().unwrap().rendered);
        reader
            .queued_action(&reread, id, A, "main", true, 1, Action::Render, None)
            .unwrap();
        reader
            .queued_action(&reread, id, A, "main", true, 1, Action::Close, None)
            .unwrap();
        assert_eq!(
            reader.read(OWNER, Some("main")).unwrap().notices[0].kind,
            "recovered"
        );
    }
    #[test]
    fn unclaimed_destination_changes_fence_previous_offers_without_resetting_event_order() {
        let p = publication(2);
        let now = Instant::now();
        let mut q = Queue::default();
        let first = q.observe(&p, 0, Some("main"), now);
        let next = q.observe(&p, 0, Some("recorder"), now);
        assert_eq!(next[0].delivery.as_ref().unwrap().generation, 2);
        assert_eq!(next[0].delivery.as_ref().unwrap().sequence, 1);
        assert!(act(&mut q, &first[0], A, Action::Receive, now).is_err());
        act(&mut q, &next[0], B, Action::Receive, now).unwrap();
        let claimed = q.observe(&p, 0, Some("main"), now);
        assert_eq!(claimed[0].target, "recorder");
        assert_eq!(claimed[0].recipient_id.as_deref(), Some(B));
        let renewed = q.observe(&p, 0, Some("main"), now + Duration::from_secs(6));
        assert_eq!(renewed[0].delivery.as_ref().unwrap().generation, 3);
        assert_eq!(renewed[0].target, "main");
        let head = q.lanes[0].pending.front_mut().unwrap();
        head.generation = SAFE;
        assert!(q
            .observe(&p, 0, Some("recorder"), now + Duration::from_secs(6))
            .iter()
            .all(|n| n.input != "microphone"));
    }
    #[test]
    fn generation_exhaustion_never_wraps_or_delivers_to_a_new_receiver() {
        let p = publication(2);
        let now = Instant::now();
        let mut q = Queue::default();
        q.observe(&p, 0, Some("main"), now);
        let head = q.lanes[0].pending.front_mut().unwrap();
        head.generation = SAFE;
        head.recipient = Some(A.into());
        head.receive_id = Some(OP.into());
        head.claimed = Some(now);
        let later = now + Duration::from_secs(6);
        let read = q.observe(&p, 0, Some("recorder"), later);
        assert!(read.iter().all(|n| n.input != "microphone"));
        assert_eq!(q.lanes[0].pending.front().unwrap().generation, SAFE);
    }
}
