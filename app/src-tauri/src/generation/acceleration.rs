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
    AutoVulkanV1,
}
impl Policy {
    pub(super) fn accepts(self, engine: &LlamaConfig) -> bool {
        match self {
            Self::AutoMetalV2 | Self::AutoVulkanV1 => {
                engine.gpu_layers == 0 && engine.validate().is_ok()
            }
        }
    }
    fn proof_version(self) -> u32 {
        match self {
            Self::AutoMetalV2 => 2,
            Self::AutoVulkanV1 => 3,
        }
    }
    fn device(self) -> &'static str {
        match self {
            Self::AutoMetalV2 => DEVICE,
            Self::AutoVulkanV1 => "Vulkan0",
        }
    }
    fn evidence(self, log: &str) -> Option<(String, u32)> {
        match self {
            Self::AutoMetalV2 => super::llama_metrics::metal_evidence(log),
            Self::AutoVulkanV1 => super::llama_metrics::vulkan_evidence(log),
        }
    }
}
pub(super) fn policy(engine: &LlamaConfig) -> Option<Policy> {
    if cfg!(target_os = "macos") && Policy::AutoMetalV2.accepts(engine) {
        Some(Policy::AutoMetalV2)
    } else if cfg!(windows) && Policy::AutoVulkanV1.accepts(engine) {
        Some(Policy::AutoVulkanV1)
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
    Cpu {
        reason: CpuReason,
    },
    Metal {
        proof: Proof,
    },
    Vulkan {
        proof: Proof,
        adapter: crate::gpu::Adapter,
    },
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
        if matches!(
            (self, policy),
            (Self::Metal { .. }, Policy::AutoVulkanV1) | (Self::Vulkan { .. }, Policy::AutoMetalV2)
        ) {
            return Err("保存GPU方式が実行方針と一致しません".into());
        }
        if let Self::Vulkan { proof, adapter } = self {
            if !adapter.validate()
                || !proof.log.lines().any(|line| {
                    line.contains("using device Vulkan0 (") && line.contains(&adapter.name)
                })
            {
                return Err("Vulkanの実アダプター証拠が不正です".into());
            }
        }
        if let Self::Metal { proof } | Self::Vulkan { proof, .. } = self {
            if proof.schema_version != policy.proof_version()
                || proof.engine_sha256 != base.executable.sha256
                || proof.model_sha256 != base.model.sha256
                || proof.context_tokens != context
                || !(1..=256).contains(&proof.gpu_layers)
                || proof.device != policy.device()
                || proof.log.len() > LOG_LIMIT
                || proof.result.len() > 4096
                || policy.evidence(&proof.log) != Some((policy.device().into(), proof.gpu_layers))
                || llama::parse_output(proof.result.as_bytes())? != json!({"ok":true})
            {
                return Err("GPUの試行結果・実デバイス・オフロード証拠が不正です".into());
            }
        }
        Ok(())
    }
    pub(super) fn ensure_available(&self, policy: Policy) -> Result<(), String> {
        if let Self::Vulkan { adapter, .. } = self {
            if !cfg!(windows) || policy != Policy::AutoVulkanV1 || !adapter.available() {
                return Err("保存したVulkan GPUを利用できません。完了済みの結果は保持しています。別の実行設定を選ぶには新しい生成を開始してください。（minutes_gpu_unavailable）".into());
            }
        }
        if matches!(self, Self::Metal { .. }) && (!cfg!(target_os = "macos")) {
            return Err("保存したGPU設定に必要なメモリ環境を確認できません。完了済み工程は保持しています。このMacで新しく生成するとCPU構成を選び直します。（minutes_gpu_unavailable）".into());
        }
        Ok(())
    }
    pub(super) fn configure(&self, config: &mut LlamaConfig) -> Option<&str> {
        match self {
            Self::Metal { proof } | Self::Vulkan { proof, .. } => {
                config.gpu_layers = proof.gpu_layers;
                Some(&proof.device)
            }
            Self::Cpu { .. } => None,
        }
    }
}
pub(super) fn resolve(
    policy: Policy,
    base: &LlamaConfig,
    context: u32,
    directory: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Resolved>, String> {
    if cancelled() {
        return Ok(None);
    }
    if !policy.accepts(base) {
        return Err("未検証モデルのGPU自動設定は使用できません".into());
    }
    let candidates = match policy {
        Policy::AutoMetalV2 | Policy::AutoVulkanV1 => AUTO_LAYERS,
    };
    // Preflight token counting may have opened the CPU worker. GPU admission
    // starts a separate trial before any durable generated unit exists.
    #[cfg(windows)]
    if policy == Policy::AutoVulkanV1 {
        crate::local_engine::resident::discard_trial();
    }
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
                Some(policy.device()),
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
            let (_, actual_layers) = policy.evidence(&log).ok_or("GPU実行の証拠がありません")?;
            crate::local_engine::resident::confirm_generation_layers(actual_layers);
            let proof = Proof {
                schema_version: policy.proof_version(),
                engine_sha256: base.executable.sha256.clone(),
                model_sha256: base.model.sha256.clone(),
                context_tokens: context,
                gpu_layers: actual_layers,
                device: policy.device().into(),
                result,
                log,
            };
            let choice = match policy {
                Policy::AutoMetalV2 => Resolved::Metal { proof },
                Policy::AutoVulkanV1 => {
                    let mut adapters = crate::gpu::adapters().into_iter().filter(|adapter| {
                        proof.log.lines().any(|line| {
                            line.contains("using device Vulkan0 (") && line.contains(&adapter.name)
                        })
                    });
                    let adapter = adapters
                        .next()
                        .ok_or("Vulkanの実デバイスを確認できません")?;
                    if adapters.next().is_some() {
                        return Err("Vulkanのデバイス識別が曖昧です".into());
                    }
                    Resolved::Vulkan { proof, adapter }
                }
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
            Err(error) => {
                crate::local_engine::resident::discard_trial();
                crate::internal_logs::detail(
                    crate::internal_logs::current_context("minutes_generation"),
                    "gpuTrial", "info", Duration::ZERO,
                    &json!({"event":"GPU試行不採用","gpuLayers":layers,"contextTokens":context,"reason":error}).to_string(),
                );
                // A rejected estimate has not loaded the model or emitted
                // placement evidence. Smaller GPU placements may still fit.
                if error.contains("minutes_memory_insufficient") {
                    continue;
                }
                // A CPU-only build or missing device cannot benefit from fewer
                // layers. Retry only when this runtime actually selected Metal.
                let log = std::fs::read_to_string(directory.join("stderr.log")).unwrap_or_default();
                if policy.evidence(&log).is_none()
                    && !log.contains(&format!("using device {} (", policy.device()))
                {
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
