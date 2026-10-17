use super::*;
use crate::exports::Destination;
use crate::storage::VocabularyEntry;
use publication::tests::all_rows;
use std::cell::RefCell;

#[test]
fn restore_operation_changed_validation_database_is_rejected_before_reconstruction_without_repair_or_live_writes(
) {
    let f = fixture();
    let mut selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let preview = selected.preview();
    let before = rows(&f.target);
    selected.archive.database.execute_batch("PRAGMA query_only=OFF; UPDATE meetings SET title='changed source'; PRAGMA query_only=ON;").unwrap();
    assert!(selected
        .publish(
            &f.target,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| panic!("must reject before rebuilding")
        )
        .err()
        .unwrap()
        .contains("確認用DB"));
    assert!(selected
        .refresh(
            &f.target,
            &crate::test_support::never_cancelled,
            &|_| panic!("must reject before rebuilding")
        )
        .is_err());
    assert_eq!(selected.preview(), preview);
    assert_eq!(rows(&f.target), before);
    assert!(!f.target.maintenance.load(Ordering::Acquire));
    assert_eq!(
        selected
            .archive
            .database
            .query_row(
                "SELECT count(*) FROM meetings WHERE title='changed source'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn restore_operation_capacity_confirmation_is_rechecked_before_durable_reservation_and_refresh_restores_a_valid_bound(
) {
    let f = fixture();
    let mut selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    // Fault-inject the confirmation bound, not the disk's actual free capacity.
    selected.preview.required_bytes = 0;
    let before = rows(&f.target);
    assert!(selected
        .publish(
            &f.target,
            &selected.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .err()
        .unwrap()
        .contains("必要容量"));
    assert_eq!(rows(&f.target), before);
    let refreshed = selected
        .refresh(&f.target, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    assert!(refreshed.required_bytes > CAPACITY_RESERVE);
    selected
        .publish(
            &f.target,
            &refreshed.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert_eq!(f.target.list_meetings().unwrap().len(), 3);
}

struct Fixture {
    _source: tempfile::TempDir,
    output: tempfile::TempDir,
    target_root: tempfile::TempDir,
    target: Storage,
    path: PathBuf,
}
fn term(text: &str, reading: Option<&str>) -> VocabularyEntry {
    VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: text.into(),
        reading: reading.map(str::to_owned),
        note: None,
        enabled: true,
    }
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let source = Storage::open(root.path().into()).unwrap();
    archive::tests::meeting(&source, "first");
    archive::tests::meeting(&source, "second");
    source
        .save_vocabulary_book(
            None,
            0,
            vec![term("一致", None), term("競合", None), term("新規", None)],
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("selected.monbackup");
    source
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    archive::tests::meeting(&target, "existing");
    target
        .save_vocabulary_book(
            None,
            0,
            vec![term("一致", None), term("競合", Some("既存優先"))],
        )
        .unwrap();
    Fixture {
        _source: root,
        output,
        target_root,
        target,
        path,
    }
}
fn rows(storage: &Storage) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    all_rows(&storage.connection.lock().unwrap())
}

#[test]
fn restore_operation_preview_fully_reconstructs_read_only_reports_capacity_terms_and_cleans_owned_copies_on_drop(
) {
    let f = fixture();
    let before = rows(&f.target);
    let paths = std::fs::read_dir(f.target_root.path()).unwrap().count();
    let phases = RefCell::new(vec![]);
    let selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|p| {
            phases.borrow_mut().push(p.to_owned());
            assert!(f.target.create_meeting("確認中の書込みを拒否").is_err());
            assert!(f
                .target
                .connection
                .lock()
                .unwrap()
                .execute_batch("PRAGMA foreign_keys=OFF")
                .is_err());
        })
        .unwrap();
    let preview = selected.preview();
    assert_eq!(preview.meeting_titles.len(), 2);
    assert!(preview.required_bytes >= CAPACITY_RESERVE);
    assert!(preview.available_bytes >= preview.required_bytes);
    assert_eq!(preview.vocabulary_added, 1);
    assert_eq!(preview.vocabulary_deduplicated, 1);
    assert_eq!(preview.vocabulary_conflicts, vec!["競合"]);
    assert_eq!(
        *phases.borrow(),
        vec!["reading", "reconstructing", "verifying"]
    );
    assert_eq!(rows(&f.target), before);
    assert!(!f.target.maintenance.load(Ordering::Acquire));
    assert_eq!(
        std::fs::read_dir(f.target_root.path()).unwrap().count(),
        paths + 1
    );
    drop(selected);
    assert_eq!(
        std::fs::read_dir(f.target_root.path()).unwrap().count(),
        paths
    );
}

#[test]
fn backup_operation_confirmed_publication_is_terminal_on_lost_response_and_never_overwrites_later_external_edits(
) {
    let f = fixture();
    let path = f.output.path().join("new.monbackup");
    let mut selected = f
        .target
        .prepare_backup_destination(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let preview = selected.preview();
    assert!(!preview.replaces_existing);
    assert!(selected
        .publish(
            &f.target,
            &Uuid::new_v4().to_string(),
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(!path.exists());
    let receipt = selected
        .publish(
            &f.target,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    std::fs::write(&path, b"external edit after publication").unwrap();
    assert_eq!(
        selected
            .publish(&f.target, &preview.confirmation_id, &|| true)
            .unwrap(),
        receipt
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"external edit after publication"
    );
    let path = f.output.path().join("race.monbackup");
    let mut selected = f
        .target
        .prepare_backup_destination(&path, &crate::test_support::never_cancelled)
        .unwrap();
    std::fs::write(&path, b"created after selection").unwrap();
    assert!(selected
        .publish(
            &f.target,
            &selected.preview().confirmation_id,
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"created after selection");
}

#[test]
fn restore_operation_partial_publication_cancel_keeps_existing_and_reconfirms_same_batch_without_double_addition(
) {
    let f = fixture();
    let mut selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let preview = selected.preview();
    let before = f.target.list_meetings().unwrap();
    let book = f.target.vocabulary_book(None).unwrap();
    let existing = before
        .iter()
        .map(|m| m.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let cancelled = std::cell::Cell::new(false);
    let check = || {
        let created = std::fs::read_dir(f.target_root.path().join("meetings"))
            .unwrap()
            .flatten()
            .any(|entry| {
                entry.file_type().unwrap().is_dir()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|id| Uuid::parse_str(id).is_ok() && !existing.contains(id))
            });
        if created {
            cancelled.set(true);
        }
        cancelled.get()
    };
    assert!(selected
        .publish(&f.target, &preview.confirmation_id, &check, &|_| {})
        .is_err());
    assert!(cancelled.get());
    assert_eq!(f.target.list_meetings().unwrap(), before);
    assert_eq!(
        serde_json::to_value(f.target.vocabulary_book(None).unwrap()).unwrap(),
        serde_json::to_value(book).unwrap()
    );
    for entry in std::fs::read_dir(f.target_root.path().join("meetings"))
        .unwrap()
        .flatten()
    {
        if let Some(id) = entry.file_name().to_str() {
            if Uuid::parse_str(id).is_ok() {
                assert!(existing.contains(id));
            }
        }
    }
    let refreshed = selected
        .refresh(&f.target, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    assert_eq!(refreshed.batch_id, preview.batch_id);
    let receipt = selected
        .publish(
            &f.target,
            &refreshed.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(!receipt.already_published);
    assert_eq!(f.target.list_meetings().unwrap().len(), 3);
    assert!(
        selected
            .publish(
                &f.target,
                &refreshed.confirmation_id,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .already_published
    );
    assert_eq!(f.target.list_meetings().unwrap().len(), 3);
}

#[test]
fn restore_operation_exact_confirm_adds_and_replays_same_batch_after_source_moves_but_explicit_new_selection_adds_another_batch(
) {
    let f = fixture();
    let keep = f.target.list_meetings().unwrap();
    let selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let preview = selected.preview();
    std::fs::rename(&f.path, f.output.path().join("moved.monbackup")).unwrap();
    let before = rows(&f.target);
    assert!(selected
        .publish(
            &f.target,
            &Uuid::new_v4().to_string(),
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert_eq!(rows(&f.target), before);
    let receipt = selected
        .publish(
            &f.target,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(!receipt.already_published);
    assert_eq!(receipt.meeting_ids.len(), 2);
    assert_eq!(receipt.vocabulary_added, Some(1));
    assert_eq!(receipt.vocabulary_conflicts, Some(vec!["競合".into()]));
    for meeting in &keep {
        assert_eq!(
            f.target.get_meeting(&meeting.id).unwrap().as_ref(),
            Some(meeting)
        );
    }
    let before = rows(&f.target);
    let replay = selected
        .publish(
            &f.target,
            &preview.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| panic!("terminal replay must not reconstruct"),
        )
        .unwrap();
    assert!(replay.already_published);
    assert_eq!(replay.vocabulary_added, None);
    assert_eq!(rows(&f.target), before);
    assert_eq!(
        receipt
            .meeting_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>(),
        replay.meeting_ids.iter().collect()
    );
    let another = f
        .target
        .prepare_additive_restore(
            &f.output.path().join("moved.monbackup"),
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert_ne!(another.preview().batch_id, preview.batch_id);
    let next = another
        .publish(
            &f.target,
            &another.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(next
        .meeting_ids
        .iter()
        .all(|id| !receipt.meeting_ids.contains(id)));
    assert_eq!(f.target.list_meetings().unwrap().len(), 5);
}

#[test]
fn restore_operation_rejects_changed_global_confirmation_before_reservation_and_refresh_invalidates_old_confirmation(
) {
    let f = fixture();
    let mut selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let old = selected.preview();
    let book = f.target.vocabulary_book(None).unwrap();
    let mut terms = book.payload;
    terms.push(term("後から追加", None));
    f.target
        .save_vocabulary_book(None, book.lock_version, terms)
        .unwrap();
    let before = rows(&f.target);
    assert!(selected
        .publish(
            &f.target,
            &old.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .err()
        .unwrap()
        .contains("再確認"));
    assert_eq!(rows(&f.target), before);
    let refreshed = selected
        .refresh(&f.target, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    assert_eq!(refreshed.batch_id, old.batch_id);
    assert_ne!(refreshed.confirmation_id, old.confirmation_id);
    assert!(selected
        .publish(
            &f.target,
            &old.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    selected
        .publish(
            &f.target,
            &refreshed.confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert_eq!(f.target.list_meetings().unwrap().len(), 3);
}

#[test]
fn restore_operation_cancelled_preview_and_changed_private_audio_are_read_only_failures_and_never_redirect_input(
) {
    let f = fixture();
    let before = rows(&f.target);
    let paths = std::fs::read_dir(f.target_root.path()).unwrap().count();
    let phase = RefCell::new(String::new());
    assert!(f
        .target
        .prepare_additive_restore(
            &f.path,
            &|| phase.borrow().as_str() == "reconstructing",
            &|p| *phase.borrow_mut() = p.into()
        )
        .is_err());
    assert_eq!(rows(&f.target), before);
    assert_eq!(
        std::fs::read_dir(f.target_root.path()).unwrap().count(),
        paths
    );
    let selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let asset = selected.archive.assets.values().next().unwrap();
    std::fs::write(asset, b"changed").unwrap();
    assert!(selected
        .publish(
            &f.target,
            &selected.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert_eq!(rows(&f.target), before);
    assert!(!f.target.maintenance.load(Ordering::Acquire));
}

#[test]
fn backup_operation_retains_selected_overwrite_proof_denies_changed_target_wrong_owner_and_format_and_cancel_keeps_existing(
) {
    let f = fixture();
    let path = f.output.path().join("destination.monbackup");
    std::fs::write(&path, b"original").unwrap();
    let selected = Destination::selected_backup(
        f.target_root.path(),
        &path,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(selected.replaces_existing());
    std::fs::write(&path, b"changed elsewhere").unwrap();
    let error = f
        .target
        .create_backup_selected(&selected, &crate::test_support::never_cancelled)
        .err()
        .unwrap();
    assert!(!error.published);
    assert_eq!(std::fs::read(&path).unwrap(), b"changed elsewhere");
    let selected = Destination::selected_backup(
        f.target_root.path(),
        &path,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let error = f
        .target
        .create_backup_selected(&selected, &|| true)
        .err()
        .unwrap();
    assert!(!error.published);
    assert_eq!(std::fs::read(&path).unwrap(), b"changed elsewhere");
    let wrong =
        Destination::selected(f.target_root.path(), &f.output.path().join("other.md")).unwrap();
    assert!(f
        .target
        .create_backup_selected(&wrong, &crate::test_support::never_cancelled)
        .is_err());
    let root = tempfile::tempdir().unwrap();
    let wrong =
        Destination::selected_backup(root.path(), &path, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(f
        .target
        .create_backup_selected(&wrong, &crate::test_support::never_cancelled)
        .is_err());
    let receipt = f
        .target
        .create_backup_selected(&selected, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(receipt.meeting_count, 1);
    assert!(receipt.size > 0);
    assert_eq!(std::fs::read_dir(f.output.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn restore_operation_rejects_links_wrong_extension_and_cross_destination_capability_without_writes()
{
    let f = fixture();
    let before = rows(&f.target);
    let link = f.output.path().join("link.monbackup");
    std::os::unix::fs::symlink(&f.path, &link).unwrap();
    assert!(f
        .target
        .prepare_additive_restore(&link, &crate::test_support::never_cancelled, &|_| {})
        .is_err());
    assert!(f
        .target
        .prepare_additive_restore(
            Path::new("relative.monbackup"),
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    let wrong = f.output.path().join("wrong.txt");
    std::fs::copy(&f.path, &wrong).unwrap();
    assert!(f
        .target
        .prepare_additive_restore(&wrong, &crate::test_support::never_cancelled, &|_| {})
        .is_err());
    let mut selected = f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let other = Storage::open(root.path().into()).unwrap();
    assert!(selected
        .refresh(&other, &crate::test_support::never_cancelled, &|_| {})
        .is_err());
    assert!(selected
        .publish(
            &other,
            &selected.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert_eq!(rows(&f.target), before);
    let hard = f.output.path().join("hard.monbackup");
    std::fs::hard_link(&f.path, &hard).unwrap();
    assert!(f
        .target
        .prepare_additive_restore(&f.path, &crate::test_support::never_cancelled, &|_| {})
        .is_err());
}
