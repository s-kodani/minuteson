use super::*;
use std::io::Write;
fn fixture() -> (tempfile::TempDir, Storage, String, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_owned();
    private_file(&path.join("owned"))
        .unwrap()
        .write_all(b"owned")
        .unwrap();
    std::fs::write(path.join("unknown"), b"unknown").unwrap();
    drop(stage);
    let listing = storage.recovery_listing().unwrap();
    let id = listing.items[0].operation_id.clone();
    (root, storage, id, path)
}
#[test]
fn recovery_keep_is_explicit_immutable_terminal_and_survives_startup_without_claiming_cleanup() {
    let (root, storage, id, path) = fixture();
    let before = storage.list_meetings().unwrap();
    let mut review = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    let confirmation = review.preview().confirmation_id;
    assert!(review.resolve(&storage, "wrong", true, &|| false).is_err());
    let receipt = review
        .resolve(&storage, &confirmation, true, &|| false)
        .unwrap();
    assert_eq!(receipt.outcome, Outcome::Retained);
    assert!(storage.recovery_listing().unwrap().items[0].retained);
    assert_eq!(
        review
            .resolve(&storage, &confirmation, true, &|| false)
            .unwrap()
            .outcome,
        Outcome::Retained
    );
    assert!(review
        .resolve(&storage, &confirmation, false, &|| false)
        .is_err());
    drop(review);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(std::fs::read(path.join("unknown")).unwrap(), b"unknown");
    assert_eq!(reopened.list_meetings().unwrap(), before);
    assert!(reopened
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .is_err());
}
#[test]
fn recovery_changes_after_confirmation_stop_all_actions_and_retry_can_finish_after_unknown_is_removed(
) {
    let (_root, storage, id, path) = fixture();
    let mut review = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    std::fs::write(path.join("new"), b"new").unwrap();
    assert!(review
        .resolve(&storage, &review.preview().confirmation_id, true, &|| false)
        .is_err());
    assert!(path.join("unknown").exists());
    std::fs::remove_file(path.join("new")).unwrap();
    std::fs::remove_file(path.join("unknown")).unwrap();
    let mut refreshed = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    let receipt = refreshed
        .resolve(
            &storage,
            &refreshed.preview().confirmation_id,
            false,
            &|| false,
        )
        .unwrap();
    assert_eq!(receipt.outcome, Outcome::Recovered);
    assert!(!path.exists());
    assert!(storage.recovery_listing().unwrap().items.is_empty());
}
#[test]
fn recovery_incomplete_retry_is_attention_not_success_and_keeps_unknown_files() {
    let (_root, storage, id, path) = fixture();
    let mut review = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    let receipt = review
        .resolve(&storage, &review.preview().confirmation_id, false, &|| {
            false
        })
        .unwrap();
    assert_eq!(receipt.outcome, Outcome::Attention);
    assert!(path.join("unknown").exists());
    assert!(!storage.recovery_listing().unwrap().items[0].retained);
}

#[test]
fn recovery_legacy_ownership_schema_upgrades_atomically_without_adopting_or_losing_areas() {
    let (root, storage, id, path) = fixture();
    drop(storage);
    let journal = root.path().join(".mon-scratch.sqlite3");
    let db = Connection::open(&journal).unwrap();
    // Recreate the exact pre-review schema, not a damaged current schema.
    #[cfg(target_os = "macos")]
    db.execute_batch("DROP TRIGGER relocation_history_fixed; DROP TRIGGER relocation_history_retained; DROP TABLE relocation_history;")
        .unwrap();
    db.execute_batch("DROP TABLE recovery_decisions; PRAGMA user_version=0;")
        .unwrap();
    drop(db);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(std::fs::read(path.join("unknown")).unwrap(), b"unknown");
    assert_eq!(
        reopened.recovery_listing().unwrap().items[0].operation_id,
        id
    );
    let mut review = reopened
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    assert_eq!(
        review
            .resolve(&reopened, &review.preview().confirmation_id, true, &|| {
                false
            })
            .unwrap()
            .outcome,
        Outcome::Retained
    );
}

#[test]
fn recovery_missing_keep_table_is_not_mistaken_for_legacy_schema_or_used_to_delete_kept_files() {
    let (root, storage, id, path) = fixture();
    let mut review = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    review
        .resolve(&storage, &review.preview().confirmation_id, true, &|| false)
        .unwrap();
    drop(review);
    drop(storage);
    Connection::open(root.path().join(".mon-scratch.sqlite3"))
        .unwrap()
        .execute_batch("DROP TABLE recovery_decisions;")
        .unwrap();
    assert!(Storage::open(root.path().into()).is_err());
    assert!(path.join("unknown").exists());
}

#[test]
fn recovery_preview_and_cancel_are_read_only_and_live_copies_are_never_offered() {
    let (_root, storage, id, path) = fixture();
    let stage = storage.backup_stage().unwrap();
    let live = stage.path().to_owned();
    private_file(&live.join("live"))
        .unwrap()
        .write_all(b"still live")
        .unwrap();
    assert_eq!(storage.recovery_listing().unwrap().items.len(), 1);
    let review = storage
        .prepare_recovery(Source::Scratch, &id, &|| false)
        .unwrap();
    drop(review);
    assert!(path.join("unknown").exists());
    assert!(storage
        .prepare_recovery(Source::Scratch, &id, &|| true)
        .is_err());
    assert_eq!(std::fs::read(live.join("live")).unwrap(), b"still live");
}
