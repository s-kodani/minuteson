//! Markdown-only generation. No candidate adoption or structured evidence.
use super::{markdown, markdown_plan as plan, model_budget::Capacity, Artifact, Task};
use crate::storage::minutes_markdown::{content_hash, GenerationTarget, MarkdownStamp};
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::{JobKind, JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    optimization_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    optimization: Option<super::optimization::Policy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resident_runtime: Option<crate::local_engine::resident::Snapshot>,
    schema_version: u32,
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
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    token_counts: std::collections::BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acceleration: Option<super::acceleration::Resolved>,
    outputs: Vec<Output>,
    pending: Option<Output>,
    markdown_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    citation_warnings: Option<markdown::citations::Warnings>,
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
            Some(9)
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
    let capacity = Capacity::for_new_job(&engine.model)?;
    let acceleration = super::acceleration::policy(&engine);
    enqueue_with_capacity(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        markdown::PromptVersion::CitationIdsV6,
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
    let capacity = Capacity::for_new_job(&engine.model)?;
    let acceleration = super::acceleration::policy(&engine);
    enqueue_with_selection(
        storage,
        request,
        engine,
        capacity,
        name,
        prompt_budget,
        expected,
        markdown::PromptVersion::CitationIdsV6,
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
        markdown::PromptVersion::CustomPromptsV5,
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
        markdown::PromptVersion::CustomPromptsV5,
        None,
    )
}
#[cfg(test)]
pub(crate) fn enqueue_synthetic_citations(
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
        3400,
        expected,
        markdown::PromptVersion::CitationIdsV6,
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
        markdown::PromptVersion::CustomPromptsV5,
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
        markdown::PromptVersion::CustomPromptsV5,
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
        markdown::PromptVersion::CustomPromptsV5,
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
        markdown::PromptVersion::CustomPromptsV5,
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
    let capacity = Capacity::synthetic(&engine, engine.context_tokens, engine.prediction_tokens);
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
    mut capacity: Capacity,
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
    let acceleration_policy = new_acceleration_policy;
    let new_runtime = crate::local_engine::resident::prepare_job(
        storage,
        &request.meeting_id,
        &request.operation_id,
        "llama",
    )?;
    let existing = storage
        .list_jobs(&request.meeting_id)?
        .iter()
        .any(|j| j.operation_id == request.operation_id);
    let prompt_settings = storage.minutes_prompt_preferences()?;
    let extract = if prompt_settings.lock_version == 0 {
        prompt_version.extract_system()
    } else {
        &prompt_settings.payload.extract_prompt
    };
    let merge = if prompt_settings.lock_version == 0 {
        prompt_version.merge_system()
    } else {
        &prompt_settings.payload.merge_prompt
    };
    let selected_optimization = if existing {
        None
    } else if capacity.is_memory_aware() {
        Some(super::optimization::memory_aware_policy(
            &engine,
            new_runtime.as_ref(),
        )?)
    } else {
        #[cfg(test)]
        {
            super::optimization::select(
                &engine,
                new_runtime.as_ref(),
                &task_system(extract, prompt_version),
                &task_system(merge, prompt_version),
                prompt_version,
            )?
        }
        #[cfg(not(test))]
        return Err("現行の議事録実行計画が必要です".into());
    };
    let prepared_token_plan = if selected_optimization
        .as_ref()
        .is_some_and(|p| p.token_packing)
    {
        let input = storage.preview_minutes_optimization_input(&request.meeting_id)?;
        let _files = crate::local_engine::verification::Scope::enter(true)?;
        let _resident =
            crate::local_engine::resident::Scope::enter(storage.root(), new_runtime.as_ref())?;
        let tokens = super::optimization::Scope::enter(
            &capacity.token_configuration(&engine)?,
            selected_optimization.as_ref(),
        )?;
        if capacity.is_memory_aware() {
            let slices: Vec<_> = input
                .analysis
                .utterances
                .iter()
                .enumerate()
                .filter(|(_, utterance)| !utterance.effective_text.trim().is_empty())
                .map(|(utterance_index, utterance)| plan::Slice {
                    utterance_index,
                    first_byte: 0,
                    end_byte: utterance.effective_text.len(),
                })
                .collect();
            let full_prompt = plan::extract_prompt_with_version(&input, &slices, prompt_version)?;
            let system = task_system(extract, prompt_version);
            let desired = if system.len().saturating_add(full_prompt.len())
                <= super::llama::MAX_PROMPT_BYTES
            {
                let count = super::optimization::count(&system, &full_prompt)
                    .ok_or("議事録の入力トークン数を確認できません")?;
                capacity.desired_context(tokens.check(count)?)?
            } else {
                // Full inputs above the bounded tokenizer request are split by
                // the existing lossless planner; no source text is omitted.
                capacity.maximum_context()
            };
            let merge_count = super::optimization::count(
                &task_system(merge, prompt_version),
                merge_sample(prompt_version),
            )
            .ok_or("統合用プロンプトのトークン数を確認できません")?;
            let minimum = capacity
                .minimum_context(
                    completion_suffix(prompt_version)
                        .len()
                        .max(super::model_budget::COMPLETION_INSTRUCTION.len()),
                )?
                .max(capacity.minimum_input_context(tokens.check(merge_count)?)?);
            let context = super::memory::select_context(
                &engine,
                new_runtime.as_ref(),
                desired.max(minimum),
                minimum,
                capacity.maximum_context(),
            )?;
            capacity = capacity.with_execution_context(context);
            tokens.configure(&capacity.token_configuration(&engine)?)?;
            if !plan::fits(
                &task_system(merge, prompt_version),
                merge_sample(prompt_version),
                plan_limit_for(&capacity, &engine, prompt_budget, prompt_version)?,
            ) {
                return Err(
                    "統合用プロンプトが長すぎて入力が収まりません。プロンプトを短くしてください"
                        .into(),
                );
            }
        }
        let plan = tokens.check(plan::build_with_system(
            &input,
            plan_limit_for(&capacity, &engine, prompt_budget, prompt_version)?,
            prompt_version,
            extract,
        ))?;
        _files.finish(&|| false)?;
        Some((input.working_content_hash, plan))
    } else {
        None
    };
    storage.enqueue_markdown_minutes_job_with_prompts(
        request,
        profile,
        target,
        |input, db, saved| {
            let prompt_version = match saved {
                Some(saved) => serde_json::from_value(saved["promptVersion"].clone())
                    .map_err(|e| e.to_string())?,
                None => prompt_version,
            };
            let capacity = match saved {
                Some(saved) => serde_json::from_value::<Capacity>(saved["capacity"].clone())
                    .map_err(|e| e.to_string())?,
                None => capacity.clone(),
            };
            let prompt_snapshot = if let Some(saved) = saved {
                serde_json::from_value::<PromptSnapshot>(saved["promptSnapshot"].clone())
                    .map_err(|e| e.to_string())?
            } else {
                let settings = crate::storage::minutes_prompt_preferences::load(db)?;
                let preferences = if settings.lock_version == 0 {
                    crate::storage::minutes_prompt_preferences::MinutesPromptPreferences {
                        extract_prompt: prompt_version.extract_system().into(),
                        merge_prompt: prompt_version.merge_system().into(),
                        ..settings.payload
                    }
                } else {
                    settings.payload
                };
                PromptSnapshot {
                    settings_lock_version: settings.lock_version,
                    preferences,
                }
            };
            let extract_system = &prompt_snapshot.preferences.extract_prompt;
            let merge_system = &prompt_snapshot.preferences.merge_prompt;
            let limit = plan_limit_for(&capacity, &engine, prompt_budget, prompt_version)?;
            // Check both instructions before enqueueing; never truncate either prompt.
            if !capacity.is_memory_aware()
                && !plan::fits(merge_system, "{\"drafts\":[\"a\"]}", limit)
            {
                return Err(
                    "統合用プロンプトが長すぎて入力が収まりません。プロンプトを短くしてください"
                        .into(),
                );
            }
            let optimization = match saved {
                Some(saved) => saved
                    .get("optimization")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| e.to_string())?,
                None => selected_optimization.clone(),
            };
            let plan = if optimization
                .as_ref()
                .is_some_and(|policy| policy.token_packing)
            {
                if let Some(saved) = saved {
                    let plan =
                        serde_json::from_value(saved["plan"].clone()).map_err(|e| e.to_string())?;
                    plan::validate_coverage(input, &plan)?;
                    plan
                } else {
                    let (hash, plan) = prepared_token_plan
                        .as_ref()
                        .ok_or("トークン分割の準備がありません")?;
                    if hash != &input.working_content_hash
                        || extract != extract_system
                        || merge != merge_system
                    {
                        return Err("トークン算定中に生成入力・指示文が変更されました".into());
                    }
                    plan.clone()
                }
            } else {
                plan::build_with_system(input, limit, prompt_version, extract_system)?
            };
            let total = plan.nodes.len() as i64;
            let mut parameters = serde_json::json!({
                "schemaVersion": crate::current_formats::MINUTES,
                "promptVersion": prompt_version,
                "engine": engine,
                "capacity": capacity,
                "promptBudget": prompt_budget,
                "plan": plan,
                "optimizationVersion": crate::current_formats::OPTIMIZATION,
                "promptSnapshot": prompt_snapshot,
            });
            let resident_runtime = match saved {
                Some(saved) => saved.get("residentRuntime").cloned(),
                None => new_runtime
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|e| e.to_string())?,
            };
            if let Some(optimization) = optimization {
                parameters["optimization"] =
                    serde_json::to_value(optimization).map_err(|e| e.to_string())?;
            }
            if let Some(runtime) = resident_runtime {
                parameters["residentRuntime"] = runtime;
            }
            if let Some(policy) = acceleration_policy {
                parameters["accelerationPolicy"] =
                    serde_json::to_value(policy).map_err(|e| e.to_string())?;
            }
            Ok((parameters, Some(total)))
        },
    )
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
    if let Some(runtime) = &p.resident_runtime {
        runtime.accepts("llama")?;
    }

    p.engine.validate()?;
    validate_capacity(&p)?;
    p.markdown_target.validate()?;
    let name = spec.profile_snapshot["profile"]
        .as_str()
        .ok_or("議事録プロファイルがありません")?;
    if spec.kind != JobKind::MinutesGeneration
        || envelope.schema_version != 1
        || !(p.schema_version == crate::current_formats::MINUTES)
        || p.markdown_target.schema_version != 1
        || envelope.request.meeting_id != spec.meeting_id
        || envelope.request.operation_id != initial_operation
        || !matches!(name, "standard" | "high_accuracy")
        || p.prompt_budget > super::budget(&p.engine)?
        || if p.optimization.as_ref().is_some_and(|p| p.token_packing) {
            plan::validate_coverage(input, &p.plan).is_err()
        } else {
            p.plan
                != plan::build_with_system(
                    input,
                    plan_limit(&p)?,
                    p.prompt_version,
                    p.extract_system(),
                )?
        }
        || spec.total_units != Some(p.plan.nodes.len() as i64)
        || spec.profile_snapshot
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
    if c.token_counts.len() > 4096
        || c.token_counts
            .keys()
            .any(|k| k.len() != 64 || !k.bytes().all(|b| b.is_ascii_hexdigit()))
        || c.token_counts.values().any(|n| {
            *n == 0
                || *n
                    > if p.capacity.is_memory_aware() {
                        super::optimization::MAX_TOKEN_COUNT
                    } else {
                        131072
                    }
        })
        || (!p.optimization.as_ref().is_some_and(|p| p.token_packing) && !c.token_counts.is_empty())
    {
        return Err("保存トークン数の固定条件が不正です".into());
    }
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
        || (p.prompt_version.uses_citations() && c.markdown_hash.is_some())
            != c.citation_warnings.is_some()
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
    // The context is fixed independently of prompt length. Portable restore
    // validates this proof without opening a tokenizer or a model.
    Ok(p.capacity.token_configuration(&p.engine)?.context_tokens)
}
fn validate_capacity(p: &Parameters) -> Result<(), String> {
    #[cfg(not(test))]
    if !p.capacity.is_memory_aware() {
        return Err("現行の議事録実行計画が必要です".into());
    }
    super::optimization::Policy::validate_model(
        p.optimization.as_ref(),
        &p.engine,
        p.resident_runtime.as_ref(),
        &p.capacity,
    )?;
    if p.resident_runtime.is_none()
        || (p.optimization_version != Some(crate::current_formats::OPTIMIZATION))
        || p.optimization.as_ref().is_some_and(|p| {
            !matches!(p.version, 1 | 2)
                || p.evidence_hash.len() != 64
                || !p.evidence_hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err("議事録最適化の固定条件が不正です".into());
    }

    match &p.prompt_snapshot {
        Some(snapshot) => {
            if !(0..=9_007_199_254_740_991).contains(&snapshot.settings_lock_version) {
                return Err("議事録プロンプトの固定版が不正です".into());
            }
            snapshot.preferences.validate()?;
        }
        _ => return Err("議事録プロンプトの固定条件が不正です".into()),
    }
    if !(p.schema_version == crate::current_formats::MINUTES)
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
const CITATION_COMPLETION: &str = "\n【引用と出力の共通規則】\n出力は {\"markdown\":\"議事録本文\"} のJSON一つです。入力のcitationId、または統合時にdraftsとsourceCitationIdsにあるIDだけを使います。A2の引用は{{A2}}と書き、根拠が複数なら{{A2}}{{A5}}のように並べます。本文の段落・箇条書き項目・表の本文行に根拠を付けてください。時刻やaudioリンクを直接書く指示があっても、この引用形式を使います。";

fn completion_suffix(version: markdown::PromptVersion) -> &'static str {
    match version {
        markdown::PromptVersion::CustomPromptsV5 => CUSTOM_COMPLETION,
        markdown::PromptVersion::CitationIdsV6 => CITATION_COMPLETION,
    }
}

fn merge_sample(version: markdown::PromptVersion) -> &'static str {
    if version.uses_citations() {
        r#"{"drafts":["a"],"sourceCitationIds":["A1"]}"#
    } else {
        r#"{"drafts":["a"],"sourceLinks":[]}"#
    }
}
fn plan_limit_for(
    capacity: &Capacity,
    engine: &super::llama::LlamaConfig,
    requested: usize,
    version: markdown::PromptVersion,
) -> Result<usize, String> {
    if version.uses_citations() && capacity.requires_tokenizer() {
        capacity.plan_limit(engine, requested)
    } else {
        capacity
            .input_limit(engine, requested)?
            .checked_sub(completion_suffix(version).len())
            .filter(|n| *n >= 2048)
            .ok_or_else(|| "議事録プロンプトと入力に十分な容量がありません".into())
    }
}
pub(in crate::generation) fn task_system(system: &str, version: markdown::PromptVersion) -> String {
    let suffix = completion_suffix(version);
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
        citation_ids: p.prompt_version.uses_citations().then(|| {
            slices
                .iter()
                .map(|slice| slice.utterance_index + 1)
                .collect()
        }),
    })
}
fn merge_task(cursor: &plan::Cursor) -> Result<Task, String> {
    Ok(Task {
        system: task_system(cursor.system(), cursor.version()),
        prompt: cursor.task()?,
        schema: cursor.version().schema(),
        citation_ids: cursor
            .version()
            .uses_citations()
            .then(|| cursor.citation_ids())
            .transpose()?,
    })
}
fn task_text(task: &Task, raw: Value) -> Result<(String, bool), String> {
    let text = markdown::text(raw)?;
    Ok(match &task.citation_ids {
        Some(allowed) => markdown::citations::normalize(text, allowed),
        None => (text, false),
    })
}

fn publication(
    version: markdown::PromptVersion,
    input: &MinutesInput,
    text: String,
    invalid: bool,
) -> Result<(String, Option<markdown::citations::Warnings>), String> {
    if version.uses_citations() {
        let (markdown, warnings) =
            markdown::citations::resolve(text, invalid, |id| plan::citation_link(input, id))?;
        Ok((markdown, Some(warnings)))
    } else {
        Ok((text, None))
    }
}

pub(crate) fn notification_warnings(
    spec: &JobSpec,
    checkpoint: &Value,
) -> Result<Vec<String>, String> {
    if spec.parameters["generation"]["promptVersion"] != "citationIdsV6" {
        return Ok(vec![]);
    }
    let warnings: markdown::citations::Warnings =
        serde_json::from_value(checkpoint["citationWarnings"].clone())
            .map_err(|_| "議事録の引用確認結果を読み取れません")?;
    Ok(if warnings.needs_review() {
        vec!["一部の音声リンクは要確認です".into()]
    } else {
        vec![]
    })
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
struct Replayed {
    outputs: Vec<String>,
    pending: Option<plan::Cursor>,
    invalid_citations: bool,
}

fn replay(
    storage: &Storage,
    meeting: &str,
    input: &MinutesInput,
    p: &Parameters,
    c: &Checkpoint,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Replayed>, String> {
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
) -> Result<Option<Replayed>, String> {
    let recorded_tokens = if p.optimization.as_ref().is_some_and(|p| p.token_packing) {
        super::optimization::Scope::replay(
            &p.capacity.token_configuration(&p.engine)?,
            p.optimization.as_ref(),
            &c.token_counts,
        )?
    } else {
        None
    };
    let replayed = (|| {
        let mut outputs = vec![];
        let mut invalid_citations = false;
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
                    let (text, invalid) = task_text(&task, verify(&task, &saved.batches[0])?)?;
                    invalid_citations |= invalid;
                    text
                }
                node => {
                    let mut current = cursor(node, &outputs, plan_limit(p)?, p)?;
                    for batch in &saved.batches {
                        if cancelled() {
                            return Ok(None);
                        }
                        let task = merge_task(&current)?;
                        let (text, invalid) = task_text(&task, verify(&task, batch)?)?;
                        invalid_citations |= invalid;
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
        if let Some(text) = outputs
            .last()
            .filter(|_| outputs.len() == p.plan.nodes.len())
        {
            let (text, warnings) =
                publication(p.prompt_version, input, text.clone(), invalid_citations)?;
            if c.markdown_hash.as_deref() != Some(&content_hash(&text))
                || c.citation_warnings != warnings
            {
                return Err("議事録Markdownの生成結果ハッシュ・引用確認が一致しません".into());
            }
        }
        let pending = if let Some(saved) = &c.pending {
            let mut current = cursor(&p.plan.nodes[saved.index], &outputs, plan_limit(p)?, p)?;
            for batch in &saved.batches {
                if cancelled() {
                    return Ok(None);
                }
                let task = merge_task(&current)?;
                let (text, invalid) = task_text(&task, verify(&task, batch)?)?;
                invalid_citations |= invalid;
                current.accept(text)?;
            }
            if current.result().is_some() {
                return Err("完了した統合が途中保存されています".into());
            }
            Some(current)
        } else {
            None
        };
        Ok(Some(Replayed {
            outputs,
            pending,
            invalid_citations,
        }))
    })();
    match recorded_tokens {
        Some(scope) => scope.check(replayed),
        None => replayed,
    }
}

#[cfg(test)]
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
    let runtime = p.engine.clone();
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
) -> Result<Option<(String, bool, Vec<Artifact>)>, String> {
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
        Some(super::acceleration::Resolved::Vulkan { .. }) => Some("vulkan"),
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
    let actual_tokens = super::optimization::count(&task.system, &task.prompt).transpose()?;
    let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
    *diagnostic_directory = Some(directory.clone());
    let runtime = p.engine.clone();
    let mut execution = p.capacity.execution(&runtime, &task.system, &task.prompt)?;
    let device = acceleration.and_then(|choice| choice.configure(&mut execution));
    crate::internal_logs::detail(
        crate::internal_logs::current_context("minutes_generation"),
        "prepared",
        "info",
        started.elapsed(),
        &serde_json::json!({
            "event":"議事録の実行枠を確定",
            "unitIndex":index,
            "actualTokens":actual_tokens,
            "contextTokens":execution.context_tokens,
            "predictionTokens":execution.prediction_tokens,
            "gpuLayers":execution.gpu_layers,
            "device":device,
        })
        .to_string(),
    );
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
    let (text, invalid) = task_text(&task, raw)?;
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
    let (verified_text, verified_invalid) = task_text(
        &task,
        super::verify_task_artifacts(storage, &spec.meeting_id, &task, &artifacts)?,
    )?;
    if verified_text != text || verified_invalid != invalid {
        return Err("生成結果が登録中に変更されています".into());
    }
    let timings = serde_json::json!({"schemaVersion":1,"jobId":job_id,"attempt":current_job.attempt,"specHash":current_job.spec_hash,"unitIndex":index,"inputTokens":actual_tokens,"optimization":p.optimization,"preparationMs":prepare_ms,"inferenceMs":inference_ms,"resultVerificationMs":result_verification_ms,"artifactRegistrationMs":saving.elapsed().as_millis(),"totalMs":started.elapsed().as_millis()});
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
    Ok(Some((text, invalid, artifacts)))
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
    let optimization = crate::processing_optimization::Scope::enter(p.optimization_version)?;
    let _resident =
        crate::local_engine::resident::Scope::enter(storage.root(), p.resident_runtime.as_ref())?;
    let token_engine = if p.optimization.as_ref().is_some_and(|p| p.token_packing) {
        p.capacity.token_configuration(&p.engine)?
    } else {
        p.engine.clone()
    };
    let tokens = super::optimization::Scope::enter(&token_engine, p.optimization.as_ref())?;
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
        let runtime = p.engine.clone();
        runtime.verify(&cancelled)?;
        managed::verify(
            &spec.profile_snapshot,
            Execution::Llama(&p.engine, p.prompt_budget),
            &cancelled,
        )?;
        let Some(Replayed {
            mut outputs,
            pending: mut resumed,
            mut invalid_citations,
        }) = replay(
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
        // Replay may count with the CPU worker before restoring a frozen
        // Vulkan choice. No generated unit has started in this attempt yet.
        #[cfg(windows)]
        if checkpoint.acceleration.is_some() {
            crate::local_engine::resident::discard_trial();
        }
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
            checkpoint.token_counts = super::optimization::counts();
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
                    let Some((text, invalid, artifacts)) = infer(
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
                    invalid_citations |= invalid;
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
                        let Some((text, invalid, artifacts)) = infer(
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
                        invalid_citations |= invalid;
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
                        checkpoint.token_counts = super::optimization::counts();
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
                let (text, warnings) = publication(
                    p.prompt_version,
                    &input,
                    outputs.last().unwrap().clone(),
                    invalid_citations,
                )?;
                checkpoint.markdown_hash = Some(content_hash(&text));
                checkpoint.citation_warnings = warnings;
            }
            let checkpoint_started = Instant::now();
            checkpoint.token_counts = super::optimization::counts();
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
        tokens.check(Ok(()))?;
        optimization.finish(&cancelled)?;
        if storage.load_minutes_input(job_id)? != input {
            return Err("議事録の固定入力が変更されています".into());
        }
        final_verification_ms = Some(verification_started.elapsed().as_millis());
        let publication_started = Instant::now();
        let completed_units = outputs.len() as i64;
        let (markdown, _) = publication(
            p.prompt_version,
            &input,
            outputs.pop().ok_or("生成結果がありません")?,
            invalid_citations,
        )?;
        let document = storage.finish_minutes_markdown(job_id, completed_units, markdown)?;
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
