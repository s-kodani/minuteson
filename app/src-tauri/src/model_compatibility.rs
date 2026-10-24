//! Explicit local smoke trials. Not publisher authentication, quality scoring,
//! profile application, or permission to silently run a package on startup.
use crate::model_inspection::{Engine, Role};
use crate::model_placement::PlacementRecord;
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

mod probe;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckState {
    Preparing,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}
impl CheckState {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CheckSummary {
    pub operation_id: String,
    pub package_id: String,
    pub review_hash: String,
    pub adapter_revision: u32,
    pub state: CheckState,
    pub error_message: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Artifact {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ProbeReport {
    pub schema_version: u32,
    pub operation_id: String,
    pub package_id: String,
    pub review_hash: String,
    pub engine: Engine,
    pub declared_engine_version: String,
    pub adapter_revision: u32,
    pub settings: serde_json::Value,
    pub artifacts: Vec<Artifact>,
    pub elapsed_ms: u64,
}
pub(crate) fn canonical_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn settings(engine: Engine) -> serde_json::Value {
    use serde_json::json;
    match engine {
        Engine::Whisper => {
            json!({"protocol":"whisper-cli-json-v1","threads":1,"useGpu":false,"language":"ja","translate":false,"beamSize":5,"input":"synthetic-silence-16k-mono-pcm16-10s","prompt":""})
        }
        Engine::Sherpa => {
            json!({"protocol":"sherpa-offline-diarization-v1","threads":1,"provider":"cpu","numSpeakers":null,"clusterThresholdMilli":500,"windowShiftMilli":500,"minDurationOnMs":300,"minDurationOffMs":500,"input":"synthetic-silence-16k-mono-pcm16-10s"})
        }
        Engine::Llama => {
            json!({"protocol":"llama-completion-json-v1","threads":1,"contextTokens":2048,"predictionTokens":64,"gpuLayers":0,"seed":42,"input":"synthetic-json-ok-true-v1"})
        }
    }
}
pub(crate) fn artifact_names(engine: Engine) -> &'static [&'static str] {
    match engine {
        Engine::Whisper => &["input.wav", "result.json", "stderr.log", "stdout.log"],
        Engine::Sherpa => &["input.wav", "stderr.log", "stdout.log"],
        Engine::Llama => &[
            "prompt.txt",
            "result.json",
            "schema.json",
            "stderr.log",
            "system.txt",
        ],
    }
}
impl ProbeReport {
    pub(crate) fn validate(&self, record: &PlacementRecord) -> Result<(), String> {
        record.validate()?;
        self.validate_portable(record)
    }
    pub(crate) fn validate_portable(&self, record: &PlacementRecord) -> Result<(), String> {
        record.validate_portable()?;
        if self.schema_version != 1
            || self.adapter_revision != 1
            || !canonical_id(&self.operation_id)
            || self.package_id != record.package_id
            || self.review_hash != record.review_hash
            || self.engine != record.manifest.engine
            || self.declared_engine_version != record.manifest.engine_version
            || self.settings != settings(self.engine)
            || self.artifacts.len() != artifact_names(self.engine).len()
            || self.elapsed_ms > i64::MAX as u64
        {
            return Err("モデル互換性の試行条件・対象が一致しません".into());
        }
        for (artifact, name) in self.artifacts.iter().zip(artifact_names(self.engine)) {
            if artifact.name != *name || artifact.size > 16 * 1024 * 1024 || !hash(&artifact.sha256)
            {
                return Err("モデル互換性の入力・出力・ログ証拠が不正です".into());
            }
        }
        Ok(())
    }
    pub(crate) fn encoded(&self, record: &PlacementRecord) -> Result<(String, String), String> {
        self.validate(record)?;
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 65536 {
            return Err("互換性の記録が上限を超えています".into());
        }
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        Ok((json, hash))
    }
}

// Fixed jobs recheck their own package, not today's profile/check selection.
// No trial is run and no database/model record is changed.
pub(crate) fn verify_fixed_package(
    models_root: &Path,
    record: &PlacementRecord,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    record.validate()?;

    {
        probe::verify_package(models_root, record, cancelled).map(|_| ())
    }
}

pub(crate) fn check(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    package: &str,
    expected_review: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<CheckSummary, String> {
    {
        probe::check(
            storage,
            models_root,
            operation,
            package,
            expected_review,
            std::time::Duration::from_secs(120),
            cancelled,
        )
    }
}

/// Read-only evidence verification. Unlike `check`, a missing operation can
/// never fall through to starting an executable or creating trial intent.
pub(crate) fn verify_success(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    package: &str,
    expected_review: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(PlacementRecord, std::path::PathBuf), String> {
    {
        probe::verify_success(
            storage,
            models_root,
            operation,
            package,
            expected_review,
            cancelled,
        )
    }
}

#[cfg(all(test, unix))]
mod tests;
