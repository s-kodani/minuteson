use super::*;

#[test]
#[ignore = "requires official local assets and explicit permission for a synthetic CPU trial"]
fn real_llama_catalog_archive_copy_and_cpu_with_reused_assets() {
    llama_trial("llama-b11533-qwen25-15b-q4km");
}

#[test]
#[ignore = "requires official reused assets and explicit synthetic CPU/Markdown generation permission"]
fn real_qwen05_catalog_copy_cpu_apply_and_markdown_roundtrip_with_reused_assets() {
    llama_trial("llama-b11533-qwen25-05b-q4km");
}

#[test]
#[ignore = "requires fixed official Q8 assets and explicit synthetic CPU/Markdown generation permission"]
fn real_qwen_q8_catalog_copy_cpu_apply_and_markdown_roundtrip_with_reused_assets() {
    llama_trial("llama-b11533-qwen25-15b-q8");
}

fn llama_trial(choice: &str) {
    let archive_path =
        PathBuf::from(std::env::var("MON_QA_LLAMA_ARCHIVE").expect("official archive path"));
    let model_path =
        PathBuf::from(std::env::var("MON_QA_QWEN_MODEL").expect("existing Qwen model path"));
    let license_path = PathBuf::from(
        std::env::var("MON_QA_QWEN_LICENSE").expect("fixed official Qwen license path"),
    );
    trial(
        choice,
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
    sherpa_trial("sherpa-1138-pyannote3-eres2net-base");
}

#[test]
#[ignore = "requires fixed official int8 assets and explicit synthetic CPU permission"]
fn real_sherpa_int8_catalog_copy_cpu_and_apply_with_reused_assets() {
    sherpa_trial("sherpa-1138-pyannote3-int8-eres2net-base");
}

fn sherpa_trial(choice: &str) {
    let assets =
        PathBuf::from(std::env::var("MON_QA_SHERPA_ASSETS").expect("existing sherpa assets"));
    let notices =
        PathBuf::from(std::env::var("MON_QA_SHERPA_NOTICES").expect("fixed official notices"));
    trial(
        choice,
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
    trial_recipe(choice_id, recipe, local, expected_calls, None, None);
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
        None,
    );
}

#[test]
#[ignore = "requires all four fixed Whisper assets, the app-owned runtime, and explicit synthetic CPU permission"]
fn real_all_whisper_variants_copy_cpu_and_apply_with_reused_assets() {
    all_whisper_trials(None);
}

#[test]
#[ignore = "requires fixed synthetic voice, official Whisper assets and bundled import helper; no recording or playback"]
fn real_all_whisper_variants_import_transcribe_publish_and_reopen_synthetic_voice() {
    let voice = PathBuf::from(std::env::var("MON_QA_SYNTHETIC_VOICE").expect("synthetic voice"));
    // Only the previously documented synthetic fixture is admitted. This is
    // not an opt-in to read arbitrary existing meetings or capture live audio.
    let proof = crate::local_engine::Fingerprint::capture(&voice, &|| false).unwrap();
    assert_eq!(
        proof.sha256,
        "7eee2b4a3ba2057234d2e0a1367b0aaf66f5aa1b0895c9cf0aaa7807f5d706be"
    );
    all_whisper_trials(Some(&voice));
    proof.verify(&|| false).unwrap();
}

fn all_whisper_trials(voice: Option<&Path>) {
    let bundle_root = PathBuf::from(std::env::var("MON_QA_ASR_BUNDLE").expect("app runtime"));
    let payload =
        PathBuf::from(std::env::var("MON_QA_ASR_PAYLOAD").expect("small payload and notices"));
    let extra =
        PathBuf::from(std::env::var("MON_QA_EXTRA_MODELS").expect("fixed base and medium assets"));
    let tiny = PathBuf::from(std::env::var("MON_QA_ASR_TINY").expect("fixed tiny asset"));
    let choices = crate::model_bundled_runtime::Bundle::open(&bundle_root)
        .unwrap()
        .recipes()
        .unwrap();
    assert_eq!(
        choices
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["tiny", "base", "small", "medium"]
    );
    // Keep only one variant's staging and managed copies alive at a time.
    // Exercise the same recipe, placement and CPU admission for every variant.
    for (name, recipe) in choices {
        let model = match name.as_str() {
            "tiny" => tiny.clone(),
            "base" | "medium" => extra.join(format!("ggml-{name}.bin")),
            "small" => payload.join("models/ggml-small.bin"),
            _ => panic!("unexpected Whisper variant"),
        };
        let mut bundle = crate::model_bundled_runtime::Bundle::open(&bundle_root).unwrap();
        bundle.prepare(&bundle_root, &recipe, &|| false).unwrap();
        trial_recipe(
            &format!("whisper-194-{name}-multilingual"),
            recipe,
            &|url| {
                if url.ends_with(&format!("ggml-{name}.bin")) {
                    model.clone()
                } else if url.ends_with("README.md") {
                    payload.join("notices/whisper-model-card.md")
                } else {
                    assert!(url.ends_with("/LICENSE"));
                    payload.join("notices/openai-whisper-MIT.txt")
                }
            },
            4,
            Some(bundle),
            voice,
        );
    }
}

fn trial_recipe(
    choice_id: &str,
    recipe: Recipe,
    local: &impl Fn(&str) -> PathBuf,
    expected_calls: usize,
    mut bundle: Option<crate::model_bundled_runtime::Bundle>,
    voice: Option<&Path>,
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
    let models = temp.path().canonicalize().unwrap().join("models");
    let storage =
        crate::storage::Storage::open_with_models(meetings.path().into(), models.clone()).unwrap();
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
    if voice.is_some() {
        // Configure the isolated test preferences before application so the
        // managed binding seals the same one-thread settings used by the job.
        let preferences = storage.analysis_preferences().unwrap();
        let mut payload = preferences.payload;
        payload.standard.threads = 1;
        payload.standard.use_gpu = false;
        storage
            .save_analysis_preferences(preferences.lock_version, payload)
            .unwrap();
    }
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
    if let Some(voice) = voice {
        assert_eq!(
            recipe.manifest.engine,
            crate::model_inspection::Engine::Whisper
        );
        real_voiced_transcription(storage, models, voice, choice_id);
        return;
    }
    if recipe.manifest.engine == crate::model_inspection::Engine::Llama {
        let model = recipe
            .manifest
            .files
            .iter()
            .find(|file| file.role == crate::model_inspection::Role::Llm)
            .unwrap();
        crate::storage::markdown_generation_tests::real_managed_model_markdown_roundtrip(
            storage,
            models,
            model.expected_hash.as_deref().unwrap(),
            choice_id,
        );
    }
    // Only test-owned copies are reclaimed by TempDir. Originals stay unchanged.
}

fn real_voiced_transcription(
    storage: crate::storage::Storage,
    models: PathBuf,
    voice: &Path,
    label: &str,
) {
    use crate::{audio_import, local_engine::Fingerprint, transcription};
    let helper = Fingerprint::capture(
        &PathBuf::from(std::env::var("MON_QA_IMPORT_HELPER").expect("bundled import helper")),
        &|| false,
    )
    .unwrap();
    assert_eq!(
        helper.sha256,
        "52841f2e260f9c010fdac5472b0b3849fc1b10eb27eb7f95fb6cc5c9612752c8"
    );
    let started = Instant::now();
    let selected =
        audio_import::inspect(voice, &helper, storage.import_scratch().unwrap(), &|| false)
            .unwrap();
    assert_eq!(
        (selected.info.sample_rate, selected.info.channels),
        (16000, 1)
    );
    assert_eq!(selected.info.duration_us, 22_147_125);
    let imported = audio_import::import(
        &storage,
        selected,
        &helper,
        "合成発話の実モデル検証",
        &id(),
        &|| false,
        &|_, _| {},
    )
    .unwrap();
    assert_eq!(imported.state, "ready");
    let preferences = storage.analysis_preferences().unwrap();
    let profiles = storage.model_profiles().unwrap();
    let engine = &preferences.payload.standard;
    let job = transcription::StartRequest {
        meeting_id: imported.meeting_id.clone(),
        operation_id: id(),
        executable_path: engine.executable_path.clone().into(),
        model_path: engine.model_path.clone().into(),
        vad_model_path: engine.vad_model_path.clone().map(Into::into),
        threads: engine.threads,
        use_gpu: engine.use_gpu,
        profile: "standard".into(),
        prompt: String::new(),
        expected_vocabulary: None,
    }
    .enqueue(&storage)
    .unwrap();
    assert!(!engine.use_gpu);
    assert_eq!(engine.threads, 1);
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    assert!(
        crate::storage::model_profiles::jobs::read(&spec.profile_snapshot)
            .unwrap()
            .is_some(),
        "managed proof must not be bypassed"
    );
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    let candidate = transcription::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
    let view = storage
        .load_visible_analysis(&imported.meeting_id, &candidate)
        .unwrap()
        .unwrap();
    assert!(!view.utterances.is_empty());
    assert!(view
        .utterances
        .iter()
        .all(|u| !u.effective_text.trim().is_empty()
            && u.start_us >= 0
            && u.end_us > u.start_us
            && u.end_us <= imported.info.duration_us));
    assert!(storage
        .load_current_analysis(&imported.meeting_id)
        .unwrap()
        .is_none());
    let meeting = storage.list_meetings().unwrap().pop().unwrap();
    storage
        .commit_analysis_candidate(&meeting.id, &candidate, meeting.lock_version, false)
        .unwrap();
    let published = storage.load_current_analysis(&meeting.id).unwrap().unwrap();
    assert_eq!(published.utterances, view.utterances);
    let root = storage.root().to_owned();
    drop(storage);
    let reopened = crate::storage::Storage::open_with_models(root, models).unwrap();
    let restored = reopened
        .load_current_analysis(&meeting.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.content_hash, published.content_hash);
    assert_eq!(restored.utterances, published.utterances);
    assert_eq!(reopened.model_profiles().unwrap(), profiles);
    let after = reopened.analysis_preferences().unwrap();
    assert_eq!(after.payload, preferences.payload);
    assert_eq!(after.lock_version, preferences.lock_version);
    assert_eq!(reopened.get_job(&job.id).unwrap().state, "succeeded");
    println!("{label} synthetic voiced import/ASR/publication/reopen: {:.3}s; {} utterances; not a quality score",
        started.elapsed().as_secs_f64(), restored.utterances.len());
}
