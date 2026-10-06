//! Owned copies and real cold Bootstrap; only synthetic audio and private DBs.
//! Production cutover retains all typed history and current-consent checks.
use super::*;
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::relocation::receiver::synthetic_audio_operation_record;
use audio::intent::{self, reconfirmation, removal::renewal};

fn owner(storage: &Storage, operation: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(storage, operation, false, false, &|| false)
        .unwrap()
}
fn policy(include: bool) -> intent::Policy {
    intent::Policy {
        include_unregistered_audio: include,
        retain_unclassified: true,
    }
}
fn complete(storage: &Storage, operation: &str, lease: &DataMaintenanceLease) {
    intent::removal::run(storage, operation, lease, &|| false).unwrap();
    assert!(
        intent::removal::completion::finish(storage, operation, lease, &|| false)
            .unwrap()
            .completed()
    );
}
fn reservation(f: &NativeFixture, operation: &str, generation: u32) -> String {
    let parent = f.audio.rsplit_once('/').unwrap().0;
    let name = if generation == 0 {
        format!(".mon-delete-audio-{operation}-0")
    } else {
        format!(".mon-delete-audio-{operation}-r{generation}-0")
    };
    let relative = format!("{parent}/{name}");
    assert!(relative.len() > 256 && relative.len() <= 512);
    relative
}
fn record(storage: &Storage, operation: &str) -> (String, String) {
    (
        operation.into(),
        synthetic_audio_operation_record(storage, operation),
    )
}
fn audit(storage: &Storage, operation: &str) -> String {
    let raw: serde_json::Value =
        serde_json::from_str(&synthetic_audio_operation_record(storage, operation)).unwrap();
    let progress: serde_json::Value = serde_json::from_str(raw[8].as_str().unwrap()).unwrap();
    assert_eq!(progress["version"], 3);
    assert_eq!(progress["confirmation"]["generation"], 1);
    progress["confirmation"]["operationId"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn unchanged(storage: &Storage, records: &[(String, String)]) {
    for (id, expected) in records {
        assert_eq!(synthetic_audio_operation_record(storage, id), *expected);
    }
}
fn fixture() -> (NativeFixture, String, Vec<(String, String)>) {
    let f = NativeFixture::with_audio_path(false, false, true);
    let storage = f.storage.as_ref().unwrap();
    let lease = owner(storage, &f.deletion);
    std::fs::write(storage.root().join(&f.audio), b"CHILD ORIGINAL AUDIO").unwrap();
    let reviewed =
        reconfirmation::Review::capture(storage, &f.deletion, &lease, &|| false).unwrap();
    let child = Uuid::new_v4().to_string();
    reconfirmation::accept(storage, &child, &reviewed, policy(false), &lease, &|| false).unwrap();
    intent::removal::start(storage, &child, &lease, &|| false).unwrap();
    let late = storage.root().join(reservation(&f, &f.deletion, 0));
    std::fs::write(&late, b"ANCESTOR RETURNED AFTER CHILD START").unwrap();
    let remaining = renewal::Review::capture(storage, &child, &lease, &|| false).unwrap();
    assert!(renewal::accept(storage, &remaining, false, &lease, &|| false).is_err());
    renewal::accept(storage, &remaining, true, &lease, &|| false).unwrap();
    complete(storage, &child, &lease);
    assert!(!late.exists() && !storage.root().join(&f.audio).exists());
    let records = vec![
        record(storage, &f.deletion),
        record(storage, &child),
        record(storage, &audit(storage, &child)),
    ];
    drop(lease);
    (f, child, records)
}
fn next_target(f: &NativeFixture) -> PathBuf {
    let path = f._base.path().join(format!("next-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    path.canonicalize().unwrap()
}

fn sql_rows(db: &Connection, sql: &str) -> Vec<String> {
    let mut query = db.prepare(sql).unwrap();
    let columns = query.column_count();
    let mut rows: Vec<String> = query
        .query_map([], |row| {
            let values: Vec<rusqlite::types::Value> = (0..columns)
                .map(|column| row.get(column))
                .collect::<Result<_, _>>()?;
            Ok(format!("{values:?}"))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort();
    rows
}
fn persistent_rows(path: &Path) -> Vec<(String, Vec<String>)> {
    // Only stopped, private fixtures with no uncheckpointed frames are read
    // as immutable. An ordinary read-only WAL connection can itself create
    // sidecars, so it cannot prove that the application left none behind.
    for sidecar in ["meetings.sqlite3-wal", "meetings.sqlite3-shm"] {
        assert!(!path.with_file_name(sidecar).exists());
    }
    let encoded: String = path
        .to_str()
        .unwrap()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let db = Connection::open_with_flags(
        format!("file:{encoded}?immutable=1"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    let mut facts = vec![(
        "sqlite_schema".into(),
        sql_rows(&db, "SELECT type,name,tbl_name,sql FROM main.sqlite_schema"),
    )];
    let tables: Vec<(String, String)> = db
        .prepare("SELECT name,sql FROM main.sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (table, schema) in tables {
        let columns = if schema.to_ascii_uppercase().contains("WITHOUT ROWID") {
            "*"
        } else {
            "rowid,*"
        };
        let quoted = table.replace('"', "\"\"");
        let rows = sql_rows(&db, &format!("SELECT {columns} FROM main.\"{quoted}\""));
        facts.push((table, rows));
    }
    facts
}

#[test]
fn audio_remaining_relocation_long_ancestral_history_reconsents_cold_resumes_and_moves_again() {
    let (mut f, child, mut records) = fixture();
    let former = f.storage.as_ref().unwrap().root().to_path_buf();
    let relative = reservation(&f, &child, 1);
    f.select();
    let (b, storage) = f.open();
    unchanged(&storage, &records);
    assert!(!storage.data_write_blocked());
    assert_eq!(
        storage
            .create_backup_archive(&f._base.path().join("first.monbackup"), &|| false)
            .unwrap()
            .deleted_audio_meeting_count,
        1
    );
    drop(storage);
    drop(b);
    let returned = f.target.join(&relative);
    std::fs::write(&returned, b"LONG RESERVED AUDIO IN MOVED ROOT").unwrap();
    let old_body = former.join(&relative);
    std::fs::write(&old_body, b"OLD ROOT MUST STAY UNTOUCHED").unwrap();
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let lease = owner(&storage, &child);
    assert!(intent::removal::run(&storage, &child, &lease, &|| false).is_err());
    let review = reconfirmation::Review::capture(&storage, &child, &lease, &|| false).unwrap();
    let current = Uuid::new_v4().to_string();
    assert!(
        reconfirmation::accept(&storage, &current, &review, policy(false), &lease, &|| {
            false
        })
        .is_err()
    );
    reconfirmation::accept(&storage, &current, &review, policy(true), &lease, &|| false).unwrap();
    intent::removal::start(&storage, &current, &lease, &|| false).unwrap();
    let original = crate::storage::deletion::relocation::receiver::synthetic_consent_record(
        &storage, &current,
    );
    unchanged(&storage, &records);
    drop(lease);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    let lease = owner(&storage, &current);
    std::fs::write(&returned, b"CURRENT SCOPE CHANGED AFTER COLD START").unwrap();
    assert!(intent::removal::run(&storage, &current, &lease, &|| false).is_err());
    let review = renewal::Review::capture(&storage, &current, &lease, &|| false).unwrap();
    assert!(renewal::accept(&storage, &review, false, &lease, &|| false).is_err());
    renewal::accept(&storage, &review, true, &lease, &|| false).unwrap();
    complete(&storage, &current, &lease);
    assert_eq!(
        crate::storage::deletion::relocation::receiver::synthetic_consent_record(
            &storage, &current
        ),
        original
    );
    assert!(!returned.exists());
    assert_eq!(
        std::fs::read(&old_body).unwrap(),
        b"OLD ROOT MUST STAY UNTOUCHED"
    );
    unchanged(&storage, &records);
    records.push(record(&storage, &current));
    records.push(record(&storage, &audit(&storage, &current)));
    drop(lease);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    let next = next_target(&f);
    repeated::select_current(&b, &storage, &next);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert_eq!(storage.root(), next);
        assert!(!storage.data_write_blocked());
        unchanged(&storage, &records);
        let lease = owner(&storage, &current);
        assert!(intent::removal::run(&storage, &current, &lease, &|| false).is_err());
        drop(lease);
        assert_eq!(
            storage
                .create_backup_archive(
                    &f._base.path().join(format!("{}.monbackup", Uuid::new_v4())),
                    &|| false
                )
                .unwrap()
                .deleted_audio_meeting_count,
            1
        );
        assert_eq!(
            std::fs::read(&old_body).unwrap(),
            b"OLD ROOT MUST STAY UNTOUCHED"
        );
        drop(storage);
        drop(b);
    }
}

#[test]
fn audio_remaining_relocation_long_history_survives_full_purge_and_two_owned_moves() {
    let (mut f, _, records) = fixture();
    let storage = f.storage.as_ref().unwrap();
    let consent: Vec<_> = records
        .iter()
        .map(|(id, _)| {
            (
                id.clone(),
                crate::storage::deletion::relocation::receiver::synthetic_consent_record(
                    storage, id,
                ),
            )
        })
        .collect();
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut prepared = operations::Prepared::fresh(storage, &f.meeting, &|| false).unwrap();
    assert!(
        prepared
            .execute(
                storage,
                &lease,
                &prepared.preview().confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    drop(lease);
    for (id, before) in consent {
        assert_eq!(
            crate::storage::deletion::relocation::receiver::synthetic_consent_record(storage, &id),
            before
        );
    }
    // Full purge detaches nullable meeting FKs; fix the post-purge facts once.
    let records: Vec<_> = records.iter().map(|(id, _)| record(storage, id)).collect();
    f.select();
    let (b, storage) = f.open();
    assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
    unchanged(&storage, &records);
    let next = next_target(&f);
    repeated::select_current(&b, &storage, &next);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert_eq!(storage.root(), next);
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        unchanged(&storage, &records);
        assert_eq!(
            storage
                .create_backup_archive(
                    &f._base.path().join(format!("{}.monbackup", Uuid::new_v4())),
                    &|| false
                )
                .unwrap()
                .deleted_audio_meeting_count,
            0
        );
        drop(storage);
        drop(b);
    }
}

#[test]
fn audio_remaining_relocation_fences_changed_audit_progress_without_native_record_mutation() {
    let (mut f, child, _) = fixture();
    let audit = audit(f.storage.as_ref().unwrap(), &child);
    f.select();
    drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
    let path = f.target.join("meetings.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
        params![audit, r#"{"audioRemainingConfirmationRecorded":false}"#],
    )
    .unwrap();
    drop(db);
    // Storage changes DELETE-mode snapshots to WAL before installing its
    // temporary attention fence. Compare every persistent row (including
    // rowids) and schema, not the journal-mode bytes in the SQLite header.
    let changed = persistent_rows(&path);
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
    let storage = b.open_storage().unwrap();
    assert!(storage.data_write_blocked());
    assert!(storage
        .create_meeting("NO NORMAL WRITES THROUGH A CORRUPT AUDIT")
        .is_err());
    let listing = audio::operations::listing(&storage).unwrap();
    assert!(listing.write_blocked);
    assert!(listing
        .items
        .iter()
        .any(|i| i.operation_id == child && !i.review_allowed && i.error_message.is_some()));
    assert!(AnalysisController::default()
        .begin_audio_deletion_maintenance(&storage, &child, false, false, &|| false)
        .is_err());
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(audio::operations::Prepared::pending(&storage, &child, &lease, &|| false).is_err());
    assert!(reconfirmation::Review::capture(&storage, &child, &lease, &|| false).is_err());
    drop(lease);
    let archive = f._base.path().join("must-not-export.monbackup");
    assert!(storage.create_backup_archive(&archive, &|| false).is_err());
    assert!(!archive.exists());
    drop(storage);
    drop(b);
    assert_eq!(persistent_rows(&path), changed);
    assert!(!f.target.join("meetings.sqlite3-wal").exists());
}
