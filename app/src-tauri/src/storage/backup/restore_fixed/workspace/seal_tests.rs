use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    output: tempfile::TempDir,
    source: archive::VerifiedArchive,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    archive::tests::meeting(&storage, "seal");
    let path = output.path().join("source.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    Fixture {
        _root: root,
        storage,
        output,
        source,
    }
}
fn work(fixed: &FixedData<'_, '_>, output: &Path) -> FixedWorkspace {
    FixedWorkspace::prepare(fixed, output, &crate::test_support::never_cancelled)
        .unwrap()
        .with_speech_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_generation_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_processing_history(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_provenance(fixed, &crate::test_support::never_cancelled)
        .unwrap()
}

#[test]
fn restore_seal_completes_all_passes_restores_trusted_schema_and_reopens_read_only() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed = SealedWorkspace::prepare(
        &fixed,
        f.output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let stage = sealed.stage.path().to_path_buf();
    assert_eq!(sealed.batch_id, ids.batch_id);
    assert_eq!(sealed.archive_id, ids.archive_id);
    assert!(sealed.required_bytes() > 0);
    let trusted = portable::schema(MIGRATIONS.len()).unwrap();
    assert_eq!(
        schema(sealed.connection()).unwrap(),
        schema(&trusted).unwrap()
    );
    assert_eq!(
        sealed
            .connection()
            .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(sealed
        .connection()
        .execute("UPDATE meetings SET title='改変'", [])
        .is_err());
    sealed
        .connection()
        .execute_batch("PRAGMA query_only=OFF;")
        .unwrap();
    assert!(sealed
        .connection()
        .execute("UPDATE meetings SET title='改変'", [])
        .is_err());
    sealed
        .connection()
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    let external = Connection::open(stage.join("fixed.sqlite3")).unwrap();
    assert!(external
        .execute("UPDATE restore_provenance SET source_key='changed'", [])
        .is_err());
    drop(external);
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    drop(sealed);
    assert!(!stage.exists());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
}

#[test]
fn restore_seal_rejects_unfinished_passes_and_completed_database_changes() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let partial = FixedWorkspace::prepare(
        &fixed,
        f.output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let stage = partial.stage.path().to_path_buf();
    assert!(partial
        .seal(&fixed, &crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("全構築"));
    assert!(!stage.exists());
    let partial = FixedWorkspace::prepare(
        &fixed,
        f.output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(partial
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .is_err());
    let complete = work(&fixed, f.output.path());
    let stage = complete.stage.path().to_path_buf();
    complete
        .database
        .execute("UPDATE meetings SET title='構築後の変更'", [])
        .unwrap();
    assert!(complete
        .seal(&fixed, &crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("構築完了後"));
    assert!(!stage.exists());
}

#[test]
fn restore_seal_checks_every_registered_file_and_rejects_unregistered_paths() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    for change in ["bytes", "extra", "directory"] {
        let complete = work(&fixed, f.output.path());
        let stage = complete.stage.path().to_path_buf();
        match change {
            "bytes" => {
                let path: String = complete
                    .database
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                std::fs::write(stage.join(path), b"changed").unwrap();
            }
            "extra" => {
                std::fs::write(stage.join("foreign.txt"), b"foreign").unwrap();
            }
            "directory" => {
                std::fs::create_dir(stage.join("foreign")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            complete
                .seal(&fixed, &crate::test_support::never_cancelled)
                .is_err(),
            "{change}"
        );
        assert!(!stage.exists());
    }
}

#[test]
fn restore_seal_reverification_detects_database_and_file_changes_and_cancellation() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    for change in ["database", "file", "cancel"] {
        let sealed = SealedWorkspace::prepare(
            &fixed,
            f.output.path(),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let stage = sealed.stage.path().to_path_buf();
        match change {
            "database" => {
                Connection::open(stage.join("fixed.sqlite3"))
                    .unwrap()
                    .execute("UPDATE meetings SET title='変更'", [])
                    .unwrap();
            }
            "file" => {
                let path: String = sealed
                    .database
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                std::fs::write(stage.join(path), b"changed").unwrap();
            }
            "cancel" => {}
            _ => unreachable!(),
        }
        assert!(sealed.verify(&|| change == "cancel").is_err());
        drop(sealed);
        assert!(!stage.exists());
    }
}

#[test]
fn restore_seal_cancellation_during_final_hashing_cleans_only_owned_stage() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let complete = work(&fixed, f.output.path());
    let stage = complete.stage.path().to_path_buf();
    let marker = f.output.path().join("keep.txt");
    std::fs::write(&marker, b"keep").unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(complete
        .seal(&fixed, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 3
        })
        .is_err());
    assert!(!stage.exists());
    assert_eq!(std::fs::read(marker).unwrap(), b"keep");
}

#[test]
fn restore_seal_rejects_incomplete_or_injected_trigger_definitions() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    for change in ["missing", "injected"] {
        let mut complete = work(&fixed, f.output.path());
        let stage = complete.stage.path().to_path_buf();
        if change == "missing" {
            complete.triggers.pop();
        } else {
            complete.triggers.push("CREATE TRIGGER fixture_injected BEFORE UPDATE ON meetings BEGIN SELECT RAISE(ABORT,'injected'); END".into());
        }
        assert!(complete
            .seal(&fixed, &crate::test_support::never_cancelled)
            .err()
            .unwrap()
            .contains("信頼済み"));
        assert!(!stage.exists());
    }
}

#[cfg(unix)]
#[test]
fn restore_seal_rejects_leaf_and_parent_symlinks_without_removing_external_data() {
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let keep = outside.path().join("keep.txt");
    std::fs::write(&keep, b"keep").unwrap();
    for change in ["leaf", "extra-link", "parent"] {
        let complete = work(&fixed, f.output.path());
        let stage = complete.stage.path().to_path_buf();
        let relative: String = complete
            .database
            .query_row(
                "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let path = stage.join(relative);
        match change {
            "leaf" => {
                std::fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(&keep, &path).unwrap();
            }
            "extra-link" => {
                std::os::unix::fs::symlink(&keep, stage.join("extra")).unwrap();
            }
            "parent" => {
                let parent = path.parent().unwrap();
                let held = parent.with_file_name("held");
                std::fs::rename(parent, &held).unwrap();
                std::os::unix::fs::symlink(outside.path(), parent).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(complete
            .seal(&fixed, &crate::test_support::never_cancelled)
            .is_err());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&keep).unwrap(), b"keep");
    }
}

#[test]
fn restore_sealed_global_merge_keeps_duplicate_origin_as_history_without_reusing_existing_id() {
    let f = fixture();
    let term = VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: "点検日".into(),
        reading: Some("てんけんび".into()),
        note: None,
        enabled: true,
    };
    f.storage
        .save_vocabulary_book(None, 0, vec![term.clone()])
        .unwrap();
    let backup = f.output.path().join("terms.monbackup");
    f.storage
        .create_backup_archive(&backup, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(backup).unwrap(),
        f.output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let mut existing = term.clone();
    existing.id = Uuid::new_v4().to_string();
    target
        .save_vocabulary_book(None, 0, vec![existing.clone()])
        .unwrap();
    let ids = RestoreIds::prepare(
        &source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let restored = ids.get("vocabulary_entries", "", &term.id).unwrap();
    assert_ne!(restored, existing.id);
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed = SealedWorkspace::prepare(
        &fixed,
        f.output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut db = target.connection.lock().unwrap();
    let merge = sealed
        .global_merge(&db, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(merge.receipt().deduplicated, 1);
    let mut tx = db.transaction().unwrap();
    merge
        .apply(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM vocabulary_entries WHERE id=?1",
            params![restored],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(sealed.database.query_row("SELECT entity_key FROM restore_provenance WHERE domain='vocabulary_entries' AND source_key=?1",params![term.id],|r|r.get::<_,String>(0)).unwrap(),restored);
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    drop(db);
    assert_eq!(
        target.vocabulary_book(None).unwrap().payload,
        vec![existing]
    );
}

#[cfg(unix)]
#[test]
fn restore_seal_rejects_hardlinks_and_relaxed_permissions_on_owned_database_files_or_root() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let ids = RestoreIds::prepare(
        &f.source,
        f.output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed =
        FixedData::new(&ids, f.output.path(), &crate::test_support::never_cancelled).unwrap();
    let outside = tempfile::tempdir().unwrap();
    for change in [
        "asset-link",
        "database-link",
        "asset-permission",
        "root-permission",
    ] {
        let complete = work(&fixed, f.output.path());
        let stage = complete.stage.path().to_path_buf();
        let relative: String = complete
            .database
            .query_row(
                "SELECT relative_path FROM file_assets WHERE state='ready' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let asset = stage.join(relative);
        let held = outside.path().join(change);
        match change {
            "asset-link" => {
                std::fs::hard_link(&asset, &held).unwrap();
            }
            "database-link" => {
                std::fs::hard_link(stage.join("fixed.sqlite3"), &held).unwrap();
            }
            "asset-permission" => {
                std::fs::set_permissions(&asset, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            "root-permission" => {
                std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            _ => unreachable!(),
        }
        let error = complete
            .seal(&fixed, &crate::test_support::never_cancelled)
            .err()
            .unwrap();
        assert!(
            error.contains("ハードリンク") || error.contains("非公開権限"),
            "{change}: {error}"
        );
        assert!(!stage.exists());
        if change.ends_with("link") {
            assert!(held.is_file());
        }
    }
}
