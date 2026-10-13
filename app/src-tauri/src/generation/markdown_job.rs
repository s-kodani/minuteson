//! Markdown-only generation. No candidate adoption or structured evidence.
use super::{markdown, markdown_plan as plan, model_budget::Capacity, Artifact, Task};
use crate::storage::minutes_markdown::{content_hash, GenerationTarget, MarkdownStamp};
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::{JobKind, JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub(super) mod restore;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    schema_version: u32,
    engine: super::llama::LlamaConfig,
    capacity: Capacity,
    prompt_budget: usize,
    plan: plan::Plan,
    markdown_target: GenerationTarget,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope {
    schema_version: u32,
    request: MinutesJobRequest,
    generation: Parameters,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Checkpoint {
    outputs: Vec<Output>,
    pending: Option<Output>,
    markdown_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Output {
    index: usize,
    batches: Vec<Vec<Artifact>>,
}

pub(crate) fn is_markdown(spec: &JobSpec) -> bool {
    spec.kind == JobKind::MinutesGeneration
        && matches!(
            spec.parameters["generation"]["schemaVersion"].as_u64(),
            Some(7)
        )
}
pub(crate) fn enqueue(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::reviewed(&engine.model)?;
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
    )
}
fn enqueue_with_capacity(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    capacity: Capacity,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    if !matches!(name, "standard" | "high_accuracy") || prompt_budget > super::budget(&engine)? {
        return Err("議事録プロファイルまたは入力上限が不正です".into());
    }
    engine.verify(&|| false)?;
    let plan_limit = capacity.plan_limit(&engine, prompt_budget)?;
    let selected = storage.job_model_selection(
        &request.meeting_id,
        &request.operation_id,
        name,
        Execution::Llama(&engine, prompt_budget),
    )?;
    let profile = managed::attach(super::profile(&engine, name), selected)?;
    let target = GenerationTarget {
        schema_version: 1,
        expected,
    };
    storage.enqueue_markdown_minutes_job_configured(request,profile,target,|input|{
        let plan=plan::build(input,plan_limit)?;let total=plan.nodes.len() as i64;
        // Storage binds the fixed target once within its admission transaction.
        let parameters=serde_json::json!({"schemaVersion":7,"engine":engine,"capacity":capacity,"promptBudget":prompt_budget,"plan":plan});
        Ok((parameters,Some(total)))
    })
}
fn checked_parameters(
    storage: &Storage,
    job_id: &str,
    spec: &JobSpec,
    input: &MinutesInput,
) -> Result<Parameters, String> {
    checked_fixed_parameters(spec, input, &storage.fixed_job_initial_operation(job_id)?)
}
// No filesystem access: the restore image intentionally has unavailable model
// paths. Its coordinator has separately verified the input and retry lineage.
fn checked_fixed_parameters(
    spec: &JobSpec,
    input: &MinutesInput,
    initial_operation: &str,
) -> Result<Parameters, String> {
    let envelope: Envelope =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    let p = envelope.generation;
    p.engine.validate()?;
    validate_capacity(&p)?;
    p.markdown_target.validate()?;
    let name = spec.profile_snapshot["profile"]
        .as_str()
        .ok_or("議事録プロファイルがありません")?;
    if spec.kind != JobKind::MinutesGeneration
        || envelope.schema_version != 1
        || p.schema_version != 7
        || p.markdown_target.schema_version != 1
        || envelope.request.meeting_id != spec.meeting_id
        || envelope.request.operation_id != initial_operation
        || !matches!(name, "standard" | "high_accuracy")
        || p.prompt_budget > super::budget(&p.engine)?
        || p.plan != plan::build(input, plan_limit(&p)?)?
        || spec.total_units != Some(p.plan.nodes.len() as i64)
        || managed::rebinding::base_profile(&spec.profile_snapshot)?
            != managed::attach(
                super::profile(&p.engine, name),
                managed::read(&spec.profile_snapshot)?,
            )?
    {
        return Err("議事録Markdownの固定入力・モデル・保存条件・計画が不正です".into());
    }
    managed::validate(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
    )?;
    Ok(p)
}
fn checked_checkpoint(
    job: &JobSnapshot,
    p: &Parameters,
    value: Option<Value>,
) -> Result<Checkpoint, String> {
    let c: Checkpoint = value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if c.outputs.len() as i64 != job.completed_units
        || c.outputs.len() > p.plan.nodes.len()
        || c.outputs.iter().enumerate().any(|(i, o)| {
            o.index != i || o.batches.is_empty() || o.batches.len() > plan::MAX_BATCHES
        })
        || c.pending.as_ref().is_some_and(|o| {
            o.index != c.outputs.len()
                || o.batches.is_empty()
                || o.batches.len() > plan::MAX_BATCHES
                || !matches!(p.plan.nodes.get(o.index), Some(plan::Node::Merge { .. }))
        })
        || (c.outputs.len() == p.plan.nodes.len()) != c.markdown_hash.is_some()
        || c.outputs.len() == p.plan.nodes.len() && c.pending.is_some()
    {
        return Err("議事録Markdownの保存位置が計画と一致しません".into());
    }
    Ok(c)
}
fn validate_capacity(p: &Parameters) -> Result<(), String> {
    if p.schema_version != 7 {
        return Err("議事録の固定モデル上限の形式が現行と一致しません".into());
    }
    p.capacity.plan_limit(&p.engine, p.prompt_budget)?;
    Ok(())
}
fn plan_limit(p: &Parameters) -> Result<usize, String> {
    p.capacity.plan_limit(&p.engine, p.prompt_budget)
}
fn task_system(system: &str) -> String {
    format!("{system}{}", super::model_budget::COMPLETION_INSTRUCTION)
}
fn extract_task(input: &MinutesInput, slices: &[plan::Slice]) -> Result<Task, String> {
    Ok(Task {
        system: task_system(markdown::system()),
        prompt: plan::extract_prompt(input, slices)?,
        schema: markdown::schema(),
        local: None,
    })
}
fn merge_task(cursor: &plan::Cursor) -> Result<Task, String> {
    Ok(Task {
        system: task_system(plan::MERGE_SYSTEM),
        prompt: cursor.task()?,
        schema: markdown::schema(),
        local: None,
    })
}
fn cursor(node: &plan::Node, outputs: &[String], limit: usize) -> Result<plan::Cursor, String> {
    let plan::Node::Merge { left, right } = node else {
        return Err("統合計画ではありません".into());
    };
    if *left >= outputs.len() || *right >= outputs.len() || left == right {
        return Err("統合の入力位置が不正です".into());
    }
    plan::Cursor::new(vec![outputs[*left].clone(), outputs[*right].clone()], limit)
}
fn replay(
    storage: &Storage,
    meeting: &str,
    input: &MinutesInput,
    p: &Parameters,
    c: &Checkpoint,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<(Vec<String>, Option<plan::Cursor>)>, String> {
    replay_with(input, p, c, cancelled, |task, artifacts| {
        super::verify_task_artifacts(storage, meeting, task, artifacts)
    })
}
fn replay_with(
    input: &MinutesInput,
    p: &Parameters,
    c: &Checkpoint,
    cancelled: &impl Fn() -> bool,
    mut verify: impl FnMut(&Task, &[Artifact]) -> Result<Value, String>,
) -> Result<Option<(Vec<String>, Option<plan::Cursor>)>, String> {
    let mut outputs = vec![];
    for saved in &c.outputs {
        if cancelled() {
            return Ok(None);
        }
        let text = match &p.plan.nodes[saved.index] {
            plan::Node::Extract { slices } => {
                if saved.batches.len() != 1 {
                    return Err("抽出の保存形式が不正です".into());
                }
                markdown::text(verify(&extract_task(input, slices)?, &saved.batches[0])?)?
            }
            node => {
                let mut current = cursor(node, &outputs, plan_limit(p)?)?;
                for batch in &saved.batches {
                    if cancelled() {
                        return Ok(None);
                    }
                    let text = markdown::text(verify(&merge_task(&current)?, batch)?)?;
                    current.accept(text)?;
                }
                current
                    .result()
                    .ok_or("統合が完了していません")?
                    .to_string()
            }
        };
        outputs.push(text);
    }
    if c.markdown_hash.as_deref()
        != outputs
            .last()
            .filter(|_| outputs.len() == p.plan.nodes.len())
            .map(|t| content_hash(t))
            .as_deref()
    {
        return Err("議事録Markdownの生成結果ハッシュが一致しません".into());
    }
    let pending = if let Some(saved) = &c.pending {
        let mut current = cursor(&p.plan.nodes[saved.index], &outputs, plan_limit(p)?)?;
        for batch in &saved.batches {
            if cancelled() {
                return Ok(None);
            }
            let text = markdown::text(verify(&merge_task(&current)?, batch)?)?;
            current.accept(text)?;
        }
        if current.result().is_some() {
            return Err("完了した統合が途中保存されています".into());
        }
        Some(current)
    } else {
        None
    };
    Ok(Some((outputs, pending)))
}
pub(crate) fn retry_preflight(
    storage: &Storage,
    job: &JobSnapshot,
    spec: &JobSpec,
    value: Option<&Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let input = storage.load_minutes_input(&job.id)?;
    let p = checked_parameters(storage, &job.id, spec, &input)?;
    let c = checked_checkpoint(job, &p, value.cloned())?;
    let runtime = managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    replay(storage, &spec.meeting_id, &input, &p, &c, cancelled)?
        .ok_or("議事録の再試行検査が中断されました")?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    if storage.load_minutes_input(&job.id)? != input || cancelled() {
        return Err("議事録の再試行検査中に固定入力が変わったか、中断されました".into());
    }
    Ok(())
}
fn infer(
    storage: &Storage,
    spec: &JobSpec,
    job_id: &str,
    index: usize,
    task: Task,
    p: &Parameters,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<(String, Vec<Artifact>)>, String> {
    let input_limit = p.capacity.input_limit(&p.engine, p.prompt_budget)?;
    if !plan::fits(&task.system, &task.prompt, input_limit) {
        return Err("議事録の入力が固定上限を超えています".into());
    }
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
    let runtime = managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?;
    let execution = p.capacity.execution(&runtime, &task.system, &task.prompt)?;
    let Some(raw) = super::llama::run_unit(
        &execution,
        &directory,
        &task.system,
        &task.prompt,
        &task.schema,
        Duration::from_secs(1800),
        cancelled,
    )?
    else {
        return Ok(None);
    };
    if cancelled() {
        return Ok(None);
    }
    let text = markdown::text(raw)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    let mut artifacts = vec![];
    for name in ["system.txt", "prompt.txt", "schema.json", "result.json"] {
        let asset = storage.register_analysis_artifact(
            &spec.meeting_id,
            &directory.join(name),
            super::artifact_type(name)?,
        )?;
        artifacts.push(Artifact {
            name: name.into(),
            id: asset.id,
            hash: asset.content_hash,
        });
    }
    // Re-read the registered bytes before accepting a durable unit.
    if markdown::text(super::verify_task_artifacts(
        storage,
        &spec.meeting_id,
        &task,
        &artifacts,
    )?)? != text
    {
        return Err("生成結果が登録中に変更されています".into());
    }
    Ok(Some((text, artifacts)))
}
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, value) = storage.fixed_job_input(job_id)?;
    let input = storage.load_minutes_input(job_id)?;
    let p = checked_parameters(storage, job_id, &spec, &input)?;
    let mut checkpoint = checked_checkpoint(&storage.get_job(job_id)?, &p, value)?;
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |j| j.state != "running")
    };
    let attempt = (|| {
        let runtime =
            managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?;
        runtime.verify(&cancelled)?;
        managed::verify(
            &spec.profile_snapshot,
            Execution::Llama(&p.engine, p.prompt_budget),
            &cancelled,
        )?;
        let Some((mut outputs, mut resumed)) = replay(
            storage,
            &spec.meeting_id,
            &input,
            &p,
            &checkpoint,
            &cancelled,
        )?
        else {
            return Ok(None);
        };
        for index in checkpoint.outputs.len()..p.plan.nodes.len() {
            if cancelled() {
                return Ok(None);
            }
            let (text, batches) = match &p.plan.nodes[index] {
                plan::Node::Extract { slices } => {
                    let Some((text, artifacts)) = infer(
                        storage,
                        &spec,
                        job_id,
                        index,
                        extract_task(&input, slices)?,
                        &p,
                        &cancelled,
                    )?
                    else {
                        return Ok(None);
                    };
                    (text, vec![artifacts])
                }
                node => {
                    let mut current = match resumed.take() {
                        Some(c) => c,
                        None => cursor(node, &outputs, plan_limit(&p)?)?,
                    };
                    if checkpoint.pending.is_none() {
                        checkpoint.pending = Some(Output {
                            index,
                            batches: vec![],
                        });
                    }
                    loop {
                        if cancelled() {
                            return Ok(None);
                        }
                        let Some((text, artifacts)) = infer(
                            storage,
                            &spec,
                            job_id,
                            index,
                            merge_task(&current)?,
                            &p,
                            &cancelled,
                        )?
                        else {
                            return Ok(None);
                        };
                        current.accept(text)?;
                        let pending = checkpoint
                            .pending
                            .as_mut()
                            .ok_or("統合の保存位置がありません")?;
                        pending.batches.push(artifacts);
                        if pending.batches.len() > plan::MAX_BATCHES {
                            return Err("議事録統合の単位数上限です".into());
                        }
                        if let Some(text) = current.result() {
                            break (text.to_string(), checkpoint.pending.take().unwrap().batches);
                        }
                        let job = storage.save_job_checkpoint(
                            job_id,
                            index as i64,
                            index as i64,
                            serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
                        )?;
                        let paused = job.state == "paused";
                        progress(job);
                        if paused {
                            return Ok(None);
                        }
                    }
                }
            };
            outputs.push(text);
            checkpoint.outputs.push(Output { index, batches });
            if outputs.len() == p.plan.nodes.len() {
                checkpoint.markdown_hash = Some(content_hash(outputs.last().unwrap()));
            }
            let job = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            )?;
            let paused = job.state == "paused";
            progress(job);
            if paused {
                return Ok(None);
            }
        }
        if cancelled() {
            return Ok(None);
        }
        runtime.verify(&cancelled)?;
        managed::verify(
            &spec.profile_snapshot,
            Execution::Llama(&p.engine, p.prompt_budget),
            &cancelled,
        )?;
        if storage.load_minutes_input(job_id)? != input {
            return Err("議事録の固定入力が変更されています".into());
        }
        let document = storage.finish_minutes_markdown(
            job_id,
            outputs.len() as i64,
            outputs.pop().ok_or("生成結果がありません")?,
        )?;
        progress(storage.get_job(job_id)?);
        Ok(Some(document.meeting_id))
    })();
    if let Ok(Some(_)) = &attempt {
        return attempt;
    }
    if cancelled() {
        let job = storage.get_job(job_id)?;
        if job.state == "running" && stopping() {
            storage.request_job_pause(job_id, "exit")?;
        }
        if storage.get_job(job_id)?.state == "pausing" {
            // Read back the last durable checkpoint, never the uncommitted
            // in-memory unit that failed or was cancelled during registration.
            let job = storage.get_job(job_id)?;
            let (_, saved) = storage.fixed_job_input(job_id)?;
            progress(storage.save_job_checkpoint(
                job_id,
                job.completed_units,
                job.completed_units,
                saved.unwrap_or(
                    serde_json::to_value(Checkpoint::default()).map_err(|e| e.to_string())?,
                ),
            )?);
        }
        return Ok(None);
    }
    attempt
}
