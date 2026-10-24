//! Single-writer, atomic Windows notification ledger. The directory and lock
//! deny rename/replacement; a failed commit permanently fences this writer.
use crate::windows_files as win;
use sha2::{Digest, Sha256};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};
const LIMIT: u64 = 128 * 1024;
const DATA: &str = "input-notices.json";
type Stamp = ((u32, u64, u32), u64, u64, String);
fn read(path: &std::path::Path) -> Result<Option<(Stamp, Vec<u8>)>, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        _ => (),
    }
    let mut file = win::read(path, false)?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.len() > LIMIT {
        return Err("通知台帳の容量が不正です".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != metadata.len() {
        return Err("通知台帳が読込み中に変更されました".into());
    }
    let stamp = (
        win::identity(&file)?,
        metadata.len(),
        metadata.last_write_time(),
        format!("{:x}", Sha256::digest(&bytes)),
    );
    Ok(Some((stamp, bytes)))
}
pub(in crate::input_status::queue::ledger) struct Store {
    directory: File,
    path: PathBuf,
    lock: File,
    identity: (u32, u64, u32),
    expected: Option<Stamp>,
    failed: Option<String>,
}
impl Store {
    pub(in crate::input_status::queue::ledger) fn open(
        directory: &File,
    ) -> Result<(Self, Option<Vec<u8>>), String> {
        let directory = directory.try_clone().map_err(|e| e.to_string())?;
        let path = win::path(&directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(0x00200000 | 0x80000000)
            .open(path.join("input-notices.lock"))
            .map_err(|e| format!("通知台帳の書込み所有を確保できません: {e}"))?;
        let identity = win::identity(&lock)?;
        if identity.2 != 1 || lock.metadata().map_err(|e| e.to_string())?.len() != 0 {
            return Err("通知台帳のロック実体が不正です".into());
        }
        lock.sync_all().map_err(|e| e.to_string())?;
        let initial = read(&path.join(DATA))?;
        let expected = initial.as_ref().map(|(stamp, _)| stamp.clone());
        Ok((
            Self {
                directory,
                path,
                lock,
                identity,
                expected,
                failed: None,
            },
            initial.map(|(_, bytes)| bytes),
        ))
    }
    fn verify_inner(&self) -> Result<(), String> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if win::path(&self.directory)? != self.path
            || win::identity(&self.lock)? != self.identity
            || read(&self.path.join(DATA))?.map(|(stamp, _)| stamp) != self.expected
        {
            return Err("通知台帳または保存先が変更されました".into());
        }
        Ok(())
    }
    pub(in crate::input_status::queue::ledger) fn verify(&mut self) -> Result<(), String> {
        let result = self.verify_inner();
        if let Err(error) = &result {
            self.failed = Some(error.clone());
        }
        result
    }
    pub(in crate::input_status::queue::ledger) fn commit(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), String> {
        let result = (|| {
            self.verify()?;
            if bytes.is_empty() || bytes.len() as u64 > LIMIT {
                return Err("通知台帳の保存容量が不正です".into());
            }
            let mut temporary = tempfile::Builder::new()
                .prefix(".input-notices-")
                .tempfile_in(&self.path)
                .map_err(|e| e.to_string())?;
            temporary.write_all(bytes).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            self.verify()?;
            temporary
                .persist(self.path.join(DATA))
                .map_err(|e| e.error.to_string())?
                .sync_all()
                .map_err(|e| e.to_string())?;
            let (stamp, saved) = read(&self.path.join(DATA))?.ok_or("通知台帳がありません")?;
            if saved != bytes {
                return Err("通知台帳の確定内容が一致しません".into());
            }
            self.expected = Some(stamp);
            self.verify_inner()
        })();
        if let Err(error) = &result {
            self.failed = Some(format!(
                "通知台帳の確定に失敗しました。再起動後に確認してください: {error}"
            ));
        }
        result
    }
}
