//! Initial-download transport/staging, not placement or runtime readiness.
//! IPC callers must hold the model-preparation
//! lease for the whole operation (including child teardown and durable IO).
//! Download recipes are explicit untrusted input, not an authenticated catalog.
pub(crate) mod catalog;
mod transport;
#[cfg(test)]
use crate::model_download_recipe::MAX_TOTAL;
use crate::model_download_recipe::{validate_url, Recipe, Review, MAX_RECIPE};
use crate::model_inspection::{self, Report, Source};
use crate::model_placement::fs::Directory;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const INTENT: &str = "download-intent.json";
const COMPLETE: &str = "download-complete.json";
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        return Err("モデルダウンロードを中断しました。途中ファイルは保全します".into());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadReceipt {
    pub operation_id: String,
    pub recipe_hash: String,
    pub directory: PathBuf,
    pub report: Report,
    pub installed: bool,
    pub ready: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Intent {
    schema_version: u32,
    operation_id: String,
    recipe_hash: String,
    recipe: Recipe,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Completion {
    schema_version: u32,
    operation_id: String,
    recipe_hash: String,
    review_hash: String,
    total_bytes: u64,
}

/// A readback is never a resume or an engine-readiness grant. Malformed or
/// changed evidence is an error, not a projection to an empty/successful state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Readback {
    pub(crate) state: &'static str,
    pub(crate) review: Review,
    pub(crate) receipt: Option<DownloadReceipt>,
    #[serde(skip)]
    recipe: Recipe,
    #[serde(skip)]
    evidence: Vec<Source>,
    #[serde(skip)]
    directories: Vec<Directory>,
}
impl Readback {
    fn verify(&self) -> Result<(), String> {
        for source in &self.evidence {
            source.verify()?;
        }
        for directory in &self.directories {
            directory.verify()?;
        }
        Ok(())
    }
}

fn read_record<T: serde::de::DeserializeOwned>(
    directory: &Path,
    name: &str,
    limit: usize,
) -> Result<(T, Source), String> {
    let mut source = Source::open(directory, name)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut source.file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "ダウンロード記録を読み戻せません")?;
    source.verify()?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err("ダウンロード記録の容量が不正です".into());
    }
    let record = serde_json::from_slice(&bytes).map_err(|_| "ダウンロード記録の形式が不正です")?;
    Ok((record, source))
}

pub(crate) fn readback(
    staging_root: &Path,
    operation_id: &str,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Readback, String> {
    check(cancelled)?;
    if !uuid::Uuid::parse_str(operation_id).is_ok_and(|id| id.to_string() == operation_id) {
        return Err("ダウンロード操作IDが不正です".into());
    }
    let root = Directory::open_existing(staging_root)?;
    let operation = Directory::open_existing(&staging_root.join(operation_id))?;
    // Intent adds bounded metadata to the <=64KiB canonical recipe.
    let (intent, source): (Intent, _) = read_record(operation.path(), INTENT, MAX_RECIPE + 1024)?;
    let review = intent.recipe.review()?;
    if intent.schema_version != 1
        || intent.operation_id != operation_id
        || intent.recipe_hash != review.recipe_hash
        || intent.recipe_hash != expected_recipe_hash
    {
        return Err("ダウンロード意図が確認済みの操作・定義と一致しません".into());
    }
    let mut result = Readback {
        state: "incomplete",
        review,
        receipt: None,
        recipe: intent.recipe,
        evidence: vec![source],
        directories: vec![root, operation],
    };
    match std::fs::symlink_metadata(staging_root.join(operation_id).join(COMPLETE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            check(cancelled)?;
            result.verify()?;
            // No marker: preserve the intent and any payload. Do not contact
            // servers, adopt partial bytes, copy or guess that this succeeded.
            return Ok(result);
        }
        Err(_) => return Err("ダウンロード完了記録を確認できません".into()),
        Ok(_) => {}
    }
    let (completion, source): (Completion, _) =
        read_record(result.directories[1].path(), COMPLETE, 1024)?;
    result.evidence.push(source);
    if completion.schema_version != 1
        || completion.operation_id != operation_id
        || completion.recipe_hash != expected_recipe_hash
        || completion.total_bytes != result.review.total_bytes
    {
        return Err("ダウンロード完了記録が意図と一致しません".into());
    }
    let payload = Directory::open_existing(&staging_root.join(operation_id).join("payload"))?;
    let report = model_inspection::inspect(payload.path(), cancelled)?;
    let manifest_bytes = serde_json::to_vec(&result.recipe.manifest).map_err(|e| e.to_string())?;
    if report.manifest_hash != format!("{:x}", Sha256::digest(manifest_bytes))
        || report.review_hash != completion.review_hash
        || report.total_bytes != completion.total_bytes
    {
        return Err("ダウンロード実体が意図・完了記録と一致しません".into());
    }
    check(cancelled)?;
    result.receipt = Some(DownloadReceipt {
        operation_id: operation_id.into(),
        recipe_hash: completion.recipe_hash,
        directory: payload.path().to_path_buf(),
        report,
        installed: false,
        ready: false,
    });
    result.directories.push(payload);
    result.verify()?;
    result.state = "complete";
    Ok(result)
}

/// Separate explicit managed copy, after readback of native-owned staging.
/// It does not probe/execute a downloaded file or apply an engine profile.
pub(crate) fn place(
    storage: &crate::storage::Storage,
    models_root: &Path,
    staging_root: &Path,
    operation_id: &str,
    expected_recipe_hash: &str,
    package_id: &str,
    expected_review_hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::model_placement::PlacementReceipt, String> {
    let recovered = readback(staging_root, operation_id, expected_recipe_hash, cancelled)?;
    let receipt = recovered
        .receipt
        .as_ref()
        .ok_or("ダウンロードが未完了です。途中ファイルは保全します")?;
    if receipt.report.review_hash != expected_review_hash {
        return Err("ダウンロード実体の確認内容が変更されています".into());
    }
    let record = crate::model_placement::PlacementRecord::downloaded(
        package_id,
        &receipt.report,
        recovered.recipe.origin(operation_id)?,
    )?;
    crate::model_placement::place_reviewed(
        storage,
        models_root,
        &receipt.directory,
        &receipt.report,
        &record,
        cancelled,
        &|| recovered.verify(),
    )
}

/// Explicit staging only. Never runs downloaded executables, mutates settings,
/// registers a package, extracts archives, resumes, or deletes partial files.
/// Successful download is not manual_import and is not installed/ready.
pub(crate) fn download(
    staging_root: &Path,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    stage(
        staging_root,
        operation_id,
        recipe,
        expected_recipe_hash,
        cancelled,
        progress,
        &mut |url, size, hash, file, cancelled, progress| {
            transport::fetch(url, size, hash, file, cancelled, progress)
        },
    )
}

type Fetch<'a> = dyn FnMut(
        &str,
        u64,
        &str,
        &mut std::fs::File,
        &dyn Fn() -> bool,
        &mut dyn FnMut(u64),
    ) -> Result<(), String>
    + 'a;

fn stage(
    staging_root: &Path,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
    fetch: &mut Fetch<'_>,
) -> Result<DownloadReceipt, String> {
    check(cancelled)?;
    if !uuid::Uuid::parse_str(operation_id).is_ok_and(|id| id.to_string() == operation_id) {
        return Err("ダウンロード操作IDが不正です".into());
    }
    let total = recipe.review()?.total_bytes;
    let recipe_hash = recipe.hash()?;
    if recipe_hash != expected_recipe_hash {
        return Err("ダウンロード定義が確認内容から変わっています。再確認してください".into());
    }
    let intent = serde_json::to_vec(&Intent {
        schema_version: 1,
        operation_id: operation_id.into(),
        recipe_hash: recipe_hash.clone(),
        recipe: recipe.clone(),
    })
    .map_err(|_| "ダウンロード意図を符号化できません")?;
    // Staging and later managed placement coexist. Reserve BOTH byte sets now,
    // though later placement must recheck free space on its actual filesystem.
    let required = total
        .checked_mul(2)
        .and_then(|n| n.checked_add(3 * MAX_RECIPE as u64 + 16 * 1024 * 1024))
        .ok_or("ダウンロード必要容量が範囲外です")?;
    let root = Directory::root(staging_root)?;
    if root.available()? < required {
        return Err("ダウンロードと配置に必要な空き容量が不足しています".into());
    }
    check(cancelled)?;
    let operation = root.child(operation_id, true)?;
    durable_file(&operation, INTENT, &intent)?;
    check(cancelled)?;
    let payload = operation.child("payload", true)?;
    let manifest_bytes =
        serde_json::to_vec(&recipe.manifest).map_err(|_| "モデルmanifestを符号化できません")?;
    let manifest_hash = format!("{:x}", Sha256::digest(&manifest_bytes));
    durable_file(&payload, model_inspection::MANIFEST, &manifest_bytes)?;
    let mut completed = 0_u64;
    progress(0, total);
    for (file, source) in recipe.manifest.files.iter().zip(&recipe.sources) {
        check(cancelled)?;
        root.verify()?;
        operation.verify()?;
        payload.verify()?;
        let (mut output, parent) = payload.create(&file.relative_path)?;
        let size = file.expected_size.unwrap();
        fetch(
            &source.url,
            size,
            file.expected_hash.as_ref().unwrap(),
            &mut output,
            cancelled,
            &mut |n| progress(completed + n, total),
        )?;
        output
            .sync_all()
            .map_err(|_| "ダウンロードファイルを確定できません")?;
        parent.sync()?;
        payload.verify()?;
        completed += size;
        progress(completed, total);
    }
    check(cancelled)?;
    // Re-read all physical files through the existing no-link preflight. The
    // fetched bytes/destination path may have been changed before completion.
    let report = model_inspection::inspect(payload.path(), cancelled)?;
    if report.total_bytes != total || report.manifest_hash != manifest_hash {
        return Err("ダウンロード後のmanifest・合計容量が確認内容と一致しません".into());
    }
    root.verify()?;
    operation.verify()?;
    payload.sync()?;
    check(cancelled)?;
    let completion = serde_json::to_vec(&Completion {
        schema_version: 1,
        operation_id: operation_id.into(),
        recipe_hash: recipe_hash.clone(),
        review_hash: report.review_hash.clone(),
        total_bytes: total,
    })
    .map_err(|_| "ダウンロード完了記録を符号化できません")?;
    durable_file(&operation, COMPLETE, &completion)?;
    root.sync()?;
    // Cancellation racing the already-durable success must not erase success.
    Ok(DownloadReceipt {
        operation_id: operation_id.into(),
        recipe_hash,
        directory: payload.path().to_path_buf(),
        report,
        installed: false,
        ready: false,
    })
}
fn durable_file(directory: &Directory, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let (mut file, parent) = directory.create(relative)?;
    file.write_all(bytes)
        .map_err(|_| "ダウンロード記録を書き込めません")?;
    file.sync_all()
        .map_err(|_| "ダウンロード記録を確定できません")?;
    parent.sync()?;
    directory.sync()
}

#[cfg(test)]
mod tests;
