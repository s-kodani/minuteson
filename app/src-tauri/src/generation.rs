//! Markdown-only generation dispatch and read-only saved-result validation.
pub(crate) mod llama;
pub(crate) mod markdown;
pub(crate) mod markdown_job;
mod markdown_plan;
pub(crate) mod model_budget;
pub(crate) mod restore;
use crate::storage::{JobSnapshot, JobSpec, MinutesInput, MinutesJobRequest, Storage};
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
#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RetryRequest {
    pub meeting_id: String,
    pub job_id: String,
    pub operation_id: String,
    pub expected_spec_hash: String,
}
#[cfg(test)]
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
    map: impl FnMut(crate::saved_artifact::Role, &str, &str) -> Result<(String, String), String>,
) -> Result<Value, String> {
    if parameters["schemaVersion"].as_u64() != Some(7) {
        return Err(REMOVED_GENERATION.into());
    }
    markdown_job::restore::references(job, parameters, value, map)
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
struct Task {
    system: String,
    prompt: String,
    schema: Value,
    local: Option<Value>,
}
fn artifact_type(name: &str) -> Result<&'static str, String> {
    match name {
        "system.txt" | "prompt.txt" => Ok("text/plain"),
        "schema.json" => Ok("application/json"),
        "result.json" => Ok("application/vnd.mon.llama-completion"),
        _ => Err("未対応の生成結果ファイルです".into()),
    }
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
pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, _) = storage.fixed_job_input(job_id)?;
    if !markdown_job::is_markdown(&spec) {
        // Never launch an old structured worker, change its checkpoint, or
        // create another candidate.
        return Err(REMOVED_GENERATION.into());
    }
    markdown_job::process(storage, job_id, stopping, progress)
}
