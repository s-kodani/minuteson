use super::*;
use std::cell::Cell;

#[test]
fn guided_model_preflight_needs_no_manifest_and_preserves_existing_manifest() {
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let (dir, mut manifest) = fixture(engine);
        manifest.target = Target::current().unwrap();
        std::fs::remove_file(dir.path().join(MANIFEST)).unwrap();
        let report =
            inspect_declared(dir.path(), &manifest, &crate::test_support::never_cancelled).unwrap();
        assert_eq!(report.manifest, manifest);
        assert_eq!(
            report.manifest_hash,
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&manifest).unwrap())
            )
        );
        assert!(!dir.path().join(MANIFEST).exists());
        assert!(!report.ready && !report.installed);
        assert_eq!(report.publisher_authenticity, "not_verified");
        let original = b"unrelated existing manifest; must not be changed or adopted";
        std::fs::write(dir.path().join(MANIFEST), original).unwrap();
        assert_eq!(
            inspect_declared(dir.path(), &manifest, &crate::test_support::never_cancelled)
                .unwrap()
                .review_hash,
            report.review_hash
        );
        assert_eq!(std::fs::read(dir.path().join(MANIFEST)).unwrap(), original);
    }
}

#[test]
fn guided_model_preflight_rejects_invalid_declaration_and_cancellation_without_writes() {
    let (dir, mut manifest) = fixture(Engine::Whisper);
    manifest.target = Target::current().unwrap();
    std::fs::remove_file(dir.path().join(MANIFEST)).unwrap();
    assert!(inspect_declared(dir.path(), &manifest, &|| true).is_err());
    manifest.files[1].relative_path = "../outside.bin".into();
    assert!(
        inspect_declared(dir.path(), &manifest, &crate::test_support::never_cancelled).is_err()
    );
    assert!(!dir.path().join(MANIFEST).exists());
}

#[cfg(unix)]
#[test]
fn guided_model_preflight_rejects_symlink_files_without_following_them() {
    let (dir, mut manifest) = fixture(Engine::Llama);
    manifest.target = Target::current().unwrap();
    std::fs::remove_file(dir.path().join(MANIFEST)).unwrap();
    let model = dir.path().join(&manifest.files[1].relative_path);
    std::fs::rename(&model, dir.path().join("real-model")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("real-model"), &model).unwrap();
    assert!(
        inspect_declared(dir.path(), &manifest, &crate::test_support::never_cancelled).is_err()
    );
    assert!(!dir.path().join(MANIFEST).exists());
}

fn fixture(engine: Engine) -> (tempfile::TempDir, PackageManifest) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("weights")).unwrap();
    let mut files = vec![("engine-cli", Role::Executable)];
    files.extend(match engine {
        Engine::Whisper | Engine::Asr => vec![("weights/model.bin", Role::Asr)],
        Engine::Sherpa => vec![
            ("weights/seg.onnx", Role::Segmentation),
            ("weights/embedding.onnx", Role::SpeakerEmbedding),
        ],
        Engine::Llama => vec![("weights/model.gguf", Role::Llm)],
    });
    let files = files
        .into_iter()
        .map(|(name, role)| {
            // Deliberately not a runnable binary or valid model. Preflight must not
            // launch it or claim engine compatibility from a hash/filename alone.
            let bytes = format!("test-only package file: {name}");
            std::fs::write(dir.path().join(name), bytes.as_bytes()).unwrap();
            PackageFile {
                relative_path: name.into(),
                role,
                expected_size: Some(bytes.len() as u64),
                expected_hash: Some(format!("{:x}", Sha256::digest(bytes.as_bytes()))),
            }
        })
        .collect();
    let manifest = PackageManifest { backend: None,
        schema_version: 1,
        engine,
        engine_version: "fixture-version".into(),
        model_name: "未評価の検査fixture".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "fixture only; no real redistribution license".into(),
        target: Target::MacosAarch64,
        files,
    };
    write_manifest(&dir, &manifest);
    (dir, manifest)
}
fn write_manifest(dir: &tempfile::TempDir, manifest: &PackageManifest) {
    std::fs::write(
        dir.path().join(MANIFEST),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
}
fn inspect_fixture(dir: &tempfile::TempDir) -> Result<Report, String> {
    inspect_for_target(
        dir.path(),
        Target::MacosAarch64,
        &crate::test_support::never_cancelled,
    )
}

#[test]
fn all_engine_packages_are_read_only_hashed_and_never_ready_or_executed() {
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let (dir, manifest) = fixture(engine);
        let manifest_bytes = std::fs::read(dir.path().join(MANIFEST)).unwrap();
        let report = inspect_fixture(&dir).unwrap();
        assert_eq!(report.manifest, manifest);
        assert_eq!(
            report.manifest_hash,
            format!("{:x}", Sha256::digest(&manifest_bytes))
        );
        assert_eq!(report.files.len(), manifest.files.len());
        assert_eq!(
            report.total_bytes,
            manifest
                .files
                .iter()
                .map(|f| f.expected_size.unwrap())
                .sum::<u64>()
        );
        assert!(!report.ready && !report.installed);
        assert_eq!(report.engine_compatibility, "not_verified");
        assert_eq!(report.publisher_authenticity, "not_verified");
        assert_eq!(
            std::fs::read(dir.path().join(MANIFEST)).unwrap(),
            manifest_bytes
        );
        assert!(!dir.path().join("executed").exists());
        for (entry, result) in manifest.files.iter().zip(report.files) {
            assert_eq!(&result.actual_hash, entry.expected_hash.as_ref().unwrap());
            assert!(result.expected_size_checked && result.expected_hash_checked);
        }
    }
}
#[test]
fn unknown_publisher_hash_is_observed_not_authenticated() {
    let (dir, mut manifest) = fixture(Engine::Whisper);
    for f in &mut manifest.files {
        f.expected_size = None;
        f.expected_hash = None;
    }
    write_manifest(&dir, &manifest);
    let report = inspect_fixture(&dir).unwrap();
    assert!(report
        .files
        .iter()
        .all(|f| !f.expected_size_checked && !f.expected_hash_checked && hash(&f.actual_hash)));
    assert_eq!(report.publisher_authenticity, "not_verified");
}
#[test]
fn unsafe_aliasing_and_duplicate_paths_are_rejected() {
    let (_, base) = fixture(Engine::Whisper);
    for name in [
        "",
        "../model",
        "a/../../model",
        "/model",
        "a//b",
        "a/./b",
        "C:weights",
        "a\\b",
        "model:stream",
        "NUL.bin",
        "com1",
        "LPT9.txt",
        "model.",
        "model ",
        "a/..",
        "mon-model.json",
        "MON-MODEL.JSON",
        "x\0y",
        "a?",
        "a\nb",
    ] {
        let mut changed = base.clone();
        changed.files[1].relative_path = name.into();
        assert!(changed.validate(Target::MacosAarch64).is_err(), "{name:?}");
    }
    for pair in [
        ("same", "SAME"),
        ("weights", "weights/model.bin"),
        ("Weights", "weights/model.bin"),
    ] {
        let mut changed = base.clone();
        changed.files[0].relative_path = pair.0.into();
        changed.files[1].relative_path = pair.1.into();
        assert!(changed.validate(Target::MacosAarch64).is_err());
    }
}
#[test]
fn wrong_target_schema_roles_and_metadata_do_not_pass() {
    let (_, base) = fixture(Engine::Whisper);
    assert!(base.validate(Target::WindowsX86_64).is_err());
    let mut changed = base.clone();
    changed.schema_version = 2;
    assert!(changed.validate(Target::MacosAarch64).is_err());
    for role in [
        Role::Llm,
        Role::Segmentation,
        Role::RuntimeLibrary,
        Role::Executable,
    ] {
        let mut changed = base.clone();
        changed.files[1].role = role;
        assert!(changed.validate(Target::MacosAarch64).is_err());
    }
    for value in [
        Some("A".repeat(64)),
        Some("1".repeat(63)),
        Some("g".repeat(64)),
    ] {
        let mut changed = base.clone();
        changed.files[1].expected_hash = value;
        assert!(changed.validate(Target::MacosAarch64).is_err());
    }
    for size in [0, u64::MAX] {
        let mut changed = base.clone();
        changed.files[1].expected_size = Some(size);
        assert!(changed.validate(Target::MacosAarch64).is_err());
    }
    let mut changed = base.clone();
    changed.license_ref.clear();
    assert!(changed.validate(Target::MacosAarch64).is_err());
    let mut changed = base;
    changed.engine_version = "bad\nversion".into();
    assert!(changed.validate(Target::MacosAarch64).is_err());
}
#[test]
fn missing_empty_changed_size_and_changed_hash_fail_preflight() {
    for replacement in [None, Some(Vec::new()), Some(b"different size".to_vec())] {
        let (dir, manifest) = fixture(Engine::Whisper);
        let model = dir.path().join(&manifest.files[1].relative_path);
        if let Some(bytes) = replacement {
            std::fs::write(&model, bytes).unwrap();
        } else {
            std::fs::remove_file(&model).unwrap();
        }
        assert!(inspect_fixture(&dir).is_err());
    }
    let (dir, manifest) = fixture(Engine::Whisper);
    std::fs::write(
        dir.path().join(&manifest.files[1].relative_path),
        vec![b'x'; manifest.files[1].expected_size.unwrap() as usize],
    )
    .unwrap();
    assert!(inspect_fixture(&dir).unwrap_err().contains("SHA"));
}
#[test]
fn unknown_duplicate_and_oversized_manifest_are_rejected() {
    let (dir, manifest) = fixture(Engine::Whisper);
    let mut value = serde_json::to_value(&manifest).unwrap();
    value["unknown"] = serde_json::json!(true);
    std::fs::write(
        dir.path().join(MANIFEST),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(inspect_fixture(&dir).is_err());
    let json = serde_json::to_string(&manifest).unwrap().replacen(
        "\"schemaVersion\":1",
        "\"schemaVersion\":1,\"schemaVersion\":1",
        1,
    );
    std::fs::write(dir.path().join(MANIFEST), json).unwrap();
    assert!(inspect_fixture(&dir).is_err());
    std::fs::write(dir.path().join(MANIFEST), vec![b' '; MAX_MANIFEST + 1]).unwrap();
    assert!(inspect_fixture(&dir).unwrap_err().contains("64KiB"));
    assert!(inspect_for_target(
        Path::new("relative"),
        Target::MacosAarch64,
        &crate::test_support::never_cancelled
    )
    .is_err());
}
#[cfg(unix)]
#[test]
fn symlink_leaf_parent_root_and_manifest_are_rejected() {
    use std::os::unix::fs::symlink;
    for leaf in [MANIFEST, "engine-cli", "weights/model.bin"] {
        let (dir, _) = fixture(Engine::Whisper);
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("source");
        std::fs::rename(dir.path().join(leaf), &target).unwrap();
        symlink(&target, dir.path().join(leaf)).unwrap();
        assert!(inspect_fixture(&dir).is_err());
    }
    let (dir, _) = fixture(Engine::Whisper);
    let outside = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path().join("weights"), outside.path().join("weights")).unwrap();
    symlink(outside.path().join("weights"), dir.path().join("weights")).unwrap();
    assert!(inspect_fixture(&dir).is_err());
    let link = outside.path().join("package");
    symlink(dir.path(), &link).unwrap();
    assert!(inspect_for_target(
        &link,
        Target::MacosAarch64,
        &crate::test_support::never_cancelled
    )
    .is_err());
}
#[test]
fn replacement_and_modification_during_hashing_are_detected() {
    for replace in [false, true] {
        let (dir, manifest) = fixture(Engine::Whisper);
        let path = dir.path().join("engine-cli");
        let calls = Cell::new(0);
        let result = inspect_for_target(dir.path(), Target::MacosAarch64, &|| {
            let n = calls.get() + 1;
            calls.set(n);
            // Initial check, before file, then first read-loop check: fd is open.
            if n == 3 {
                let bytes = std::fs::read(&path).unwrap();
                if replace {
                    let renamed = std::fs::rename(&path, dir.path().join("saved-cli"));
                    #[cfg(windows)]
                    if renamed.is_err() {
                        return true;
                    }
                    renamed.unwrap();
                }
                let written = std::fs::write(
                    &path,
                    if replace {
                        bytes
                    } else {
                        vec![b'z'; manifest.files[0].expected_size.unwrap() as usize]
                    },
                );
                #[cfg(windows)]
                if written.is_err() {
                    return true;
                }
                written.unwrap();
            }
            false
        });
        assert!(result.is_err());
    }
}
#[test]
fn cancelled_read_has_no_success_report_and_single_operation_lifetime_is_guarded() {
    let (dir, _) = fixture(Engine::Whisper);
    let calls = Cell::new(0);
    let error = inspect_for_target(dir.path(), Target::MacosAarch64, &|| {
        calls.set(calls.get() + 1);
        calls.get() >= 4
    })
    .unwrap_err();
    assert!(
        error.contains("取り消し") || error == "ファイル照合を中断しました",
        "{error}"
    );
    let inspector = Inspector::default();
    assert!(inspector.status("invalid").is_err());
    assert!(inspector.begin("invalid").is_err());
    let token = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let guard = inspector.begin(&token).unwrap();
    assert_eq!(inspector.status(&token).unwrap(), (true, false));
    assert!(inspector.begin(&other).is_err());
    inspector.cancel(&other).unwrap();
    assert_eq!(inspector.status(&other).unwrap(), (false, true));
    assert!(!guard.cancelled());
    inspector.cancel(&token).unwrap();
    assert_eq!(inspector.status(&token).unwrap(), (true, true));
    assert!(guard.cancelled());
    assert!(inspector.begin(&other).is_err());
    drop(guard);
    assert_eq!(inspector.status(&token).unwrap(), (false, true));
    assert!(inspector.begin(&other).is_err()); // cancellation arrived before begin
    let third = uuid::Uuid::new_v4().to_string();
    let next = inspector.begin(&third).unwrap();
    inspector.cancel(&token).unwrap();
    assert!(!next.cancelled());
    drop(next);
    assert!(inspector.begin(&token).is_err());
    assert!(inspector.begin(&uuid::Uuid::new_v4().to_string()).is_ok());
}
