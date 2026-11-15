use super::super::tests::Fixture;
use super::*;
use crate::{analysis_worker::AnalysisController, model_inspection, model_placement};

fn setup() -> (Fixture, Journal) {
    let f = Fixture::new();
    let keeper = uuid::Uuid::new_v4().to_string();
    let report = model_inspection::inspect(&f.source, &|| false).unwrap();
    model_placement::place(
        &f.storage,
        &f.root,
        &f.source,
        &keeper,
        &report.review_hash,
        &|| false,
    )
    .unwrap();
    let source =
        super::super::capture_mode(&f.storage, &f.root, &f.package, &f.review, &|| false, false)
            .unwrap();
    let keeper = super::super::capture_mode(
        &f.storage,
        &f.root,
        &keeper,
        &report.review_hash,
        &|| false,
        false,
    )
    .unwrap();
    let journal = Journal {
        version: 1,
        models_root: f.root.to_str().unwrap().into(),
        source,
        keeper,
        next: 0,
        pending: None,
    };
    (f, journal)
}
#[test]
fn automatic_cleanup_removes_only_duplicate_body_keeps_history_and_does_not_fence_meetings() {
    let (f, mut j) = setup();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    save(&f.storage, &j, None, &lease, &|| false).unwrap();
    assert!(!f.storage.data_write_blocked());
    f.storage.create_meeting("整理中にも会議を保存").unwrap();
    assert!(
        f.storage
            .list_model_placements()
            .unwrap()
            .iter()
            .find(|e| e.package_id == f.package)
            .unwrap()
            .body_removed
    );
    run(&f.storage, &mut j, &lease, &|| false).unwrap();
    assert!(j.complete());
    assert!(!f.root.join(&f.package).exists());
    assert!(f
        .root
        .join(&j.keeper.record.package_id)
        .join("weights/model.bin")
        .exists());
    assert_eq!(f.storage.list_model_placements().unwrap().len(), 2);
    assert!(!f.storage.data_write_blocked());
    // Terminal readback does not perform another unlink or adopt a reappeared root.
    run(&f.storage, &mut j, &lease, &|| false).unwrap();
    std::fs::create_dir(f.root.join(&f.package)).unwrap();
    std::fs::write(f.root.join(&f.package).join("external"), b"keep").unwrap();
    assert!(run(&f.storage, &mut j, &lease, &|| false).is_err());
    assert!(f.root.join(&f.package).join("external").exists());
}
#[test]
fn automatic_cleanup_resumes_after_journaled_rename_and_reopen() {
    for renamed in [false, true] {
        let (f, mut j) = setup();
        let controller = AnalysisController::default();
        let lease = controller
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        save(&f.storage, &j, None, &lease, &|| false).unwrap();
        let models = Directory::root(&f.root).unwrap();
        let o = j.source.observations.last().unwrap().clone();
        let held = parent(&models, &j, &o.entry.relative_path)
            .unwrap()
            .unwrap();
        let leaf = o.entry.relative_path.rsplit('/').next().unwrap();
        let mut next = j.clone();
        next.pending = Some(Pending {
            before: observed(&held, leaf, &o.entry, &|| false).unwrap(),
            staged: None,
        });
        save(&f.storage, &next, Some(&j), &lease, &|| false).unwrap();
        j = next;
        if renamed {
            held.rename_exclusive(leaf, &j.reserved(0)).unwrap();
        }
        drop(held);
        drop(models);
        drop(lease);
        let f = f.cold();
        assert!(!f.storage.data_write_blocked());
        let lease = controller
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        let mut saved = {
            let db = f.storage.connection.lock().unwrap();
            read(&db, &f.package).unwrap().unwrap()
        };
        run(&f.storage, &mut saved, &lease, &|| false).unwrap();
        assert!(saved.complete());
        assert!(!f.root.join(&f.package).exists());
    }
}
#[test]
fn automatic_cleanup_rejects_changed_keeper_unknown_files_and_cancellation() {
    for change in ["keeper", "unknown", "cancel"] {
        let (f, mut j) = setup();
        let controller = AnalysisController::default();
        let lease = controller
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        if change == "keeper" {
            std::fs::write(
                f.root
                    .join(&j.keeper.record.package_id)
                    .join("weights/model.bin"),
                b"changed",
            )
            .unwrap();
        }
        if change == "unknown" {
            std::fs::write(f.root.join(&f.package).join("external"), b"keep").unwrap();
        }
        let check = || change == "cancel";
        let result = save(&f.storage, &j, None, &lease, &check)
            .and_then(|_| run(&f.storage, &mut j, &lease, &check));
        assert!(result.is_err(), "{change}");
        assert!(f.root.join(&f.package).join("weights/model.bin").exists());
        assert!(!f.storage.data_write_blocked());
    }
}
#[test]
fn automatic_cleanup_journal_never_accepts_other_models_or_corrupt_saved_state() {
    let (f, mut j) = setup();
    j.keeper.record.files[1].actual_hash = "f".repeat(64);
    assert!(j.validate().is_err());
    let (f2, j) = setup();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    save(&f2.storage, &j, None, &lease, &|| false).unwrap();
    let db = f2.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE app_settings SET content_hash=?2 WHERE key=?1",
        params![format!("{PREFIX}{}", f2.package), "0".repeat(64)],
    )
    .unwrap();
    assert!(read(&db, &f2.package).is_err());
    assert!(f.root.join(&f.package).exists() && f2.root.join(&f2.package).exists());
}

#[test]
fn automatic_cleanup_protects_resumable_jobs_but_not_completed_history() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE app_settings(key,value_json,content_hash,lock_version); CREATE TABLE model_profiles(id); CREATE TABLE processing_jobs(state,profile_snapshot_json);").unwrap();
    for state in [
        "queued",
        "running",
        "pausing",
        "paused",
        "interrupted",
        "failed",
        "cancelled",
        "succeeded",
    ] {
        db.execute("DELETE FROM processing_jobs", []).unwrap();
        db.execute(
            "INSERT INTO processing_jobs VALUES(?1,?2)",
            params![
                state,
                r#"{"managedModel":{"package":{"packageId":"target"}}}"#
            ],
        )
        .unwrap();
        assert_eq!(
            protected(&db, "target").unwrap(),
            matches!(
                state,
                "queued" | "running" | "pausing" | "paused" | "interrupted"
            )
        );
        assert!(!protected(&db, "other").unwrap());
    }
}

#[test]
fn automatic_cleanup_keeps_selection_otherwise_the_latest_checked_model() {
    let (f, _) = setup();
    let mut group = f.storage.list_model_placements().unwrap();
    for e in &mut group {
        e.compatibility = Some(crate::model_compatibility::CheckSummary {
            operation_id: uuid::Uuid::new_v4().to_string(),
            package_id: e.package_id.clone(),
            review_hash: e.review_hash.clone(),
            adapter_revision: 1,
            state: crate::model_compatibility::CheckState::Succeeded,
            error_message: None,
        });
    }
    assert_eq!(
        keeper(&group, &BTreeSet::new()).unwrap().package_id,
        group[0].package_id
    );
    assert_eq!(
        keeper(&group, &BTreeSet::from([group[1].package_id.clone()]))
            .unwrap()
            .package_id,
        group[1].package_id
    );
    group[0].compatibility = None;
    assert_eq!(
        keeper(&group, &BTreeSet::new()).unwrap().package_id,
        group[1].package_id
    );
    group[1].compatibility = None;
    assert!(keeper(&group, &BTreeSet::new()).is_none());
}

#[test]
fn automatic_cleanup_sweep_retires_old_profile_body_after_success_without_changing_selection() {
    use crate::model_compatibility::{artifact_names, Artifact, CheckState, ProbeReport};
    use crate::storage::model_profiles::ProfileName;
    let (f, j) = setup();
    for record in [&j.source.record, &j.keeper.record] {
        let check = uuid::Uuid::new_v4().to_string();
        f.storage.begin_model_check(&check, record).unwrap();
        let report = ProbeReport {
            schema_version: 1,
            operation_id: check.clone(),
            package_id: record.package_id.clone(),
            review_hash: record.review_hash.clone(),
            engine: record.manifest.engine,
            declared_engine_version: record.manifest.engine_version.clone(),
            adapter_revision: 1,
            settings: crate::model_compatibility::settings(record.manifest.engine),
            elapsed_ms: 1,
            artifacts: artifact_names(record.manifest.engine)
                .iter()
                .map(|name| Artifact {
                    name: (*name).into(),
                    size: 1,
                    sha256: "a".repeat(64),
                })
                .collect(),
        };
        f.storage
            .finish_model_check(&check, record, Some(&report), CheckState::Succeeded, None)
            .unwrap();
        let directory = f.root.join(&record.package_id);
        let preview = f
            .storage
            .preview_model_profile_with_tuning(
                ProfileName::Standard,
                record,
                &directory,
                &check,
                None,
                None,
            )
            .unwrap();
        f.storage
            .apply_model_profile(
                &uuid::Uuid::new_v4().to_string(),
                &preview,
                record,
                &directory,
                &|| false,
            )
            .unwrap();
    }
    let before = serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let mut failed = HashSet::new();
    assert!(sweep(&f.storage, &f.root, &lease, &mut failed, &|| false).unwrap());
    assert!(!f.root.join(&f.package).exists());
    assert!(f.root.join(&j.keeper.record.package_id).exists());
    assert_eq!(
        serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap(),
        before
    );
    assert!(!sweep(&f.storage, &f.root, &lease, &mut failed, &|| false).unwrap());
    assert!(failed.is_empty());
    drop(lease);
    let f = f.cold();
    assert_eq!(
        serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap(),
        before
    );
    assert!(!f.storage.data_write_blocked());
}
