//! Evaluated optimizations only. Evidence is shipped with the app, never read
//! from user input or a restored job as permission to optimize new work.
use super::llama::LlamaConfig;
use crate::local_engine::{resident, OwnedCommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    pub version: u32,
    pub token_packing: bool,
    pub prefix_reuse: bool,
    pub evidence_hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    model_sha256: String,
    worker_sha256: String,
    prompt_sha256: String,
    prompt_version: super::markdown::PromptVersion,
    token_packing: bool,
    prefix_reuse: bool,
    baseline_ms: [u64; 3],
    optimized_ms: [u64; 3],
    critical_facts_complete: bool,
    no_unsupported_claims: bool,
    links_valid: bool,
    coverage_not_worse: bool,
    prefix_tokens_equal: bool,
    cpu_qualified: bool,
    metal_qualified: bool,
}
fn median(mut a: [u64; 3]) -> u64 {
    a.sort_unstable();
    a[1]
}
pub(super) fn select(
    engine: &LlamaConfig,
    runtime: Option<&resident::Snapshot>,
    extract: &str,
    merge: &str,
    prompt_version: super::markdown::PromptVersion,
) -> Result<Option<Policy>, String> {
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    // Required model-specific counting is a correctness policy, independent
    // of measured speedups. Never enable prefix reuse under this policy.
    if super::model_budget::Capacity::reviewed(&engine.model).is_ok_and(|c| c.requires_tokenizer())
    {
        return Ok(Some(Policy::gguf(engine, runtime)));
    }
    let prompt = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(extract, merge)).map_err(|e| e.to_string())?)
    );
    let entries: Vec<Evidence> = serde_json::from_str(include_str!("optimization-evidence.json"))
        .map_err(|e| e.to_string())?;
    for e in entries {
        if e.model_sha256 == engine.model.sha256
            && e.worker_sha256 == runtime.sha256
            && e.prompt_sha256 == prompt
            && e.prompt_version == prompt_version
            && e.accepted()
        {
            return Ok(Some(Policy {
                version: 1,
                token_packing: e.token_packing,
                prefix_reuse: e.prefix_reuse,
                evidence_hash: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&e).map_err(|e| e.to_string())?)
                ),
            }));
        }
    }
    Ok(None)
}
impl Policy {
    fn gguf(engine: &LlamaConfig, runtime: &resident::Snapshot) -> Self {
        Self {
            version: 2,
            token_packing: true,
            prefix_reuse: false,
            evidence_hash: format!(
                "{:x}",
                Sha256::digest(format!(
                    "gguf-tokenizer-v1:{}:{}",
                    engine.model.sha256, runtime.sha256
                ))
            ),
        }
    }
    pub(super) fn validate_model(
        policy: Option<&Self>,
        engine: &LlamaConfig,
        runtime: Option<&resident::Snapshot>,
        capacity: &super::model_budget::Capacity,
    ) -> Result<(), String> {
        if capacity.requires_tokenizer() {
            let runtime = runtime.ok_or("GGUFトークナイザーの実行環境がありません")?;
            if policy != Some(&Self::gguf(engine, runtime)) {
                return Err("GGUFトークナイザーの固定条件が一致しません".into());
            }
        } else if policy.is_some_and(|p| p.version != 1) {
            return Err("未対応の議事録最適化です".into());
        }
        Ok(())
    }
}
impl Evidence {
    fn accepted(&self) -> bool {
        self.cpu_qualified
            && self.metal_qualified
            && self.critical_facts_complete
            && self.no_unsupported_claims
            && self.links_valid
            && self.coverage_not_worse
            && (!self.prefix_reuse || self.prefix_tokens_equal)
            && median(self.baseline_ms) > 0
            && u128::from(median(self.optimized_ms)) * 100
                <= u128::from(median(self.baseline_ms)) * 90
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Count {
    schema_version: u32,
    tokens: u32,
    system_prefix_tokens: u32,
}
struct Counter {
    engine: LlamaConfig,
    directory: tempfile::TempDir,
    cache: BTreeMap<String, u32>,
    error: Option<String>,
    calls: u64,
    hits: u64,
    output_reserve: u32,
    read_only: bool,
    #[cfg(test)]
    synthetic: bool,
}
thread_local! {static COUNTER:RefCell<Option<Counter>>=const{RefCell::new(None)};static PREFIX:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
pub(super) struct Scope;
impl Scope {
    pub(super) fn enter(engine: &LlamaConfig, policy: Option<&Policy>) -> Result<Self, String> {
        if policy.is_some_and(|p| {
            !matches!(p.version, 1 | 2) || p.version == 2 && (!p.token_packing || p.prefix_reuse)
        }) {
            return Err("未対応の議事録最適化です".into());
        }
        if COUNTER.with(|v| v.borrow().is_some()) || PREFIX.with(|p| p.get()) {
            return Err("議事録最適化セッションが重複しています".into());
        }
        PREFIX.with(|p| p.set(policy.is_some_and(|p| p.prefix_reuse)));
        if policy.is_some_and(|p| p.token_packing) {
            COUNTER.with(|c| {
                *c.borrow_mut() = Some(Counter {
                    engine: engine.clone(),
                    directory: tempfile::tempdir().map_err(|e| e.to_string())?,
                    cache: BTreeMap::new(),
                    error: None,
                    calls: 0,
                    hits: 0,
                    output_reserve: engine.prediction_tokens,
                    read_only: false,
                    #[cfg(test)]
                    synthetic: false,
                });
                Ok::<_, String>(())
            })?;
        }
        Ok(Self)
    }
    #[cfg(test)]
    pub(super) fn fixture() -> (Scope, LlamaConfig, Policy) {
        let proof = crate::local_engine::Fingerprint {
            path: "/test/model".into(),
            size: 1,
            sha256: "a".repeat(64),
        };
        let engine = LlamaConfig {
            executable: proof.clone(),
            model: proof,
            threads: 1,
            context_tokens: 32768,
            prediction_tokens: 8192,
            gpu_layers: 0,
            seed: 42,
        };
        let policy = Policy {
            version: 1,
            token_packing: true,
            prefix_reuse: false,
            evidence_hash: "b".repeat(64),
        };
        let scope = Scope::enter(&engine, Some(&policy)).unwrap();
        COUNTER.with(|c| c.borrow_mut().as_mut().unwrap().synthetic = true);
        (scope, engine, policy)
    }
    pub(super) fn replay(
        engine: &LlamaConfig,
        policy: Option<&Policy>,
        counts: &BTreeMap<String, u32>,
    ) -> Result<Option<Self>, String> {
        if !policy.is_some_and(|p| p.token_packing) || packing() {
            return Ok(None);
        }
        if counts.len() > 4096
            || counts
                .keys()
                .any(|k| k.len() != 64 || !k.bytes().all(|b| b.is_ascii_hexdigit()))
            || counts.values().any(|n| *n == 0)
        {
            return Err("保存トークン数の形式が不正です".into());
        }
        let scope = Self::enter(engine, policy)?;
        COUNTER.with(|c| {
            let mut c = c.borrow_mut();
            let c = c.as_mut().unwrap();
            c.cache = counts.clone();
            c.read_only = true;
        });
        Ok(Some(scope))
    }
    pub(super) fn check<T>(&self, value: Result<T, String>) -> Result<T, String> {
        if let Some(error) = COUNTER.with(|c| c.borrow_mut().as_mut().and_then(|c| c.error.take()))
        {
            Err(error)
        } else {
            value
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        PREFIX.with(|p| p.set(false));
        if let Some(c) = COUNTER.with(|c| c.borrow_mut().take()) {
            crate::internal_logs::detail(crate::internal_logs::current_context("minutes_generation"),"stopped","info",Duration::ZERO,&serde_json::json!({"event":"議事録トークン算定終了","tokenizerCalls":c.calls,"tokenizerCacheHits":c.hits}).to_string());
        }
    }
}
pub(super) fn counts() -> BTreeMap<String, u32> {
    COUNTER.with(|c| {
        c.borrow()
            .as_ref()
            .map(|c| c.cache.clone())
            .unwrap_or_default()
    })
}
pub(super) fn kind() -> &'static str {
    if PREFIX.with(|p| p.get()) {
        "llama_prefix"
    } else {
        "llama"
    }
}
pub(super) fn packing() -> bool {
    COUNTER.with(|c| c.borrow().is_some())
}
pub(super) fn count(system: &str, prompt: &str) -> Option<Result<u32, String>> {
    COUNTER.with(|c| {
        let mut c = c.borrow_mut();
        let c = c.as_mut()?;
        Some(c.count(system, prompt))
    })
}
pub(super) fn fits(system: &str, prompt: &str) -> Option<bool> {
    COUNTER.with(|c| {
        let mut c = c.borrow_mut();
        let c = c.as_mut()?;
        if system.len() + prompt.len() > super::llama::MAX_PROMPT_BYTES {
            return Some(false);
        };
        match c.count(system, prompt) {
            Ok(n) => Some(
                n.checked_add(c.output_reserve.saturating_add(512))
                    .is_some_and(|n| n <= c.engine.context_tokens),
            ),
            Err(e) => {
                c.error = Some(e);
                Some(false)
            }
        }
    })
}
impl Counter {
    fn count(&mut self, system: &str, prompt: &str) -> Result<u32, String> {
        let key = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(system, prompt)).map_err(|e| e.to_string())?)
        );
        if let Some(n) = self.cache.get(&key) {
            self.hits += 1;
            return Ok(*n);
        }
        #[cfg(test)]
        if self.synthetic {
            let n = (system.chars().count() + prompt.chars().count() + 20) as u32;
            self.cache.insert(key, n);
            return Ok(n);
        }
        if self.read_only {
            return Err("復元に必要なトークン算定記録がありません".into());
        }
        if self.cache.len() >= 4096 {
            return Err("トークン算定記録の件数上限です".into());
        }
        if system.len() + prompt.len() > super::llama::MAX_PROMPT_BYTES {
            return Err("トークン算定の入力上限です".into());
        }
        let dir = self.directory.path().join(self.calls.to_string());
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        self.calls += 1;
        std::fs::write(dir.join("system.txt"), system).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("prompt.txt"), prompt).map_err(|e| e.to_string())?;
        for name in ["count.json", "count.log"] {
            std::fs::File::create(dir.join(name)).map_err(|e| e.to_string())?;
        }
        self.engine.model.verify(&|| false)?;
        let mut cmd = OwnedCommand::for_purpose(
            &self.engine.executable.path,
            mon_native::process::Purpose::Tokenize,
        )?;
        cmd.command
            .args(["--model"])
            .arg(&self.engine.model.path)
            .arg("--system-prompt-file")
            .arg(dir.join("system.txt"))
            .arg("--file")
            .arg(dir.join("prompt.txt"))
            .args([
                "--jinja",
                "--conversation",
                "--no-escape",
                "--reasoning",
                "off",
                "--offline",
            ])
            .args(super::llama::model_arguments(&self.engine.model));
        let mut run =
            resident::Run::spawn(&mut cmd, "llama_tokenize", &dir, "count.json", "count.log")?;
        let started = Instant::now();
        loop {
            if let Some(status) = run.try_wait()? {
                if !status.success() {
                    return Err("トークン算定に失敗しました".into());
                }
                break;
            }
            if mon_native::process::Purpose::Tokenize
                .policy()
                .expired(started.elapsed(), None)
            {
                return Err("トークン算定が時間上限を超えました".into());
            }
            if dir
                .join("count.json")
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                > mon_native::process::Purpose::Tokenize.policy().stdout_bytes
                || dir
                    .join("count.log")
                    .metadata()
                    .map_err(|e| e.to_string())?
                    .len()
                    > mon_native::process::Purpose::Tokenize.policy().stderr_bytes
            {
                return Err("トークン算定の出力上限です".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(dir.join("count.json"))
            .map_err(|e| e.to_string())?
            .take(mon_native::process::Purpose::Tokenize.policy().stdout_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > mon_native::process::Purpose::Tokenize.policy().stdout_bytes {
            return Err("トークン算定の出力上限です".into());
        }
        let result: Count = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if result.schema_version != 1
            || result.tokens == 0
            || result.system_prefix_tokens > result.tokens
        {
            return Err("トークン算定結果が不正です".into());
        }
        if self.cache.len() < 4096 {
            self.cache.insert(key, result.tokens);
        }
        Ok(result.tokens)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_gguf_counting_binds_model_and_worker_without_prefix_reuse() {
        let (_scope, mut engine, _) = Scope::fixture();
        engine.model.size = 4_977_171_584;
        engine.model.sha256 =
            "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87".into();
        let capacity = super::super::model_budget::Capacity::reviewed(&engine.model).unwrap();
        let mut runtime = resident::Snapshot {
            gpu: None,
            protocol: 1,
            engine: "llama".into(),
            name: "mon-llama-worker".into(),
            bytes: 1,
            sha256: "b".repeat(64),
        };
        let policy = select(
            &engine,
            Some(&runtime),
            "extract",
            "merge",
            super::super::markdown::PromptVersion::CustomPromptsV5,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (policy.version, policy.token_packing, policy.prefix_reuse),
            (2, true, false)
        );
        assert!(Policy::validate_model(Some(&policy), &engine, Some(&runtime), &capacity).is_ok());
        assert!(Policy::validate_model(None, &engine, Some(&runtime), &capacity).is_err());
        let mut invalid = policy.clone();
        invalid.prefix_reuse = true;
        assert!(
            Policy::validate_model(Some(&invalid), &engine, Some(&runtime), &capacity).is_err()
        );
        runtime.sha256 = "c".repeat(64);
        assert!(Policy::validate_model(Some(&policy), &engine, Some(&runtime), &capacity).is_err());
    }
    #[test]
    fn long_context_packing_accepts_inputs_above_the_previous_byte_limit() {
        let (_scope, _, _) = Scope::fixture();
        COUNTER.with(|c| c.borrow_mut().as_mut().unwrap().engine.context_tokens = 262144);
        assert_eq!(fits("system", &"会議の発言。".repeat(5000)), Some(true));
        assert_eq!(fits("system", &"x".repeat(262144)), Some(false));
        assert_eq!(
            fits("system", &"x".repeat(super::super::llama::MAX_PROMPT_BYTES)),
            Some(false)
        );
    }

    #[test]
    fn optimization_evidence_requires_quality_and_ten_percent_median_gain() {
        let mut e = Evidence {
            model_sha256: "a".repeat(64),
            worker_sha256: "b".repeat(64),
            prompt_sha256: "c".repeat(64),
            prompt_version: super::super::markdown::PromptVersion::CustomPromptsV5,
            token_packing: true,
            prefix_reuse: true,
            baseline_ms: [100, 1000, 100],
            optimized_ms: [89, 900, 89],
            critical_facts_complete: true,
            no_unsupported_claims: true,
            links_valid: true,
            coverage_not_worse: true,
            prefix_tokens_equal: true,
            cpu_qualified: true,
            metal_qualified: true,
        };
        assert!(e.accepted());
        e.prefix_tokens_equal = false;
        assert!(!e.accepted());
        e.prefix_tokens_equal = true;
        e.critical_facts_complete = false;
        assert!(!e.accepted());
        e.critical_facts_complete = true;
        e.optimized_ms = [91, 91, 1];
        assert!(!e.accepted());
        assert!(serde_json::from_str::<Vec<Evidence>>(include_str!("optimization-evidence.json")).unwrap().is_empty(),"Adding approved evidence requires reviewing its source measurements and updating this assertion");
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires explicit local pinned llama model and worker; synthetic input only"]
fn real_token_counter_builds_and_replays_complete_plans() {
    use crate::local_engine::Fingerprint;
    let worker = std::path::PathBuf::from(std::env::var("MON_PIPELINE_WORKER").unwrap());
    let model = std::path::PathBuf::from(std::env::var("MON_PIPELINE_MODEL").unwrap());
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let worker = Fingerprint::capture(&worker, &|| false).unwrap();
    let snapshot = resident::Snapshot {
        gpu: None,
        protocol: 1,
        engine: "llama".into(),
        name: "mon-llama-worker".into(),
        bytes: worker.size,
        sha256: worker.sha256.clone(),
    };
    let directory = root_path.join("inference-runtimes").join(&snapshot.sha256);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::copy(&worker.path, directory.join(&snapshot.name)).unwrap();
    let engine = LlamaConfig {
        executable: worker,
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        threads: 2,
        context_tokens: 32768,
        prediction_tokens: 8192,
        gpu_layers: 0,
        seed: 42,
    };
    let policy = Policy {
        version: 1,
        token_packing: true,
        prefix_reuse: false,
        evidence_hash: "a".repeat(64),
    };
    let mut input = crate::storage::current_minutes_fixture::input();
    let u = input.analysis.utterances[0].clone();
    input.analysis.utterances = (0..120)
        .map(|i| {
            let mut u = u.clone();
            u.id = format!("u{i}");
            u.effective_text = format!("点検の確認事項{i}を報告した。");
            u
        })
        .collect();
    let plan;
    let recorded_counts;
    {
        let _files = crate::local_engine::verification::Scope::enter(true).unwrap();
        let _resident = resident::Scope::enter(&root_path, Some(&snapshot)).unwrap();
        let scope = Scope::enter(&engine, Some(&policy)).unwrap();
        plan = scope
            .check(super::markdown_plan::build_with_version(
                &input,
                24000,
                super::markdown::PromptVersion::CustomPromptsV5,
            ))
            .unwrap();
        super::markdown_plan::validate_coverage(&input, &plan).unwrap();
        recorded_counts = counts();
        assert!(!recorded_counts.is_empty());
    }
    let scope = Scope::replay(&engine, Some(&policy), &recorded_counts)
        .unwrap()
        .unwrap();
    assert_eq!(
        scope
            .check(super::markdown_plan::build_with_version(
                &input,
                24000,
                super::markdown::PromptVersion::CustomPromptsV5
            ))
            .unwrap(),
        plan
    );
}
