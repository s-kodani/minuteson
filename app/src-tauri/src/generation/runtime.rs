//! Ephemeral observations. A unit only completes through its durable checkpoint.
use super::llama_metrics::Phase;
use crate::storage::JobSnapshot;
use serde::Serialize;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    job_id: String,
    spec_hash: String,
    attempt: i64,
    unit_index: usize,
    invocation: String,
    invocation_order: u64,
    started_at_utc_ms: i64,
    sequence: u64,
    phase: Phase,
    merge: bool,
    backend: Option<&'static str>,
    elapsed_ms: u128,
}
pub(super) struct Observer<'a> {
    job: &'a JobSnapshot,
    started: Instant,
    started_at: i64,
    invocation: String,
    invocation_order: u64,
    sequence: Cell<u64>,
    index: usize,
    merge: bool,
    backend: Option<&'static str>,
    publish: &'a dyn Fn(Progress),
}
impl<'a> Observer<'a> {
    pub(super) fn new(
        job: &'a JobSnapshot,
        index: usize,
        merge: bool,
        publish: &'a dyn Fn(Progress),
    ) -> Self {
        Self {
            job,
            started: Instant::now(),
            started_at: chrono::Utc::now().timestamp_millis(),
            invocation: uuid::Uuid::new_v4().to_string(),
            invocation_order: NEXT_INVOCATION.fetch_add(1, Ordering::Relaxed),
            sequence: Cell::new(0),
            index,
            merge,
            backend: None,
            publish,
        }
    }
    pub(super) fn with_backend(mut self, backend: Option<&'static str>) -> Self {
        self.backend = backend;
        self
    }
    pub(super) fn emit(&self, phase: Phase) {
        self.sequence.set(self.sequence.get() + 1);
        (self.publish)(Progress {
            job_id: self.job.id.clone(),
            spec_hash: self.job.spec_hash.clone(),
            attempt: self.job.attempt,
            unit_index: self.index,
            invocation: self.invocation.clone(),
            invocation_order: self.invocation_order,
            started_at_utc_ms: self.started_at,
            sequence: self.sequence.get(),
            phase,
            merge: self.merge,
            backend: self.backend,
            elapsed_ms: self.started.elapsed().as_millis(),
        });
    }
}
