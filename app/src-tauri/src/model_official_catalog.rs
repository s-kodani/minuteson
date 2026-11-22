//! Version-fixed download choices. Availability here is not measured quality,
//! publisher signature authentication, execution consent, or profile selection.
use crate::model_download_recipe::Recipe;
use crate::model_download_recipe::RecipeExt;
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
    pub(crate) id: String,
    pub(crate) label: String,
    description: String,
    pub(crate) definition: String,
    pub(crate) review: crate::model_download_recipe::Review,
    token_limits: Option<crate::generation::model_budget::TokenLimits>,
    license_summary: crate::model_license::LicenseSummary,
}
fn options() -> Result<Vec<OptionView>, String> {
    let choices: Vec<Choice> = serde_json::from_str(include_str!("../../model-catalog/macos.json"))
        .map_err(|_| "モデル選択情報を読み取れません")?;
    choices.into_iter().map(option_view).collect()
}
fn option_view(choice: Choice) -> Result<OptionView, String> {
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
        license_summary: crate::model_license::official_summary(&choice.recipe.manifest)?,
        id: choice.id,
        label: choice.label,
        description: choice.description,
        definition: String::from_utf8(choice.recipe.bytes()?).map_err(|e| e.to_string())?,
        review: choice.recipe.review()?,
        token_limits: choice.token_limits,
    })
}

#[tauri::command]
pub(crate) fn get_official_model_options(
    window: WebviewWindow,
) -> Result<serde_json::Value, mon_contracts::AppError> {
    let result: Result<serde_json::Value, String> = (|| {
        if window.label() != "main" {
            return Err("モデル選択はメイン画面で行ってください".into());
        }
        serde_json::to_value(native_options(window.app_handle())?).map_err(|e| e.to_string())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_official_model_options", message)
    })
}

pub(crate) fn native_options(app: &tauri::AppHandle) -> Result<Vec<OptionView>, String> {
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    {
        options_with_bundle(&resources.join("asr-runtime"))
    }
    #[cfg(target_os = "windows")]
    {
        options_windows(&resources.join("windows-runtime"))
    }
}

#[cfg(windows)]
pub(crate) fn options_windows(root: &std::path::Path) -> Result<Vec<OptionView>, String> {
    let choices: Vec<Choice> = serde_json::from_slice(&crate::windows_runtime::catalog(root)?)
        .map_err(|e| e.to_string())?;
    choices.into_iter().map(option_view).collect()
}

#[cfg(target_os = "macos")]
pub(crate) fn options_with_bundle(root: &std::path::Path) -> Result<Vec<OptionView>, String> {
    let mut result = options()?;
    let mut whisper = Vec::new();
    for (name, recipe) in crate::model_bundled_runtime::Bundle::open(root)?.recipes()? {
        whisper.push(OptionView {
            license_summary: crate::model_license::official_summary(&recipe.manifest)?,
            id: format!("whisper-194-{name}-multilingual-vad620"),
            label: format!("Whisper {name}（多言語）"),
            description: format!("アプリ同梱のCPU・Metal対応エンジンと、固定版Whisper {name}（多言語）と発話区間検知用のSilero VADを準備します。文字起こしでは利用可能なGPUを使います。品質・速度は正式評価前です。"),
            definition: String::from_utf8(recipe.bytes()?).map_err(|e| e.to_string())?,
            review: recipe.review()?, token_limits: None,
        });
    }
    whisper.append(&mut result);
    for choice in crate::model_asr_catalog::options(
        &root
            .parent()
            .ok_or("ASR素材の場所がありません")?
            .join("inference-runtime"),
    )? {
        whisper.push(OptionView {
            license_summary: crate::model_license::official_summary(&choice.recipe.manifest)?,
            id: choice.id,
            label: choice.label,
            description: choice.description,
            definition: String::from_utf8(choice.recipe.bytes()?).map_err(|e| e.to_string())?,
            review: choice.recipe.review()?,
            token_limits: None,
        });
    }
    Ok(whisper)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn fixed_catalog_is_bounded_complete_and_review_only_with_notices() {
        let choices = options().unwrap();
        assert_eq!(choices.len(), 8);
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
                    if choice.id.contains("qwen35-") {
                        (262144, 32768, 262144)
                    } else if choice.id.contains("gemma4-") {
                        (131072, 32768, 131072)
                    } else if matches!(
                        choice.id.as_str(),
                        "llama-b11533-qwen3-8b-q4km" | "llama-b11533-qwen3-4b-q4km"
                    ) {
                        (40960, 32768, 40960)
                    } else {
                        panic!("unverified minutes model")
                    }
                );
            }
            assert_eq!(recipe.hash().unwrap(), choice.review.recipe_hash);
            let expected = match choice.id.as_str() {
                "sherpa-1138-pyannote3-eres2net-base" => {
                    "c8dfeba400033dc9c3076dd0335a28b79628800310b57da07a31656a2acc55ae"
                }
                "sherpa-1138-pyannote3-int8-eres2net-base" => {
                    "8849a721ab5b73353b6093de25354f921865c0ccddaa24db79535392a004a8af"
                }
                "sherpa-1138-pyannote3-int8-titanet-small" => {
                    "b2ac4fcbd3527ffe7776601b28eb6198d922164cc383340cc67a9cec2e16ceb7"
                }
                "llama-b11533-qwen3-8b-q4km" => {
                    "3dac185509b8baa483b55c2cb0af54f315427a82f8d1b75299fb755df6217583"
                }
                "llama-b11533-qwen3-4b-q4km" => {
                    "29b2c7cb6ec73039e2bfc5c97797e3e1a090c6d8c0cd27e8300574391bcd065f"
                }
                "llama-b11533-qwen35-4b-q4km" => {
                    "80d2c542ae08d0c91f45f732f74e468a5a369906128f6823dee8fc8b14b1a365"
                }
                "llama-b11533-qwen35-9b-q4km" => {
                    "8e6b9e8ee758acfc197825bcbc9aa8cca62a84f2c96166cc9810bd6d6778de6d"
                }
                "llama-b11960-gemma4-e4b-it-q4km" => {
                    "be0dcb833bfcc307124ace43b1059e6418832fefeed99b2f3a7630ec30a59316"
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
