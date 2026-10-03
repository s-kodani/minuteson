//! Shared identity and lifetime guards for local native inference engines.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Child;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Fingerprint {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

impl Fingerprint {
    pub fn capture(path: &Path, cancelled: &impl Fn() -> bool) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("モデル・実行ファイルには絶対パスが必要です".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if path.to_str().is_none() {
            return Err("パスを文字列として保存できません".into());
        }
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() == 0 {
            return Err("モデル・実行ファイルが空または通常ファイルではありません".into());
        }
        let mut hash = Sha256::new();
        let mut bytes = [0; 65_536];
        loop {
            if cancelled() {
                return Err("ファイル照合を中断しました".into());
            }
            let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        let after = file.metadata().map_err(|e| e.to_string())?;
        if before.len() != after.len()
            || before.modified().map_err(|e| e.to_string())?
                != after.modified().map_err(|e| e.to_string())?
        {
            return Err("照合中にモデル・実行ファイルが変更されました".into());
        }
        Ok(Self {
            path,
            size: before.len(),
            sha256: format!("{:x}", hash.finalize()),
        })
    }

    pub fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if *self != Self::capture(&self.path, cancelled)? {
            return Err("固定したモデル・実行ファイルのハッシュが一致しません".into());
        }
        Ok(())
    }
}

pub(crate) struct ManagedChild(pub(crate) Child);
pub(crate) struct StableFile {
    path: PathBuf,
    metadata: std::fs::Metadata,
}

impl StableFile {
    pub(crate) fn watch(proof: &Fingerprint) -> Result<Self, String> {
        let metadata = std::fs::metadata(&proof.path).map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.len() != proof.size
            || proof.path.canonicalize().map_err(|e| e.to_string())? != proof.path
        {
            return Err("固定したモデル・実行ファイルが変更されています".into());
        }
        Ok(Self {
            path: proof.path.clone(),
            metadata,
        })
    }

    pub(crate) fn verify(&self) -> Result<(), String> {
        let current = std::fs::metadata(&self.path).map_err(|e| e.to_string())?;
        if current.len() != self.metadata.len()
            || current.modified().map_err(|e| e.to_string())?
                != self.metadata.modified().map_err(|e| e.to_string())?
        {
            return Err("実行中にモデル・実行ファイルが変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if current.dev() != self.metadata.dev() || current.ino() != self.metadata.ino() {
                return Err("実行中にモデル・実行ファイルが差し替えられました".into());
            }
        }
        Ok(())
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        // Dropping std::process::Child alone leaves a native worker alive.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
