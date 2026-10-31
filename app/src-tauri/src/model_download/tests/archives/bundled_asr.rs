use super::*;

fn fixture(temp: &tempfile::TempDir) -> PathBuf {
    let resources = temp.path().canonicalize().unwrap();
    let asr = resources.join("inference-runtime");
    std::fs::create_dir(&asr).unwrap();
    let notice = b"synthetic runtime notice";
    let bytes = tar(&[
        ("bin/mon-transducer-worker", BIN, b'0'),
        ("notices/nlohmann-MIT.txt", notice, b'0'),
    ]);
    std::fs::write(asr.join("asr-runtime.tar"), &bytes).unwrap();
    std::fs::write(
        asr.join("asr-index.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "archiveBytes": bytes.len(),
            "archiveSha256": hash(&bytes),
            "files": [
                {"relativePath":"bin/mon-transducer-worker", "role":"executable",
                 "expectedSize":BIN.len(), "expectedHash":hash(BIN)},
                {"relativePath":"notices/nlohmann-MIT.txt", "role":"notice",
                 "expectedSize":notice.len(), "expectedHash":hash(notice)}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    resources.join("asr-runtime") // No Whisper resources are needed by ReazonSpeech.
}

fn prepare_runtime_without_network(bundle: &Path, staging: &Path, recipe: &Recipe) {
    let cancelled = AtomicBool::new(false);
    let operation = id();
    let mut progress = Vec::new();
    let error = download_platform(
        staging,
        bundle,
        None,
        &operation,
        recipe,
        &recipe.hash().unwrap(),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |done, total| {
            progress.push((done, total));
            // The second report follows extraction of both bundled members.
            // Stop before any model, library or notice is fetched over HTTPS.
            if progress.len() == 2 {
                cancelled.store(true, Ordering::SeqCst);
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("中断"), "{error}");
    assert_eq!(progress, vec![(0, recipe.transfer_bytes().unwrap()); 2]);
    let directory = staging.join(operation);
    assert!(directory.join(INTENT).is_file());
    assert!(!directory.join(COMPLETE).exists());
    for (file, source) in recipe.manifest.files.iter().zip(&recipe.sources) {
        let path = directory.join("payload").join(&file.relative_path);
        if source.url == crate::model_asr_catalog::URI {
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(Some(bytes.len() as u64), file.expected_size);
            assert_eq!(Some(hash(&bytes)), file.expected_hash);
        } else {
            assert!(
                !path.exists(),
                "network file fetched: {}",
                file.relative_path
            );
        }
    }
}

#[test]
fn reazonspeech_variants_prepare_their_own_bundle_without_whisper_before_network() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = fixture(&temp);
    let choices =
        crate::model_asr_catalog::options(&bundle.parent().unwrap().join("inference-runtime"))
            .unwrap();
    assert_eq!(choices.len(), 2);
    for choice in choices {
        assert!(choice.id.starts_with("reazonspeech-k2-v2-"));
        prepare_runtime_without_network(&bundle, &private_root(&temp), &choice.recipe);
    }
    assert!(!bundle.exists());
}

#[test]
fn haas_bundled_vocabulary_is_selected_verified_and_prepared_before_network() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = fixture(&temp);
    let asr = bundle.parent().unwrap().join("inference-runtime");
    let choice_id = "haas-ja-v1.0-onnx-quantized";
    let definitions: Vec<crate::model_asr_catalog::Choice> = serde_json::from_str(include_str!(
        "../../../../../model-catalog/asr-transducer.json"
    ))
    .unwrap();
    let choice = definitions.into_iter().find(|c| c.id == choice_id).unwrap();
    let vocabulary = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../third-party/asr-assets/haas-ja-v1.0-onnx-quantized/models/bpe.vocab"
    ));
    let file = choice
        .recipe
        .manifest
        .files
        .iter()
        .find(|f| f.relative_path == "models/bpe.vocab")
        .unwrap();
    assert_eq!(
        file.expected_hash.as_deref(),
        Some(hash(vocabulary).as_str())
    );
    let member = format!("assets/{choice_id}/models/bpe.vocab");
    let notice = b"synthetic runtime notice";
    let bytes = tar(&[
        ("bin/mon-transducer-worker", BIN, b'0'),
        ("notices/nlohmann-MIT.txt", notice, b'0'),
        (&member, vocabulary, b'0'),
    ]);
    let index_path = asr.join("asr-index.json");
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
    index["archiveBytes"] = json!(bytes.len());
    index["archiveSha256"] = json!(hash(&bytes));
    index["modelAssets"] = json!({choice_id: [file]});
    std::fs::write(asr.join("asr-runtime.tar"), &bytes).unwrap();
    std::fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    let choices = crate::model_asr_catalog::options(&asr).unwrap();
    assert_eq!(choices.len(), 3);
    let haas = choices.iter().find(|c| c.id == choice_id).unwrap();
    let source = haas
        .recipe
        .sources
        .iter()
        .find(|s| s.relative_path == "models/bpe.vocab")
        .unwrap();
    assert_eq!(source.archive.as_ref().unwrap().expected_hash, hash(&bytes));
    assert_eq!(source.archive.as_ref().unwrap().member_path, member);
    prepare_runtime_without_network(&bundle, &private_root(&temp), &haas.recipe);
    index["modelAssets"][choice_id][0]["expectedHash"] = json!(hash(b"changed"));
    std::fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    assert!(crate::model_asr_catalog::options(&asr)
        .err()
        .unwrap()
        .contains("一致しません"));
}

#[test]
fn reazonspeech_rejects_changed_recipe_before_creating_download_intent() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = fixture(&temp);
    let choices =
        crate::model_asr_catalog::options(&bundle.parent().unwrap().join("inference-runtime"))
            .unwrap();
    for choice in choices {
        for change in ["model-hash", "model-url", "runtime-hash"] {
            let mut recipe = choice.recipe.clone();
            match change {
                "model-hash" => recipe.manifest.files[2].expected_hash = Some(hash(b"changed")),
                "model-url" => recipe.sources[2].url = "https://example.com/changed".into(),
                _ => {
                    for source in &mut recipe.sources[..2] {
                        source.archive.as_mut().unwrap().expected_hash = hash(b"changed");
                    }
                }
            }
            let staging = private_root(&temp);
            let error = download_platform(
                &staging,
                &bundle,
                None,
                &id(),
                &recipe,
                &recipe.hash().unwrap(),
                &crate::test_support::never_cancelled,
                &mut |_, _| panic!("invalid recipe started downloading"),
            )
            .unwrap_err();
            assert!(
                error.contains("アプリ同梱ASRモデル定義と一致しません"),
                "{change}: {error}"
            );
            assert!(!staging.exists());
        }
    }
}

#[test]
#[ignore = "requires MON_REAZON_BUNDLE_ROOT; verifies packaged runtime without downloading models or changing app data"]
fn packaged_reazonspeech_variants_prepare_runtime_without_network() {
    let bundle = PathBuf::from(std::env::var_os("MON_REAZON_BUNDLE_ROOT").unwrap());
    let choices =
        crate::model_asr_catalog::options(&bundle.parent().unwrap().join("inference-runtime"))
            .unwrap();
    assert_eq!(choices.len(), 2);
    let temp = tempfile::tempdir().unwrap();
    for choice in choices {
        prepare_runtime_without_network(&bundle, &private_root(&temp), &choice.recipe);
    }
}
