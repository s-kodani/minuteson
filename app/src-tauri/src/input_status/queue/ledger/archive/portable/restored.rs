//! Mutable destination delivery, bound to immutable producer evidence. This
//! value is not a writer capability: Storage must commit under its row/root
//! ownership and maintenance lease before returning a batch or receipt.
use super::Snapshot;
use crate::input_status::{
    history::Event,
    queue::{Action, Queue},
    Receipt, SAFE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub(crate) const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct State {
    schema_version: u8,
    owner_id: String,
    source_hash: String,
    revision: u64,
    queue: Queue,
    // Only the last removed retained event per lane is needed to bind the
    // remaining ordered suffix. Older source proof stays in Snapshot unchanged.
    // A receipt retained here is evidence, NEVER idempotent IPC authority.
    removed: [Option<Receipt>; 2],
    // Preserve actual durable render receipts. A cold queue has no Instant;
    // never manufacture a fresh lease or a receipt when suppressing its head.
    rendered: [Option<Receipt>; 2],
}

fn fingerprint(source: &Snapshot) -> Result<String, String> {
    Ok(format!("{:x}", Sha256::digest(source.encode()?)))
}

impl State {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn owner(&self) -> &str {
        &self.owner_id
    }

    pub(crate) fn initial(source: &Snapshot, owner: &str) -> Result<Option<Self>, String> {
        if !crate::input_levels::valid_owner(owner) {
            return Err("復元通知の録音所属が不正です".into());
        }
        let source_hash = fingerprint(source)?;
        let Some(seed) = &source.delivery else {
            return Ok(None);
        };
        let result = Self {
            schema_version: 1,
            owner_id: owner.into(),
            source_hash,
            revision: 1,
            queue: seed.queue.clone(),
            removed: [None, None],
            rendered: [None, None],
        };
        result.validate(source, owner)?;
        Ok(Some(result))
    }

    fn proof(
        source: &Snapshot,
        index: usize,
        proof: &Receipt,
        event: &Event,
    ) -> Result<(), String> {
        let seed = source
            .delivery
            .as_ref()
            .ok_or("復元通知の元記録がありません")?;
        let context = super::super::bounds(seed)?;
        let delivery = proof
            .delivery
            .as_ref()
            .ok_or("復元通知の表示証拠がありません")?;
        if proof.target != "main"
            || delivery.sequence != event.sequence
            || delivery.status != event.status
        {
            return Err("復元通知の表示証拠と元の遷移が一致しません".into());
        }
        let mut queue = seed.queue.clone();
        for lane in &mut queue.lanes {
            lane.pending.clear();
            lane.closed = None;
        }
        queue.lanes[index].closed = Some(proof.clone());
        super::super::super::validate_queue(&context, &queue, &proof.owner_id)
    }

    fn validate(&self, source: &Snapshot, owner: &str) -> Result<(), String> {
        if self.schema_version != 1
            || self.owner_id != owner
            || !crate::input_levels::valid_owner(owner)
            || !(1..=SAFE).contains(&self.revision)
            || self.source_hash != fingerprint(source)?
        {
            return Err("復元通知の所属・元記録または版が不正です".into());
        }
        let seed = source
            .delivery
            .as_ref()
            .ok_or("復元通知の元記録がありません")?;
        let context = super::super::bounds(seed)?;
        super::super::super::validate_queue(&context, &self.queue, owner)?;
        for index in 0..2 {
            let original = &seed.queue.lanes[index];
            let current = &self.queue.lanes[index];
            if current.seen != original.seen
                || current.lost != original.lost
                || current.pending.len() > original.pending.len()
            {
                return Err("復元通知の保持範囲が元記録と一致しません".into());
            }
            let removed = original.pending.len() - current.pending.len();
            for (old, new) in original.pending.iter().skip(removed).zip(&current.pending) {
                if old.event != new.event || new.generation < old.generation {
                    return Err("復元通知の残りの順序・遷移または受付世代が不正です".into());
                }
            }
            match (removed, &self.removed[index]) {
                (0, None) => {}
                (n, Some(proof)) if n > 0 => {
                    Self::proof(source, index, proof, &original.pending[n - 1].event)?
                }
                _ => return Err("復元通知の表示済み範囲の証拠がありません".into()),
            }
            if let Some(closed) = &current.closed {
                if self.removed[index].as_ref() != Some(closed) {
                    return Err("復元通知の終了証拠が表示済み範囲と一致しません".into());
                }
            }
            match (current.pending.front(), &self.rendered[index]) {
                (Some(head), Some(proof)) if head.rendered => {
                    Self::proof(source, index, proof, &head.event)?;
                    let delivery = proof.delivery.as_ref().unwrap();
                    if proof.owner_id != owner
                        || head.target.as_deref() != Some(&proof.target)
                        || head.recipient.as_deref() != Some(&proof.recipient_id)
                        || delivery.generation != head.generation
                        || delivery.receive_id != head.receive_id
                    {
                        return Err("復元通知の実表示と受付先が一致しません".into());
                    }
                }
                (Some(head), None) if !head.rendered => {}
                (None, None) => {}
                _ => return Err("復元通知の実表示の証拠が不正です".into()),
            }
            if current
                .pending
                .iter()
                .any(|head| head.target.as_deref().is_some_and(|t| t != "main"))
            {
                return Err("復元通知を録音画面へ配送できません".into());
            }
        }
        Ok(())
    }

    pub(crate) fn decode(bytes: &[u8], source: &Snapshot, owner: &str) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("復元通知台帳の容量が不正です".into());
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if serde_json::to_value(&result).map_err(|e| e.to_string())?
            != serde_json::from_slice::<serde_json::Value>(bytes).map_err(|e| e.to_string())?
        {
            return Err("復元通知台帳の必須項目が不正です".into());
        }
        result.validate(source, owner)?;
        Ok(result)
    }
    pub(crate) fn encode(&self, source: &Snapshot) -> Result<Vec<u8>, String> {
        self.validate(source, &self.owner_id)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("復元通知台帳が容量上限を超えました".into());
        }
        Ok(bytes)
    }

    fn finish(&self, source: &Snapshot, mut candidate: Self) -> Result<Self, String> {
        // Instant renewal is local only. A heartbeat without a durable delivery
        // transition must not advance the revision or sync SQLite again.
        if serde_json::to_value(&candidate).map_err(|e| e.to_string())?
            != serde_json::to_value(self).map_err(|e| e.to_string())?
        {
            candidate.revision = self
                .revision
                .checked_add(1)
                .filter(|n| *n <= SAFE)
                .ok_or("復元通知の版が上限に達しました")?;
        }
        candidate.encode(source)?;
        Ok(candidate)
    }

    pub(crate) fn fenced(&self, source: &Snapshot, owner: &str) -> Result<Self, String> {
        self.validate(source, &self.owner_id)?;
        let mut candidate = self.clone();
        for index in 0..2 {
            if candidate.queue.lanes[index]
                .pending
                .front()
                .is_some_and(|h| h.rendered)
            {
                candidate.removed[index] = candidate.rendered[index].take();
            }
        }
        super::super::super::fence_queue(&mut candidate.queue)?;
        candidate.owner_id = owner.into();
        self.finish(source, candidate)
    }

    pub(crate) fn polled(
        &self,
        source: &Snapshot,
        visible: bool,
    ) -> Result<(Self, super::super::Batch), String> {
        self.validate(source, &self.owner_id)?;
        let seed = source.delivery.as_ref().unwrap();
        let context = super::super::bounds(seed)?;
        let mut candidate = self.clone();
        let started = Instant::now();
        let mut notices = candidate.queue.observe_histories(
            &candidate.owner_id,
            [
                context.microphone_history.as_ref(),
                context.system_history.as_ref(),
            ],
            visible,
            visible.then_some("main"),
            started,
        );
        for index in 0..2 {
            if candidate.queue.lanes[index].pending.len() < self.queue.lanes[index].pending.len() {
                candidate.removed[index] = candidate.queue.lanes[index].closed.clone();
                candidate.rendered[index] = None;
            }
        }
        let candidate = self.finish(source, candidate)?;
        for notice in &mut notices {
            if let Some(delivery) = &mut notice.delivery {
                delivery.account_persistence_time(started.elapsed());
            }
        }
        let batch = super::super::Batch {
            schema_version: 1,
            owner_id: candidate.owner_id.clone(),
            revision: candidate.revision,
            notices,
            notice_loss: candidate.queue.loss(),
            history_issue: seed.history_issue.clone(),
        };
        Ok((candidate, batch))
    }

    pub(crate) fn acted(
        &self,
        source: &Snapshot,
        id: &str,
        recipient: &str,
        generation: u64,
        action: Action,
        receive_id: Option<&str>,
        visible: bool,
    ) -> Result<(Self, Receipt), String> {
        self.validate(source, &self.owner_id)?;
        if !visible
            || !crate::input_levels::valid_owner(recipient)
            || id.len() > 128
            || !(1..=SAFE).contains(&generation)
            || (!matches!(action, Action::Receive) && receive_id.is_some())
        {
            return Err("復元通知の画面または受付引数が不正です".into());
        }
        let mut candidate = self.clone();
        let started = Instant::now();
        let mut receipt = candidate.queue.act(
            &candidate.owner_id,
            id,
            recipient,
            "main",
            generation,
            action,
            started,
            receive_id,
        )?;
        for index in 0..2 {
            if candidate.queue.lanes[index].pending.len() < self.queue.lanes[index].pending.len() {
                candidate.removed[index] = candidate.queue.lanes[index].closed.clone();
                candidate.rendered[index] = None;
            } else if candidate.queue.lanes[index]
                .pending
                .front()
                .is_some_and(|h| {
                    h.rendered && h.event.sequence == receipt.delivery.as_ref().unwrap().sequence
                })
                && receipt.notice_id.contains(if index == 0 {
                    ":microphone:"
                } else {
                    ":system:"
                })
            {
                candidate.rendered[index] = Some(receipt.clone());
            }
        }
        let candidate = self.finish(source, candidate)?;
        if let Some(delivery) = &mut receipt.delivery {
            delivery.account_persistence_time(started.elapsed());
        }
        Ok((candidate, receipt))
    }
}

#[cfg(test)]
mod tests;
