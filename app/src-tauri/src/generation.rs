//! Fixed-input llama.cpp extraction and merging with durable unit provenance.
pub(crate) mod llama;
mod merge;
mod plan;
pub(crate) mod result;
use crate::minutes_document::{from_generated, GeneratedItem, TEMPLATE_VERSION};
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
        let input = storage.load_minutes_input(&original.id)?;
        let initial_operation = storage.fixed_job_initial_operation(&original.id)?;
        let p = checked_parameters(&spec, &input, &initial_operation)?;
        let decoded = checked_checkpoint(&original, &p, checkpoint.clone())?;
        p.engine.verify(&|| false)?;
        verify_saved_outputs(storage, &spec, &input, &p, &decoded, &|| false)?
            .ok_or("議事録再試行の検証が中断されました")?;
        p.engine.verify(&|| false)?;
        if storage.load_minutes_input(&original.id)? != input {
            return Err("検証中に議事録の固定入力が変更されました".into());
        }
        storage.enqueue_retry_job(&original, &spec, checkpoint.as_ref(), &self.operation_id)
    }
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
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UnitOutput {
    index: usize,
    artifacts: Vec<Artifact>,
    normalized: Artifact,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Artifact {
    name: String,
    id: String,
    hash: String,
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
    if !matches!(name, "standard" | "high_accuracy") || prompt_budget > budget(&engine)? {
        return Err("議事録プロファイルまたは入力上限が不正です".into());
    }
    engine.verify(&|| false)?;
    storage.enqueue_minutes_job_configured(request, profile(&engine, name), |input| {
        let plan = plan::build(input, prompt_budget)?;
        let total = plan.nodes.len() as i64;
        let parameters = Parameters {
            schema_version: 1,
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
        || p.schema_version != 1
        || p.template_version != TEMPLATE_VERSION
        || envelope.request.meeting_id != spec.meeting_id
        || envelope.request.operation_id != initial_operation
        || !matches!(name, "standard" | "high_accuracy")
        || spec.profile_snapshot != profile(&p.engine, name)
        || p.prompt_budget > budget(&p.engine)?
        || p.plan != plan::build(input, p.prompt_budget)?
        || spec.total_units != Some(p.plan.nodes.len() as i64)
    {
        return Err("議事録の固定入力・モデル・処理計画が不正です".into());
    }
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
    Ok(checkpoint)
}
fn verify_saved_outputs(
    storage: &Storage,
    spec: &JobSpec,
    input: &MinutesInput,
    p: &Parameters,
    checkpoint: &Checkpoint,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<Vec<GeneratedItem>>>, String> {
    let mut outputs = vec![];
    for saved in &checkpoint.outputs {
        if cancelled() {
            return Ok(None);
        }
        let task = task(input, &p.plan.nodes[saved.index], &outputs, p)?;
        let expected = [
            ("system.txt", task.system.as_bytes().to_vec()),
            ("prompt.txt", task.prompt.as_bytes().to_vec()),
            (
                "schema.json",
                serde_json::to_vec(&task.schema).map_err(|e| e.to_string())?,
            ),
        ];
        if saved.artifacts.len() != 4 || saved.normalized.name != "items.json" {
            return Err("議事録の完了結果ファイルが不足しています".into());
        }
        for (artifact, (name, bytes)) in saved.artifacts[..3].iter().zip(expected) {
            if artifact.name != name || read_artifact(storage, &spec.meeting_id, artifact)? != bytes
            {
                return Err("議事録の固定プロンプト・schemaが変更されています".into());
            }
        }
        if saved.artifacts[3].name != "result.json" {
            return Err("議事録の完了結果形式が不正です".into());
        }
        let raw = llama::parse_output(&read_artifact(
            storage,
            &spec.meeting_id,
            &saved.artifacts[3],
        )?)?;
        if task.local.is_some_and(|local| local != raw) {
            return Err("議事録のローカル確定処理が変更されています".into());
        }
        let items = parse_node(raw, input, &p.plan.nodes[saved.index], &outputs)?;
        let normalized: Vec<GeneratedItem> = serde_json::from_slice(&read_artifact(
            storage,
            &spec.meeting_id,
            &saved.normalized,
        )?)
        .map_err(|e| e.to_string())?;
        if normalized != items {
            return Err("議事録の保存項目と生成原文が一致しません".into());
        }
        outputs.push(items);
    }

    Ok(Some(outputs))
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
        p.engine.verify(&cancelled)?;
        let Some(mut outputs) =
            verify_saved_outputs(storage, &spec, &input, &p, &checkpoint, &cancelled)?
        else {
            return Ok(None);
        };
        for index in checkpoint.outputs.len()..p.plan.nodes.len() {
            if cancelled() {
                return Ok(None);
            }
            let task = task(&input, &p.plan.nodes[index], &outputs, &p)?;
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
                let Some(raw) = llama::run_unit(
                    &p.engine,
                    &directory,
                    &task.system,
                    &task.prompt,
                    &task.schema,
                    Duration::from_secs(1800),
                    &cancelled,
                )?
                else {
                    return Ok(None);
                };
                raw
            };
            if cancelled() {
                return Ok(None);
            }
            let items = parse_node(raw, &input, &p.plan.nodes[index], &outputs)?;
            write_new(
                &directory.join("items.json"),
                &serde_json::to_vec(&items).map_err(|e| e.to_string())?,
            )?;
            let mut artifacts = vec![];
            for name in [
                "system.txt",
                "prompt.txt",
                "schema.json",
                "result.json",
                "items.json",
            ] {
                let asset = storage.register_analysis_artifact(
                    &spec.meeting_id,
                    &directory.join(name),
                    artifact_type(name)?,
                )?;
                artifacts.push(Artifact {
                    name: name.into(),
                    id: asset.id,
                    hash: asset.content_hash,
                });
            }
            let normalized = artifacts.pop().unwrap();
            checkpoint.outputs.push(UnitOutput {
                index,
                artifacts,
                normalized,
            });
            let saved = storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            );
            let job = match saved {
                Ok(job) => job,
                Err(error) => {
                    checkpoint.outputs.pop();
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
        p.engine.verify(&cancelled)?;
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
