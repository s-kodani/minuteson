//! Fixed Windows runtime resources. Only recipes shipped with this app may
//! reference the private runtime URI; callers cannot select arbitrary files.
use crate::{
    model_download_recipe::{Recipe, BUNDLED_WINDOWS},
    model_inspection::Source,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub(crate) fn resources() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let installed = exe
        .parent()
        .ok_or("アプリの場所がありません")?
        .join("windows-runtime");
    if installed.is_dir() {
        return installed.canonicalize().map_err(|e| e.to_string());
    }
    // Build-tree runs use the same packaged resource directory as the installer.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cache/windows-runtime/package")
        .canonicalize()
        .map_err(|_| "Windows推論ランタイムをビルドしてください".into())
}
pub(crate) fn catalog(root: &Path) -> Result<Vec<u8>, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut source = Source::open(&root, "catalog.json")?;
    let mut bytes = Vec::new();
    (&mut source.file)
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > 512 * 1024 {
        return Err("Windowsモデル一覧の容量が不正です".into());
    }
    source.verify()?;
    Ok(bytes)
}
pub(crate) fn validate_recipe(root: &Path, recipe: &Recipe) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Choice {
        recipe: Recipe,
    }
    let choices: Vec<Choice> =
        serde_json::from_slice(&catalog(root)?).map_err(|e| e.to_string())?;
    if !choices.iter().any(|c| c.recipe == *recipe) {
        return Err("アプリ同梱のWindowsモデル定義と一致しません".into());
    }
    recipe.review()?;
    Ok(())
}
pub(crate) fn fetch(
    root: &Path,
    size: u64,
    hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut source = Source::open(&root, "runtime.tar")?;
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        if cancelled() {
            return Err("同梱素材の準備を中断しました".into());
        }
        let n = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .filter(|n| *n <= size)
            .ok_or("同梱素材が確認容量を超えました")?;
        output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        digest.update(&buffer[..n]);
    }
    if count != size || format!("{:x}", digest.finalize()) != hash {
        return Err("同梱素材の容量・SHAが一致しません".into());
    }
    source.verify()?;
    output.sync_all().map_err(|e| e.to_string())
}
pub(crate) const URI: &str = BUNDLED_WINDOWS;
pub(crate) fn embedding() -> Result<PathBuf, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Index {
        schema_version: u32,
        name: String,
        bytes: u64,
        sha256: String,
    }
    let root = resources()?;
    let mut source = Source::open(&root, "embedding-index.json")?;
    let mut bytes = Vec::new();
    (&mut source.file)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("特徴量ヘルパーの定義が上限を超えています".into());
    }
    let index: Index = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if index.schema_version != 1 || index.name != "mon-speaker-embedding.exe" {
        return Err("特徴量ヘルパーの定義が不正です".into());
    }
    let path = root.join(index.name);
    let proof = crate::local_engine::Fingerprint::capture(&path, &|| false)?;
    if proof.size != index.bytes || proof.sha256 != index.sha256 {
        return Err("同梱特徴量ヘルパーの容量・SHAが一致しません".into());
    }
    source.verify()?;
    Ok(path)
}
