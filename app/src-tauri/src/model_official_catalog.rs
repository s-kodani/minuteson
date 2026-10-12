//! Version-fixed download choices. Availability here is not measured quality,
//! publisher signature authentication, execution consent, or profile selection.
use crate::model_download_recipe::Recipe;
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewWindow};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Choice {
    id: String,
    label: String,
    description: String,
    recipe: Recipe,
    #[serde(default)]
    token_limits: Option<crate::generation::model_budget::TokenLimits>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OptionView {
    id: String,
    label: String,
    description: String,
    definition: String,
    review: crate::model_download_recipe::Review,
    token_limits: Option<crate::generation::model_budget::TokenLimits>,
}
fn options() -> Result<Vec<OptionView>, String> {
    let choices: Vec<Choice> = serde_json::from_str(include_str!("../../model-catalog/macos.json"))
        .map_err(|_| "モデル選択情報を読み取れません")?;
    choices
        .into_iter()
        .map(|choice| {
            if choice.recipe.manifest.engine == crate::model_inspection::Engine::Llama {
                choice
                    .token_limits
                    .as_ref()
                    .ok_or("議事録モデルの入力・出力トークン上限がありません")?
                    .validate()?;
            } else if choice.token_limits.is_some() {
                return Err("トークン上限は議事録モデルにだけ指定してください".into());
            }
            Ok(OptionView {
                id: choice.id,
                label: choice.label,
                description: choice.description,
                definition: String::from_utf8(choice.recipe.bytes()?).map_err(|e| e.to_string())?,
                review: choice.recipe.review()?,
                token_limits: choice.token_limits,
            })
        })
        .collect()
}
#[tauri::command]
pub(crate) fn get_official_model_options(
    window: WebviewWindow,
) -> Result<serde_json::Value, String> {
    if window.label() != "main" {
        return Err("モデル選択はメイン画面で行ってください".into());
    }
    #[cfg(target_os = "macos")]
    {
        let root = window
            .app_handle()
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("asr-runtime");
        serde_json::to_value(options_with_bundle(&root)?).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(serde_json::json!([]))
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn options_with_bundle(root: &std::path::Path) -> Result<Vec<OptionView>, String> {
    let mut result = options()?;
    let recipe = crate::model_bundled_runtime::Bundle::open(root)?.recipe()?;
    result.insert(0, OptionView {
        id: "whisper-194-small-multilingual".into(), label: "文字起こし".into(),
        description: "アプリ同梱のCPU実行ファイルと、固定版Whisper small（多言語）を準備します。品質・速度は正式評価前です。".into(),
        definition: String::from_utf8(recipe.bytes()?).map_err(|e| e.to_string())?, review: recipe.review()?, token_limits: None,
    });
    Ok(result)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn fixed_catalog_is_bounded_complete_and_review_only_with_notices() {
        let choices = options().unwrap();
        assert_eq!(choices.len(), 2);
        let mut ids = std::collections::BTreeSet::new();
        for choice in choices {
            assert!(ids.insert(choice.id));
            let recipe = Recipe::parse(choice.definition.as_bytes()).unwrap();
            if let Some(limits) = &choice.token_limits {
                let file = recipe
                    .manifest
                    .files
                    .iter()
                    .find(|f| f.role == crate::model_inspection::Role::Llm)
                    .unwrap();
                let proof = crate::local_engine::Fingerprint {
                    path: "/not-executed/model.gguf".into(),
                    size: file.expected_size.unwrap(),
                    sha256: file.expected_hash.clone().unwrap(),
                };
                crate::generation::model_budget::Capacity::reviewed(&proof).unwrap();
                assert_eq!(
                    (
                        limits.max_input_tokens,
                        limits.max_output_tokens,
                        limits.max_context_tokens
                    ),
                    (32768, 8192, 32768)
                );
            }
            assert_eq!(recipe.hash().unwrap(), choice.review.recipe_hash);
            let (expected, notices) = match recipe.manifest.engine {
                crate::model_inspection::Engine::Llama => (
                    "e4aaf7c71da23a1cadc141d92208d6eac4274eb12fa1901383f7753234f210ba",
                    2,
                ),
                crate::model_inspection::Engine::Sherpa => (
                    "0996bb77a0fab2f604c97af2bde017cebe813e83529228be92778c4befd4a7c8",
                    5,
                ),
                _ => panic!("unverified catalog engine"),
            };
            assert_eq!(choice.review.recipe_hash, expected);
            assert!(!choice.review.installed && !choice.review.ready);
            assert_eq!(choice.review.publisher_authenticity, "not_verified");
            assert_eq!(
                recipe
                    .manifest
                    .files
                    .iter()
                    .filter(|file| file.role == crate::model_inspection::Role::Notice)
                    .count(),
                notices
            );
            assert!(recipe.transfer_bytes().unwrap() < 4 * 1024 * 1024 * 1024);
            assert!(recipe.manifest.license_ref.contains("notices"));
        }
    }
}
