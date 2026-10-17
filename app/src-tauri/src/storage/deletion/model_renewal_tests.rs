//! Reconfirmation uses synthetic model files, isolated SQLite and actual leases.
use super::*;
use removal::renewal;

fn progress(f: &Fixture, op: &str) -> String {
    f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT progress_json FROM maintenance_operations WHERE id=?1",
            params![op],
            |r| r.get(0),
        )
        .unwrap()
}
fn audits(f: &Fixture) -> Vec<(String, String, String, i64)> {
    let db = f.storage.connection.lock().unwrap();
    let rows = db.prepare("SELECT id,target_manifest_json,target_hash,consent_at FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='modelDeletionReconfirmation' ORDER BY id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    rows
}
fn pause(f: &Fixture, op: &str, owner: &DataMaintenanceLease, boundary: &str) {
    let stop = Cell::new(false);
    assert!(removal::run_observed(
        &f.storage,
        op,
        owner,
        &|| stop.get(),
        &|name| if name == boundary {
            stop.set(true)
        }
    )
    .is_err());
    assert!(f.storage.data_write_blocked());
}
#[test]
fn model_deletion_renewal_preserves_original_and_partial_history_requires_unknown_consent_and_replays(
) {
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    let fixed = receipt(&f, &op);
    let before = history(&f);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    pause(&f, &op, &owner, "advanced");
    assert!(!f.path("weights/model.bin").exists());
    std::fs::write(f.path("cli"), b"CHANGED BUT NOW REVIEWED").unwrap();
    std::fs::write(f.path("NEW"), b"ONLY NEW CONSENT MAY REMOVE").unwrap();
    let old = progress(&f, &op);
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(view
        .entries()
        .iter()
        .any(|e| e.changed && e.relative_path.ends_with("/cli")));
    assert!(view
        .entries()
        .iter()
        .any(|e| !e.registered && e.relative_path.ends_with("/NEW")));
    assert_eq!(progress(&f, &op), old);
    assert!(audits(&f).is_empty());
    assert!(renewal::accept(
        &f.storage,
        &view,
        "wrong",
        true,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(progress(&f, &op), old);
    assert!(audits(&f).is_empty());
    let accepted = renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
        false
    })
    .unwrap();
    let saved_audits = audits(&f);
    assert_eq!(saved_audits.len(), 1);
    assert_eq!(
        accepted,
        renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
            false
        })
        .unwrap()
    );
    assert!(renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    assert!(
        renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
            false
        })
        .unwrap()
        .completed()
    );
    assert_eq!(audits(&f), saved_audits);
    assert_eq!(receipt(&f, &op), fixed);
    assert_eq!(history(&f), before);
    assert!(!f.path("").exists());
    assert!(!f.storage.data_write_blocked());
}
#[test]
fn model_deletion_renewal_exact_staged_root_and_reappeared_logical_root_never_enumerate_siblings() {
    for logical in [false, true] {
        let f = Fixture::new();
        let owner = lease();
        let op = prepare(&f, false, &owner);
        let fixed = receipt(&f, &op);
        let before = history(&f);
        removal::start(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let staged = f.root.join(format!(".mon-delete-model-{op}-4"));
        let touched = Cell::new(false);
        assert!(removal::run_observed(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
            &|event| {
                if event == "beforeUnlink"
                    && !f.path("").exists()
                    && staged.exists()
                    && !touched.replace(true)
                {
                    std::fs::write(staged.join("NEW-IN-STAGE"), b"EXPLICIT NEW SCOPE").unwrap();
                }
            }
        )
        .is_err());
        assert!(touched.get());
        if logical {
            std::fs::create_dir(f.path("")).unwrap();
            std::fs::write(f.path("NEW-IN-LOGICAL"), b"NEW LOGICAL").unwrap();
        }
        let sibling = f.root.join("UNRELATED-MODEL");
        std::fs::create_dir(&sibling).unwrap();
        std::os::unix::fs::symlink("/not-readable", sibling.join("KEEP")).unwrap();
        let view = renewal::review(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(view
            .entries()
            .iter()
            .any(|e| e.relative_path.ends_with("/NEW-IN-STAGE")));
        assert_eq!(
            view.entries()
                .iter()
                .any(|e| e.relative_path.ends_with("/NEW-IN-LOGICAL")),
            logical
        );
        assert!(renewal::accept(
            &f.storage,
            &view,
            view.census_hash(),
            false,
            &owner,
            &crate::test_support::never_cancelled
        )
        .is_err());
        renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
            false
        })
        .unwrap();
        assert!(removal::run(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .completed());
        assert!(!f.path("").exists() && !staged.exists());
        assert!(sibling
            .join("KEEP")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(receipt(&f, &op), fixed);
        assert_eq!(history(&f), before);
    }
}
#[test]
fn model_deletion_renewal_completed_source_preserves_completed_state_and_requires_cold_restart() {
    let mut f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    let fixed = receipt(&f, &op);
    let before = history(&f);
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
    std::fs::create_dir(f.path("weights")).unwrap();
    std::fs::write(
        f.path("weights/model.bin"),
        b"REAPPEARED MODEL; NEW CONSENT",
    )
    .unwrap();
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(f.storage.data_write_blocked());
    assert!(f.storage.list_model_placements().unwrap()[0].body_removed);
    let state: String = f
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            params![op],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "completed");
    drop(owner);
    reopen(&mut f, |_| {});
    assert!(f.storage.data_write_blocked());
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
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    assert!(f
        .storage
        .require_data_writable()
        .unwrap_err()
        .contains("再起動"));
    assert_eq!(receipt(&f, &op), fixed);
    assert_eq!(history(&f), before);
    drop(owner);
    reopen(&mut f, |_| {});
    assert!(!f.storage.data_write_blocked());
}
#[test]
fn model_deletion_renewal_rejects_changes_after_review_and_retains_all_evidence() {
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
    pause(&f, &op, &owner, "advanced");
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let old = progress(&f, &op);
    std::fs::write(f.path("cli"), b"CHANGED AFTER VIEW").unwrap();
    assert!(
        renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
            false
        })
        .is_err()
    );
    assert!(audits(&f).is_empty());
    assert_eq!(progress(&f, &op), old);
    assert!(f.path("cli").exists());
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let saved = audits(&f);
    std::fs::write(f.path("NEW"), b"NOT IN SAVED RECONFIRMATION").unwrap();
    assert!(renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(audits(&f), saved);
    assert!(f.path("cli").exists() && f.path("NEW").exists());
}
#[test]
fn model_deletion_renewal_cancel_rolls_back_all_audit_proofs_and_cursor_at_each_boundary() {
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
    pause(&f, &op, &owner, "advanced");
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let old = progress(&f, &op);
    let evidence = f.evidence();
    for boundary in [
        "auditStored",
        "proofStored",
        "auditCompleted",
        "beforeCommit",
    ] {
        let stop = Cell::new(false);
        assert!(
            renewal::accept_observed(&f.storage, &view, false, &owner, &|| stop.get(), &|event| {
                if event == boundary {
                    stop.set(true)
                }
            })
            .is_err(),
            "{boundary}"
        );
        assert!(stop.get(), "{boundary}");
        assert!(audits(&f).is_empty());
        assert_eq!(old, progress(&f, &op));
        assert_eq!(evidence, f.evidence());
        assert!(f.storage.data_write_blocked());
    }
    renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
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
}
#[test]
fn model_deletion_renewal_multiple_generations_and_partial_resume_keep_previous_audits() {
    let mut f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    let fixed = receipt(&f, &op);
    let before = history(&f);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    pause(&f, &op, &owner, "renamed");
    let a = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    renewal::accept(
        &f.storage,
        &a,
        a.census_hash(),
        true,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let first = audits(&f);
    pause(&f, &op, &owner, "advanced");
    std::fs::write(f.path("NEW-GENERATION"), b"SECOND EXPLICIT CONSENT").unwrap();
    let b = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    renewal::accept(
        &f.storage,
        &b,
        b.census_hash(),
        true,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(audits(&f).len(), 2);
    assert!(audits(&f).contains(&first[0]));
    assert!(renewal::accept(
        &f.storage,
        &a,
        a.census_hash(),
        true,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    pause(&f, &op, &owner, "unlinked");
    drop(owner);
    reopen(&mut f, |_| {});
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
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    assert_eq!(receipt(&f, &op), fixed);
    assert_eq!(history(&f), before);
    assert!(audits(&f).contains(&first[0]));
}
fn replace_trigger(db: &Connection, name: &str, mutate: impl FnOnce(&Connection)) {
    let sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            params![name],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
    mutate(db);
    db.execute_batch(&sql).unwrap();
}
#[test]
fn model_deletion_renewal_corrupt_chain_and_proof_are_never_adopted_or_executed() {
    for case in 0..5 {
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
        pause(&f, &op, &owner, "advanced");
        let view = renewal::review(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        renewal::accept(
            &f.storage,
            &view,
            view.census_hash(),
            false,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let id = audits(&f)[0].0.clone();
        reopen(&mut f, |db| {
            if case < 3 {
                let p: String = db
                    .query_row::<String, _, _>(
                        "SELECT progress_json FROM maintenance_operations WHERE id=?1",
                        params![op],
                        |r| r.get(0),
                    )
                    .unwrap();
                let next = match case {
                    0 => p.replace("\"generation\":1", "\"generation\":2"),
                    1 => {
                        let value: serde_json::Value = serde_json::from_str(&p).unwrap();
                        p.replace(
                            value["confirmation"]["intentHash"].as_str().unwrap(),
                            &"0".repeat(64),
                        )
                    }
                    _ => format!("{},\"previouslyCompleted\":true}}", &p[..p.len() - 1]),
                };
                db.execute(
                    "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
                    params![op, next],
                )
                .unwrap();
            } else if case == 3 {
                replace_trigger(db, "maintenance_path_fixed", |db| {
                    db.execute(
                        "UPDATE maintenance_paths SET identity_hash=?2 WHERE operation_id=?1",
                        params![id, "0".repeat(64)],
                    )
                    .unwrap();
                });
            } else {
                replace_trigger(db, "maintenance_path_retained", |db| {
                    db.execute("DELETE FROM maintenance_paths WHERE operation_id=?1 AND relative_path='model-deletion-proof/000000'",params![id]).unwrap();
                });
            }
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
        assert!(
            renewal::review(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{case}"
        );
        assert!(f.path("cli").exists());
        assert!(f.storage.data_write_blocked());
    }
}
#[test]
fn model_deletion_renewal_links_root_replacement_and_reservation_collision_preserve_targets() {
    for case in 0..3 {
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
        pause(&f, &op, &owner, "advanced");
        let old = progress(&f, &op);
        if case == 0 {
            let held = f.root.parent().unwrap().join("held-cli");
            std::fs::rename(f.path("cli"), &held).unwrap();
            std::os::unix::fs::symlink(&held, f.path("cli")).unwrap();
            assert!(renewal::review(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err());
            assert!(held.exists());
        } else if case == 1 {
            let held = f.root.parent().unwrap().join("held-models");
            std::fs::rename(&f.root, &held).unwrap();
            std::fs::create_dir(&f.root).unwrap();
            assert!(renewal::review(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err());
            assert!(held.join(&f.package).exists());
        } else {
            let view = renewal::review(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            // The renewed tree has four objects, with the logical root last.
            let collision = f.root.join(format!(".mon-delete-model-{op}-g1-3"));
            std::fs::create_dir(&collision).unwrap();
            std::fs::write(collision.join("KEEP"), b"UNREVIEWED COLLISION").unwrap();
            assert!(renewal::accept(
                &f.storage,
                &view,
                view.census_hash(),
                false,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err());
            assert_eq!(
                std::fs::read(collision.join("KEEP")).unwrap(),
                b"UNREVIEWED COLLISION"
            );
        }
        assert_eq!(progress(&f, &op), old);
        assert!(audits(&f).is_empty());
    }
}

#[test]
fn model_deletion_renewal_sigkill_at_audit_boundaries_is_atomic_and_preserves_original() {
    for completed in [false, true] {
        for boundary in [
            "auditStored",
            "proofStored",
            "auditCompleted",
            "beforeCommit",
            "committed",
        ] {
            let mut f = Fixture::new();
            let owner = lease();
            let op = prepare(&f, false, &owner);
            let fixed = receipt(&f, &op);
            let before = history(&f);
            removal::start(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            if completed {
                removal::run(
                    &f.storage,
                    &op,
                    &owner,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
                std::fs::create_dir(f.path("")).unwrap();
            } else {
                pause(&f, &op, &owner, "advanced");
            }
            std::fs::write(f.path("NEW-AUDIT"), b"EXPLICIT NEW CONSENT").unwrap();
            let old = progress(&f, &op);
            drop(owner);
            let root = f.storage.root().to_path_buf();
            let placeholder = tempfile::tempdir().unwrap();
            let original = std::mem::replace(
                &mut f.storage,
                Storage::open(placeholder.path().into()).unwrap(),
            );
            drop(original);
            let output=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","storage::deletion::models::tests::durable::removing::renewing::model_deletion_renewal_crash_child","--ignored","--nocapture"])
                .env("MON_MODEL_RENEWAL_TEST_ROOT",&root).env("MON_MODEL_RENEWAL_TEST_MODELS",&f.root).env("MON_MODEL_RENEWAL_TEST_OPERATION",&op).env("MON_MODEL_RENEWAL_TEST_BOUNDARY",boundary).output().unwrap();
            assert_eq!(
                output.status.signal(),
                Some(libc::SIGKILL),
                "{completed}/{boundary}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            f.storage = Storage::open_with_models(root, f.root.clone()).unwrap();
            assert_eq!(receipt(&f, &op), fixed);
            assert_eq!(history(&f), before);
            assert_eq!(audits(&f).len(), usize::from(boundary == "committed"));
            assert!(f.path("NEW-AUDIT").exists());
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
            if boundary != "committed" {
                assert_eq!(progress(&f, &op), old);
                let view = renewal::review(
                    &f.storage,
                    &op,
                    &owner,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
                renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
                    false
                })
                .unwrap();
            } else {
                assert_ne!(progress(&f, &op), old);
                assert!(f.storage.data_write_blocked());
            }
            assert!(removal::run(
                &f.storage,
                &op,
                &owner,
                &crate::test_support::never_cancelled
            )
            .unwrap()
            .completed());
            assert!(!f.path("").exists());
            assert_eq!(audits(&f).len(), 1);
            assert_eq!(receipt(&f, &op), fixed);
            assert_eq!(history(&f), before);
        }
    }
}
#[test]
#[ignore = "child process only; synthetic renewal fixture handoff required"]
fn model_deletion_renewal_crash_child() {
    let root = std::env::var_os("MON_MODEL_RENEWAL_TEST_ROOT").expect("synthetic root handoff");
    let op =
        std::env::var("MON_MODEL_RENEWAL_TEST_OPERATION").expect("synthetic operation handoff");
    let boundary =
        std::env::var("MON_MODEL_RENEWAL_TEST_BOUNDARY").expect("synthetic boundary handoff");
    let models =
        std::env::var_os("MON_MODEL_RENEWAL_TEST_MODELS").expect("synthetic native models root");
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
    let view =
        renewal::review(&storage, &op, &owner, &crate::test_support::never_cancelled).unwrap();
    renewal::accept_observed(
        &storage,
        &view,
        true,
        &owner,
        &crate::test_support::never_cancelled,
        &|event| {
            if event == boundary {
                unsafe {
                    libc::kill(std::process::id() as i32, libc::SIGKILL);
                }
            }
        },
    )
    .unwrap();
    panic!("synthetic renewal crash boundary was not reached");
}
#[test]
fn model_deletion_renewal_sigkill_resumes_reserved_files_from_saved_new_scope() {
    for boundary in [
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
        let fixed = receipt(&f, &op);
        let before = history(&f);
        removal::start(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        pause(&f, &op, &owner, "renamed");
        let view = renewal::review(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        renewal::accept(&f.storage, &view, view.census_hash(), true, &owner, &|| {
            false
        })
        .unwrap();
        let saved_audits = audits(&f);
        drop(owner);
        let root = f.storage.root().to_path_buf();
        let placeholder = tempfile::tempdir().unwrap();
        let original = std::mem::replace(
            &mut f.storage,
            Storage::open(placeholder.path().into()).unwrap(),
        );
        drop(original);
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
        assert!(removal::run(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .completed());
        assert!(!f.path("").exists());
        assert_eq!(saved_audits, audits(&f));
        assert_eq!(fixed, receipt(&f, &op));
        assert_eq!(before, history(&f));
    }
}
#[test]
fn model_deletion_renewal_generation_bound_is_enforced_without_replacing_evidence() {
    let f = Fixture::new();
    let owner = lease();
    let op = prepare(&f, false, &owner);
    let fixed = receipt(&f, &op);
    removal::start(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    for _ in 0..32 {
        let view = renewal::review(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        renewal::accept(
            &f.storage,
            &view,
            view.census_hash(),
            false,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    }
    let old = progress(&f, &op);
    let saved = audits(&f);
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(saved.len(), 32);
    assert_eq!(saved, audits(&f));
    assert_eq!(old, progress(&f, &op));
    assert_eq!(fixed, receipt(&f, &op));
    assert!(f.path("cli").exists());
}
#[test]
fn model_deletion_renewal_rehashed_foreign_scope_is_not_a_deletion_authority() {
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
    pause(&f, &op, &owner, "advanced");
    let view = renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    renewal::accept(
        &f.storage,
        &view,
        view.census_hash(),
        false,
        &owner,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let foreign = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir(f.root.join(&foreign)).unwrap();
    std::fs::write(f.root.join(&foreign).join("KEEP"), b"FOREIGN MODEL").unwrap();
    let (id, header, sha, _) = audits(&f)[0].clone();
    let package = f.package.clone();
    reopen(&mut f, |db| {
        let raw:Vec<String>=db.prepare("SELECT identity_json FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path").unwrap().query_map(params![id],|r|r.get(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        let mut bytes = Vec::new();
        for json in &raw {
            let chunk: serde_json::Value = serde_json::from_str(json).unwrap();
            for pair in chunk["data"].as_str().unwrap().as_bytes().chunks_exact(2) {
                bytes.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
            }
        }
        let old_hash = format!("{:x}", Sha256::digest(&bytes));
        let plan = String::from_utf8(bytes)
            .unwrap()
            .replacen(&package, &foreign, 1);
        let new_hash = format!("{:x}", Sha256::digest(plan.as_bytes()));
        let next_header = header.replace(&old_hash, &new_hash);
        let next_sha = format!("{:x}", Sha256::digest(next_header.as_bytes()));
        replace_trigger(db, "maintenance_path_fixed", |db| {
            for (index, part) in plan.as_bytes().chunks(384).enumerate() {
                let data: String = part.iter().map(|b| format!("{b:02x}")).collect();
                let json = format!(r#"{{"version":1,"data":"{data}"}}"#);
                db.execute("UPDATE maintenance_paths SET content_hash=?3,identity_json=?4,identity_hash=?5 WHERE operation_id=?1 AND relative_path=?2",params![id,format!("model-deletion-proof/{index:06}"),format!("{:x}",Sha256::digest(part)),json,format!("{:x}",Sha256::digest(json.as_bytes()))]).unwrap();
            }
        });
        replace_trigger(db, "maintenance_operation_fixed", |db| {
            db.execute("UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",params![id,next_header,next_sha]).unwrap();
        });
        let p: String = db
            .query_row(
                "SELECT progress_json FROM maintenance_operations WHERE id=?1",
                params![op],
                |r| r.get(0),
            )
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![op, p.replace(&sha, &next_sha)],
        )
        .unwrap();
    });
    assert!(removal::run(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(renewal::review(
        &f.storage,
        &op,
        &owner,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f.path("cli").exists());
    assert_eq!(
        std::fs::read(f.root.join(foreign).join("KEEP")).unwrap(),
        b"FOREIGN MODEL"
    );
}

#[test]
fn model_deletion_renewal_completed_cancel_and_cancelled_lease_do_not_leave_a_phantom_fence() {
    for boundary in [
        "auditStored",
        "proofStored",
        "auditCompleted",
        "beforeCommit",
        "lease",
    ] {
        let f = Fixture::new();
        let controller = AnalysisController::default();
        let mut owner = controller
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        let op = prepare(&f, false, &owner);
        let fixed = receipt(&f, &op);
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
        std::fs::write(f.path("cli"), b"NEW EXPLICIT REGISTERED CONTENT").unwrap();
        let old = progress(&f, &op);
        let view = renewal::review(
            &f.storage,
            &op,
            &owner,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let stop = Cell::new(false);
        assert!(
            renewal::accept_observed(&f.storage, &view, false, &owner, &|| stop.get(), &|event| {
                if boundary == "lease" && event == "proofStored" {
                    controller.begin_exit().unwrap();
                    controller.abort_shutdown();
                } else if event == boundary {
                    stop.set(true);
                }
            })
            .is_err(),
            "{boundary}"
        );
        assert!(!f.storage.data_write_blocked(), "{boundary}");
        assert!(audits(&f).is_empty());
        assert_eq!(old, progress(&f, &op));
        assert_eq!(fixed, receipt(&f, &op));
        assert!(f.path("cli").exists());
        assert!(f.storage.list_model_placements().unwrap()[0].body_removed);
        if boundary == "lease" {
            assert!(renewal::accept(
                &f.storage,
                &view,
                view.census_hash(),
                false,
                &owner,
                &crate::test_support::never_cancelled
            )
            .is_err());
            drop(owner);
            owner = controller
                .begin_model_deletion_maintenance(
                    &f.storage,
                    &op,
                    false,
                    false,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
        }
        renewal::accept(
            &f.storage,
            &view,
            view.census_hash(),
            false,
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
        assert_eq!(fixed, receipt(&f, &op));
    }
}
