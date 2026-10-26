//! A synthetic inference freezes the Windows backend before the first source
//! unit. A resumed GPU job never changes to CPU or another physical adapter.
use super::whisper::{self, WhisperConfig};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path, time::Duration};
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "backend", rename_all = "camelCase")]
pub(super) enum Resolved {
    Cpu {
        reason: String,
    },
    Vulkan {
        adapter: crate::gpu::Adapter,
        engine_sha256: String,
        model_sha256: String,
        log: String,
        result: String,
    },
}
impl Resolved {
    pub(super) fn configure(&self, engine: &mut WhisperConfig) -> Result<(), String> {
        match self {
            Self::Cpu { reason } => {
                if !["no_device", "trial_failed"].contains(&reason.as_str()) {
                    return Err("文字起こしCPUの選択根拠が不正です".into());
                }
                engine.use_gpu = false;
            }
            Self::Vulkan {
                adapter,
                engine_sha256,
                model_sha256,
                log,
                result,
            } => {
                if !adapter.validate()
                    || *engine_sha256 != engine.executable.sha256
                    || *model_sha256 != engine.model.sha256
                    || log.len() > 65536
                    || result.len() > 65536
                    || !log.contains("using Vulkan0 backend")
                    || !log.contains(&adapter.name)
                {
                    return Err("文字起こしVulkanの試行証拠が不正です".into());
                }
                whisper::parse_output(result.as_bytes(), &engine.model.path, 2_000_000)?;
                if !adapter.available() {
                    return Err("保存したVulkan GPUを利用できません。完了済み区間は保持しています。別の設定で新しい文字起こしを開始してください。（transcription_gpu_unavailable）".into());
                }
                engine.use_gpu = true;
            }
        }
        Ok(())
    }
}
pub(super) fn resolve(
    engine: &WhisperConfig,
    directory: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Resolved>, String> {
    if cancelled() {
        return Ok(None);
    }
    let adapters = crate::gpu::adapters();
    if adapters.is_empty() {
        return Ok(Some(Resolved::Cpu {
            reason: "no_device".into(),
        }));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("input.wav"))
        .map_err(|e| e.to_string())?;
    let frames = 32000u32;
    let mut bytes = Vec::with_capacity(64044);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(frames * 2 + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&32000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(frames * 2).to_le_bytes());
    for frame in 0..frames {
        let sample =
            ((frame as f64 * 440.0 * std::f64::consts::TAU / 16000.0).sin() * 4096.0) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let trial: Result<Option<Resolved>, String> = (|| {
        let mut config = engine.clone();
        config.use_gpu = true;
        config.vad_model = None;
        config.vad_options = None;
        config.prompt.clear();
        let Some(_) = whisper::run_unit(
            &config,
            directory,
            2_000_000,
            Duration::from_secs(60),
            cancelled,
        )?
        else {
            return Ok(None);
        };
        let log =
            std::fs::read_to_string(directory.join("stderr.log")).map_err(|e| e.to_string())?;
        let result =
            std::fs::read_to_string(directory.join("result.json")).map_err(|e| e.to_string())?;
        let mut matching = adapters
            .into_iter()
            .filter(|adapter| log.contains(&adapter.name));
        let adapter = matching
            .next()
            .ok_or("Vulkanの実デバイス証拠がありません")?;
        if matching.next().is_some() {
            return Err("Vulkanの実デバイスが曖昧です".into());
        }
        let resolved = Resolved::Vulkan {
            adapter,
            engine_sha256: engine.executable.sha256.clone(),
            model_sha256: engine.model.sha256.clone(),
            log,
            result,
        };
        resolved.configure(&mut config)?;
        Ok(Some(resolved))
    })();
    let probe=directory.join("input.wav");
    crate::local_engine::verification::release(&probe)?;
    std::fs::remove_file(&probe).map_err(|e|e.to_string())?;
    if cancelled() {
        return Ok(None);
    }
    match trial {
        Ok(value) => Ok(value),
        Err(_) => {
            crate::local_engine::resident::discard_trial();
            Ok(Some(Resolved::Cpu {
                reason: "trial_failed".into(),
            }))
        }
    }
}
