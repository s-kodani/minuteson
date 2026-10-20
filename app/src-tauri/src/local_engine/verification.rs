//! Job-owned verification evidence. Directory inventories are still checked by
//! their callers; only physical-file hashing is shared. Nothing survives a job.
use super::{Fingerprint, StableFile};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek},
    path::{Path, PathBuf},
};
thread_local! { static CACHE: RefCell<Option<Cache>> = const { RefCell::new(None) }; }
#[derive(Default)]
struct Cache {
    entries: BTreeMap<PathBuf, Entry>,
    hits: u64,
    bytes: u64,
    reused_bytes: u64,
    hash_ms: u128,
    inventories: BTreeMap<PathBuf, Vec<(PathBuf, String)>>,
}
struct Entry {
    proof: Fingerprint,
    watched: StableFile,
    audit: File,
    stamp: String,
}
pub(crate) struct Scope {
    enabled: bool,
    owns: bool,
    started: std::time::Instant,
    final_bytes: std::cell::Cell<u64>,
    final_ms: std::cell::Cell<u128>,
}
impl Scope {
    pub(crate) fn enter(enabled: bool) -> Result<Self, String> {
        if enabled {
            CACHE.with(|c| -> Result<(), String> {
                if c.borrow().is_some() {
                    return Err("ファイル検証セッションが重複しています".into());
                }
                *c.borrow_mut() = Some(Cache::default());
                Ok(())
            })?;
        }
        Ok(Self {
            enabled,
            owns: enabled,
            started: std::time::Instant::now(),
            final_bytes: std::cell::Cell::new(0),
            final_ms: std::cell::Cell::new(0),
        })
    }
    /// Admission helpers share the enclosing operation's evidence. Each
    /// commit still invokes finish(), which physically checks retained files.
    pub(crate) fn join_or_enter(enabled: bool) -> Result<Self, String> {
        if enabled && CACHE.with(|c| c.borrow().is_some()) {
            let mut scope = Self::enter(false)?;
            scope.enabled = true;
            return Ok(scope);
        }
        Self::enter(enabled)
    }
    pub(crate) fn finish(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        // Temporarily remove evidence so final validation physically reads all
        // retained files, even when nested model/package checks use this API.
        let cache = CACHE
            .with(|c| c.borrow_mut().take())
            .ok_or("検証セッションがありません")?;
        let final_started = std::time::Instant::now();
        let result = (|| {
            for (path, inventory) in &cache.inventories {
                if &inventory_of(path)? != inventory {
                    return Err("処理中にモデルパッケージの構成が変更されました".into());
                }
            }
            for e in cache.entries.values() {
                e.watched.verify()?;
                verify_audit(e)?;
                e.proof.verify(cancelled)?;
                e.watched.verify()?;
                verify_audit(e)?;
            }
            Ok(())
        })();
        self.final_ms.set(final_started.elapsed().as_millis());
        if result.is_ok() {
            self.final_bytes
                .set(cache.entries.values().map(|e| e.proof.size).sum());
        }
        CACHE.with(|c| *c.borrow_mut() = Some(cache));
        result
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.owns {
            if let Some(c) = CACHE.with(|c| c.borrow_mut().take()) {
                crate::internal_logs::detail(crate::internal_logs::current_context("analysis"),"stopped","info",self.started.elapsed(),&serde_json::json!({"event":"ファイル検証共有終了","fileCount":c.entries.len(),"verificationHits":c.hits,"physicalHashBytes":c.bytes+self.final_bytes.get(),"hashReusedBytes":c.reused_bytes,"initialHashMs":c.hash_ms,"finalVerificationMs":self.final_ms.get()}).to_string());
            }
        }
    }
}
fn verify_audit(e: &Entry) -> Result<(), String> {
    if crate::audio::reuse::stamp(&e.audit.metadata().map_err(|e| e.to_string())?) != e.stamp {
        return Err("処理中に検証済みファイルが変更されました".into());
    }
    Ok(())
}
pub(crate) fn hash(
    path: &Path,
    file: &mut File,
    cancelled: &impl Fn() -> bool,
    retain: bool,
) -> Result<String, String> {
    if cancelled() {
        return Err("ファイル照合を中断しました".into());
    }
    let before = file.metadata().map_err(|e| e.to_string())?;
    let identity = crate::audio::reuse::stamp(&before);
    let hit = CACHE.with(|c| -> Result<Option<String>, String> {
        let mut c = c.borrow_mut();
        let Some(c) = c.as_mut() else { return Ok(None) };
        if let Some(e) = c.entries.get(path) {
            e.watched.verify()?;
            verify_audit(e)?;
            if e.stamp != identity {
                return Err("検証済みファイルが差し替えられました".into());
            }
            let hash = e.proof.sha256.clone();
            c.hits += 1;
            c.reused_bytes += e.proof.size;
            return Ok(Some(hash));
        }
        Ok(None)
    })?;
    if let Some(h) = hit {
        return Ok(h);
    }
    let hash_started = std::time::Instant::now();
    file.rewind().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        if cancelled() {
            return Err("ファイル照合を中断しました".into());
        }
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let hash = format!("{:x}", hash.finalize());
    if crate::audio::reuse::stamp(&file.metadata().map_err(|e| e.to_string())?) != identity {
        return Err("照合中にファイルが変更されました".into());
    }
    CACHE.with(|c| -> Result<(), String> {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.bytes += before.len();
            c.hash_ms += hash_started.elapsed().as_millis();
            if retain && c.entries.len() < 128 {
                let proof = Fingerprint {
                    path: path.into(),
                    size: before.len(),
                    sha256: hash.clone(),
                };
                let watched = StableFile::watch(&proof)?;
                c.entries.insert(
                    path.into(),
                    Entry {
                        proof,
                        watched,
                        audit: file.try_clone().map_err(|e| e.to_string())?,
                        stamp: identity,
                    },
                );
            }
        }
        Ok(())
    })?;
    Ok(hash)
}

fn inventory_of(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut result = Vec::new();
    while let Some(path) = pending.pop() {
        let m = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if m.file_type().is_symlink() {
            return Err("モデルパッケージ内にリンクがあります".into());
        }
        result.push((path.clone(), crate::audio::reuse::stamp(&m)));
        if result.len() > 16384 {
            return Err("モデルパッケージの構成上限です".into());
        }
        if m.is_dir() {
            for e in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                pending.push(e.map_err(|e| e.to_string())?.path());
            }
        }
    }
    result.sort();
    Ok(result)
}
pub(crate) fn inventory(root: &Path) -> Result<(), String> {
    if !CACHE.with(|c| c.borrow().is_some()) {
        return Ok(());
    }
    let value = inventory_of(root)?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let c = c.as_mut().ok_or("検証セッションがありません")?;
        if let Some(old) = c.inventories.get(root) {
            if old != &value {
                return Err("処理中にモデルパッケージの構成が変更されました".into());
            }
        } else {
            c.inventories.insert(root.into(), value);
        }
        Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_shares_hashes_and_detects_restored_time_and_inventory_changes() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("model");
        std::fs::write(&p, b"original").unwrap();
        let proof = Fingerprint::capture(&p, &|| false).unwrap();
        let scope = Scope::enter(true).unwrap();
        inventory(t.path()).unwrap();
        proof.verify(&|| false).unwrap();
        proof.verify(&|| false).unwrap();
        assert_eq!(CACHE.with(|c| c.borrow().as_ref().unwrap().hits), 1);
        scope.finish(&|| false).unwrap();
        let old = p.metadata().unwrap().modified().unwrap();
        std::fs::write(&p, b"tampered").unwrap();
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        assert!(proof.verify(&|| false).is_err());
        assert!(scope.finish(&|| false).is_err());
        drop(scope);
        let scope = Scope::enter(true).unwrap();
        inventory(t.path()).unwrap();
        std::fs::write(t.path().join("unknown"), b"extra").unwrap();
        assert!(inventory(t.path()).is_err());
        assert!(scope.finish(&|| false).is_err());
    }
    #[test]
    fn temporary_capture_is_not_retained_and_legacy_scopes_remain_disabled() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("temporary");
        std::fs::write(&p, b"data").unwrap();
        let scope = Scope::enter(true).unwrap();
        Fingerprint::capture(&p, &|| false).unwrap();
        std::fs::remove_file(&p).unwrap();
        scope.finish(&|| false).unwrap();
        drop(scope);
        let _scope = Scope::enter(false).unwrap();
        assert!(CACHE.with(|c| c.borrow().is_none()));
    }
}
// Release evidence only for a completed, verified disposable input. Models and
// durable artifacts remain retained until the final job-wide physical check.
pub(crate) fn release(path: &Path) -> Result<(), String> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(cache) = c.as_mut() {
            if let Some(e) = cache.entries.get(&path) {
                e.watched.verify()?;
                verify_audit(e)?;
            }
            cache.entries.remove(&path);
        }
        Ok(())
    })
}
