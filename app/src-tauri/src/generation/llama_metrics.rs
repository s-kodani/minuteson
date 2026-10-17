//! Local timing metadata only. No prompts or generated text are copied here.
use super::llama::LlamaConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Phase {
    Verifying,
    Starting,
    Input,
    Generating,
    Saving,
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Evaluation {
    pub milliseconds: Option<f64>,
    pub tokens: Option<u64>,
    pub runs: Option<u64>,
    pub tokens_per_second: Option<f64>,
}
fn evaluation(line: &str) -> Option<Evaluation> {
    let (_, rest) = line.split_once('=')?;
    let (ms, rest) = rest.split_once(" ms /")?;
    let mut count = rest.split_whitespace();
    let total = count.next()?.parse().ok()?;
    let unit = count.next()?;
    if !matches!(unit, "tokens" | "runs") {
        return None;
    }
    let rate = rest
        .split_once("tokens per second")?
        .0
        .rsplit(',')
        .next()?
        .trim()
        .parse()
        .ok()?;
    let ms: f64 = ms.trim().parse().ok()?;
    let rate: f64 = rate;
    if !ms.is_finite() || ms < 0.0 || !rate.is_finite() || rate < 0.0 {
        return None;
    }
    Some(Evaluation {
        milliseconds: Some(ms),
        tokens: (unit == "tokens").then_some(total),
        runs: (unit == "runs").then_some(total),
        tokens_per_second: Some(rate),
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Metrics<'a> {
    schema_version: u32,
    engine_sha256: &'a str,
    model_sha256: &'a str,
    threads: u32,
    gpu_layers: u32,
    context_tokens: u32,
    prediction_tokens: u32,
    input_bytes: usize,
    total_ms: u128,
    verification_ms: u128,
    spawn_ms: u128,
    first_output_ms: Option<u128>,
    validation_ms: u128,
    native_load_ms: Option<f64>,
    engine_initialization_ms: Option<f64>,
    prompt_evaluation: Evaluation,
    generation: Evaluation,
    metal_device: Option<String>,
    offloaded_layers: Option<u32>,
    peak_memory_bytes: Option<u64>,
    swap_delta_bytes: Option<i64>,
}
pub(super) fn metal_device(log: &str) -> Option<String> {
    let mut selected = None;
    for line in log.lines() {
        let Some((_, rest)) = line.split_once("using device ") else {
            continue;
        };
        let id = rest.split_whitespace().next()?;
        let suffix = id
            .strip_prefix("MTL")
            .or_else(|| id.strip_prefix("Metal"))?;
        if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if selected.as_deref().is_some_and(|prior| prior != id) {
            return None;
        }
        selected = Some(id.to_owned());
    }
    selected
}
fn metal_offload(log: &str) -> Option<(u32, u32)> {
    let mut found = None;
    for line in log.lines() {
        let Some((_, rest)) = line.split_once("offloaded ") else {
            continue;
        };
        let (counts, suffix) = rest.split_once(" layers to GPU")?;
        if !suffix.trim().is_empty() {
            return None;
        }
        let (actual, total) = counts.split_once('/')?;
        let (actual, total) = (actual.parse::<u32>().ok()?, total.parse::<u32>().ok()?);
        if actual == 0 || actual > total {
            return None;
        }
        if found.is_some_and(|prior| prior != (actual, total)) {
            return None;
        }
        found = Some((actual, total));
    }
    found
}
pub(super) fn metal_evidence(log: &str) -> Option<(String, u32)> {
    Some((metal_device(log)?, metal_offload(log)?.0))
}
pub(super) fn metal_matches(log: &str, device: &str, requested_layers: u32) -> bool {
    metal_device(log).as_deref() == Some(device)
        && metal_offload(log).is_some_and(|(actual, total)| actual == requested_layers.min(total))
}
impl<'a> Metrics<'a> {
    pub(super) fn new(
        config: &'a LlamaConfig,
        log: &str,
        input_bytes: usize,
        total: Duration,
        verification: Duration,
        spawn: Duration,
        first_output: Option<Duration>,
        validation: Duration,
    ) -> Self {
        let mut prompt_evaluation = Evaluation::default();
        let mut generation = Evaluation::default();
        let mut load_ms = None;
        let mut engine_initialization_ms = None;
        for line in log.lines() {
            if engine_initialization_ms.is_none() && line.contains("generate: n_ctx =") {
                // b11533 logs elapsed minutes.seconds.milliseconds.microseconds.
                let clock: Option<Vec<f64>> = line
                    .split_whitespace()
                    .next()
                    .map(|v| {
                        v.split('.')
                            .map(str::parse::<f64>)
                            .collect::<Result<Vec<_>, _>>()
                            .ok()
                    })
                    .flatten();
                if let Some(parts) =
                    clock.filter(|v| v.len() == 4 && v.iter().all(|n| n.is_finite() && *n >= 0.0))
                {
                    engine_initialization_ms = Some(
                        parts[0] * 60_000.0 + parts[1] * 1000.0 + parts[2] + parts[3] / 1000.0,
                    );
                }
            }
            if line.contains("prompt eval time") {
                if let Some(v) = evaluation(line) {
                    prompt_evaluation = v;
                }
            } else if line.contains("eval time") {
                if let Some(v) = evaluation(line) {
                    generation = v;
                }
            } else if let Some((_, value)) = line.split_once("load time =") {
                load_ms = value
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0);
            }
        }
        let metal = metal_evidence(log);
        Self {
            schema_version: 1,
            engine_sha256: &config.executable.sha256,
            model_sha256: &config.model.sha256,
            threads: config.threads,
            gpu_layers: config.gpu_layers,
            context_tokens: config.context_tokens,
            prediction_tokens: config.prediction_tokens,
            input_bytes,
            total_ms: total.as_millis(),
            verification_ms: verification.as_millis(),
            spawn_ms: spawn.as_millis(),
            first_output_ms: first_output.map(|v| v.as_millis()),
            validation_ms: validation.as_millis(),
            native_load_ms: load_ms,
            engine_initialization_ms,
            prompt_evaluation,
            generation,
            metal_device: metal.as_ref().map(|v| v.0.clone()),
            offloaded_layers: metal.map(|v| v.1),
            peak_memory_bytes: None,
            swap_delta_bytes: None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_only_real_measurements_and_positive_offload() {
        let v = evaluation("llama_perf_context_print: prompt eval time = 738120.00 ms / 14850 tokens (49.70 ms per token, 20.12 tokens per second)").unwrap();
        assert_eq!(v.tokens, Some(14850));
        assert_eq!(v.tokens_per_second, Some(20.12));
        assert!(
            evaluation("eval time = nan ms / 2 runs (0 ms per token, nan tokens per second)")
                .is_none()
        );
        assert!(metal_evidence("found device Metal0\n offloaded 0/33 layers to GPU").is_none());
        assert_eq!(metal_evidence("llama_model_load_from_file_impl: using device Metal0 (Apple M1 Pro) - 200 MiB free\nload_tensors: offloaded 17/33 layers to GPU"), Some(("Metal0".into(),17)));
    }
}
