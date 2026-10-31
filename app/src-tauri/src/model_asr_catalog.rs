//! Catalog assembly binds app-built runtime bytes separately from fixed weights.
use crate::model_download_recipe::RecipeExt;
use crate::{
    model_download_recipe::{ArchiveMember, DownloadSource, Recipe},
    model_inspection::{PackageFile, Source},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
};
pub(crate) const URI: &str = "mon-bundled://asr-v1/asr-runtime.tar";
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Index {
    schema_version: u32,
    archive_bytes: u64,
    archive_sha256: String,
    files: Vec<PackageFile>,
    #[serde(default)]
    model_assets: BTreeMap<String, Vec<PackageFile>>,
}
#[derive(Deserialize)]
pub(crate) struct Choice {
    pub id: String,
    pub label: String,
    pub description: String,
    pub recipe: Recipe,
}
pub(crate) fn options(root: &Path) -> Result<Vec<Choice>, String> {
    let mut source = Source::open(root, "asr-index.json")?;
    let mut bytes = Vec::new();
    (&mut source.file)
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    source.verify()?;
    if bytes.len() > 65536 {
        return Err("ASRランタイム定義が上限を超えています".into());
    }
    let index: Index = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if index.schema_version != 1
        || index.files.len() != 2
        || index.model_assets.len() > 32
        || index.archive_bytes > 128 * 1024 * 1024
        || !crate::model_download_ipc::hash(&index.archive_sha256)
    {
        return Err("ASRランタイム定義が不正です".into());
    }
    let mut choices: Vec<Choice> =
        serde_json::from_str(include_str!("../../model-catalog/asr-transducer.json"))
            .map_err(|e| e.to_string())?;
    // Legacy bundles still provide their original choices. A model requiring
    // a new bundled asset is offered only by a bundle declaring that asset.
    choices.retain(|choice| {
        !choice.recipe.sources.iter().any(|s| s.url == URI)
            || index.model_assets.contains_key(&choice.id)
    });
    for choice in &mut choices {
        let assets = index
            .model_assets
            .get(&choice.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if assets.len() > 16 {
            return Err("ASRモデル別素材の件数が不正です".into());
        }
        let mut used = std::collections::BTreeSet::new();
        for source in &mut choice.recipe.sources {
            if source.url != URI {
                continue;
            }
            let file = choice
                .recipe
                .manifest
                .files
                .iter()
                .find(|f| f.relative_path == source.relative_path)
                .ok_or("ASRモデル別素材の宣言がありません")?;
            let expected_member = format!("assets/{}/{}", choice.id, source.relative_path);
            if !used.insert(source.relative_path.clone())
                || file.role != crate::model_inspection::Role::AsrAsset
                || !assets.iter().any(|asset| asset == file)
                || source
                    .archive
                    .as_ref()
                    .is_none_or(|a| a.member_path != expected_member)
            {
                return Err("ASRモデル別素材の容量・SHA・宣言が一致しません".into());
            }
            source.archive = Some(ArchiveMember {
                format: "tar".into(),
                expected_size: index.archive_bytes,
                expected_hash: index.archive_sha256.clone(),
                member_path: expected_member,
            });
        }
        if used.len() != assets.len() {
            return Err("ASRモデル別素材に未参照のファイルがあります".into());
        }
        for file in index.files.iter().rev() {
            choice.recipe.manifest.files.insert(0, file.clone());
            choice.recipe.sources.insert(
                0,
                DownloadSource {
                    relative_path: file.relative_path.clone(),
                    url: URI.into(),
                    archive: Some(ArchiveMember {
                        format: "tar".into(),
                        expected_size: index.archive_bytes,
                        expected_hash: index.archive_sha256.clone(),
                        member_path: file.relative_path.clone(),
                    }),
                },
            );
        }
        choice.recipe.review()?;
    }
    Ok(choices)
}
pub(crate) fn validate(root: &Path, recipe: &Recipe) -> Result<(), String> {
    if !options(root)?.iter().any(|c| &c.recipe == recipe) {
        return Err("アプリ同梱ASRモデル定義と一致しません".into());
    }
    Ok(())
}
pub(crate) fn fetch(
    root: &Path,
    size: u64,
    hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let mut source = Source::open(root, "asr-runtime.tar")?;
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        if cancelled() {
            return Err("ASRランタイムの準備を中断しました".into());
        }
        let n = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > size {
            return Err("ASRランタイム容量が不正です".into());
        }
        digest.update(&buffer[..n]);
        output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
    }
    source.verify()?;
    if count != size || format!("{:x}", digest.finalize()) != hash {
        return Err("ASRランタイムの容量・SHAが不正です".into());
    }
    Ok(())
}
