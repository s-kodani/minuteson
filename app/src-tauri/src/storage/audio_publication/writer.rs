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
pub(in crate::storage) struct Pending {
    directory: Directory,
    plan: Plan,
    stamp: Stamp,
    verified: Option<Stamp>,
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
        let mut pending = Self {
            directory,
            plan,
            stamp,
            verified: None,
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
    fn verify_file(&self, file: &mut File, expected: &str) -> Result<Stamp, String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let stamp = Stamp::read(file)?;
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
        stamp.require(file)?;
        Ok(stamp)
    }
    fn verify_completed(&mut self) -> Result<(), String> {
        let final_file = self.directory.optional(&self.plan.name, false)?;
        let part = self.directory.optional(&self.part(), false)?;
        if final_file.is_some() && part.is_some() {
            return Err("保存WAVの途中名と最終名が重複しています".into());
        }
        let mut file = final_file.or(part).ok_or("保存WAVの実体がありません")?;
        let stamp = self.verify_file(
            &mut file,
            self.plan.hash.as_deref().ok_or("WAV確定証拠がありません")?,
        )?;
        self.verified = Some(stamp);
        Ok(())
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
    /// Bind a reopened final descriptor to the exact bytes already checked by
    /// completion. Never adopt a fresh stamp from an unverified replacement.
    pub fn completed_file(&self) -> Result<File, String> {
        let stamp = self
            .verified
            .as_ref()
            .ok_or("保存WAVの検証結果がありません")?;
        let file = self.directory.required(&self.plan.name, false)?;
        stamp.require(&file)?;
        Ok(file)
    }
    pub fn path(&self) -> PathBuf {
        self.directory.path.join(&self.plan.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn completed() -> (tempfile::TempDir, Pending, Vec<u8>) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let mut pending = Pending::open(&directory, "owned-wav", 2).unwrap();
        let mut bytes = crate::audio::wave::streaming_header(2).unwrap().to_vec();
        bytes.extend([1, 0, 2, 0]);
        let mut file = pending.begin().unwrap().unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
        drop(file);
        pending
            .complete(format!("{:x}", Sha256::digest(&bytes)))
            .unwrap();
        (root, pending, bytes)
    }

    #[test]
    fn completed_descriptor_remains_bound_after_reopen_but_not_after_payload_change() {
        let (root, pending, _) = completed();
        pending.completed_file().unwrap();
        let resumed = Pending::open(&root.path().canonicalize().unwrap(), "owned-wav", 2).unwrap();
        resumed.completed_file().unwrap();
        let mut changed = std::fs::OpenOptions::new()
            .append(true)
            .open(pending.path())
            .unwrap();
        changed.write_all(&[3]).unwrap();
        changed.sync_all().unwrap();
        drop(changed);
        assert!(pending.completed_file().is_err());
        assert!(resumed.completed_file().is_err());
    }

    #[test]
    fn completed_descriptor_does_not_adopt_an_identical_replacement() {
        let (_root, pending, bytes) = completed();
        let path = pending.path();
        std::fs::rename(&path, path.with_extension("held")).unwrap();
        let mut replacement = pending.directory.create(&pending.plan.name).unwrap();
        replacement.write_all(&bytes).unwrap();
        replacement.sync_all().unwrap();
        drop(replacement);
        assert!(pending.completed_file().is_err());
    }
}
