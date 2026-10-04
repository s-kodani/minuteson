//! Version-2 bounded merge passes. Every source is consumed exactly once per
//! pass; a second, transposed pass brings different windows together. This is
//! not a claim that an unevaluated model finds every semantic duplicate.
use super::merge;
use crate::minutes_document::GeneratedItem;
use serde_json::json;
use std::collections::{HashMap, HashSet};

pub(super) type Sources = Vec<(String, GeneratedItem)>;
pub(super) struct Cursor {
    batches: Vec<Sources>,
    next: usize,
    results: Vec<Vec<GeneratedItem>>,
    cross_pass: bool,
    input_limit: usize,
    output_limit: usize,
}

pub(super) fn identity(sources: &Sources) -> serde_json::Value {
    json!({"schemaVersion":1,"items":sources.iter().map(|(id,item)|json!({"body":item.body,"sourceItemIds":[id]})).collect::<Vec<_>>()})
}
fn partition(
    sources: Sources,
    input_limit: usize,
    output_limit: usize,
) -> Result<(Vec<Sources>, bool), String> {
    if sources.len() > 10_000 {
        return Err("統合項目数が文書上限を超えています".into());
    }
    let mut ids = HashSet::new();
    let mut groups: Vec<Sources> = vec![];
    let mut group_indices = HashMap::new();
    for (id, item) in sources {
        if !ids.insert(id.clone()) {
            return Err("統合元の項目IDが重複しています".into());
        }
        // Metadata is frozen. Incompatible items never need semantic merging.
        let key = serde_json::to_string(&(
            item.kind,
            &item.assignee_name,
            &item.assignee_speaker_id,
            &item.due_date,
            &item.due_text,
        ))
        .map_err(|e| e.to_string())?;
        let index = *group_indices.entry(key).or_insert_with(|| {
            groups.push(vec![]);
            groups.len() - 1
        });
        groups[index].push((id, item));
    }
    let fits = |batch: &Sources| -> Result<bool, String> {
        Ok(batch.len() <= 1000
            && merge::SYSTEM.len() + merge::prompt(batch)?.len() + 256 <= input_limit
            && serde_json::to_vec(&merge::schema(batch)).map_err(|e|e.to_string())?.len() <= 32*1024
            // The model must also have room to retain distinct items. This
            // conservative byte estimate is not an exact tokenizer count.
            && serde_json::to_vec(&identity(batch)).map_err(|e|e.to_string())?.len() + 128 <= output_limit)
    };
    let mut batches = vec![];
    let mut cross_pass = false;
    let mut local = vec![];
    for group in groups {
        let first_batch = batches.len();
        let mut batch = vec![];
        for source in group {
            batch.push(source);
            if batch.len() > 1 && !fits(&batch)? {
                let last = batch.pop().unwrap();
                batches.push(std::mem::take(&mut batch));
                batch.push(last);
            }
        }
        if batch.len() == 1 && batches.len() == first_batch {
            // Coalesce incompatible singleton groups into a local identity
            // operation, avoiding one inference/file set per metadata group.
            local.extend(batch);
        } else if !batch.is_empty() {
            batches.push(batch);
        }
        let group_batches = &batches[first_batch..];
        cross_pass |= group_batches.len() > 1 && group_batches.iter().any(|b| b.len() > 1);
    }
    for batch in local.chunks(1000) {
        batches.push(batch.to_vec());
    }
    if batches.is_empty() {
        batches.push(vec![]);
    }
    Ok((batches, cross_pass))
}

impl Cursor {
    pub(super) fn new(
        sources: Sources,
        input_limit: usize,
        output_limit: usize,
    ) -> Result<Self, String> {
        let (batches, cross_pass) = partition(sources, input_limit, output_limit)?;
        Ok(Self {
            batches,
            next: 0,
            results: vec![],
            cross_pass,
            input_limit,
            output_limit,
        })
    }
    pub(super) fn next(&self) -> Option<&Sources> {
        self.batches.get(self.next)
    }
    pub(super) fn local(&self) -> bool {
        let Some(batch) = self.next() else {
            return false;
        };
        batch.len() <= 1
            || batch.windows(2).any(|pair| {
                let a = &pair[0].1;
                let b = &pair[1].1;
                (
                    a.kind,
                    &a.assignee_name,
                    &a.assignee_speaker_id,
                    &a.due_date,
                    &a.due_text,
                ) != (
                    b.kind,
                    &b.assignee_name,
                    &b.assignee_speaker_id,
                    &b.due_date,
                    &b.due_text,
                )
            })
    }
    pub(super) fn accept(&mut self, items: Vec<GeneratedItem>) -> Result<(), String> {
        if self.next().is_none() {
            return Err("統合単位の再開位置が終端を超えています".into());
        }
        self.results.push(items);
        self.next += 1;
        if self.next == self.batches.len() && self.cross_pass {
            let mut sources = vec![];
            let width = self.results.iter().map(Vec::len).max().unwrap_or(0);
            // Transpose instead of repeating the exact same partition: adjacent
            // batches' outputs can be reconciled without an oversized prompt.
            for item in 0..width {
                for (batch, output) in self.results.iter().enumerate() {
                    if let Some(value) = output.get(item) {
                        sources.push((format!("r1-b{batch}-i{item}"), value.clone()));
                    }
                }
            }
            self.batches = partition(sources, self.input_limit, self.output_limit)?.0;
            self.next = 0;
            self.results.clear();
            self.cross_pass = false;
        }
        Ok(())
    }
    pub(super) fn finish(self) -> Result<Vec<GeneratedItem>, String> {
        if self.next().is_some() {
            return Err("統合単位が未完了です".into());
        }
        Ok(self.results.into_iter().flatten().collect())
    }
}

#[cfg(test)]
#[path = "adaptive_tests.rs"]
mod tests;
