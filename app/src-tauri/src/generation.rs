//! Markdown-only generation dispatch and read-only saved-result validation.
mod adaptive;
pub(crate) mod llama;
pub(crate) mod markdown;
pub(crate) mod markdown_job;
mod markdown_plan;
mod merge;
pub(crate) mod model_budget;
mod plan;
pub(crate) mod restore;
pub(crate) mod result;
#[cfg(test)]
pub(crate) mod stored_fixture;
use crate::minutes_document::{from_generated, GeneratedItem, TEMPLATE_VERSION};
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::storage::{JobKind, JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const REMOVED_GENERATION: &str = "旧形式の議事録生成は廃止しました。全体設定のモデルで新しく生成してください。（minutes_structured_generation_removed）";

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
    pub(crate) fn enqueue_markdown(
        self,
        storage: &Storage,
        expected: Option<crate::storage::minutes_markdown::MarkdownStamp>,
    ) -> Result<JobSnapshot, String> {
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
        markdown_job::enqueue(
            storage,
            self.input,
            engine,
            &self.profile,
            self.prompt_budget,
            expected,
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
    if markdown_job::is_markdown(spec) {
        return markdown_job::retry_preflight(storage, original, spec, checkpoint, cancelled);
    }
    Err(REMOVED_GENERATION.into())
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
impl Parameters {
    fn adaptive_merge(&self) -> bool {
        matches!(self.schema_version, 2 | 3 | 4)
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
    if matches!(parameters["schemaVersion"].as_u64(), Some(5 | 6 | 7)) {
        return markdown_job::restore::references(job, parameters, value, map);
    }
    use crate::saved_artifact::Role;
    let parameters: Parameters = serde_json::from_value(parameters).map_err(|e| e.to_string())?;
    if !matches!(parameters.schema_version, 1 | 2 | 3 | 4)
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
        let adaptive = parameters.adaptive_merge()
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
        || !matches!(p.schema_version, 1 | 2 | 3 | 4)
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
        || p.plan != plan::build_versioned(input, p.prompt_budget, p.schema_version)?
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
                system: plan::extract_system(p.schema_version)?,
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
    version: u32,
) -> Result<Vec<GeneratedItem>, String> {
    result::normalize_versioned(parse_node_raw(value, input, node, outputs)?, version)
}
fn parse_node_raw(
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
#[cfg(test)]
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
    llama::validate_markdown_bound(&raw, &task.schema)?;
    if task.local.as_ref().is_some_and(|local| local != &raw) {
        return Err("議事録のローカル確定処理が変更されています".into());
    }
    Ok(raw)
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
        !p.adaptive_merge()
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
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, _) = storage.fixed_job_input(job_id)?;
    if !markdown_job::is_markdown(&spec) {
        // Never launch an old structured worker, change its checkpoint, or
        // create another candidate. Saved data remains readable/restorable.
        return Err(REMOVED_GENERATION.into());
    }
    markdown_job::process(storage, job_id, stopping, progress)
}
