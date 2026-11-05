use super::*;

#[test]
fn restore_rebinding_missing_current_binding_table_is_not_adopted_as_v3_or_recreated() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let meeting = s.create_meeting("current registry").unwrap();
    drop(s);
    let path = root.path().join(".mon-scratch.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER relocation_restore_binding_fixed; DROP TRIGGER relocation_restore_binding_retained; DROP TABLE relocation_restore_bindings;").unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    assert!(Storage::open(root.path().into()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let db = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
    assert!(db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
            params![meeting.id],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn old_registry_versions_two_and_three_are_rejected_without_rewriting_keep_receipts() {
    for version in [2, 3] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        s.create_meeting("current root").unwrap();
        let stage = s.backup_stage().unwrap();
        let path = stage.path().to_owned();
        std::fs::write(path.join("unknown"), b"keep owned body").unwrap();
        drop(stage);
        let id = s.recovery_listing().unwrap().items[0].operation_id.clone();
        let mut keep = s
            .prepare_recovery(
                crate::storage::backup::recovery::Source::Scratch,
                &id,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
        keep.resolve(
            &s,
            &keep.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        drop(keep);
        let journal = s.root.join(".mon-scratch.sqlite3");
        drop(s);
        let db = Connection::open(&journal).unwrap();
        let receipt: String = db
            .query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
                r.get(0)
            })
            .unwrap();
        db.execute_batch("DROP TRIGGER relocation_restore_binding_fixed; DROP TRIGGER relocation_restore_binding_retained; DROP TABLE relocation_restore_bindings;").unwrap();
        if version == 2 {
            db.execute_batch("DROP TRIGGER relocation_history_fixed; DROP TRIGGER relocation_history_retained; DROP TABLE relocation_history;").unwrap();
        }
        db.pragma_update(None, "user_version", version).unwrap();
        drop(db);
        let before = std::fs::read(&journal).unwrap();
        assert!(Storage::open(root.path().into()).is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), before);
        assert_eq!(
            std::fs::read(path.join("unknown")).unwrap(),
            b"keep owned body"
        );
        let db = Connection::open(&journal).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            db.query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            receipt
        );
        assert!(!db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1)",
                ["relocation_restore_bindings"],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
}

#[test]
fn history_install_missing_current_history_is_not_adopted_as_old_schema_or_recreated() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    s.create_meeting("protected meeting").unwrap();
    let db = Connection::open(s.root.join(".mon-scratch.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER relocation_history_fixed; DROP TRIGGER relocation_history_retained; DROP TABLE relocation_history;").unwrap();
    drop(db);
    drop(s);
    let registry = root.path().join(".mon-scratch.sqlite3");
    let before = std::fs::read(&registry).unwrap();
    assert!(Storage::open(root.path().into()).is_err());
    assert_eq!(std::fs::read(&registry).unwrap(), before);
    let db = Connection::open(&registry).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}
