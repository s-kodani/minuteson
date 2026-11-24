//! Cross-operation archive reuse uses real records and bounded TAR extraction.
//! All payloads are synthetic; no model/runtime is executed or fetched online.
use super::*;
use crate::model_download_recipe::{ArchiveMember, DownloadSource};
use crate::model_inspection::{Engine, PackageFile, PackageManifest, Role, Target};
use std::sync::atomic::{AtomicBool, Ordering};

const BIN: &[u8] = b"synthetic diarization runtime";
const EMBEDDING: &[u8] = b"synthetic shared embedding";
const NORMAL: &[u8] = b"synthetic normal segmentation";
const INT8: &[u8] = b"synthetic INT8 segmentation";
const ARCHIVE_URL: &str = "https://fixture.invalid/shared-segmentation.tar";

fn hash(bytes: &[u8]) -> String {
    mon_core::hash::sha256_hex(bytes)
}

fn archive() -> Vec<u8> {
    let mut result = Vec::new();
    for (name, bytes) in [("model.onnx", NORMAL), ("model.int8.onnx", INT8)] {
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum: u64 = header.iter().map(|n| *n as u64).sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        result.extend(header);
        result.extend(bytes);
        result.resize(result.len().div_ceil(512) * 512, 0);
    }
    result.resize(result.len() + 1024, 0);
    result
}

fn recipe(int8: bool, archive: &[u8]) -> Recipe {
    let segmentation = if int8 { INT8 } else { NORMAL };
    let files = [
        ("models/segmentation.onnx", Role::Segmentation, segmentation),
        ("bin/engine", Role::Executable, BIN),
        ("models/embedding.onnx", Role::SpeakerEmbedding, EMBEDDING),
    ]
    .into_iter()
    .map(|(relative, role, bytes)| PackageFile {
        relative_path: relative.into(),
        role,
        expected_size: Some(bytes.len() as u64),
        expected_hash: Some(hash(bytes)),
    })
    .collect::<Vec<_>>();
    let sources = files
        .iter()
        .map(|file| DownloadSource {
            relative_path: file.relative_path.clone(),
            url: if file.role == Role::Segmentation {
                ARCHIVE_URL.into()
            } else {
                format!("https://fixture.invalid/{}", file.relative_path)
            },
            archive: (file.role == Role::Segmentation).then(|| ArchiveMember {
                format: "tar".into(),
                expected_size: archive.len() as u64,
                expected_hash: hash(archive),
                member_path: if int8 {
                    "model.int8.onnx"
                } else {
                    "model.onnx"
                }
                .into(),
            }),
        })
        .collect();
    Recipe {
        schema_version: 4,
        manifest: PackageManifest {
            schema_version: 1,
            engine: Engine::Sherpa,
            engine_version: "synthetic".into(),
            model_name: if int8 {
                "INT8 variant"
            } else {
                "normal variant"
            }
            .into(),
            model_version: "fixed".into(),
            quantization: None,
            license_ref: "synthetic fixture".into(),
            target: Target::current().unwrap(),
            files,
            backend: None,
        },
        sources,
    }
}

fn acquire(
    root: &Path,
    operation: &str,
    recipe: &Recipe,
    archive: &[u8],
    allow_archive_fetch: bool,
) -> (DownloadReceipt, usize, Vec<(u64, u64)>) {
    let mut archive_fetches = 0;
    let mut progress = Vec::new();
    let receipt = stage(
        root,
        operation,
        recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |done, total| progress.push((done, total)),
        &mut |url, _, _, output, _, report| {
            let bytes = if recipe
                .sources
                .iter()
                .any(|source| source.url == url && source.archive.is_some())
            {
                assert!(
                    allow_archive_fetch,
                    "cached archive reached the network: {url}"
                );
                archive_fetches += 1;
                archive
            } else if url.ends_with("bin/engine") {
                BIN
            } else if url.ends_with("models/embedding.onnx") {
                EMBEDDING
            } else {
                panic!("unexpected network request: {url}");
            };
            output.write_all(bytes).map_err(|error| error.to_string())?;
            report(bytes.len() as u64);
            Ok(())
        },
    )
    .unwrap();
    let total = recipe.transfer_bytes().unwrap();
    assert_eq!(progress.last(), Some(&(total, total)));
    assert!(progress
        .iter()
        .all(|(done, all)| *all == total && done <= all));
    assert!(progress.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    (receipt, archive_fetches, progress)
}

#[test]
fn completed_archive_is_reused_for_another_member_after_payload_moves() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("downloads");
    let bytes = archive();
    let original = recipe(false, &bytes);
    let first = uuid::Uuid::new_v4().to_string();
    let (receipt, fetched, _) = acquire(&root, &first, &original, &bytes, true);
    assert_eq!(fetched, 1);
    std::fs::rename(&receipt.directory, base.join("placed-payload")).unwrap();
    let saved = root.join(&first).join("archives/source-0.tar");
    assert_eq!(std::fs::read(&saved).unwrap(), bytes);
    let mut next = recipe(true, &bytes);
    // The old and new recipes may give the same archive different indices.
    next.manifest.files.swap(0, 2);
    next.sources.swap(0, 2);
    let identity = next.hash().unwrap();
    let second = uuid::Uuid::new_v4().to_string();
    let (receipt, fetched, _) = acquire(&root, &second, &next, &bytes, false);
    assert_eq!(fetched, 0);
    assert_eq!(
        std::fs::read(receipt.directory.join("models/segmentation.onnx")).unwrap(),
        INT8
    );
    assert_eq!(
        std::fs::read(root.join(&second).join("archives/source-2.tar")).unwrap(),
        bytes
    );
    assert_eq!(std::fs::read(&saved).unwrap(), bytes);
    assert_eq!(next.hash().unwrap(), identity);
    assert_eq!(
        readback(&root, &second, &identity, &|| false)
            .unwrap()
            .state,
        "complete"
    );
}

#[test]
fn missing_corrupt_and_unconfirmed_archives_fall_back_without_adopting_old_bytes() {
    for change in [
        "missing",
        "truncated",
        "sha",
        "incomplete",
        "completion",
        "intent",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("downloads");
        let bytes = archive();
        let first = uuid::Uuid::new_v4().to_string();
        acquire(&root, &first, &recipe(false, &bytes), &bytes, true);
        let old = root.join(&first);
        let saved = old.join("archives/source-0.tar");
        match change {
            "missing" => std::fs::remove_file(&saved).unwrap(),
            "truncated" => std::fs::write(&saved, &bytes[..bytes.len() - 1]).unwrap(),
            "sha" => {
                let mut corrupt = bytes.clone();
                corrupt[600] ^= 1;
                std::fs::write(&saved, corrupt).unwrap();
            }
            "incomplete" => std::fs::remove_file(old.join(COMPLETE)).unwrap(),
            "completion" => {
                let path = old.join(COMPLETE);
                let mut completion: Completion =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                completion.total_bytes += 1;
                std::fs::write(path, serde_json::to_vec(&completion).unwrap()).unwrap();
            }
            _ => {
                let path = old.join(INTENT);
                let mut intent: Intent =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                intent.recipe_hash = "f".repeat(64);
                std::fs::write(path, serde_json::to_vec(&intent).unwrap()).unwrap();
            }
        }
        let before = std::fs::read(&saved).ok();
        let second = uuid::Uuid::new_v4().to_string();
        let (receipt, fetched, _) = acquire(&root, &second, &recipe(true, &bytes), &bytes, true);
        assert_eq!(fetched, 1, "{change}");
        assert_eq!(
            std::fs::read(receipt.directory.join("models/segmentation.onnx")).unwrap(),
            INT8
        );
        assert_eq!(
            std::fs::read(&saved).ok(),
            before,
            "old evidence changed: {change}"
        );
    }
}

#[test]
fn archive_reuse_requires_matching_source_metadata_even_when_bytes_match() {
    for change in ["url", "size", "hash"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("downloads");
        let bytes = archive();
        let first = uuid::Uuid::new_v4().to_string();
        acquire(&root, &first, &recipe(false, &bytes), &bytes, true);
        let old = root.join(&first);
        let mut intent: Intent =
            serde_json::from_slice(&std::fs::read(old.join(INTENT)).unwrap()).unwrap();
        match change {
            "url" => {
                intent.recipe.sources[0].url =
                    "https://different-provider.invalid/shared-segmentation.tar".into()
            }
            "size" => {
                intent.recipe.sources[0]
                    .archive
                    .as_mut()
                    .unwrap()
                    .expected_size += 1
            }
            _ => {
                intent.recipe.sources[0]
                    .archive
                    .as_mut()
                    .unwrap()
                    .expected_hash = "a".repeat(64)
            }
        }
        intent.recipe_hash = intent.recipe.hash().unwrap();
        let mut completion: Completion =
            serde_json::from_slice(&std::fs::read(old.join(COMPLETE)).unwrap()).unwrap();
        completion.recipe_hash = intent.recipe_hash.clone();
        std::fs::write(old.join(INTENT), serde_json::to_vec(&intent).unwrap()).unwrap();
        std::fs::write(old.join(COMPLETE), serde_json::to_vec(&completion).unwrap()).unwrap();
        let (_, fetched, _) = acquire(
            &root,
            &uuid::Uuid::new_v4().to_string(),
            &recipe(true, &bytes),
            &bytes,
            true,
        );
        assert_eq!(fetched, 1, "{change}");
    }
}

#[test]
fn cached_archive_destination_write_errors_are_not_treated_as_cache_misses() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("downloads");
    let bytes = archive();
    acquire(
        &root,
        &uuid::Uuid::new_v4().to_string(),
        &recipe(false, &bytes),
        &bytes,
        true,
    );
    let destination = base.join("read-only-destination");
    std::fs::write(&destination, b"unchanged destination").unwrap();
    let mut output = std::fs::File::open(&destination).unwrap();
    let result = archive_reuse::try_copy(
        &root,
        &uuid::Uuid::new_v4().to_string(),
        ARCHIVE_URL,
        bytes.len() as u64,
        &hash(&bytes),
        &mut output,
        &|| false,
        &mut |_| panic!("failed writes cannot publish progress"),
    );
    assert!(result.is_err(), "a destination error must stop preparation");
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"unchanged destination"
    );
}

#[test]
fn cancelling_cached_copy_preserves_source_and_leaves_new_operation_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("downloads");
    let bytes = archive();
    let first = uuid::Uuid::new_v4().to_string();
    acquire(&root, &first, &recipe(false, &bytes), &bytes, true);
    let second = uuid::Uuid::new_v4().to_string();
    let next = recipe(true, &bytes);
    let cancelled = AtomicBool::new(false);
    let error = stage(
        &root,
        &second,
        &next,
        &next.hash().unwrap(),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |done, _| {
            if done == bytes.len() as u64 {
                cancelled.store(true, Ordering::SeqCst);
            }
        },
        &mut |_, _, _, _, _, _| panic!("cancelled cached preparation reached network"),
    )
    .unwrap_err();
    assert!(error.contains("中断"), "{error}");
    assert_eq!(
        std::fs::read(root.join(&first).join("archives/source-0.tar")).unwrap(),
        bytes
    );
    assert!(root.join(&first).join(COMPLETE).is_file());
    assert!(!root.join(&second).join(COMPLETE).exists());
    assert_eq!(
        readback(&root, &second, &next.hash().unwrap(), &|| false)
            .unwrap()
            .state,
        "incomplete"
    );
}
