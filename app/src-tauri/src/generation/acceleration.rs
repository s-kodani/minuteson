//! A separate Metal trial, pinned before the first generated unit. The CPU
//! package-compatibility receipt is never treated as evidence of GPU execution.
use super::llama::{self, LlamaConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::Path, time::Duration};

pub(super) const SYSTEM: &str = "Return only the exact JSON object required by the schema.";
pub(super) const PROMPT: &str = "Return {\"ok\":true}.";
const DEVICE: &str = "MTL0";
const AUTO_LAYERS: &[u32] = &[256, 32, 16, 8, 1];
const LOG_LIMIT: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Policy {
    AutoMetalV2,
}
impl Policy {
    pub(super) fn accepts(self, engine: &LlamaConfig) -> bool {
        match self {
            Self::AutoMetalV2 => engine.gpu_layers == 0 && engine.validate().is_ok(),
        }
    }
    fn proof_version(self) -> u32 {
        match self {
            Self::AutoMetalV2 => 2,
        }
    }
}
pub(super) fn policy(engine: &LlamaConfig) -> Option<Policy> {
    if cfg!(target_os = "macos") && Policy::AutoMetalV2.accepts(engine) {
        Some(Policy::AutoMetalV2)
    } else {
        None
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CpuReason {
    Memory,
    TrialFailed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Proof {
    schema_version: u32,
    engine_sha256: String,
    model_sha256: String,
    context_tokens: u32,
    gpu_layers: u32,
    device: String,
    // Synthetic input only. These immutable bytes also travel with backups.
    result: String,
    log: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "backend", rename_all = "camelCase")]
pub(super) enum Resolved {
    Cpu { reason: CpuReason },
    Metal { proof: Proof },
}
impl Resolved {
    pub(super) fn validate(
        &self,
        policy: Policy,
        base: &LlamaConfig,
        context: u32,
    ) -> Result<(), String> {
        if !policy.accepts(base) {
            return Err("GPU試行の固定モデルが一致しません".into());
        }
        if let Self::Metal { proof } = self {
            if proof.schema_version != policy.proof_version()
                || proof.engine_sha256 != base.executable.sha256
                || proof.model_sha256 != base.model.sha256
                || proof.context_tokens != context
                || !(1..=256).contains(&proof.gpu_layers)
                || proof.device != DEVICE
                || proof.log.len() > LOG_LIMIT
                || proof.result.len() > 4096
                || super::llama_metrics::metal_evidence(&proof.log)
                    != Some((DEVICE.into(), proof.gpu_layers))
                || llama::parse_output(proof.result.as_bytes())? != json!({"ok":true})
            {
                return Err("GPUの試行結果・実デバイス・オフロード証拠が不正です".into());
            }
        }
        Ok(())
    }
    pub(super) fn ensure_available(&self, policy: Policy) -> Result<(), String> {
        if matches!(self, Self::Metal { .. }) && (!cfg!(target_os = "macos")) {
            return Err("保存したGPU設定に必要なメモリ環境を確認できません。完了済み工程は保持しています。このMacで新しく生成するとCPU構成を選び直します。（minutes_gpu_unavailable）".into());
        }
        Ok(())
    }
    pub(super) fn configure(&self, config: &mut LlamaConfig) -> Option<&str> {
        match self {
            Self::Metal { proof } => {
                config.gpu_layers = proof.gpu_layers;
                Some(&proof.device)
            }
            Self::Cpu { .. } => None,
        }
    }
}
fn memory_sufficient() -> bool {
    #[cfg(target_os = "macos")]
    {
        let mut bytes = 0_u64;
        let mut size = std::mem::size_of_val(&bytes);
        unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) == 0
                && size == 8
                && bytes >= 32 * 1024 * 1024 * 1024
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
pub(super) fn resolve(
    policy: Policy,
    base: &LlamaConfig,
    context: u32,
    directory: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Resolved>, String> {
    resolve_with_memory(
        policy,
        base,
        context,
        directory,
        cancelled,
        memory_sufficient(),
    )
}
fn resolve_with_memory(
    policy: Policy,
    base: &LlamaConfig,
    context: u32,
    directory: &Path,
    cancelled: &impl Fn() -> bool,
    memory_available: bool,
) -> Result<Option<Resolved>, String> {
    if cancelled() {
        return Ok(None);
    }
    if !policy.accepts(base) {
        return Err("未検証モデルのGPU自動設定は使用できません".into());
    }
    let candidates = match policy {
        Policy::AutoMetalV2 => AUTO_LAYERS,
    };
    for (index, layers) in candidates.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        let directory = if index == 0 {
            directory.to_path_buf()
        } else {
            let path = directory.join(format!("gpu-trial-{index}"));
            std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            path
        };
        let mut config = base.clone();
        config.context_tokens = context;
        config.prediction_tokens = 64;
        config.gpu_layers = *layers;
        let trial: Result<Option<Resolved>, String> = (|| {
            let schema = json!({"type":"object","properties":{"ok":{"type":"boolean","const":true}},"required":["ok"],"additionalProperties":false});
            let Some(value) = llama::run_unit_observed_on_device(
                &config,
                &directory,
                SYSTEM,
                PROMPT,
                &schema,
                Duration::from_secs(60),
                cancelled,
                &|_| {},
                Some(DEVICE),
            )?
            else {
                return Ok(None);
            };
            if value != json!({"ok":true}) {
                return Err("GPU試行のJSONが一致しません".into());
            }
            let log =
                std::fs::read_to_string(directory.join("stderr.log")).map_err(|e| e.to_string())?;
            let result = std::fs::read_to_string(directory.join("result.json"))
                .map_err(|e| e.to_string())?;
            let (_, actual_layers) =
                super::llama_metrics::metal_evidence(&log).ok_or("GPU実行の証拠がありません")?;
            let choice = Resolved::Metal {
                proof: Proof {
                    schema_version: policy.proof_version(),
                    engine_sha256: base.executable.sha256.clone(),
                    model_sha256: base.model.sha256.clone(),
                    context_tokens: context,
                    gpu_layers: actual_layers,
                    device: DEVICE.into(),
                    result,
                    log,
                },
            };
            choice.validate(policy, base, context)?;
            Ok(Some(choice))
        })();
        if cancelled() {
            return Ok(None);
        }
        match trial {
            Ok(Some(choice)) => return Ok(Some(choice)),
            Ok(None) => return Ok(None),
            Err(_) => {
                crate::local_engine::resident::discard_trial();
                // A CPU-only build or missing device cannot benefit from fewer
                // layers. Retry only when this runtime actually selected Metal.
                let log = std::fs::read_to_string(directory.join("stderr.log")).unwrap_or_default();
                if super::llama_metrics::metal_device(&log).as_deref() != Some(DEVICE) {
                    break;
                }
            }
        }
    }
    Ok(Some(Resolved::Cpu {
        reason: CpuReason::TrialFailed,
    }))
}

#[cfg(test)]
#[path = "acceleration_tests.rs"]
mod tests;
