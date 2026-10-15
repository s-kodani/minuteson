use super::*;
mod real;

fn archive_recipe(bytes: &[u8]) -> Recipe {
    let mut value = recipe_value();
    value["schemaVersion"] = json!(2);
    for source in value["sources"].as_array_mut().unwrap() {
        source["url"] = json!("https://example.com/engine.tar.gz");
        source["archive"] = json!({"format":"tar","expectedSize":bytes.len(),"expectedHash":hash(bytes),"memberPath":format!("upstream/{}", source["relativePath"].as_str().unwrap())});
    }
    Recipe::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn valid() -> Vec<u8> {
    tar(&[
        ("upstream/", b"", b'5'),
        ("upstream/bin/whisper", BIN, b'0'),
        ("upstream/models/model.gguf", MODEL, b'0'),
        ("upstream/not-needed.sh", b"never-save-or-run", b'0'),
    ])
}
#[test]
fn hybrid_bundle_stage_counts_network_only_and_keeps_v3_origin_after_readback_and_copy() {
    let bytes = tar(&[("bin/whisper-cli", BIN, b'0')]);
    let mut value = recipe_value();
    value["schemaVersion"] = json!(3);
    value["manifest"]["engineVersion"] = json!("v1.9.4");
    value["sources"][0]["url"] = json!(crate::model_download_recipe::BUNDLED_WHISPER);
    value["sources"][0]["archive"] = json!({"format":"tar","expectedSize":bytes.len(),"expectedHash":hash(&bytes),"memberPath":"bin/whisper-cli"});
    value["sources"][1]["url"] = json!("https://example.com/model");
    value["sources"][1]
        .as_object_mut()
        .unwrap()
        .remove("archive");
    let recipe = Recipe::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    let review = recipe.review().unwrap();
    assert_eq!(review.download_bytes, MODEL.len() as u64);
    assert_eq!(review.bundled_bytes, Some(bytes.len() as u64));
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let mut calls = 0;
    let mut progress = vec![];
    let receipt = stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |n, total| progress.push((n, total)),
        &mut |url, _, _, file, _, report| {
            calls += 1;
            let content = if url == crate::model_download_recipe::BUNDLED_WHISPER {
                &bytes[..]
            } else {
                MODEL
            };
            file.write_all(content).unwrap();
            report(content.len() as u64);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls, 2);
    assert!(progress
        .iter()
        .all(|(n, total)| *n <= MODEL.len() as u64 && *total == MODEL.len() as u64));
    assert_eq!(
        progress.last(),
        Some(&(MODEL.len() as u64, MODEL.len() as u64))
    );
    assert_eq!(
        readback(&root, &operation, &recipe.hash().unwrap(), &|| false)
            .unwrap()
            .state,
        "complete"
    );
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
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
    let record = crate::model_placement::PlacementRecord::downloaded(
        &package,
        &receipt.report,
        recipe.origin(&operation).unwrap(),
    )
    .unwrap();
    record.validate().unwrap();
    assert_eq!(
        serde_json::to_value(record.download).unwrap()["recipeSchemaVersion"],
        3
    );
    assert_eq!(
        storage.list_model_placements().unwrap()[0].source_type,
        "download"
    );
    assert!(storage.list_meetings().unwrap().is_empty());
}
#[test]
fn archive_is_fetched_once_selected_regular_members_only_and_v2_origin_survives_copy() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = valid();
    let recipe = archive_recipe(&bytes);
    let review = recipe.review().unwrap();
    assert_eq!(review.total_bytes, (BIN.len() + MODEL.len()) as u64);
    assert_eq!(review.download_bytes, bytes.len() as u64);
    assert_eq!(review.archive_bytes, bytes.len() as u64);
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let mut calls = 0;
    let mut progress = vec![];
    let receipt = stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| false,
        &mut |n, total| progress.push((n, total)),
        &mut |_, _, _, file, _, report| {
            calls += 1;
            file.write_all(&bytes).unwrap();
            report(bytes.len() as u64);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        progress.last(),
        Some(&(bytes.len() as u64, bytes.len() as u64))
    );
    assert!(!receipt.directory.join("upstream").exists());
    assert_eq!(
        std::fs::read(receipt.directory.join("bin/whisper")).unwrap(),
        BIN
    );
    assert_eq!(
        std::fs::metadata(receipt.directory.join("bin/whisper"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        readback(&root, &operation, &recipe.hash().unwrap(), &|| false)
            .unwrap()
            .state,
        "complete"
    );
    let meetings = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(meetings.path().into()).unwrap();
    let models = temp.path().canonicalize().unwrap().join("models");
    let package = id();
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
    let record = crate::model_placement::PlacementRecord::downloaded(
        &package,
        &receipt.report,
        recipe.origin(&operation).unwrap(),
    )
    .unwrap();
    record.validate().unwrap();
    assert!(storage.list_model_placements().unwrap()[0].placement_recorded);
    let origin = serde_json::to_value(&record.download).unwrap();
    assert_eq!(origin["recipeSchemaVersion"], 2);
    assert_eq!(
        origin["sources"][0]["archive"]["expectedHash"],
        hash(&bytes)
    );
    assert_eq!(
        storage.list_model_placements().unwrap()[0].source_type,
        "download"
    );
    let mut forged = serde_json::to_value(&record).unwrap();
    forged["download"]["recipeSchemaVersion"] = json!(1);
    assert!(
        serde_json::from_value::<crate::model_placement::PlacementRecord>(forged)
            .unwrap()
            .validate()
            .is_err()
    );
}
#[test]
fn archive_rejects_unsafe_duplicate_missing_link_device_and_hash_mismatch_without_completion() {
    for variant in [
        "traversal",
        "absolute",
        "case-duplicate",
        "missing",
        "symlink",
        "hardlink",
        "device",
        "member-sha",
        "archive-sha",
        "header",
    ] {
        let mut entries = vec![
            ("upstream/bin/whisper", BIN, b'0'),
            ("upstream/models/model.gguf", MODEL, b'0'),
        ];
        match variant {
            "traversal" => entries.push(("../outside", b"bad", b'0')),
            "absolute" => entries.push(("/outside", b"bad", b'0')),
            "case-duplicate" => entries.push(("UPSTREAM/BIN/WHISPER", BIN, b'0')),
            "missing" => {
                entries.pop();
            }
            "symlink" => entries[0] = ("upstream/bin/whisper", b"", b'2'),
            "hardlink" => entries[0] = ("upstream/bin/whisper", b"", b'1'),
            "device" => entries[0] = ("upstream/bin/whisper", b"", b'3'),
            _ => {}
        }
        let mut bytes = tar(&entries);
        if variant == "header" {
            bytes[148] = b'x';
        }
        let mut recipe = archive_recipe(&bytes);
        if variant == "member-sha" {
            recipe.manifest.files[0].expected_hash = Some(hash(b"different"));
        }
        if variant == "archive-sha" {
            for s in &mut recipe.sources {
                s.archive.as_mut().unwrap().expected_hash = hash(b"different");
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let root = private_root(&temp);
        let operation = id();
        let result = stage(
            &root,
            &operation,
            &recipe,
            &recipe.hash().unwrap(),
            &|| false,
            &mut |_, _| {},
            &mut |_, _, _, file, _, _| {
                file.write_all(&bytes).unwrap();
                Ok(())
            },
        );
        assert!(result.is_err(), "{variant}");
        assert!(!root.join(&operation).join(COMPLETE).exists(), "{variant}");
        assert!(!temp.path().join("outside").exists());
        assert_eq!(
            readback(&root, &operation, &recipe.hash().unwrap(), &|| false)
                .unwrap()
                .state,
            "incomplete"
        );
    }
}
#[test]
fn archive_cancellation_after_fetch_preserves_non_executable_partial_and_never_marks_complete() {
    let bytes = valid();
    let recipe = archive_recipe(&bytes);
    let temp = tempfile::tempdir().unwrap();
    let root = private_root(&temp);
    let operation = id();
    let cancelled = AtomicBool::new(false);
    let result = stage(
        &root,
        &operation,
        &recipe,
        &recipe.hash().unwrap(),
        &|| cancelled.load(Ordering::SeqCst),
        &mut |_, _| {},
        &mut |_, _, _, file, _, _| {
            file.write_all(&bytes).unwrap();
            cancelled.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(root
        .join(&operation)
        .join("archives/source-0.tar")
        .is_file());
    assert!(!root.join(&operation).join(COMPLETE).exists());
    assert_eq!(
        readback(&root, &operation, &recipe.hash().unwrap(), &|| false)
            .unwrap()
            .state,
        "incomplete"
    );
}
#[test]
fn archive_definition_rejects_ambiguous_transfer_metadata_before_io() {
    let bytes = valid();
    let recipe = archive_recipe(&bytes);
    for variant in [
        "v1",
        "size",
        "hash",
        "member",
        "duplicate",
        "raw-url",
        "format",
    ] {
        let mut value = serde_json::to_value(&recipe).unwrap();
        match variant {
            "v1" => value["schemaVersion"] = json!(1),
            "size" => value["sources"][1]["archive"]["expectedSize"] = json!(1),
            "hash" => value["sources"][1]["archive"]["expectedHash"] = json!("a".repeat(64)),
            "member" => value["sources"][1]["archive"]["memberPath"] = json!("../escape"),
            "duplicate" => {
                value["sources"][1]["archive"]["memberPath"] =
                    value["sources"][0]["archive"]["memberPath"].clone()
            }
            "raw-url" => {
                value["sources"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("archive");
            }
            _ => value["sources"][0]["archive"]["format"] = json!("zip"),
        }
        assert!(
            Recipe::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{variant}"
        );
    }
}
