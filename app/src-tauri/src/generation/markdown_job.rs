//! Markdown-only generation. No candidate adoption or structured evidence.
use super::{markdown, markdown_plan as plan, model_budget::Capacity, Artifact, Task};
use crate::storage::minutes_markdown::{content_hash, GenerationTarget, MarkdownStamp};
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::{JobKind, JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

pub(super) mod restore;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    schema_version: u32,
    #[serde(default, skip_serializing_if = "markdown::PromptVersion::is_legacy")]
    prompt_version: markdown::PromptVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acceleration_policy: Option<super::acceleration::Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_snapshot: Option<PromptSnapshot>,
    engine: super::llama::LlamaConfig,
    capacity: Capacity,
    prompt_budget: usize,
    plan: plan::Plan,
    markdown_target: GenerationTarget,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PromptSnapshot {
    settings_lock_version: i64,
    preferences: crate::storage::minutes_prompt_preferences::MinutesPromptPreferences,
}
impl Parameters {
    fn extract_system(&self) -> &str {
        self.prompt_snapshot.as_ref().map_or_else(
            || self.prompt_version.extract_system(),
            |s| s.preferences.extract_prompt.as_str(),
        )
    }
    fn merge_system(&self) -> &str {
        self.prompt_snapshot.as_ref().map_or_else(
            || self.prompt_version.merge_system(),
            |s| s.preferences.merge_prompt.as_str(),
        )
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acceleration: Option<super::acceleration::Resolved>,
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
    let acceleration = super::acceleration::policy(&engine);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        markdown::PromptVersion::CustomPromptsV5,
        acceleration,
    )
}

pub(crate) fn enqueue_prepared(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
    selection: managed::Snapshot,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::reviewed(&engine.model)?;
    let acceleration = super::acceleration::policy(&engine);
    enqueue_with_selection(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        markdown::PromptVersion::CustomPromptsV5,
        acceleration,
        Some(selection),
    )
}

#[cfg(test)]
pub(crate) fn enqueue_prepared_synthetic(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    prompt_budget: usize,
    selection: managed::Snapshot,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_selection(
        storage,
        request,
        engine,
        capacity,
        "standard",
        prompt_budget,
        None,
        markdown::PromptVersion::Legacy,
        None,
        Some(selection),
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
        markdown::PromptVersion::Legacy,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_model_maximum(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic_model_maximum(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        markdown::PromptVersion::Legacy,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_model_adaptive(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic_model_adaptive(&engine, 40960, 32768);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        "standard",
        2400,
        expected,
        markdown::PromptVersion::Legacy,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_inline(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        "standard",
        2400,
        expected,
        markdown::PromptVersion::InlineEvidenceV2,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_compact(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        "standard",
        4000,
        expected,
        markdown::PromptVersion::CompactEvidenceV3,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_concise(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        "standard",
        5400,
        expected,
        markdown::PromptVersion::ConciseSummaryV4,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_custom(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    expected: Option<MarkdownStamp>,
) -> Result<JobSnapshot, String> {
    let capacity = Capacity::synthetic(&engine, 32768, 8192);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        "standard",
        5400,
        expected,
        markdown::PromptVersion::CustomPromptsV5,
        None,
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
    prompt_version: markdown::PromptVersion,
    new_acceleration_policy: Option<super::acceleration::Policy>,
) -> Result<JobSnapshot, String> {
    enqueue_with_selection(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        prompt_version,
        new_acceleration_policy,
        None,
    )
}

fn enqueue_with_selection(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: super::llama::LlamaConfig,
    capacity: Capacity,
    name: &str,
    prompt_budget: usize,
    expected: Option<MarkdownStamp>,
    prompt_version: markdown::PromptVersion,
    new_acceleration_policy: Option<super::acceleration::Policy>,
    prepared: Option<managed::Snapshot>,
) -> Result<JobSnapshot, String> {
    if !matches!(name, "standard" | "high_accuracy") || prompt_budget > super::budget(&engine)? {
        return Err("議事録プロファイルまたは入力上限が不正です".into());
    }
    capacity.plan_limit(&engine, prompt_budget)?;
    let selected = if let Some(snapshot) = prepared {
        snapshot.validate(name, &Execution::Llama(&engine, prompt_budget))?;
        Some(snapshot)
    } else {
        engine.verify(&|| false)?;
        storage.job_model_selection(
            &request.meeting_id,
            &request.operation_id,
            name,
            Execution::Llama(&engine, prompt_budget),
        )?
    };
    let profile = managed::attach(super::profile(&engine, name), selected)?;
    let target = GenerationTarget {
        schema_version: 1,
        expected,
    };
    // Lost admission responses may be replayed after an app upgrade. Preserve
    // that operation's original policy, including absence on older V3 jobs.
    let (acceleration_policy, prompt_version) = if let Some(existing) = storage
        .list_jobs(&request.meeting_id)?
        .iter()
        .find(|j| j.operation_id == request.operation_id)
    {
        let (spec, _) = storage.fixed_job_input(&existing.id)?;
        let saved = &spec.parameters["generation"];
        (
            serde_json::from_value::<Option<super::acceleration::Policy>>(
                saved["accelerationPolicy"].clone(),
            )
            .map_err(|e| e.to_string())?,
            saved
                .get("promptVersion")
                .cloned()
                .map(serde_json::from_value::<markdown::PromptVersion>)
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default(),
        )
    } else {
        (
            if prompt_version.is_compact() {
                new_acceleration_policy
            } else {
                None
            },
            prompt_version,
        )
    };
    storage.enqueue_markdown_minutes_job_with_prompts(request, profile, target, |input, db, saved| {
        let prompt_snapshot = if prompt_version == markdown::PromptVersion::CustomPromptsV5 {
            Some(if let Some(saved) = saved {
                serde_json::from_value::<PromptSnapshot>(saved["promptSnapshot"].clone()).map_err(|e| e.to_string())?
            } else {
                let settings = crate::storage::minutes_prompt_preferences::load(db)?;
                PromptSnapshot { settings_lock_version: settings.lock_version, preferences: settings.payload }
            })
        } else { None };
        let extract_system = prompt_snapshot.as_ref().map_or_else(|| prompt_version.extract_system(), |s| s.preferences.extract_prompt.as_str());
        let merge_system = prompt_snapshot.as_ref().map_or_else(|| prompt_version.merge_system(), |s| s.preferences.merge_prompt.as_str());
        let limit = plan_limit_for(&capacity, &engine, prompt_budget, prompt_version)?;
        // Check both instructions before enqueueing; never truncate either prompt.
        if !plan::fits(merge_system, "{\"drafts\":[\"a\"]}", limit) {
            return Err("統合用プロンプトが長すぎて入力が収まりません。プロンプトを短くしてください".into());
        }
        let plan = plan::build_with_system(input, limit, prompt_version, extract_system)?;
        let total = plan.nodes.len() as i64;
        let mut parameters = serde_json::json!({"schemaVersion":7,"promptVersion":prompt_version,"engine":engine,"capacity":capacity,"promptBudget":prompt_budget,"plan":plan});
        if let Some(snapshot) = prompt_snapshot { parameters["promptSnapshot"] = serde_json::to_value(snapshot).map_err(|e| e.to_string())?; }
        if let Some(policy) = acceleration_policy { parameters["accelerationPolicy"] = serde_json::to_value(policy).map_err(|e| e.to_string())?; }
        Ok((parameters, Some(total)))
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
        || p.plan
            != plan::build_with_system(
                input,
                plan_limit(&p)?,
                p.prompt_version,
                p.extract_system(),
            )?
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
    if p.acceleration_policy.is_none() && c.acceleration.is_some()
        || p.acceleration_policy.is_some()
            && c.acceleration.is_none()
            && (!c.outputs.is_empty() || c.pending.is_some())
    {
        return Err("GPU選択の保存位置が固定条件と一致しません".into());
    }
    if let Some(choice) = &c.acceleration {
        choice.validate(
            p.acceleration_policy.ok_or("GPU選択の方針がありません")?,
            &p.engine,
            acceleration_context(p)?,
        )?;
    }
    Ok(c)
}
fn acceleration_context(p: &Parameters) -> Result<u32, String> {
    Ok(p.capacity
        .execution(
            &p.engine,
            super::acceleration::SYSTEM,
            super::acceleration::PROMPT,
        )?
        .context_tokens)
}
fn validate_capacity(p: &Parameters) -> Result<(), String> {
    match (&p.prompt_snapshot, p.prompt_version) {
        (Some(snapshot), markdown::PromptVersion::CustomPromptsV5) => {
            if !(0..=9_007_199_254_740_991).contains(&snapshot.settings_lock_version) {
                return Err("議事録プロンプトの固定版が不正です".into());
            }
            snapshot.preferences.validate()?;
        }
        (None, version) if version != markdown::PromptVersion::CustomPromptsV5 => {}
        _ => return Err("議事録プロンプトの固定条件が不正です".into()),
    }
    if p.schema_version != 7
        || p.acceleration_policy
            .is_some_and(|policy| !policy.accepts(&p.engine) || !p.prompt_version.is_compact())
    {
        return Err("議事録の固定モデル上限の形式が現行と一致しません".into());
    }
    p.capacity.plan_limit(&p.engine, p.prompt_budget)?;
    Ok(())
}
fn plan_limit(p: &Parameters) -> Result<usize, String> {
    plan_limit_for(&p.capacity, &p.engine, p.prompt_budget, p.prompt_version)
}
const CUSTOM_COMPLETION: &str =
    "\n出力はmarkdown文字列一つだけのJSONオブジェクトとし、JSONを最後まで完結してください。";
fn plan_limit_for(
    capacity: &Capacity,
    engine: &super::llama::LlamaConfig,
    requested: usize,
    version: markdown::PromptVersion,
) -> Result<usize, String> {
    if version == markdown::PromptVersion::CustomPromptsV5 {
        capacity
            .input_limit(engine, requested)?
            .checked_sub(CUSTOM_COMPLETION.len())
            .filter(|n| *n >= 2048)
            .ok_or_else(|| "議事録プロンプトと入力に十分な容量がありません".into())
    } else {
        capacity.plan_limit(engine, requested)
    }
}
fn task_system(system: &str, version: markdown::PromptVersion) -> String {
    let suffix = if version == markdown::PromptVersion::CustomPromptsV5 {
        CUSTOM_COMPLETION
    } else {
        super::model_budget::COMPLETION_INSTRUCTION
    };
    format!("{system}{suffix}")
}
fn extract_task(
    input: &MinutesInput,
    slices: &[plan::Slice],
    p: &Parameters,
) -> Result<Task, String> {
    Ok(Task {
        system: task_system(p.extract_system(), p.prompt_version),
        prompt: plan::extract_prompt_with_version(input, slices, p.prompt_version)?,
        schema: p.prompt_version.schema(),
        local: None,
    })
}
fn merge_task(cursor: &plan::Cursor) -> Result<Task, String> {
    Ok(Task {
        system: task_system(cursor.system(), cursor.version()),
        prompt: cursor.task()?,
        schema: cursor.version().schema(),
        local: None,
    })
}
fn task_text(_task: &Task, raw: Value) -> Result<String, String> {
    // Content format is unrestricted. Invalid JSON, empty/NUL/oversized text
    // and artifact/CAS failures still stop publication through their checks.
    markdown::text(raw)
}

fn finish_summary(text: &mut String, version: markdown::PromptVersion) {
    if version == markdown::PromptVersion::ConciseSummaryV4 {
        *text = markdown::summary::limit(text);
    }
}

fn cursor(
    node: &plan::Node,
    outputs: &[String],
    limit: usize,
    p: &Parameters,
) -> Result<plan::Cursor, String> {
    let plan::Node::Merge { left, right } = node else {
        return Err("統合計画ではありません".into());
    };
    if *left >= outputs.len() || *right >= outputs.len() || left == right {
        return Err("統合の入力位置が不正です".into());
    }
    plan::Cursor::with_system(
        vec![outputs[*left].clone(), outputs[*right].clone()],
        limit,
        p.prompt_version,
        p.merge_system(),
    )
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
                let task = extract_task(input, slices, p)?;
                task_text(&task, verify(&task, &saved.batches[0])?)?
            }
            node => {
                let mut current = cursor(node, &outputs, plan_limit(p)?, p)?;
                for batch in &saved.batches {
                    if cancelled() {
                        return Ok(None);
                    }
                    let task = merge_task(&current)?;
                    let text = task_text(&task, verify(&task, batch)?)?;
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
    if outputs.len() == p.plan.nodes.len() {
        if let Some(text) = outputs.last_mut() {
            finish_summary(text, p.prompt_version);
        }
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
        let mut current = cursor(&p.plan.nodes[saved.index], &outputs, plan_limit(p)?, p)?;
        for batch in &saved.batches {
            if cancelled() {
                return Ok(None);
            }
            let task = merge_task(&current)?;
            let text = task_text(&task, verify(&task, batch)?)?;
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
    observe: &impl Fn(super::runtime::Progress),
    acceleration: Option<&super::acceleration::Resolved>,
    diagnostic_directory: &mut Option<std::path::PathBuf>,
) -> Result<Option<(String, Vec<Artifact>)>, String> {
    let started = Instant::now();
    let current_job = storage.get_job(job_id)?;
    let observer = super::runtime::Observer::new(
        &current_job,
        index,
        matches!(p.plan.nodes[index], plan::Node::Merge { .. }),
        observe,
    )
    .with_backend(match acceleration {
        Some(super::acceleration::Resolved::Metal { .. }) => Some("metal"),
        Some(super::acceleration::Resolved::Cpu { .. }) => Some("cpu"),
        None if p.engine.gpu_layers == 0 => Some("cpu"),
        _ => None,
    });
    observer.emit(super::llama_metrics::Phase::Verifying);
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
    *diagnostic_directory = Some(directory.clone());
    let runtime = managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?;
    let mut execution = p.capacity.execution(&runtime, &task.system, &task.prompt)?;
    let device = acceleration.and_then(|choice| choice.configure(&mut execution));
    let prepare_ms = started.elapsed().as_millis();
    let inference = Instant::now();
    let Some(raw) = super::llama::run_unit_observed_on_device(
        &execution,
        &directory,
        &task.system,
        &task.prompt,
        &task.schema,
        Duration::from_secs(1800),
        cancelled,
        &|phase| observer.emit(phase),
        device,
    )?
    else {
        return Ok(None);
    };
    if cancelled() {
        return Ok(None);
    }
    let inference_ms = inference.elapsed().as_millis();
    let checking = Instant::now();
    let text = task_text(&task, raw)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    let result_verification_ms = checking.elapsed().as_millis();
    let saving = Instant::now();
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
    if task_text(
        &task,
        super::verify_task_artifacts(storage, &spec.meeting_id, &task, &artifacts)?,
    )? != text
    {
        return Err("生成結果が登録中に変更されています".into());
    }
    let timings = serde_json::json!({"schemaVersion":1,"jobId":job_id,"attempt":current_job.attempt,"specHash":current_job.spec_hash,"unitIndex":index,"preparationMs":prepare_ms,"inferenceMs":inference_ms,"resultVerificationMs":result_verification_ms,"artifactRegistrationMs":saving.elapsed().as_millis(),"totalMs":started.elapsed().as_millis()});
    if let Ok(bytes) = serde_json::to_vec_pretty(&timings) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("unit-performance.json"))
        {
            let _ = file.write_all(&bytes);
        }
    }
    Ok(Some((text, artifacts)))
}
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    process_observed(storage, job_id, stopping, progress, &|_| {})
}
pub(crate) fn process_observed(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
    observe: &impl Fn(super::runtime::Progress),
) -> Result<Option<String>, String> {
    let started = Instant::now();
    let mut preflight_ms = None;
    let mut checkpoint_ms = 0_u128;
    let mut final_verification_ms = None;
    let mut publication_ms = None;
    let mut diagnostic_directory = None;
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
    let current_job = storage.get_job(job_id)?;
    let preflight = super::runtime::Observer::new(
        &current_job,
        checkpoint.outputs.len(),
        matches!(
            p.plan.nodes.get(checkpoint.outputs.len()),
            Some(plan::Node::Merge { .. })
        ),
        observe,
    );
    preflight.emit(super::llama_metrics::Phase::Verifying);
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
        if p.acceleration_policy.is_some() && checkpoint.acceleration.is_none() {
            let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, 0)?;
            diagnostic_directory = Some(directory.clone());
            let Some(choice) = super::acceleration::resolve(
                p.acceleration_policy.ok_or("GPU選択の方針がありません")?,
                &runtime,
                acceleration_context(&p)?,
                &directory,
                &cancelled,
            )?
            else {
                return Ok(None);
            };
            checkpoint.acceleration = Some(choice);
            // Pin CPU or concrete Metal configuration before any extract/merge.
            let job = storage.save_job_checkpoint(
                job_id,
                0,
                0,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            )?;
            let paused = job.state == "paused";
            progress(job);
            if paused {
                return Ok(None);
            }
        }
        if let Some(choice) = &checkpoint.acceleration {
            choice.ensure_available(p.acceleration_policy.ok_or("GPU選択の方針がありません")?)?;
        }
        preflight_ms = Some(started.elapsed().as_millis());
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
                        extract_task(&input, slices, &p)?,
                        &p,
                        &cancelled,
                        observe,
                        checkpoint.acceleration.as_ref(),
                        &mut diagnostic_directory,
                    )?
                    else {
                        return Ok(None);
                    };
                    (text, vec![artifacts])
                }
                node => {
                    let mut current = match resumed.take() {
                        Some(c) => c,
                        None => cursor(node, &outputs, plan_limit(&p)?, &p)?,
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
                            observe,
                            checkpoint.acceleration.as_ref(),
                            &mut diagnostic_directory,
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
                        let checkpoint_started = Instant::now();
                        let job = storage.save_job_checkpoint(
                            job_id,
                            index as i64,
                            index as i64,
                            serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
                        )?;
                        checkpoint_ms += checkpoint_started.elapsed().as_millis();
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
                if let Some(text) = outputs.last_mut() {
                    finish_summary(text, p.prompt_version);
                }
                checkpoint.markdown_hash = Some(content_hash(outputs.last().unwrap()));
            }
            let checkpoint_started = Instant::now();
            let job = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            )?;
            checkpoint_ms += checkpoint_started.elapsed().as_millis();
            let paused = job.state == "paused";
            progress(job);
            if paused {
                return Ok(None);
            }
        }
        if cancelled() {
            return Ok(None);
        }
        let verification_started = Instant::now();
        runtime.verify(&cancelled)?;
        managed::verify(
            &spec.profile_snapshot,
            Execution::Llama(&p.engine, p.prompt_budget),
            &cancelled,
        )?;
        if storage.load_minutes_input(job_id)? != input {
            return Err("議事録の固定入力が変更されています".into());
        }
        final_verification_ms = Some(verification_started.elapsed().as_millis());
        let publication_started = Instant::now();
        let document = storage.finish_minutes_markdown(
            job_id,
            outputs.len() as i64,
            outputs.pop().ok_or("生成結果がありません")?,
        )?;
        publication_ms = Some(publication_started.elapsed().as_millis());
        progress(storage.get_job(job_id)?);
        Ok(Some(document.meeting_id))
    })();
    if let Some(directory) = diagnostic_directory {
        let value = serde_json::json!({"schemaVersion":1,"jobId":job_id,"attempt":current_job.attempt,"specHash":current_job.spec_hash,"completedUnits":checkpoint.outputs.len(),"totalMs":started.elapsed().as_millis(),"preflightMs":preflight_ms,"checkpointMs":checkpoint_ms,"finalVerificationMs":final_verification_ms,"publicationMs":publication_ms,"outcome":match &attempt { Ok(Some(_))=>"succeeded",Ok(None)=>"stopped",Err(_)=>"failed" }});
        if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("attempt-performance-{}.json", uuid::Uuid::new_v4())))
            {
                let _ = file.write_all(&bytes);
            }
        }
    }
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
