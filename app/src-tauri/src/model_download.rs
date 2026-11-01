//! Initial-download transport/staging, not placement or runtime readiness.
//! IPC callers must hold the model-preparation
//! lease for the whole operation (including child teardown and durable IO).
//! Download recipes are explicit untrusted input, not an authenticated catalog.
use crate::model_download_recipe::RecipeExt;
#[cfg(any(target_os = "macos", windows))]
mod archive;
pub(crate) mod catalog;
mod placement;
mod reuse;
#[cfg(target_os = "macos")]
mod transport;
#[cfg(windows)]
#[path = "model_download/transport_windows.rs"]
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
    place_with_progress(
        storage,
        models_root,
        staging_root,
        operation_id,
        expected_recipe_hash,
        package_id,
        expected_review_hash,
        cancelled,
        &|| {},
    )
}

pub(crate) fn place_with_progress(
    storage: &crate::storage::Storage,
    models_root: &Path,
    staging_root: &Path,
    operation_id: &str,
    expected_recipe_hash: &str,
    package_id: &str,
    expected_review_hash: &str,
    cancelled: &impl Fn() -> bool,
    verifying: &impl Fn(),
) -> Result<crate::model_placement::PlacementReceipt, String> {
    placement::place(
        storage,
        models_root,
        staging_root,
        operation_id,
        expected_recipe_hash,
        package_id,
        expected_review_hash,
        cancelled,
        verifying,
    )
}

/// Explicit staging only. Never runs downloaded executables, mutates settings,
/// registers a package, resumes, or deletes partial files. Explicit archive
/// members are streamed to private non-executable files, never disk-extracted.
/// Successful download is not manual_import and is not installed/ready.
pub(crate) fn download(
    staging_root: &Path,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    if recipe.has_bundled() {
        return Err("同梱実行ファイルはアプリの専用準備経路で照合してください".into());
    }
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

#[cfg(target_os = "macos")]
pub(crate) fn download_with_bundle(
    staging_root: &Path,
    bundle_root: &Path,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    download_with_bundle_reuse(
        staging_root,
        bundle_root,
        None,
        operation_id,
        recipe,
        expected_recipe_hash,
        cancelled,
        progress,
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn download_with_bundle_reuse(
    staging_root: &Path,
    bundle_root: &Path,
    reusable: Option<(&Path, &[crate::model_placement::CatalogEntry])>,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    if !recipe.has_bundled() {
        return download(
            staging_root,
            operation_id,
            recipe,
            expected_recipe_hash,
            cancelled,
            progress,
        );
    }
    if recipe.review()?.recipe_hash != expected_recipe_hash {
        return Err("同梱モデルの確認した取得定義と一致しません".into());
    }
    let mut bundle = if recipe
        .sources
        .iter()
        .any(|source| source.url == crate::model_download_recipe::BUNDLED_WHISPER)
    {
        let mut bundle = crate::model_bundled_runtime::Bundle::open(bundle_root)?;
        bundle.prepare(bundle_root, recipe, cancelled)?;
        Some(bundle)
    } else {
        None
    };
    let asr_root = bundle_root
        .parent()
        .ok_or("ASR素材の場所がありません")?
        .join("inference-runtime");
    if recipe
        .sources
        .iter()
        .any(|s| s.url == crate::model_asr_catalog::URI)
    {
        crate::model_asr_catalog::validate(&asr_root, recipe)?;
    }
    let candidates = reusable
        .map(|(root, entries)| reuse::candidates(root, entries, recipe))
        .unwrap_or_default();
    stage(
        staging_root,
        operation_id,
        recipe,
        expected_recipe_hash,
        cancelled,
        progress,
        &mut |url, size, hash, output, cancelled, report| {
            if url == crate::model_download_recipe::BUNDLED_WHISPER {
                bundle
                    .as_mut()
                    .ok_or("Whisperの同梱実体を確認できません")?
                    .fetch(url, size, hash, output, cancelled)
            } else if url == crate::model_asr_catalog::URI {
                crate::model_asr_catalog::fetch(&asr_root, size, hash, output, cancelled)
            } else if let Some(candidate) = candidates.iter().find(|c| c.url == url) {
                candidate.copy(size, hash, output, cancelled, report)
            } else {
                transport::fetch(url, size, hash, output, cancelled, report)
            }
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

pub(crate) fn download_platform(
    staging_root: &Path,
    bundle_root: &Path,
    reusable: Option<(&Path, &[crate::model_placement::CatalogEntry])>,
    operation_id: &str,
    recipe: &Recipe,
    expected_recipe_hash: &str,
    cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<DownloadReceipt, String> {
    #[cfg(target_os = "macos")]
    {
        download_with_bundle_reuse(
            staging_root,
            bundle_root,
            reusable,
            operation_id,
            recipe,
            expected_recipe_hash,
            cancelled,
            progress,
        )
    }
    #[cfg(windows)]
    {
        let candidates = reusable
            .map(|(root, entries)| reuse::candidates(root, entries, recipe))
            .unwrap_or_default();
        crate::windows_runtime::validate_recipe(bundle_root, recipe)?;
        stage(
            staging_root,
            operation_id,
            recipe,
            expected_recipe_hash,
            cancelled,
            progress,
            &mut |url, size, hash, output, cancelled, report| {
                if url == crate::model_download_recipe::BUNDLED_WINDOWS {
                    crate::windows_runtime::fetch(bundle_root, size, hash, output, cancelled)
                } else if let Some(candidate) = candidates.iter().find(|c| c.url == url) {
                    candidate.copy(size, hash, output, cancelled, report)
                } else {
                    transport::fetch(url, size, hash, output, cancelled, report)
                }
            },
        )
    }
}

#[cfg(all(test, windows))]
pub(crate) fn windows_offline_parity_stage(
    staging: &Path,
    bundle: &Path,
    operation: &str,
    recipe: &Recipe,
    hash: &str,
    index: &serde_json::Value,
) -> Result<DownloadReceipt, String> {
    crate::windows_runtime::validate_recipe(bundle, recipe)?;
    stage(
        staging,
        operation,
        recipe,
        hash,
        &|| false,
        &mut |_, _| {},
        &mut |url, size, sha, output, cancelled, report| {
            if url == crate::model_download_recipe::BUNDLED_WINDOWS {
                return crate::windows_runtime::fetch(bundle, size, sha, output, cancelled);
            }
            let path = Path::new(
                index[sha]["path"]
                    .as_str()
                    .ok_or_else(|| format!("verified offline asset missing: {sha}"))?,
            );
            let proof = crate::local_engine::Fingerprint::capture(path, &|| false)?;
            if proof.size != size || proof.sha256 != sha {
                return Err("オフライン素材の容量・SHAが一致しません".into());
            }
            let parents =
                crate::windows_files::parents(path.parent().ok_or("素材の親がありません")?)?;
            let mut source = crate::windows_files::read(path, false)?;
            let count = std::io::copy(&mut source, output).map_err(|e| e.to_string())?;
            if count != size {
                return Err("オフライン素材のコピー長が一致しません".into());
            }
            report(count);
            drop(parents);
            Ok(())
        },
    )
}
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
    let transfer_total = recipe.transfer_bytes()?;
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
    // Normal placement renames staging on the same filesystem. A cross-volume
    // fallback checks the destination's copy capacity when it is needed.
    let required = total
        .checked_add(recipe.archive_bytes()?)
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
    progress(0, transfer_total);
    let mut archives_seen = std::collections::BTreeSet::new();
    for (index, (file, source)) in recipe
        .manifest
        .files
        .iter()
        .zip(&recipe.sources)
        .enumerate()
    {
        check(cancelled)?;
        root.verify()?;
        operation.verify()?;
        payload.verify()?;
        if let Some(archive_source) = &source.archive {
            if !archives_seen.insert(&source.url) {
                continue;
            }

            {
                let archives = operation.child("archives", false)?;
                let name = format!("source-{index}.tar");
                let (mut output, parent) = archives.create(&name)?;
                fetch(
                    &source.url,
                    archive_source.expected_size,
                    &archive_source.expected_hash,
                    &mut output,
                    cancelled,
                    &mut |n| {
                        if !crate::model_download_recipe::is_bundled(&source.url) {
                            progress(completed + n, transfer_total);
                        }
                    },
                )?;
                output
                    .sync_all()
                    .map_err(|_| "アーカイブを確定できません")?;
                parent.sync()?;
                drop(output);
                let indices = recipe
                    .sources
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| (s.url == source.url).then_some(i))
                    .collect::<Vec<_>>();
                archive::extract(&archives, &name, &payload, recipe, &indices, cancelled)?;
                if !crate::model_download_recipe::is_bundled(&source.url) {
                    completed += archive_source.expected_size;
                }
                progress(completed, transfer_total);
                continue;
            }
        }
        let (mut output, parent) = payload.create(&file.relative_path)?;
        let size = file.expected_size.unwrap();
        fetch(
            &source.url,
            size,
            file.expected_hash.as_ref().unwrap(),
            &mut output,
            cancelled,
            &mut |n| progress(completed + n, transfer_total),
        )?;
        output
            .sync_all()
            .map_err(|_| "ダウンロードファイルを確定できません")?;
        parent.sync()?;
        payload.verify()?;
        completed += size;
        progress(completed, transfer_total);
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

#[cfg(all(test, target_os = "macos"))]
mod tests;
