//! Reviewed limits belong to exact model bytes, not a name, path or UI profile.
//! Qwen's byte-level BPE needs no more input tokens than UTF-8 bytes. This is
//! an upper bound, not an exact token count; template/framing gets its own
//! reserve. No such assumption is made about an unreviewed tokenizer.
use super::llama::LlamaConfig;
use crate::local_engine::Fingerprint;
use serde::{Deserialize, Serialize};

pub(super) const COMPLETION_INSTRUCTION: &str =
    "\n必要な事項を簡潔にまとめ、文とJSONを最後まで完結してください。";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Capacity {
    model_sha256: String,
    max_input_tokens: u32,
    max_context_tokens: u32,
    max_output_tokens: u32,
    template_reserve_tokens: u32,
    tokenizer: Tokenizer,
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
        let choices: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../model-catalog/macos.json"))
                .map_err(|_| "モデル選択情報を読み取れません")?;
        let choice = choices.iter().find(|choice| choice["recipe"]["manifest"]["engine"] == "llama.cpp"
            && choice["recipe"]["manifest"]["files"].as_array().is_some_and(|files| files.iter().any(|file|
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
        })
    }
    pub(super) fn validate(&self, engine: &LlamaConfig) -> Result<(), String> {
        engine.validate()?;
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
    pub(super) fn input_limit(
        &self,
        engine: &LlamaConfig,
        requested: usize,
    ) -> Result<usize, String> {
        self.validate(engine)?;
        let reserved_output = engine.prediction_tokens.min(self.max_output_tokens);
        let available = self
            .context(engine)
            .checked_sub(reserved_output)
            .and_then(|n| n.checked_sub(self.template_reserve_tokens))
            .ok_or("選択モデルの入力と生成予算を確保できません")? as usize;
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
        let input_upper_bound = system
            .len()
            .checked_add(prompt.len())
            .and_then(|n| u32::try_from(n).ok())
            .and_then(|n| n.checked_add(self.template_reserve_tokens))
            .ok_or("議事録の入力トークン予算が範囲外です")?;
        let available = self
            .context(engine)
            .checked_sub(input_upper_bound)
            .ok_or("選択モデルのコンテキストに入力が収まりません")?;
        if input_upper_bound > self.max_input_tokens {
            return Err("選択モデルの入力トークン上限を超えています".into());
        }
        let mut runtime = engine.clone();
        runtime.context_tokens = self.context(engine);
        runtime.prediction_tokens = available.min(self.max_output_tokens).min(8192);
        runtime.validate()?;
        Ok(runtime)
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let large = Capacity::synthetic(&e, 32768, 8192);
        let a = small.execution(&e, "system", &"あ".repeat(400)).unwrap();
        let b = large.execution(&e, "system", &"あ".repeat(400)).unwrap();
        assert_eq!((a.context_tokens, a.prediction_tokens), (4096, 1024));
        assert_eq!((b.context_tokens, b.prediction_tokens), (8192, 6474));
        let long = large.execution(&e, "system", &"あ".repeat(1800)).unwrap();
        assert_eq!(long.prediction_tokens, 2274);
        assert!(long.prediction_tokens < b.prediction_tokens);
        assert_eq!(small.input_limit(&e, 5400).unwrap(), 2560);
        assert!(large.execution(&e, "system", &"x".repeat(8000)).is_err());
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
        let choices: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../model-catalog/macos.json")).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for choice in choices
            .iter()
            .filter(|c| c["recipe"]["manifest"]["engine"] == "llama.cpp")
        {
            let file = choice["recipe"]["manifest"]["files"]
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
            assert_eq!(capacity.max_output_tokens, 8192);
            assert_eq!(
                capacity
                    .execution(&e, "system", "短い入力")
                    .unwrap()
                    .prediction_tokens,
                8192
            );
            assert!(
                capacity
                    .execution(&e, "system", &"x".repeat(27000))
                    .unwrap()
                    .prediction_tokens
                    < 8192
            );
            e.model.size += 1;
            assert!(Capacity::reviewed(&e.model).is_err());
        }
        assert_eq!(seen.len(), 2);
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
}
