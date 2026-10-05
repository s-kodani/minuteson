//! Canonical old audit generations, synthetic files and real native admission.
use super::root_renewal_tests::paused_root;
use super::*;

fn legacy(f: &Fixture) -> (Receipt, PathBuf) {
    let (r, old) = paused_root(f, "staged");
    // Emulate the old reservation having disappeared before the scope switch.
    std::fs::remove_dir(&old).unwrap();
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"LEGACY LOGICAL AUDIO").unwrap();
    std::fs::write(f.file.parent().unwrap().join("extra"), b"LEGACY EXTRA").unwrap();
    let r = renewal::seed_historical_record_for_test(&f.storage, &r, 1, &[]).unwrap();
    assert!(r.manifest.reservation_roots.is_empty());
    (r, old)
}
fn roots(f: &Fixture, r: &Receipt) -> Vec<String> {
    let db = f.storage.connection.lock().unwrap();
    renewal::reconfirmation_roots(&db, r.confirmed(), &r.progress, &|| false).unwrap()
}
fn reappear(path: &PathBuf) {
    std::fs::create_dir(path).unwrap();
    std::fs::write(path.join("late"), b"OLD RESERVED UNKNOWN").unwrap();
}
fn relative(f: &Fixture, path: &PathBuf) -> String {
    path.strip_prefix(f.storage.root())
        .unwrap()
        .to_str()
        .unwrap()
        .into()
}

#[test]
fn historical_renewal_v1_generations_recover_all_names_before_unlink_and_require_new_consent() {
    let f = Fixture::new();
    let (mut r, old) = legacy(&f);
    let fixed = snapshot(&f);
    let first_roots = roots(&f, &r);
    std::fs::write(f.file.parent().unwrap().join("second"), b"SECOND SCOPE").unwrap();
    r = renewal::seed_historical_record_for_test(&f.storage, &r, 1, &[]).unwrap();
    let all = roots(&f, &r);
    assert!(first_roots.iter().all(|p| all.contains(p)));
    assert!(all.contains(&relative(&f, &old)));
    assert_eq!(all.len(), 3);
    reappear(&old);
    let progress = r.progress_json().unwrap();
    assert!(run(&f.storage, &mut r, &|| false).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"LEGACY LOGICAL AUDIO");
    assert_eq!(r.progress_json().unwrap(), progress);
    let v = review(&f, &r);
    assert_eq!(v.manifest().reservation_roots, all);
    assert_eq!(audit_count(&f), 2);
    assert!(renewal::accept(&f.storage, &mut r, &v, false, &lease(), &|| false).is_err());
    assert_eq!(
        std::fs::read(old.join("late")).unwrap(),
        b"OLD RESERVED UNKNOWN"
    );
    renewal::accept(&f.storage, &mut r, &v, true, &lease(), &|| false).unwrap();
    assert_eq!(r.manifest.reservation_roots, all);
    assert_eq!(snapshot(&f), fixed);
    let db = f.storage.connection.lock().unwrap();
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![r.progress.confirmation.as_ref().unwrap().operation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["version"],
        3
    );
    drop(db);
    finish(&f, &mut r);
    assert!(!old.exists());
    assert_eq!(snapshot(&f), fixed);
}

#[test]
fn historical_renewal_v2_immediate_roots_remain_readable_without_forgetting_earlier_v1_names() {
    let f = Fixture::new();
    let (mut r, old) = legacy(&f);
    let immediate = r
        .progress
        .reconfirmation_roots(&r.manifest, &f.operation)
        .unwrap();
    assert!(!immediate.contains(&relative(&f, &old)));
    r = renewal::seed_historical_record_for_test(&f.storage, &r, 2, &immediate).unwrap();
    assert_eq!(r.manifest.reservation_roots, immediate);
    assert!(roots(&f, &r).contains(&relative(&f, &old)));
    reappear(&old);
    assert!(run(&f.storage, &mut r, &|| false).is_err());
    accept(&f, &mut r, true);
    assert!(r.manifest.reservation_roots.contains(&relative(&f, &old)));
    finish(&f, &mut r);
    assert!(!old.exists());
}

#[test]
fn historical_renewal_cold_native_review_preserves_legacy_scope_and_restart_fence() {
    let f = Fixture::new();
    let (r, old) = legacy(&f);
    let expected = roots(&f, &r);
    let root = f.storage.root().to_path_buf();
    let operation = f.operation.clone();
    let other_file = f.other_file.clone();
    reappear(&old);
    drop(r);
    drop(f.storage);
    let storage = Storage::open(root).unwrap();
    assert!(storage.data_write_blocked());
    let analysis = AnalysisController::default();
    let admission = analysis
        .begin_deletion_maintenance(&storage, &operation, false, false, &|| false)
        .unwrap();
    let mut p = operations::Prepared::pending(&storage, &operation, &|| false).unwrap();
    assert!(!p.preview().execute_allowed && p.preview().reconfirmation_allowed);
    let before = p.preview().confirmation_id;
    p.review_remaining(&storage, &admission, &before, &|| false)
        .unwrap();
    let preview = p.preview();
    assert_ne!(preview.confirmation_id, before);
    assert_eq!(preview.reservation_roots, expected);
    assert!(p
        .page(&preview.confirmation_id, 0)
        .unwrap()
        .entries
        .iter()
        .any(|e| e.relative_path.ends_with("/late") && !e.registered));
    let done = p
        .execute(
            &storage,
            &admission,
            &preview.confirmation_id,
            true,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted && done.restart_required);
    assert!(!old.exists());
    assert!(storage.require_data_writable().is_err());
    assert_eq!(std::fs::read(other_file).unwrap(), b"KEEP OTHER AUDIO");
}

#[test]
fn historical_renewal_v3_omission_or_extra_roots_roll_back_even_with_canonical_hashes() {
    for change in ["omit", "extra", "foreign", "unsupported"] {
        let f = Fixture::new();
        let (r, old) = legacy(&f);
        let mut proposed = roots(&f, &r);
        let version = match change {
            "omit" => {
                proposed.retain(|p| p != &relative(&f, &old));
                3
            }
            "extra" => {
                proposed.push(format!("meetings/.mon-delete-{}-r32-199999", f.operation));
                3
            }
            "foreign" => {
                proposed.push(format!("meetings/.mon-delete-{}-0", f.other));
                3
            }
            _ => 4,
        };
        proposed.sort();
        let previous = r.progress_json().unwrap();
        let fixed = snapshot(&f);
        assert!(
            renewal::seed_historical_record_for_test(&f.storage, &r, version, &proposed).is_err(),
            "{change}"
        );
        assert_eq!(
            read(&f.storage, &f.operation, &|| false)
                .unwrap()
                .progress_json()
                .unwrap(),
            previous
        );
        assert_eq!(audit_count(&f), 1);
        assert_eq!(snapshot(&f), fixed);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"LEGACY LOGICAL AUDIO");
        f.unaffected();
    }
}

#[test]
fn historical_renewal_old_exact_sibling_links_and_files_are_not_adopted_or_followed() {
    for link in [false, true] {
        let f = Fixture::new();
        let (mut r, old) = legacy(&f);
        if link {
            std::os::unix::fs::symlink(f.base.path().join("models"), &old).unwrap();
        } else {
            std::fs::write(&old, b"FOREIGN ROOT FILE").unwrap();
        }
        assert!(run(&f.storage, &mut r, &|| false).is_err());
        assert!(renewal::Review::capture(&f.storage, &r, &|| false).is_err());
        assert!(std::fs::symlink_metadata(old).is_ok());
        assert_eq!(audit_count(&f), 1);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"LEGACY LOGICAL AUDIO");
        f.unaffected();
    }
}

#[test]
fn historical_renewal_database_stages_keep_old_names_and_never_restore_deleted_rows() {
    for phase in [
        completion::Phase::RowsPending,
        completion::Phase::RowsDeleted,
        completion::Phase::Finished,
    ] {
        let f = Fixture::new();
        let (mut r, old) = legacy(&f);
        let fixed = snapshot(&f);
        run(&f.storage, &mut r, &|| false).unwrap();
        let mut database = completion::start(&f.storage, &r, &|| false).unwrap();
        if phase != completion::Phase::RowsPending {
            let stop = Cell::new(false);
            let result =
                completion::run_observed(&f.storage, &mut database, &|| stop.get(), &|event| {
                    if phase == completion::Phase::RowsDeleted && event == "rowsCommitted" {
                        stop.set(true);
                    }
                });
            if phase == completion::Phase::RowsDeleted {
                assert!(result.is_err() && stop.get());
            } else {
                result.unwrap();
            }
        }
        assert_eq!(database.database_phase(), phase);
        reappear(&old);
        assert!(completion::run(&f.storage, &mut database, &|| false).is_err());
        let v = renewal::Review::capture_database(&f.storage, &database, &|| false).unwrap();
        assert!(v.manifest().reservation_roots.contains(&relative(&f, &old)));
        assert!(
            renewal::accept_database(&f.storage, &database, &v, false, &lease(), &|| false)
                .is_err()
        );
        let mut next =
            renewal::accept_database(&f.storage, &database, &v, true, &lease(), &|| false).unwrap();
        assert_eq!(next.progress.database_phase, Some(phase));
        assert_eq!(
            f.storage.get_meeting(&f.meeting).unwrap().is_some(),
            phase == completion::Phase::RowsPending
        );
        finish(&f, &mut next);
        assert!(!old.exists());
        assert_eq!(snapshot(&f), fixed);
    }
}
