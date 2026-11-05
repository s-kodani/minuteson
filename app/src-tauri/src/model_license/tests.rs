use super::*;
fn fixed() -> Vec<PackageManifest> {
    [
        include_str!("../../../model-catalog/macos.json"),
        include_str!("../../../model-catalog/asr-transducer.json"),
    ]
    .into_iter()
    .flat_map(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).unwrap())
    .map(|c| serde_json::from_value(c["recipe"]["manifest"].clone()).unwrap())
    .collect()
}
fn qwen() -> PackageManifest {
    fixed()
        .into_iter()
        .find(|manifest| manifest.model_name == "Qwen3-4B")
        .unwrap()
}
#[test]
fn every_fixed_model_has_offline_terms_bound_to_actual_weight_identity() {
    for manifest in fixed() {
        let disclosure = official(&manifest).unwrap().unwrap();
        assert!(disclosure.official);
        assert!(!disclosure.notices.is_empty());
        assert_eq!(disclosure.disclosure_hash.len(), 64);
        let mut changed = manifest.clone();
        changed
            .files
            .iter_mut()
            .find(|f| {
                f.role != Role::Notice
                    && f.role != Role::Executable
                    && f.role != Role::RuntimeLibrary
            })
            .unwrap()
            .expected_hash = Some("a".repeat(64));
        assert!(official(&changed).unwrap().is_none());
        let mut changed = manifest.clone();
        changed.files.retain(|f| f.role != Role::Notice);
        assert!(official(&changed).unwrap().is_none());
        assert!(validate_download_policy(&changed).is_err());
    }
}
#[test]
fn all_dynamic_whisper_weights_resolve_without_runtime_or_recipe_identity_changes() {
    let network: serde_json::Value =
        serde_json::from_str(include_str!("../../../model-catalog/whisper-network.json")).unwrap();
    let vad: serde_json::Value = serde_json::from_str(include_str!(
        "../../../model-catalog/whisper-vad-network.json"
    ))
    .unwrap();
    let variants: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../model-catalog/whisper-variants.json")).unwrap();
    assert_eq!(variants.len(), 3);
    for variant in variants {
        let mut manifest = qwen();
        manifest.engine = Engine::Whisper;
        manifest.files = serde_json::from_value(network["files"].clone()).unwrap();
        manifest
            .files
            .extend(serde_json::from_value::<Vec<PackageFile>>(vad["files"].clone()).unwrap());
        manifest.files[0].expected_size = variant["bytes"].as_u64();
        manifest.files[0].expected_hash = variant["sha256"].as_str().map(str::to_owned);
        assert!(official(&manifest).unwrap().is_some());
    }
}
#[test]
fn preserved_notice_bytes_and_sources_are_checked_and_complete() {
    let registry = registry().unwrap();
    assert_eq!(registry.models.len(), 12);
    assert_eq!(registry.notices.len(), 19);
    for record in &registry.notices {
        let bytes = assets::embedded(&record.sha256).unwrap();
        let text = verify_notice(record, bytes).unwrap();
        assert_eq!(digest(text.as_bytes()), record.sha256);
        let mut corrupt = bytes.to_vec();
        corrupt[0] ^= 1;
        assert!(verify_notice(record, &corrupt).is_err());
    }
    let mut corrupt: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../model-catalog/model-license-policy.json"
    ))
    .unwrap();
    corrupt["notices"][0]["bytes"] = 1.into();
    assert!(Registry::parse(&serde_json::to_vec(&corrupt).unwrap()).is_err());
}
#[test]
fn unknown_installed_package_reports_declaration_without_guessing_rights() {
    let mut manifest = qwen();
    manifest.files[0].expected_hash = Some("f".repeat(64)); // runtime alone does not change model license
    let weight = manifest
        .files
        .iter_mut()
        .find(|f| f.role == Role::Llm)
        .unwrap();
    weight.expected_hash = Some("f".repeat(64));
    manifest.license_ref = "提供者独自の条件: unknown".into();
    let review = installed(&manifest, &fixed()).unwrap();
    assert!(!review.official);
    assert_eq!(review.license_names, ["条件未確認"]);
    assert_eq!(review.notices[0].text, manifest.license_ref);
    assert_eq!(review.notices[0].source_url, "");
    assert_eq!(
        review.disclosure_hash,
        "0680295d5b30b478966e7af531723613f6eff790caed0535a9b7f7c631f45664"
    );
    assert!(validate_download_policy(&manifest).is_ok());
}
#[test]
fn disclosure_hash_binds_summary_restrictions_text_and_source_but_not_action_binding() {
    let manifest = qwen();
    let original = official(&manifest).unwrap().unwrap().disclosure_hash;
    for mutation in 0..4 {
        let mut d = official(&manifest).unwrap().unwrap();
        match mutation {
            0 => d.summary.push('!'),
            1 => d.restrictions.push("条件".into()),
            2 => d.notices[0].text.push('!'),
            _ => d.notices[0].source_url.push_str("/changed"),
        }
        assert_ne!(d.bind_hash().unwrap().disclosure_hash, original);
    }
    for invalid in [
        serde_json::json!({"recipeHash":"a".repeat(64),"terms":"injected"}),
        serde_json::json!({"recipeHash":"a".repeat(64),"packageId":"injected","expectedReviewHash":"b".repeat(64)}),
    ] {
        assert!(serde_json::from_value::<LicenseTarget>(invalid).is_err());
    }
}
#[test]
fn disclosure_hash_matches_renderer_golden_with_original_notice_bytes() {
    let manifest = qwen();
    assert_eq!(
        official(&manifest).unwrap().unwrap().disclosure_hash,
        "77b6234add679035e883adc9955877111dd475233e2897fe7d3d7456b9570de3"
    );
    let gemma = fixed()
        .into_iter()
        .find(|m| m.model_name == "gemma-4-E4B-it")
        .unwrap();
    let disclosure = official(&gemma).unwrap().unwrap();
    let notice = disclosure
        .notices
        .iter()
        .find(|n| n.name == "Gemma-Apache-2.0.txt")
        .unwrap();
    assert!(notice.source_url.contains("google/gemma_pytorch/014acb7"));
    assert!(!notice.source_url.contains("3D-Speaker"));
}

#[test]
fn license_command_role_is_main_only() {
    assert!(crate::command_roles::permits(
        "get_model_license_review",
        "main"
    ));
    for role in [
        "recorder",
        "recorder-info",
        "recording-confirmation",
        "other",
    ] {
        assert!(!crate::command_roles::permits(
            "get_model_license_review",
            role
        ));
    }
}

#[test]
fn matching_weights_with_a_non_catalog_runtime_remain_unconfirmed() {
    let mut manifest = qwen();
    assert!(installed(&manifest, &fixed()).unwrap().official);
    manifest
        .files
        .iter_mut()
        .find(|f| f.role == Role::Executable)
        .unwrap()
        .expected_hash = Some("f".repeat(64));
    assert!(official(&manifest).unwrap().is_some());
    assert!(!installed(&manifest, &fixed()).unwrap().official);
}

#[test]
fn removing_one_official_model_policy_fails_closed_before_any_transfer() {
    let mut incomplete: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../model-catalog/model-license-policy.json"
    ))
    .unwrap();
    incomplete["models"].as_array_mut().unwrap().pop();
    assert!(Registry::parse(&serde_json::to_vec(&incomplete).unwrap()).is_err());
}
