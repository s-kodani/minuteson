//! Read-only candidates plus presentation receipts; feature commands own actions.
use crate::{
    model_download_recipe::Recipe,
    model_inspection::{Engine, Role},
    storage::Storage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
use tauri::{Manager, WebviewWindow};

#[derive(Default)]
pub(crate) struct Notices(Mutex<BTreeSet<(String, String)>>);
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notice {
    id: String,
    revision: String,
    kind: &'static str,
    title: String,
    message: String,
    priority: u8,
    deferred: bool,
    option_id: Option<String>,
    package_id: Option<String>,
    recipe_hash: Option<String>,
}
struct Information {
    id: &'static str,
    revision: &'static str,
    title: &'static str,
    message: &'static str,
}
// Register future bundled, read-only announcements here. No remote content.
const INFORMATION: &[Information] = &[];
fn information(definitions: &[Information], receipts: &BTreeMap<String, String>) -> Vec<Notice> {
    definitions
        .iter()
        .filter(|d| {
            receipts
                .get(d.id)
                .is_none_or(|r| r != &format!("{:x}", Sha256::digest(d.revision.as_bytes())))
        })
        .map(|d| Notice {
            id: d.id.into(),
            revision: format!("{:x}", Sha256::digest(d.revision.as_bytes())),
            kind: "information",
            title: d.title.into(),
            message: d.message.into(),
            priority: 10,
            deferred: false,
            option_id: None,
            package_id: None,
            recipe_hash: None,
        })
        .collect()
}
fn matching_runtime_weights(entry: &crate::model_placement::CatalogEntry, recipe: &Recipe) -> bool {
    if entry.source_type != "download"
        || !entry.placement_recorded
        || entry.body_removed
        || entry.engine != "llama.cpp"
        || entry.engine_version != "b11533"
        || recipe.manifest.engine != Engine::Llama
        || recipe.manifest.engine_version != "b11960"
    {
        return false;
    }
    let weights: Vec<_> = recipe
        .manifest
        .files
        .iter()
        .filter(|f| f.role == Role::Llm)
        .collect();
    !weights.is_empty()
        && weights.len() == entry.model_files.len()
        && weights.iter().all(|f| {
            entry.model_files.iter().any(|old| {
                old.role == f.role
                    && Some(old.size) == f.expected_size
                    && Some(&old.sha256) == f.expected_hash.as_ref()
            })
        })
}
fn candidates(app: &tauri::AppHandle) -> Result<Vec<Notice>, String> {
    let storage = app.state::<Storage>();
    let mut result = information(INFORMATION, &storage.startup_notice_receipts()?);
    let profiles = storage.model_profiles()?;
    let entries = storage.list_model_placements()?;
    let used = profiles
        .iter()
        .find(|p| p.name == crate::storage::model_profiles::ProfileName::Standard)
        .and_then(|p| {
            p.components
                .iter()
                .find(|c| c.engine == Engine::Llama && c.settings_match && c.compatibility_current)
        });
    if let Some(entry) = used.and_then(|c| {
        entries.iter().find(|e| {
            e.package_id == c.package_id
                && e.source_type == "download"
                && e.placement_recorded
                && !e.body_removed
                && e.engine == "llama.cpp"
                && e.engine_version == "b11533"
        })
    }) {
        for option in crate::model_official_catalog::native_options(app)? {
            let recipe: Recipe =
                serde_json::from_str(&option.definition).map_err(|e| e.to_string())?;
            if !matching_runtime_weights(entry, &recipe) {
                continue;
            }
            let revision = format!(
                "{:x}",
                Sha256::digest(format!(
                    "{}:{}",
                    entry.package_id, option.review.recipe_hash
                ))
            );
            result.push(Notice { id:"llama-runtime-update".into(),revision,kind:"model_runtime_update",title:"議事録生成エンジンの更新".into(),message:"モデル本体は変更せず、議事録生成エンジンを更新します。再利用できるモデルデータはダウンロードせずコピーします。旧コピーを保持するため追加の空き容量が必要です。更新完了までは現在の構成を保持します。".into(),priority:0,deferred:false,option_id:Some(option.id),package_id:Some(entry.package_id.clone()),recipe_hash:Some(option.review.recipe_hash) });
            break;
        }
    }
    result.sort_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)));
    Ok(result)
}
fn main_only(window: &WebviewWindow, command: &str) -> Result<(), String> {
    if window.label() != "main" {
        return Err("起動時案内はメイン画面で確認してください".into());
    }
    window
        .state::<crate::startup_attention::Startup>()
        .admit_command(window.label(), command)
}
#[tauri::command]
pub(crate) async fn get_startup_notices(
    window: WebviewWindow,
) -> Result<Vec<Notice>, mon_contracts::AppError> {
    let result: Result<Vec<Notice>, String> = async {
        main_only(&window, "get_startup_notices")?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut notices = candidates(&app)?;
            let state = app.state::<Notices>();
            let deferred = state.0.lock().map_err(|_| "起動時案内を確認できません")?;
            for n in &mut notices {
                n.deferred = deferred.contains(&(n.id.clone(), n.revision.clone()));
            }
            Ok(notices)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|m| mon_contracts::AppError::from_command("get_startup_notices", m))
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Response {
    Acknowledge,
    Later,
}
#[tauri::command]
pub(crate) async fn respond_startup_notice(
    window: WebviewWindow,
    id: String,
    revision: String,
    response: Response,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = async {
        main_only(&window, "respond_startup_notice")?;
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let notice = candidates(&app)?
                .into_iter()
                .find(|n| n.id == id && n.revision == revision)
                .ok_or("案内の対象が変更されました。案内を開き直してください")?;
            if matches!(response, Response::Acknowledge) {
                if notice.kind != "information" {
                    return Err("操作が必要な案内は既読だけでは完了できません".into());
                }
                app.state::<Storage>()
                    .acknowledge_startup_notice(&id, &revision)?;
            }
            app.state::<Notices>()
                .0
                .lock()
                .map_err(|_| "案内の延期を記録できません")?
                .insert((id, revision));
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|m| mon_contracts::AppError::from_command("respond_startup_notice", m))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_notice_requires_exact_managed_weights_and_an_old_present_package() {
        let choices: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../model-catalog/macos.json")).unwrap();
        let recipe: Recipe = serde_json::from_value(choices[0]["recipe"].clone()).unwrap();
        let weight = recipe
            .manifest
            .files
            .iter()
            .find(|f| f.role == Role::Llm)
            .unwrap();
        let mut entry = crate::model_placement::CatalogEntry {
            package_id: uuid::Uuid::new_v4().to_string(),
            source_type: "download".into(),
            download: None,
            model_name: recipe.manifest.model_name.clone(),
            model_version: recipe.manifest.model_version.clone(),
            quantization: recipe.manifest.quantization.clone(),
            model_files: vec![crate::model_placement::ModelFileIdentity {
                role: Role::Llm,
                size: weight.expected_size.unwrap(),
                sha256: weight.expected_hash.clone().unwrap(),
            }],
            engine: "llama.cpp".into(),
            engine_version: "b11533".into(),
            backend_id: None,
            total_bytes: 1,
            placement_recorded: true,
            body_removed: false,
            ready: false,
            review_hash: "a".repeat(64),
            compatibility: None,
        };
        assert!(matching_runtime_weights(&entry, &recipe));
        entry.model_files[0].sha256 = "b".repeat(64);
        assert!(!matching_runtime_weights(&entry, &recipe));
        entry.model_files[0].sha256 = weight.expected_hash.clone().unwrap();
        entry.body_removed = true;
        assert!(!matching_runtime_weights(&entry, &recipe));
        entry.body_removed = false;
        entry.source_type = "manual_import".into();
        assert!(!matching_runtime_weights(&entry, &recipe));
        entry.source_type = "download".into();
        entry.engine_version = "b11960".into();
        assert!(!matching_runtime_weights(&entry, &recipe));
    }
    #[test]
    fn startup_notice_commands_are_main_only() {
        for command in ["get_startup_notices", "respond_startup_notice"] {
            assert!(crate::command_roles::permits(command, "main"));
            for role in [
                "recorder",
                "recorder-info",
                "recording-confirmation",
                "startup-attention",
                "unknown",
            ] {
                assert!(!crate::command_roles::permits(command, role));
            }
        }
    }
    #[test]
    fn information_receipts_match_only_the_exact_revision() {
        let definitions = [Information {
            id: "feature",
            revision: "r1",
            title: "案内",
            message: "本文",
        }];
        let mut receipts = BTreeMap::new();
        assert_eq!(information(&definitions, &receipts).len(), 1);
        receipts.insert("feature".into(), "r0".into());
        assert_eq!(information(&definitions, &receipts).len(), 1);
        receipts.insert("feature".into(), format!("{:x}", Sha256::digest(b"r1")));
        assert!(information(&definitions, &receipts).is_empty());
    }
}
