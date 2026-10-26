//! Synthetic model bodies only. Child processes never execute a model engine.
#[path = "model_renewal_tests.rs"]
mod renewing;
use super::*;
use intent::removal;
use std::{
    cell::Cell,
    os::unix::process::ExitStatusExt,
    panic::{catch_unwind, AssertUnwindSafe},
};

fn prepare(f: &Fixture, include_unknown: bool, owner: &DataMaintenanceLease) -> String {
    let preview = f.preview().unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    consent(
        f,
        &op,
        &preview.census_hash,
        include_unknown,
        owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    op
}
fn history(f: &Fixture) -> String {
    let db = f.storage.connection.lock().unwrap();
    db.query_row("SELECT json_array((SELECT json_group_array(json_array(id,manifest_json,manifest_hash,state)) FROM model_packages),(SELECT json_group_array(json_array(id,package_id,relative_path,role,actual_hash,state)) FROM model_files),(SELECT json_group_array(json_array(id,package_id,state,review_hash,error_message,created_at,finished_at)) FROM model_compatibility_checks),(SELECT json_group_array(json_array(id,name,profile_version,state)) FROM model_profiles),(SELECT json_group_array(json_array(profile_id,role,package_id,settings_json)) FROM profile_components),(SELECT json_group_array(json_array(id,state,profile_snapshot_json)) FROM processing_jobs),(SELECT json_group_array(json_array(key,value_json,content_hash,lock_version,updated_at)) FROM app_settings),(SELECT json_group_array(json_array(id,title)) FROM meetings))",[],|r|r.get(0)).unwrap()
}
#[test]
fn model_deletion_body_removes_only_fixed_package_and_preserves_cpu_profiles_jobs_settings() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    db.execute("INSERT INTO model_compatibility_checks(id,package_id,review_hash,adapter_revision,state,error_message,created_at,finished_at) VALUES(?1,?2,?3,1,'cancelled','synthetic old check',1,1)",params![uuid::Uuid::new_v4().to_string(),f.package,f.review]).unwrap();
    db.execute("INSERT INTO model_profiles(id,name,profile_version,state) VALUES('historical','standard',1,'preparing')",[]).unwrap();
    db.execute("INSERT INTO profile_components(profile_id,role,package_id,settings_json) VALUES('historical','asr',?1,'{}')",params![f.package]).unwrap();
    db.execute(
        "UPDATE model_profiles SET state='ready' WHERE id='historical'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('synthetic','{}',?1,1,1)",params![format!("{:x}",Sha256::digest(b"{}"))]).unwrap();
    let meeting: String = db
        .query_row("SELECT id FROM meetings LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let profile =
        serde_json::json!({"managedModel":{"package":{"packageId":f.package}}}).to_string();
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('history',?1,'transcription','paused','history',?2,?3,'{\"schemaVersion\":4}',?2,1,1)",params![meeting,"a".repeat(64),profile]).unwrap();
    drop(db);
    let other = f.root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("KEEP"), b"OTHER MODEL").unwrap();
    let keep = f.storage.root().join("keep-audio");
    std::fs::write(&keep, b"SYNTHETIC AUDIO KEEP").unwrap();
    let before = history(&f);
    let owner = lease();
    let op = prepare(&f, false, &owner);
    let consent = receipt(&f, &op);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let done = removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(done.completed());
    assert!(!f.path("").exists());
    assert!(!f.storage.data_write_blocked());
    assert_eq!(before, history(&f));
    assert_eq!(consent, receipt(&f, &op));
    assert_eq!(std::fs::read(other.join("KEEP")).unwrap(), b"OTHER MODEL");
    assert_eq!(std::fs::read(keep).unwrap(), b"SYNTHETIC AUDIO KEEP");
    assert_eq!(
        std::fs::read(f.source.join("cli")).unwrap(),
        b"synthetic; do not execute"
    );
    let db = f.storage.connection.lock().unwrap();
    assert!(
        crate::storage::model_placements::load_history(&db, &f.package)
            .unwrap()
            .unwrap()
            .1
    );
    assert!(
        !crate::storage::model_placements::load(&db, &f.package)
            .unwrap()
            .unwrap()
            .1
    );
    drop(db);
    let catalog = f.storage.list_model_placements().unwrap();
    assert!(
        catalog[0].placement_recorded
            && catalog[0].body_removed
            && catalog[0].compatibility.is_some()
    );
    assert!(f
        .storage
        .model_package_for_check(&f.package, &f.review)
        .is_err());
    assert_eq!(
        removal::run(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled
        )
        .unwrap(),
        done
    );
    assert!(intent::keep_unstarted(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn model_deletion_body_cancellation_and_cold_resume_at_every_journal_boundary() {
    for boundary in [
        "armed",
        "beforeRename",
        "renamed",
        "staged",
        "beforeUnlink",
        "unlinked",
        "advanced",
        "beforeComplete",
    ] {
        let mut f = Fixture::new();
        let owner = lease();
        let op = prepare(&f, false, &owner);
        let before = history(&f);
        let consent = receipt(&f, &op);
        removal::start(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let stop = Cell::new(false);
        assert!(
            removal::run_observed(&f.storage, &op, &owner, &|| stop.get(), &|name| if name
                == boundary
            {
                stop.set(true)
            })
            .is_err(),
            "{boundary}"
        );
        assert!(f.storage.data_write_blocked());
        assert_eq!(before, history(&f));
        assert!(intent::keep_unstarted(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled
        )
        .is_err());
        drop(owner);
        reopen(&mut f, |_| {});
        let controller = AnalysisController::default();
        controller.require_storage_restart();
        let owner = controller
            .begin_model_deletion_maintenance(
                &f.storage,
                &op,
                false,
                false,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        assert!(
            removal::run(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .unwrap()
            .completed(),
            "{boundary}"
        );
        assert!(!f.path("").exists());
        assert_eq!(before, history(&f));
        assert_eq!(consent, receipt(&f, &op));
        assert!(f
            .storage
            .require_data_writable()
            .unwrap_err()
            .contains("再起動"));
        drop(owner);
        reopen(&mut f, |_| {});
        assert!(!f.storage.data_write_blocked());
    }
}

#[test]
fn model_deletion_body_missing_and_changed_registered_files_need_fixed_consent_not_recreation() {
    let f = Fixture::new();
    std::fs::remove_file(f.path("weights/model.bin")).unwrap();
    std::fs::write(f.path("cli"), b"REVIEWED CHANGED CLI; DO NOT EXECUTE").unwrap();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    assert!(!f.path("").exists());
    let f = Fixture::new();
    std::fs::rename(f.path(""), f.root.join("held")).unwrap();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.path("").exists());
    assert!(f.root.join("held/cli").exists());
}

#[test]
fn model_deletion_body_explicit_unknowns_are_removed_but_new_contents_are_never_adopted() {
    let f = Fixture::new();
    std::fs::create_dir(f.path("unknown-dir")).unwrap();
    std::fs::write(f.path("unknown-dir/reviewed"), b"EXPLICITLY REVIEWED").unwrap();
    let owner = lease();
    let op = prepare(&f, true, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.path("").exists());
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(f.path("added"), b"NEVER DELETE").unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read(f.path("added")).unwrap(), b"NEVER DELETE");
    assert!(f.path("cli").exists() && f.path("weights/model.bin").exists());
}

#[test]
fn model_deletion_body_changed_tree_at_irreversible_boundary_blocks_unlink() {
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let changed = Cell::new(false);
    assert!(removal::run_observed(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
        &|name| if name == "beforeUnlink" && !changed.replace(true) {
            std::fs::write(f.path("added"), b"RACING KEEP").unwrap();
        }
    )
    .is_err());
    assert_eq!(std::fs::read(f.path("added")).unwrap(), b"RACING KEEP");
    assert!(f
        .path("weights/.mon-delete-model-")
        .parent()
        .unwrap()
        .read_dir()
        .unwrap()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mon-delete-model-")));
    assert!(f.path("cli").exists());
    assert!(f.storage.data_write_blocked());
}

#[test]
fn model_deletion_body_reserved_name_collision_and_links_are_retained() {
    for kind in ["root-reservation", "link", "hardlink", "root-replaced"] {
        let f = Fixture::new();
        let owner = lease();
        let op = prepare(&f, false, &owner);
        let held = f.root.parent().unwrap().join("held");
        if kind == "root-reservation" {
            std::fs::write(
                f.root.join(format!(".mon-delete-model-{op}-4")),
                b"UNKNOWN RESERVED",
            )
            .unwrap();
        }
        if kind == "link" {
            std::fs::rename(f.path("cli"), &held).unwrap();
            std::os::unix::fs::symlink(&held, f.path("cli")).unwrap();
        }
        if kind == "hardlink" {
            std::fs::hard_link(f.path("cli"), &held).unwrap();
        }
        if kind == "root-replaced" {
            std::fs::rename(&f.root, &held).unwrap();
            std::fs::create_dir(&f.root).unwrap();
        }
        assert!(
            removal::start(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{kind}"
        );
        if kind != "root-reservation" {
            assert!(held.exists());
        }
        assert!(f.storage.data_write_blocked());
    }
}

#[test]
fn model_deletion_body_completed_record_does_not_authorize_reappeared_contents() {
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::create_dir(f.path("")).unwrap();
    std::fs::write(f.path("late"), b"REAPPEARED KEEP").unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read(f.path("late")).unwrap(), b"REAPPEARED KEEP");
    assert!(f.storage.list_model_placements().unwrap()[0].body_removed);
}

#[test]
fn model_deletion_body_panic_releases_writer_without_releasing_persistent_fence() {
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(catch_unwind(AssertUnwindSafe(|| removal::run_observed(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
        &|name| if name == "renamed" {
            panic!("synthetic interruption")
        }
    )))
    .is_err());
    // File I/O no longer holds the DB mutex. A panic must leave it usable,
    // while the durable deletion fence still rejects ordinary writes.
    assert!(f.storage.data_write_blocked());
    let db = f.storage.connection.lock().unwrap();
    assert!(db.execute("UPDATE meetings SET title=title", []).is_err());
    drop(db);
    assert!(f.storage.list_meetings().is_ok());
    let mut f = f;
    drop(owner);
    reopen(&mut f, |_| {});
    assert!(removal::run(
        &f.storage,
        &op,
        &lease(),
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
}

#[test]
fn model_deletion_body_cancelled_real_lease_rejects_resume_and_preserves_remaining() {
    let f = Fixture::new();
    let controller = AnalysisController::default();
    let owner = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let op = prepare(&f, false, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f.path("cli").exists() && f.path("weights/model.bin").exists());
    drop(owner);
    let next = controller
        .begin_model_deletion_maintenance(
            &f.storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &next,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
}

#[test]
fn model_deletion_body_corrupt_progress_is_not_deletion_or_availability_authority() {
    for case in ["cursor", "hash", "unknown", "completed"] {
        let mut f = Fixture::new();
        let owner = lease();
        let op = prepare(&f, false, &owner);
        removal::start(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        reopen(&mut f, |db| {
            let json: String = db
                .query_row(
                    "SELECT progress_json FROM maintenance_operations WHERE id=?1",
                    params![op],
                    |r| r.get(0),
                )
                .unwrap();
            let mut p: serde_json::Value = serde_json::from_str(&json).unwrap();
            match case {
                "cursor" => p["next"] = 10000.into(),
                "hash" => p["censusHash"] = "f".repeat(64).into(),
                "unknown" => p["deleteAllowed"] = true.into(),
                _ => p["phase"] = "completed".into(),
            }
            db.execute(
                "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
                params![op, p.to_string()],
            )
            .unwrap();
        });
        assert!(
            removal::run(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{case}"
        );
        assert!(f.path("cli").exists() && f.path("weights/model.bin").exists());
        assert!(f.storage.data_write_blocked());
    }
}

#[test]
fn model_deletion_body_process_sigkill_resumes_every_persisted_boundary_without_history_loss() {
    for boundary in [
        "armed",
        "renamed",
        "staged",
        "unlinked",
        "advanced",
        "beforeComplete",
        "completed",
    ] {
        let mut f = Fixture::new();
        let owner = lease();
        let op = prepare(&f, false, &owner);
        let before = history(&f);
        let consent = receipt(&f, &op);
        removal::start(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        drop(owner);
        let root = f.storage.root().to_path_buf();
        let placeholder = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut f.storage,
            Storage::open(placeholder.path().into()).unwrap(),
        );
        drop(old);
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","storage::deletion::models::tests::durable::removing::model_deletion_body_crash_child","--ignored","--nocapture"])
            .env("MON_MODEL_REMOVAL_TEST_ROOT",&root).env("MON_MODEL_REMOVAL_TEST_MODELS",&f.root).env("MON_MODEL_REMOVAL_TEST_OPERATION",&op).env("MON_MODEL_REMOVAL_TEST_BOUNDARY",boundary).output().unwrap();
        assert_eq!(
            output.status.signal(),
            Some(libc::SIGKILL),
            "{boundary}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        f.storage = Storage::open_with_models(root, f.root.clone()).unwrap();
        assert_eq!(before, history(&f));
        assert_eq!(consent, receipt(&f, &op));
        let controller = AnalysisController::default();
        let owner = controller
            .begin_model_deletion_maintenance(
                &f.storage,
                &op,
                false,
                false,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        assert!(
            removal::run(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .unwrap()
            .completed(),
            "{boundary}"
        );
        assert!(!f.path("").exists());
        assert_eq!(before, history(&f));
    }
}

#[test]
#[ignore = "child process only; synthetic parent fixture handoff required"]
fn model_deletion_body_crash_child() {
    let root = std::env::var_os("MON_MODEL_REMOVAL_TEST_ROOT").expect("synthetic root handoff");
    let op =
        std::env::var("MON_MODEL_REMOVAL_TEST_OPERATION").expect("synthetic operation handoff");
    let boundary =
        std::env::var("MON_MODEL_REMOVAL_TEST_BOUNDARY").expect("synthetic boundary handoff");
    let models =
        std::env::var_os("MON_MODEL_REMOVAL_TEST_MODELS").expect("synthetic native models root");
    let storage = Storage::open_with_models(std::path::PathBuf::from(root), models.into()).unwrap();
    let controller = AnalysisController::default();
    let owner = controller
        .begin_model_deletion_maintenance(
            &storage,
            &op,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    removal::run_observed(
        &storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
        &|name| {
            if name == boundary {
                unsafe {
                    libc::kill(std::process::id() as i32, libc::SIGKILL);
                }
            }
        },
    )
    .unwrap();
    panic!("synthetic crash boundary was not reached");
}

#[test]
fn model_deletion_body_full_256_object_limit_does_not_double_count_staged_missing_originals() {
    let f = Fixture::new();
    for n in 0..251 {
        std::fs::write(f.path(&format!("extra-{n:03}")), b"EXPLICIT REVIEW").unwrap();
    }
    assert_eq!(f.preview().unwrap().entries.len(), 256);
    let owner = lease();
    let op = prepare(&f, true, &owner);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    assert!(!f.path("").exists());
    assert!(!f.storage.data_write_blocked());
}
