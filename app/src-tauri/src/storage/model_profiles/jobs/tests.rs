use super::*;
use crate::model_compatibility::{artifact_names, settings, Artifact};
use crate::model_inspection::{FileReport, PackageFile, PackageManifest, Target};
use crate::transcription::asr::{BackendBinding, TRANSDUCER};

// Use the shipped model definitions, with synthetic worker/DLL identities.
// These tests never load weights or run an executable.
fn fixture(choice: usize, target: Target) -> (Snapshot, AsrConfig) {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../../model-catalog/asr-transducer.json"
    ))
    .unwrap();
    let mut manifest: PackageManifest =
        serde_json::from_value(catalog[choice]["recipe"]["manifest"].clone()).unwrap();
    manifest.target = target;
    let file = |path: &str, role| PackageFile {
        relative_path: path.into(),
        role,
        expected_size: Some(path.len() as u64),
        expected_hash: Some(digest(&path).unwrap()),
    };
    manifest
        .files
        .insert(0, file("bin/worker", Role::Executable));
    manifest
        .files
        .push(file("notices/nlohmann-MIT.txt", Role::Notice));
    if target == Target::WindowsX86_64 {
        manifest.files.retain(|f| f.role != Role::RuntimeLibrary);
        for path in [
            "lib/sherpa-onnx-c-api.dll",
            "lib/onnxruntime.dll",
            "lib/msvcp140.dll",
        ] {
            manifest.files.push(file(path, Role::RuntimeLibrary));
        }
        manifest
            .backend
            .as_mut()
            .unwrap()
            .files
            .insert("library".into(), "lib/sherpa-onnx-c-api.dll".into());
    }
    let files = manifest
        .files
        .iter()
        .map(|f| FileReport {
            relative_path: f.relative_path.clone(),
            role: f.role,
            actual_size: f.expected_size.unwrap(),
            actual_hash: f.expected_hash.clone().unwrap(),
            expected_size_checked: true,
            expected_hash_checked: true,
        })
        .collect::<Vec<_>>();
    let source_manifest_hash = digest(&manifest).unwrap();
    let package = PlacementRecord {
        schema_version: 3,
        package_id: Uuid::new_v4().to_string(),
        review_hash: crate::model_inspection::review_hash(&manifest, &source_manifest_hash, &files)
            .unwrap(),
        source_manifest_hash,
        manifest,
        files,
        download: None,
    };
    let compatibility = ProbeReport {
        schema_version: 1,
        operation_id: Uuid::new_v4().to_string(),
        package_id: package.package_id.clone(),
        review_hash: package.review_hash.clone(),
        engine: Engine::Asr,
        declared_engine_version: package.manifest.engine_version.clone(),
        adapter_revision: 1,
        settings: settings(Engine::Asr),
        artifacts: artifact_names(Engine::Asr)
            .iter()
            .map(|name| Artifact {
                name: name.to_string(),
                size: 1,
                sha256: digest(name).unwrap(),
            })
            .collect(),
        elapsed_ms: 1,
    };
    let directory = std::env::current_dir()
        .unwrap()
        .join("models")
        .join(&package.package_id);
    let proof = |file: &FileReport| Fingerprint {
        path: directory.join(&file.relative_path),
        size: file.actual_size,
        sha256: file.actual_hash.clone(),
    };
    let role = |role| proof(package.files.iter().find(|f| f.role == role).unwrap());
    let config = AsrConfig {
        executable: role(Role::Executable),
        model: role(Role::Asr),
        vad_model: Some(role(Role::Vad)),
        vad_options: None,
        threads: 4,
        use_gpu: false,
        prompt: String::new(),
        backend: Some(BackendBinding {
            id: TRANSDUCER.into(),
            files: package
                .manifest
                .backend
                .as_ref()
                .unwrap()
                .files
                .iter()
                .map(|(slot, path)| {
                    (
                        slot.clone(),
                        proof(
                            package
                                .files
                                .iter()
                                .find(|f| &f.relative_path == path)
                                .unwrap(),
                        ),
                    )
                })
                .collect(),
        }),
    };
    let snapshot = Snapshot {
        schema_version: 1,
        profile: ProfileName::Standard,
        profile_id: Uuid::new_v4().to_string(),
        profile_version: 1,
        profile_hash: "a".repeat(64),
        package,
        compatibility,
        configuration: Execution::Asr(&config).configuration(),
    };
    (snapshot, config)
}

#[test]
fn managed_asr_catalog_snapshots_accept_implicit_libraries_and_notices() {
    for target in [Target::MacosAarch64, Target::WindowsX86_64] {
        for choice in 0..2 {
            let (snapshot, config) = fixture(choice, target);
            snapshot
                .validate("standard", &Execution::Asr(&config))
                .unwrap();
            let mut portable = config.clone();
            for proof in [&mut portable.executable, &mut portable.model]
                .into_iter()
                .chain(portable.vad_model.iter_mut())
                .chain(portable.backend.as_mut().unwrap().files.values_mut())
            {
                proof.path = PathBuf::from(format!("mon-unavailable:{}", proof.sha256));
            }
            snapshot
                .validate("standard", &Execution::Asr(&portable))
                .unwrap();

            // The directly supplied C API library must still be present and
            // match its own bytes and path, rather than an implicit dependency.
            let mut invalid = config.clone();
            invalid.backend.as_mut().unwrap().files.remove("library");
            assert!(snapshot
                .validate("standard", &Execution::Asr(&invalid))
                .is_err());
            let mut invalid = config.clone();
            invalid
                .backend
                .as_mut()
                .unwrap()
                .files
                .get_mut("library")
                .unwrap()
                .sha256 = "b".repeat(64);
            assert!(snapshot
                .validate("standard", &Execution::Asr(&invalid))
                .is_err());
            let mut invalid = config.clone();
            let dependency = snapshot
                .package
                .files
                .iter()
                .find(|f| {
                    f.role == Role::RuntimeLibrary && !execution_file(&snapshot.package.manifest, f)
                })
                .unwrap();
            let library = invalid
                .backend
                .as_mut()
                .unwrap()
                .files
                .get_mut("library")
                .unwrap();
            library.path = library
                .path
                .parent()
                .unwrap()
                .join(Path::new(&dependency.relative_path).file_name().unwrap());
            library.size = dependency.actual_size;
            library.sha256 = dependency.actual_hash.clone();
            assert!(snapshot
                .validate("standard", &Execution::Asr(&invalid))
                .is_err());
        }
    }
}

#[test]
fn managed_asr_rebinding_preserves_implicit_dependencies_and_notices() {
    for target in [Target::MacosAarch64, Target::WindowsX86_64] {
        for choice in 0..2 {
            let (snapshot, config) = fixture(choice, target);
            let profile = attach(json!({"profile":"standard"}), Some(snapshot.clone())).unwrap();
            let mut package = snapshot.package.clone();
            package.package_id = Uuid::new_v4().to_string();
            let mut compatibility = snapshot.compatibility.clone();
            compatibility.package_id = package.package_id.clone();
            let directory = config
                .executable
                .path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(&package.package_id);
            let files = package
                .files
                .iter()
                .map(|file| Fingerprint {
                    path: directory.join(&file.relative_path),
                    size: file.actual_size,
                    sha256: file.actual_hash.clone(),
                })
                .collect::<Vec<_>>();
            let binding: rebinding::Binding = serde_json::from_value(json!({
                "schemaVersion":1,"package":package,"compatibility":compatibility,"files":files
            }))
            .unwrap();
            let rebinding::Resolved::Asr(resolved) =
                binding.resolve(&profile, Execution::Asr(&config)).unwrap()
            else {
                panic!("expected ASR")
            };
            assert_eq!(
                resolved.model.path,
                directory.join(
                    &snapshot
                        .package
                        .files
                        .iter()
                        .find(|f| f.role == Role::Asr)
                        .unwrap()
                        .relative_path
                )
            );
            assert_eq!(
                resolved.backend.as_ref().unwrap().files["library"].path,
                directory
                    .join(&snapshot.package.manifest.backend.as_ref().unwrap().files["library"])
            );
            let mut changed = serde_json::to_value(&binding).unwrap();
            let index = package
                .files
                .iter()
                .position(|f| {
                    f.role == Role::RuntimeLibrary && !execution_file(&package.manifest, f)
                })
                .unwrap();
            changed["files"][index]["sha256"] = json!("b".repeat(64));
            let changed: rebinding::Binding = serde_json::from_value(changed).unwrap();
            assert!(changed.resolve(&profile, Execution::Asr(&config)).is_err());
        }
    }
}

#[test]
fn managed_asr_verification_still_checks_implicit_libraries_and_notices() {
    let (mut snapshot, mut config) = fixture(1, Target::current().unwrap());
    let temp = tempfile::tempdir().unwrap();
    let directory = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("models")
        .join(&snapshot.package.package_id);
    std::fs::create_dir_all(&directory).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (declared, actual) in snapshot
        .package
        .manifest
        .files
        .iter_mut()
        .zip(&mut snapshot.package.files)
    {
        let bytes = declared.relative_path.as_bytes();
        actual.actual_size = bytes.len() as u64;
        actual.actual_hash = format!("{:x}", Sha256::digest(bytes));
        declared.expected_size = Some(actual.actual_size);
        declared.expected_hash = Some(actual.actual_hash.clone());
        let path = directory.join(&declared.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    snapshot.package.source_manifest_hash = digest(&snapshot.package.manifest).unwrap();
    snapshot.package.review_hash = crate::model_inspection::review_hash(
        &snapshot.package.manifest,
        &snapshot.package.source_manifest_hash,
        &snapshot.package.files,
    )
    .unwrap();
    snapshot.compatibility.review_hash = snapshot.package.review_hash.clone();
    std::fs::write(
        directory.join(crate::model_inspection::MANIFEST),
        serde_json::to_vec(&snapshot.package.manifest).unwrap(),
    )
    .unwrap();
    let previous = config
        .executable
        .path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    for proof in [&mut config.executable, &mut config.model]
        .into_iter()
        .chain(config.vad_model.iter_mut())
        .chain(config.backend.as_mut().unwrap().files.values_mut())
    {
        let relative = proof.path.strip_prefix(&previous).unwrap();
        let file = snapshot
            .package
            .files
            .iter()
            .find(|f| Path::new(&f.relative_path) == relative)
            .unwrap();
        proof.path = directory.join(relative);
        proof.size = file.actual_size;
        proof.sha256 = file.actual_hash.clone();
    }
    snapshot
        .verify("standard", &Execution::Asr(&config), &|| false)
        .unwrap();
    for file in snapshot
        .package
        .files
        .iter()
        .filter(|f| !execution_file(&snapshot.package.manifest, f))
    {
        let path = directory.join(&file.relative_path);
        let original = std::fs::read(&path).unwrap();
        // Preserve size so the physical SHA check, not just the length, matters.
        std::fs::write(&path, vec![b'x'; original.len()]).unwrap();
        assert!(snapshot
            .verify("standard", &Execution::Asr(&config), &|| false)
            .is_err());
        std::fs::write(path, original).unwrap();
    }
}
