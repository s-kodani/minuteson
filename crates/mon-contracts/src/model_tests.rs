use crate::{PackageManifest, Recipe};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn recipe_and_manifest_bytes_retain_the_legacy_golden_hashes() {
    // The same input/canonical/hash vectors are consumed by the generated UI
    // structural validators and canonicalRecipe tests. They describe wire
    // values, not authorization to download or execute a model on this host.
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../tests/contracts/model-recipes.json")).unwrap();
    for vector in fixtures["vectors"].as_array().unwrap() {
        let recipe: Recipe = serde_json::from_value(vector["input"].clone()).unwrap();
        let canonical = serde_json::to_string(&recipe).unwrap();
        assert_eq!(
            canonical,
            vector["canonical"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(canonical.as_bytes())),
            vector["sha256"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        let manifest = serde_json::to_string(&recipe.manifest).unwrap();
        assert_eq!(
            manifest,
            vector["manifestCanonical"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(manifest.as_bytes())),
            vector["manifestSha256"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(serde_json::from_str::<Recipe>(&canonical).unwrap(), recipe);
    }
    for vector in fixtures["invalid"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<Recipe>(vector["input"].clone()).is_err(),
            "{}",
            vector["name"]
        );
    }
}

#[test]
fn optional_inspection_fields_keep_null_while_backend_and_archive_none_are_omitted() {
    let manifest: PackageManifest = serde_json::from_value(serde_json::json!({
        "schemaVersion":1,"engine":"llama.cpp","engineVersion":"fixed","modelName":"manual",
        "modelVersion":"fixed","licenseRef":"notice","target":"macos_aarch64",
        "files":[{"relativePath":"model.gguf","role":"llm"}],"backend":null
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_string(&manifest).unwrap(),
        r#"{"schemaVersion":1,"engine":"llama.cpp","engineVersion":"fixed","modelName":"manual","modelVersion":"fixed","quantization":null,"licenseRef":"notice","target":"macos_aarch64","files":[{"relativePath":"model.gguf","role":"llm","expectedSize":null,"expectedHash":null}]}"#
    );
}
