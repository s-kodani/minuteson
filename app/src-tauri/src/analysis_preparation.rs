//! Transient observations. Durable flow/job reads remain authoritative.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    pub meeting_title: String,
    pub meeting_id: String,
    pub operation_id: String,
    pub phase: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub started_at_ms: i64,
    pub sequence: u64,
}
fn entries() -> &'static Mutex<BTreeMap<String, Progress>> {
    static STATE: OnceLock<Mutex<BTreeMap<String, Progress>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}
pub(crate) struct Scope {
    meeting: String,
    operation: String,
    owns: bool,
}
pub(crate) fn begin(meeting: &str, operation: &str) -> Scope {
    let mut owns = false;
    if let Ok(mut entries) = entries().lock() {
        if !entries
            .get(meeting)
            .is_some_and(|p| p.operation_id == operation)
        {
            entries.insert(
                meeting.into(),
                Progress {
                    meeting_title: String::new(),
                    meeting_id: meeting.into(),
                    operation_id: operation.into(),
                    phase: "waiting".into(),
                    completed: None,
                    total: None,
                    started_at_ms: chrono::Utc::now().timestamp_millis(),
                    sequence: 0,
                },
            );
            owns = true;
        }
    }
    Scope {
        meeting: meeting.into(),
        operation: operation.into(),
        owns,
    }
}

pub(crate) fn update(
    meeting: &str,
    operation: &str,
    phase: &str,
    completed: Option<u64>,
    total: Option<u64>,
) {
    if let Ok(mut entries) = entries().lock() {
        if let Some(p) = entries
            .get_mut(meeting)
            .filter(|p| p.operation_id == operation)
        {
            p.phase = phase.into();
            p.completed = completed;
            p.total = total;
            p.sequence += 1;
        }
    }
}
pub(crate) fn snapshot() -> Vec<Progress> {
    entries()
        .lock()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default()
}
impl Drop for Scope {
    fn drop(&mut self) {
        if let Ok(mut entries) = entries().lock() {
            if self.owns
                && entries
                    .get(&self.meeting)
                    .is_some_and(|p| p.operation_id == self.operation)
            {
                entries.remove(&self.meeting);
            }
        }
    }
}
