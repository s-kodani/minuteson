use super::*;

fn reference(name: &str, hash: char, role: &str) -> ModelFileMetadata {
    ModelFileMetadata {
        name: name.into(),
        sha256: hash.to_string().repeat(64),
        size: 5,
        role: role.into(),
    }
}
fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT,applied_at INTEGER);").unwrap();
    for &(version, sql) in MIGRATIONS {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,0)",
            params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
        )
        .unwrap();
    }
    db
}
fn save(db: &mut Connection, refs: &[ModelFileMetadata]) {
    let tx = db.transaction().unwrap();
    retain(&tx, refs, &crate::test_support::never_cancelled, &|_| {
        Ok(true)
    })
    .unwrap();
    tx.commit().unwrap();
}
#[test]
fn historical_model_metadata_keeps_names_roles_and_distinct_hashes_without_claiming_authenticity() {
    let mut db = db();
    let refs = vec![
        reference("model.bin", 'a', "model"),
        reference("model.bin", 'b', "model"),
        reference("alias.bin", 'a', "model"),
        reference("model.bin", 'a', "executable"),
    ];
    save(&mut db, &refs);
    save(&mut db, &refs);
    save(&mut db, &[refs[0].clone(), refs[0].clone()]);
    assert_eq!(
        historical(&db)
            .unwrap()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        refs.into_iter().collect()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM model_packages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM model_packages WHERE engine!='unrecorded' OR model_version!='unrecorded' OR state!='unavailable' OR license_ref IS NOT NULL) OR EXISTS(SELECT 1 FROM model_files WHERE expected_hash IS NOT NULL OR expected_size IS NOT NULL OR state!='unavailable')",[],|r|r.get::<_,bool>(0)).unwrap());
}
#[test]
fn historical_model_metadata_rejects_traversal_invalid_hash_and_oversized_identity() {
    for name in ["", ".", "..", "../model", "/model", "C:\\model", "a\0b"] {
        assert!(reference(name, 'a', "model").validate().is_err());
    }
    let mut r = reference("model.bin", 'A', "model");
    assert!(r.validate().is_err());
    r.sha256 = "a".repeat(64);
    r.size = u64::MAX;
    assert!(r.validate().is_err());
    r.size = 1;
    r.role = "a".repeat(257);
    assert!(r.validate().is_err());
}
#[test]
fn historical_model_metadata_requires_transaction_and_rollback_retains_no_partial_packages() {
    let mut db = db();
    let refs = [
        reference("model.bin", 'a', "model"),
        reference("other.bin", 'b', "model"),
    ];
    assert!(
        retain(&db, &refs, &crate::test_support::never_cancelled, &|_| Ok(
            true
        ))
        .is_err()
    );
    let tx = db.transaction().unwrap();
    retain(&tx, &refs, &crate::test_support::never_cancelled, &|_| {
        Ok(true)
    })
    .unwrap();
    tx.rollback().unwrap();
    assert!(historical(&db).unwrap().is_empty());
    let tx = db.transaction().unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(retain(
        &tx,
        &refs,
        &|| {
            calls.set(calls.get() + 1);
            calls.get() > 3
        },
        &|_| Ok(true)
    )
    .is_err());
    tx.rollback().unwrap();
    assert!(historical(&db).unwrap().is_empty());
}
#[test]
fn historical_model_metadata_cannot_promote_availability_or_mutate_fingerprint() {
    let mut db = db();
    save(&mut db, &[reference("model.bin", 'a', "model")]);
    for sql in [
        "UPDATE model_packages SET state='ready'",
        "UPDATE model_packages SET model_name='other'",
        "UPDATE model_files SET state='ready'",
        "UPDATE model_files SET actual_hash=replace(actual_hash,'a','b')",
    ] {
        assert!(db.execute_batch(sql).is_err());
    }
    assert_eq!(
        historical(&db).unwrap(),
        vec![reference("model.bin", 'a', "model")]
    );
}
#[test]
fn historical_model_metadata_rejects_corrupt_or_missing_records_before_name_reuse() {
    for sql in [
        "UPDATE model_packages SET manifest_hash=replace(manifest_hash,'a','b')",
        "UPDATE model_files SET actual_hash=replace(actual_hash,'a','b')",
        "UPDATE model_packages SET engine='guessed'",
        "DELETE FROM model_files",
    ] {
        let mut db = db();
        save(&mut db, &[reference("model.bin", 'a', "model")]);
        db.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        // Hash may not contain 'a'; use a guaranteed different 64-byte value.
        let sql = if sql.starts_with("UPDATE model_packages SET manifest_hash") {
            "UPDATE model_packages SET manifest_hash=printf('%064d',0)"
        } else {
            sql
        };
        db.execute_batch(sql).unwrap();
        assert!(historical(&db).is_err(), "accepted {sql}");
    }
}
#[test]
fn historical_model_metadata_current_reopen_preserves_existing_data_and_empty_history() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("既存会議").unwrap();
    assert_eq!(
        verify_migration_history(&storage.connection.lock().unwrap()).unwrap(),
        MIGRATIONS.len()
    );
    let managed = storage.root.clone();
    drop(storage);
    let storage = Storage::open(managed).unwrap();
    assert_eq!(storage.get_meeting(&meeting.id).unwrap().unwrap(), meeting);
    assert_eq!(
        verify_migration_history(&storage.connection.lock().unwrap()).unwrap(),
        MIGRATIONS.len()
    );
    assert!(historical(&storage.connection.lock().unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn model_metadata_schema_allows_package_subdirectories_but_rejects_unsafe_relative_paths() {
    let db = db();
    let package = Uuid::new_v4().to_string();
    db.execute("INSERT INTO model_packages(id,engine,model_name,model_version,source_type,manifest_json,manifest_hash,state) VALUES(?1,'unrecorded','test','unrecorded','manual_import','{}',?2,'preparing')",params![package,"a".repeat(64)]).unwrap();
    let insert = |path: &str| {
        db.execute("INSERT INTO model_files(id,package_id,relative_path,role,actual_size,actual_hash,state) VALUES(?1,?2,?3,'model',1,?4,'preparing')",params![Uuid::new_v4().to_string(),package,path,"b".repeat(64)])
    };
    assert_eq!(insert("weights/model.gguf").unwrap(), 1);
    for path in [
        "",
        ".",
        "..",
        "../weights",
        "weights/../model",
        "weights/./model",
        "weights//model",
        "/weights/model",
        "weights/",
        "weights\\model",
        "C:weights/model",
        "a\0b",
    ] {
        assert!(insert(path).is_err(), "unsafe package path {path:?}");
    }
}
