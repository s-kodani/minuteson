use super::*;
use crate::model_inspection::{Engine, PackageFile};
use std::cell::Cell;
use std::os::unix::fs::{symlink, PermissionsExt};

#[test]
fn guided_model_placement_copies_canonical_manifest_without_touching_sources_and_retries_same_receipt(
) {
    let f = fixture();
    std::fs::remove_file(f.source.path().join(model_inspection::MANIFEST)).unwrap();
    let manifest = f.report.manifest.clone();
    let report = model_inspection::inspect_declared(
        f.source.path(),
        &manifest,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let token = uuid::Uuid::new_v4().to_string();
    let root = f.app_data.path().join("models");
    let before = std::fs::read(f.source.path().join("weights/model.bin")).unwrap();
    let receipt = place_with_manifest(
        &f.storage,
        &root,
        f.source.path(),
        &token,
        &report.review_hash,
        Some(&manifest),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!receipt.ready);
    assert!(receipt.placement_recorded);
    assert!(!f.source.path().join(model_inspection::MANIFEST).exists());
    assert_eq!(
        std::fs::read(f.source.path().join("weights/model.bin")).unwrap(),
        before
    );
    let placed =
        model_inspection::inspect(&root.join(&token), &crate::test_support::never_cancelled)
            .unwrap();
    assert_eq!(placed.review_hash, report.review_hash);
    assert_eq!(
        std::fs::read(root.join(&token).join(model_inspection::MANIFEST)).unwrap(),
        serde_json::to_vec(&manifest).unwrap()
    );
    let retry = place_with_manifest(
        &f.storage,
        &root,
        f.source.path(),
        &token,
        &report.review_hash,
        Some(&manifest),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(retry.review_hash, receipt.review_hash);
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    assert!(PlacementRecord::from_report(&token, &report)
        .download
        .is_none());
}

#[test]
fn guided_model_placement_rejects_changed_declaration_bytes_and_cancel_before_any_write() {
    let f = fixture();
    std::fs::remove_file(f.source.path().join(model_inspection::MANIFEST)).unwrap();
    let mut manifest = f.report.manifest.clone();
    let report = model_inspection::inspect_declared(
        f.source.path(),
        &manifest,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let root = f.app_data.path().join("models");
    let token = uuid::Uuid::new_v4().to_string();
    assert!(place_with_manifest(
        &f.storage,
        &root,
        f.source.path(),
        &token,
        &report.review_hash,
        Some(&manifest),
        &|| true
    )
    .is_err());
    manifest.model_name = "different declaration".into();
    assert!(place_with_manifest(
        &f.storage,
        &root,
        f.source.path(),
        &token,
        &report.review_hash,
        Some(&manifest),
        &crate::test_support::never_cancelled
    )
    .is_err());
    manifest.model_name = report.manifest.model_name.clone();
    std::fs::write(f.source.path().join("weights/model.bin"), b"changed bytes").unwrap();
    assert!(place_with_manifest(
        &f.storage,
        &root,
        f.source.path(),
        &token,
        &report.review_hash,
        Some(&manifest),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(!root.exists());
    assert!(!f.source.path().join(model_inspection::MANIFEST).exists());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
}

struct Fixture {
    source: tempfile::TempDir,
    meetings: tempfile::TempDir,
    app_data: tempfile::TempDir,
    storage: Storage,
    report: Report,
}
fn fixture() -> Fixture {
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir(source.path().join("weights")).unwrap();
    std::fs::write(
        source.path().join("engine-cli"),
        b"test only; must not execute",
    )
    .unwrap();
    std::fs::write(source.path().join("weights/model.bin"), vec![b'm'; 150_000]).unwrap();
    let manifest = PackageManifest {
        schema_version: 1,
        engine: Engine::Whisper,
        engine_version: "unverified release".into(),
        model_name: "合成モデル".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "fixture only".into(),
        target: Target::current().unwrap(),
        files: vec![
            PackageFile {
                relative_path: "engine-cli".into(),
                role: Role::Executable,
                expected_size: None,
                expected_hash: None,
            },
            PackageFile {
                relative_path: "weights/model.bin".into(),
                role: Role::Asr,
                expected_size: None,
                expected_hash: None,
            },
        ],
    };
    std::fs::write(
        source.path().join(model_inspection::MANIFEST),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let report =
        model_inspection::inspect(source.path(), &crate::test_support::never_cancelled).unwrap();
    let meetings = tempfile::tempdir().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let storage = Storage::open(meetings.path().into()).unwrap();
    storage.create_meeting("既存会議を保護").unwrap();
    Fixture {
        source,
        meetings,
        app_data,
        storage,
        report,
    }
}
fn placement(
    f: &Fixture,
    token: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<PlacementReceipt, String> {
    place(
        &f.storage,
        &f.app_data.path().join("models"),
        f.source.path(),
        token,
        &f.report.review_hash,
        cancelled,
    )
}
#[test]
fn manual_v1_record_keeps_legacy_serialized_bytes_hash_and_no_download_origin() {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Legacy<'a> {
        schema_version: u32,
        package_id: &'a str,
        manifest: &'a PackageManifest,
        source_manifest_hash: &'a str,
        review_hash: &'a str,
        files: &'a [FileReport],
    }
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let record = PlacementRecord::from_report(&token, &f.report);
    let old = serde_json::to_vec(&Legacy {
        schema_version: 1,
        package_id: &token,
        manifest: &f.report.manifest,
        source_manifest_hash: &f.report.manifest_hash,
        review_hash: &f.report.review_hash,
        files: &f.report.files,
    })
    .unwrap();
    let new = serde_json::to_vec(&record).unwrap();
    assert_eq!(new, old);
    assert_eq!(Sha256::digest(&new), Sha256::digest(&old));
    assert!(record.download.is_none());
    assert_eq!(record.source_type(), "manual_import");
    let decoded: PlacementRecord = serde_json::from_slice(&old).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, record);
}
#[test]
fn verified_copy_is_separate_private_persistent_idempotent_and_not_ai_ready() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let receipt = placement(&f, &token, &crate::test_support::never_cancelled).unwrap();
    assert!(receipt.placement_recorded && !receipt.ready);
    assert_eq!(receipt.engine_compatibility, "not_verified");
    assert_eq!(receipt.review_hash, f.report.review_hash);
    let models = f.app_data.path().join("models");
    let package = models.join(&token);
    for file in &f.report.files {
        let source = f.source.path().join(&file.relative_path);
        let target = package.join(&file.relative_path);
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&target).unwrap()
        );
        assert_eq!(
            target.metadata().unwrap().permissions().mode() & 0o777,
            if file.role == Role::Executable {
                0o700
            } else {
                0o600
            }
        );
    }
    assert_eq!(
        package.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!f.meetings.path().join("models").exists());
    placement(&f, &token, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(std::fs::read_dir(&models).unwrap().count(), 1);
    let old_meetings = f.storage.list_meetings().unwrap();
    let old_preferences = f.storage.analysis_preferences().unwrap().payload;
    assert_eq!(f.storage.list_model_placements().unwrap().len(), 1);
    drop(f.storage);
    std::fs::rename(f.source.path(), f.app_data.path().join("moved-source")).unwrap();
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert_eq!(reopened.list_meetings().unwrap(), old_meetings);
    assert_eq!(
        reopened.analysis_preferences().unwrap().payload,
        old_preferences
    );
    let entry = reopened.list_model_placements().unwrap().remove(0);
    assert!(entry.placement_recorded && !entry.ready);
    assert_eq!(entry.review_hash, f.report.review_hash);
    assert_eq!(entry.quantization, f.report.manifest.quantization);
    let expected: Vec<_> = f
        .report
        .files
        .iter()
        .filter(|file| {
            !matches!(
                file.role,
                Role::Executable | Role::RuntimeLibrary | Role::Notice
            )
        })
        .collect();
    assert_eq!(entry.model_files.len(), expected.len());
    for (actual, expected) in entry.model_files.iter().zip(expected) {
        assert_eq!(actual.role, expected.role);
        assert_eq!(actual.size, expected.actual_size);
        assert_eq!(actual.sha256, expected.actual_hash);
    }
    assert!(model_inspection::inspect(&package, &crate::test_support::never_cancelled).is_ok());
}
#[test]
fn reviewed_bytes_bind_unknown_publisher_hashes_and_manifest_changes() {
    for change_manifest in [false, true] {
        let f = fixture();
        if change_manifest {
            std::fs::write(
                f.source.path().join(model_inspection::MANIFEST),
                serde_json::to_vec(&f.report.manifest).unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(
                f.source.path().join("weights/model.bin"),
                vec![b'z'; 150_000],
            )
            .unwrap();
        }
        let token = uuid::Uuid::new_v4().to_string();
        assert!(placement(&f, &token, &crate::test_support::never_cancelled)
            .unwrap_err()
            .contains("再検査"));
        assert!(!f.app_data.path().join("models").exists());
        assert!(f.storage.list_model_placements().unwrap().is_empty());
    }
}

#[test]
fn model_placement_and_backup_maintenance_exclude_each_others_writes() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let record = PlacementRecord::from_report(&token, &f.report);
    f.storage.begin_model_placement(&record).unwrap();
    // Backup and additive publication share this maintenance gate. No snapshot
    // can observe a half-written model placement as a completed preparation.
    assert!(f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("モデル準備"));
    f.storage.interrupt_model_placement(&record).unwrap();
    let snapshot = f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    let next = uuid::Uuid::new_v4().to_string();
    assert!(placement(&f, &next, &crate::test_support::never_cancelled).is_err());
    assert!(!f.app_data.path().join("models").join(&next).exists());
    assert_eq!(f.storage.list_model_placements().unwrap().len(), 1);
    drop(snapshot);
    let placed = placement(&f, &next, &crate::test_support::never_cancelled).unwrap();
    assert!(placed.placement_recorded && !placed.ready);
    assert!(f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_ok());
}
#[test]
fn cancellation_after_intent_keeps_partial_bytes_unavailable_across_restart() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let calls = Cell::new(0);
    let result = placement(&f, &token, &|| {
        calls.set(calls.get() + 1);
        let path = f
            .app_data
            .path()
            .join("models")
            .join(&token)
            .join("weights/model.bin");
        path.metadata().is_ok_and(|m| m.len() > 0)
    });
    assert!(result.is_err());
    let partial = f
        .app_data
        .path()
        .join("models")
        .join(&token)
        .join("weights/model.bin");
    let before = std::fs::read(&partial).unwrap();
    assert!(!before.is_empty() && before.len() < 150_000);
    let entries = f.storage.list_model_placements().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].placement_recorded && !entries[0].ready);
    assert!(placement(&f, &token, &crate::test_support::never_cancelled).is_err());
    assert_eq!(std::fs::read(&partial).unwrap(), before);
    drop(f.storage);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert!(!reopened.list_model_placements().unwrap()[0].placement_recorded);
    assert_eq!(std::fs::read(&partial).unwrap(), before);
    let next = uuid::Uuid::new_v4().to_string();
    place(
        &reopened,
        &f.app_data.path().join("models"),
        f.source.path(),
        &next,
        &f.report.review_hash,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(reopened.list_model_placements().unwrap().len(), 2);
    assert_eq!(std::fs::read(&partial).unwrap(), before);
}
#[test]
fn colliding_package_and_symlink_root_never_overwrite_foreign_data() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let root = f.app_data.path().join("models");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(root.join(&token)).unwrap();
    let foreign = root.join(&token).join("existing.txt");
    std::fs::write(&foreign, b"preserve me").unwrap();
    assert!(placement(&f, &token, &crate::test_support::never_cancelled).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"preserve me");
    assert!(!f.storage.list_model_placements().unwrap()[0].placement_recorded);
    let other = fixture();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), other.app_data.path().join("models")).unwrap();
    assert!(placement(
        &other,
        &uuid::Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(other.storage.list_model_placements().unwrap().is_empty());
}
#[test]
fn descriptor_relative_creation_rejects_leaf_parent_substitution_and_existing_files() {
    let parent = tempfile::tempdir().unwrap();
    let root = fs::Directory::root(&parent.path().join("models")).unwrap();
    let package = root.child(&uuid::Uuid::new_v4().to_string(), true).unwrap();
    let external = tempfile::tempdir().unwrap();
    symlink(external.path(), package.path().join("weights")).unwrap();
    assert!(package.create("weights/new.bin").is_err());
    assert!(!external.path().join("new.bin").exists());
    let (mut file, dir) = package.create("existing.bin").unwrap();
    file.write_all(b"original").unwrap();
    file.sync_all().unwrap();
    dir.sync().unwrap();
    assert!(package.create("existing.bin").is_err());
    assert_eq!(
        std::fs::read(package.path().join("existing.bin")).unwrap(),
        b"original"
    );
    let moved = parent.path().join("moved-models");
    std::fs::rename(root.path(), &moved).unwrap();
    symlink(external.path(), root.path()).unwrap();
    assert!(root.child("new-child", true).is_err());
    assert!(!external.path().join("new-child").exists());
}
#[test]
fn successful_retry_revalidates_target_and_metadata_corruption_is_rejected() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    placement(&f, &token, &crate::test_support::never_cancelled).unwrap();
    let target = f
        .app_data
        .path()
        .join("models")
        .join(&token)
        .join("weights/model.bin");
    std::fs::write(&target, b"tampered target").unwrap();
    assert!(placement(&f, &token, &crate::test_support::never_cancelled).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"tampered target");
    drop(f.storage);
    let db = rusqlite::Connection::open(f.meetings.path().join("meetings.sqlite3")).unwrap();
    db.execute(
        "UPDATE model_files SET actual_hash=?1 WHERE package_id=?2 AND role='asr'",
        rusqlite::params!["a".repeat(64), token],
    )
    .unwrap();
    drop(db);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    assert!(reopened.list_model_placements().is_err());
}
#[test]
fn source_substitution_during_copy_preserves_source_and_never_commits_placement() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let changed = Cell::new(false);
    let original = f.source.path().join("weights/model.bin");
    let saved = f.source.path().join("saved-model.bin");
    let result = placement(&f, &token, &|| {
        let partial = f
            .app_data
            .path()
            .join("models")
            .join(&token)
            .join("weights/model.bin");
        if !changed.get() && partial.metadata().is_ok_and(|m| m.len() > 0) {
            std::fs::rename(&original, &saved).unwrap();
            std::fs::write(&original, vec![b'z'; 150_000]).unwrap();
            changed.set(true);
        }
        false
    });
    assert!(result.is_err());
    assert!(changed.get());
    assert_eq!(std::fs::read(&saved).unwrap(), vec![b'm'; 150_000]);
    assert_eq!(std::fs::read(&original).unwrap(), vec![b'z'; 150_000]);
    assert!(!f.storage.list_model_placements().unwrap()[0].placement_recorded);
}

#[test]
fn startup_interrupts_preparing_intent_without_resuming_copy_or_deleting_partial_files() {
    let f = fixture();
    let token = uuid::Uuid::new_v4().to_string();
    let record = PlacementRecord::from_report(&token, &f.report);
    f.storage.begin_model_placement(&record).unwrap();
    let root = fs::Directory::root(&f.app_data.path().join("models")).unwrap();
    let package = root.child(&token, true).unwrap();
    let (mut file, parent) = package.create("weights/model.bin").unwrap();
    file.write_all(b"partial body kept after restart").unwrap();
    file.sync_all().unwrap();
    parent.sync().unwrap();
    drop(f.storage);
    let reopened = Storage::open(f.meetings.path().into()).unwrap();
    let entry = reopened.list_model_placements().unwrap().remove(0);
    assert!(!entry.ready && !entry.placement_recorded);
    assert_eq!(
        std::fs::read(package.path().join("weights/model.bin")).unwrap(),
        b"partial body kept after restart"
    );
    assert!(!package.path().join("engine-cli").exists());
}
#[test]
fn model_intent_cannot_reuse_an_existing_meeting_identity() {
    let f = fixture();
    let token = f.storage.list_meetings().unwrap()[0].id.clone();
    assert!(placement(&f, &token, &crate::test_support::never_cancelled).is_err());
    assert!(f.storage.list_model_placements().unwrap().is_empty());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    assert!(!f.app_data.path().join("models").join(token).exists());
}
