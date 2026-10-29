//! Portable model/download wire values. Field declaration order and Option
//! serialization are part of existing recipe/manifest SHA-256 identities.
//! Native filesystem, host and execution validation stays at its boundary.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BackendManifest {
    pub id: String,
    /// Slot -> package-relative path; the primary model and VAD have common roles.
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PackageManifest {
    pub schema_version: u32,
    pub engine: Engine,
    pub engine_version: String,
    pub model_name: String,
    pub model_version: String,
    pub quantization: Option<String>,
    pub license_ref: String,
    pub target: Target,
    pub files: Vec<PackageFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendManifest>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum Engine {
    #[serde(rename = "whisper.cpp")]
    Whisper,
    #[serde(rename = "asr")]
    Asr,
    #[serde(rename = "sherpa-onnx")]
    Sherpa,
    #[serde(rename = "llama.cpp")]
    Llama,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    MacosAarch64,
    WindowsX86_64,
}
impl Target {
    pub fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacosAarch64),
            ("windows", "x86_64") => Ok(Self::WindowsX86_64),
            _ => Err("このOS・CPUはモデル取り込みの対応対象外です".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Executable,
    RuntimeLibrary,
    Notice,
    Asr,
    AsrAsset,
    Vad,
    Segmentation,
    SpeakerEmbedding,
    Llm,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PackageFile {
    pub relative_path: String,
    pub role: Role,
    pub expected_size: Option<u64>,
    pub expected_hash: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Recipe {
    pub schema_version: u32,
    pub manifest: PackageManifest,
    pub sources: Vec<DownloadSource>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DownloadSource {
    pub relative_path: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveMember>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArchiveMember {
    pub format: String,
    pub expected_size: u64,
    pub expected_hash: String,
    pub member_path: String,
}
