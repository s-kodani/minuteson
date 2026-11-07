use super::super::{files::Census, tests::Fixture as DatabaseFixture};
use super::*;
use std::{cell::Cell, path::PathBuf};

#[path = "database_renewal_tests.rs"]
mod database_renewal_tests;

fn at_database_phase(f: &Fixture, phase: Phase) -> Receipt {
    let mut receipt = f.begin();
    match phase {
        Phase::RowsPending => {}
        Phase::RowsDeleted => {
            let stop = Cell::new(false);
            assert!(
                run_inner(&f.f.storage, &mut receipt, &|| stop.get(), &|event| {
                    if event == "rowsCommitted" {
                        stop.set(true);
                    }
                })
                .is_err()
            );
            assert_eq!(receipt.progress.database_phase, Phase::RowsDeleted);
        }
        Phase::Finished => run(
            &f.f.storage,
            &mut receipt,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
    }
    receipt
}

#[test]
fn compound_current_census_keeps_original_scope_and_never_resurrects_deleted_asset_ownership() {
    for phase in [Phase::RowsPending, Phase::RowsDeleted, Phase::Finished] {
        let f = Fixture::new();
        let receipt = at_database_phase(&f, phase.clone());
        std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
        std::fs::write(&f.file, b"REAPPEARED CURRENT AUDIO").unwrap();
        let db = f.f.storage.connection.lock().unwrap();
        let saved = || {
            db.query_row("SELECT target_manifest_json,target_hash,consent_at,progress_json,updated_at FROM maintenance_operations WHERE id=?1", params![f.operation], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))).unwrap()
        };
        let before = saved();
        let census = receipt
            .review_census(&f.f.storage, &db, &crate::test_support::never_cancelled)
            .unwrap();
        assert_eq!(census.scope(), receipt.confirmed.scope());
        assert_eq!(
            census.manifest().database_scope_hash,
            receipt.confirmed.scope().hash().unwrap()
        );
        let path = f
            .file
            .strip_prefix(f.f.storage.root())
            .unwrap()
            .to_str()
            .unwrap();
        let entry = census
            .manifest()
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap();
        assert!(entry.proof.is_some());
        assert_eq!(entry.registered.is_some(), phase == Phase::RowsPending);
        assert_eq!(
            removal::Reviewed::confirm(&census, false).is_ok(),
            phase == Phase::RowsPending
        );
        removal::Reviewed::confirm(&census, true).unwrap();
        assert_eq!(saved(), before);
        assert!(receipt
            .verify_live(&f.f.storage, &db, &crate::test_support::never_cancelled)
            .is_err());
        drop(db);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"REAPPEARED CURRENT AUDIO");
        assert_eq!(
            f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some(),
            phase == Phase::RowsPending
        );
        f.unaffected();
    }
}

#[test]
fn compound_current_census_rejects_changed_rows_and_recreated_deleted_meeting_without_adoption() {
    for committed in [false, true] {
        let f = Fixture::new();
        let receipt = at_database_phase(
            &f,
            if committed {
                Phase::RowsDeleted
            } else {
                Phase::RowsPending
            },
        );
        let db = f.f.storage.connection.lock().unwrap();
        {
            let _writer = super::super::fence::write_completion(
                &f.f.storage,
                &db,
                &receipt,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            if committed {
                db.execute("INSERT INTO meetings(id,title,lifecycle,source_type,created_at,updated_at,lock_version) VALUES(?1,'RECREATED','draft','recording',0,0,0)", params![f.f.meeting]).unwrap();
            } else {
                db.execute(
                    "UPDATE meetings SET title='CHANGED DB SCOPE' WHERE id=?1",
                    params![f.f.meeting],
                )
                .unwrap();
            }
        }
        assert!(receipt
            .review_census(&f.f.storage, &db, &crate::test_support::never_cancelled)
            .is_err());
        let title: String = db
            .query_row(
                "SELECT title FROM meetings WHERE id=?1",
                params![f.f.meeting],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            title,
            if committed {
                "RECREATED"
            } else {
                "CHANGED DB SCOPE"
            }
        );
        drop(db);
        f.unaffected();
    }
}

#[test]
fn compound_current_census_retains_reserved_root_and_external_symlink_without_following_them() {
    for reserved in [false, true] {
        let f = Fixture::new();
        let receipt = at_database_phase(&f, Phase::RowsDeleted);
        let target = if reserved {
            let index = receipt
                .manifest
                .entries
                .iter()
                .rev()
                .position(|e| e.path == format!("meetings/{}", f.f.meeting))
                .unwrap();
            let path =
                f.f.storage
                    .root()
                    .join(format!("meetings/.mon-delete-{}-{index}", f.operation));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("unknown"), b"KEEP RESERVED CONTENT").unwrap();
            path
        } else {
            let path = f.f.storage.root().join(format!("meetings/{}", f.f.meeting));
            std::os::unix::fs::symlink(f.outside.path(), &path).unwrap();
            path
        };
        let db = f.f.storage.connection.lock().unwrap();
        let census =
            receipt.review_census(&f.f.storage, &db, &crate::test_support::never_cancelled);
        if reserved {
            let census = census.unwrap();
            assert!(census
                .manifest()
                .entries
                .iter()
                .any(|e| !e.directory && e.registered.is_none() && e.path.ends_with("/unknown")));
        } else {
            assert!(census.is_err());
        }
        drop(db);
        assert!(std::fs::symlink_metadata(&target).is_ok());
        if reserved {
            assert_eq!(
                std::fs::read(target.join("unknown")).unwrap(),
                b"KEEP RESERVED CONTENT"
            );
        }
        f.unaffected();
    }
}

#[test]
fn compound_current_census_rejects_root_reservation_created_during_final_saved_verification() {
    let f = Fixture::new();
    let receipt = at_database_phase(&f, Phase::RowsDeleted);
    let index = receipt
        .manifest
        .entries
        .iter()
        .rev()
        .position(|e| e.path == format!("meetings/{}", f.f.meeting))
        .unwrap();
    let reserved =
        f.f.storage
            .root()
            .join(format!("meetings/.mon-delete-{}-{index}", f.operation));
    let db = f.f.storage.connection.lock().unwrap();
    let calls = Cell::new(0usize);
    receipt
        .review_census(&f.f.storage, &db, &|| {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap();
    let total = calls.replace(0);
    assert!(total > 1);
    assert!(receipt
        .review_census(&f.f.storage, &db, &|| {
            calls.set(calls.get() + 1);
            if calls.get() == total {
                std::fs::create_dir(&reserved).unwrap();
                std::fs::write(reserved.join("unknown"), b"KEEP LATE RESERVATION").unwrap();
            }
            false
        })
        .is_err());
    assert_eq!(calls.get(), total);
    drop(db);
    assert_eq!(
        std::fs::read(reserved.join("unknown")).unwrap(),
        b"KEEP LATE RESERVATION"
    );
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    f.unaffected();
}

#[test]
fn compound_current_census_cancellation_is_read_only_and_changed_second_inventory_is_not_old_consent(
) {
    let f = Fixture::new();
    let receipt = at_database_phase(&f, Phase::RowsDeleted);
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"FIRST CURRENT CONTENT").unwrap();
    let db = f.f.storage.connection.lock().unwrap();
    assert!(receipt.review_census(&f.f.storage, &db, &|| true).is_err());
    let first = receipt
        .review_census(&f.f.storage, &db, &crate::test_support::never_cancelled)
        .unwrap();
    std::fs::write(&f.file, b"SECOND CURRENT CONTENT").unwrap();
    let second = receipt
        .review_census(&f.f.storage, &db, &crate::test_support::never_cancelled)
        .unwrap();
    assert_ne!(first.manifest(), second.manifest());
    assert_eq!(first.scope(), second.scope());
    assert!(receipt
        .verify_live(&f.f.storage, &db, &crate::test_support::never_cancelled)
        .is_err());
    drop(db);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"SECOND CURRENT CONTENT");
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    f.unaffected();
}

#[test]
fn compound_current_census_after_cold_row_commit_keeps_absent_rows_and_original_receipt() {
    let f = Fixture::new();
    let original = at_database_phase(&f, Phase::RowsDeleted);
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"KEEP COLD CURRENT CONTENT").unwrap();
    let operation = f.operation.clone();
    let path = f.f.storage.root().to_path_buf();
    let meeting = f.f.meeting.clone();
    let other = f.f.other.clone();
    drop(f.f.storage);
    let storage = Storage::open(path).unwrap();
    assert!(storage.data_write_blocked());
    let receipt = read(&storage, &operation, &crate::test_support::never_cancelled).unwrap();
    original
        .confirmed
        .matches_saved(&receipt.confirmed)
        .unwrap();
    assert_eq!(original.progress, receipt.progress);
    let db = storage.connection.lock().unwrap();
    let census = receipt
        .review_census(&storage, &db, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(census
        .manifest()
        .entries
        .iter()
        .filter(|e| !e.directory)
        .all(|e| e.registered.is_none()));
    assert!(receipt
        .verify_live(&storage, &db, &crate::test_support::never_cancelled)
        .is_err());
    drop(db);
    assert!(storage.require_data_writable().is_err());
    assert!(storage.get_meeting(&meeting).unwrap().is_none());
    assert!(storage.get_meeting(&other).unwrap().is_some());
    assert_eq!(
        std::fs::read(&f.file).unwrap(),
        b"KEEP COLD CURRENT CONTENT"
    );
}

fn assert_free_pages_zero(bytes: &[u8]) {
    let read_u32 = |bytes: &[u8]| u32::from_be_bytes(bytes.try_into().unwrap());
    let page_size = match u16::from_be_bytes(bytes[16..18].try_into().unwrap()) {
        1 => 65536,
        n => usize::from(n),
    };
    let page = |n: u32| &bytes[(n as usize - 1) * page_size..n as usize * page_size];
    let mut trunk = read_u32(&bytes[32..36]);
    let expected = read_u32(&bytes[36..40]) as usize;
    let mut seen = std::collections::BTreeSet::new();
    while trunk != 0 {
        assert!(seen.insert(trunk));
        let b = page(trunk);
        let count = read_u32(&b[4..8]) as usize;
        assert!(8 + 4 * count <= page_size);
        for i in 0..count {
            let leaf = read_u32(&b[8 + 4 * i..12 + 4 * i]);
            assert!(seen.insert(leaf));
            assert!(page(leaf).iter().all(|b| *b == 0));
        }
        assert!(b[8 + 4 * count..].iter().all(|b| *b == 0));
        trunk = read_u32(&b[..4]);
    }
    assert_eq!(seen.len(), expected);
}

fn assert_matching_copy_is_not_a_writer(f: &Fixture, receipt: &Receipt) {
    let db = f.f.storage.connection.lock().unwrap();
    let mut copied = Connection::open_in_memory().unwrap();
    rusqlite::backup::Backup::new(&db, &mut copied)
        .unwrap()
        .run_to_completion(32, std::time::Duration::from_millis(1), None)
        .unwrap();
    copied.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    receipt
        .verify_saved(&copied, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(super::super::fence::write_completion(
        &f.f.storage,
        &copied,
        receipt,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

struct Fixture {
    f: DatabaseFixture,
    outside: tempfile::TempDir,
    operation: String,
    file: PathBuf,
    other_file: PathBuf,
    marker: String,
    settings: Vec<(String, String, String, i64, i64)>,
}

fn saved_settings(db: &Connection) -> Vec<(String, String, String, i64, i64)> {
    db.prepare(
        "SELECT key,value_json,content_hash,lock_version,updated_at FROM app_settings ORDER BY key",
    )
    .unwrap()
    .query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}
impl Fixture {
    fn new() -> Self {
        Self::at(tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }
    fn at(root: tempfile::TempDir, outside: tempfile::TempDir) -> Self {
        let f = DatabaseFixture::at(root);
        let file = f
            .storage
            .root()
            .join(format!("meetings/{}/audio/source.flac", f.meeting));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"DELETE TARGET AUDIO").unwrap();
        f.storage
            .register_ready_meeting_asset(
                &f.meeting,
                file.strip_prefix(f.storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        let other_file = f
            .storage
            .root()
            .join(format!("meetings/{}/audio/source.flac", f.other));
        std::fs::create_dir_all(other_file.parent().unwrap()).unwrap();
        std::fs::write(&other_file, b"KEEP OTHER AUDIO").unwrap();
        std::fs::write(outside.path().join("export.md"), b"KEEP EXTERNAL EXPORT").unwrap();
        let model = f.storage.root().join("models/model.gguf");
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(model, b"KEEP MODEL").unwrap();
        let marker = format!("DELETE-PRIVATE-{}-BODY", uuid::Uuid::new_v4());
        let db = f.storage.connection.lock().unwrap();
        // The legacy minutes-only fixture used a non-UUID synthetic path.
        // Keep its asset ID/FKs, but make this end-to-end fixture a valid
        // meeting-scoped missing asset, not a production validation exception.
        assert_eq!(db.execute(
            "UPDATE file_assets SET relative_path=?2 WHERE meeting_id=?1 AND relative_path='meetings/test/audio.caf'",
            params![f.meeting,format!("meetings/{}/audio/fixture.caf",f.meeting)],
        ).unwrap(),1);
        assert_eq!(db.execute(
            "UPDATE file_assets SET relative_path=?2 WHERE meeting_id=?1 AND relative_path='meetings/test/manifest.json'",
            params![f.meeting,format!("meetings/{}/manifest.json",f.meeting)],
        ).unwrap(),1);
        db.execute(
            "UPDATE meetings SET title=?2 WHERE id=?1",
            params![f.meeting, marker],
        )
        .unwrap();
        db.execute("UPDATE working_drafts SET payload_json=json_set(payload_json,'$.deletionProbe',?2) WHERE meeting_id=?1", params![f.meeting,marker.repeat(500)]).unwrap();
        let settings = saved_settings(&db);
        drop(db);
        Self {
            f,
            outside,
            operation: uuid::Uuid::new_v4().to_string(),
            file,
            other_file,
            marker,
            settings,
        }
    }
    fn files(&self) -> removal::Receipt {
        let c = Census::capture(
            &self.f.storage,
            &self.f.meeting,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let review = removal::Reviewed::confirm(&c, false).unwrap();
        let p = intent::prepare(
            &self.f.storage,
            &self.operation,
            &c,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        removal::start(
            &self.f.storage,
            &p,
            &review,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn begin(&self) -> Receipt {
        let mut files = self.files();
        removal::run(
            &self.f.storage,
            &mut files,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        start(
            &self.f.storage,
            &files,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn unaffected(&self) {
        assert_eq!(
            std::fs::read(&self.other_file).unwrap(),
            b"KEEP OTHER AUDIO"
        );
        assert_eq!(
            std::fs::read(self.outside.path().join("export.md")).unwrap(),
            b"KEEP EXTERNAL EXPORT"
        );
        assert_eq!(
            std::fs::read(self.f.storage.root().join("models/model.gguf")).unwrap(),
            b"KEEP MODEL"
        );
        assert!(self.f.storage.get_meeting(&self.f.other).unwrap().is_some());
        let db = self.f.storage.connection.lock().unwrap();
        assert_eq!(saved_settings(&db), self.settings);
        assert_eq!(
            db.query_row("SELECT count(*) FROM model_packages", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM vocabulary_entries WHERE scope_id='global'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT term FROM vocabulary_entries WHERE id='global-term' AND scope_id='global'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
            "点検日"
        );
    }
}
#[test]
fn compound_deletion_requires_files_then_purges_fixed_rows_replays_and_refuses_recreated_content() {
    let f = Fixture::new();
    let mut files = f.files();
    assert!(start(&f.f.storage, &files, &crate::test_support::never_cancelled).is_err());
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"DELETE TARGET AUDIO");
    removal::run(
        &f.f.storage,
        &mut files,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut r = start(&f.f.storage, &files, &crate::test_support::never_cancelled).unwrap();
    let replay = start(&f.f.storage, &files, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(r.progress, replay.progress);
    let db = f.f.storage.connection.lock().unwrap();
    let original:(String,String,i64,i64)=db.query_row("SELECT target_manifest_json,target_hash,consent_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",params![f.operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let other =
        database::Scope::capture(&db, &f.f.other, "", &crate::test_support::never_cancelled)
            .unwrap();
    drop(db);
    run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).unwrap();
    assert!(r.completed());
    assert!(!f.file.exists());
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    let db = f.f.storage.connection.lock().unwrap();
    let after:(String,String,i64,i64)=db.query_row("SELECT target_manifest_json,target_hash,consent_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",params![f.operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(original, after);
    assert!(!original.0.contains(&f.marker));
    assert_eq!(
        other,
        database::Scope::capture(&db, &f.f.other, "", &crate::test_support::never_cancelled)
            .unwrap()
    );
    assert_eq!(
        db.query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    for name in ["meetings.sqlite3", "meetings.sqlite3-wal"] {
        let p = f.f.storage.root().join(name);
        if p.exists() {
            assert!(!std::fs::read(p)
                .unwrap()
                .windows(f.marker.len())
                .any(|w| w == f.marker.as_bytes()));
        }
    }
    assert_free_pages_zero(&std::fs::read(f.f.storage.root().join("meetings.sqlite3")).unwrap());
    let replay = start(&f.f.storage, &files, &crate::test_support::never_cancelled).unwrap();
    assert!(replay.completed());
    let mut replay = read(
        &f.f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    run(
        &f.f.storage,
        &mut replay,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(!f.f.storage.data_write_blocked());
    f.unaffected();
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"KEEP NEW CONTENT").unwrap();
    assert!(run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP NEW CONTENT");
    f.unaffected();
}
#[test]
fn compound_deletion_cancel_before_row_commit_rolls_back_rows_and_progress_together() {
    let f = Fixture::new();
    let mut r = f.begin();
    let original = r.progress_json().unwrap();
    for phase in ["rowsErased", "beforeRowsCommit"] {
        let stop = Cell::new(false);
        assert!(
            run_inner(&f.f.storage, &mut r, &|| stop.get(), &|p| if p == phase {
                stop.set(true)
            })
            .is_err()
        );
        assert!(stop.get(), "{phase}");
        assert_eq!(r.progress.database_phase, Phase::RowsPending);
        assert_eq!(r.progress_json().unwrap(), original);
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
        assert!(!f.file.exists());
        let saved = read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(saved.progress.database_phase, Phase::RowsPending);
        assert_eq!(saved.progress_json().unwrap(), original);
        assert!(f.f.storage.data_write_blocked());
        f.unaffected();
    }
    let mut saved = read(
        &f.f.storage,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    run(
        &f.f.storage,
        &mut saved,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    f.unaffected();
}
#[test]
fn compound_deletion_after_row_commit_cancel_retains_partial_state_and_finishes_without_redeleting()
{
    for phase in [
        "rowsCommitted",
        "checkpointed",
        "vacuumed",
        "purged",
        "beforeCompletionCommit",
        "completed",
    ] {
        let f = Fixture::new();
        let mut r = f.begin();
        let stop = Cell::new(false);
        assert!(
            run_inner(&f.f.storage, &mut r, &|| stop.get(), &|p| if p == phase {
                stop.set(true)
            })
            .is_err()
        );
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
        let mut saved = read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_ne!(saved.progress.database_phase, Phase::RowsPending);
        run(
            &f.f.storage,
            &mut saved,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(saved.completed());
        f.unaffected();
    }
}
#[test]
fn compound_deletion_refuses_changed_database_scope_and_recreated_file_tree_before_rows() {
    for change in ["scope", "files"] {
        let f = Fixture::new();
        let mut r = f.begin();
        if change == "scope" {
            let db = f.f.storage.connection.lock().unwrap();
            let _writer = super::super::fence::write_completion(
                &f.f.storage,
                &db,
                &r,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            db.execute(
                "UPDATE meetings SET title='KEEP CHANGED' WHERE id=?1",
                params![f.f.meeting],
            )
            .unwrap();
        } else {
            std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
            std::fs::write(&f.file, b"KEEP RECREATED").unwrap();
        }
        assert!(run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
        assert_eq!(r.progress.database_phase, Phase::RowsPending);
    }
}
#[test]
fn compound_deletion_rejects_corrupt_phase_file_proof_and_foreign_progress() {
    for change in ["version", "phase", "files", "unknown"] {
        let f = Fixture::new();
        let r = f.begin();
        let db = f.f.storage.connection.lock().unwrap();
        let _writer = super::super::fence::write_completion(
            &f.f.storage,
            &db,
            &r,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let mut json = serde_json::to_value(&r.progress).unwrap();
        match change {
            "version" => json["version"] = 2.into(),
            "phase" => json["databasePhase"] = "rowsDeleted".into(),
            "files" => json["files"]["filePhase"] = "removing".into(),
            "unknown" => json["unexpected"] = true.into(),
            _ => unreachable!(),
        }
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![f.operation, serde_json::to_string(&json).unwrap()],
        )
        .unwrap();
        drop(_writer);
        drop(db);
        assert!(read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
    }
}
#[test]
fn compound_deletion_foreign_connection_and_other_pending_operation_cannot_mutate_rows() {
    let f = Fixture::new();
    let mut r = f.begin();
    assert_matching_copy_is_not_a_writer(&f, &r);
    let db = f.f.storage.connection.lock().unwrap();
    let _writer = super::super::fence::write_completion(
        &f.f.storage,
        &db,
        &r,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','attention','{}',?3,'{}',1,1)",params![uuid::Uuid::new_v4().to_string(),f.f.other,"0".repeat(64)]).unwrap();
    drop(_writer);
    drop(db);
    assert!(run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).is_err());
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
    let f = Fixture::new();
    let mut r = f.begin();
    run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).unwrap();
    assert_matching_copy_is_not_a_writer(&f, &r);
}
#[test]
fn compound_deletion_blocked_wal_checkpoint_retains_cleanup_then_retries_after_reader_releases() {
    let f = Fixture::new();
    let mut r = f.begin();
    let db = f.f.storage.connection.lock().unwrap();
    let mut statement = db.prepare("SELECT id FROM model_packages").unwrap();
    let mut rows = statement.query([]).unwrap();
    assert!(rows.next().unwrap().is_some());
    assert!(run_locked(
        &f.f.storage,
        &db,
        &mut r,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
    assert_eq!(r.progress.database_phase, Phase::RowsDeleted);
    drop(rows);
    drop(statement);
    drop(db);
    assert!(f.f.storage.data_write_blocked());
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
    run(&f.f.storage, &mut r, &crate::test_support::never_cancelled).unwrap();
    assert!(r.completed());
    f.unaffected();
}
#[test]
fn compound_deletion_rejects_claimed_row_completion_when_actual_rows_still_exist() {
    for phase in [Phase::RowsDeleted, Phase::Finished] {
        let f = Fixture::new();
        let r = f.begin();
        let db = f.f.storage.connection.lock().unwrap();
        let _writer = super::super::fence::write_completion(
            &f.f.storage,
            &db,
            &r,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let mut progress = r.progress.clone();
        progress.database_phase = phase;
        let state = if progress.database_phase == Phase::Finished {
            "completed"
        } else {
            "cleanup"
        };
        db.execute("UPDATE maintenance_operations SET state=?2,meeting_id=NULL,progress_json=?3 WHERE id=?1",params![f.operation,state,progress.encode().unwrap()]).unwrap();
        drop(_writer);
        drop(db);
        let mut forged = read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(run(
            &f.f.storage,
            &mut forged,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
    }
}
#[test]
fn compound_deletion_unknown_at_row_commit_boundary_rolls_back_rows_and_keeps_new_content() {
    let f = Fixture::new();
    let mut r = f.begin();
    let original = database::Scope::capture(
        &f.f.storage.connection.lock().unwrap(),
        &f.f.meeting,
        &f.operation,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(run_inner(
        &f.f.storage,
        &mut r,
        &crate::test_support::never_cancelled,
        &|point| if point == "beforeRowsCommit" {
            std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
            std::fs::write(&f.file, b"KEEP NEW CONTENT BEFORE ROW COMMIT").unwrap();
        }
    )
    .is_err());
    assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_some());
    assert_eq!(
        original,
        database::Scope::capture(
            &f.f.storage.connection.lock().unwrap(),
            &f.f.meeting,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    );
    assert_eq!(r.progress.database_phase, Phase::RowsPending);
    assert_eq!(
        read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .progress
        .database_phase,
        Phase::RowsPending
    );
    assert_eq!(
        std::fs::read(&f.file).unwrap(),
        b"KEEP NEW CONTENT BEFORE ROW COMMIT"
    );
    assert!(f.f.storage.data_write_blocked());
    f.unaffected();
}
#[test]
fn compound_deletion_reappearing_unknown_after_row_commit_is_kept_and_never_acknowledged() {
    for phase in [
        "rowsCommitted",
        "checkpointed",
        "vacuumed",
        "purged",
        "beforeCompletionCommit",
        "completed",
    ] {
        let f = Fixture::new();
        let mut r = f.begin();
        assert!(run_inner(
            &f.f.storage,
            &mut r,
            &crate::test_support::never_cancelled,
            &|point| if point == phase {
                std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
                std::fs::write(&f.file, b"KEEP NEW UNKNOWN").unwrap();
            }
        )
        .is_err());
        assert!(f.f.storage.get_meeting(&f.f.meeting).unwrap().is_none());
        assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP NEW UNKNOWN");
        let mut saved = read(
            &f.f.storage,
            &f.operation,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(run(
            &f.f.storage,
            &mut saved,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP NEW UNKNOWN");
        f.unaffected();
    }
}
#[test]
fn compound_deletion_database_replacement_stops_before_commit_or_next_purge_operation() {
    for phase in ["beforeRowsCommit", "checkpointed", "vacuumed", "completed"] {
        let f = Fixture::new();
        let mut r = f.begin();
        let path = f.f.storage.root().join("meetings.sqlite3");
        let kept = f.outside.path().join("kept.sqlite3");
        assert!(run_inner(
            &f.f.storage,
            &mut r,
            &crate::test_support::never_cancelled,
            &|point| if point == phase {
                std::fs::rename(&path, &kept).unwrap();
                std::fs::write(&path, b"KEEP REPLACEMENT WITHOUT SQLITE INIT").unwrap();
            }
        )
        .is_err());
        assert!(kept.is_file());
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"KEEP REPLACEMENT WITHOUT SQLITE INIT"
        );
        assert!(f.f.storage.data_write_blocked());
        assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
        assert_eq!(
            std::fs::read(f.outside.path().join("export.md")).unwrap(),
            b"KEEP EXTERNAL EXPORT"
        );
    }
}
#[test]
#[ignore = "subprocess fixture; invoked by compound deletion SIGKILL parent"]
fn compound_deletion_checkpoint_child_fixture() {
    let base = PathBuf::from(std::env::var("MON_COMPOUND_TEST_BASE").unwrap());
    let phase = std::env::var("MON_COMPOUND_TEST_PHASE").unwrap();
    let f = Fixture::at(
        tempfile::tempdir_in(&base).unwrap(),
        tempfile::tempdir_in(&base).unwrap(),
    );
    let mut r = f.begin();
    run_inner(
        &f.f.storage,
        &mut r,
        &crate::test_support::never_cancelled,
        &|point| {
            if point == phase {
                let receipt = serde_json::to_vec(&(
                    f.f.storage.root(),
                    &f.operation,
                    &f.f.meeting,
                    &f.f.other,
                    &f.other_file,
                    f.outside.path().join("export.md"),
                    &f.marker,
                ))
                .unwrap();
                std::fs::write(base.join("ready.tmp"), receipt).unwrap();
                std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        },
    )
    .unwrap();
    panic!("compound checkpoint not reached");
}
#[test]
fn compound_deletion_sigkill_at_eight_database_boundaries_reopens_and_finishes_all_phases() {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "rowsErased",
        "beforeRowsCommit",
        "rowsCommitted",
        "checkpointed",
        "vacuumed",
        "purged",
        "beforeCompletionCommit",
        "completed",
    ] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::completion::tests::compound_deletion_checkpoint_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_COMPOUND_TEST_BASE", base.path())
            .env("MON_COMPOUND_TEST_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let ready = base.path().join("ready.json");
        let started = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || started.elapsed() > std::time::Duration::from_secs(30)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("compound checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (root, operation, meeting, other, other_file, external, marker): (
            PathBuf,
            String,
            String,
            String,
            PathBuf,
            PathBuf,
            String,
        ) = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        assert!(root
            .canonicalize()
            .unwrap()
            .starts_with(base.path().canonicalize().unwrap()));
        let storage = Storage::open(root.clone()).unwrap();
        let mut receipt =
            read(&storage, &operation, &crate::test_support::never_cancelled).unwrap();
        let pending_at_open = !receipt.completed();
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().is_some(),
            matches!(phase, "rowsErased" | "beforeRowsCommit")
        );
        run(
            &storage,
            &mut receipt,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(receipt.completed());
        assert!(storage.get_meeting(&meeting).unwrap().is_none());
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert_eq!(std::fs::read(other_file).unwrap(), b"KEEP OTHER AUDIO");
        assert_eq!(std::fs::read(external).unwrap(), b"KEEP EXTERNAL EXPORT");
        assert_eq!(
            std::fs::read(root.join("models/model.gguf")).unwrap(),
            b"KEEP MODEL"
        );
        assert_eq!(storage.data_write_blocked(), pending_at_open);
        for name in ["meetings.sqlite3", "meetings.sqlite3-wal"] {
            let file = root.join(name);
            if file.exists() {
                assert!(!std::fs::read(file)
                    .unwrap()
                    .windows(marker.len())
                    .any(|w| w == marker.as_bytes()));
            }
        }
        drop(storage);
        let storage = Storage::open(root).unwrap();
        assert!(!storage.data_write_blocked());
        let mut receipt =
            read(&storage, &operation, &crate::test_support::never_cancelled).unwrap();
        run(
            &storage,
            &mut receipt,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert!(receipt.completed());
    }
}
