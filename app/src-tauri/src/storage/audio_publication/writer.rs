//! Durable ownership of the single output. Resume a partial write only when
//! its source key and physical identity match the fsynced reservation.
use super::super::publication_files::{Directory, Identity, Stamp};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

const RECEIPT: &str = "wave-publication.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Plan {
    version: u32,
    key: String,
    frames: u64,
    name: String,
    identity: Identity,
    hash: Option<String>,
}
pub(super) struct Pending {
    directory: Directory,
    plan: Plan,
    stamp: Stamp,
}
impl Pending {
    pub fn open(path: &Path, key: &str, frames: u64) -> Result<Self, String> {
        let directory = Directory::open(path)?;
        let (plan, stamp) = if let Some((bytes, stamp)) = directory.read_json(RECEIPT)? {
            let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            (plan, stamp)
        } else {
            let name = format!("{}.wav", uuid::Uuid::new_v4());
            let file = directory.create(&format!(".{name}.part"))?;
            let identity = Identity::file(&file)?;
            file.sync_all().map_err(|e| e.to_string())?;
            let plan = Plan {
                version: 1,
                key: key.into(),
                frames,
                name,
                identity,
                hash: None,
            };
            directory.json(RECEIPT, &plan, None)?;
            let stamp = directory
                .read_json(RECEIPT)?
                .ok_or("WAV保存予約がありません")?
                .1;
            (plan, stamp)
        };
        if plan.version != 1
            || plan.key != key
            || plan.frames != frames
            || plan.frames == 0
            || !plan.name.ends_with(".wav")
            || uuid::Uuid::parse_str(plan.name.trim_end_matches(".wav"))
                .map_or(true, |id| format!("{id}.wav") != plan.name)
        {
            return Err("WAV保存予約と入力音声が一致しません".into());
        }
        let pending = Self {
            directory,
            plan,
            stamp,
        };
        if pending.plan.hash.is_some() {
            pending.verify_completed()?;
        }
        Ok(pending)
    }
    fn part(&self) -> String {
        format!(".{}.part", self.plan.name)
    }
    pub fn begin(&self) -> Result<Option<File>, String> {
        if self.plan.hash.is_some() {
            return Ok(None);
        }
        if self.directory.optional(&self.plan.name, false)?.is_some() {
            return Err("未確定WAVに最終ファイルがあります".into());
        }
        let mut file = self.directory.required(&self.part(), true)?;
        self.plan.identity.require(&file)?;
        self.stamp
            .require(&self.directory.required(RECEIPT, false)?)?;
        file.set_len(0).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Ok(Some(file))
    }
    fn verify_file(&self, file: &mut File, expected: &str) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        self.plan.identity.require(file)?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let layout = crate::audio::wave::inspect(file, size)?;
        if layout.frames != self.plan.frames {
            return Err("保存WAVの長さが一致しません".into());
        }
        file.rewind().map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        if format!("{:x}", hash.finalize()) != expected {
            return Err("保存WAVのPCMチェックサムが一致しません".into());
        }
        self.plan.identity.require(file)
    }
    fn verify_completed(&self) -> Result<(), String> {
        let final_file = self.directory.optional(&self.plan.name, false)?;
        let part = self.directory.optional(&self.part(), false)?;
        if final_file.is_some() && part.is_some() {
            return Err("保存WAVの途中名と最終名が重複しています".into());
        }
        let mut file = final_file.or(part).ok_or("保存WAVの実体がありません")?;
        self.verify_file(
            &mut file,
            self.plan.hash.as_deref().ok_or("WAV確定証拠がありません")?,
        )
    }
    pub fn complete(&mut self, hash: String) -> Result<(), String> {
        if let Some(old) = &self.plan.hash {
            if *old != hash {
                return Err("WAV確定ハッシュが変更されています".into());
            }
        } else {
            self.verify_file(&mut self.directory.required(&self.part(), false)?, &hash)?;
            self.plan.hash = Some(hash);
            self.directory
                .json(RECEIPT, &self.plan, Some(&self.stamp))?;
            self.stamp = self
                .directory
                .read_json(RECEIPT)?
                .ok_or("WAV確定証拠がありません")?
                .1;
        }
        if self.directory.optional(&self.plan.name, false)?.is_none() {
            self.directory.rename(&self.part(), &self.plan.name, true)?;
        }
        self.verify_completed()
    }
    pub fn completed_hash(&self) -> Option<&str> {
        self.plan.hash.as_deref()
    }
    pub fn path(&self) -> PathBuf {
        self.directory.path.join(&self.plan.name)
    }
}
