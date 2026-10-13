use super::*;
use serde_json::{json, Value};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>, Value) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("bundle");
    std::fs::create_dir(&root).unwrap();
    let bytes = b"synthetic non-executable archive bytes".to_vec();
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../model-catalog/whisper-runtime-build.json"
    ))
    .unwrap();
    value["archive"] =
        json!({"file":"whisper-runtime.tar","bytes":bytes.len(),"sha256":hash(&bytes)});
    value["files"] = json!(MEMBERS
        .iter()
        .map(|path| json!({"relativePath":path,"bytes":16,"sha256":hash(b"synthetic member")}))
        .collect::<Vec<_>>());
    value["runtimeSigning"] = json!("ad_hoc_only");
    std::fs::write(
        root.join("runtime-index.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("whisper-runtime.tar"), &bytes).unwrap();
    (temp, root, bytes, value)
}
#[test]
fn read_only_catalog_and_review_separate_bundle_network_and_unverified_readiness() {
    let (temp, root, bytes, _) = fixture();
    let bundle = Bundle::open(&root).unwrap();
    assert!(bundle.archive.is_none());
    let catalog =
        serde_json::to_value(crate::model_official_catalog::options_with_bundle(&root).unwrap())
            .unwrap();
    assert_eq!(catalog.as_array().unwrap().len(), 6);
    assert_eq!(catalog[0]["label"], "Whisper tiny（多言語）");
    assert_eq!(catalog[0]["review"]["bundledBytes"], bytes.len());
    assert_eq!(
        catalog[4]["review"]["recipeHash"],
        "e4aaf7c71da23a1cadc141d92208d6eac4274eb12fa1901383f7753234f210ba"
    );
    assert_eq!(
        catalog[5]["review"]["recipeHash"],
        "0996bb77a0fab2f604c97af2bde017cebe813e83529228be92778c4befd4a7c8"
    );
    let recipe = bundle.recipe().unwrap();
    let review = recipe.review().unwrap();
    assert_eq!(review.recipe_schema_version, Some(3));
    assert_eq!(review.download_bytes, Some(487_606_226));
    assert_eq!(review.bundled_bytes, Some(bytes.len() as u64));
    assert_eq!(review.archive_bytes, review.bundled_bytes);
    assert_eq!(review.total_bytes, 487_606_226 + 48);
    assert!(!review.installed && !review.ready);
    assert_eq!(review.publisher_authenticity, "not_verified");
    let origin = recipe.origin(&uuid::Uuid::new_v4().to_string()).unwrap();
    let manifest_hash = hash(&serde_json::to_vec(&recipe.manifest).unwrap());
    // Pure historical evidence survives without current app resources.
    drop(bundle);
    drop(temp);
    origin.validate(&recipe.manifest, &manifest_hash).unwrap();
    assert_eq!(
        serde_json::to_value(&origin).unwrap()["recipeSchemaVersion"],
        3
    );
}
#[test]
fn all_multilingual_variants_bind_exact_model_bytes_and_preserve_shared_runtime_and_notices() {
    let (_temp, root, bytes, _) = fixture();
    let bundle = Bundle::open(&root).unwrap();
    let recipes = bundle.recipes().unwrap();
    assert_eq!(recipes.len(), 4);
    let mut hashes = std::collections::BTreeSet::new();
    for (name, recipe) in &recipes {
        bundle.bind(recipe).unwrap();
        assert!(hashes.insert(recipe.hash().unwrap()));
        let model = recipe
            .manifest
            .files
            .iter()
            .find(|f| f.role == Role::Asr)
            .unwrap();
        let source = recipe
            .sources
            .iter()
            .find(|s| s.relative_path == model.relative_path)
            .unwrap();
        assert_eq!(model.relative_path, format!("models/ggml-{name}.bin"));
        assert!(source.url.ends_with(&format!("/ggml-{name}.bin")));
        assert!(source
            .url
            .contains("/5359861c739e955e79d9a303bcbc70fb988958b1/"));
        assert_eq!(
            recipe.review().unwrap().bundled_bytes,
            Some(bytes.len() as u64)
        );
        assert!(recipe.transfer_bytes().unwrap() < 2 * 1024 * 1024 * 1024);
        for shared in [
            "bin/whisper-cli",
            "notices/whisper-MIT.txt",
            "build-info.json",
            "notices/whisper-model-card.md",
            "notices/openai-whisper-MIT.txt",
        ] {
            assert_eq!(
                recipe
                    .manifest
                    .files
                    .iter()
                    .find(|f| f.relative_path == shared),
                recipes[0]
                    .1
                    .manifest
                    .files
                    .iter()
                    .find(|f| f.relative_path == shared)
            );
        }
        for field in ["hash", "size", "url", "name"] {
            let mut forged = recipe.clone();
            match field {
                "hash" => forged.manifest.files[3].expected_hash = Some("0".repeat(64)),
                "size" => forged.manifest.files[3].expected_size = Some(1),
                "url" => forged.sources[3].url = source.url.replace("/resolve/", "/resolve/main/"),
                "name" => forged.manifest.model_name = "arbitrary model".into(),
                _ => unreachable!(),
            }
            assert!(bundle.bind(&forged).is_err(), "{name}:{field}");
        }
        let mut preparing = Bundle::open(&root).unwrap();
        assert!(preparing.prepare(&root, recipe, &|| true).is_err());
        assert!(preparing.archive.is_none());
        preparing.prepare(&root, recipe, &|| false).unwrap();
    }
    assert!(bundle.recipe_for("tiny.en").is_err());
    // The already shipped small recipe keeps its model provenance unchanged.
    let small = recipes.iter().find(|(name, _)| name == "small").unwrap();
    assert_eq!(small.1, bundle.recipe().unwrap());
}
#[test]
fn index_rejects_changed_version_options_types_paths_sha_unknown_and_oversize() {
    for variant in [
        "version",
        "options",
        "integer-bool",
        "target",
        "archive-path",
        "archive-size",
        "archive-sha",
        "member-path",
        "member-size",
        "member-sha",
        "unknown",
        "signing",
        "oversize",
    ] {
        let (_temp, root, _, mut value) = fixture();
        match variant {
            "version" => value["engineVersion"] = json!("v0"),
            "options" => value["buildOptions"][0] = json!("-DGGML_METAL=ON"),
            "integer-bool" => value["cpuOnly"] = json!(1),
            "target" => value["target"] = json!("windows_x86_64"),
            "archive-path" => value["archive"]["file"] = json!("../outside"),
            "archive-size" => value["archive"]["bytes"] = json!(MAX_PACKAGE + 1),
            "archive-sha" => value["archive"]["sha256"] = json!("bad"),
            "member-path" => value["files"][0]["relativePath"] = json!("bin/another-cli"),
            "member-size" => value["files"][1]["bytes"] = json!(65_537),
            "member-sha" => value["files"][0]["sha256"] = json!("bad"),
            "unknown" => value["execute"] = json!(true),
            "signing" => value["runtimeSigning"] = json!("developer_id_verified"),
            "oversize" => value["padding"] = json!("x".repeat(65_537)),
            _ => unreachable!(),
        }
        std::fs::write(
            root.join("runtime-index.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(Bundle::open(&root).is_err(), "{variant}");
    }
}
#[test]
fn acquisition_preflight_rejects_changed_archive_recipe_and_cancel_before_staging() {
    for variant in ["archive-bytes", "recipe-model", "recipe-url", "cancel"] {
        let (temp, root, _, _) = fixture();
        let bundle = Bundle::open(&root).unwrap();
        let mut recipe = bundle.recipe().unwrap();
        match variant {
            "archive-bytes" => {
                std::fs::write(root.join("whisper-runtime.tar"), b"changed").unwrap()
            }
            "recipe-model" => recipe.manifest.files[3].expected_hash = Some(hash(b"changed")),
            "recipe-url" => recipe.sources[3].url = "https://example.com/model".into(),
            _ => {}
        }
        let staging = temp.path().canonicalize().unwrap().join("downloads");
        let result = crate::model_download::download_with_bundle(
            &staging,
            &root,
            &uuid::Uuid::new_v4().to_string(),
            &recipe,
            &recipe.hash().unwrap(),
            &|| variant == "cancel",
            &mut |_, _| {},
        );
        assert!(result.is_err(), "{variant}");
        assert!(!staging.exists());
    }
}
#[test]
fn developer_id_packaging_metadata_keeps_unverified_trust_and_binds_changed_payload() {
    let (temp, root, _, mut value) = fixture();
    let original = Bundle::open(&root).unwrap().recipe().unwrap();
    let bytes = b"synthetic re-signed archive; never executed";
    value["runtimeSigning"] = json!("developer_id_signed");
    value["signingTeamId"] = json!("ABCDE12345");
    value["archive"]["bytes"] = json!(bytes.len());
    value["archive"]["sha256"] = json!(hash(bytes));
    value["files"][0]["bytes"] = json!(32);
    value["files"][0]["sha256"] = json!(hash(b"synthetic re-signed CLI"));
    std::fs::write(root.join("whisper-runtime.tar"), bytes).unwrap();
    std::fs::write(
        root.join("runtime-index.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let mut bundle = Bundle::open(&root).unwrap();
    let recipe = bundle.recipe().unwrap();
    assert_ne!(recipe.hash().unwrap(), original.hash().unwrap());
    assert!(bundle.bind(&original).is_err());
    assert_eq!(recipe.manifest.files[1..], original.manifest.files[1..]);
    let review = recipe.review().unwrap();
    assert_eq!(review.publisher_authenticity, "not_verified");
    assert!(!review.ready && !review.installed);
    bundle.prepare(&root, &recipe, &|| false).unwrap();
    let mut output = std::fs::File::create(temp.path().join("checked-copy")).unwrap();
    bundle
        .fetch(
            BUNDLED_WHISPER,
            bytes.len() as u64,
            &hash(bytes),
            &mut output,
            &|| false,
        )
        .unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("checked-copy")).unwrap(),
        bytes
    );
    std::fs::write(root.join("whisper-runtime.tar"), b"tampered").unwrap();
    assert!(bundle
        .fetch(
            BUNDLED_WHISPER,
            bytes.len() as u64,
            &hash(bytes),
            &mut output,
            &|| false
        )
        .is_err());
}
#[test]
fn signing_team_state_shape_never_accepts_null_wrong_team_or_self_approved_provenance() {
    for (state, team) in [
        ("ad_hoc_only", Some(json!("ABCDE12345"))),
        ("ad_hoc_only", Some(Value::Null)),
        ("developer_id_signed", None),
        ("developer_id_signed", Some(Value::Null)),
        ("developer_id_signed", Some(json!(1234567890))),
        ("developer_id_signed", Some(json!("abcde12345"))),
        ("developer_id_signed", Some(json!("ABCDE1234"))),
        ("developer_id_verified", Some(json!("ABCDE12345"))),
    ] {
        let (_temp, root, _, mut value) = fixture();
        value["runtimeSigning"] = json!(state);
        if let Some(team) = team {
            value["signingTeamId"] = team;
        }
        std::fs::write(
            root.join("runtime-index.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(
            Bundle::open(&root).is_err(),
            "{state}: {}",
            value.get("signingTeamId").unwrap_or(&Value::Null)
        );
    }
    for (key, changed) in [
        ("publisherAuthenticity", json!("verified")),
        ("distributionReady", json!(true)),
        ("licenseReview", json!("approved")),
    ] {
        let (_temp, root, _, mut value) = fixture();
        value["runtimeSigning"] = json!("developer_id_signed");
        value["signingTeamId"] = json!("ABCDE12345");
        value[key] = changed;
        std::fs::write(
            root.join("runtime-index.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(Bundle::open(&root).is_err(), "{key}");
    }
}
#[test]
fn native_bundle_copy_checks_identity_size_sha_and_never_runs_or_reports_network() {
    let (temp, root, bytes, _) = fixture();
    let mut bundle = Bundle::open(&root).unwrap();
    let recipe = bundle.recipe().unwrap();
    bundle.prepare(&root, &recipe, &|| false).unwrap();
    let output_path = temp.path().join("copy");
    let mut output = std::fs::File::create(&output_path).unwrap();
    bundle
        .fetch(
            BUNDLED_WHISPER,
            bytes.len() as u64,
            &hash(&bytes),
            &mut output,
            &|| false,
        )
        .unwrap();
    assert_eq!(std::fs::read(output_path).unwrap(), bytes);
    for (url, size, sha) in [
        ("file:///etc/passwd", bytes.len() as u64, hash(&bytes)),
        (BUNDLED_WHISPER, bytes.len() as u64 + 1, hash(&bytes)),
        (BUNDLED_WHISPER, bytes.len() as u64, hash(b"changed")),
    ] {
        assert!(bundle
            .fetch(url, size, &sha, &mut output, &|| false)
            .is_err());
    }
    assert!(bundle
        .fetch(
            BUNDLED_WHISPER,
            bytes.len() as u64,
            &hash(&bytes),
            &mut output,
            &|| true
        )
        .is_err());
    std::fs::write(root.join("whisper-runtime.tar"), b"changed").unwrap();
    assert!(bundle
        .fetch(
            BUNDLED_WHISPER,
            bytes.len() as u64,
            &hash(&bytes),
            &mut output,
            &|| false
        )
        .is_err());
}
#[test]
fn changed_index_after_review_and_symlinked_index_archive_or_root_are_rejected() {
    use std::os::unix::fs::symlink;
    for variant in ["index-change", "index-link", "archive-link", "root-link"] {
        let (temp, root, _, mut value) = fixture();
        let mut bundle = Bundle::open(&root).unwrap();
        let recipe = bundle.recipe().unwrap();
        match variant {
            "index-change" => {
                value["files"][0]["bytes"] = json!(17);
                std::fs::write(
                    root.join("runtime-index.json"),
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
            }
            "index-link" => {
                std::fs::rename(
                    root.join("runtime-index.json"),
                    temp.path().join("outside-index"),
                )
                .unwrap();
                symlink(
                    temp.path().join("outside-index"),
                    root.join("runtime-index.json"),
                )
                .unwrap();
            }
            "archive-link" => {
                std::fs::rename(
                    root.join("whisper-runtime.tar"),
                    temp.path().join("outside-archive"),
                )
                .unwrap();
                symlink(
                    temp.path().join("outside-archive"),
                    root.join("whisper-runtime.tar"),
                )
                .unwrap();
            }
            "root-link" => {
                let link = temp.path().join("bundle-link");
                symlink(&root, &link).unwrap();
                assert!(Bundle::open(&link).is_err());
                continue;
            }
            _ => unreachable!(),
        }
        assert!(
            bundle.prepare(&root, &recipe, &|| false).is_err(),
            "{variant}"
        );
        if variant == "index-change" || variant == "index-link" {
            assert!(bundle.recipe().is_err());
        }
    }
}
#[test]
fn bundled_identifiers_are_not_generic_local_urls_or_legacy_network_sources() {
    let (_temp, root, _, _) = fixture();
    let recipe = Bundle::open(&root).unwrap().recipe().unwrap();
    for variant in [
        "schema1",
        "schema2",
        "unknown-id",
        "traversal",
        "raw",
        "wrong-role",
        "network-only",
        "no-network-asr",
    ] {
        let mut value = serde_json::to_value(&recipe).unwrap();
        match variant {
            "schema1" => value["schemaVersion"] = json!(1),
            "schema2" => value["schemaVersion"] = json!(2),
            "unknown-id" => value["sources"][0]["url"] = json!("mon-bundled://other/runtime.tar"),
            "traversal" => value["sources"][0]["archive"]["memberPath"] = json!("../outside"),
            "raw" => {
                value["sources"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("archive");
            }
            "wrong-role" => value["manifest"]["files"][0]["role"] = json!("notice"),
            "network-only" => {
                for source in value["sources"].as_array_mut().unwrap() {
                    source["url"] = json!("https://example.com/archive");
                }
            }
            "no-network-asr" => value["sources"][3]["url"] = json!(BUNDLED_WHISPER),
            _ => unreachable!(),
        }
        assert!(
            Recipe::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{variant}"
        );
    }
}
