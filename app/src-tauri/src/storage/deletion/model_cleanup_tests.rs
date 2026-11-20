use super::super::tests::Fixture;
use super::*;
use crate::{analysis_worker::AnalysisController, model_inspection, model_placement};

fn setup() -> (Fixture, Journal) {
    setup_with_fixture(Fixture::new())
}

fn setup_with_fixture(f: Fixture) -> (Fixture, Journal) {
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
        job: None,
    };
    (f, journal)
}

fn folder_job(keeper: &str) -> Job {
    Job {
        id: uuid::Uuid::new_v4().to_string(),
        target: "合成モデル".into(),
        started_at_ms: 1,
        applied_package: Some(keeper.into()),
    }
}

#[test]
fn folder_cleanup_resumes_original_job_and_preserves_unowned_changes() {
    for stage in ["armed", "renamed", "partial", "removed", "extra", "changed"] {
        let (f, mut journal) = setup();
        let controller = AnalysisController::default();
        let lease = controller
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        let job = folder_job(&journal.keeper.record.package_id);
        journal.version = 2;
        journal.job = Some(job.clone());
        let before = journal.source.observations[0]
            .proof
            .as_ref()
            .unwrap()
            .directory_scope();
        journal.pending = Some(Pending {
            before: before.clone(),
            staged: None,
        });
        save(&f.storage, &journal, None, &lease, &|| false).unwrap();
        let reserved = format!(".mon-delete-cleanup-{}-folder", f.package);
        let staged = f.root.join(&reserved);
        if stage != "armed" {
            Directory::root(&f.root)
                .unwrap()
                .rename_exclusive(&f.package, &reserved)
                .unwrap();
        }
        if !matches!(stage, "armed" | "renamed") {
            let prior = journal.clone();
            journal.pending.as_mut().unwrap().staged = Some(before);
            save(&f.storage, &journal, Some(&prior), &lease, &|| false).unwrap();
        }
        match stage {
            "partial" => std::fs::remove_file(staged.join("cli")).unwrap(),
            "removed" => std::fs::remove_dir_all(&staged).unwrap(),
            "extra" => std::fs::write(staged.join("unregistered"), b"keep").unwrap(),
            "changed" => std::fs::write(staged.join("weights/model.bin"), b"keep").unwrap(),
            _ => {}
        }
        let f = f.cold();
        let targets = plan(&f.storage, None).unwrap();
        assert_eq!(targets.len(), 1, "{stage}");
        let target = targets.into_iter().next().unwrap();
        assert_eq!(target.job(), Some(&job));
        let result = execute(&f.storage, &f.root, vec![target], &job, &lease);
        if matches!(stage, "extra" | "changed") {
            assert!(result.is_err(), "{stage}");
            let changed = if stage == "extra" {
                "unregistered"
            } else {
                "weights/model.bin"
            };
            assert_eq!(std::fs::read(staged.join(changed)).unwrap(), b"keep");
            assert!(staged.join("cli").exists());
        } else {
            result.unwrap();
            assert!(
                !staged.exists() && !f.root.join(&f.package).exists(),
                "{stage}"
            );
            assert!(plan(&f.storage, None).unwrap().is_empty());
        }
        assert!(f
            .root
            .join(&journal.keeper.record.package_id)
            .join("weights/model.bin")
            .exists());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn folder_cleanup_reads_each_package_once_and_records_three_boundaries() {
    use crate::operation_observation;
    use std::time::Instant;

    let controller = AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let (old, mut journal) = setup_with_fixture(Fixture::with_weight_size(32 * 1024 * 1024));
    let (new, candidate) = setup_with_fixture(Fixture::with_weight_size(32 * 1024 * 1024));
    let (old_read, old_checks, old_elapsed) = {
        let started = Instant::now();
        let _measure = operation_observation::Scope::enter("model_cleanup_test", |_| {});
        for item in [&mut journal.source, &mut journal.keeper] {
            *item = super::super::capture_mode(
                &old.storage,
                &old.root,
                &item.record.package_id,
                &item.record.review_hash,
                &|| false,
                false,
            )
            .unwrap();
        }
        save(&old.storage, &journal, None, &lease, &|| false).unwrap();
        run_legacy(&old.storage, &mut journal, &lease, &|| false).unwrap();
        let (bytes, checks) = operation_observation::measurements();
        (bytes, checks, started.elapsed())
    };
    let (new_read, new_checks, new_elapsed) = {
        let entries = new.storage.list_model_placements().unwrap();
        let entry = |id: &str| {
            Box::new(
                entries
                    .iter()
                    .find(|item| item.package_id == id)
                    .unwrap()
                    .clone(),
            )
        };
        let target = Target {
            work: Work::Duplicate {
                source: entry(&new.package),
                keeper: entry(&candidate.keeper.record.package_id),
            },
        };
        let started = Instant::now();
        let _measure = operation_observation::Scope::enter("model_cleanup_test", |_| {});
        execute(
            &new.storage,
            &new.root,
            vec![target],
            &folder_job(&candidate.keeper.record.package_id),
            &lease,
        )
        .unwrap();
        let (bytes, checks) = operation_observation::measurements();
        (bytes, checks, started.elapsed())
    };
    eprintln!("legacy: {old_read} bytes, {old_checks} hashes, {old_elapsed:?}; folder: {new_read} bytes, {new_checks} hashes, {new_elapsed:?}");
    let expected = candidate
        .source
        .observations
        .iter()
        .chain(&candidate.keeper.observations)
        .filter_map(|item| item.entry.bytes)
        .collect::<Vec<_>>();
    assert_eq!(new_read, expected.iter().sum::<u64>());
    assert_eq!(new_checks, expected.len() as u64);
    assert!(old_read > new_read * 2);
    let db = new.storage.connection.lock().unwrap();
    let writes: i64 = db
        .query_row(
            "SELECT lock_version FROM app_settings WHERE key=?1",
            [format!("{PREFIX}{}", new.package)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(writes, 3);
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
    run_legacy(&f.storage, &mut j, &lease, &|| false).unwrap();
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
    run_legacy(&f.storage, &mut j, &lease, &|| false).unwrap();
    std::fs::create_dir(f.root.join(&f.package)).unwrap();
    std::fs::write(f.root.join(&f.package).join("external"), b"keep").unwrap();
    assert!(run_legacy(&f.storage, &mut j, &lease, &|| false).is_err());
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
        run_legacy(&f.storage, &mut saved, &lease, &|| false).unwrap();
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
            .and_then(|_| run_legacy(&f.storage, &mut j, &lease, &check));
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
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        target: "重複モデルの整理".into(),
        started_at_ms: 1,
        applied_package: None,
    };
    let targets = plan(&f.storage, Some(&j.keeper.record.package_id)).unwrap();
    assert_eq!(targets.len(), 1);
    execute(&f.storage, &f.root, targets, &job, &lease).unwrap();
    assert!(!f.root.join(&f.package).exists());
    assert!(f.root.join(&j.keeper.record.package_id).exists());
    assert_eq!(
        serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap(),
        before
    );
    assert!(plan(&f.storage, None).unwrap().is_empty());
    drop(lease);
    let f = f.cold();
    assert_eq!(
        serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap(),
        before
    );
    assert!(!f.storage.data_write_blocked());
}
