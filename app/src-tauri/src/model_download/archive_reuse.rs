//! Reuse retained, completed download archives without creating a second cache.
//! Both the old recipe evidence and actual archive bytes must match. A cache
//! miss never mutates the old operation or makes incomplete model data ready.
use super::{check, Completion, Directory, Intent, Source, COMPLETE, INTENT, MAX_RECIPE};
use crate::model_download_recipe::RecipeExt;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_OPERATIONS: usize = 4096;
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MIN_ARCHIVE_READ_BUDGET: u64 = 64 * 1024 * 1024;

struct Candidate {
    archive: Source,
    evidence: Vec<Source>,
    directories: Vec<Directory>,
}

fn record<T: DeserializeOwned>(
    directory: &Path,
    name: &str,
    limit: usize,
    remaining: &mut u64,
) -> Result<(T, Source), String> {
    let mut source = Source::open(directory, name)?;
    let size = source.file.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > limit as u64 || size > *remaining {
        return Err("再利用する取得記録の容量が範囲外です".into());
    }
    *remaining -= size;
    let mut bytes = Vec::new();
    Read::by_ref(&mut source.file)
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    source.verify()?;
    if bytes.len() as u64 != size {
        return Err("再利用する取得記録の容量が変わっています".into());
    }
    let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok((value, source))
}

impl Candidate {
    fn open(
        root: &Path,
        operation_id: &str,
        url: &str,
        size: u64,
        hash: &str,
        remaining: &mut u64,
    ) -> Result<Self, String> {
        let operation = Directory::open_existing(&root.join(operation_id))?;
        // Missing completion is a miss. Never adopt interrupted archive writes.
        let (completion, completion_proof): (Completion, _) =
            record(operation.path(), COMPLETE, 1024, remaining)?;
        let (intent, intent_proof): (Intent, _) =
            record(operation.path(), INTENT, MAX_RECIPE + 1024, remaining)?;
        if intent.schema_version != 1
            || completion.schema_version != 1
            || intent.operation_id != operation_id
            || completion.operation_id != operation_id
            || completion.recipe_hash != intent.recipe_hash
            || !crate::model_download_ipc::hash(&completion.review_hash)
        {
            return Err("再利用する取得記録が一致しません".into());
        }
        // stage names archives by the FIRST source index in the OLD recipe.
        // Member paths deliberately do not participate in archive identity.
        let index = intent
            .recipe
            .sources
            .iter()
            .position(|source| {
                source.url == url
                    && source.archive.as_ref().is_some_and(|archive| {
                        archive.expected_size == size && archive.expected_hash == hash
                    })
            })
            .ok_or("一致する保存済みアーカイブがありません")?;
        if intent.recipe.hash()? != intent.recipe_hash
            || intent.recipe.validate_for(intent.recipe.manifest.target)? != completion.total_bytes
        {
            return Err("再利用する取得定義のSHA・容量が一致しません".into());
        }
        let archives = Directory::open_existing(&operation.path().join("archives"))?;
        let archive = Source::open(archives.path(), &format!("source-{index}.tar"))?;
        if archive.file.metadata().map_err(|e| e.to_string())?.len() != size {
            return Err("保存済みアーカイブの容量が一致しません".into());
        }
        let candidate = Self {
            archive,
            evidence: vec![intent_proof, completion_proof],
            directories: vec![operation, archives],
        };
        candidate.verify()?;
        Ok(candidate)
    }

    fn verify(&self) -> Result<(), String> {
        self.archive.verify()?;
        for evidence in &self.evidence {
            evidence.verify()?;
        }
        for directory in &self.directories {
            directory.verify()?;
        }
        Ok(())
    }

    // Only destination errors and cancellation abort preparation. Untrusted
    // cache read/identity/hash failures are misses and are rolled back by caller.
    fn copy(
        &mut self,
        size: u64,
        hash: &str,
        output: &mut File,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            check(&cancelled)?;
            let n = match self.archive.file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => return Ok(false),
            };
            count += n as u64;
            if count > size {
                return Ok(false);
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            digest.update(&buffer[..n]);
        }
        check(&cancelled)?;
        Ok(count == size
            && mon_core::hash::sha256_output_hex(digest.finalize()) == hash
            && self.verify().is_ok())
    }
}

pub(super) fn try_copy(
    staging_root: &Path,
    current_operation: &str,
    url: &str,
    size: u64,
    hash: &str,
    output: &mut File,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
) -> Result<bool, String> {
    check(&cancelled)?;
    let root = match Directory::open_existing(staging_root) {
        Ok(root) => root,
        Err(_) => return Ok(false),
    };
    let entries = match std::fs::read_dir(root.path()) {
        Ok(entries) => entries,
        Err(_) => return Ok(false),
    };
    let mut operations = Vec::new();
    for (count, entry) in entries.enumerate() {
        check(&cancelled)?;
        if count >= MAX_OPERATIONS {
            return Ok(false);
        }
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name != current_operation
            && uuid::Uuid::parse_str(&name).is_ok_and(|id| id.to_string() == name)
        {
            operations.push(name);
        }
    }
    operations.sort();
    let mut remaining_metadata = MAX_METADATA_BYTES;
    let mut remaining_archive = size.max(MIN_ARCHIVE_READ_BUDGET);
    for operation in operations {
        check(&cancelled)?;
        if remaining_metadata == 0 || remaining_archive < size {
            break;
        }
        let Ok(mut candidate) = Candidate::open(
            root.path(),
            &operation,
            url,
            size,
            hash,
            &mut remaining_metadata,
        ) else {
            continue;
        };
        remaining_archive -= size;
        if candidate.copy(size, hash, output, cancelled)? {
            root.verify()?;
            // Count copied bytes only after all cache evidence succeeds, so a
            // cache miss never publishes progress that network fallback resets.
            progress(size);
            return Ok(true);
        }
        output.set_len(0).map_err(|e| e.to_string())?;
        output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    }
    check(&cancelled)?;
    Ok(false)
}
