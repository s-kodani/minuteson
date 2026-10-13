use super::*;

#[test]
fn filenames_are_portable_bounded_and_do_not_need_structured_minutes() {
    for title in ["a/b:c*?\n<>|\\\"", "../", "", " . "] {
        let name = safe_filename(title, 0).unwrap();
        assert!(name.ends_with("_議事録.md"));
        assert!(!name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)));
    }
    assert!(safe_filename("定例", 0)
        .unwrap()
        .ends_with("_定例_議事録.md"));
    assert!(safe_filename(&"会".repeat(200), 0).unwrap().len() <= 240);
    assert!(safe_filename("会議", i64::MAX).is_err());
}

#[test]
fn stored_receipts_keep_meeting_ownership_and_detached_revision_without_external_writes() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let m = super::super::stored_minutes_fixture::meeting(&s);
    let revision = super::super::stored_minutes_fixture::candidate(&s, &m);
    s.seed_stored_minutes_head(&m, &revision).unwrap();
    let draft = s.seed_stored_minutes_draft(&m, &revision).unwrap();
    let path = root.path().join("historical.md");
    let receipt =
        s.seed_stored_export_receipt(&m, &revision, &draft, &path, "succeeded", b"past output");
    let other = s.create_meeting("other").unwrap();
    let db = s.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE export_records SET meeting_id=?1 WHERE id=?2",
            params![other.id, receipt.id]
        )
        .is_err());
    db.execute(
        "UPDATE export_records SET minutes_revision_id=NULL WHERE id=?1",
        [&receipt.id],
    )
    .unwrap();
    drop(db);
    assert!(s.exports(&m).unwrap()[0].minutes_revision_id.is_none());
    assert!(!path.exists());
}
