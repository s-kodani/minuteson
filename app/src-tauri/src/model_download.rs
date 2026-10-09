//! Initial-download transport/staging, not placement or runtime readiness.
//! No IPC exposes this yet. Its future caller must hold the model-preparation
//! lease for the whole operation (including child teardown and durable IO).
//! Download recipes are explicit untrusted input, not an authenticated catalog.
mod transport;
use crate::model_inspection::{self, PackageManifest, Report, Target};
use crate::model_placement::fs::Directory;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_RECIPE: usize = 65_536;
const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
const INTENT: &str = "download-intent.json";
const COMPLETE: &str = "download-complete.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Recipe {
    schema_version: u32,
    manifest: PackageManifest,
    sources: Vec<DownloadSource>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DownloadSource {
    relative_path: String,
    url: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Review {
    recipe_hash: String,
    manifest: PackageManifest,
    sources: Vec<DownloadSource>,
    total_bytes: u64,
    publisher_authenticity: &'static str,
    installed: bool,
    ready: bool,
}
impl Recipe {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > MAX_RECIPE {
            return Err("ダウンロード定義は64KiB以内で指定してください".into());
        }
        let recipe: Self =
            serde_json::from_slice(bytes).map_err(|_| "ダウンロード定義の形式が不正です")?;
        recipe.validate()?;
        Ok(recipe)
    }
    pub(crate) fn review(&self) -> Result<Review, String> {
        let total_bytes = self.validate()?;
        Ok(Review {
            recipe_hash: self.hash()?,
            manifest: self.manifest.clone(),
            sources: self.sources.clone(),
            total_bytes,
            publisher_authenticity: "not_verified",
            installed: false,
            ready: false,
        })
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "ダウンロード定義を符号化できません")?;
        if bytes.len() > MAX_RECIPE {
            return Err("ダウンロード定義が64KiBを超えています".into());
        }
        Ok(bytes)
    }
    fn hash(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.bytes()?)))
    }
    fn validate(&self) -> Result<u64, String> {
        if self.schema_version != 1 {
            return Err("ダウンロード定義の形式版が未対応です".into());
        }
        self.manifest.validate(Target::current()?)?;
        if self.sources.len() != self.manifest.files.len() {
            return Err("モデルファイルと配布元の構成が一致しません".into());
        }
        let mut total = 0_u64;
        for (file, source) in self.manifest.files.iter().zip(&self.sources) {
            if file.relative_path != source.relative_path
                || file.expected_size.is_none()
                || file.expected_hash.is_none()
            {
                return Err("全ダウンロードファイルに同順の配布元・容量・SHA-256が必要です".into());
            }
            validate_url(&source.url)?;
            total = total
                .checked_add(file.expected_size.unwrap())
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or("ダウンロード合計容量の上限は32GiBです")?;
        }
        self.bytes()?;
        Ok(total)
    }
}

fn validate_url(value: &str) -> Result<(), String> {
    // Do not silently normalize a pasted credential, whitespace or backslash.
    // Query strings (often access tokens) are not accepted in initial recipes;
    // HTTPS-only server redirects may supply signed CDN query strings internally.
    let invalid = || "配布元は認証情報・query・fragmentを含まないHTTPS URLで指定してください";
    if value.len() > 4096
        || !value.starts_with("https://")
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(invalid().into());
    }
    let authority = value["https://".len()..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(invalid().into());
    }
    let parsed = tauri::Url::parse(value).map_err(|_| invalid())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid().into());
    }
    Ok(())
}
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
    let total = recipe.validate()?;
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
