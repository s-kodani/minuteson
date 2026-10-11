use super::*;

#[test]
#[ignore = "requires official local assets and explicit permission for a synthetic CPU trial"]
fn real_llama_catalog_archive_copy_and_cpu_with_reused_assets() {
    let archive_path =
        PathBuf::from(std::env::var("MON_QA_LLAMA_ARCHIVE").expect("official archive path"));
    let model_path =
        PathBuf::from(std::env::var("MON_QA_QWEN_MODEL").expect("existing Qwen model path"));
    let license_path = PathBuf::from(
        std::env::var("MON_QA_QWEN_LICENSE").expect("fixed official Qwen license path"),
    );
    trial(
        "llama-b11533-qwen25-15b-q4km",
        &|url| {
            if url.ends_with(".tar.gz") {
                archive_path.clone()
            } else if url.ends_with(".gguf") {
                model_path.clone()
            } else {
                license_path.clone()
            }
        },
        3,
    );
}

#[test]
#[ignore = "requires official local assets and explicit permission for a synthetic CPU trial"]
fn real_sherpa_catalog_archive_copy_and_cpu_with_reused_assets() {
    let assets =
        PathBuf::from(std::env::var("MON_QA_SHERPA_ASSETS").expect("existing sherpa assets"));
    let notices =
        PathBuf::from(std::env::var("MON_QA_SHERPA_NOTICES").expect("fixed official notices"));
    trial(
        "sherpa-1138-pyannote3-eres2net-base",
        &|url| {
            if url.ends_with("osx-arm64-shared-no-tts.tar.bz2") {
                assets.join("sherpa.tar.bz2")
            } else if url.ends_with("segmentation-3-0.tar.bz2") {
                assets.join("segmentation.tar.bz2")
            } else if url.ends_with("16k.onnx") {
                assets.join("embedding-base.onnx")
            } else if url.contains("/k2-fsa/sherpa-onnx/") {
                notices.join("sherpa-Apache-2.0.txt")
            } else if url.contains("/modelscope/3D-Speaker/") {
                notices.join("3DSpeaker-Apache-2.0.txt")
            } else if url.ends_with("/ThirdPartyNotices.txt") {
                notices.join("onnxruntime-ThirdPartyNotices.txt")
            } else {
                assert!(url.contains("/microsoft/onnxruntime/") && url.ends_with("/LICENSE"));
                notices.join("onnxruntime-MIT.txt")
            }
        },
        7,
    );
}

fn trial(choice_id: &str, local: &impl Fn(&str) -> PathBuf, expected_calls: usize) {
    let options: Value =
        serde_json::from_str(include_str!("../../../../../model-catalog/macos.json")).unwrap();
    let choice = options
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == choice_id)
        .unwrap();
    let recipe = Recipe::parse(&serde_json::to_vec(&choice["recipe"]).unwrap()).unwrap();
    trial_recipe(choice_id, recipe, local, expected_calls, None);
}

#[test]
#[ignore = "requires the app-owned bundled runtime and fixed small model assets; explicit synthetic CPU permission"]
fn real_whisper_hybrid_catalog_copy_cpu_and_test_owned_apply_with_reused_assets() {
    let bundle_root = PathBuf::from(
        std::env::var("MON_QA_ASR_BUNDLE").expect("native app bundle resource directory"),
    );
    let payload =
        PathBuf::from(std::env::var("MON_QA_ASR_PAYLOAD").expect("fixed ASR download payload"));
    let mut bundle = crate::model_bundled_runtime::Bundle::open(&bundle_root).unwrap();
    let recipe = bundle.recipe().unwrap();
    bundle.prepare(&bundle_root, &recipe, &|| false).unwrap();
    trial_recipe(
        "whisper-194-small-multilingual",
        recipe,
        &|url| {
            if url.ends_with("ggml-small.bin") {
                payload.join("models/ggml-small.bin")
            } else if url.ends_with("README.md") {
                payload.join("notices/whisper-model-card.md")
            } else {
                assert!(url.ends_with("/LICENSE"));
                payload.join("notices/openai-whisper-MIT.txt")
            }
        },
        4,
        Some(bundle),
    );
}

fn trial_recipe(
    choice_id: &str,
    recipe: Recipe,
    local: &impl Fn(&str) -> PathBuf,
    expected_calls: usize,
    mut bundle: Option<crate::model_bundled_runtime::Bundle>,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let start = Instant::now();
    let mut calls = 0;
    let receipt = stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |_, _| {},
        &mut |url, expected_size, expected_hash, output, cancelled, report| {
            calls += 1;
            if url == crate::model_download_recipe::BUNDLED_WHISPER {
                return bundle
                    .as_mut()
                    .ok_or("同梱ファイルの事前照合がありません")?
                    .fetch(url, expected_size, expected_hash, output, cancelled);
            }
            let path = local(url);
            let mut source = Source::open(
                path.parent().unwrap(),
                path.file_name().unwrap().to_str().unwrap(),
            )?;
            let mut digest = Sha256::new();
            let mut count = 0_u64;
            let mut buffer = [0_u8; 65_536];
            loop {
                if cancelled() {
                    return Err("CPU試行の素材コピーを中断しました".into());
                }
                let n = source.file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > expected_size {
                    return Err("元素材容量が不一致です".into());
                }
                digest.update(&buffer[..n]);
                output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                report(count);
            }
            source.verify()?;
            if count != expected_size || format!("{:x}", digest.finalize()) != expected_hash {
                return Err("元素材SHAが不一致です".into());
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls, expected_calls);
    println!(
        "{choice_id} local real-asset stage: {:.3}s; {} payload bytes",
        start.elapsed().as_secs_f64(),
        receipt.report.total_bytes
    );
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
    let start = Instant::now();
    place(
        &storage,
        &models,
        &root,
        &operation,
        &recipe.hash().unwrap(),
        &package,
        &receipt.report.review_hash,
        &|| false,
    )
    .unwrap();
    println!(
        "real-asset managed copy: {:.3}s",
        start.elapsed().as_secs_f64()
    );
    let start = Instant::now();
    let check_id = id();
    let result = crate::model_compatibility::check(
        &storage,
        &models,
        &check_id,
        &package,
        &receipt.report.review_hash,
        &|| false,
    )
    .unwrap();
    if result.state != crate::model_compatibility::CheckState::Succeeded {
        // This opt-in trial has synthetic prompts only. Preserve diagnostics in
        // the test output before its private temporary copies are reclaimed.
        let log = temp
            .path()
            .join("model-checks")
            .join(&check_id)
            .join("stderr.log");
        if let Ok(file) = std::fs::File::open(log) {
            let mut bytes = vec![];
            file.take(4 * 1024 * 1024).read_to_end(&mut bytes).unwrap();
            eprintln!(
                "synthetic CPU trial stderr: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
    assert_eq!(
        result.state,
        crate::model_compatibility::CheckState::Succeeded,
        "{:?}",
        result
    );
    println!(
        "synthetic CPU trial: {:.3}s; no real meeting",
        start.elapsed().as_secs_f64()
    );
    assert_eq!(
        storage.list_model_placements().unwrap()[0].source_type,
        "download"
    );
    let profiles = storage.model_profiles().unwrap();
    let preview = crate::model_application::preview(
        &storage,
        &models,
        crate::storage::model_profiles::ProfileName::Standard,
        &package,
        &receipt.report.review_hash,
        &check_id,
        &|| false,
    )
    .unwrap();
    let applied =
        crate::model_application::apply(&storage, &models, &id(), &preview, &|| false).unwrap();
    assert_eq!(applied.components.len(), 1);
    assert!(
        !applied.configuration_complete,
        "one engine is not all three ready"
    );
    assert_eq!(
        storage.model_profiles().unwrap()[1],
        profiles[1],
        "high accuracy stays unchanged"
    );
    assert!(storage.list_meetings().unwrap().is_empty());
    for (entry, source) in recipe.manifest.files.iter().zip(&recipe.sources) {
        if entry.role == crate::model_inspection::Role::Notice && source.archive.is_none() {
            assert_eq!(
                std::fs::read(models.join(&package).join(&entry.relative_path)).unwrap(),
                std::fs::read(local(&source.url)).unwrap()
            );
        }
    }
    println!(
        "test-owned profile applied; original assets, other profile and real meetings untouched"
    );
    // Only test-owned copies are reclaimed by TempDir. Originals stay unchanged.
}
