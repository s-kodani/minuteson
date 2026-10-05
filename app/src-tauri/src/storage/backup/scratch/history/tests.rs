use super::*;

#[test]
fn history_install_v2_registry_upgrade_preserves_meetings_and_original_keep_receipt() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let meeting = s.create_meeting("existing v2 root").unwrap();
    let stage = s.backup_stage().unwrap();
    let path = stage.path().to_owned();
    std::fs::write(path.join("unknown"), b"keep existing v2 body").unwrap();
    drop(stage);
    let id = s.recovery_listing().unwrap().items[0].operation_id.clone();
    let mut keep = s
        .prepare_recovery(
            crate::storage::backup::recovery::Source::Scratch,
            &id,
            &|| false,
        )
        .unwrap();
    keep.resolve(&s, &keep.preview().confirmation_id, true, &|| false)
        .unwrap();
    drop(keep);
    let db = Connection::open(s.root.join(".mon-scratch.sqlite3")).unwrap();
    let receipt: String = db
        .query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute_batch("DROP TRIGGER relocation_history_fixed; DROP TRIGGER relocation_history_retained; DROP TABLE relocation_history; PRAGMA user_version=2;").unwrap();
    drop(db);
    drop(s);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        reopened.get_meeting(&meeting.id).unwrap().unwrap().id,
        meeting.id
    );
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"keep existing v2 body"
    );
    assert!(reopened.recovery_listing().unwrap().items[0].retained);
    let db = Connection::open(reopened.root.join(".mon-scratch.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        receipt
    );
}

#[test]
fn history_install_missing_v3_history_is_not_adopted_as_old_schema_or_recreated() {
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
        3
    );
}
