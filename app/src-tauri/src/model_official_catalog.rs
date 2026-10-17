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
    let mut whisper = Vec::new();
    for (name, recipe) in crate::model_bundled_runtime::Bundle::open(root)?.recipes()? {
        whisper.push(OptionView {
            id: format!("whisper-194-{name}-multilingual-vad620"),
            label: format!("Whisper {name}（多言語）"),
            description: format!("アプリ同梱のCPU・Metal対応エンジンと、固定版Whisper {name}（多言語）と発話区間検知用のSilero VADを準備します。文字起こしでは利用可能なGPUを使います。品質・速度は正式評価前です。"),
            definition: String::from_utf8(recipe.bytes()?).map_err(|e| e.to_string())?,
            review: recipe.review()?, token_limits: None,
        });
    }
    whisper.append(&mut result);
    let result = whisper;
    Ok(result)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn fixed_catalog_is_bounded_complete_and_review_only_with_notices() {
        let choices = options().unwrap();
        assert_eq!(choices.len(), 6);
        let mut ids = std::collections::BTreeSet::new();
        for choice in choices {
            assert!(ids.insert(choice.id.clone()));
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
                    if matches!(
                        choice.id.as_str(),
                        "llama-b11533-qwen3-8b-q4km" | "llama-b11533-qwen3-4b-q4km"
                    ) {
                        (40960, 32768, 40960)
                    } else {
                        (32768, 8192, 32768)
                    }
                );
            }
            assert_eq!(recipe.hash().unwrap(), choice.review.recipe_hash);
            let expected = match choice.id.as_str() {
                "llama-b11533-qwen25-15b-q4km" => {
                    "e4aaf7c71da23a1cadc141d92208d6eac4274eb12fa1901383f7753234f210ba"
                }
                "sherpa-1138-pyannote3-eres2net-base" => {
                    "1389a2b0250673b9b01a534d65f112bdb48cdd40ea3e1d446de5798e7064baee"
                }
                "llama-b11533-qwen25-15b-q8" => {
                    "d8fcced97a1a295bce39eafe05a2411c58be3d7ab39dc286b549f008d3f06221"
                }
                "sherpa-1138-pyannote3-int8-eres2net-base" => {
                    "7ae4eeab8889de993a9c726a9cf3f11a6e70a2dd0c1c8a9596f85e33822c7cfe"
                }
                "llama-b11533-qwen3-8b-q4km" => {
                    "1d3b48eebaea1750b93abbe1551423ca080ddff9b6c4f5069952aa188acbde37"
                }
                "llama-b11533-qwen3-4b-q4km" => {
                    "75f65a85091ad8cf44a33bfe2902be95acb3984244b099777d868de3e4f41fc8"
                }
                _ => panic!("unverified catalog choice"),
            };
            let notices = match recipe.manifest.engine {
                crate::model_inspection::Engine::Llama => 2,
                crate::model_inspection::Engine::Sherpa => 5,
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
            assert!(recipe.transfer_bytes().unwrap() < 6 * 1024 * 1024 * 1024);
            assert!(recipe.manifest.license_ref.contains("notices"));
        }
    }
}
