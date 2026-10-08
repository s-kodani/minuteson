//! Durable delivery evidence. It does not freshen capture status or persist an
//! Instant. The live controller must commit a candidate before publishing it.
use super::{Pending, Queue};
use crate::input_status::{history::Event, Publication, SAFE};
use serde::{Deserialize, Serialize};
use std::fs::File;

pub(super) mod archive;
mod file;
#[cfg(all(test, unix))]
mod tests;

const LIMIT: u64 = 128 * 1024;
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    schema_version: u8,
    owner_id: String,
    publication: Publication,
    queue: Queue,
}

pub(in crate::input_status) struct Ledger {
    store: file::Store,
    owner: String,
    previous: Option<Record>,
}

fn event_valid(e: &Event, name: &str) -> bool {
    e.sequence > 0
        && e.sequence <= SAFE
        && e.status.interruptions == e.sequence / 2 + e.sequence % 2
        && crate::input_status::valid_input(&e.status, name)
        && if e.sequence % 2 == 1 {
            e.kind == "interrupted"
                && matches!(e.status.state.as_str(), "disconnected" | "unavailable")
        } else {
            e.kind == "recovered" && e.status.state == "connected"
        }
}
fn validate(record: &Record) -> Result<(), String> {
    let p = &record.publication;
    if record.schema_version != 1
        || record.owner_id != p.owner_id
        || p.schema_version != 3
        || crate::input_status::parse(
            &serde_json::to_vec(p).map_err(|e| e.to_string())?,
            &record.owner_id,
        )? != *p
    {
        return Err("通知台帳の所有または取得履歴が不正です".into());
    }
    for (index, lane) in record.queue.lanes.iter().enumerate() {
        let name = if index == 0 { "microphone" } else { "system" };
        let history = if index == 0 {
            p.microphone_history.as_ref()
        } else {
            p.system_history.as_ref()
        };
        let epoch = if index == 0 {
            p.microphone.source_epoch
        } else {
            p.system.as_ref().map_or(0, |s| s.source_epoch)
        };
        if lane.seen != history.map_or(0, |h| h.last_sequence)
            || lane.lost > lane.seen
            || lane.pending.len() > 64
        {
            return Err("通知台帳の保持範囲が不正です".into());
        }
        let mut prior: Option<&Pending> = None;
        for (at, pending) in lane.pending.iter().enumerate() {
            let e = &pending.event;
            if !event_valid(e, name)
                || e.sequence > lane.seen
                || e.status.source_epoch > epoch
                || !(1..=SAFE).contains(&pending.generation)
                || pending
                    .target
                    .as_deref()
                    .is_some_and(|t| !matches!(t, "main" | "recorder"))
                || pending
                    .recipient
                    .as_deref()
                    .is_some_and(|id| !crate::input_levels::valid_owner(id))
                || pending
                    .receive_id
                    .as_deref()
                    .is_some_and(|id| !crate::input_levels::valid_owner(id))
                || pending.recipient.is_some() != pending.receive_id.is_some()
                || (pending.recipient.is_some() && pending.target.is_none())
                || (pending.rendered && pending.recipient.is_none())
                || (at > 0
                    && (pending.target.is_some()
                        || pending.recipient.is_some()
                        || pending.rendered
                        || pending.generation != 1))
                || prior.is_some_and(|old| {
                    e.sequence <= old.event.sequence
                        || e.status.source_epoch < old.event.status.source_epoch
                })
                || history.is_some_and(|h| e.sequence > h.dropped_through && !h.events.contains(e))
            {
                return Err("通知台帳の遷移または受付が不正です".into());
            }
            prior = Some(pending);
        }
        if let Some(done) = &lane.closed {
            let delivery = done
                .delivery
                .as_ref()
                .ok_or("通知終了記録に表示証拠がありません")?;
            let kind = if delivery.sequence % 2 == 1 {
                "interrupted"
            } else {
                "recovered"
            };
            let event = Event {
                sequence: delivery.sequence,
                kind: kind.into(),
                status: delivery.status.clone(),
            };
            if done.schema_version != 2
                || done.owner_id != record.owner_id
                || done.notice_id
                    != format!(
                        "{}:{name}:{}:{kind}",
                        record.owner_id, event.status.interruptions
                    )
                || !matches!(done.target.as_str(), "main" | "recorder")
                || !crate::input_levels::valid_owner(&done.recipient_id)
                || !event_valid(&event, name)
                || event.sequence > lane.seen
                || event.status.source_epoch > epoch
                || lane
                    .pending
                    .front()
                    .is_some_and(|h| event.sequence >= h.event.sequence)
                || !(1..=SAFE).contains(&delivery.generation)
                || !delivery.rendered
                || !delivery
                    .receive_id
                    .as_deref()
                    .is_some_and(crate::input_levels::valid_owner)
                || delivery.lease_age_ms.is_none_or(|v| v > SAFE)
                || history.is_some_and(|h| {
                    event.sequence > h.dropped_through && !h.events.contains(&event)
                })
            {
                return Err("通知台帳の終了記録が不正です".into());
            }
        }
    }
    Ok(())
}

fn decode(bytes: &[u8], owner: &str) -> Result<Record, String> {
    if bytes.is_empty() || bytes.len() as u64 > LIMIT {
        return Err("通知台帳の容量が不正です".into());
    }
    // Original-byte decoding rejects duplicate known keys. Round-trip equality
    // also requires explicit nulls rather than serde's optional-field defaults.
    let record: Record = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if record.owner_id != owner
        || serde_json::to_value(&record).map_err(|e| e.to_string())?
            != serde_json::from_slice::<serde_json::Value>(bytes).map_err(|e| e.to_string())?
    {
        return Err("通知台帳の所有または必須項目が不正です".into());
    }
    validate(&record)?;
    Ok(record)
}

impl Ledger {
    pub(in crate::input_status) fn open(
        directory: &File,
        owner: &str,
    ) -> Result<(Self, Option<(Publication, Queue)>), String> {
        if !crate::input_levels::valid_owner(owner) {
            return Err("通知台帳の録音IDが不正です".into());
        }
        let (store, bytes) = file::Store::open(directory)?;
        let previous = bytes.as_deref().map(|b| decode(b, owner)).transpose()?;
        let restored = previous
            .as_ref()
            .map(|r| (r.publication.clone(), r.queue.clone()));
        Ok((
            Self {
                store,
                owner: owner.into(),
                previous,
            },
            restored,
        ))
    }
    pub(in crate::input_status) fn advances(
        p: &Publication,
        old: &Publication,
    ) -> Result<(), String> {
        if p.owner_id != old.owner_id
            || p.schema_version != old.schema_version
            || p.revision < old.revision
            || (p.revision == old.revision && p != old)
            || crate::input_status::backwards(&p.microphone, &old.microphone)
            || p.system.is_some() != old.system.is_some()
            || p.system
                .as_ref()
                .zip(old.system.as_ref())
                .is_some_and(|(s, prior)| crate::input_status::backwards(s, prior))
            || p.microphone_history
                .as_ref()
                .zip(old.microphone_history.as_ref())
                .is_some_and(|(h, prior)| !h.advances(prior))
            || p.system_history
                .as_ref()
                .zip(old.system_history.as_ref())
                .is_some_and(|(h, prior)| !h.advances(prior))
        {
            return Err("通知台帳に対して取得履歴が逆行しました".into());
        }
        Ok(())
    }
    pub(in crate::input_status) fn fence(&mut self, queue: &mut Queue) -> Result<(), String> {
        fence_queue(queue)
    }
    pub(in crate::input_status) fn commit(
        &mut self,
        p: &Publication,
        queue: &Queue,
    ) -> Result<(), String> {
        if let Some(prior) = &self.previous {
            Self::advances(p, &prior.publication)?;
            // A new telemetry heartbeat is not a delivery transition. Avoid
            // syncing the same queue on every 100ms producer revision.
            if serde_json::to_value(queue).map_err(|e| e.to_string())?
                == serde_json::to_value(&prior.queue).map_err(|e| e.to_string())?
                && p.microphone == prior.publication.microphone
                && p.system == prior.publication.system
                && p.microphone_history == prior.publication.microphone_history
                && p.system_history == prior.publication.system_history
            {
                return self.store.verify();
            }
        }
        let record = Record {
            schema_version: 1,
            owner_id: self.owner.clone(),
            publication: p.clone(),
            queue: queue.clone(),
        };
        validate(&record)?;
        let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("通知台帳の容量が上限を超えました".into());
        }
        self.store.commit(&bytes)?;
        self.previous = Some(record);
        Ok(())
    }
}
fn fence_queue(queue: &mut Queue) -> Result<(), String> {
    for lane in &mut queue.lanes {
        // No IPC from a prior application incarnation is idempotently
        // accepted. Keep seen/loss so completed events are not recreated.
        lane.closed = None;
        if lane.pending.front().is_some_and(|h| h.rendered) {
            lane.pending.pop_front();
        }
        if let Some(head) = lane.pending.front_mut() {
            if head.target.is_some() || head.recipient.is_some() {
                head.generation = head
                    .generation
                    .checked_add(1)
                    .filter(|v| *v <= SAFE)
                    .ok_or("通知の受付世代が上限に達しました")?;
            }
            head.target = None;
            head.recipient = None;
            head.claimed = None;
            head.receive_id = None;
        }
    }
    Ok(())
}
