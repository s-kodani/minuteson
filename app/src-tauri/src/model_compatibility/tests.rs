use super::*;
use crate::local_engine::Fingerprint;
use crate::model_inspection::{PackageFile, PackageManifest, Target};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

struct Fixture {
    source: tempfile::TempDir,
    app_data: tempfile::TempDir,
    meetings: tempfile::TempDir,
    storage: Storage,
    record: PlacementRecord,
    models: std::path::PathBuf,
}
fn fixture(engine: Engine, script: Option<&str>) -> Fixture {
    let source = tempfile::tempdir().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let meetings = tempfile::tempdir().unwrap();
    let storage = Storage::open(meetings.path().into()).unwrap();
    storage.create_meeting("keep existing meeting").unwrap();
    let package = uuid::Uuid::new_v4().to_string();
    let models = app_data.path().canonicalize().unwrap().join("models");
    let roles = match engine {
        Engine::Whisper => vec![("model.bin", Role::Asr)],
        Engine::Llama => vec![("model.bin", Role::Llm)],
        Engine::Sherpa => vec![
            ("seg.onnx", Role::Segmentation),
            ("embed.onnx", Role::SpeakerEmbedding),
        ],
    };
    for (name, _) in &roles {
        std::fs::write(
            source.path().join(name),
            b"synthetic model bytes; not a real model",
        )
        .unwrap();
    }
    let native=match engine {
        Engine::Whisper=>"model=; output=\nwhile [ \"$#\" -gt 0 ]; do case \"$1\" in --model) model=\"$2\"; shift;; --output-file) output=\"$2\"; shift;; esac; shift; done\nprintf '{\"model\":{\"multilingual\":true},\"params\":{\"model\":\"%s\",\"language\":\"ja\",\"translate\":false},\"transcription\":[]}' \"$model\" > \"$output.json\"".into(),
        Engine::Llama=>"printf '{\"ok\":true}'".into(),
        Engine::Sherpa=> {
            let proof=|name: &str| {let mut p=Fingerprint::capture(&source.path().join(name),&||false).unwrap();p.path=models.join(&package).join(name);p};
            let config=crate::diarization::sherpa::SherpaConfig {executable:proof("seg.onnx"),segmentation_model:proof("seg.onnx"),embedding_model:proof("embed.onnx"),runtime_libraries:vec![],threads:1,cluster_threshold_milli:500,window_shift_milli:500,min_duration_on_ms:300,min_duration_off_ms:500};
            format!("printf '%s\\nStarted\\n' '{}'",config.expected_header(None))
        },
    };
    let body=format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > args.log\n/usr/bin/env > env.log\necho ran >> executed.marker\n{}\n",script.unwrap_or(&native));
    std::fs::write(source.path().join("engine-cli"), body).unwrap();
    std::fs::set_permissions(
        source.path().join("engine-cli"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let mut files = vec![PackageFile {
        relative_path: "engine-cli".into(),
        role: Role::Executable,
        expected_size: None,
        expected_hash: None,
    }];
    files.extend(roles.into_iter().map(|(name, role)| PackageFile {
        relative_path: name.into(),
        role,
        expected_size: None,
        expected_hash: None,
    }));
    let manifest = PackageManifest {
        schema_version: 1,
        engine,
        engine_version: "fixture declaration".into(),
        model_name: "fixture".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "fixture only".into(),
        target: Target::current().unwrap(),
        files,
    };
    std::fs::write(
        source.path().join("mon-model.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let report = crate::model_inspection::inspect(source.path(), &|| false).unwrap();
    crate::model_placement::place(
        &storage,
        &models,
        source.path(),
        &package,
        &report.review_hash,
        &|| false,
    )
    .unwrap();
    let record = storage
        .model_package_for_check(&package, &report.review_hash)
        .unwrap();
    Fixture {
        source,
        app_data,
        meetings,
        storage,
        record,
        models,
    }
}
fn trial(f: &Fixture, operation: &str) -> CheckSummary {
    check(
        &f.storage,
        &f.models,
        operation,
        &f.record.package_id,
        &f.record.review_hash,
        &|| false,
    )
    .unwrap()
}
fn directory(f: &Fixture, operation: &str) -> std::path::PathBuf {
    f.app_data
        .path()
        .canonicalize()
        .unwrap()
        .join("model-checks")
        .join(operation)
}
fn pid_gone(path: &Path) {
    let pid = std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "native trial must have stopped before completion"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn compatibility_three_adapters_preserve_data_store_exact_cpu_evidence_and_repeat_without_execution(
) {
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let f = fixture(engine, None);
        let meetings = f.storage.list_meetings().unwrap();
        let settings = f.storage.analysis_preferences().unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        assert_eq!(trial(&f, &operation).state, CheckState::Succeeded);
        let (summary, report) = f.storage.model_check_record(&operation).unwrap().unwrap();
        assert_eq!(summary.review_hash, f.record.review_hash);
        let report = report.unwrap();
        report.validate(&f.record).unwrap();
        assert_eq!(report.settings, super::settings(engine));
        assert_eq!(
            report
                .artifacts
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            artifact_names(engine)
        );
        let catalog = f.storage.list_model_placements().unwrap().remove(0);
        assert!(catalog.placement_recorded && !catalog.ready);
        assert_eq!(catalog.compatibility.unwrap().state, CheckState::Succeeded);
        assert_eq!(
            std::fs::read_to_string(directory(&f, &operation).join("executed.marker")).unwrap(),
            "ran\n"
        );
        assert_eq!(trial(&f, &operation), summary);
        assert_eq!(
            std::fs::read_dir(f.app_data.path().join("model-checks"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(directory(&f, &operation).join("executed.marker")).unwrap(),
            "ran\n"
        );
        let env = std::fs::read_to_string(directory(&f, &operation).join("env.log")).unwrap();
        assert!(
            !env.contains("HF_TOKEN=")
                && !env.contains("LLAMA_ARG_")
                && !env.contains("DYLD_INSERT_LIBRARIES=")
        );
        assert_eq!(f.storage.list_meetings().unwrap(), meetings);
        assert_eq!(
            f.storage.analysis_preferences().unwrap().payload,
            settings.payload
        );
        for file in &f.record.files {
            assert_eq!(
                std::fs::read(f.source.path().join(&file.relative_path)).unwrap(),
                std::fs::read(
                    f.models
                        .join(&f.record.package_id)
                        .join(&file.relative_path)
                )
                .unwrap()
            );
        }
        drop(f.storage);
        let reopened = Storage::open(f.meetings.path().into()).unwrap();
        assert_eq!(
            reopened
                .model_check_record(&operation)
                .unwrap()
                .unwrap()
                .0
                .state,
            CheckState::Succeeded
        );
        assert_eq!(
            reopened
                .list_model_placements()
                .unwrap()
                .remove(0)
                .compatibility
                .unwrap()
                .state,
            CheckState::Succeeded
        );
    }
}

#[test]
fn compatibility_failure_and_new_explicit_retry_keep_completed_history_fixed() {
    let f = fixture(
        Engine::Llama,
        Some("if [ -f ../permit ]; then printf '{\"ok\":true}'; else exit 7; fi"),
    );
    let first = uuid::Uuid::new_v4().to_string();
    assert_eq!(trial(&f, &first).state, CheckState::Failed);
    assert!(f
        .storage
        .model_check_record(&first)
        .unwrap()
        .unwrap()
        .0
        .error_message
        .is_some());
    assert!(check(
        &f.storage,
        &f.models,
        &first,
        &f.record.package_id,
        &f.record.review_hash,
        &|| false
    )
    .is_err());
    std::fs::write(
        f.app_data.path().join("model-checks/permit"),
        b"explicit test fixture toggle",
    )
    .unwrap();
    let second = uuid::Uuid::new_v4().to_string();
    assert_eq!(trial(&f, &second).state, CheckState::Succeeded);
    assert!(f
        .storage
        .model_check_record(&first)
        .unwrap_err()
        .contains("古い"));
    assert!(directory(&f, &first).join("stderr.log").exists());
    drop(f.storage);
    let db = rusqlite::Connection::open(f.meetings.path().join("meetings.sqlite3")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state FROM model_compatibility_checks WHERE id=?1",
            [&first],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "failed"
    );
    assert!(db
        .execute(
            "UPDATE model_compatibility_checks SET error_message='rewritten' WHERE id=?1",
            [&first]
        )
        .is_err());
}

#[test]
fn compatibility_cancel_waits_for_the_actual_native_child_and_retains_partial_logs() {
    let f = fixture(Engine::Llama, Some("echo $$ > pid; exec /bin/sleep 10"));
    let operation = uuid::Uuid::new_v4().to_string();
    let path = directory(&f, &operation);
    let result = check(
        &f.storage,
        &f.models,
        &operation,
        &f.record.package_id,
        &f.record.review_hash,
        &|| path.join("pid").exists(),
    )
    .unwrap();
    assert_eq!(result.state, CheckState::Cancelled);
    pid_gone(&path.join("pid"));
    assert!(path.join("stderr.log").exists());
    assert!(f
        .storage
        .model_check_record(&operation)
        .unwrap()
        .unwrap()
        .1
        .is_none());
    assert!(
        f.storage
            .list_model_placements()
            .unwrap()
            .remove(0)
            .placement_recorded
    );
}

#[test]
fn compatibility_timeout_is_failed_not_success_and_native_child_is_reaped() {
    let f = fixture(Engine::Llama, Some("echo $$ > pid; exec /bin/sleep 10"));
    let operation = uuid::Uuid::new_v4().to_string();
    let summary = probe::check(
        &f.storage,
        &f.models,
        &operation,
        &f.record.package_id,
        &f.record.review_hash,
        // Allow the shell to start under parallel test load. The fixture still
        // sleeps ten seconds, so this tests the deadline rather than completion.
        Duration::from_secs(1),
        &|| false,
    )
    .unwrap();
    assert_eq!(summary.state, CheckState::Failed);
    assert!(summary.error_message.unwrap().contains("制限時間"));
    pid_gone(&directory(&f, &operation).join("pid"));
}

#[test]
fn compatibility_preflight_rejects_missing_changed_models_and_foreign_trial_directories_without_execution(
) {
    for collision in [false, true] {
        let f = fixture(Engine::Llama, None);
        let operation = uuid::Uuid::new_v4().to_string();
        if collision {
            let path = directory(&f, &operation);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::set_permissions(
                path.parent().unwrap(),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            std::fs::write(path.join("foreign"), b"keep").unwrap();
        } else {
            std::fs::write(
                f.models.join(&f.record.package_id).join("model.bin"),
                b"changed",
            )
            .unwrap();
        }
        assert_eq!(trial(&f, &operation).state, CheckState::Failed);
        assert!(!directory(&f, &operation).join("executed.marker").exists());
        if collision {
            assert_eq!(
                std::fs::read(directory(&f, &operation).join("foreign")).unwrap(),
                b"keep"
            );
        }
    }
}

#[test]
fn compatibility_identity_consent_and_prestart_cancellation_have_no_intent_or_execution() {
    let f = fixture(Engine::Llama, None);
    for (operation, review, cancelled) in [
        ("not-a-uuid".into(), f.record.review_hash.clone(), false),
        (uuid::Uuid::new_v4().to_string(), "a".repeat(64), false),
        (
            uuid::Uuid::new_v4().to_string(),
            f.record.review_hash.clone(),
            true,
        ),
        (
            f.record.package_id.clone(),
            f.record.review_hash.clone(),
            false,
        ),
    ] {
        assert!(check(
            &f.storage,
            &f.models,
            &operation,
            &f.record.package_id,
            &review,
            &|| cancelled
        )
        .is_err());
    }
    assert!(!f.app_data.path().join("model-checks").exists());
    assert!(f
        .storage
        .list_model_placements()
        .unwrap()
        .remove(0)
        .compatibility
        .is_none());
}

#[test]
fn compatibility_saved_artifacts_and_rehashed_invalid_report_are_not_trusted() {
    let f = fixture(Engine::Llama, None);
    let operation = uuid::Uuid::new_v4().to_string();
    trial(&f, &operation);
    std::fs::write(directory(&f, &operation).join("result.json"), b"{}").unwrap();
    assert!(check(
        &f.storage,
        &f.models,
        &operation,
        &f.record.package_id,
        &f.record.review_hash,
        &|| false
    )
    .unwrap_err()
    .contains("証拠"));
    let mut report = f
        .storage
        .model_check_record(&operation)
        .unwrap()
        .unwrap()
        .1
        .unwrap();
    report.settings["gpuLayers"] = serde_json::json!(99);
    assert!(report.validate(&f.record).is_err());
    let json = serde_json::to_string(&report).unwrap();
    drop(f.storage);
    let db = rusqlite::Connection::open(f.meetings.path().join("meetings.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER model_compatibility_fixed;")
        .unwrap();
    db.execute(
        "UPDATE model_compatibility_checks SET report_json=?1,report_hash=?2 WHERE id=?3",
        rusqlite::params![
            json,
            format!("{:x}", Sha256::digest(json.as_bytes())),
            operation
        ],
    )
    .unwrap();
    drop(db);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert!(reopened.list_model_placements().is_err());
}

#[test]
fn compatibility_restart_interrupts_pending_intent_without_running_or_deleting_models_or_logs() {
    let f = fixture(Engine::Llama, None);
    let operation = uuid::Uuid::new_v4().to_string();
    f.storage.begin_model_check(&operation, &f.record).unwrap();
    let path = directory(&f, &operation);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("stderr.log"), b"partial").unwrap();
    assert!(f.storage.prepare_backup_snapshot(&|| false).is_err());
    drop(f.storage);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert_eq!(
        reopened
            .model_check_record(&operation)
            .unwrap()
            .unwrap()
            .0
            .state,
        CheckState::Interrupted
    );
    assert_eq!(std::fs::read(path.join("stderr.log")).unwrap(), b"partial");
    assert!(!path.join("executed.marker").exists());
    assert!(
        reopened
            .list_model_placements()
            .unwrap()
            .remove(0)
            .placement_recorded
    );
    assert!(reopened.prepare_backup_snapshot(&|| false).is_ok());
}

#[test]
fn compatibility_v23_upgrade_keeps_placement_and_backup_omits_host_runtime_trials() {
    let f = fixture(Engine::Llama, None);
    let meeting = f.storage.list_meetings().unwrap();
    drop(f.storage);
    let db = rusqlite::Connection::open(f.meetings.path().join("meetings.sqlite3")).unwrap();
    crate::storage::model_profiles::remove_schema_for_test(&db);
    db.execute_batch(
        "DROP TABLE model_compatibility_checks; DELETE FROM schema_migrations WHERE version=24;",
    )
    .unwrap();
    drop(db);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert_eq!(reopened.list_meetings().unwrap(), meeting);
    assert!(reopened
        .list_model_placements()
        .unwrap()
        .remove(0)
        .compatibility
        .is_none());
    let operation = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        check(
            &reopened,
            &f.models,
            &operation,
            &f.record.package_id,
            &f.record.review_hash,
            &|| false
        )
        .unwrap()
        .state,
        CheckState::Succeeded
    );
    let output = f.app_data.path().join("backup.monbackup");
    reopened.create_backup_archive(&output, &|| false).unwrap();
    let bytes = std::fs::read(output).unwrap();
    assert!(!bytes
        .windows(operation.len())
        .any(|w| w == operation.as_bytes()));
    assert!(!bytes
        .windows("executed.marker".len())
        .any(|w| w == b"executed.marker"));
}
