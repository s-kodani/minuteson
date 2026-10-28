//! Reuse exact, non-executable model bytes from recorded app-managed packages.
use crate::model_inspection::{Role, Source};
use crate::model_placement::CatalogEntry;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(super) struct Candidate {
    pub url: String,
    root: PathBuf,
    relative: String,
}
pub(super) fn candidates(
    root: &Path,
    entries: &[CatalogEntry],
    recipe: &super::Recipe,
) -> Vec<Candidate> {
    let mut result = vec![];
    for (file, source) in recipe.manifest.files.iter().zip(&recipe.sources) {
        if !matches!(file.role, Role::Asr | Role::AsrAsset | Role::Vad) || source.archive.is_some()
        {
            continue;
        }
        if let Some(entry) = entries.iter().find(|entry| {
            entry.placement_recorded
                && !entry.body_removed
                && entry.source_type == "download"
                && entry.model_files.iter().any(|f| {
                    f.role == file.role
                        && Some(f.size) == file.expected_size
                        && Some(&f.sha256) == file.expected_hash.as_ref()
                })
                && root
                    .join(&entry.package_id)
                    .join(&file.relative_path)
                    .exists()
        }) {
            result.push(Candidate {
                url: source.url.clone(),
                root: root.join(&entry.package_id),
                relative: file.relative_path.clone(),
            });
        }
    }
    result
}
impl Candidate {
    pub(super) fn copy(
        &self,
        size: u64,
        hash: &str,
        output: &mut std::fs::File,
        cancelled: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let mut source = Source::open(&self.root, &self.relative)?;
        if source.file.metadata().map_err(|e| e.to_string())?.len() != size {
            return Err("再利用するモデルの容量が変わっています".into());
        }
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 65536];
        loop {
            if cancelled() {
                return Err("モデルの再利用を中断しました".into());
            }
            let n = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > size {
                return Err("再利用するモデルの容量が増大しました".into());
            }
            digest.update(&buffer[..n]);
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            progress(count);
        }
        source.verify()?;
        if count != size || format!("{:x}", digest.finalize()) != hash {
            return Err("再利用するモデルの容量・SHA256が一致しません".into());
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn asr_model_acquisition_reuses_weights_when_only_the_runtime_recipe_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let models = root.join("models");
        std::fs::create_dir(&models).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let package = models.join(&id);
        std::fs::create_dir_all(package.join("models")).unwrap();
        let weights = b"existing GGML bytes kept across adapter updates";
        let engine = b"new runtime bytes";
        let sha = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        std::fs::write(package.join("models/model.bin"), weights).unwrap();
        let recipe:super::super::Recipe=serde_json::from_value(serde_json::json!({"schemaVersion":4,"manifest":{
            "schemaVersion":1,"engine":"whisper.cpp","engineVersion":"v1.9.4","modelName":"Whisper test","modelVersion":"unchanged","quantization":null,"target":"macos_aarch64","licenseRef":"test",
            "files":[{"relativePath":"bin/engine","role":"executable","expectedSize":engine.len(),"expectedHash":sha(engine)},
                     {"relativePath":"models/model.bin","role":"asr","expectedSize":weights.len(),"expectedHash":sha(weights)}]},
            "sources":[{"relativePath":"bin/engine","url":"https://runtime.invalid/engine"},{"relativePath":"models/model.bin","url":"https://weights.invalid/model"}]})).unwrap();
        let entry = CatalogEntry {
            backend_id: None,
            package_id: id,
            source_type: "download".into(),
            download: None,
            model_name: "Whisper test".into(),
            model_version: "unchanged".into(),
            quantization: None,
            model_files: vec![crate::model_placement::ModelFileIdentity {
                role: Role::Asr,
                size: weights.len() as u64,
                sha256: sha(weights),
            }],
            engine: "whisper.cpp".into(),
            engine_version: "old runtime".into(),
            total_bytes: weights.len() as u64,
            placement_recorded: true,
            body_removed: false,
            ready: false,
            review_hash: "a".repeat(64),
            compatibility: None,
        };
        let found = candidates(&models, &[entry], &recipe);
        assert_eq!(found.len(), 1);
        let receipt = super::super::stage(
            &root.join("downloads"),
            &uuid::Uuid::new_v4().to_string(),
            &recipe,
            &recipe.hash().unwrap(),
            &|| false,
            &mut |_, _| {},
            &mut |url, size, hash, out, cancelled, progress| {
                if let Some(existing) = found.iter().find(|f| f.url == url) {
                    existing.copy(size, hash, out, cancelled, progress)
                } else {
                    assert_eq!(
                        url, "https://runtime.invalid/engine",
                        "model network fetch must never happen"
                    );
                    out.write_all(engine).map_err(|e| e.to_string())?;
                    progress(engine.len() as u64);
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(
            receipt
                .report
                .files
                .iter()
                .find(|f| f.role == Role::Asr)
                .unwrap()
                .actual_hash,
            sha(weights)
        );
        assert_eq!(
            std::fs::read(package.join("models/model.bin")).unwrap(),
            weights
        );
    }
    #[test]
    fn reused_bytes_are_copied_only_after_recorded_identity_and_hash_checks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let package = root.join(&id);
        std::fs::create_dir_all(package.join("models")).unwrap();
        let path = package.join("models/model.bin");
        let data = b"exact installed ASR weights";
        std::fs::write(&path, data).unwrap();
        let hash = format!("{:x}", Sha256::digest(data));
        let recipe:super::super::Recipe=serde_json::from_value(serde_json::json!({"schemaVersion":4,"manifest":{"schemaVersion":1,"engine":"whisper.cpp","engineVersion":"v1.9.4","modelName":"test","modelVersion":"fixed","quantization":null,"target":"macos_aarch64","licenseRef":"test","files":[{"relativePath":"models/model.bin","role":"asr","expectedSize":data.len(),"expectedHash":hash}]},"sources":[{"relativePath":"models/model.bin","url":"https://example.com/model"}]})).unwrap();
        let mut entry = CatalogEntry {
            backend_id: None,
            package_id: id,
            source_type: "download".into(),
            download: None,
            model_name: "test".into(),
            model_version: "fixed".into(),
            quantization: None,
            model_files: vec![crate::model_placement::ModelFileIdentity {
                role: Role::Asr,
                size: data.len() as u64,
                sha256: hash.clone(),
            }],
            engine: "whisper.cpp".into(),
            engine_version: "test".into(),
            total_bytes: data.len() as u64,
            placement_recorded: true,
            body_removed: false,
            ready: false,
            review_hash: "a".repeat(64),
            compatibility: None,
        };
        for removed in [false, true] {
            entry.body_removed = removed;
            let found = candidates(&root, std::slice::from_ref(&entry), &recipe);
            assert_eq!(found.len(), usize::from(!removed));
            if removed {
                break;
            }
            for mode in ["valid", "cancel", "tamper", "symlink"] {
                let output = tempfile::tempfile().unwrap();
                let mut output = output;
                if mode == "tamper" {
                    std::fs::write(&path, vec![b'x'; data.len()]).unwrap();
                }
                if mode == "symlink" {
                    std::fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink(root.join("output"), &path).unwrap();
                }
                let result = found[0].copy(
                    data.len() as u64,
                    &hash,
                    &mut output,
                    &|| mode == "cancel",
                    &mut |_| {},
                );
                assert_eq!(result.is_ok(), mode == "valid", "{mode}");
                if mode == "symlink" {
                    std::fs::remove_file(&path).unwrap();
                }
                std::fs::write(&path, data).unwrap();
            }
        }
    }
}
