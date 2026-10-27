use super::*;

// The application remains closed. SQLite reads only the accepted workspace
// snapshot; source handles below inspect metadata and absence without writes.
#[test]
#[ignore = "requires explicitly selected source and workspace DB snapshot"]
fn supplied_completed_deletion_history_survives_audio_schema_upgrade() {
    let source = std::path::PathBuf::from(std::env::var_os("MON_STARTUP_HISTORY_SOURCE").unwrap());
    let snapshot =
        std::path::PathBuf::from(std::env::var_os("MON_STARTUP_HISTORY_SNAPSHOT").unwrap());
    assert_ne!(source.join("meetings.sqlite3"), snapshot);
    let db =
        Connection::open_with_flags(snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .unwrap();
    let before: String = db.query_row("SELECT json_group_array(json_array(id,target_hash,progress_json)) FROM (SELECT * FROM maintenance_operations ORDER BY id)", [], |r|r.get(0)).unwrap();
    let anchor = RootAnchor::bind(RootAnchor::directory(&source).unwrap()).unwrap();
    let attention = completed_attention(&db, &anchor, &source.join("models")).unwrap();
    assert!(attention.is_empty(), "{attention:?}");
    let after: String = db.query_row("SELECT json_group_array(json_array(id,target_hash,progress_json)) FROM (SELECT * FROM maintenance_operations ORDER BY id)", [], |r|r.get(0)).unwrap();
    assert_eq!(before, after);
}
