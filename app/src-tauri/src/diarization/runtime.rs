//! Display-only engine observations; checkpoints remain the only durable progress.
use crate::storage::JobSnapshot;
use serde::Serialize;
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    job_id: String,
    spec_hash: String,
    attempt: i64,
    unit_index: usize,
    invocation_order: u64,
    sequence: u64,
    engine_basis_points: u32,
    finishing: bool,
}

pub(super) struct Observer<'a> {
    job: &'a JobSnapshot,
    index: usize,
    order: u64,
    sequence: Cell<u64>,
    publish: &'a dyn Fn(Progress),
}

impl<'a> Observer<'a> {
    pub(super) fn new(job: &'a JobSnapshot, index: usize, publish: &'a dyn Fn(Progress)) -> Self {
        Self {
            job,
            index,
            order: NEXT_INVOCATION.fetch_add(1, Ordering::Relaxed),
            sequence: Cell::new(0),
            publish,
        }
    }

    pub(super) fn emit(&self, engine_basis_points: u32, finishing: bool) {
        self.sequence.set(self.sequence.get() + 1);
        (self.publish)(Progress {
            job_id: self.job.id.clone(),
            spec_hash: self.job.spec_hash.clone(),
            attempt: self.job.attempt,
            unit_index: self.index,
            invocation_order: self.order,
            sequence: self.sequence.get(),
            engine_basis_points,
            finishing,
        });
    }
}
