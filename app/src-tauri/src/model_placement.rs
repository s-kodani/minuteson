//! Manual placement is not runtime compatibility or AI readiness. All copying
//! is explicit, create-new-only, bound to a reviewed byte set and durable intent.
#[cfg(unix)]
pub(crate) mod fs;
#[cfg(target_os = "windows")]
#[path = "model_placement/fs_windows.rs"]
pub(crate) mod fs;
use crate::model_inspection::{self, FileReport, PackageManifest, Report, Role, Source, Target};
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PlacementRecord {
    pub schema_version: u32,
    pub package_id: String,
    pub manifest: PackageManifest,
    pub source_manifest_hash: String,
    pub review_hash: String,
    pub files: Vec<FileReport>,
    #[serde(deserialize_with = "crate::current_formats::optional")]
    pub download: Option<crate::model_download_recipe::DownloadOrigin>,
}
impl PlacementRecord {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_for(Target::current()?)
    }
    // Historical provenance can be decoded on a different supported host.
    // This does not grant permission to execute that host's package here.
    pub(crate) fn validate_portable(&self) -> Result<(), String> {
        self.validate_for(self.manifest.target)
    }
    fn validate_for(&self, target: Target) -> Result<(), String> {
        if !uuid(&self.package_id) {
            return Err("モデル配置記録の形式版・操作IDが不正です".into());
        }
        self.manifest.validate(target)?;
        if self.schema_version != 3 {
            return Err("モデル配置記録の形式版が未対応です".into());
        }
        if let Some(origin) = &self.download {
            origin.validate(&self.manifest, &self.source_manifest_hash)?;
        }
        if self.source_manifest_hash.len() != 64
            || !self
                .source_manifest_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.files.len() != self.manifest.files.len()
            || model_inspection::review_hash(
                &self.manifest,
                &self.source_manifest_hash,
                &self.files,
            )? != self.review_hash
        {
            return Err("モデル配置記録と確認済みファイルの証拠が一致しません".into());
        }
        for (declared, actual) in self.manifest.files.iter().zip(&self.files) {
            if declared.relative_path != actual.relative_path
                || declared.role != actual.role
                || actual.actual_size == 0
                || actual.actual_size > i64::MAX as u64
                || actual.actual_hash.len() != 64
                || !actual
                    .actual_hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || declared
                    .expected_size
                    .is_some_and(|n| n != actual.actual_size)
                || declared
                    .expected_hash
                    .as_ref()
                    .is_some_and(|h| h != &actual.actual_hash)
                || actual.expected_size_checked != declared.expected_size.is_some()
                || actual.expected_hash_checked != declared.expected_hash.is_some()
            {
                return Err("モデル配置記録の名前・役割・容量・SHAが一致しません".into());
            }
        }
        Ok(())
    }
    fn from_report(package_id: &str, report: &Report) -> Self {
        Self {
            schema_version: 3,
            package_id: package_id.into(),
            manifest: report.manifest.clone(),
            source_manifest_hash: report.manifest_hash.clone(),
            review_hash: report.review_hash.clone(),
            files: report.files.clone(),
            download: None,
        }
    }
    pub(crate) fn downloaded(
        package_id: &str,
        report: &Report,
        origin: crate::model_download_recipe::DownloadOrigin,
    ) -> Result<Self, String> {
        let mut record = Self::from_report(package_id, report);
        record.download = Some(origin);
        record.validate()?;
        Ok(record)
    }
    pub(crate) fn source_type(&self) -> &'static str {
        if self.download.is_some() {
            "download"
        } else {
            "manual_import"
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlacementReceipt {
    pub package_id: String,
    pub review_hash: String,
    pub total_bytes: u64,
    pub placement_recorded: bool,
    pub engine_compatibility: String,
    pub ready: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogEntry {
    pub package_id: String,
    pub source_type: String,
    pub download: Option<DownloadIdentity>,
    pub model_name: String,
    pub model_version: String,
    pub quantization: Option<String>,
    pub model_files: Vec<ModelFileIdentity>,
    pub engine: String,
    pub engine_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_id: Option<String>,
    pub total_bytes: u64,
    pub placement_recorded: bool,
    pub body_removed: bool,
    pub ready: bool,
    pub review_hash: String,
    pub compatibility: Option<crate::model_compatibility::CheckSummary>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelFileIdentity {
    pub role: crate::model_inspection::Role,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadIdentity {
    pub operation_id: String,
    pub recipe_hash: String,
}
fn uuid(text: &str) -> bool {
    uuid::Uuid::parse_str(text).is_ok_and(|id| id.to_string() == text)
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("モデル配置を中断しました。未完了の配置は保全します".into())
    } else {
        Ok(())
    }
}

/// Storage root is intentionally separate from the OS app-data model root.
pub(crate) fn place(
    storage: &Storage,
    models_root: &Path,
    source: &Path,
    package_id: &str,
    expected_review_hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<PlacementReceipt, String> {
    place_with_manifest(
        storage,
        models_root,
        source,
        package_id,
        expected_review_hash,
        None,
        cancelled,
    )
}

pub(crate) fn place_with_manifest(
    storage: &Storage,
    models_root: &Path,
    source: &Path,
    package_id: &str,
    expected_review_hash: &str,
    manifest: Option<&PackageManifest>,
    cancelled: &impl Fn() -> bool,
) -> Result<PlacementReceipt, String> {
    {
        if !uuid(package_id) {
            return Err("モデル配置の操作IDが不正です".into());
        }
        check(cancelled)?;
        let report = match manifest {
            Some(declared) => model_inspection::inspect_declared(source, declared, cancelled)?,
            None => model_inspection::inspect(source, cancelled)?,
        };
        if report.review_hash != expected_review_hash {
            return Err(
                "検査後にモデル構成・ファイルが変更されています。再検査してください".into(),
            );
        }
        let record = PlacementRecord::from_report(package_id, &report);
        let manifest_bytes = manifest
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|e| e.to_string())?;
        place_reviewed_with_manifest(
            storage,
            models_root,
            source,
            &report,
            &record,
            manifest_bytes.as_deref(),
            cancelled,
            &|| Ok(()),
        )
    }
}

pub(crate) fn place_reviewed(
    storage: &Storage,
    models_root: &Path,
    source: &Path,
    report: &Report,
    record: &PlacementRecord,
    cancelled: &impl Fn() -> bool,
    verify_origin: &impl Fn() -> Result<(), String>,
) -> Result<PlacementReceipt, String> {
    place_reviewed_with_manifest(
        storage,
        models_root,
        source,
        report,
        record,
        None,
        cancelled,
        verify_origin,
    )
}

fn place_reviewed_with_manifest(
    storage: &Storage,
    models_root: &Path,
    source: &Path,
    report: &Report,
    record: &PlacementRecord,
    manifest_bytes: Option<&[u8]>,
    cancelled: &impl Fn() -> bool,
    verify_origin: &impl Fn() -> Result<(), String>,
) -> Result<PlacementReceipt, String> {
    let package_id = &record.package_id;
    record.validate()?;
    check(cancelled)?;
    verify_origin()?;
    if report.review_hash != record.review_hash
        || report.manifest != record.manifest
        || report.files != record.files
        || report.manifest_hash != record.source_manifest_hash
    {
        return Err("モデル配置の検査内容と由来が一致しません".into());
    }
    // Serialize/check limits before creating a package or writing DB intent.
    let json = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    if json.len() > 65_536 {
        return Err("配置記録が64KiBを超えています。構成や長いファイル名を見直してください".into());
    }
    let root = fs::Directory::root(models_root)?;
    let required = report
        .total_bytes
        .checked_add(65_536 + 16 * 1024 * 1024)
        .ok_or("モデル配置容量が範囲外です")?;
    if root.available()? < required {
        return Err("モデル管理領域の空き容量が不足しています".into());
    }
    check(cancelled)?;
    if let Some(saved) = storage.model_placement_receipt(record)? {
        // Lost-response retry never creates or overwrites another package.
        if saved.placement_recorded {
            root.verify()?;
            let existing = model_inspection::inspect(&root.path().join(package_id), cancelled)?;
            if existing.review_hash != record.review_hash {
                return Err("配置済みモデルが欠損・変更されています。既存物は保全します".into());
            }
            verify_origin()?;
            return Ok(saved);
        }
        return Err(
            "この操作は未完了です。途中配置を上書きせず、新しい取り込み操作で再試行してください"
                .into(),
        );
    }
    storage.begin_model_placement(record)?;
    let result = (|| {
        check(cancelled)?;
        // UUID directory is created after durable intent, never adopted.
        let destination = root.child(package_id, true)?;
        copy_package(source, report, &destination, manifest_bytes, cancelled)?;
        root.verify()?;
        destination.sync()?;
        let copied = model_inspection::inspect(destination.path(), cancelled)?;
        if copied.review_hash != record.review_hash {
            return Err("配置先の全ファイルが検査済み素材と一致しません".into());
        }
        check(cancelled)?;
        root.verify()?;
        destination.verify()?;
        verify_origin()?;
        storage.finish_model_placement(record)?;
        Ok(PlacementReceipt {
            package_id: package_id.clone(),
            review_hash: record.review_hash.clone(),
            total_bytes: report.total_bytes,
            placement_recorded: true,
            engine_compatibility: "not_verified".into(),
            ready: false,
        })
    })();
    if result.is_err() {
        // Keep partial files and foreign/unknown objects. No guessed cleanup,
        // chmod of existing entries, recursive deletion, or automatic retry.
        if let Err(error) = storage.interrupt_model_placement(record) {
            return Err(format!(
                "配置失敗後の未完了記録を確定できません。再起動して確認してください: {error}"
            ));
        }
    }
    result
}

fn copy_package(
    source: &Path,
    expected: &Report,
    destination: &fs::Directory,
    manifest_bytes: Option<&[u8]>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let mut sources = Vec::new();
    if let Some(bytes) = manifest_bytes {
        check(cancelled)?;
        if bytes.len() > 65_536 || format!("{:x}", Sha256::digest(bytes)) != expected.manifest_hash
        {
            return Err("確認した画面の構成とコピーするmanifestが一致しません".into());
        }
        let (mut output, parent) = destination.create(model_inspection::MANIFEST)?;
        output.write_all(bytes).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        parent.sync()?;
    }
    for (relative, size, hash, executable) in std::iter::once((
        model_inspection::MANIFEST,
        None,
        expected.manifest_hash.as_str(),
        false,
    ))
    .chain(expected.files.iter().map(|f| {
        (
            f.relative_path.as_str(),
            Some(f.actual_size),
            f.actual_hash.as_str(),
            f.role == Role::Executable,
        )
    })) {
        if relative == model_inspection::MANIFEST && manifest_bytes.is_some() {
            continue;
        }
        check(cancelled)?;
        destination.verify()?;
        let mut input = Source::open(&source, relative)?;
        let (mut output, parent) = destination.create(relative)?;
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 65_536];
        loop {
            check(cancelled)?;
            let n = input.file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            count = count
                .checked_add(n as u64)
                .ok_or("モデルコピー容量が範囲外です")?;
            if count > size.unwrap_or(65_536) {
                return Err("コピー中にモデルファイルが増大しました".into());
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            digest.update(&buffer[..n]);
        }
        if size.is_some_and(|s| s != count) || format!("{:x}", digest.finalize()) != hash {
            return Err("コピー中に確認済みモデルの容量・SHAが変わりました".into());
        }
        input.verify()?;
        #[cfg(unix)]
        output
            .set_permissions(std::fs::Permissions::from_mode(if executable {
                0o700
            } else {
                0o600
            }))
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        parent.sync()?;
        destination.verify()?;
        sources.push(input);
    }
    check(cancelled)?;
    for source in sources {
        source.verify()?;
    }
    destination.sync()
}

#[cfg(all(test, unix))]
mod tests;
