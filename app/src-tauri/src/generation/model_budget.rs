//! Reviewed limits belong to exact model bytes, not a name, path or UI profile.
//! Qwen's byte-level BPE needs no more input tokens than UTF-8 bytes. This is
//! an upper bound, not an exact token count; template/framing gets its own
//! reserve. No such assumption is made about an unreviewed tokenizer.
use super::llama::LlamaConfig;
use crate::local_engine::Fingerprint;
use serde::{Deserialize, Serialize};

pub(super) const COMPLETION_INSTRUCTION: &str =
    "\n必要な事項を簡潔にまとめ、文とJSONを最後まで完結してください。";

// Part of the serialized ModelAdaptive policy. A future change to this floor
// must use a new policy variant so existing jobs keep their fixed budgets.
const ADAPTIVE_OUTPUT_RESERVE: u32 = 8192;

// The only current allocation policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Allocation {
    #[default]
    ModelAdaptive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Capacity {
    model_sha256: String,
    max_input_tokens: u32,
    max_context_tokens: u32,
    max_output_tokens: u32,
    template_reserve_tokens: u32,
    tokenizer: Tokenizer,
    allocation: Allocation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tokenizer {
    Utf8ByteBpe,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TokenLimits {
    pub(crate) max_input_tokens: u32,
    pub(crate) max_output_tokens: u32,
    pub(crate) max_context_tokens: u32,
    tokenizer: Tokenizer,
}
impl TokenLimits {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(2048..=1_048_576).contains(&self.max_context_tokens)
            || !(2048..=self.max_context_tokens).contains(&self.max_input_tokens)
            || !(64..=self.max_context_tokens).contains(&self.max_output_tokens)
        {
            return Err("モデル選択情報の入力・出力トークン上限が不正です".into());
        }
        Ok(())
    }
}
impl Capacity {
    pub(crate) fn reviewed(model: &Fingerprint) -> Result<Self, String> {
        // Official model cards and fixed revisions are reviewed per catalog
        // entry. Capacity binds exact model bytes, including quantization.
        let choices: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../model-catalog/model-definitions.json"
        ))
        .map_err(|_| "モデル選択情報を読み取れません")?;
        Self::reviewed_from(model, &choices)
    }
    fn reviewed_from(model: &Fingerprint, choices: &[serde_json::Value]) -> Result<Self, String> {
        let choice = choices.iter().find(|choice| choice["manifest"]["engine"] == "llama.cpp"
            && choice["files"].as_array().is_some_and(|files| files.iter().any(|file|
                file["role"] == "llm" && file["expectedHash"] == model.sha256 && file["expectedSize"] == model.size)))
            .ok_or("選択モデルの生成上限が未確認です。対応済みの議事録モデルを全体設定で選んでください。（minutes_model_limits_unknown）")?;
        let limits: TokenLimits = serde_json::from_value(choice["tokenLimits"].clone())
            .map_err(|_| "選択モデルの入力・出力トークン上限がありません")?;
        limits.validate()?;
        Ok(Self {
            model_sha256: model.sha256.clone(),
            max_input_tokens: limits.max_input_tokens,
            max_context_tokens: limits.max_context_tokens,
            max_output_tokens: limits.max_output_tokens,
            template_reserve_tokens: 512,
            tokenizer: limits.tokenizer,
            allocation: Allocation::ModelAdaptive,
        })
    }
    pub(super) fn validate(&self, engine: &LlamaConfig) -> Result<(), String> {
        engine.validate()?;
        if self.allocation != Allocation::ModelAdaptive {
            return Err("非対応の旧トークン予算です".into());
        }
        if self.model_sha256 != engine.model.sha256
            || !(2048..=1_048_576).contains(&self.max_context_tokens)
            || !(2048..=self.max_context_tokens).contains(&self.max_input_tokens)
            || !(64..=self.max_context_tokens).contains(&self.max_output_tokens)
            || !(256..=2048).contains(&self.template_reserve_tokens)
        {
            return Err("議事録モデルの固定上限・取得条件が一致しません".into());
        }
        Ok(())
    }
    fn context(&self, engine: &LlamaConfig) -> u32 {
        engine.context_tokens.min(self.max_context_tokens)
    }
    fn model_configuration(&self, engine: &LlamaConfig) -> Result<LlamaConfig, String> {
        let mut runtime = engine.clone();
        runtime.context_tokens = self.max_context_tokens;
        runtime.prediction_tokens = self.max_output_tokens;
        // Reject unsupported catalog limits instead of silently shrinking the
        // model's context or output. Keep the selected device/thread settings.
        runtime.validate()?;
        Ok(runtime)
    }
    pub(super) fn token_configuration(&self, engine: &LlamaConfig) -> Result<LlamaConfig, String> {
        self.validate(engine)?;
        let mut c = self.model_configuration(engine)?;
        c.prediction_tokens = self.max_output_tokens.min(ADAPTIVE_OUTPUT_RESERVE);
        Ok(c)
    }
    pub(super) fn input_limit(
        &self,
        engine: &LlamaConfig,
        requested: usize,
    ) -> Result<usize, String> {
        self.validate(engine)?;
        let runtime = self.model_configuration(engine)?;
        let reserved_output = self.max_output_tokens.min(ADAPTIVE_OUTPUT_RESERVE);
        let available = self
            .context(&runtime)
            .checked_sub(reserved_output)
            .and_then(|n| n.checked_sub(self.template_reserve_tokens))
            .ok_or("選択モデルの入力と生成予算を確保できません")? as usize;
        let requested = available;
        let limit = requested.min(available).min(
            self.max_input_tokens
                .saturating_sub(self.template_reserve_tokens) as usize,
        );
        if limit < 2048 + COMPLETION_INSTRUCTION.len() {
            return Err("選択モデルの入力と生成予算を確保できません".into());
        }
        Ok(limit)
    }
    pub(super) fn plan_limit(
        &self,
        engine: &LlamaConfig,
        requested: usize,
    ) -> Result<usize, String> {
        Ok(self.input_limit(engine, requested)? - COMPLETION_INSTRUCTION.len())
    }
    pub(super) fn execution(
        &self,
        engine: &LlamaConfig,
        system: &str,
        prompt: &str,
    ) -> Result<LlamaConfig, String> {
        self.validate(engine)?;
        let input_upper_bound = match super::optimization::count(system, prompt) {
            Some(n) => n?.checked_add(self.template_reserve_tokens),
            None => system
                .len()
                .checked_add(prompt.len())
                .and_then(|n| u32::try_from(n).ok())
                .and_then(|n| n.checked_add(self.template_reserve_tokens)),
        }
        .ok_or("議事録の入力トークン予算が範囲外です")?;
        if input_upper_bound > self.max_input_tokens {
            return Err("選択モデルの入力トークン上限を超えています".into());
        }
        if self.allocation == Allocation::ModelAdaptive {
            let mut runtime = self.model_configuration(engine)?;
            let available = runtime
                .context_tokens
                .checked_sub(input_upper_bound)
                .ok_or("選択モデルのコンテキストに入力が収まりません")?;
            if available < self.max_output_tokens.min(ADAPTIVE_OUTPUT_RESERVE) {
                return Err("議事録の出力枠を確保できません。入力を分割してください".into());
            }
            runtime.prediction_tokens = available.min(self.max_output_tokens);
            runtime.validate()?;
            return Ok(runtime);
        }
        Err("非対応の旧トークン予算です".into())
    }
    #[cfg(test)]
    pub(crate) fn synthetic(engine: &LlamaConfig, context: u32, output: u32) -> Self {
        Self {
            model_sha256: engine.model.sha256.clone(),
            max_input_tokens: context,
            max_context_tokens: context,
            max_output_tokens: output,
            template_reserve_tokens: 512,
            tokenizer: Tokenizer::Utf8ByteBpe,
            allocation: Allocation::ModelAdaptive,
        }
    }
    #[cfg(test)]
    pub(crate) fn synthetic_model_adaptive(
        engine: &LlamaConfig,
        context: u32,
        output: u32,
    ) -> Self {
        Self {
            allocation: Allocation::ModelAdaptive,
            ..Self::synthetic(engine, context, output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_is_bound_to_neutral_model_bytes_even_for_a_platform_only_definition() {
        let model = Fingerprint {
            path: "/not-executed/windows-only.gguf".into(),
            size: 17,
            sha256: "a".repeat(64),
        };
        let definitions = vec![serde_json::json!({
            "id":"windows-only-model","targets":["windows_x86_64"],
            "manifest":{"engine":"llama.cpp"},
            "files":[{"role":"llm","expectedSize":17,"expectedHash":"a".repeat(64)}],
            "tokenLimits":{"maxInputTokens":4096,"maxOutputTokens":128,
                "maxContextTokens":4096,"tokenizer":"utf8_byte_bpe"}
        })];
        let capacity = Capacity::reviewed_from(&model, &definitions).unwrap();
        assert_eq!(capacity.max_context_tokens, 4096);
        assert_eq!(capacity.max_output_tokens, 128);
        let changed = Fingerprint { size: 18, ..model };
        assert!(Capacity::reviewed_from(&changed, &definitions).is_err());
    }
    fn engine() -> LlamaConfig {
        let proof = Fingerprint {
            path: "/synthetic/model.gguf".into(),
            size: 1,
            sha256: "a".repeat(64),
        };
        LlamaConfig {
            executable: proof.clone(),
            model: proof,
            threads: 1,
            context_tokens: 8192,
            prediction_tokens: 2048,
            gpu_layers: 0,
            seed: 1,
        }
    }
    #[test]
    fn different_model_limits_and_inputs_allocate_different_output_without_a_character_cap() {
        let e = engine();
        let small = Capacity::synthetic(&e, 4096, 1024);
        let large = Capacity::synthetic(&e, 32768, 16384);
        let a = small.execution(&e, "system", &"あ".repeat(400)).unwrap();
        let b = large.execution(&e, "system", &"あ".repeat(400)).unwrap();
        assert_eq!((a.context_tokens, a.prediction_tokens), (4096, 1024));
        assert_eq!((b.context_tokens, b.prediction_tokens), (32768, 16384));
        let long = large.execution(&e, "system", &"あ".repeat(7000)).unwrap();
        assert_eq!(long.prediction_tokens, 11250);
        assert!(long.prediction_tokens < b.prediction_tokens);
        assert_eq!(small.input_limit(&e, 5400).unwrap(), 2560);
        assert!(large.execution(&e, "system", &"x".repeat(30000)).is_err());
    }
    #[test]
    fn unknown_or_changed_model_bytes_do_not_inherit_another_models_limits() {
        let mut e = engine();
        assert!(Capacity::reviewed(&e.model)
            .unwrap_err()
            .contains("minutes_model_limits_unknown"));
        let c = Capacity::synthetic(&e, 32768, 8192);
        e.model.sha256 = "b".repeat(64);
        assert!(c.validate(&e).is_err());
        e.model.sha256 = "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e".into();
        e.model.size = 1117320736;
        let reviewed = Capacity::reviewed(&e.model).unwrap();
        assert_eq!(
            (reviewed.max_context_tokens, reviewed.max_output_tokens),
            (32768, 8192)
        );
        e.model.size -= 1;
        assert!(Capacity::reviewed(&e.model).is_err());
        e.model.sha256 = "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db".into();
        e.model.size = 491400032;
        assert!(Capacity::reviewed(&e.model)
            .unwrap_err()
            .contains("minutes_model_limits_unknown"));
    }
    #[test]
    fn all_catalog_llms_bind_independent_exact_bytes_and_never_exceed_output_caps() {
        let choices: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../model-catalog/model-definitions.json"
        ))
        .unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for choice in choices
            .iter()
            .filter(|c| c["manifest"]["engine"] == "llama.cpp")
        {
            let file = choice["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["role"] == "llm")
                .unwrap();
            let mut e = engine();
            e.model.sha256 = file["expectedHash"].as_str().unwrap().into();
            e.model.size = file["expectedSize"].as_u64().unwrap();
            assert!(seen.insert(e.model.sha256.clone()));
            e.context_tokens = 32768;
            let capacity = Capacity::reviewed(&e.model).unwrap();
            let expected_output = choice["tokenLimits"]["maxOutputTokens"].as_u64().unwrap() as u32;
            assert_eq!(
                capacity
                    .execution(&e, "system", "短い入力")
                    .unwrap()
                    .prediction_tokens,
                expected_output
            );
            let long = capacity.execution(&e, "system", &"x".repeat(27000));
            let context = choice["tokenLimits"]["maxContextTokens"].as_u64().unwrap() as u32;
            if expected_output > ADAPTIVE_OUTPUT_RESERVE {
                let runtime = long.unwrap();
                assert_eq!(runtime.context_tokens, context);
                assert_eq!(
                    runtime.prediction_tokens,
                    expected_output.min(context - 27000 - 6 - 512)
                );
            } else {
                assert!(long.is_err());
            }
            if context == 262144 {
                let limit = capacity.input_limit(&e, 5400).unwrap();
                assert_eq!(limit, 262144 - 8192 - 512);
                let runtime = capacity.execution(&e, "", &"x".repeat(limit)).unwrap();
                assert_eq!(
                    (runtime.context_tokens, runtime.prediction_tokens),
                    (262144, 8192)
                );
                assert!(capacity.execution(&e, "", &"x".repeat(limit + 1)).is_err());
            }
            e.model.size += 1;
            assert!(Capacity::reviewed(&e.model).is_err());
        }
        assert_eq!(seen.len(), 6);
    }
    #[test]
    fn model_input_limit_is_independent_of_context_and_output_limits() {
        let e = engine();
        let mut c = Capacity::synthetic(&e, 32768, 8192);
        c.max_input_tokens = 3000;
        assert_eq!(c.input_limit(&e, 5400).unwrap(), 2488);
        assert!(c.execution(&e, "system", &"x".repeat(2500)).is_err());
        assert!(c.execution(&e, "system", &"x".repeat(2400)).is_ok());
    }

    #[test]
    fn older_capacity_allocations_are_rejected() {
        let e = engine();
        for allocation in ["profile", "modelMaximum"] {
            let mut value = serde_json::to_value(Capacity::synthetic(&e, 32768, 8192)).unwrap();
            value["allocation"] = serde_json::json!(allocation);
            assert!(serde_json::from_value::<Capacity>(value).is_err());
        }
    }
    #[test]
    fn model_adaptive_uses_remaining_context_and_bounds_input_by_minimum_output() {
        let e = engine(); // The saved 8192 / 2048 profile must not limit new jobs.
        let c = Capacity::synthetic_model_adaptive(&e, 40960, 32768);
        assert_eq!(c.input_limit(&e, 2400).unwrap(), 32256);
        let short = c.execution(&e, "system", "短い入力").unwrap();
        assert_eq!(
            (short.context_tokens, short.prediction_tokens),
            (40960, 32768)
        );
        // Exact system/prompt byte sizes at HRB_202604's stalled merge.
        let pair = c
            .execution(&e, &"s".repeat(635), &"p".repeat(9986))
            .unwrap();
        assert_eq!(pair.prediction_tokens, 29827);
        assert_eq!(
            (pair.threads, pair.gpu_layers, pair.seed),
            (e.threads, e.gpu_layers, e.seed)
        );
        assert_eq!(pair.model, e.model);
        assert_eq!(pair.executable, e.executable);
        let boundary = c.execution(&e, "", &"x".repeat(32256)).unwrap();
        assert_eq!(boundary.prediction_tokens, 8192);
        assert!(c.execution(&e, "", &"x".repeat(32257)).is_err());
        assert!(c.execution(&e, "", &"x".repeat(40960)).is_err());
        let mut input_limited = c.clone();
        input_limited.max_input_tokens = 10000;
        assert_eq!(input_limited.input_limit(&e, 2400).unwrap(), 9488);
        assert!(input_limited.execution(&e, "", &"x".repeat(9489)).is_err());
        let small = Capacity::synthetic_model_adaptive(&e, 4096, 1024);
        assert_eq!(small.input_limit(&e, 2400).unwrap(), 2560);
        assert_eq!(
            small
                .execution(&e, "", "短い入力")
                .unwrap()
                .prediction_tokens,
            1024
        );
        assert_eq!(
            serde_json::to_value(&c).unwrap()["allocation"],
            "modelAdaptive"
        );
        assert_eq!(
            serde_json::from_value::<Capacity>(serde_json::to_value(&c).unwrap()).unwrap(),
            c
        );
    }

    #[test]
    fn adaptive_merge_can_consume_both_hrb_sized_drafts_without_rewriting_them_separately() {
        use crate::generation::markdown_plan as plan;
        let e = engine();
        let new = Capacity::synthetic_model_adaptive(&e, 40960, 32768);
        let parts = vec!["左".repeat(1092), format!("{}x", "右".repeat(2196))];
        assert_eq!(
            parts.iter().map(String::len).collect::<Vec<_>>(),
            vec![3276, 6589]
        );
        let mut cursor =
            plan::Cursor::new(parts.clone(), new.plan_limit(&e, 2400).unwrap()).unwrap();
        let prompt = cursor.task().unwrap();
        let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        assert_eq!(value["drafts"], serde_json::json!(parts));
        let system = format!("{}{COMPLETION_INSTRUCTION}", plan::MERGE_SYSTEM);
        let runtime = new.execution(&e, &system, &prompt).unwrap();
        assert!(runtime.prediction_tokens < 32768 && runtime.prediction_tokens >= 8192);
        assert!(system.len() + prompt.len() + 512 + runtime.prediction_tokens as usize <= 40960);
        cursor.accept("# 統合した議事録".into()).unwrap();
        assert_eq!(cursor.result(), Some("# 統合した議事録"));
    }

    #[test]
    fn adaptive_partition_keeps_all_utf8_bytes_and_every_task_has_output_room() {
        use crate::generation::{markdown, markdown_plan as plan};
        let e = engine();
        let c = Capacity::synthetic_model_adaptive(&e, 40960, 32768);
        let limit = c.plan_limit(&e, 2400).unwrap();
        let mut input = crate::storage::current_minutes_fixture::input();
        input.analysis.utterances.truncate(1);
        let original = "議題。\"引用\"\\資料\n".repeat(4000);
        input.analysis.utterances[0].effective_text = original.clone();
        let planned = plan::build(&input, limit).unwrap();
        let mut rebuilt = String::new();
        let mut calls = 0;
        for node in planned.nodes {
            if let plan::Node::Extract { slices } = node {
                let prompt = plan::extract_prompt(&input, &slices).unwrap();
                let system = format!("{}{COMPLETION_INSTRUCTION}", markdown::system());
                let runtime = c.execution(&e, &system, &prompt).unwrap();
                assert!((8192..=32768).contains(&runtime.prediction_tokens));
                assert!(
                    system.len() + prompt.len() + 512 + runtime.prediction_tokens as usize <= 40960
                );
                for slice in slices {
                    rebuilt.push_str(&original[slice.first_byte..slice.end_byte]);
                }
                calls += 1;
            }
        }
        assert!(calls > 1);
        assert_eq!(rebuilt, original);
        let parts = vec![original.clone(), original];
        let mut cursor = plan::Cursor::new(parts.clone(), limit).unwrap();
        let mut consumed = String::new();
        while cursor.result().is_none() {
            let prompt = cursor.task().unwrap();
            let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
            let system = format!("{}{COMPLETION_INSTRUCTION}", plan::MERGE_SYSTEM);
            let runtime = c.execution(&e, &system, &prompt).unwrap();
            assert!((8192..=32768).contains(&runtime.prediction_tokens));
            if consumed.len() < parts.concat().len() {
                for text in value["drafts"].as_array().unwrap() {
                    consumed.push_str(text.as_str().unwrap());
                }
            }
            cursor.accept("統合した議事録".into()).unwrap();
        }
        assert_eq!(consumed, parts.concat());
    }
}
