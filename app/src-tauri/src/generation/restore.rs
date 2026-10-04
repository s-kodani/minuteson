//! Semantic replay of saved generation units in an owned restore workspace.
//! No executable is launched and no model path is made available here. The
//! coordinator still owns archive provenance, ID ownership and publication.
use super::*;
use crate::saved_artifact::Role;
use sha2::{Digest, Sha256};

pub(crate) struct Context<'a> {
    pub job: &'a JobSnapshot,
    pub spec: &'a JobSpec,
    pub input: &'a MinutesInput,
    // The coordinator verifies this against the bounded retry lineage.
    pub initial_operation: &'a str,
}

pub(crate) trait Assets {
    // Must check exact meeting/asset ownership, original logical hash, media
    // type, and physical archive hash before returning these bounded bytes.
    fn read(&mut self, role: Role, id: &str, hash: &str) -> Result<Vec<u8>, String>;
    // Only a private stage may be written; repeated historical references must
    // yield identical bytes/IDs/hashes, never overwrite a conflicting file.
    fn write(
        &mut self,
        role: Role,
        id: &str,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(String, String), String>;
    fn map(&mut self, domain: &str, id: &str) -> Result<String, String>;
}

const LIMIT: usize = 2 * 1024 * 1024;
fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("生成成果物の復元が中断されました".into())
    } else {
        Ok(())
    }
}
fn role(artifact: &Artifact) -> Result<Role, String> {
    match artifact.name.as_str() {
        "system.txt" => Ok(Role::GenerationSystem),
        "prompt.txt" => Ok(Role::GenerationPrompt),
        "schema.json" => Ok(Role::GenerationSchema),
        "result.json" => Ok(Role::GenerationRaw),
        "items.json" => Ok(Role::GenerationItems),
        _ => Err("復元する生成成果物の種別が不正です".into()),
    }
}
fn read(assets: &mut impl Assets, artifact: &Artifact) -> Result<Vec<u8>, String> {
    let bytes = assets.read(role(artifact)?, &artifact.id, &artifact.hash)?;
    if bytes.len() > LIMIT {
        return Err("生成成果物が容量上限を超えています".into());
    }
    Ok(bytes)
}
fn write(assets: &mut impl Assets, artifact: &mut Artifact, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > LIMIT {
        return Err("再構成した生成成果物が容量上限を超えています".into());
    }
    let (id, hash) = assets.write(role(artifact)?, &artifact.id, &artifact.hash, bytes)?;
    if id.is_empty() || hash != format!("{:x}", Sha256::digest(bytes)) {
        return Err("再構成した生成成果物のID・ハッシュが不正です".into());
    }
    artifact.id = id;
    artifact.hash = hash;
    Ok(())
}
fn mapped(assets: &mut impl Assets, items: &[GeneratedItem]) -> Result<Vec<GeneratedItem>, String> {
    let mut result = items.to_vec();
    for item in &mut result {
        if let Some(id) = &mut item.assignee_speaker_id {
            *id = assets.map("speakers", id)?;
        }
        for id in &mut item.evidence_utterance_ids {
            *id = assets.map("utterances", id)?;
        }
    }
    Ok(result)
}
fn normalized(
    assets: &mut impl Assets,
    artifact: &mut Artifact,
    old: &[GeneratedItem],
    new: &[GeneratedItem],
) -> Result<(), String> {
    if artifact.name != "items.json"
        || serde_json::from_slice::<Vec<GeneratedItem>>(&read(assets, artifact)?)
            .map_err(|e| e.to_string())?
            != old
        || mapped(assets, old)? != new
    {
        return Err("生成原文・保存項目・再構成した参照が一致しません".into());
    }
    write(
        assets,
        artifact,
        &serde_json::to_vec(new).map_err(|e| e.to_string())?,
    )
}
fn save_task(
    assets: &mut impl Assets,
    artifacts: &mut [Artifact],
    task: &Task,
    raw: &Value,
) -> Result<(), String> {
    if task.local.as_ref().is_some_and(|local| local != raw) {
        return Err("再構成したローカル生成結果が一致しません".into());
    }
    let bytes = [
        task.system.as_bytes().to_vec(),
        task.prompt.as_bytes().to_vec(),
        serde_json::to_vec(&task.schema).map_err(|e| e.to_string())?,
        serde_json::to_vec(raw).map_err(|e| e.to_string())?,
    ];
    // verify_task_bytes has already checked this exact ordered artifact set.
    for (artifact, bytes) in artifacts.iter_mut().zip(bytes) {
        write(assets, artifact, &bytes)?;
    }
    Ok(())
}

struct Replay<'a, A> {
    old: &'a Context<'a>,
    new: &'a Context<'a>,
    old_parameters: Parameters,
    new_parameters: Parameters,
    assets: &'a mut A,
    old_outputs: Vec<Vec<GeneratedItem>>,
    new_outputs: Vec<Vec<GeneratedItem>>,
}
impl<A: Assets> Replay<'_, A> {
    fn batches(
        &mut self,
        index: usize,
        batches: &mut [SavedBatch],
        check: &impl Fn() -> bool,
    ) -> Result<(adaptive::Cursor, adaptive::Cursor), String> {
        if batches.len() > 4096 {
            return Err("復元する統合単位数が上限を超えています".into());
        }
        let mut old = merge_cursor(
            &self.old_parameters.plan.nodes[index],
            &self.old_outputs,
            &self.old_parameters,
        )?;
        let mut new = merge_cursor(
            &self.new_parameters.plan.nodes[index],
            &self.new_outputs,
            &self.new_parameters,
        )?;
        for batch in batches {
            cancelled(check)?;
            let old_task = merge_task(&old)?;
            let new_task = merge_task(&new)?;
            let raw = verify_task_bytes(&old_task, &batch.artifacts, |a| read(self.assets, a))?;
            // Merge IDs describe positions (n…/r1-b…), not application IDs.
            let old_items = merge::parse(raw.clone(), self.old.input, old.next().unwrap())?;
            let new_items = merge::parse(raw.clone(), self.new.input, new.next().unwrap())?;
            normalized(self.assets, &mut batch.normalized, &old_items, &new_items)?;
            save_task(self.assets, &mut batch.artifacts, &new_task, &raw)?;
            old.accept(old_items)?;
            new.accept(new_items)?;
        }
        // A partial merge must reach the same structural position after ID
        // mapping. A changed partition must never silently discard work.
        if old
            .next()
            .map(|s| s.iter().map(|(id, _)| id).collect::<Vec<_>>())
            != new
                .next()
                .map(|s| s.iter().map(|(id, _)| id).collect::<Vec<_>>())
        {
            return Err("参照再構成後の統合再開位置が一致しません".into());
        }
        Ok((old, new))
    }
    fn unit(&mut self, saved: &mut UnitOutput, check: &impl Fn() -> bool) -> Result<(), String> {
        cancelled(check)?;
        let node = &self.old_parameters.plan.nodes[saved.index];
        let (old_items, new_items) = if self.old_parameters.schema_version == 2
            && matches!(node, plan::Node::Merge { .. })
        {
            if !saved.artifacts.is_empty() || saved.merge_batches.is_empty() {
                return Err("復元する分割統合の完了形式が不正です".into());
            }
            let (old, new) = self.batches(saved.index, &mut saved.merge_batches, check)?;
            let (old, new) = (old.finish()?, new.finish()?);
            from_generated(self.old.input, old.clone())?;
            from_generated(self.new.input, new.clone())?;
            (old, new)
        } else {
            if !saved.merge_batches.is_empty() {
                return Err("抽出・従来統合に分割統合成果物が混入しています".into());
            }
            let old_task = task(
                self.old.input,
                node,
                &self.old_outputs,
                &self.old_parameters,
            )?;
            let new_task = task(
                self.new.input,
                node,
                &self.new_outputs,
                &self.new_parameters,
            )?;
            let raw = verify_task_bytes(&old_task, &saved.artifacts, |a| read(self.assets, a))?;
            let old = parse_node(raw.clone(), self.old.input, node, &self.old_outputs)?;
            let new_raw = if matches!(node, plan::Node::Extract { .. }) {
                // Old parse has already enforced all required/unknown fields.
                // Only typed references change; natural-language bytes survive.
                json!({"schemaVersion":1,"items":mapped(self.assets, &old)?})
            } else {
                raw
            };
            let new = parse_node(new_raw.clone(), self.new.input, node, &self.new_outputs)?;
            if mapped(self.assets, &old)? != new {
                return Err("再構成した生成原文の意味・参照が一致しません".into());
            }
            save_task(self.assets, &mut saved.artifacts, &new_task, &new_raw)?;
            (old, new)
        };
        normalized(self.assets, &mut saved.normalized, &old_items, &new_items)?;
        self.old_outputs.push(old_items);
        self.new_outputs.push(new_items);
        Ok(())
    }
}

#[allow(dead_code)] // Wired into the private restore construction in a later stage.
pub(crate) fn checkpoint(
    old: Context<'_>,
    new: Context<'_>,
    value: Value,
    assets: &mut impl Assets,
    check: &impl Fn() -> bool,
) -> Result<Value, String> {
    cancelled(check)?;
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("復元元の生成チェックポイントが容量上限を超えています".into());
    }
    let old_parameters = checked_parameters(old.spec, old.input, old.initial_operation)?;
    let new_parameters = checked_parameters(new.spec, new.input, new.initial_operation)?;
    let mut engine = new_parameters.engine.clone();
    // Portable restoration can change locations, never the fixed model,
    // executable identity or inference settings used for these saved outputs.
    engine.model.path = old_parameters.engine.model.path.clone();
    engine.executable.path = old_parameters.engine.executable.path.clone();
    for context in [&old, &new] {
        if context.spec.input_analysis_id.as_deref() != Some(&context.input.analysis.id)
            || context.spec.input_hash != context.input.analysis.content_hash
            || context.spec.meeting_id != context.input.analysis.meeting_id
            || context.job.input_hash != context.spec.input_hash
        {
            return Err("復元する生成結果の固定入力・会議が一致しません".into());
        }
    }
    if old_parameters.schema_version != new_parameters.schema_version
        || old_parameters.prompt_budget != new_parameters.prompt_budget
        || old_parameters.engine != engine
        || old_parameters.plan != new_parameters.plan
        || old.job.completed_units != new.job.completed_units
        || old.job.total_units != old.spec.total_units
        || new.job.total_units != new.spec.total_units
        || old.job.meeting_id != old.spec.meeting_id
        || new.job.meeting_id != new.spec.meeting_id
        || old.job.kind != "minutes_generation"
        || new.job.kind != "minutes_generation"
    {
        return Err("復元前後の生成固定計画・進捗が一致しません".into());
    }
    let mut checkpoint = checked_checkpoint(old.job, &old_parameters, Some(value))?;
    let mut replay = Replay {
        old: &old,
        new: &new,
        old_parameters,
        new_parameters,
        assets,
        old_outputs: vec![],
        new_outputs: vec![],
    };
    for saved in &mut checkpoint.outputs {
        replay.unit(saved, check)?;
    }
    if let Some(pending) = &mut checkpoint.pending {
        replay.batches(pending.index, &mut pending.batches, check)?;
    }
    cancelled(check)?;
    let result = serde_json::to_value(checkpoint).map_err(|e| e.to_string())?;
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > 1024 * 1024
    {
        return Err("再構成した生成チェックポイントが容量上限を超えています".into());
    }
    // Reuse the runtime checkpoint structural validator for the mapped job too.
    checked_checkpoint(new.job, &replay.new_parameters, Some(result.clone()))?;
    Ok(result)
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
