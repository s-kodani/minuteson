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
    id: String,
    label: String,
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
        assert_eq!(choices.len(), 11);
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
                    if choice.id.contains("qwen35-") || choice.id.contains("lfm3-") {
                        (262144, 32768, 262144)
                    } else if choice.id.contains("gemma4-") || choice.id.contains("phi4-") {
                        (131072, 32768, 131072)
                    } else if matches!(
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
                    "4f2b8fe8ba750b9dcf5feb531f13f1929845556b91265d09f51a52e4d215d7a3"
                }
                "sherpa-1138-pyannote3-eres2net-base" => {
                    "b564f0b5f9b2b88dbfebed9cb7c5ff6f12403b7ee5619701aa582153d443e666"
                }
                "llama-b11533-qwen25-15b-q8" => {
                    "a8f5330b3cd2a0c700b81b0d0402473c441dad3faa3814060162ec132d1fdd59"
                }
                "sherpa-1138-pyannote3-int8-eres2net-base" => {
                    "e1dc02a6b2a5224f76e2432f7daaf97154427fccb3c16627340ccf07380c8b96"
                }
                "llama-b11533-qwen3-8b-q4km" => {
                    "344413ffe456e123c918af5f422a82f64f7443fa6a81f332a58068caeed12f14"
                }
                "llama-b11533-qwen3-4b-q4km" => {
                    "920cc12b71fdc8d1dc66234050be25d0c577f16a639e4bf55048ca8e634e70b8"
                }
                "llama-b11533-qwen35-4b-q4km" => {
                    "00f77ecbf06bfa6823532998d9a79785097ad153838ee90c5fb85bfe6e7835b7"
                }
                "llama-b11533-qwen35-9b-q4km" => {
                    "a87db4f037d6eb2276580a1a2f296cdbe479025b84b0363a6b969ac78e0f2ac2"
                }
                "llama-b11960-gemma4-e4b-it-q4km" => {
                    "733daf690e7514e028c97f186ecb3f806a00338347ba2aed83f15abf9facd4aa"
                }
                "llama-b11960-phi4-mini-instruct-q4km" => {
                    "5303c39fa0d3b3114d52fc071a9e09ea5502532d1a52104467db8b5f0680d190"
                }
                "llama-b11960-lfm3-17b-q4km" => {
                    "9193aca93434d1450a772da9a997c4b28265537479a9a452c7fda4827f87d75f"
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
