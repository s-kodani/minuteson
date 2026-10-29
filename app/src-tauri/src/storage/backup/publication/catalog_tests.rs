use super::*;
use crate::model_download_recipe::RecipeExt;
use crate::storage::model_profiles::portable::history::Bundle;
use serde_json::json;

fn source() -> (tempfile::TempDir, tempfile::TempDir, Storage, String) {
    crate::storage::model_profiles::tests::catalog_fixture(true)
}

#[test]
fn downloaded_provenance_survives_archive_restore_and_rebackup_without_installing_or_selecting_models(
) {
    downloaded_history_roundtrip(false);
}
#[test]
fn bundled_runtime_provenance_survives_restore_and_rebackup_without_current_app_resource_or_model_execution(
) {
    downloaded_history_roundtrip(true);
}
fn downloaded_history_roundtrip(bundled: bool) {
    let (_root, _models, storage, _) = source();
    let mut record = {
        let db = storage.connection.lock().unwrap();
        let id: String = db
            .query_row(
                "SELECT id FROM model_packages WHERE engine='whisper.cpp' ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        crate::storage::model_placements::load_history(&db, &id)
            .unwrap()
            .unwrap()
            .0
    };
    if bundled {
        record.manifest.engine_version = "v1.9.4".into();
    }
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
        schema_version: 4,
        manifest: record.manifest.clone(),
        sources: record
            .manifest
            .files
            .iter()
            .map(|file| crate::model_download_recipe::DownloadSource {
                relative_path: file.relative_path.clone(),
                url: if bundled && file.role == crate::model_inspection::Role::Executable {
                    crate::model_download_recipe::BUNDLED_WHISPER.into()
                } else {
                    format!("https://example.com/{}", file.relative_path)
                },
                archive: (file.role == crate::model_inspection::Role::Executable).then(|| {
                    crate::model_download_recipe::ArchiveMember {
                        format: "tar".into(),
                        expected_size: 4096,
                        expected_hash: format!(
                            "{:x}",
                            Sha256::digest(
                                b"synthetic historical archive; not physical availability"
                            )
                        ),
                        member_path: "bin/whisper-cli".into(),
                    }
                }),
            })
            .collect(),
    };
    record.schema_version = 3;
    record.download = Some(recipe.origin(&Uuid::new_v4().to_string()).unwrap());
    storage.begin_model_placement(&record).unwrap();
    storage.interrupt_model_placement(&record).unwrap();
    let output = tempfile::tempdir().unwrap();
    let original = archive(&storage, output.path(), "download.monbackup");
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let before_preferences = target.analysis_preferences().unwrap().payload;
    restore(
        &original,
        &target,
        output.path(),
        &Uuid::new_v4().to_string(),
    );
    assert!(target.list_model_placements().unwrap().is_empty());
    assert_eq!(
        target.analysis_preferences().unwrap().payload,
        before_preferences
    );
    let history = target.model_catalog_history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, "unavailable");
    let value = serde_json::to_value(&history[0].catalog).unwrap();
    let saved = value["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["packageId"] == record.package_id)
        .unwrap();
    assert_eq!(saved, &serde_json::to_value(&record).unwrap());
    let rebacked = archive(&target, output.path(), "rebacked.monbackup");
    let final_root = tempfile::tempdir().unwrap();
    let final_storage = Storage::open(final_root.path().into()).unwrap();
    restore(
        &rebacked,
        &final_storage,
        output.path(),
        &Uuid::new_v4().to_string(),
    );
    assert!(final_storage.list_model_placements().unwrap().is_empty());
    assert!(final_storage
        .model_catalog_history()
        .unwrap()
        .iter()
        .any(|entry| serde_json::to_value(&entry.catalog).unwrap() == value));
}

#[test]
fn catalog_reader_rejects_rehashed_missing_mixed_version_foreign_paths_availability_and_unlisted_model_fingerprints_without_live_writes(
) {
    let (_root, _models, storage, _) = source();
    let output = tempfile::tempdir().unwrap();
    let verified = archive(&storage, output.path(), "valid.monbackup");
    let raw = std::fs::read(output.path().join("valid.monbackup")).unwrap();
    let header = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
    let original = serde_json::to_value(&verified.manifest).unwrap();
    let required_reference = serde_json::to_value(
        verified
            .manifest
            .model_catalog
            .file_references()
            .unwrap()
            .iter()
            .next()
            .unwrap(),
    )
    .unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let keep = target.create_meeting("既存を保護").unwrap();
    for condition in [
        "old_format",
        "unsupported",
        "old_schema",
        "missing",
        "null",
        "bundle_version",
        "empty",
        "duplicate",
        "foreign_path",
        "ready",
        "numeric",
        "missing_package",
        "model_hash",
        "unlisted_file",
        "profile_trial",
    ] {
        let mut value = original.clone();
        match condition {
            "old_format" => value["formatVersion"] = json!(1),
            "unsupported" => value["formatVersion"] = json!(3),
            "old_schema" => value["schemaVersion"] = json!(26),
            "missing" => {
                value.as_object_mut().unwrap().remove("modelCatalog");
            }
            "null" => value["modelCatalog"] = serde_json::Value::Null,
            "bundle_version" => value["modelCatalog"]["schemaVersion"] = json!(2),
            "empty" => value["modelCatalog"]["catalogs"] = json!([]),
            "duplicate" => {
                let catalog = value["modelCatalog"]["catalogs"][0].clone();
                value["modelCatalog"]["catalogs"]
                    .as_array_mut()
                    .unwrap()
                    .push(catalog);
            }
            "foreign_path" => {
                value["modelCatalog"]["catalogs"][0]["preferences"][0]["configurations"][0]
                    ["modelPath"] = json!("/Users/origin/model.bin")
            }
            "ready" => value["modelCatalog"]["catalogs"][0]["packages"][0]["ready"] = json!(true),
            "numeric" => {
                value["modelCatalog"]["catalogs"][0]["preferences"][0]["configurations"][0]
                    ["threads"] = json!(0)
            }
            "missing_package" => value["modelCatalog"]["catalogs"][0]["packages"] = json!([]),
            "model_hash" => {
                value["modelCatalog"]["catalogs"][0]["packages"][0]["files"][0]["actualHash"] =
                    json!("z".repeat(64))
            }
            "unlisted_file" => {
                value["fileReferences"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|item| *item != required_reference);
            }
            "profile_trial" => {
                value["modelCatalog"]["catalogs"][0]["profiles"][0]["checkOperationId"] =
                    json!(Uuid::new_v4().to_string())
            }
            _ => unreachable!(),
        }
        let manifest = serde_json::to_vec(&value).unwrap();
        let mut bytes = raw[..8].to_vec();
        bytes.extend((manifest.len() as u32).to_le_bytes());
        bytes.extend(Sha256::digest(&manifest));
        bytes.extend(manifest);
        bytes.extend_from_slice(&raw[44 + header..]);
        let path = output.path().join("invalid.monbackup");
        std::fs::write(&path, bytes).unwrap();
        let count = std::fs::read_dir(output.path()).unwrap().count();
        assert!(
            archive::read(
                &mut File::open(path).unwrap(),
                output.path(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "reject {condition}"
        );
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), count);
        assert_eq!(target.list_meetings().unwrap(), vec![keep.clone()]);
        assert!(target.model_catalog_history().unwrap().is_empty());
    }
}
fn archive(storage: &Storage, output: &Path, name: &str) -> archive::VerifiedArchive {
    let path = output.join(name);
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    archive::read(
        &mut File::open(path).unwrap(),
        output,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
}
fn restore(source: &archive::VerifiedArchive, target: &Storage, output: &Path, batch: &str) {
    let ids =
        RestoreIds::prepare(source, output, batch, &crate::test_support::never_cancelled).unwrap();
    let fixed =
        restore_fixed::FixedData::new(&ids, output, &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output, &crate::test_support::never_cancelled).unwrap();
    assert!(matches!(
        target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        Publication::Published { .. }
    ));
    assert_eq!(
        target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        Publication::AlreadyPublished
    );
}

#[test]
fn catalog_archive_additive_restore_preserves_all_unused_models_profiles_preferences_and_existing_selection_then_rebacks_three_times(
) {
    let (_source_root, app_data, storage, meeting) = source();
    let output = tempfile::tempdir().unwrap();
    let mut original = archive(&storage, output.path(), "source.monbackup");
    assert_eq!(
        original.manifest.format_version,
        crate::current_formats::BACKUP
    );
    let native =
        serde_json::to_value(&original.manifest.model_catalog).unwrap()["catalogs"][0].clone();
    assert_eq!(native["packages"].as_array().unwrap().len(), 4);
    assert_eq!(native["profiles"].as_array().unwrap().len(), 6);
    assert!(!serde_json::to_string(&original.manifest)
        .unwrap()
        .contains(app_data.path().to_str().unwrap()));
    drop(app_data);
    let (root, target_models, mut target, target_meeting) =
        crate::storage::model_profiles::tests::catalog_fixture(false);
    let original_meeting = target.get_meeting(&target_meeting).unwrap();
    let profiles = target.model_profiles().unwrap();
    let before_preferences = [
        serde_json::to_value(target.analysis_preferences().unwrap().payload).unwrap(),
        serde_json::to_value(target.diarization_preferences().unwrap().payload).unwrap(),
        serde_json::to_value(target.minutes_preferences().unwrap().payload).unwrap(),
    ];
    let placement = serde_json::to_value(target.list_model_placements().unwrap()).unwrap();
    let batch = Uuid::new_v4().to_string();
    restore(&original, &target, output.path(), &batch);
    assert_eq!(
        target.get_meeting(&target_meeting).unwrap(),
        original_meeting
    );
    assert_eq!(target.model_profiles().unwrap(), profiles);
    assert_eq!(
        serde_json::to_value(target.list_model_placements().unwrap()).unwrap(),
        placement
    );
    assert_eq!(
        before_preferences,
        [
            serde_json::to_value(target.analysis_preferences().unwrap().payload).unwrap(),
            serde_json::to_value(target.diarization_preferences().unwrap().payload).unwrap(),
            serde_json::to_value(target.minutes_preferences().unwrap().payload).unwrap(),
        ]
    );
    assert_eq!(target.model_catalog_history().unwrap().len(), 1);
    let entries = target.model_catalog_history().unwrap();
    assert_eq!(serde_json::to_value(&entries[0].catalog).unwrap(), native);
    assert_eq!(entries[0].state, "unavailable");
    assert!(!original
        .manifest
        .model_catalog
        .origin_ids()
        .contains(&entries[0].id));
    let next_batch = Uuid::new_v4().to_string();
    restore(&original, &target, output.path(), &next_batch);
    assert_eq!(target.model_catalog_history().unwrap(), entries);
    assert_eq!(target.list_meetings().unwrap().len(), 3);
    {
        let db = target.connection.lock().unwrap();
        assert!(db.query_row("SELECT NOT EXISTS(SELECT 1 FROM model_packages WHERE source_type='historical_snapshot' AND state!='unavailable') AND NOT EXISTS(SELECT 1 FROM model_files f JOIN model_packages p ON p.id=f.package_id WHERE p.source_type='historical_snapshot' AND f.state!='unavailable')", [], |r| r.get::<_, bool>(0)).unwrap());
        for reference in &original.manifest.file_references {
            assert!(crate::storage::model_metadata::retained_files(&db)
                .unwrap()
                .contains(reference));
        }
        // Existing queued work is untouched. Imported work is interrupted and
        // cannot infer a ready model from the catalog's old-current flag.
        let imported: i64 = db
            .query_row(
                "SELECT count(*) FROM processing_jobs WHERE meeting_id!=?1 AND state='interrupted'",
                params![target_meeting],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(imported, 2);
    }
    // Keep fixture roots alive while checking reopen and successive archives.
    let mut roots = vec![root];
    for generation in 0..3 {
        let path = target.root.clone();
        drop(target);
        target = Storage::open(path).unwrap();
        assert!(target
            .model_catalog_history()
            .unwrap()
            .iter()
            .any(|entry| serde_json::to_value(&entry.catalog).unwrap() == native));
        original = archive(
            &target,
            output.path(),
            &format!("next-{generation}.monbackup"),
        );
        let bundle = serde_json::to_value(&original.manifest.model_catalog).unwrap();
        assert!(bundle["catalogs"].as_array().unwrap().contains(&native));
        assert_eq!(
            bundle["catalogs"].as_array().unwrap().len(),
            if generation == 0 { 2 } else { 3 }
        );
        let root = tempfile::tempdir().unwrap();
        target = Storage::open(root.path().into()).unwrap();
        restore(
            &original,
            &target,
            output.path(),
            &Uuid::new_v4().to_string(),
        );
        roots.push(root);
    }
    assert!(target
        .model_profiles()
        .unwrap()
        .iter()
        .all(|v| v.id.is_none() && !v.configuration_complete));
    assert!(target.list_model_placements().unwrap().is_empty());
    assert!(target.acquire_local_inference_job().unwrap().is_none());
    assert!(!meeting.is_empty());
    drop(target_models);
}

#[test]
fn catalog_history_failure_during_atomic_publication_rolls_back_all_meetings_metadata_vocabulary_and_owned_files(
) {
    let f = tests::fixture();
    let ids = tests::ids(&f, &Uuid::new_v4().to_string());
    let sealed = tests::sealed(&f, &ids);
    let before = tests::all_rows(&f.target.connection.lock().unwrap());
    f.target.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_catalog BEFORE INSERT ON model_catalog_history BEGIN SELECT RAISE(ABORT,'fixture catalog commit failure'); END;").unwrap();
    assert!(f
        .target
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .is_err());
    f.target
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER temp.reject_catalog;")
        .unwrap();
    assert_eq!(
        tests::all_rows(&f.target.connection.lock().unwrap()),
        before
    );
    for meeting in read_meetings(sealed.connection()).unwrap() {
        assert!(!f.target.root.join("meetings").join(meeting.id).exists());
    }
    assert!(!f.target.maintenance.load(Ordering::Acquire));
    assert!(matches!(
        f.target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        Publication::Published { .. }
    ));
}

#[test]
fn catalog_history_requires_transaction_preserves_distinct_catalogs_with_shared_origin_ids_and_late_cancel_rolls_back(
) {
    let (_root, _models, storage, _) = source();
    let original = Bundle::export(
        &storage.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut value = serde_json::to_value(&original).unwrap();
    let mut different = value["catalogs"][0].clone();
    different["preferences"][0]["configurations"][0]["threads"] = json!(19);
    value["catalogs"].as_array_mut().unwrap().push(different);
    let bundle: Bundle = serde_json::from_value(value).unwrap();
    bundle.validate().unwrap();
    let root = tempfile::tempdir().unwrap();
    let target = Storage::open(root.path().into()).unwrap();
    let mut db = target.connection.lock().unwrap();
    assert!(bundle
        .retain(&db, &crate::test_support::never_cancelled, &|_| Ok(true))
        .is_err());
    {
        let tx = db.transaction().unwrap();
        assert!(bundle
            .retain(
                &tx,
                &|| {
                    tx.query_row("SELECT count(*) FROM model_catalog_history", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
                        == 2
                },
                &|_| Ok(true)
            )
            .is_err());
        tx.rollback().unwrap();
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM model_catalog_history", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let tx = db.transaction().unwrap();
    bundle
        .retain(&tx, &crate::test_support::never_cancelled, &|_| Ok(true))
        .unwrap();
    bundle
        .retain(&tx, &crate::test_support::never_cancelled, &|_| Ok(true))
        .unwrap();
    tx.commit().unwrap();
    drop(db);
    let entries = target.model_catalog_history().unwrap();
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].id, entries[1].id);
    assert_eq!(entries[0].origin_ids(), entries[1].origin_ids());
    assert_ne!(entries[0].catalog, entries[1].catalog);
    for entry in &entries {
        assert!(!bundle.origin_ids().contains(&entry.id));
    }
    // An incoming origin must not retroactively turn a local history PK into
    // source provenance. Reject before appending anything, preserving history.
    let mut collision = serde_json::to_value(&original).unwrap();
    collision["catalogs"][0]["profiles"][0]["sourceProfileId"] = json!(entries[0].id);
    let collision: Bundle = serde_json::from_value(collision).unwrap();
    collision.validate().unwrap();
    let mut db = target.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    assert!(collision
        .retain(&tx, &crate::test_support::never_cancelled, &|_| Ok(true))
        .err()
        .unwrap()
        .contains("衝突"));
    tx.rollback().unwrap();
    drop(db);
    assert_eq!(target.model_catalog_history().unwrap(), entries);
}

#[test]
fn catalog_history_current_reopen_preserves_rows_settings_and_immutable_unavailable_records_reject_tampering(
) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("維持する会議").unwrap();
    let mut settings = storage.analysis_preferences().unwrap().payload;
    settings.standard.threads = 12;
    storage.save_analysis_preferences(0, settings).unwrap();
    let bundle = Bundle::export(
        &storage.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preferences =
        serde_json::to_value(storage.analysis_preferences().unwrap().payload).unwrap();
    drop(storage);
    let target = Storage::open(root.path().into()).unwrap();
    assert_eq!(target.get_meeting(&meeting.id).unwrap(), Some(meeting));
    assert_eq!(
        serde_json::to_value(target.analysis_preferences().unwrap().payload).unwrap(),
        preferences
    );
    assert!(target.model_catalog_history().unwrap().is_empty());
    {
        let mut db = target.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        bundle
            .retain(&tx, &crate::test_support::never_cancelled, &|_| Ok(true))
            .unwrap();
        tx.commit().unwrap();
        for sql in [
            "UPDATE model_catalog_history SET state='ready'",
            "UPDATE model_catalog_history SET payload_json='{}'",
            "DELETE FROM model_catalog_history",
        ] {
            assert!(db.execute_batch(sql).is_err());
        }
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute("UPDATE model_catalog_history SET payload_hash=CASE WHEN payload_hash=?1 THEN ?2 ELSE ?1 END", params!["a".repeat(64), "b".repeat(64)]).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    assert!(target.model_catalog_history().is_err());
    assert!(target
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    assert!(target.create_meeting("破損履歴を自動修復しない").is_ok());
}

#[test]
fn catalog_history_rejects_bundle_and_local_record_count_limits_without_repairing_or_touching_meetings(
) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let bundle = Bundle::export(
        &storage.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut value = serde_json::to_value(bundle).unwrap();
    let empty = value["catalogs"][0].clone();
    value["catalogs"] = serde_json::Value::Array(vec![empty; 4097]);
    let oversized: Bundle = serde_json::from_value(value).unwrap();
    assert!(oversized.validate().err().unwrap().contains("件数"));
    let meeting = storage.create_meeting("上限時にも保全").unwrap();
    {
        let mut db = storage.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        let mut insert = tx
            .prepare("INSERT INTO model_catalog_history VALUES(?1,'{}',?2,'unavailable',0)")
            .unwrap();
        for n in 0..4097 {
            insert
                .execute(params![Uuid::new_v4().to_string(), format!("{n:064x}")])
                .unwrap();
        }
        drop(insert);
        tx.commit().unwrap();
    }
    assert!(storage
        .model_catalog_history()
        .err()
        .unwrap()
        .contains("件数"));
    assert_eq!(storage.get_meeting(&meeting.id).unwrap(), Some(meeting));
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM model_catalog_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        4097
    );
}
