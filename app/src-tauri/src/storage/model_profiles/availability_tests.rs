//! Temporary synthetic bodies and stored trial evidence; no engine execution.
use super::*;
use crate::model_compatibility::{artifact_names, Artifact, ProbeReport};
use crate::storage::deletion::models::{operations::Prepared, tests::Fixture};

fn checked(storage: &Storage, record: &PlacementRecord) -> String {
    let id = Uuid::new_v4().to_string();
    storage.begin_model_check(&id, record).unwrap();
    let report = ProbeReport {
        schema_version: 1,
        operation_id: id.clone(),
        package_id: record.package_id.clone(),
        review_hash: record.review_hash.clone(),
        engine: record.manifest.engine,
        declared_engine_version: record.manifest.engine_version.clone(),
        adapter_revision: 1,
        settings: crate::model_compatibility::settings(record.manifest.engine),
        artifacts: artifact_names(record.manifest.engine)
            .iter()
            .map(|name| Artifact {
                name: (*name).into(),
                size: 1,
                sha256: "a".repeat(64),
            })
            .collect(),
        elapsed_ms: 1,
    };
    storage
        .finish_model_check(&id, record, Some(&report), CheckState::Succeeded, None)
        .unwrap();
    id
}

fn minutes(f: &Fixture, base: &PlacementRecord) -> PlacementRecord {
    let mut record = base.clone();
    record.package_id = Uuid::new_v4().to_string();
    record.manifest.engine = Engine::Llama;
    record.manifest.model_name = "synthetic minutes".into();
    for file in &mut record.manifest.files {
        if file.role == Role::Asr {
            file.role = Role::Llm;
        }
    }
    for file in &mut record.files {
        if file.role == Role::Asr {
            file.role = Role::Llm;
        }
        let path = f.root.join(&record.package_id).join(&file.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = if file.role == Role::Executable {
            b"synthetic; do not execute".to_vec()
        } else {
            vec![b'm'; 150000]
        };
        assert_eq!(bytes.len() as u64, file.actual_size);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), file.actual_hash);
        std::fs::write(path, bytes).unwrap();
    }
    record.source_manifest_hash = digest(&record.manifest).unwrap();
    record.review_hash = crate::model_inspection::review_hash(
        &record.manifest,
        &record.source_manifest_hash,
        &record.files,
    )
    .unwrap();
    f.storage.begin_model_placement(&record).unwrap();
    f.storage.finish_model_placement(&record).unwrap();
    record
}

fn preview(f: &Fixture, record: &PlacementRecord, check: &str) -> ApplyPreview {
    f.storage
        .preview_model_profile_with_tuning(
            ProfileName::Standard,
            record,
            &f.root.join(&record.package_id),
            check,
            None,
            None,
        )
        .unwrap()
}

fn apply(f: &Fixture, record: &PlacementRecord, check: &str) -> (String, ApplyPreview) {
    let preview = preview(f, record, check);
    let id = Uuid::new_v4().to_string();
    f.storage
        .apply_model_profile(
            &id,
            &preview,
            record,
            &f.root.join(&record.package_id),
            &|| false,
        )
        .unwrap();
    (id, preview)
}

fn history(storage: &Storage) -> String {
    storage.connection.lock().unwrap().query_row(
        "SELECT json_array((SELECT json_group_array(json_array(id,name,profile_version,state)) FROM model_profiles),(SELECT json_group_array(json_array(profile_id,role,package_id,settings_json)) FROM profile_components),(SELECT json_group_array(json_array(key,value_json,content_hash,lock_version)) FROM app_settings),(SELECT json_group_array(json_array(id,profile_snapshot_json,parameters_json)) FROM processing_jobs))",
        [], |r| r.get(0),
    ).unwrap()
}

#[test]
fn deleted_selected_model_keeps_profiles_readable_and_allows_replacement_after_restart() {
    let mut f = Fixture::new();
    let asr = f
        .storage
        .model_package_for_check(&f.package, &f.review)
        .unwrap();
    let asr_check = checked(&f.storage, &asr);
    apply(&f, &asr, &asr_check);
    let old = minutes(&f, &asr);
    let old_check = checked(&f.storage, &old);
    let (old_application, old_preview) = apply(&f, &old, &old_check);
    let before = history(&f.storage);
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let mut deletion = Prepared::fresh_directory(
        &f.storage,
        &f.root,
        &old.package_id,
        &old.review_hash,
        &lease,
        &|| false,
    )
    .unwrap();
    let confirmation = deletion.preview().confirmation_id;
    assert!(
        deletion
            .execute(&f.storage, &lease, &confirmation, false, &|| false, &|_| {})
            .unwrap()
            .body_deleted
    );
    drop(deletion);
    drop(lease);
    assert!(!f.root.join(&old.package_id).exists());
    assert_eq!(history(&f.storage), before);

    let root = f.storage.root().to_path_buf();
    let placeholder = tempfile::tempdir().unwrap();
    drop(std::mem::replace(
        &mut f.storage,
        Storage::open(placeholder.path().into()).unwrap(),
    ));
    f.storage = Storage::open_with_models(root, f.root.clone()).unwrap();
    let profiles = f.storage.model_profiles().unwrap();
    assert_eq!(profiles[0].components.len(), 2);
    assert!(!profiles[0].configuration_complete);
    let removed = profiles[0]
        .components
        .iter()
        .find(|c| c.engine == Engine::Llama)
        .unwrap();
    assert!(removed.settings_match);
    assert!(!removed.compatibility_current);
    assert!(
        profiles[0]
            .components
            .iter()
            .find(|c| c.engine == Engine::Whisper)
            .unwrap()
            .compatibility_current
    );
    let catalog = f.storage.list_model_placements().unwrap();
    assert!(
        catalog
            .iter()
            .find(|c| c.package_id == old.package_id)
            .unwrap()
            .body_removed
    );
    assert_eq!(history(&f.storage), before);

    assert!(f
        .storage
        .preview_model_profile_with_tuning(
            ProfileName::Standard,
            &old,
            &f.root.join(&old.package_id),
            &old_check,
            None,
            None,
        )
        .is_err());
    assert!(f
        .storage
        .apply_model_profile(
            &old_application,
            &old_preview,
            &old,
            &f.root.join(&old.package_id),
            &|| false,
        )
        .is_err());
    {
        let db = f.storage.connection.lock().unwrap();
        let (heads, _) = heads(&db).unwrap();
        let stored =
            load_profile(&db, ProfileName::Standard, heads.standard.as_ref().unwrap()).unwrap();
        let binding = stored
            .bindings
            .iter()
            .find(|b| b.settings.engine() == Engine::Llama)
            .unwrap();
        assert!(binding.validate(&db).is_ok());
        assert!(binding.validate_for_use(&db).is_err());
    }
    // An unavailable minutes component must not prevent reapplying ASR.
    apply(&f, &asr, &asr_check);
    let next = minutes(&f, &asr);
    let next_check = checked(&f.storage, &next);
    apply(&f, &next, &next_check);
    let profiles = f.storage.model_profiles().unwrap();
    assert!(profiles[0]
        .components
        .iter()
        .all(|c| c.compatibility_current && c.settings_match));
    assert!(profiles[0]
        .components
        .iter()
        .any(|c| c.package_id == next.package_id));
    assert!(!profiles[0]
        .components
        .iter()
        .any(|c| c.package_id == old.package_id));
}

#[test]
#[ignore = "requires an explicitly provided database snapshot; read-only diagnostics"]
fn saved_profiles_can_be_inspected_without_opening_or_mutating_app_storage() {
    let path =
        std::env::var_os("MON_PROFILE_DIAGNOSTIC_DB").expect("explicit database snapshot required");
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    let (heads, _) = heads(&db).unwrap();
    for name in [ProfileName::Standard, ProfileName::HighAccuracy] {
        let stored = heads
            .get(name)
            .map(|head| load_profile(&db, name, head))
            .transpose()
            .unwrap();
        let profile = view(&db, name, stored.as_ref()).unwrap();
        eprintln!("{}", serde_json::to_string(&profile).unwrap());
    }
}
