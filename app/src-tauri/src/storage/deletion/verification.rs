//! Operation-local content proofs. Windows pins deny writes but allow our
//! journaled rename/unlink; other platforms always perform the complete hash.
use super::fs::{Directory, Proof};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::VecDeque, fs::File, io::Read};

struct Entry {
    proof: Proof,
    sha: String,
    _pin: File,
}
thread_local! { static CACHE: RefCell<Option<VecDeque<Entry>>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<VecDeque<Entry>>);
impl Scope {
    pub(crate) fn enter() -> Self {
        Self(CACHE.with(|c| c.replace(Some(VecDeque::new()))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CACHE.with(|c| {
            c.replace(self.0.take());
        });
    }
}
pub(crate) fn release_pins() {
    CACHE.with(|c| {
        if let Some(entries) = c.borrow_mut().as_mut() {
            entries.clear();
        }
    });
}
pub(super) fn hash(
    dir: &Directory,
    leaf: &str,
    check: &impl Fn() -> bool,
) -> Result<(Proof, String), String> {
    cancel(check)?;
    #[cfg(windows)]
    let (mut file, protected) = match dir.protected_regular(leaf) {
        Ok(file) => (file, true),
        Err(_) => (dir.regular(leaf)?, false),
    };
    #[cfg(not(windows))]
    let (mut file, protected) = (dir.regular(leaf)?, false);
    let before = Proof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
    before.validate(false)?;
    if protected {
        let found = CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|v| v.iter().find(|e| e.proof == before).map(|e| e.sha.clone()))
        });
        if let Some(sha) = found {
            dir.verify_file(leaf, &file)?;
            return Ok((before, sha));
        }
    }
    crate::operation_observation::file_check();
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0; 128 * 1024];
    loop {
        cancel(check)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("検査容量が範囲外です")?;
        if size > before.size {
            return Err("検査中にファイルが拡張されました".into());
        }
        hash.update(&buffer[..n]);
        crate::operation_observation::read(n);
    }
    dir.verify_file(leaf, &file)?;
    if size != before.size
        || Proof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)? != before
    {
        return Err("検査中にファイルが変更されました".into());
    }
    let sha = format!("{:x}", hash.finalize());
    if protected {
        CACHE.with(|c| {
            if let Some(entries) = c.borrow_mut().as_mut() {
                if entries.len() == 64 {
                    entries.pop_front();
                }
                entries.push_back(Entry {
                    proof: before.clone(),
                    sha: sha.clone(),
                    _pin: file,
                });
            }
        });
    }
    Ok((before, sha))
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("ファイル検査を停止しました".into())
    } else {
        Ok(())
    }
}
