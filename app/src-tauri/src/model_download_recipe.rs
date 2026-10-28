//! Portable download provenance. Validation is not publisher authentication or
//! permission to download/execute on the current host.
use crate::model_inspection::{PackageManifest, Target};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_RECIPE: usize = 65_536;
pub(crate) const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
// An identifier, never a user-selected local path. Resolution happens only at
// the native acquisition boundary against the app's own resource directory.
pub(crate) const BUNDLED_WHISPER: &str = "mon-bundled://whisper-v1.9.4/whisper-runtime.tar";
pub(crate) const BUNDLED_WINDOWS: &str = "mon-bundled://windows-runtime-v1/runtime.tar";
pub(crate) fn is_bundled(url: &str) -> bool {
    matches!(url, BUNDLED_WHISPER | BUNDLED_WINDOWS) || url == crate::model_asr_catalog::URI
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Recipe {
    pub(super) schema_version: u32,
    pub(super) manifest: PackageManifest,
    pub(super) sources: Vec<DownloadSource>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DownloadSource {
    pub(super) relative_path: String,
    pub(super) url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) archive: Option<ArchiveMember>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ArchiveMember {
    pub(super) format: String,
    pub(super) expected_size: u64,
    pub(super) expected_hash: String,
    pub(super) member_path: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Review {
    pub(crate) recipe_hash: String,
    pub(crate) manifest: PackageManifest,
    pub(crate) sources: Vec<DownloadSource>,
    pub(crate) total_bytes: u64,
    pub(crate) recipe_schema_version: u32,
    pub(crate) download_bytes: u64,
    pub(crate) archive_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bundled_bytes: Option<u64>,
    pub(crate) publisher_authenticity: &'static str,
    pub(crate) installed: bool,
    pub(crate) ready: bool,
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
        self.validate()?;
        self.ensure_placement_record_fits()?;
        Ok(Review {
            recipe_hash: self.hash()?,
            manifest: self.manifest.clone(),
            sources: self.sources.clone(),
            total_bytes: self.validate()?,
            recipe_schema_version: self.schema_version,
            download_bytes: self.transfer_bytes()?,
            archive_bytes: self.archive_bytes()?,
            bundled_bytes: self
                .has_bundled()
                .then(|| self.bundled_bytes())
                .transpose()?,
            publisher_authenticity: "not_verified",
            installed: false,
            ready: false,
        })
    }
    fn ensure_placement_record_fits(&self) -> Result<(), String> {
        // Expected metadata for a byte-size budget ONLY, never observed file
        // evidence, a stored intent, a download receipt or execution permission.
        let manifest_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?)
        );
        let files = self
            .manifest
            .files
            .iter()
            .map(|file| crate::model_inspection::FileReport {
                relative_path: file.relative_path.clone(),
                role: file.role,
                actual_size: file.expected_size.unwrap(),
                actual_hash: file.expected_hash.clone().unwrap(),
                expected_size_checked: true,
                expected_hash_checked: true,
            })
            .collect::<Vec<_>>();
        let report = crate::model_inspection::Report {
            review_hash: crate::model_inspection::review_hash(
                &self.manifest,
                &manifest_hash,
                &files,
            )?,
            manifest: self.manifest.clone(),
            manifest_hash,
            files,
            total_bytes: 0,
            engine_compatibility: "not_verified",
            publisher_authenticity: "not_verified",
            installed: false,
            ready: false,
        };
        let placeholder = uuid::Uuid::nil().to_string();
        let record = crate::model_placement::PlacementRecord::downloaded(
            &placeholder,
            &report,
            self.origin(&placeholder)?,
        )?;
        if serde_json::to_vec(&record)
            .map_err(|e| e.to_string())?
            .len()
            > 65_536
        {
            return Err(
                "将来の配置記録が64KiBを超えます。取得前に構成・長いパスやURLを見直してください"
                    .into(),
            );
        }
        Ok(())
    }
    pub(crate) fn bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "ダウンロード定義を符号化できません")?;
        if bytes.len() > MAX_RECIPE {
            return Err("ダウンロード定義が64KiBを超えています".into());
        }
        Ok(bytes)
    }
    pub(crate) fn hash(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.bytes()?)))
    }
    pub(crate) fn validate(&self) -> Result<u64, String> {
        self.validate_for(Target::current()?)
    }
    fn validate_for(&self, target: Target) -> Result<u64, String> {
        if self.schema_version != 4 {
            return Err("ダウンロード定義の形式版が未対応です".into());
        }
        self.manifest.validate(target)?;
        if self.sources.len() != self.manifest.files.len() {
            return Err("モデルファイルと配布元の構成が一致しません".into());
        }
        let mut total = 0_u64;
        let mut archives = std::collections::BTreeMap::new();
        for (file, source) in self.manifest.files.iter().zip(&self.sources) {
            if file.relative_path != source.relative_path
                || file.expected_size.is_none()
                || file.expected_hash.is_none()
            {
                return Err("全ダウンロードファイルに同順の配布元・容量・SHA-256が必要です".into());
            }
            if source.url == BUNDLED_WHISPER {
                if self.manifest.engine != crate::model_inspection::Engine::Whisper
                    || self.manifest.target != Target::MacosAarch64
                    || self.manifest.engine_version != "v1.9.4"
                    || source.archive.is_none()
                    || !matches!(
                        (
                            file.role,
                            source.archive.as_ref().map(|a| a.member_path.as_str())
                        ),
                        (
                            crate::model_inspection::Role::Executable,
                            Some("bin/whisper-cli")
                        ) | (
                            crate::model_inspection::Role::Notice,
                            Some("notices/whisper-MIT.txt" | "build-info.json")
                        )
                    )
                {
                    return Err("同梱実行ファイルの識別子・版・対象・構成が不正です".into());
                }
            } else if source.url == crate::model_asr_catalog::URI {
                if self.manifest.engine != crate::model_inspection::Engine::Asr
                    || self.manifest.target != Target::MacosAarch64
                    || source.archive.is_none()
                    || !matches!(
                        file.role,
                        crate::model_inspection::Role::Executable
                            | crate::model_inspection::Role::Notice
                    )
                {
                    return Err("ASR同梱素材の対象・構成が不正です".into());
                }
            } else if source.url == BUNDLED_WINDOWS {
                if self.manifest.target != Target::WindowsX86_64
                    || source.archive.is_none()
                    || !matches!(
                        file.role,
                        crate::model_inspection::Role::Executable
                            | crate::model_inspection::Role::RuntimeLibrary
                            | crate::model_inspection::Role::Notice
                    )
                {
                    return Err("Windows同梱素材の対象・構成が不正です".into());
                }
            } else {
                validate_url(&source.url)?;
            }
            if let Some(archive) = &source.archive {
                if archive.format != "tar"
                    || archive.expected_size == 0
                    || archive.expected_size > MAX_TOTAL
                    || !crate::model_inspection::relative(&archive.member_path)
                    || !crate::model_download_ipc::hash(&archive.expected_hash)
                {
                    return Err("アーカイブの形式・容量・SHA・対象パスが不正です".into());
                }
                let metadata = (archive.expected_size, &archive.expected_hash);
                if archives
                    .insert(&source.url, metadata)
                    .is_some_and(|previous| previous != metadata)
                {
                    return Err("同じアーカイブURLの容量・SHAが一致しません".into());
                }
            }
            total = total
                .checked_add(file.expected_size.unwrap())
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or("ダウンロード合計容量の上限は32GiBです")?;
        }
        if self.sources.iter().any(|s| s.url == BUNDLED_WHISPER)
            && !self.manifest.files.iter().zip(&self.sources).any(|(f, s)| {
                f.role == crate::model_inspection::Role::Asr && s.url.starts_with("https://")
            })
        {
            return Err("同梱実行ファイルにはHTTPSの文字起こしモデルが必要です".into());
        }
        for (index, source) in self.sources.iter().enumerate() {
            if source.archive.is_none() && archives.contains_key(&source.url) {
                return Err("同じURLを通常ファイルとアーカイブには指定できません".into());
            }
            if source.archive.is_some()
                && self.sources[..index].iter().any(|previous| {
                    previous.url == source.url
                        && previous.archive.as_ref().map(|a| &a.member_path)
                            == source.archive.as_ref().map(|a| &a.member_path)
                })
            {
                return Err("同じアーカイブ内ファイルを重複指定できません".into());
            }
        }
        self.transfer_bytes()?;
        self.archive_bytes()?;
        self.bundled_bytes()?;
        self.bytes()?;
        Ok(total)
    }
    pub(crate) fn transfer_bytes(&self) -> Result<u64, String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0_u64;
        for (file, source) in self.manifest.files.iter().zip(&self.sources) {
            if is_bundled(&source.url) {
                continue;
            }
            let size = match &source.archive {
                Some(archive) if seen.insert(&source.url) => archive.expected_size,
                Some(_) => 0,
                None => file.expected_size.ok_or("取得容量がありません")?,
            };
            total = total
                .checked_add(size)
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or("通信容量の上限は32GiBです")?;
        }
        Ok(total)
    }
    pub(crate) fn has_bundled(&self) -> bool {
        self.sources.iter().any(|s| is_bundled(&s.url))
    }
    pub(crate) fn bundled_bytes(&self) -> Result<u64, String> {
        Ok(self
            .sources
            .iter()
            .find(|s| is_bundled(&s.url))
            .and_then(|s| s.archive.as_ref())
            .map_or(0, |a| a.expected_size))
    }
    pub(crate) fn archive_bytes(&self) -> Result<u64, String> {
        let mut seen = std::collections::BTreeSet::new();
        self.sources
            .iter()
            .filter_map(|source| source.archive.as_ref().filter(|_| seen.insert(&source.url)))
            .try_fold(0_u64, |n, a| {
                n.checked_add(a.expected_size)
                    .filter(|n| *n <= MAX_TOTAL)
                    .ok_or_else(|| "アーカイブ容量の上限は32GiBです".into())
            })
    }
    pub(crate) fn origin(&self, operation_id: &str) -> Result<DownloadOrigin, String> {
        Ok(DownloadOrigin {
            operation_id: operation_id.into(),
            recipe_hash: self.hash()?,
            sources: self.sources.clone(),
            recipe_schema_version: self.schema_version,
        })
    }
}

/// The already-stored manifest completes this recipe; no machine paths or
/// execution capabilities travel.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DownloadOrigin {
    operation_id: String,
    recipe_hash: String,
    sources: Vec<DownloadSource>,
    recipe_schema_version: u32,
}
impl DownloadOrigin {
    pub(crate) fn matches(&self, operation: &str, recipe: &str) -> bool {
        self.operation_id == operation && self.recipe_hash == recipe
    }
    pub(crate) fn identity(&self) -> (&str, &str) {
        (&self.operation_id, &self.recipe_hash)
    }
    pub(crate) fn validate(
        &self,
        manifest: &PackageManifest,
        manifest_hash: &str,
    ) -> Result<(), String> {
        if !uuid::Uuid::parse_str(&self.operation_id)
            .is_ok_and(|id| id.to_string() == self.operation_id)
        {
            return Err("ダウンロード由来の操作IDが不正です".into());
        }
        let recipe = Recipe {
            schema_version: self.recipe_schema_version,
            manifest: manifest.clone(),
            sources: self.sources.clone(),
        };
        // Portable evidence validation must not depend on this host's target.
        recipe.validate_for(manifest.target)?;
        let canonical = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
        if recipe.hash()? != self.recipe_hash
            || format!("{:x}", Sha256::digest(canonical)) != manifest_hash
        {
            return Err("ダウンロード由来の定義・manifest・SHAが一致しません".into());
        }
        Ok(())
    }
}

pub(crate) fn validate_url(value: &str) -> Result<(), String> {
    // Initial recipes reject credentials/tokens; HTTPS CDN redirects may carry
    // signed queries internally. This does not authenticate the publisher.
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
