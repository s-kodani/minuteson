use super::*;
use crate::storage::model_profiles::portable::{export, Catalog};
use serde_json::{json, Value};

fn catalog(f: &Fixture) -> Catalog {
    export(&f.storage.connection.lock().unwrap(), &|| false).unwrap()
}
fn decode(value: Value) -> Result<Catalog, String> {
    let catalog: Catalog = serde_json::from_value(value).map_err(|e| e.to_string())?;
    catalog.validate()?;
    Ok(catalog)
}

#[test]
fn portable_catalog_retains_download_origin_without_availability_or_current_host_execution() {
    let mut f = Fixture::new();
    let index = f.add(Engine::Whisper, false, false);
    let mut record = f.packages[index].record.clone();
    record.package_id = Uuid::new_v4().to_string();
    for (declared, actual) in record.manifest.files.iter_mut().zip(&mut record.files) {
        declared.expected_size = Some(actual.actual_size);
        declared.expected_hash = Some(actual.actual_hash.clone());
        actual.expected_size_checked = true;
        actual.expected_hash_checked = true;
    }
    record.source_manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&record.manifest).unwrap())
    );
    record.review_hash = crate::model_inspection::review_hash(
        &record.manifest,
        &record.source_manifest_hash,
        &record.files,
    )
    .unwrap();
    let recipe = crate::model_download_recipe::Recipe {
        schema_version: 1,
        manifest: record.manifest.clone(),
        sources: record
            .manifest
            .files
            .iter()
            .map(|file| crate::model_download_recipe::DownloadSource {
                relative_path: file.relative_path.clone(),
                url: format!("https://example.com/{}", file.relative_path),
                archive: None,
            })
            .collect(),
    };
    record.schema_version = 2;
    record.download = Some(recipe.origin(&Uuid::new_v4().to_string()).unwrap());
    record.validate().unwrap();
    // Persist synthetic intent only. No model execution or completed copy is
    // asserted here; portable export retains even interrupted provenance.
    f.storage.begin_model_placement(&record).unwrap();
    f.storage.interrupt_model_placement(&record).unwrap();
    let exported = catalog(&f);
    let value = serde_json::to_value(&exported).unwrap();
    let package = value["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["packageId"] == record.package_id)
        .unwrap();
    assert_eq!(
        package["download"],
        serde_json::to_value(&record.download).unwrap()
    );
    assert_eq!(decode(value.clone()).unwrap(), exported);
    let text = serde_json::to_string(package).unwrap();
    assert!(!text.contains("modelsRoot") && !text.contains("ready") && !text.contains("installed"));
    let mut historical = record.clone();
    historical.manifest.target = match record.manifest.target {
        Target::MacosAarch64 => Target::WindowsX86_64,
        Target::WindowsX86_64 => Target::MacosAarch64,
    };
    let mut other_recipe = recipe.clone();
    other_recipe.manifest = historical.manifest.clone();
    historical.download = Some(other_recipe.origin(&Uuid::new_v4().to_string()).unwrap());
    historical.source_manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&historical.manifest).unwrap())
    );
    historical.review_hash = crate::model_inspection::review_hash(
        &historical.manifest,
        &historical.source_manifest_hash,
        &historical.files,
    )
    .unwrap();
    historical.validate_portable().unwrap();
    assert!(historical.validate().is_err());
}

#[test]
fn portable_model_catalog_preserves_unused_packages_all_versions_and_both_numeric_profiles_without_paths_or_execution(
) {
    let mut f = Fixture::new();
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let index = f.add(engine, true, false);
        f.apply(index, ProfileName::Standard);
        f.apply(index, ProfileName::HighAccuracy);
    }
    let unused = f.add(Engine::Whisper, false, false);
    let before = [
        f.count("processing_jobs"),
        f.count("model_profiles"),
        f.count("model_compatibility_checks"),
    ];
    let executed_before = (0..3).map(|index| f.executed(index)).collect::<Vec<_>>();
    let original = catalog(&f);
    let value = serde_json::to_value(&original).unwrap();
    assert_eq!(value["packages"].as_array().unwrap().len(), 4);
    assert_eq!(value["profiles"].as_array().unwrap().len(), 6);
    assert!(value["packages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["packageId"] == f.packages[unused].id));
    assert_eq!(
        value["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["wasCurrent"] == true)
            .count(),
        2
    );
    let configs = &value["preferences"];
    assert_eq!(
        configs[0]["configurations"][0],
        json!({"engine":"whisper.cpp","threads":7,"useGpu":true})
    );
    assert_eq!(
        configs[1]["configurations"][0],
        json!({"engine":"whisper.cpp","threads":3,"useGpu":false})
    );
    assert_eq!(
        configs[0]["configurations"][1],
        json!({"engine":"sherpa-onnx","threads":6,"clusterThresholdMilli":700,"windowShiftMilli":400,"minDurationOnMs":200,"minDurationOffMs":600})
    );
    assert_eq!(
        configs[1]["configurations"][1]["clusterThresholdMilli"],
        900
    );
    assert_eq!(
        configs[0]["configurations"][2],
        json!({"engine":"llama.cpp","threads":5,"contextTokens":8192,"predictionTokens":512,"gpuLayers":2,"seed":777,"promptBudget":5000})
    );
    assert_eq!(configs[1]["configurations"][2]["contextTokens"], 16384);
    assert_eq!(configs[1]["configurations"][2]["promptBudget"], 9000);
    assert_eq!(configs[1]["configurations"][2]["seed"], 14);
    let text = serde_json::to_string(&original).unwrap();
    for excluded in [
        f.models().to_str().unwrap(),
        f.root.path().to_str().unwrap(),
        "modelsRoot",
        "executablePath",
        "modelPath",
        "runtimeLibraryPaths",
        "checkOperationId",
        "compatibility",
        "installed",
        "ready",
    ] {
        assert!(!text.contains(excluded), "must exclude {excluded}");
    }
    assert_eq!(decode(value).unwrap(), original);
    assert_eq!(
        before,
        [
            f.count("processing_jobs"),
            f.count("model_profiles"),
            f.count("model_compatibility_checks")
        ]
    );
    for (index, executed) in executed_before.iter().enumerate() {
        assert!(!executed.is_empty());
        assert_eq!(&f.executed(index), executed);
    }
    // Metadata remains readable after every physical managed model and probe
    // artifact is gone. Exporting it must not re-run a compatibility check.
    drop(f.app_data);
    assert_eq!(
        export(&f.storage.connection.lock().unwrap(), &|| false).unwrap(),
        original
    );
}

#[test]
fn portable_model_catalog_rejects_unknown_fields_hash_roles_missing_packages_duplicate_heads_and_invalid_numeric_conditions(
) {
    let mut f = Fixture::new();
    let whisper = f.add(Engine::Whisper, true, false);
    let llama = f.add(Engine::Llama, true, false);
    f.apply(whisper, ProfileName::Standard);
    f.apply(llama, ProfileName::Standard);
    let original = serde_json::to_value(catalog(&f)).unwrap();
    for condition in [
        "host_path",
        "probe",
        "availability",
        "version",
        "file_hash",
        "role",
        "missing_package",
        "duplicate_package",
        "duplicate_profile",
        "duplicate_version",
        "old_current",
        "no_current",
        "missing_preferences",
        "duplicate_preferences",
        "missing_engine",
        "duplicate_engine",
        "threads",
        "budget",
        "review",
        "engine",
        "unknown_config",
        "unknown_profile",
        "id_collision",
    ] {
        let mut value = original.clone();
        match condition {
            "host_path" => value["modelsRoot"] = json!("/private/source/models"),
            "probe" => value["packages"][0]["compatibility"] = json!({"state":"succeeded"}),
            "availability" => value["profiles"][0]["ready"] = json!(true),
            "version" => value["schemaVersion"] = json!(2),
            "file_hash" => value["packages"][0]["files"][0]["actualHash"] = json!("a".repeat(64)),
            "role" => value["packages"][0]["files"][0]["role"] = json!("llm"),
            "missing_package" => {
                value["packages"].as_array_mut().unwrap().pop();
            }
            "duplicate_package" => {
                let p = value["packages"][0].clone();
                value["packages"].as_array_mut().unwrap().push(p);
            }
            "duplicate_profile" => {
                let p = value["profiles"][0].clone();
                value["profiles"].as_array_mut().unwrap().push(p);
            }
            "duplicate_version" => value["profiles"][1]["profileVersion"] = json!(1),
            "old_current" => {
                value["profiles"][0]["wasCurrent"] = json!(true);
                value["profiles"][1]["wasCurrent"] = json!(false);
            }
            "no_current" => value["profiles"][1]["wasCurrent"] = json!(false),
            "missing_preferences" => {
                value["preferences"].as_array_mut().unwrap().pop();
            }
            "duplicate_preferences" => value["preferences"][1] = value["preferences"][0].clone(),
            "missing_engine" => {
                value["preferences"][0]["configurations"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            "duplicate_engine" => {
                value["preferences"][0]["configurations"][1] =
                    value["preferences"][0]["configurations"][0].clone()
            }
            "threads" => value["preferences"][0]["configurations"][0]["threads"] = json!(0),
            "budget" => value["preferences"][0]["configurations"][2]["promptBudget"] = json!(60000),
            "review" => {
                value["profiles"][0]["components"][0]["packageReviewHash"] = json!("a".repeat(64))
            }
            "engine" => {
                value["profiles"][0]["components"][0]["configuration"] = json!({"engine":"llama.cpp","threads":1,"contextTokens":8192,"predictionTokens":64,"gpuLayers":0,"seed":1,"promptBudget":2048})
            }
            "unknown_config" => {
                value["preferences"][0]["configurations"][0]["useCpu"] = json!(true)
            }
            "unknown_profile" => value["profiles"][0]["state"] = json!("unavailable"),
            "id_collision" => {
                value["profiles"][0]["sourceProfileId"] = value["packages"][0]["packageId"].clone()
            }
            _ => unreachable!(),
        }
        assert!(decode(value).is_err(), "reject {condition}");
    }
}

#[test]
fn portable_model_catalog_rejects_corrupt_unused_package_and_backup_releases_maintenance_without_publishing(
) {
    let mut f = Fixture::new();
    let index = f.add(Engine::Whisper, false, false);
    let output = tempfile::tempdir().unwrap();
    let target = output.path().join("invalid.monbackup");
    {
        let db = f.storage.connection.lock().unwrap();
        db.execute(
            "UPDATE model_files SET actual_size=actual_size+1 WHERE package_id=?1",
            params![f.packages[index].id],
        )
        .unwrap();
        assert!(export(&db, &|| false).is_err());
    }
    let failure = f
        .storage
        .create_backup_archive(&target, &|| false)
        .err()
        .unwrap();
    assert!(!failure.published);
    assert!(!target.exists());
    let meeting = f.storage.get_meeting(&f.meeting).unwrap().unwrap();
    assert!(f
        .storage
        .rename_meeting(&f.meeting, "保全", meeting.lock_version)
        .is_ok());
}

#[test]
fn portable_model_catalog_checks_superseded_profile_and_never_repairs_it_or_changes_current_head() {
    let mut f = Fixture::new();
    let whisper = f.add(Engine::Whisper, true, false);
    let llama = f.add(Engine::Llama, true, false);
    let old = f.apply(whisper, ProfileName::Standard).id.unwrap();
    let latest = f.apply(llama, ProfileName::Standard).id.unwrap();
    let db = f.storage.connection.lock().unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )
    .unwrap();
    db.execute("UPDATE profile_components SET settings_json=json_set(settings_json,'$.binding.settings.settings.threads',0) WHERE profile_id=?1", params![old]).unwrap();
    db.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        true,
    )
    .unwrap();
    assert!(export(&db, &|| false).is_err());
    let (heads, _) = heads(&db).unwrap();
    assert_eq!(heads.standard.unwrap().id, latest);
    let threads: i64 = db.query_row("SELECT json_extract(settings_json,'$.binding.settings.settings.threads') FROM profile_components WHERE profile_id=?1", params![old], |r| r.get(0)).unwrap();
    assert_eq!(threads, 0);
}

#[test]
fn portable_model_catalog_cancellation_is_read_only_and_defaults_are_complete_without_managed_models(
) {
    let f = Fixture::new();
    let initial = catalog(&f);
    let value = serde_json::to_value(&initial).unwrap();
    assert_eq!(value["packages"], json!([]));
    assert_eq!(value["profiles"], json!([]));
    let before = f.storage.analysis_preferences().unwrap().lock_version;
    assert!(export(&f.storage.connection.lock().unwrap(), &|| true).is_err());
    let calls = std::cell::Cell::new(0);
    assert!(export(&f.storage.connection.lock().unwrap(), &|| {
        calls.set(calls.get() + 1);
        calls.get() >= 3
    })
    .is_err());
    assert_eq!(catalog(&f), initial);
    assert_eq!(
        f.storage.analysis_preferences().unwrap().lock_version,
        before
    );
}
