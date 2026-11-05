//! Native-owned, offline disclosures. These records do not alter hashed recipes,
//! package identities, readiness, jobs, or stored preferences.
use crate::model_download_recipe::RecipeExt;
use crate::model_inspection::{Engine, PackageFile, PackageManifest, Role};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::OnceLock;
use tauri::{Manager, WebviewWindow};

#[path = "model_license_assets.rs"]
mod assets;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Registry {
    schema_version: u32,
    notices: Vec<NoticeRecord>,
    models: Vec<ModelPolicy>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NoticeRecord {
    name: String,
    file: String,
    bytes: usize,
    sha256: String,
    source_url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ModelPolicy {
    id: String,
    engine: Engine,
    model_files: Vec<Identity>,
    license_names: Vec<String>,
    summary: String,
    restrictions: Vec<String>,
    required_notices: Vec<NoticeIdentity>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NoticeIdentity {
    name: String,
    sha256: String,
    source_url: String,
}
impl NoticeIdentity {
    fn key(&self) -> (String, String, String) {
        (
            self.name.clone(),
            self.sha256.clone(),
            self.source_url.clone(),
        )
    }
}
impl NoticeRecord {
    fn key(&self) -> (String, String, String) {
        (
            self.name.clone(),
            self.sha256.clone(),
            self.source_url.clone(),
        )
    }
}

#[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Identity {
    role: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LicenseSummary {
    license_names: Vec<String>,
    summary: String,
    restrictions: Vec<String>,
    disclosure_hash: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    name: String,
    text: String,
    sha256: String,
    source_url: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Disclosure {
    disclosure_hash: String,
    model_label: String,
    license_names: Vec<String>,
    summary: String,
    restrictions: Vec<String>,
    notices: Vec<Notice>,
    official: bool,
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum LicenseTarget {
    Recipe(RecipeTarget),
    Package(PackageTarget),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RecipeTarget {
    recipe_hash: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PackageTarget {
    package_id: String,
    expected_review_hash: String,
}
#[derive(Serialize)]
#[serde(untagged)]
enum TargetBinding {
    Recipe {
        #[serde(rename = "recipeHash")]
        recipe_hash: String,
    },
    Package {
        #[serde(rename = "packageId")]
        package_id: String,
        #[serde(rename = "reviewHash")]
        review_hash: String,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LicenseReview {
    target_binding: TargetBinding,
    #[serde(flatten)]
    disclosure: Disclosure,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bounded(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.len() <= max
}
fn identities(files: &[PackageFile]) -> Vec<Identity> {
    let mut values: Vec<_> = files
        .iter()
        .filter(|f| {
            !matches!(
                f.role,
                Role::Executable | Role::RuntimeLibrary | Role::Notice
            )
        })
        .map(|f| Identity {
            role: serde_json::to_value(f.role)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            size: f.expected_size.unwrap_or(0),
            sha256: f.expected_hash.clone().unwrap_or_default(),
        })
        .collect();
    values.sort();
    values
}
fn verify_notice(record: &NoticeRecord, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() != record.bytes || digest(bytes) != record.sha256 {
        return Err("ライセンス原文の容量・SHAが一致しません".into());
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "ライセンス原文をUTF-8で読み取れません".into())
}
impl Registry {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut registry: Self =
            serde_json::from_slice(bytes).map_err(|_| "モデル利用条件の固定情報が不正です")?;
        if registry.schema_version != 1
            || registry.models.is_empty()
            || registry.models.len() > 64
            || registry.notices.len() > 32
        {
            return Err("モデル利用条件の形式・件数が不正です".into());
        }
        let mut hashes = BTreeSet::new();
        let mut total = 0usize;
        for record in &registry.notices {
            total = total
                .checked_add(record.bytes)
                .ok_or("ライセンス原文の合計容量が範囲外です")?;
            if !bounded(&record.name, 256)
                || !crate::model_download_ipc::hash(&record.sha256)
                || !hashes.insert(record.key())
                || record.file != format!("{}.txt", record.sha256)
                || record.bytes == 0
                || record.bytes > 400_000
                || total > 2 * 1024 * 1024
            {
                return Err("ライセンス原文の固定情報が不正です".into());
            }
            crate::model_download_recipe::validate_url(&record.source_url)?;
            verify_notice(
                record,
                assets::embedded(&record.sha256).ok_or("同梱ライセンス原文がありません")?,
            )?;
        }
        let mut ids = BTreeSet::new();
        let mut weights = BTreeSet::new();
        for model in &mut registry.models {
            model.model_files.sort();
            let identity: Vec<_> = model
                .model_files
                .iter()
                .map(|f| (&f.role, f.size, &f.sha256))
                .collect();
            if !ids.insert(model.id.clone())
                || !weights.insert(identity)
                || model.model_files.is_empty()
                || model.model_files.len() > 16
                || model
                    .model_files
                    .iter()
                    .any(|f| f.size == 0 || !crate::model_download_ipc::hash(&f.sha256))
                || model.license_names.is_empty()
                || model.license_names.len() > 16
                || model.license_names.iter().any(|s| !bounded(s, 256))
                || !bounded(&model.summary, 4096)
                || model.restrictions.len() > 16
                || model.restrictions.iter().any(|s| !bounded(s, 4096))
                || model.required_notices.is_empty()
                || model
                    .required_notices
                    .iter()
                    .any(|n| !hashes.contains(&n.key()))
            {
                return Err("モデル利用条件とモデル本体の識別情報が不正です".into());
            }
        }
        registry.validate_coverage()?;
        Ok(registry)
    }
    fn covers(&self, engine: Engine, files: &[PackageFile]) -> bool {
        let identity = identities(files);
        self.models
            .iter()
            .filter(|m| m.engine == engine && m.model_files == identity)
            .count()
            == 1
    }
    fn validate_coverage(&self) -> Result<(), String> {
        for source in [
            include_str!("../../model-catalog/macos.json"),
            include_str!("../../model-catalog/asr-transducer.json"),
        ] {
            let choices: Vec<serde_json::Value> =
                serde_json::from_str(source).map_err(|_| "公式カタログを読み取れません")?;
            for choice in choices {
                let manifest: PackageManifest =
                    serde_json::from_value(choice["recipe"]["manifest"].clone())
                        .map_err(|_| "公式モデル定義が不正です")?;
                if !self.covers(manifest.engine, &manifest.files) {
                    return Err("公式モデルの利用条件が欠けています".into());
                }
            }
        }
        let network: serde_json::Value =
            serde_json::from_str(include_str!("../../model-catalog/whisper-network.json"))
                .map_err(|_| "Whisper固定情報が不正です")?;
        let vad: serde_json::Value =
            serde_json::from_str(include_str!("../../model-catalog/whisper-vad-network.json"))
                .map_err(|_| "VAD固定情報が不正です")?;
        let mut files: Vec<PackageFile> = serde_json::from_value(network["files"].clone())
            .map_err(|_| "Whisper構成が不正です")?;
        files.extend(
            serde_json::from_value::<Vec<PackageFile>>(vad["files"].clone())
                .map_err(|_| "VAD構成が不正です")?,
        );
        let variants: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../model-catalog/whisper-variants.json"))
                .map_err(|_| "Whisper候補が不正です")?;
        for variant in variants {
            let weight = files.first_mut().ok_or("Whisperのモデル本体がありません")?;
            weight.expected_size = variant["bytes"].as_u64();
            weight.expected_hash = variant["sha256"].as_str().map(str::to_owned);
            if !self.covers(Engine::Whisper, &files) {
                return Err("Whisperモデルの利用条件が欠けています".into());
            }
        }
        Ok(())
    }
}
fn registry() -> Result<&'static Registry, String> {
    static REGISTRY: OnceLock<Result<Registry, String>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            Registry::parse(include_bytes!(
                "../../model-catalog/model-license-policy.json"
            ))
        })
        .as_ref()
        .map_err(Clone::clone)
}
impl Disclosure {
    fn bind_hash(mut self) -> Result<Self, String> {
        // This explicit array is also the renderer's canonical hash preimage.
        let notices: Vec<_> = self
            .notices
            .iter()
            .map(|n| (&n.name, &n.text, &n.sha256, &n.source_url))
            .collect();
        let bytes = serde_json::to_vec(&(
            "model-license-disclosure-v1",
            &self.model_label,
            &self.license_names,
            &self.summary,
            &self.restrictions,
            notices,
            self.official,
        ))
        .map_err(|e| e.to_string())?;
        self.disclosure_hash = digest(&bytes);
        Ok(self)
    }
    fn summary(&self) -> LicenseSummary {
        LicenseSummary {
            license_names: self.license_names.clone(),
            summary: self.summary.clone(),
            restrictions: self.restrictions.clone(),
            disclosure_hash: self.disclosure_hash.clone(),
        }
    }
}
fn official(manifest: &PackageManifest) -> Result<Option<Disclosure>, String> {
    let registry = registry()?;
    let model_files = identities(&manifest.files);
    let Some(model) = registry
        .models
        .iter()
        .find(|m| m.engine == manifest.engine && m.model_files == model_files)
    else {
        return Ok(None);
    };
    let mut present = BTreeSet::new();
    for file in manifest.files.iter().filter(|f| f.role == Role::Notice) {
        // Build provenance is not license text; its bytes remain recipe-bound.
        if manifest.engine == Engine::Whisper && file.relative_path == "build-info.json" {
            continue;
        }
        let Some(hash) = file.expected_hash.as_ref() else {
            return Ok(None);
        };
        let name = file
            .relative_path
            .strip_prefix("notices/")
            .unwrap_or(&file.relative_path);
        let preferred = model
            .required_notices
            .iter()
            .find(|n| &n.sha256 == hash && n.name == name);
        let record = registry.notices.iter().find(|n| {
            &n.sha256 == hash
                && n.name == name
                && Some(n.bytes as u64) == file.expected_size
                && preferred.is_none_or(|p| n.source_url == p.source_url)
        });
        let Some(record) = record else {
            return Ok(None);
        };
        present.insert(record.key());
    }
    if model
        .required_notices
        .iter()
        .any(|n| !present.contains(&n.key()))
    {
        return Ok(None);
    }
    let notices = present
        .iter()
        .map(|key| {
            let record = registry.notices.iter().find(|n| &n.key() == key).unwrap();
            Ok(Notice {
                name: record.name.clone(),
                text: verify_notice(
                    record,
                    assets::embedded(&record.sha256).ok_or("同梱ライセンス原文がありません")?,
                )?,
                sha256: record.sha256.clone(),
                source_url: record.source_url.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Some(
        Disclosure {
            disclosure_hash: String::new(),
            model_label: manifest.model_name.clone(),
            license_names: model.license_names.clone(),
            summary: model.summary.clone(),
            restrictions: model.restrictions.clone(),
            notices,
            official: true,
        }
        .bind_hash()?,
    ))
}
pub(crate) fn official_summary(manifest: &PackageManifest) -> Result<LicenseSummary, String> {
    official(manifest)?
        .ok_or(
            "このモデルの利用条件が未確認です。取得前に公式カタログの利用条件を修復してください"
                .to_owned(),
        )
        .map(|d| d.summary())
}
fn unknown(manifest: &PackageManifest) -> Result<Disclosure, String> {
    let text = manifest.license_ref.clone();
    Disclosure { disclosure_hash: String::new(), model_label: manifest.model_name.clone(), license_names: vec!["条件未確認".into()], summary: "このパッケージはアプリの利用条件カタログとの一致を確認できません。manifestのlicenseRefは提供者の申告であり、利用権の確認を示しません。".into(), restrictions: vec!["利用・再配布の前に、提供者の原文と必要な許諾を確認してください。".into()], notices: vec![Notice { name: "manifest licenseRef（提供者の申告・条件未確認）".into(), sha256: digest(text.as_bytes()), text, source_url: String::new() }], official: false }.bind_hash()
}
fn installed(
    manifest: &PackageManifest,
    catalog: &[PackageManifest],
) -> Result<Disclosure, String> {
    // Weight equality supplies license policy; only an exact native catalog
    // manifest supplies the official label for the whole installed package.
    if catalog.iter().any(|known| known == manifest) {
        official(manifest)?.ok_or_else(|| "公式モデルの利用条件がありません".into())
    } else {
        unknown(manifest)
    }
}

// A known weight identity must have complete terms before a new transfer starts.
// This check does not impose an acknowledgement record or gate running jobs.
pub(crate) fn validate_download_policy(manifest: &PackageManifest) -> Result<(), String> {
    let files = identities(&manifest.files);
    if registry()?
        .models
        .iter()
        .any(|m| m.engine == manifest.engine && m.model_files == files)
    {
        official_summary(manifest)?;
    }
    Ok(())
}
pub(crate) fn credit_notices() -> Result<Vec<(String, String)>, String> {
    registry()?
        .notices
        .iter()
        .map(|record| {
            Ok((
                format!(
                    "{} — {} — SHA-256: {}",
                    record.name, record.source_url, record.sha256
                ),
                verify_notice(
                    record,
                    assets::embedded(&record.sha256).ok_or("同梱ライセンス原文がありません")?,
                )?,
            ))
        })
        .collect()
}

#[tauri::command]
pub(crate) fn get_model_license_review(
    window: WebviewWindow,
    target: LicenseTarget,
) -> Result<serde_json::Value, mon_contracts::AppError> {
    let result: Result<serde_json::Value, String> = (|| {
        if window.label() != "main" {
            return Err("モデル利用条件はメイン画面で確認してください".into());
        }
        let review = match target {
            LicenseTarget::Recipe(target) => {
                if !crate::model_download_ipc::hash(&target.recipe_hash) {
                    return Err("モデル定義のSHAが不正です".into());
                }
                let choice = crate::model_official_catalog::native_options(window.app_handle())?
                    .into_iter()
                    .find(|c| c.review.recipe_hash == target.recipe_hash)
                    .ok_or("このモデル定義は現在の公式カタログにありません")?;
                let recipe =
                    crate::model_download_recipe::Recipe::parse(choice.definition.as_bytes())?;
                LicenseReview {
                    target_binding: TargetBinding::Recipe {
                        recipe_hash: target.recipe_hash,
                    },
                    disclosure: official(&recipe.manifest)?
                        .ok_or("公式モデルの利用条件がありません")?,
                }
            }
            LicenseTarget::Package(target) => {
                if !crate::model_download_ipc::hash(&target.expected_review_hash) {
                    return Err("モデル確認SHAが不正です".into());
                }
                let record = window
                    .state::<crate::Storage>()
                    .model_package_for_check(&target.package_id, &target.expected_review_hash)?;
                LicenseReview {
                    target_binding: TargetBinding::Package {
                        package_id: record.package_id,
                        review_hash: record.review_hash,
                    },
                    disclosure: installed(
                        &record.manifest,
                        &crate::model_official_catalog::native_options(window.app_handle())?
                            .into_iter()
                            .map(|c| c.review.manifest)
                            .collect::<Vec<_>>(),
                    )?,
                }
            }
        };
        serde_json::to_value(review).map_err(|e| e.to_string())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_model_license_review", message)
    })
}

#[cfg(test)]
mod tests;
