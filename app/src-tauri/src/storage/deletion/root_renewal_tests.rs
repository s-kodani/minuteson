//! Synthetic storage and explicit shared admission; no user data or recording.
use super::*;

#[test]
fn root_renewal_native_names_and_header_bounds_reject_overflow_without_adopting_scope() {
    let operation = "11111111-1111-4111-8111-111111111111";
    for suffix in ["0", "199999", "r1-0", "r32-199999"] {
        assert!(crate::storage::deletion::files::valid_reservation_root(
            &format!("meetings/.mon-delete-{operation}-{suffix}")
        ));
    }
    for suffix in [
        "01",
        "200000",
        "r0-0",
        "r33-0",
        "r01-0",
        "1/child",
        "1/../neighbor",
        "-1",
        "999999999999999999999999999999999999",
    ] {
        assert!(!crate::storage::deletion::files::valid_reservation_root(
            &format!("meetings/.mon-delete-{operation}-{suffix}")
        ));
    }
    let f = Fixture::new();
    let r = f.begin(false);
    let mut bounded = r.manifest.clone();
    bounded.version = 2;
    bounded.reservation_roots = (0..256)
        .map(|n| format!("meetings/.mon-delete-{}-r1-{n}", f.operation))
        .collect();
    bounded.reservation_roots.sort();
    bounded.validate_roots().unwrap();
    assert!(r
        .progress
        .reconfirmation_roots(&bounded, &f.operation)
        .is_err());
    let mut invalid = bounded.clone();
    invalid
        .reservation_roots
        .push(format!("meetings/.mon-delete-{}-r2-0", f.operation));
    invalid.reservation_roots.sort();
    assert!(invalid.validate_roots().is_err());
    invalid = bounded.clone();
    invalid.reservation_roots.reverse();
    assert!(invalid.validate_roots().is_err());
    invalid = bounded.clone();
    invalid.reservation_roots[1] = invalid.reservation_roots[0].clone();
    assert!(invalid.validate_roots().is_err());
    invalid = bounded;
    invalid.version = 1;
    assert!(invalid.validate_roots().is_err());
    assert_eq!(audit_count(&f), 0);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}

pub(super) fn paused_root(f: &Fixture, point: &str) -> (Receipt, PathBuf) {
    let mut r = f.begin(false);
    let logical = f.storage.root().join(format!("meetings/{}", f.meeting));
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|event| {
        if event == point && !logical.exists() {
            stop.set(true);
        }
    })
    .is_err());
    assert!(stop.get());
    let path = f.storage.root().join("meetings").join(r.reserved());
    assert!(path.is_dir());
    (r, path)
}

#[test]
fn root_renewal_coexisting_current_tree_and_reserved_contents_require_fresh_consent_and_keep_original_evidence(
) {
    for point in ["renamed", "staged"] {
        for coexist in [false, true] {
            let f = Fixture::new();
            let (mut r, reserved) = paused_root(&f, point);
            let evidence = snapshot(&f);
            let old = r.progress_json().unwrap();
            std::fs::write(reserved.join("unknown.txt"), b"RESERVED UNKNOWN").unwrap();
            if coexist {
                std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
                std::fs::write(&f.file, b"NEW LOGICAL AUDIO").unwrap();
            }
            assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
            let inspected = review(&f, &r);
            assert_eq!(audit_count(&f), 0);
            assert_eq!(r.progress_json().unwrap(), old);
            assert!(inspected
                .manifest()
                .entries
                .iter()
                .any(|e| e.path.ends_with("/unknown.txt") && e.registered.is_none()));
            assert!(renewal::accept(
                &f.storage,
                &mut r,
                &inspected,
                false,
                &lease(),
                &crate::test_support::never_cancelled
            )
            .is_err());
            assert_eq!(
                std::fs::read(reserved.join("unknown.txt")).unwrap(),
                b"RESERVED UNKNOWN"
            );
            renewal::accept(
                &f.storage,
                &mut r,
                &inspected,
                true,
                &lease(),
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            assert_eq!(snapshot(&f), evidence);
            assert_eq!(audit_count(&f), 1);
            assert_eq!(r.manifest.reservation_roots.len(), 1);
            finish(&f, &mut r);
            assert!(!reserved.exists());
            assert_eq!(snapshot(&f), evidence);
        }
    }
}

#[test]
fn root_renewal_native_cold_review_lists_exact_siblings_and_preserves_restart_fence() {
    let f = Fixture::new();
    let (r, reserved) = paused_root(&f, "staged");
    std::fs::write(reserved.join("unknown"), b"COLD RESERVED").unwrap();
    let path = f.storage.root().to_path_buf();
    let operation = f.operation.clone();
    drop(r);
    drop(f.storage);
    let storage = Storage::open(path).unwrap();
    assert!(storage.data_write_blocked());
    let analysis = AnalysisController::default();
    let admitted = analysis
        .begin_deletion_maintenance(
            &storage,
            &operation,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let mut p =
        operations::Prepared::pending(&storage, &operation, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(!p.preview().execute_allowed && p.preview().reconfirmation_allowed);
    p.review_remaining(
        &storage,
        &admitted,
        &p.preview().confirmation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let preview = p.preview();
    assert_eq!(preview.phase, "reconfirmation");
    assert_eq!(preview.unregistered_count, 1);
    assert_eq!(
        preview.reservation_roots,
        vec![reserved
            .strip_prefix(storage.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()]
    );
    let page = p.page(&preview.confirmation_id, 0).unwrap();
    assert!(page
        .entries
        .iter()
        .any(|e| e.relative_path.ends_with("/unknown") && !e.registered));
    assert!(page
        .entries
        .iter()
        .all(|e| !e.relative_path.contains(&f.other)));
    let done = p
        .execute(
            &storage,
            &admitted,
            &preview.confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted && done.restart_required);
    assert!(storage.require_data_writable().is_err());
    assert!(!reserved.exists());
    assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
}

#[test]
fn root_renewal_precommit_cancellation_retains_both_trees_and_old_progress_atomically() {
    for point in [
        "reviewIntentStored",
        "reviewPathStored",
        "reviewRecorded",
        "beforeReviewCommit",
    ] {
        let f = Fixture::new();
        let (mut r, reserved) = paused_root(&f, "renamed");
        std::fs::write(reserved.join("unknown"), b"KEEP RESERVED").unwrap();
        std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
        std::fs::write(&f.file, b"KEEP LOGICAL").unwrap();
        let v = review(&f, &r);
        let old = r.progress_json().unwrap();
        let stop = Cell::new(false);
        assert!(renewal::accept_observed(
            &f.storage,
            &mut r,
            &v,
            true,
            &lease(),
            &|| stop.get(),
            &|event| {
                if event == point {
                    stop.set(true);
                }
            }
        )
        .is_err());
        assert!(stop.get());
        assert_eq!(audit_count(&f), 0);
        assert_eq!(r.progress_json().unwrap(), old);
        assert_eq!(
            std::fs::read(reserved.join("unknown")).unwrap(),
            b"KEEP RESERVED"
        );
        assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP LOGICAL");
        assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
        f.unaffected();
    }
}

#[test]
fn root_renewal_committed_plan_cold_reopens_without_forgetting_original_reserved_root() {
    let f = Fixture::new();
    let (mut r, reserved) = paused_root(&f, "staged");
    std::fs::write(reserved.join("unknown"), b"PERSIST RESERVED").unwrap();
    accept(&f, &mut r, true);
    let json = r.progress_json().unwrap();
    let roots = r.manifest.reservation_roots.clone();
    let path = f.storage.root().to_path_buf();
    let operation = f.operation.clone();
    drop(r);
    drop(f.storage);
    let storage = Storage::open(path).unwrap();
    let mut saved = read(&storage, &operation, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(saved.progress_json().unwrap(), json);
    assert_eq!(saved.manifest.reservation_roots, roots);
    assert!(storage.data_write_blocked());
    run(&storage, &mut saved, &crate::test_support::never_cancelled).unwrap();
    let mut db =
        completion::start(&storage, &saved, &crate::test_support::never_cancelled).unwrap();
    completion::run(&storage, &mut db, &crate::test_support::never_cancelled).unwrap();
    assert!(db.completed() && storage.restart_required());
    assert!(!reserved.exists());
    assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
}

#[test]
fn root_renewal_second_generation_handles_another_reserved_rename_without_expanding_to_neighbor() {
    let f = Fixture::new();
    let (mut r, first) = paused_root(&f, "renamed");
    std::fs::write(first.join("unknown"), b"FIRST UNKNOWN").unwrap();
    accept(&f, &mut r, true);
    let index = r
        .manifest
        .entries
        .iter()
        .rev()
        .position(|e| {
            e.path
                == first
                    .strip_prefix(f.storage.root())
                    .unwrap()
                    .to_str()
                    .unwrap()
        })
        .unwrap();
    let second = f
        .storage
        .root()
        .join("meetings")
        .join(r.progress.reserved(&f.operation, index));
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|event| {
        if event == "renamed" && second.exists() {
            std::fs::write(second.join("new-unknown"), b"SECOND UNKNOWN").unwrap();
            stop.set(true);
        }
    })
    .is_err());
    assert!(stop.get());
    assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    accept(&f, &mut r, true);
    assert!(r.manifest.reservation_roots.contains(
        &first
            .strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    ));
    assert!(r.manifest.reservation_roots.contains(
        &second
            .strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    ));
    assert_eq!(audit_count(&f), 2);
    finish(&f, &mut r);
    assert!(!first.exists() && !second.exists());
}

#[test]
fn current_v3_renewal_chain_retains_all_reserved_names_and_requires_fresh_consent_on_reappearance()
{
    let f = Fixture::new();
    let (mut r, reserved) = paused_root(&f, "staged");
    let original = snapshot(&f);
    std::fs::remove_dir(&reserved).unwrap();
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"CURRENT LOGICAL AUDIO").unwrap();
    accept(&f, &mut r, true);
    let first_roots = r.manifest.reservation_roots.clone();
    let relative = reserved
        .strip_prefix(f.storage.root())
        .unwrap()
        .to_str()
        .unwrap();
    assert!(first_roots.iter().any(|p| p == relative));
    std::fs::write(f.file.parent().unwrap().join("extra"), b"SECOND SCOPE").unwrap();
    accept(&f, &mut r, true);
    assert!(first_roots
        .iter()
        .all(|p| r.manifest.reservation_roots.contains(p)));
    assert_eq!(r.progress.confirmation.as_ref().unwrap().generation, 2);
    let db = f.storage.connection.lock().unwrap();
    let versions: Vec<u32> = db.prepare("SELECT json_extract(target_manifest_json,'$.version') FROM maintenance_operations WHERE kind='delete_meeting' AND json_extract(progress_json,'$.confirmationRecorded')=1 ORDER BY id")
        .unwrap().query_map([], |row| row.get(0))
        .unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(versions, vec![3, 3]);
    drop(db);
    std::fs::create_dir(&reserved).unwrap();
    std::fs::write(reserved.join("late"), b"CURRENT REAPPEARED UNKNOWN").unwrap();
    let progress = r.progress_json().unwrap();
    assert!(run(&f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    assert_eq!(r.progress_json().unwrap(), progress);
    assert_eq!(
        std::fs::read(reserved.join("late")).unwrap(),
        b"CURRENT REAPPEARED UNKNOWN"
    );
    accept(&f, &mut r, true);
    f.unaffected();
    finish(&f, &mut r);
    assert!(!reserved.exists());
    assert_eq!(snapshot(&f), original);
}

#[test]
fn root_renewal_exact_sibling_links_or_files_are_retained_without_following_external_data() {
    for link in [false, true] {
        let f = Fixture::new();
        let (r, reserved) = paused_root(&f, "renamed");
        std::fs::remove_dir(&reserved).unwrap();
        if link {
            std::os::unix::fs::symlink(f.base.path().join("models"), &reserved).unwrap();
        } else {
            std::fs::write(&reserved, b"KEEP FOREIGN FILE").unwrap();
        }
        assert!(
            renewal::Review::capture(&f.storage, &r, &crate::test_support::never_cancelled)
                .is_err()
        );
        assert_eq!(audit_count(&f), 0);
        assert!(std::fs::symlink_metadata(&reserved).is_ok());
        f.unaffected();
    }
}

#[test]
fn root_renewal_late_historical_root_reappearance_blocks_fixed_plan_and_requires_another_review() {
    let f = Fixture::new();
    let (mut r, reserved) = paused_root(&f, "renamed");
    std::fs::write(reserved.join("unknown"), b"DELETE FIRST").unwrap();
    accept(&f, &mut r, true);
    run(&f.storage, &mut r, &crate::test_support::never_cancelled).unwrap();
    assert!(!reserved.exists());
    std::fs::create_dir(&reserved).unwrap();
    std::fs::write(reserved.join("late"), b"KEEP UNTIL ANOTHER CONSENT").unwrap();
    assert!(completion::start(&f.storage, &r, &crate::test_support::never_cancelled).is_err());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(
        std::fs::read(reserved.join("late")).unwrap(),
        b"KEEP UNTIL ANOTHER CONSENT"
    );
    accept(&f, &mut r, true);
    assert_eq!(audit_count(&f), 2);
    finish(&f, &mut r);
    assert!(!reserved.exists());
}
