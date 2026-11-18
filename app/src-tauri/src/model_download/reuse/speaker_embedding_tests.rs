use super::*;
use crate::model_download_recipe::{DownloadSource, Recipe, RecipeExt};
use crate::model_inspection::{Engine, PackageFile, PackageManifest, Target};
use crate::model_placement::{fs::Directory, ModelFileIdentity};

const EMBEDDING: &[u8] = b"shared speaker embedding reused across segmentation variants";
const SEGMENTATION: &[u8] = b"new segmentation model";
const ENGINE: &[u8] = b"new diarization runtime";

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fixture() -> (tempfile::TempDir, PathBuf, CatalogEntry, Recipe) {
    let temp = tempfile::tempdir().unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let root = Directory::root(&models).unwrap();
    let package_id = uuid::Uuid::new_v4().to_string();
    let package = root.child(&package_id, true).unwrap();
    let (mut output, _) = package.create("models/embedding.onnx").unwrap();
    output.write_all(EMBEDDING).unwrap();
    output.sync_all().unwrap();
    let files = [
        ("bin/engine", Role::Executable, ENGINE),
        ("models/segmentation.onnx", Role::Segmentation, SEGMENTATION),
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
            url: format!("https://fixture.invalid/{}", file.relative_path),
            archive: None,
        })
        .collect();
    let recipe = Recipe {
        schema_version: 4,
        manifest: PackageManifest {
            schema_version: 1,
            engine: Engine::Sherpa,
            engine_version: "new runtime".into(),
            model_name: "new segmentation variant".into(),
            model_version: "fixed".into(),
            quantization: None,
            license_ref: "synthetic fixture".into(),
            target: Target::current().unwrap(),
            files,
            backend: None,
        },
        sources,
    };
    let entry = CatalogEntry {
        backend_id: None,
        package_id,
        source_type: "download".into(),
        download: None,
        model_name: "previous segmentation variant".into(),
        model_version: "previous".into(),
        quantization: None,
        model_files: vec![ModelFileIdentity {
            role: Role::SpeakerEmbedding,
            size: EMBEDDING.len() as u64,
            sha256: hash(EMBEDDING),
        }],
        engine: "sherpa-onnx".into(),
        engine_version: "old runtime".into(),
        total_bytes: EMBEDDING.len() as u64,
        placement_recorded: true,
        body_removed: false,
        cleanup_pending: false,
        ready: false,
        review_hash: "a".repeat(64),
        compatibility: None,
    };
    (temp, models, entry, recipe)
}

#[test]
fn speaker_embedding_is_copied_without_a_network_fetch_for_a_new_segmentation_variant() {
    let (temp, models, entry, recipe) = fixture();
    let found = candidates(&models, std::slice::from_ref(&entry), &recipe);
    assert_eq!(found.len(), 1);
    let mut fetched = Vec::new();
    let receipt = super::super::stage(
        &temp.path().join("downloads"),
        &uuid::Uuid::new_v4().to_string(),
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut |url, size, sha, output, cancelled, progress| {
            if let Some(candidate) = found.iter().find(|candidate| candidate.url == url) {
                return candidate.copy(size, sha, output, cancelled, progress);
            }
            let bytes = match url.rsplit('/').next().unwrap() {
                "engine" => ENGINE,
                "segmentation.onnx" => SEGMENTATION,
                _ => panic!("shared embedding reached the network transport: {url}"),
            };
            fetched.push(url.to_owned());
            output.write_all(bytes).map_err(|error| error.to_string())?;
            progress(bytes.len() as u64);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(fetched.len(), 2);
    for root in [receipt.directory, models.join(entry.package_id)] {
        assert_eq!(
            std::fs::read(crate::native_fs::join(&root, "models/embedding.onnx")).unwrap(),
            EMBEDDING
        );
    }
}

#[test]
fn changing_from_eres2net_to_titanet_never_reuses_the_previous_embedding() {
    let (_temp, models, entry, mut recipe) = fixture();
    let model = recipe
        .manifest
        .files
        .iter_mut()
        .find(|f| f.role == Role::SpeakerEmbedding)
        .unwrap();
    // The same managed relative path does not make different weights reusable.
    model.expected_hash =
        Some("ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e".into());
    model.expected_size = Some(40_257_283);
    assert!(candidates(&models, &[entry], &recipe).is_empty());
}

#[test]
fn speaker_embedding_reuse_requires_recorded_identity_and_still_detects_cancellation_and_tampering()
{
    let (_temp, models, entry, recipe) = fixture();
    for change in ["unrecorded", "removed", "manual", "size", "sha", "role"] {
        let mut changed = entry.clone();
        match change {
            "unrecorded" => changed.placement_recorded = false,
            "removed" => changed.body_removed = true,
            "manual" => changed.source_type = "manual_import".into(),
            "size" => changed.model_files[0].size += 1,
            "sha" => changed.model_files[0].sha256 = hash(b"other embedding"),
            _ => changed.model_files[0].role = Role::Executable,
        }
        assert!(
            candidates(&models, &[changed], &recipe).is_empty(),
            "{change}"
        );
    }
    let found = candidates(&models, std::slice::from_ref(&entry), &recipe);
    let mut output = tempfile::tempfile().unwrap();
    assert!(found[0]
        .copy(
            EMBEDDING.len() as u64,
            &hash(EMBEDDING),
            &mut output,
            &|| true,
            &mut |_| panic!("cancelled embedding copy progressed"),
        )
        .unwrap_err()
        .contains("中断"));
    std::fs::write(
        crate::native_fs::join(&models.join(&entry.package_id), "models/embedding.onnx"),
        vec![b'x'; EMBEDDING.len()],
    )
    .unwrap();
    assert!(found[0]
        .copy(
            EMBEDDING.len() as u64,
            &hash(EMBEDDING),
            &mut output,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err()
        .contains("SHA256"));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_acquisition_without_a_bundle_reuses_installed_model_bytes_before_transport() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (temp, models, entry, mut recipe) = fixture();
    // Copy the installed file first, then stop before acquiring the new runtime.
    // A regression to the old no-bundle fast path can only contact loopback.
    recipe.manifest.files.swap(0, 2);
    recipe.sources.swap(0, 2);
    for source in &mut recipe.sources {
        source.url = format!("https://127.0.0.1:1/{}", source.relative_path);
    }
    assert!(!recipe.has_bundled());
    let operation = uuid::Uuid::new_v4().to_string();
    let staging = temp.path().join("downloads");
    let cancelled = AtomicBool::new(false);
    let mut copied_reports = 0;
    let error = super::super::download_with_bundle_reuse(
        &staging,
        &temp.path().join("absent-asr-runtime"),
        Some((&models, &[entry])),
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |done, _| {
            if done == EMBEDDING.len() as u64 {
                copied_reports += 1;
                if copied_reports == 2 {
                    cancelled.store(true, Ordering::SeqCst);
                }
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("中断"), "{error}");
    assert_eq!(copied_reports, 2);
    let payload = staging.join(operation).join("payload");
    assert_eq!(
        std::fs::read(payload.join("models/embedding.onnx")).unwrap(),
        EMBEDDING
    );
    assert!(!payload.join("bin/engine").exists());
    assert!(!payload
        .parent()
        .unwrap()
        .join("download-complete.json")
        .exists());
}
