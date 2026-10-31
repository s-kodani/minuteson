use super::*;
use crate::model_download_recipe::{DownloadSource, Recipe, RecipeExt};
use crate::model_inspection::{PackageFile, Target};
use crate::model_placement::fs::Directory;

fn bytes(relative: &str) -> Vec<u8> {
    format!("synthetic model fixture: {relative}").into_bytes()
}

fn fixture() -> (tempfile::TempDir, PathBuf, CatalogEntry, Recipe) {
    let temp = tempfile::tempdir().unwrap();
    // Match app_data_dir(): an ordinary absolute Windows path, without \\?\.
    let models = temp.path().join("モデル copies").join("models");
    crate::native_fs::private_directories(&models, true).unwrap();
    assert_ne!(models, models.canonicalize().unwrap());
    let root = Directory::open_existing(&models).unwrap();
    let package_id = uuid::Uuid::new_v4().to_string();
    let package = root.child(&package_id, true).unwrap();
    let choices: Vec<crate::model_asr_catalog::Choice> = serde_json::from_str(include_str!(
        "../../../../model-catalog/asr-transducer.json"
    ))
    .unwrap();
    let lightweight = &choices
        .iter()
        .find(|choice| choice.id == "reazonspeech-k2-v2-int8-fp32")
        .unwrap()
        .recipe;
    let mut model_files = Vec::new();
    for file in &lightweight.manifest.files {
        if !matches!(file.role, Role::Asr | Role::AsrAsset | Role::Vad) {
            continue;
        }
        let data = bytes(&file.relative_path);
        let (mut output, _) = package.create(&file.relative_path).unwrap();
        output.write_all(&data).unwrap();
        output.sync_all().unwrap();
        model_files.push(crate::model_placement::ModelFileIdentity {
            role: file.role,
            size: data.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&data)),
        });
    }
    let entry = CatalogEntry {
        backend_id: Some("sherpa-onnx-transducer".into()),
        package_id,
        source_type: "download".into(),
        download: None,
        model_name: lightweight.manifest.model_name.clone(),
        model_version: lightweight.manifest.model_version.clone(),
        quantization: lightweight.manifest.quantization.clone(),
        total_bytes: model_files.iter().map(|file| file.size).sum(),
        model_files,
        engine: "asr".into(),
        engine_version: "test".into(),
        placement_recorded: true,
        body_removed: false,
        ready: false,
        review_hash: "a".repeat(64),
        compatibility: None,
    };
    let mut recipe = choices
        .into_iter()
        .find(|choice| choice.id == "reazonspeech-k2-v2-fp32")
        .unwrap()
        .recipe;
    recipe.manifest.target = Target::WindowsX86_64;
    recipe.manifest.files.insert(
        0,
        PackageFile {
            relative_path: "bin/worker.exe".into(),
            role: Role::Executable,
            expected_size: None,
            expected_hash: None,
        },
    );
    for file in &mut recipe.manifest.files {
        let data = bytes(&file.relative_path);
        file.expected_size = Some(data.len() as u64);
        file.expected_hash = Some(format!("{:x}", Sha256::digest(data)));
    }
    recipe.sources = recipe
        .manifest
        .files
        .iter()
        .map(|file| DownloadSource {
            relative_path: file.relative_path.clone(),
            url: format!("https://fixture.invalid/{}", file.relative_path),
            archive: None,
        })
        .collect();
    (temp, models, entry, recipe)
}

#[test]
fn windows_reazonspeech_fp32_reuses_shared_files_from_normal_and_extended_paths() {
    let (temp, models, entry, recipe) = fixture();
    for root in [models.clone(), models.canonicalize().unwrap()] {
        let found = candidates(&root, std::slice::from_ref(&entry), &recipe);
        assert_eq!(found.len(), 3, "decoder, tokens and VAD must be reused");
        let mut reused = Vec::new();
        let receipt = super::super::stage(
            &temp.path().join("downloads"),
            &uuid::Uuid::new_v4().to_string(),
            &recipe,
            &recipe.hash().unwrap(),
            &|| false,
            &mut |_, _| {},
            &mut |url, size, hash, output, cancelled, progress| {
                if let Some(candidate) = found.iter().find(|candidate| candidate.url == url) {
                    candidate.copy(size, hash, output, cancelled, progress)?;
                    reused.push(candidate.relative.clone());
                } else {
                    let relative = url.strip_prefix("https://fixture.invalid/").unwrap();
                    let data = bytes(relative);
                    output.write_all(&data).map_err(|error| error.to_string())?;
                    progress(data.len() as u64);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(reused.len(), 3);
        for relative in reused {
            assert_eq!(
                std::fs::read(crate::native_fs::join(&receipt.directory, &relative)).unwrap(),
                bytes(&relative)
            );
            assert_eq!(
                std::fs::read(crate::native_fs::join(
                    &models.join(&entry.package_id),
                    &relative
                ))
                .unwrap(),
                bytes(&relative)
            );
        }
    }
}

#[test]
fn windows_reused_model_files_keep_pins_hash_checks_and_cancellation() {
    let (temp, models, mut entry, recipe) = fixture();
    let found = candidates(&models, std::slice::from_ref(&entry), &recipe);
    let candidate = &found[0];
    let relative = &candidate.relative;
    let data = bytes(relative);
    let hash = format!("{:x}", Sha256::digest(&data));
    let package = models.join(&entry.package_id);
    let path = crate::native_fs::join(&package, relative);
    let mut output = tempfile::tempfile().unwrap();
    candidate
        .copy(
            data.len() as u64,
            &hash,
            &mut output,
            &|| false,
            &mut |_| {
                assert!(std::fs::rename(&package, models.join("moved")).is_err());
                assert!(std::fs::write(&path, b"replacement").is_err());
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(candidate
        .copy(
            data.len() as u64,
            &hash,
            &mut output,
            &|| true,
            &mut |_| panic!("cancelled copy progressed")
        )
        .unwrap_err()
        .contains("中断"));
    std::fs::write(&path, vec![b'x'; data.len()]).unwrap();
    assert!(candidate
        .copy(
            data.len() as u64,
            &hash,
            &mut output,
            &|| false,
            &mut |_| {}
        )
        .unwrap_err()
        .contains("SHA256"));
    std::fs::write(&path, &data).unwrap();
    std::fs::hard_link(&path, temp.path().join("alias")).unwrap();
    assert!(candidate
        .copy(
            data.len() as u64,
            &hash,
            &mut output,
            &|| false,
            &mut |_| panic!("linked file copied")
        )
        .is_err());
    entry.body_removed = true;
    assert!(candidates(&models, std::slice::from_ref(&entry), &recipe).is_empty());
    entry.body_removed = false;
    entry.placement_recorded = false;
    assert!(candidates(&models, &[entry], &recipe).is_empty());
}
