use super::*;

#[test]
#[ignore = "requires an existing Sherpa package in MON_SHERPA_PACKAGE; copies it into isolated storage"]
fn real_sherpa_package_passes_preparation_and_profile_application() {
    let source = std::path::PathBuf::from(std::env::var_os("MON_SHERPA_PACKAGE").unwrap());
    let inspected = crate::model_inspection::inspect(&source, &|| false).unwrap();
    assert_eq!(inspected.manifest.engine, Engine::Sherpa);
    let root = tempfile::tempdir().unwrap();
    let models = root.path().join("models");
    let storage = Storage::open_with_models(root.path().join("data"), models.clone()).unwrap();
    let package = uuid::Uuid::new_v4().to_string();
    crate::model_placement::place(
        &storage,
        &models,
        &source,
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let summary = check(
        &storage,
        &models,
        &operation,
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    assert_eq!(summary.state, CheckState::Succeeded, "{summary:?}");
    verify_success(
        &storage,
        &models,
        &operation,
        &package,
        &inspected.review_hash,
        &|| false,
    )
    .unwrap();
    let preview = crate::model_application::preview_with_sensitivity(
        &storage,
        &models,
        crate::storage::model_profiles::ProfileName::Standard,
        &package,
        &inspected.review_hash,
        &operation,
        None,
        None,
        None,
        None,
        &|| false,
    )
    .unwrap();
    crate::model_application::apply(
        &storage,
        &models,
        &uuid::Uuid::new_v4().to_string(),
        &preview,
        &|| false,
    )
    .unwrap();
    let (_, report) = storage.model_check_record(&operation).unwrap().unwrap();
    let report = report.unwrap();
    assert_eq!(report.settings, settings(Engine::Sherpa));
    eprintln!(
        "Sherpa preparation and application passed: {}, {} ms, review {}",
        inspected.manifest.model_name, report.elapsed_ms, inspected.review_hash
    );
}
