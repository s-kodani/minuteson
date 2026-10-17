//! Replay saved Markdown without inference or natural-language/interval edits.
//! The backup coordinator owns IDs, physical byte provenance and publication.
use super::*;
use crate::generation::restore::{Assets, Context};
use crate::saved_artifact::Role;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const LIMIT: usize = 1024 * 1024;
fn bounded(value: &Value) -> Result<(), String> {
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("議事録Markdownの復元位置が容量上限を超えています".into());
    }
    Ok(())
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("議事録Markdownの復元が中断されました".into())
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
        _ => Err("議事録Markdownの復元成果物種別が不正です".into()),
    }
}
fn visit(
    c: &mut Checkpoint,
    mut transform: impl FnMut(&mut Artifact) -> Result<(), String>,
) -> Result<(), String> {
    for saved in c.outputs.iter_mut().chain(c.pending.iter_mut()) {
        for batch in &mut saved.batches {
            if batch.len() != 4
                || batch.iter().map(|a| a.name.as_str()).ne([
                    "system.txt",
                    "prompt.txt",
                    "schema.json",
                    "result.json",
                ])
            {
                return Err("議事録Markdownの復元成果物が不足・重複・順序不正です".into());
            }
            for a in batch {
                transform(a)?;
            }
        }
    }
    Ok(())
}
pub(in crate::generation) fn references(
    job: &JobSnapshot,
    parameters: Value,
    value: Value,
    mut map: impl FnMut(Role, &str, &str) -> Result<(String, String), String>,
) -> Result<Value, String> {
    bounded(&value)?;
    let p: Parameters = serde_json::from_value(parameters).map_err(|e| e.to_string())?;
    p.engine.validate()?;
    validate_capacity(&p)?;
    p.markdown_target.validate()?;
    if p.schema_version != 7
        || job.kind != "minutes_generation"
        || job.total_units != Some(p.plan.nodes.len() as i64)
        || p.plan.nodes.is_empty()
        || p.plan.nodes.len() > plan::MAX_UNITS
    {
        return Err("議事録Markdownの復元固定条件が不正です".into());
    }
    let mut c = checked_checkpoint(job, &p, Some(value))?;
    visit(&mut c, |a| {
        let (id, hash) = map(role(a)?, &a.id, &a.hash)?;
        if id.is_empty()
            || hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("議事録Markdownの復元参照・ハッシュが不正です".into());
        }
        a.id = id;
        a.hash = hash;
        Ok(())
    })?;
    let value = serde_json::to_value(c).map_err(|e| e.to_string())?;
    bounded(&value)?;
    Ok(value)
}
pub(in crate::generation) fn checkpoint(
    old: Context<'_>,
    new: Context<'_>,
    value: Value,
    assets: &mut impl Assets,
    cancelled: &impl Fn() -> bool,
) -> Result<Value, String> {
    check(cancelled)?;
    bounded(&value)?;
    let old_p = checked_fixed_parameters(old.spec, old.input, old.initial_operation)?;
    let new_p = checked_fixed_parameters(new.spec, new.input, new.initial_operation)?;
    let mut engine = new_p.engine.clone();
    engine.model.path = old_p.engine.model.path.clone();
    engine.executable.path = old_p.engine.executable.path.clone();
    for ctx in [&old, &new] {
        if ctx.spec.input_analysis_id.as_deref() != Some(&ctx.input.analysis.id)
            || ctx.spec.input_hash != ctx.input.analysis.content_hash
            || ctx.spec.meeting_id != ctx.input.analysis.meeting_id
            || ctx.job.input_hash != ctx.spec.input_hash
            || ctx.job.operation_id != ctx.spec.operation_id
            || ctx.job.meeting_id != ctx.spec.meeting_id
            || ctx.job.kind != "minutes_generation"
            || ctx.job.total_units != ctx.spec.total_units
        {
            return Err("議事録Markdownの復元会議・固定入力・処理が一致しません".into());
        }
    }
    if old_p.engine != engine
        || old_p.resident_runtime != new_p.resident_runtime
        || old_p.schema_version != new_p.schema_version
        || old_p.prompt_version != new_p.prompt_version
        || old_p.prompt_snapshot != new_p.prompt_snapshot
        || old_p.acceleration_policy != new_p.acceleration_policy
        || old_p.capacity != new_p.capacity
        || old_p.prompt_budget != new_p.prompt_budget
        || old_p.plan != new_p.plan
        || old_p.markdown_target != new_p.markdown_target
        || old.job.completed_units != new.job.completed_units
    {
        return Err("議事録Markdownの復元前後の計画・保存条件・進捗が一致しません".into());
    }
    let mut c = checked_checkpoint(old.job, &old_p, Some(value))?;
    // Keep hashes, not a second in-memory copy of all historical files. Every
    // later read/write must still match the bytes proved on the first pass.
    let mut proofs = BTreeMap::new();
    for (index, (ctx, p)) in [(&old, &old_p), (&new, &new_p)].into_iter().enumerate() {
        // Recompute prompts under both inputs, never search/replace IDs through
        // the human Markdown. Decoded text and precise times stay exact.
        replay_with(ctx.input, p, &c, cancelled, |task, batch| {
            super::super::verify_task_bytes(task, batch, |a| {
                read(assets, a, &mut proofs, index == 0, cancelled)
            })
        })?
        .ok_or("議事録Markdownの復元照合が中断されました")?;
    }
    check(cancelled)?;
    visit(&mut c, |a| {
        check(cancelled)?;
        let data = read(assets, a, &mut proofs, false, cancelled)?;
        let (id, hash) = assets.write(role(a)?, &a.id, &a.hash, &data)?;
        if id.is_empty() || hash != format!("{:x}", Sha256::digest(&data)) {
            return Err("再構成した議事録Markdown成果物の参照・ハッシュが不正です".into());
        }
        a.id = id;
        a.hash = hash;
        Ok(())
    })?;
    check(cancelled)?;
    let value = serde_json::to_value(c).map_err(|e| e.to_string())?;
    bounded(&value)?;
    checked_checkpoint(new.job, &new_p, Some(value.clone()))?;
    Ok(value)
}

type ByteProofs = BTreeMap<(String, String, String), (usize, String)>;
fn read(
    assets: &mut impl Assets,
    a: &Artifact,
    proofs: &mut ByteProofs,
    record: bool,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    check(cancelled)?;
    let role = role(a)?;
    let data = assets.read(role, &a.id, &a.hash)?;
    if data.len() as u64 > role.max_bytes() {
        return Err("議事録Markdownの復元成果物が容量上限を超えています".into());
    }
    let key = (a.name.clone(), a.id.clone(), a.hash.clone());
    let actual = (data.len(), format!("{:x}", Sha256::digest(&data)));
    if let Some(expected) = proofs.get(&key) {
        if expected != &actual {
            return Err("議事録Markdownの復元中に成果物が変わりました".into());
        }
    } else if record {
        proofs.insert(key, actual);
    } else {
        return Err("照合されていない議事録Markdown成果物です".into());
    }
    check(cancelled)?;
    Ok(data)
}

#[cfg(test)]
mod tests;
