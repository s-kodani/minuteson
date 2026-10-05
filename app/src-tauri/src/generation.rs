//! Fixed-input llama.cpp extraction and merging with durable unit provenance.
mod adaptive;
pub(crate) mod llama;
mod merge;
mod plan;
pub(crate) mod restore;
pub(crate) mod result;
use crate::minutes_document::{from_generated, GeneratedItem, TEMPLATE_VERSION};
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::{JobKind, JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StartRequest {
    pub input: MinutesJobRequest,
    pub executable_path: std::path::PathBuf,
    pub model_path: std::path::PathBuf,
    pub threads: u32,
    pub context_tokens: u32,
    pub prediction_tokens: u32,
    pub gpu_layers: u32,
    pub seed: u32,
    pub profile: String,
    pub prompt_budget: usize,
}
impl StartRequest {
    #[allow(dead_code)]
    pub(crate) fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        use crate::local_engine::Fingerprint;
        let engine = llama::LlamaConfig {
            executable: Fingerprint::capture(&self.executable_path, &|| false)?,
            model: Fingerprint::capture(&self.model_path, &|| false)?,
            threads: self.threads,
            context_tokens: self.context_tokens,
            prediction_tokens: self.prediction_tokens,
            gpu_layers: self.gpu_layers,
            seed: self.seed,
        };
        enqueue(
            storage,
            self.input,
            engine,
            &self.profile,
            self.prompt_budget,
        )
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RetryRequest {
    pub meeting_id: String,
    pub job_id: String,
    pub operation_id: String,
    pub expected_spec_hash: String,
}
impl RetryRequest {
    pub(crate) fn enqueue(self, storage: &Storage) -> Result<JobSnapshot, String> {
        let original = storage.get_job(&self.job_id)?;
        if original.meeting_id != self.meeting_id || original.spec_hash != self.expected_spec_hash {
            return Err("確認した議事録再試行の会議・固定条件が一致しません".into());
        }
        if original.kind != "minutes_generation"
            || !matches!(original.state.as_str(), "failed" | "cancelled")
        {
            return Err("失敗・取消済みの議事録生成だけ再試行できます".into());
        }
        // Keep a new attempt within the bounded lineage verification limit.
        if original.attempt >= 1000 {
            return Err("議事録再試行の試行数上限に達しています。新たに生成してください".into());
        }
        let (spec, checkpoint) = storage.fixed_job_input(&original.id)?;
        retry_preflight(storage, &original, &spec, checkpoint.as_ref(), &|| false)?;
        storage.enqueue_retry_job(&original, &spec, checkpoint.as_ref(), &self.operation_id)
    }
}
pub(crate) fn retry_preflight(
    storage: &Storage,
    original: &JobSnapshot,
    spec: &JobSpec,
    checkpoint: Option<&Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let input = storage.load_minutes_input(&original.id)?;
    let initial_operation = storage.fixed_job_initial_operation(&original.id)?;
    let p = checked_parameters(spec, &input, &initial_operation)?;
    let decoded = checked_checkpoint(original, &p, checkpoint.cloned())?;
    let runtime = managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    verify_saved_outputs(storage, spec, &input, &p, &decoded, cancelled)?
        .ok_or("議事録再試行の検証が中断されました")?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    if storage.load_minutes_input(&original.id)? != input {
        return Err("検証中に議事録の固定入力が変更されました".into());
    }
    if cancelled() {
        return Err("議事録再試行の照合を中断しました".into());
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Parameters {
    schema_version: u32,
    template_version: String,
    engine: llama::LlamaConfig,
    prompt_budget: usize,
    plan: plan::Plan,
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
    outputs: Vec<UnitOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<PendingMerge>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PendingMerge {
    index: usize,
    batches: Vec<SavedBatch>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SavedBatch {
    artifacts: Vec<Artifact>,
    normalized: Artifact,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UnitOutput {
    index: usize,
    artifacts: Vec<Artifact>,
    normalized: Artifact,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    merge_batches: Vec<SavedBatch>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Artifact {
    name: String,
    id: String,
    hash: String,
}

pub(crate) fn restore_checkpoint_references(
    job: &JobSnapshot,
    parameters: Value,
    value: Value,
    mut map: impl FnMut(crate::saved_artifact::Role, &str, &str) -> Result<(String, String), String>,
) -> Result<Value, String> {
    use crate::saved_artifact::Role;
    let parameters: Parameters = serde_json::from_value(parameters).map_err(|e| e.to_string())?;
    if !matches!(parameters.schema_version, 1 | 2)
        || parameters.template_version != TEMPLATE_VERSION
    {
        return Err("未対応の生成条件です".into());
    }
    let mut checkpoint = checked_checkpoint(job, &parameters, Some(value))?;
    if job
        .total_units
        .is_some_and(|total| checkpoint.outputs.len() as i64 > total)
    {
        return Err("生成結果が固定した総単位数を超えています".into());
    }
    let artifact_set = |artifacts: &[Artifact]| -> Result<(), String> {
        if artifacts.len() != 4
            || artifacts.iter().map(|a| a.name.as_str()).ne([
                "system.txt",
                "prompt.txt",
                "schema.json",
                "result.json",
            ])
        {
            return Err("生成チェックポイントの成果物が不足・重複・順序不正です".into());
        }
        Ok(())
    };
    let mut reference = |artifact: &mut Artifact, normalized: bool| -> Result<(), String> {
        let role = match (artifact.name.as_str(), normalized) {
            ("system.txt", false) => Role::GenerationSystem,
            ("prompt.txt", false) => Role::GenerationPrompt,
            ("schema.json", false) => Role::GenerationSchema,
            ("result.json", false) => Role::GenerationRaw,
            ("items.json", true) => Role::GenerationItems,
            _ => return Err("生成チェックポイントの成果物種別が不正です".into()),
        };
        (artifact.id, artifact.hash) = map(role, &artifact.id, &artifact.hash)?;
        Ok(())
    };
    for output in &mut checkpoint.outputs {
        let adaptive = parameters.schema_version == 2
            && matches!(
                parameters.plan.nodes.get(output.index),
                Some(plan::Node::Merge { .. })
            );
        if adaptive {
            if !output.artifacts.is_empty() || output.merge_batches.is_empty() {
                return Err("分割統合の保存形式が不正です".into());
            }
        } else {
            if !output.merge_batches.is_empty() {
                return Err("抽出結果に統合単位が混入しています".into());
            }
            artifact_set(&output.artifacts)?;
        }
        for artifact in &mut output.artifacts {
            reference(artifact, false)?;
        }
        reference(&mut output.normalized, true)?;
        for batch in &mut output.merge_batches {
            artifact_set(&batch.artifacts)?;
            for artifact in &mut batch.artifacts {
                reference(artifact, false)?;
            }
            reference(&mut batch.normalized, true)?;
        }
    }
    if let Some(pending) = &mut checkpoint.pending {
        for batch in &mut pending.batches {
            artifact_set(&batch.artifacts)?;
            for artifact in &mut batch.artifacts {
                reference(artifact, false)?;
            }
            reference(&mut batch.normalized, true)?;
        }
    }
    serde_json::to_value(checkpoint).map_err(|e| e.to_string())
}

fn profile(engine: &llama::LlamaConfig, name: &str) -> Value {
    json!({"profile":name,"engine":"llama.cpp","modelSha256":engine.model.sha256,"qualityStatus":"unverified"})
}
fn budget(engine: &llama::LlamaConfig) -> Result<usize, String> {
    engine
        .context_tokens
        .checked_sub(engine.prediction_tokens)
        .and_then(|n| n.checked_sub(512))
        .map(|n| n as usize)
        .filter(|n| *n >= 2048)
        .ok_or("議事録の入力と出力に十分なコンテキストがありません".into())
}
pub(crate) fn enqueue(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
) -> Result<JobSnapshot, String> {
    enqueue_version(storage, request, engine, name, prompt_budget, 2)
}
#[cfg(test)]
pub(crate) fn enqueue_legacy(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
) -> Result<JobSnapshot, String> {
    enqueue_version(storage, request, engine, name, prompt_budget, 1)
}
fn enqueue_version(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    schema_version: u32,
) -> Result<JobSnapshot, String> {
    if !matches!(name, "standard" | "high_accuracy") || prompt_budget > budget(&engine)? {
        return Err("議事録プロファイルまたは入力上限が不正です".into());
    }
    engine.verify(&|| false)?;
    let selected = storage.job_model_selection(
        &request.meeting_id,
        &request.operation_id,
        name,
        Execution::Llama(&engine, prompt_budget),
    )?;
    let profile = managed::attach(profile(&engine, name), selected)?;
    storage.enqueue_minutes_job_configured(request, profile, |input| {
        let plan = plan::build(input, prompt_budget)?;
        let total = plan.nodes.len() as i64;
        let parameters = Parameters {
            schema_version,
            template_version: TEMPLATE_VERSION.into(),
            engine,
            prompt_budget,
            plan,
        };
        Ok((
            serde_json::to_value(parameters).map_err(|e| e.to_string())?,
            Some(total),
        ))
    })
}
fn checked_parameters(
    spec: &JobSpec,
    input: &MinutesInput,
    initial_operation: &str,
) -> Result<Parameters, String> {
    let envelope: Envelope =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    let p = envelope.generation;
    let name = spec.profile_snapshot["profile"]
        .as_str()
        .ok_or("議事録プロファイルがありません")?;
    if spec.kind != JobKind::MinutesGeneration
        || envelope.schema_version != 1
        || !matches!(p.schema_version, 1 | 2)
        || p.template_version != TEMPLATE_VERSION
        || envelope.request.meeting_id != spec.meeting_id
        || envelope.request.operation_id != initial_operation
        || !matches!(name, "standard" | "high_accuracy")
        || managed::rebinding::base_profile(&spec.profile_snapshot)?
            != managed::attach(
                profile(&p.engine, name),
                managed::read(&spec.profile_snapshot)?,
            )?
        || p.prompt_budget > budget(&p.engine)?
        || p.plan != plan::build(input, p.prompt_budget)?
        || spec.total_units != Some(p.plan.nodes.len() as i64)
    {
        return Err("議事録の固定入力・モデル・処理計画が不正です".into());
    }
    managed::validate(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
    )?;
    Ok(p)
}
struct Task {
    system: String,
    prompt: String,
    schema: Value,
    local: Option<Value>,
}
fn task(
    input: &MinutesInput,
    node: &plan::Node,
    outputs: &[Vec<GeneratedItem>],
    p: &Parameters,
) -> Result<Task, String> {
    let task = match node {
        plan::Node::Extract { slices } => {
            let (prompt, scoped, ids) = plan::extract_prompt(input, slices)?;
            let empty = ids.is_empty();
            Task {
                system: plan::EXTRACT_SYSTEM.into(),
                prompt,
                schema: if empty {
                    json!({"type":"object"})
                } else {
                    result::schema(&scoped, &ids)?
                },
                local: empty.then(|| json!({"schemaVersion":1,"items":[]})),
            }
        }
        plan::Node::Merge { left, right } => {
            let sources = merge::sources(*left, *right, outputs)?;
            let local = if sources.len() <= 1 {
                Some(
                    json!({"schemaVersion":1,"items":sources.iter().map(|(id,item)|json!({"body":item.body,"sourceItemIds":[id]})).collect::<Vec<_>>()}),
                )
            } else {
                None
            };
            Task {
                system: merge::SYSTEM.into(),
                prompt: merge::prompt(&sources)?,
                schema: if sources.is_empty() {
                    json!({"type":"object"})
                } else {
                    merge::schema(&sources)
                },
                local,
            }
        }
    };
    if task.system.len() + task.prompt.len() + 256 > budget(&p.engine)? {
        return Err("統合入力が固定コンテキスト上限を超えています。入力を省略せず停止しました。大きいコンテキストで再生成してください".into());
    }
    Ok(task)
}
fn parse_node(
    value: Value,
    input: &MinutesInput,
    node: &plan::Node,
    outputs: &[Vec<GeneratedItem>],
) -> Result<Vec<GeneratedItem>, String> {
    match node {
        plan::Node::Extract { slices } => {
            let (_, scoped, ids) = plan::extract_prompt(input, slices)?;
            result::parse_items(value, &scoped, &ids)
        }
        plan::Node::Merge { left, right } => {
            merge::parse(value, input, &merge::sources(*left, *right, outputs)?)
        }
    }
}
fn artifact_type(name: &str) -> Result<&'static str, String> {
    match name {
        "system.txt" | "prompt.txt" => Ok("text/plain"),
        "schema.json" | "items.json" => Ok("application/json"),
        "result.json" => Ok("application/vnd.mon.llama-completion"),
        _ => Err("未対応の生成結果ファイルです".into()),
    }
}
fn write_new(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn read_artifact(storage: &Storage, meeting: &str, artifact: &Artifact) -> Result<Vec<u8>, String> {
    storage.read_inference_artifact(
        meeting,
        &artifact.id,
        &artifact.hash,
        artifact_type(&artifact.name)?,
        2 * 1024 * 1024,
    )
}
fn verify_task_artifacts(
    storage: &Storage,
    meeting: &str,
    task: &Task,
    artifacts: &[Artifact],
) -> Result<Value, String> {
    verify_task_bytes(task, artifacts, |artifact| {
        read_artifact(storage, meeting, artifact)
    })
}
// The restore codec must use the same prompt/schema/local-result checks as the
// runtime without needing an available engine or a live Storage instance.
fn verify_task_bytes(
    task: &Task,
    artifacts: &[Artifact],
    mut read: impl FnMut(&Artifact) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    let expected = [
        ("system.txt", task.system.as_bytes().to_vec()),
        ("prompt.txt", task.prompt.as_bytes().to_vec()),
        (
            "schema.json",
            serde_json::to_vec(&task.schema).map_err(|e| e.to_string())?,
        ),
    ];
    if artifacts.len() != 4 {
        return Err("議事録の完了結果ファイルが不足しています".into());
    }
    for (artifact, (name, bytes)) in artifacts[..3].iter().zip(expected) {
        if artifact.name != name || read(artifact)? != bytes {
            return Err("議事録の固定プロンプト・schemaが変更されています".into());
        }
    }
    if artifacts[3].name != "result.json" {
        return Err("議事録の完了結果形式が不正です".into());
    }
    let raw = llama::parse_output(&read(&artifacts[3])?)?;
    if task.local.as_ref().is_some_and(|local| local != &raw) {
        return Err("議事録のローカル確定処理が変更されています".into());
    }
    Ok(raw)
}
fn verify_normalized(
    storage: &Storage,
    meeting: &str,
    artifact: &Artifact,
    items: &[GeneratedItem],
) -> Result<(), String> {
    if artifact.name != "items.json" {
        return Err("議事録の保存項目形式が不正です".into());
    }
    let normalized: Vec<GeneratedItem> =
        serde_json::from_slice(&read_artifact(storage, meeting, artifact)?)
            .map_err(|e| e.to_string())?;
    if normalized != items {
        return Err("議事録の保存項目と生成原文が一致しません".into());
    }
    Ok(())
}
fn merge_cursor(
    node: &plan::Node,
    outputs: &[Vec<GeneratedItem>],
    p: &Parameters,
) -> Result<adaptive::Cursor, String> {
    let plan::Node::Merge { left, right } = node else {
        return Err("統合計画ではありません".into());
    };
    adaptive::Cursor::new(
        merge::adaptive_sources(*left, *right, outputs)?,
        p.prompt_budget,
        p.engine.prediction_tokens as usize,
    )
}
fn merge_task(cursor: &adaptive::Cursor) -> Result<Task, String> {
    let sources = cursor.next().ok_or("統合単位が既に完了しています")?;
    Ok(Task {
        system: merge::SYSTEM.into(),
        prompt: merge::prompt(sources)?,
        schema: if sources.is_empty() {
            json!({"type":"object"})
        } else {
            merge::schema(sources)
        },
        local: cursor.local().then(|| adaptive::identity(sources)),
    })
}
fn replay_merge(
    storage: &Storage,
    spec: &JobSpec,
    input: &MinutesInput,
    p: &Parameters,
    node: &plan::Node,
    outputs: &[Vec<GeneratedItem>],
    batches: &[SavedBatch],
    cancelled: &impl Fn() -> bool,
) -> Result<Option<adaptive::Cursor>, String> {
    if batches.len() > 4096 {
        return Err("統合の処理単位数が上限を超えています".into());
    }
    let mut cursor = merge_cursor(node, outputs, p)?;
    for batch in batches {
        if cancelled() {
            return Ok(None);
        }
        let task = merge_task(&cursor)?;
        let raw = verify_task_artifacts(storage, &spec.meeting_id, &task, &batch.artifacts)?;
        let items = merge::parse(raw, input, cursor.next().unwrap())?;
        verify_normalized(storage, &spec.meeting_id, &batch.normalized, &items)?;
        cursor.accept(items)?;
    }
    Ok(Some(cursor))
}
fn checked_checkpoint(
    job: &JobSnapshot,
    p: &Parameters,
    value: Option<Value>,
) -> Result<Checkpoint, String> {
    let checkpoint: Checkpoint = value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if checkpoint.outputs.len() as i64 != job.completed_units
        || checkpoint.outputs.len() > p.plan.nodes.len()
        || checkpoint
            .outputs
            .iter()
            .enumerate()
            .any(|(i, o)| o.index != i)
    {
        return Err("議事録の再開位置が固定計画と一致しません".into());
    }
    if checkpoint.pending.as_ref().is_some_and(|pending| {
        p.schema_version != 2
            || pending.index != checkpoint.outputs.len()
            || pending.batches.len() > 4096
            || !matches!(
                p.plan.nodes.get(pending.index),
                Some(plan::Node::Merge { .. })
            )
    }) {
        return Err("統合の途中保存位置が固定計画と一致しません".into());
    }
    Ok(checkpoint)
}
struct VerifiedOutputs {
    nodes: Vec<Vec<GeneratedItem>>,
    pending: Option<adaptive::Cursor>,
}
fn verify_saved_outputs(
    storage: &Storage,
    spec: &JobSpec,
    input: &MinutesInput,
    p: &Parameters,
    checkpoint: &Checkpoint,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<VerifiedOutputs>, String> {
    let mut outputs = vec![];
    for saved in &checkpoint.outputs {
        if cancelled() {
            return Ok(None);
        }
        let node = &p.plan.nodes[saved.index];
        let items = if p.schema_version == 2 && matches!(node, plan::Node::Merge { .. }) {
            if !saved.artifacts.is_empty() || saved.merge_batches.is_empty() {
                return Err("分割統合の完了結果形式が不正です".into());
            }
            let Some(cursor) = replay_merge(
                storage,
                spec,
                input,
                p,
                node,
                &outputs,
                &saved.merge_batches,
                cancelled,
            )?
            else {
                return Ok(None);
            };
            let items = cursor.finish()?;
            from_generated(input, items.clone())?;
            items
        } else {
            if !saved.merge_batches.is_empty() {
                return Err("従来の生成単位に統合結果が混入しています".into());
            }
            let task = task(input, node, &outputs, p)?;
            parse_node(
                verify_task_artifacts(storage, &spec.meeting_id, &task, &saved.artifacts)?,
                input,
                node,
                &outputs,
            )?
        };
        verify_normalized(storage, &spec.meeting_id, &saved.normalized, &items)?;
        outputs.push(items);
    }
    let pending = if let Some(pending) = &checkpoint.pending {
        let Some(cursor) = replay_merge(
            storage,
            spec,
            input,
            p,
            &p.plan.nodes[pending.index],
            &outputs,
            &pending.batches,
            cancelled,
        )?
        else {
            return Ok(None);
        };
        Some(cursor)
    } else {
        None
    };
    Ok(Some(VerifiedOutputs {
        nodes: outputs,
        pending,
    }))
}
fn run_task(
    storage: &Storage,
    spec: &JobSpec,
    job_id: &str,
    index: usize,
    task: Task,
    p: &Parameters,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<(Value, std::path::PathBuf)>, String> {
    let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
    let raw = if let Some(local) = task.local {
        for (name, bytes) in [
            ("system.txt", task.system.as_bytes().to_vec()),
            ("prompt.txt", task.prompt.as_bytes().to_vec()),
            (
                "schema.json",
                serde_json::to_vec(&task.schema).map_err(|e| e.to_string())?,
            ),
            (
                "result.json",
                serde_json::to_vec(&local).map_err(|e| e.to_string())?,
            ),
        ] {
            write_new(&directory.join(name), &bytes)?;
        }
        local
    } else {
        managed::verify(
            &spec.profile_snapshot,
            Execution::Llama(&p.engine, p.prompt_budget),
            cancelled,
        )?;
        let Some(raw) = llama::run_unit(
            &managed::rebinding::llama(&spec.profile_snapshot, &p.engine, p.prompt_budget)?,
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
        raw
    };
    if cancelled() {
        return Ok(None);
    }
    managed::verify(
        &spec.profile_snapshot,
        Execution::Llama(&p.engine, p.prompt_budget),
        cancelled,
    )?;
    Ok(Some((raw, directory)))
}
fn save_items(
    storage: &Storage,
    meeting: &str,
    directory: &std::path::Path,
    items: &[GeneratedItem],
) -> Result<Artifact, String> {
    write_new(
        &directory.join("items.json"),
        &serde_json::to_vec(items).map_err(|e| e.to_string())?,
    )?;
    let asset = storage.register_analysis_artifact(
        meeting,
        &directory.join("items.json"),
        artifact_type("items.json")?,
    )?;
    Ok(Artifact {
        name: "items.json".into(),
        id: asset.id,
        hash: asset.content_hash,
    })
}
fn save_task_output(
    storage: &Storage,
    meeting: &str,
    directory: &std::path::Path,
    items: &[GeneratedItem],
) -> Result<SavedBatch, String> {
    let normalized = save_items(storage, meeting, directory, items)?;
    let mut artifacts = vec![];
    for name in ["system.txt", "prompt.txt", "schema.json", "result.json"] {
        let asset = storage.register_analysis_artifact(
            meeting,
            &directory.join(name),
            artifact_type(name)?,
        )?;
        artifacts.push(Artifact {
            name: name.into(),
            id: asset.id,
            hash: asset.content_hash,
        });
    }
    Ok(SavedBatch {
        artifacts,
        normalized,
    })
}
#[allow(clippy::too_many_arguments)]
fn run_merge(
    storage: &Storage,
    spec: &JobSpec,
    job_id: &str,
    index: usize,
    input: &MinutesInput,
    p: &Parameters,
    mut cursor: adaptive::Cursor,
    checkpoint: &mut Checkpoint,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<(Vec<GeneratedItem>, Artifact)>, String> {
    if checkpoint.pending.is_none() {
        checkpoint.pending = Some(PendingMerge {
            index,
            batches: vec![],
        });
    }
    while cursor.next().is_some() {
        if cancelled() {
            return Ok(None);
        }
        if checkpoint.pending.as_ref().unwrap().batches.len() >= 4096 {
            return Err("統合の処理単位数が上限を超えています".into());
        }
        let Some((raw, directory)) = run_task(
            storage,
            spec,
            job_id,
            index,
            merge_task(&cursor)?,
            p,
            cancelled,
        )?
        else {
            return Ok(None);
        };
        let items = merge::parse(raw, input, cursor.next().unwrap())?;
        let saved = save_task_output(storage, &spec.meeting_id, &directory, &items)?;
        cursor.accept(items)?;
        checkpoint.pending.as_mut().unwrap().batches.push(saved);
        let job = match storage.save_job_checkpoint(
            job_id,
            index as i64,
            index as i64,
            serde_json::to_value(&*checkpoint).map_err(|e| e.to_string())?,
        ) {
            Ok(job) => job,
            Err(error) => {
                checkpoint.pending.as_mut().unwrap().batches.pop();
                return Err(error);
            }
        };
        let paused = job.state == "paused";
        progress(job);
        if paused {
            return Ok(None);
        }
    }
    if cancelled() {
        return Ok(None);
    }
    let items = cursor.finish()?;
    from_generated(input, items.clone())?;
    let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
    Ok(Some((
        items.clone(),
        save_items(storage, &spec.meeting_id, &directory, &items)?,
    )))
}
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, checkpoint) = storage.fixed_job_input(job_id)?;
    let input = storage.load_minutes_input(job_id)?;
    let p = checked_parameters(&spec, &input, &storage.fixed_job_initial_operation(job_id)?)?;
    let job = storage.get_job(job_id)?;
    let mut checkpoint = checked_checkpoint(&job, &p, checkpoint)?;
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
        let Some(VerifiedOutputs {
            nodes: mut outputs,
            pending: mut resumed_merge,
        }) = verify_saved_outputs(storage, &spec, &input, &p, &checkpoint, &cancelled)?
        else {
            return Ok(None);
        };
        for index in checkpoint.outputs.len()..p.plan.nodes.len() {
            if cancelled() {
                return Ok(None);
            }
            let node = &p.plan.nodes[index];
            let (items, output) =
                if p.schema_version == 2 && matches!(node, plan::Node::Merge { .. }) {
                    let cursor = match resumed_merge.take() {
                        Some(cursor) => cursor,
                        None => merge_cursor(node, &outputs, &p)?,
                    };
                    let Some((items, normalized)) = run_merge(
                        storage,
                        &spec,
                        job_id,
                        index,
                        &input,
                        &p,
                        cursor,
                        &mut checkpoint,
                        &cancelled,
                        progress,
                    )?
                    else {
                        return Ok(None);
                    };
                    let pending = checkpoint
                        .pending
                        .take()
                        .ok_or("統合の保存位置がありません")?;
                    (
                        items,
                        UnitOutput {
                            index,
                            artifacts: vec![],
                            normalized,
                            merge_batches: pending.batches,
                        },
                    )
                } else {
                    let Some((raw, directory)) = run_task(
                        storage,
                        &spec,
                        job_id,
                        index,
                        task(&input, node, &outputs, &p)?,
                        &p,
                        &cancelled,
                    )?
                    else {
                        return Ok(None);
                    };
                    let items = parse_node(raw, &input, node, &outputs)?;
                    let saved = save_task_output(storage, &spec.meeting_id, &directory, &items)?;
                    (
                        items,
                        UnitOutput {
                            index,
                            artifacts: saved.artifacts,
                            normalized: saved.normalized,
                            merge_batches: vec![],
                        },
                    )
                };
            checkpoint.outputs.push(output);
            let saved = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            );
            let job = match saved {
                Ok(job) => job,
                Err(error) => {
                    let output = checkpoint.outputs.pop().unwrap();
                    if !output.merge_batches.is_empty() {
                        checkpoint.pending = Some(PendingMerge {
                            index,
                            batches: output.merge_batches,
                        });
                    }
                    return Err(error);
                }
            };
            outputs.push(items);
            let paused = job.state == "paused";
            progress(job);
            if paused {
                return Ok(None);
            }
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
        let content = from_generated(&input, outputs.pop().ok_or("議事録の最終結果がありません")?)?;
        let candidate =
            storage.finish_minutes_candidate(job_id, checkpoint.outputs.len() as i64, content)?;
        progress(storage.get_job(job_id)?);
        Ok(Some(candidate))
    })();
    if cancelled() {
        let job = storage.get_job(job_id)?;
        if job.state == "running" && stopping() {
            storage.request_job_pause(job_id, "exit")?;
        }
        if storage.get_job(job_id)?.state == "pausing" {
            progress(storage.save_job_checkpoint(
                job_id,
                checkpoint.outputs.len() as i64,
                checkpoint.outputs.len() as i64,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            )?);
        }
        if let Ok(Some(candidate)) = attempt {
            return Ok(Some(candidate));
        }
        return Ok(None);
    }
    attempt
}
