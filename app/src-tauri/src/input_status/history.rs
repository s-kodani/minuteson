//! Validate producer evidence, never reconstruct transitions from the sampled
//! current state. Retention loss has an explicit sequence floor.
use super::{valid_input, Input, SAFE};
use serde::{Deserialize, Serialize};

const CAPACITY: u64 = 64;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Event {
    pub(super) sequence: u64,
    pub(super) kind: String,
    pub(super) status: Input,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct History {
    pub(super) last_sequence: u64,
    pub(super) dropped_through: u64,
    pub(super) events: Vec<Event>,
}

pub(super) fn closed(value: &serde_json::Value) -> bool {
    fn keys(v: &serde_json::Value, names: &[&str]) -> bool {
        v.as_object().is_some_and(|o| {
            o.len() == names.len() && names.iter().all(|name| o.contains_key(*name))
        })
    }
    keys(value, &["lastSequence", "droppedThrough", "events"])
        && value["events"].as_array().is_some_and(|events| {
            events.len() <= CAPACITY as usize
                && events.iter().all(|e| {
                    keys(e, &["sequence", "kind", "status"])
                        && keys(
                            &e["status"],
                            &["state", "sourceEpoch", "interruptions", "reason"],
                        )
                })
        })
}

impl History {
    pub(super) fn valid(&self, current: &Input, input: &str) -> bool {
        if self.last_sequence > SAFE
            || self.dropped_through != self.last_sequence.saturating_sub(CAPACITY)
            || self.events.len() as u64 != self.last_sequence.min(CAPACITY)
            || current.interruptions != self.last_sequence / 2 + self.last_sequence % 2
        {
            return false;
        }
        let mut prior: Option<&Event> = None;
        for (i, event) in self.events.iter().enumerate() {
            let sequence = self.dropped_through + i as u64 + 1;
            let interrupted = sequence % 2 == 1;
            if event.sequence != sequence
                || event.kind
                    != if interrupted {
                        "interrupted"
                    } else {
                        "recovered"
                    }
                || event.status.interruptions != sequence / 2 + sequence % 2
                || !valid_input(&event.status, input)
                || event.status.source_epoch > current.source_epoch
                || if interrupted {
                    !matches!(event.status.state.as_str(), "disconnected" | "unavailable")
                } else {
                    event.status.state != "connected"
                }
                || prior.is_some_and(|old| {
                    event.status.source_epoch < old.status.source_epoch
                        || (!interrupted && event.status.source_epoch <= old.status.source_epoch)
                })
            {
                return false;
            }
            prior = Some(event);
        }
        match self.events.last() {
            None => current.interruptions == 0,
            Some(last) if current.state == "stopped" => {
                current.source_epoch >= last.status.source_epoch
            }
            Some(last) if last.kind == "recovered" => current == &last.status,
            Some(_) => current.state != "connected",
        }
    }

    pub(super) fn advances(&self, old: &Self) -> bool {
        self.last_sequence >= old.last_sequence
            && self.dropped_through >= old.dropped_through
            && (self.last_sequence != old.last_sequence || self == old)
            && old.events.iter().all(|event| {
                event.sequence <= self.dropped_through || self.events.iter().any(|new| new == event)
            })
    }
}
