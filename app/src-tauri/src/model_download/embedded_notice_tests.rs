//! Real embedded notice bytes through the production staging/readback boundary.
//! Model/runtime payloads are synthetic and are never executed.
use super::*;
use crate::model_inspection::{PackageFile, Role, Target};
use std::sync::atomic::{AtomicBool, Ordering};

fn fixture(lightweight: bool) -> (Recipe, Vec<(String, Vec<u8>)>) {
    let choices: Vec<crate::model_asr_catalog::Choice> =
        serde_json::from_str(include_str!("../../../model-catalog/asr-transducer.json")).unwrap();
    let mut recipe = choices
        .into_iter()
        .find(|choice| {
            choice
                .id
                .ends_with(if lightweight { "int8-fp32" } else { "-fp32" })
                && (lightweight || !choice.id.contains("int8"))
        })
        .unwrap()
        .recipe;
    recipe.manifest.target = Target::current().unwrap();
    let mut payloads = Vec::new();
    for (file, source) in recipe.manifest.files.iter_mut().zip(&mut recipe.sources) {
        if file.role != Role::Notice {
            let bytes = format!("synthetic payload: {}", file.relative_path).into_bytes();
            file.expected_size = Some(bytes.len() as u64);
            file.expected_hash = Some(format!("{:x}", Sha256::digest(&bytes)));
            // Exercise staging only; archive expansion has independent tests.
            source.archive = None;
            source.url = format!("https://fixture.invalid/{}", file.relative_path);
            payloads.push((source.url.clone(), bytes));
        }
    }
    let bin = b"synthetic transducer worker, never executed".to_vec();
    recipe.manifest.files.insert(
        0,
        PackageFile {
            relative_path: "bin/fixture-worker".into(),
            role: Role::Executable,
            expected_size: Some(bin.len() as u64),
            expected_hash: Some(format!("{:x}", Sha256::digest(&bin))),
        },
    );
    recipe.sources.insert(
        0,
        crate::model_download_recipe::DownloadSource {
            relative_path: "bin/fixture-worker".into(),
            url: "https://fixture.invalid/worker".into(),
            archive: None,
        },
    );
    payloads.push(("https://fixture.invalid/worker".into(), bin));
    recipe.validate().unwrap();
    (recipe, payloads)
}

#[test]
fn reazonspeech_variants_stage_embedded_notices_without_provider_requests() {
    for lightweight in [true, false] {
        let (recipe, payloads) = fixture(lightweight);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("downloads");
        let operation = uuid::Uuid::new_v4().to_string();
        let hash = recipe.hash().unwrap();
        let bytes = recipe.bytes().unwrap();
        let total: u64 = payloads.iter().map(|(_, bytes)| bytes.len() as u64).sum();
        assert_eq!(recipe.transfer_bytes().unwrap(), total);
        let mut progress = Vec::new();
        let mut requests = Vec::new();
        let receipt = stage(
            &root,
            &operation,
            &recipe,
            &hash,
            &|| false,
            &mut |done, all| progress.push((done, all)),
            &mut |url, _, _, output, _, report| {
                requests.push(url.to_owned());
                // Any document request would fail exactly as the unavailable
                // ONNX provider does. Embedded notices must bypass transport.
                let Some((_, bytes)) = payloads.iter().find(|(source, _)| source == url) else {
                    return Err(mon_native::transport::HTTP_FAILURE.into());
                };
                output.write_all(bytes).map_err(|e| e.to_string())?;
                report(bytes.len() as u64);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(requests.len(), payloads.len());
        let mut notices = 0;
        for (file, source) in recipe.manifest.files.iter().zip(&recipe.sources) {
            if file.role == Role::Notice {
                let expected = crate::model_license::embedded_download_notice(file, source)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    std::fs::read(receipt.directory.join(&file.relative_path)).unwrap(),
                    expected
                );
                notices += 1;
            }
        }
        assert_eq!(notices, 5);
        assert_eq!(progress.last(), Some(&(total, total)));
        assert!(progress
            .iter()
            .all(|(done, all)| done <= all && *all == total));
        assert!(progress.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(recipe.bytes().unwrap(), bytes);
        assert_eq!(receipt.recipe_hash, hash);
        assert_eq!(receipt.report.total_bytes, recipe.validate().unwrap());
        let recovered = readback(&root, &operation, &hash, &|| false).unwrap();
        assert_eq!(recovered.state, "complete");
        assert_eq!(recovered.review.download_bytes, total);
        assert_eq!(
            recovered.receipt.unwrap().report.review_hash,
            receipt.report.review_hash
        );
    }
}

#[test]
fn embedded_notice_placement_obeys_cancellation_without_network_or_completion() {
    let (mut recipe, _) = fixture(true);
    let index = recipe
        .manifest
        .files
        .iter()
        .position(|file| file.role == Role::Notice)
        .unwrap();
    let notice = recipe.manifest.files.remove(index);
    let source = recipe.sources.remove(index);
    let relative = notice.relative_path.clone();
    recipe.manifest.files.insert(0, notice);
    recipe.sources.insert(0, source);
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("downloads");
    let operation = uuid::Uuid::new_v4().to_string();
    let cancelled = AtomicBool::new(false);
    let mut events = 0;
    let error = stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |done, _| {
            assert_eq!(done, 0);
            events += 1;
            if events == 2 {
                cancelled.store(true, Ordering::SeqCst);
            }
        },
        &mut |_, _, _, _, _, _| panic!("cancelled preparation must not contact a provider"),
    )
    .unwrap_err();
    assert!(error.contains("中断"), "{error}");
    assert!(root
        .join(&operation)
        .join("payload")
        .join(relative)
        .is_file());
    assert!(!root.join(&operation).join(COMPLETE).exists());
    assert_eq!(
        readback(&root, &operation, &recipe.hash().unwrap(), &|| false)
            .unwrap()
            .state,
        "incomplete"
    );
}
