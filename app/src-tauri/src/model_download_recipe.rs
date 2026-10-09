//! Portable download provenance. Validation is not publisher authentication or
//! permission to download/execute on the current host.
use crate::model_inspection::{PackageManifest, Target};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_RECIPE: usize = 65_536;
pub(crate) const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;

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
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Review {
    pub(crate) recipe_hash: String,
    pub(crate) manifest: PackageManifest,
    pub(crate) sources: Vec<DownloadSource>,
    pub(crate) total_bytes: u64,
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
        Ok(Review {
            recipe_hash: self.hash()?,
            manifest: self.manifest.clone(),
            sources: self.sources.clone(),
            total_bytes: self.validate()?,
            publisher_authenticity: "not_verified",
            installed: false,
            ready: false,
        })
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
        if self.schema_version != 1 {
            return Err("ダウンロード定義の形式版が未対応です".into());
        }
        self.manifest.validate(target)?;
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
    pub(crate) fn origin(&self, operation_id: &str) -> Result<DownloadOrigin, String> {
        Ok(DownloadOrigin {
            operation_id: operation_id.into(),
            recipe_hash: self.hash()?,
            sources: self.sources.clone(),
        })
    }
}

/// Included only in version-2 placement records. The already-stored manifest
/// completes this recipe; no machine paths or execution capabilities travel.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DownloadOrigin {
    operation_id: String,
    recipe_hash: String,
    sources: Vec<DownloadSource>,
}
impl DownloadOrigin {
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
            schema_version: 1,
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
