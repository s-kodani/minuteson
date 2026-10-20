//! Test-only fixture builder for persisted model bindings and restore invariants.
use crate::storage::model_profiles::jobs::rebinding::{self, Binding};
use crate::storage::{retry_source_permitted, JobKind, JobSnapshot, JobSpec, Storage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Preview {
    schema_version: u32,
    source: JobSnapshot,
    checkpoint_hash: String,
    logical_conditions: Value,
    binding: Binding,
    // A content digest for review identity, not an authorization signature.
    review_hash: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub job: Option<JobSnapshot>,
    pub active: bool,
    pub cancelled: bool,
}

fn digest(domain: &[u8], value: &impl Serialize) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(serde_json::to_vec(value).map_err(|e| e.to_string())?);
    Ok(format!("{:x}", hash.finalize()))
}
fn checkpoint_hash(job: &JobSnapshot, cp: Option<&Value>) -> Result<String, String> {
    digest(
        b"mon-model-retry-checkpoint-v1",
        &(&job.spec_hash, job.completed_units, cp),
    )
}
impl Preview {
    fn hash(&self) -> Result<String, String> {
        let mut unsealed = self.clone();
        unsealed.review_hash.clear();
        digest(b"mon-model-job-rebinding-v1", &unsealed)
    }
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.hash()? != self.review_hash {
            return Err("再試行の確認内容・形式版が一致しません".into());
        }
        eligible(&self.source)
    }
    fn input(&self, storage: &Storage) -> Result<(JobSnapshot, JobSpec, Option<Value>), String> {
        self.validate()?;
        let job = storage.get_job(&self.source.id)?;
        let (spec, cp) = storage.fixed_job_input(&job.id)?;
        if job.spec_hash != self.source.spec_hash
            || job.meeting_id != self.source.meeting_id
            || checkpoint_hash(&job, cp.as_ref())? != self.checkpoint_hash
            || rebinding::conditions(&spec)? != self.logical_conditions
        {
            return Err("確認後に再試行元の固定条件・完了結果が変わっています".into());
        }
        Ok((job, spec, cp))
    }
}
fn eligible(job: &JobSnapshot) -> Result<(), String> {
    for id in [&job.id, &job.meeting_id] {
        uuid::Uuid::parse_str(id).map_err(|_| "再試行元のIDが不正です")?;
    }
    if !retry_source_permitted(job, true)
        || !(1..1000).contains(&job.attempt)
        || !matches!(
            job.kind.as_str(),
            "transcription" | "diarization" | "minutes_generation"
        )
        || job.completed_units < 0
        || job
            .total_units
            .is_none_or(|n| n < job.completed_units || n < 1)
    {
        return Err("モデル再照合による再試行が可能なジョブではありません".into());
    }
    Ok(())
}
fn preflight(
    storage: &Storage,
    job: &JobSnapshot,
    spec: &JobSpec,
    cp: Option<&Value>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    match spec.kind {
        JobKind::VoiceRegistration | JobKind::VoiceMatching | JobKind::SpeakerCompletion => {
            Err("声の登録・照合はモデル設定を確認して新しく実行してください".into())
        }
        JobKind::Transcription => {
            crate::transcription::retry_preflight(storage, job, spec, cp, cancelled)
        }
        JobKind::Diarization => {
            crate::diarization::retry_preflight(storage, job, spec, cp, cancelled)
        }
        JobKind::MinutesGeneration => {
            crate::generation::retry_preflight(storage, job, spec, cp, cancelled)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn preview(
    storage: &Storage,
    models_root: &Path,
    meeting: &str,
    job: &str,
    expected_spec_hash: &str,
    package: &str,
    review: &str,
    check: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Preview, String> {
    let source = storage.get_job(job)?;
    eligible(&source)?;
    if source.meeting_id != meeting || source.spec_hash != expected_spec_hash {
        return Err("選択した再試行元の会議・固定条件が一致しません".into());
    }
    let (spec, cp) = storage.fixed_job_input(job)?;
    let logical_conditions = rebinding::conditions(&spec)?;
    let binding = rebinding::prepare(
        storage,
        models_root,
        &spec,
        package,
        review,
        check,
        cancelled,
    )?;
    let mut prospective = spec.clone();
    prospective.profile_snapshot = binding.attach(&spec)?;
    preflight(storage, &source, &prospective, cp.as_ref(), cancelled)?;
    if storage.get_job(job)? != source || storage.fixed_job_input(job)? != (spec, cp.clone()) {
        return Err("確認中に再試行元の条件・状態が変わっています".into());
    }
    if cancelled() {
        return Err("再試行の確認を取り消しました".into());
    }
    let mut result = Preview {
        schema_version: 1,
        checkpoint_hash: checkpoint_hash(&source, cp.as_ref())?,
        logical_conditions,
        source,
        binding,
        review_hash: String::new(),
    };
    result.review_hash = result.hash()?;
    Ok(result)
}

// Lost responses use a read-only receipt. No native model, current trial,
// preparation lease or dispatch is required to identify a committed retry.
pub(crate) fn result(
    storage: &Storage,
    operation: &str,
    preview: &Preview,
) -> Result<Option<JobSnapshot>, String> {
    uuid::Uuid::parse_str(operation).map_err(|_| "再試行の操作IDが不正です")?;
    let (_, spec, cp) = preview.input(storage)?;
    storage.rebound_retry_result(
        &preview.source,
        &spec,
        cp.as_ref(),
        operation,
        &preview.binding,
    )
}

pub(crate) fn receipt(
    storage: &Storage,
    inspector: &crate::model_inspection::Inspector,
    operation: &str,
    preview: &Preview,
) -> Result<Receipt, String> {
    // Lifetime must be observed before the receipt. Otherwise a commit could
    // occur after an absent receipt read but before an inactive status read.
    let (active, cancelled) = inspector.status(operation)?;
    let job = result(storage, operation, preview)?;
    Ok(Receipt {
        job,
        active,
        cancelled,
    })
}

pub(crate) fn apply(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    preview: &Preview,
    cancelled: &impl Fn() -> bool,
) -> Result<JobSnapshot, String> {
    if let Some(job) = result(storage, operation, preview)? {
        return Ok(job);
    }
    let (source, spec, cp) = preview.input(storage)?;
    if source != preview.source {
        return Err("確認後に再試行元の状態が変わっています".into());
    }
    eligible(&source)?;
    preview
        .binding
        .recheck(storage, models_root, &spec, cancelled)?;
    let mut prospective = spec.clone();
    prospective.profile_snapshot = preview.binding.attach(&spec)?;
    preflight(storage, &source, &prospective, cp.as_ref(), cancelled)?;
    preview
        .binding
        .recheck(storage, models_root, &spec, cancelled)?;
    storage.enqueue_rebound_retry_job(
        &source,
        &spec,
        cp.as_ref(),
        operation,
        &preview.binding,
        cancelled,
    )
}
