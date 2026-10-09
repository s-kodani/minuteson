//! Synthetic completed receipts; no recording, models or real user files.
use super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::deletion::{files::Census, intent, operations},
};
use rusqlite::config::DbConfig;
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    base: tempfile::TempDir,
    storage: Option<Storage>,
    meeting: String,
    other: String,
    file: PathBuf,
    keep: PathBuf,
    operation: String,
    reserved: PathBuf,
}
fn delete(storage: &Storage, meeting: &str) -> String {
    let analysis = AnalysisController::default();
    let admission = analysis
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut p = operations::Prepared::fresh(storage, meeting, &|| false).unwrap();
    let preview = p.preview();
    let done = p
        .execute(
            storage,
            &admission,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted);
    preview.operation_id
}
impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("DELETE").unwrap().id;
        let other = storage.create_meeting("KEEP").unwrap().id;
        let file = storage
            .root()
            .join(format!("meetings/{meeting}/audio/input.flac"));
        let keep = storage
            .root()
            .join(format!("meetings/{other}/audio/input.flac"));
        for (id, path, bytes) in [
            (&meeting, &file, b"DELETE".as_slice()),
            (&other, &keep, b"KEEP".as_slice()),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
            storage
                .register_ready_meeting_asset(
                    id,
                    path.strip_prefix(storage.root()).unwrap(),
                    "audio/flac",
                )
                .unwrap();
        }
        let operation = delete(&storage, &meeting);
        let reserved = storage
            .root()
            .join(format!("meetings/.mon-delete-{operation}-2"));
        assert!(!reserved.exists());
        Self {
            base,
            storage: Some(storage),
            meeting,
            other,
            file,
            keep,
            operation,
            reserved,
        }
    }
    fn storage(&self) -> &Storage {
        self.storage.as_ref().unwrap()
    }
    fn cold(&mut self) {
        let root = self.storage().root().to_owned();
        drop(self.storage.take());
        self.storage = Some(Storage::open(root).unwrap());
    }
    fn reappear(&self, reserved: bool) {
        let path = if reserved {
            self.reserved.join("late")
        } else {
            self.file.clone()
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"REAPPEARED UNKNOWN").unwrap();
    }
    fn kept(&self) {
        assert_eq!(std::fs::read(&self.keep).unwrap(), b"KEEP");
        assert!(self.storage().get_meeting(&self.meeting).unwrap().is_none());
        assert!(self.storage().get_meeting(&self.other).unwrap().is_some());
    }
}
fn evidence(storage: &Storage) -> String {
    let db = storage.connection.lock().unwrap();
    evidence_database(&db)
}
fn evidence_database(db: &Connection) -> String {
    db.query_row("SELECT json_group_array(json_object('id',id,'meeting',meeting_id,'kind',kind,'state',state,'intent',target_manifest_json,'hash',target_hash,'consent',consent_at,'progress',progress_json,'error',error_code,'created',created_at,'updated',updated_at)) FROM (SELECT * FROM maintenance_operations ORDER BY id)",[],|r|r.get(0)).unwrap()
}

#[test]
fn history_v27_completed_meeting_upgrades_without_rewriting_consent_or_fencing_clean_data() {
    let mut f = Fixture::new();
    {
        let db = f.storage().connection.lock().unwrap();
        intent::historical_completed_fixture(&db, &f.operation);
        crate::storage::edit_recovery::remove_schema_for_test(&db);
        assert_eq!(crate::storage::verify_migration_history(&db).unwrap(), 27);
    }
    let saved = evidence(f.storage());
    f.cold();
    assert_eq!(evidence(f.storage()), saved);
    assert!(!f.storage().data_write_blocked());
    assert_eq!(
        super::reason(&f.storage().connection.lock().unwrap(), &f.operation).unwrap(),
        None
    );
    assert_eq!(
        crate::storage::verify_migration_history(&f.storage().connection.lock().unwrap()).unwrap(),
        crate::storage::MIGRATIONS.len()
    );
    f.kept();
    f.storage()
        .create_meeting("AFTER VERIFIED UPGRADE")
        .unwrap();
    f.cold();
    assert_eq!(evidence(f.storage()), saved);
    assert!(!f.storage().data_write_blocked());
}

#[test]
fn history_v27_reappearance_preserves_old_database_and_never_migrates_under_old_permission() {
    let mut f = Fixture::new();
    {
        let db = f.storage().connection.lock().unwrap();
        intent::historical_completed_fixture(&db, &f.operation);
        crate::storage::edit_recovery::remove_schema_for_test(&db);
    }
    let root = f.storage().root().to_owned();
    let saved = evidence(f.storage());
    f.reappear(false);
    drop(f.storage.take());
    let error = Storage::open(root.clone()).err().unwrap().to_string();
    assert!(error.contains("未完了の削除があるDBは移行せず保持"));
    assert_eq!(std::fs::read(&f.file).unwrap(), b"REAPPEARED UNKNOWN");
    assert_eq!(std::fs::read(&f.keep).unwrap(), b"KEEP");
    let db = Connection::open_with_flags(
        root.join("meetings.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(evidence_database(&db), saved);
    assert_eq!(crate::storage::verify_migration_history(&db).unwrap(), 27);
    assert!(!db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='transcript_edit_recovery')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}
fn finish_pending(f: &Fixture) {
    finish_operation(f.storage(), &f.operation);
}
fn finish_operation(storage: &Storage, operation: &str) {
    let analysis = AnalysisController::default();
    let admission = analysis
        .begin_deletion_maintenance(storage, operation, false, false, &|| false)
        .unwrap();
    let mut p = operations::Prepared::pending(storage, operation, &|| false).unwrap();
    assert!(p.preview().reconfirmation_allowed && !p.preview().execute_allowed);
    p.review_remaining(storage, &admission, &p.preview().confirmation_id, &|| false)
        .unwrap();
    let preview = p.preview();
    assert!(preview.row_phase_committed && preview.unregistered_count > 0);
    let done = p
        .execute(
            storage,
            &admission,
            &preview.confirmation_id,
            true,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(done.deleted);
    assert_eq!(done.restart_required, storage.restart_required());
}

#[test]
fn deletion_startup_canonical_file_completion_keeps_existing_pending_fence_without_invalid_hint() {
    let mut f = Fixture::new();
    let second = f.storage().create_meeting("SECOND").unwrap().id;
    let second_operation = delete(f.storage(), &second);
    let second_root = f.storage().root().join(format!("meetings/{second}"));
    std::fs::create_dir_all(&second_root).unwrap();
    std::fs::write(second_root.join("late"), b"SECOND UNKNOWN").unwrap();
    f.storage().connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale',?1,'transcription','running','stale',?2,'{}','{}',?2,1,1)",params![f.other,"a".repeat(64)]).unwrap();
    f.reappear(true);
    f.cold();
    let controller = AnalysisController::default();
    let admission = controller
        .begin_deletion_maintenance(f.storage(), &f.operation, false, false, &|| false)
        .unwrap();
    let receipt = completion::read(f.storage(), &f.operation, &|| false).unwrap();
    let review = renewal::Review::capture_database(f.storage(), &receipt, &|| false).unwrap();
    let mut files =
        renewal::accept_database(f.storage(), &receipt, &review, true, &admission, &|| false)
            .unwrap();
    removal::run(f.storage(), &mut files, &|| false).unwrap();
    drop(admission);
    let before = evidence(f.storage());
    f.cold();
    assert!(f.storage().data_write_blocked());
    let listing = operations::listing(f.storage()).unwrap();
    assert_eq!(listing.items.len(), 2);
    let own = listing
        .items
        .iter()
        .find(|r| r.operation_id == f.operation)
        .unwrap();
    assert_eq!(own.state, "completed");
    assert!(own.error_code.is_none());
    assert_eq!(evidence(f.storage()), before);
    assert!(!contains(&f.storage().connection.lock().unwrap(), &f.operation).unwrap());
    let mut p = operations::Prepared::pending(f.storage(), &f.operation, &|| false).unwrap();
    let preview = p.preview();
    assert!(preview.execute_allowed && !preview.reconfirmation_allowed);
    let admission = controller
        .begin_deletion_maintenance(f.storage(), &f.operation, false, false, &|| false)
        .unwrap();
    assert!(
        p.execute(
            f.storage(),
            &admission,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {}
        )
        .unwrap()
        .deleted
    );
    assert!(!f.storage().restart_required());
    let listing = operations::listing(f.storage()).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].operation_id, second_operation);
    assert_eq!(
        std::fs::read(second_root.join("late")).unwrap(),
        b"SECOND UNKNOWN"
    );
    assert!(f.storage().create_meeting("DENIED").is_err());
    drop(admission);
    finish_operation(f.storage(), &second_operation);
    assert!(f.storage().restart_required());
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    f.cold();
    assert!(!f.storage().data_write_blocked());
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "interrupted"
    );
    f.kept();
}

#[test]
fn deletion_startup_clean_completed_history_and_unrelated_stale_jobs_allow_ordinary_recovery() {
    let mut f = Fixture::new();
    let before = evidence(f.storage());
    f.storage().connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale',?1,'transcription','running','stale',?2,'{}','{}',?2,1,1)",params![f.other,"a".repeat(64)]).unwrap();
    f.cold();
    assert!(!f.storage().data_write_blocked());
    assert!(operations::listing(f.storage()).unwrap().items.is_empty());
    assert_eq!(evidence(f.storage()), before);
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "interrupted"
    );
    f.storage().create_meeting("ALLOWED").unwrap();
    f.kept();
}

#[test]
fn deletion_startup_reappeared_completed_logical_or_reserved_tree_requires_explicit_review_and_restart(
) {
    for reserved in [false, true] {
        let mut f = Fixture::new();
        let before = evidence(f.storage());
        f.reappear(reserved);
        f.cold();
        assert!(f.storage().data_write_blocked() && !f.storage().restart_required());
        assert!(f.storage().scratch.is_none());
        assert!(f.storage().create_meeting("DENIED").is_err());
        let listing = operations::listing(f.storage()).unwrap();
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].state, "completed");
        assert_eq!(listing.items[0].error_code.as_deref(), Some(REAPPEARED));
        assert!(listing.items[0].meeting_id.is_none() && listing.deleting_meeting_ids.is_empty());
        assert_eq!(evidence(f.storage()), before);
        f.kept();
        finish_pending(&f);
        assert!(operations::listing(f.storage()).unwrap().items.is_empty());
        assert!(f.storage().data_write_blocked() && f.storage().restart_required());
        assert!(!f.file.exists() && !f.reserved.exists());
        f.cold();
        assert!(!f.storage().data_write_blocked());
        f.kept();
    }
}

#[test]
fn deletion_startup_attention_is_memory_only_and_cold_reopen_rediscovers_without_touching_receipts()
{
    let mut f = Fixture::new();
    f.reappear(true);
    let before = evidence(f.storage());
    for _ in 0..2 {
        f.cold();
        assert_eq!(evidence(f.storage()), before);
        let db = f.storage().connection.lock().unwrap();
        assert!(contains(&db, &f.operation).unwrap());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='mon_deletion_startup_attention'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM temp.mon_deletion_startup_attention",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(db
            .execute("DELETE FROM temp.mon_deletion_startup_attention", [])
            .is_err());
        assert!(db
            .execute(
                "UPDATE maintenance_operations SET error_code='silenced'",
                []
            )
            .is_err());
    }
    assert_eq!(
        std::fs::read(f.reserved.join("late")).unwrap(),
        b"REAPPEARED UNKNOWN"
    );
    f.kept();
}

#[test]
fn deletion_startup_corrupt_hash_progress_or_forged_audit_marker_is_attention_not_success() {
    for change in ["hash", "progress", "marker"] {
        let mut f = Fixture::new();
        f.reappear(false);
        {
            let db = f.storage().connection.lock().unwrap();
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                .unwrap();
            match change {
                "hash" => {
                    db.execute(
                        "UPDATE maintenance_operations SET target_hash=?2 WHERE id=?1",
                        params![f.operation, "0".repeat(64)],
                    )
                    .unwrap();
                }
                "progress" => {
                    db.execute(
                        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                        params![f.operation],
                    )
                    .unwrap();
                }
                _ => {
                    db.execute("UPDATE maintenance_operations SET progress_json='{\"confirmationRecorded\":true}' WHERE id=?1",params![f.operation]).unwrap();
                }
            }
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
                .unwrap();
        }
        let before = evidence(f.storage());
        f.cold();
        assert!(f.storage().data_write_blocked());
        let listing = operations::listing(f.storage()).unwrap();
        assert_eq!(listing.items[0].error_code.as_deref(), Some(INVALID));
        assert!(operations::authorize_review(f.storage(), Some(&f.operation)).is_err());
        assert_eq!(evidence(f.storage()), before);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"REAPPEARED UNKNOWN");
        f.kept();
    }
}

#[test]
fn deletion_startup_recreated_deleted_database_rows_are_retained_without_adopting_ownership() {
    let mut f = Fixture::new();
    f.storage().connection.lock().unwrap().execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,'RECREATED','draft',1,1)",params![f.meeting]).unwrap();
    let before = evidence(f.storage());
    f.cold();
    assert_eq!(
        operations::listing(f.storage()).unwrap().items[0]
            .error_code
            .as_deref(),
        Some(INVALID)
    );
    assert!(f.storage().get_meeting(&f.meeting).unwrap().is_some());
    let p = operations::Prepared::pending(f.storage(), &f.operation, &|| false).unwrap();
    assert!(!p.preview().execute_allowed && !p.preview().reconfirmation_allowed);
    assert_eq!(evidence(f.storage()), before);
}

#[test]
fn deletion_startup_exact_sibling_regular_files_links_and_neighbor_names_are_not_adopted() {
    for link in [false, true] {
        let mut f = Fixture::new();
        if link {
            std::os::unix::fs::symlink(&f.keep, &f.reserved).unwrap();
        } else {
            std::fs::write(&f.reserved, b"FOREIGN REGULAR ROOT").unwrap();
        }
        f.cold();
        assert!(f.storage().data_write_blocked());
        assert_eq!(
            operations::listing(f.storage()).unwrap().items[0]
                .error_code
                .as_deref(),
            Some(if link { UNINSPECTABLE } else { REAPPEARED })
        );
        let p = operations::Prepared::pending(f.storage(), &f.operation, &|| false).unwrap();
        assert!(!p.preview().execute_allowed && !p.preview().reconfirmation_allowed);
        assert!(std::fs::symlink_metadata(&f.reserved).is_ok());
        f.kept();
    }
    let mut f = Fixture::new();
    let neighbor = f
        .reserved
        .with_file_name(format!(".mon-delete-{}-200", f.operation));
    std::fs::write(&neighbor, b"NEIGHBOR").unwrap();
    f.cold();
    assert!(!f.storage().data_write_blocked());
    assert_eq!(std::fs::read(neighbor).unwrap(), b"NEIGHBOR");
}

#[test]
fn deletion_startup_completed_audits_are_verified_against_the_active_source_not_ignored_by_marker()
{
    let mut f = Fixture::new();
    f.reappear(false);
    let source = completion::read(f.storage(), &f.operation, &|| false).unwrap();
    let review = renewal::Review::capture_database(f.storage(), &source, &|| false).unwrap();
    let analysis = AnalysisController::default();
    let admission = analysis
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut files =
        renewal::accept_database(f.storage(), &source, &review, true, &admission, &|| false)
            .unwrap();
    let audit = files
        .progress
        .confirmation
        .as_ref()
        .unwrap()
        .operation_id
        .clone();
    removal::run(f.storage(), &mut files, &|| false).unwrap();
    let mut complete = completion::start(f.storage(), &files, &|| false).unwrap();
    completion::run(f.storage(), &mut complete, &|| false).unwrap();
    f.cold();
    assert!(operations::listing(f.storage()).unwrap().items.is_empty());
    assert!(!f.storage().data_write_blocked());
    // Same canonical audit payload under a new, unlinked ID is not authority.
    let orphan = uuid::Uuid::new_v4().to_string();
    f.storage().connection.lock().unwrap().execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) SELECT ?2,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at FROM maintenance_operations WHERE id=?1",params![audit,orphan]).unwrap();
    f.cold();
    let listing = operations::listing(f.storage()).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].operation_id, orphan);
    assert_eq!(listing.items[0].error_code.as_deref(), Some(INVALID));
    assert!(operations::authorize_review(f.storage(), Some(&orphan)).is_err());
    f.kept();
}

#[test]
fn deletion_startup_several_completed_reappearances_can_be_resolved_independently_without_unlocking(
) {
    let mut f = Fixture::new();
    let third = f.storage().create_meeting("THIRD").unwrap().id;
    let third_operation = delete(f.storage(), &third);
    let third_root = f.storage().root().join(format!("meetings/{third}"));
    std::fs::create_dir(&third_root).unwrap();
    std::fs::write(third_root.join("late"), b"THIRD UNKNOWN").unwrap();
    f.reappear(true);
    f.cold();
    assert_eq!(operations::listing(f.storage()).unwrap().items.len(), 2);
    finish_pending(&f);
    let listing = operations::listing(f.storage()).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].operation_id, third_operation);
    assert!(f.storage().data_write_blocked() && !f.storage().restart_required());
    assert_eq!(
        std::fs::read(third_root.join("late")).unwrap(),
        b"THIRD UNKNOWN"
    );
    f.kept();
}

#[test]
fn deletion_startup_final_ack_reappearance_keeps_attention_until_a_new_review() {
    let mut f = Fixture::new();
    f.reappear(true);
    f.cold();
    // The object disappeared externally; acknowledgement still uses a fully
    // validated receipt, and checks again after the final checkpoint event.
    std::fs::remove_file(f.reserved.join("late")).unwrap();
    std::fs::remove_dir(&f.reserved).unwrap();
    let mut receipt = completion::read(f.storage(), &f.operation, &|| false).unwrap();
    let hit = Cell::new(false);
    assert!(
        completion::run_observed(f.storage(), &mut receipt, &|| false, &|event| {
            if event == "ackCheckpointed" {
                hit.set(true);
                f.reappear(true);
            }
        })
        .is_err()
    );
    assert!(hit.get());
    assert_eq!(
        operations::listing(f.storage()).unwrap().items[0]
            .error_code
            .as_deref(),
        Some(REAPPEARED)
    );
    assert!(!f.storage().restart_required());
    finish_pending(&f);
    f.kept();
}

#[test]
fn deletion_startup_unowned_copied_history_stays_fenced_without_adopting_original_identity() {
    let mut f = Fixture::new();
    let before = evidence(f.storage());
    let root = f.storage().root().to_owned();
    drop(f.storage.take());
    let copy = f.base.path().join("copy");
    std::fs::create_dir(&copy).unwrap();
    std::fs::copy(root.join("meetings.sqlite3"), copy.join("meetings.sqlite3")).unwrap();
    let storage = Storage::open(copy.clone()).unwrap();
    assert!(storage.data_write_blocked());
    assert_eq!(evidence(&storage), before);
    assert_eq!(
        operations::listing(&storage).unwrap().items[0]
            .error_code
            .as_deref(),
        Some(INVALID)
    );
    assert!(operations::authorize_review(&storage, Some(&f.operation)).is_err());
    assert!(storage.create_meeting("NO UNOWNED COPY ADMISSION").is_err());
    drop(storage);
    let path = copy.join(format!("meetings/{}/late", f.meeting));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"COPIED NEW DATA").unwrap();
    let storage = Storage::open(copy).unwrap();
    assert_eq!(
        operations::listing(&storage).unwrap().items[0]
            .error_code
            .as_deref(),
        Some(INVALID)
    );
    assert!(storage.data_write_blocked());
    assert_eq!(evidence(&storage), before);
    assert!(operations::authorize_review(&storage, Some(&f.operation)).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"COPIED NEW DATA");
}

#[test]
fn deletion_startup_old_v1_scope_switches_recover_historical_reserved_names_after_completion() {
    let mut f = Fixture::new();
    let storage = f.storage();
    let meeting = storage.create_meeting("LEGACY").unwrap().id;
    let file = storage
        .root()
        .join(format!("meetings/{meeting}/audio/input.flac"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"LEGACY SOURCE").unwrap();
    storage
        .register_ready_meeting_asset(
            &meeting,
            file.strip_prefix(storage.root()).unwrap(),
            "audio/flac",
        )
        .unwrap();
    let census = Census::capture(storage, &meeting, &|| false).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let prepared = intent::prepare(storage, &operation, &census, &|| false).unwrap();
    let reviewed = removal::Reviewed::confirm(&census, false).unwrap();
    let mut files = removal::start(storage, &prepared, &reviewed, &|| false).unwrap();
    let reserved = storage
        .root()
        .join(format!("meetings/.mon-delete-{operation}-2"));
    let stop = Cell::new(false);
    assert!(
        removal::run_observed(storage, &mut files, &|| stop.get(), &|event| {
            if event == "staged" && reserved.exists() {
                stop.set(true);
            }
        })
        .is_err()
    );
    assert!(stop.get());
    std::fs::remove_dir(&reserved).unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"NEW LEGACY TREE").unwrap();
    files = renewal::seed_historical_record_for_test(storage, &files, 1, &[]).unwrap();
    removal::run(storage, &mut files, &|| false).unwrap();
    let mut complete = completion::start(storage, &files, &|| false).unwrap();
    completion::run(storage, &mut complete, &|| false).unwrap();
    f.operation = operation;
    f.meeting = meeting;
    f.file = file;
    f.reserved = reserved;
    f.reappear(true);
    let before = evidence(f.storage());
    f.cold();
    let listing = operations::listing(f.storage()).unwrap();
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].operation_id, f.operation);
    assert_eq!(listing.items[0].error_code.as_deref(), Some(REAPPEARED));
    assert_eq!(evidence(f.storage()), before);
    finish_pending(&f);
    f.kept();
}

#[test]
fn deletion_startup_attention_schema_does_not_admit_shadow_tables_or_same_name_substitutions() {
    for sql in [
        "CREATE TEMP TABLE meetings(id TEXT)",
        "CREATE TEMP TABLE mon_deletion_startup_attention(operation_id TEXT)",
    ] {
        let f = Fixture::new();
        let db = f.storage().connection.lock().unwrap();
        db.execute_batch(sql).unwrap();
        assert!(verify_temp_schema(&db)
            .unwrap_err()
            .contains("一時オブジェクト"));
        drop(db);
        assert!(operations::Prepared::fresh(f.storage(), &f.other, &|| false).is_err());
        assert_eq!(std::fs::read(&f.keep).unwrap(), b"KEEP");
        // Read the real table explicitly; the hostile TEMP fixture shadows
        // ordinary unqualified reads by design and must remain unadopted.
        assert_eq!(
            f.storage()
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM main.meetings WHERE id=?1",
                    params![f.other],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}

#[test]
fn deletion_startup_reappearance_defers_mutable_recovery_and_never_relabels_unrelated_running_jobs()
{
    let mut f = Fixture::new();
    f.storage().connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale',?1,'transcription','running','stale',?2,'{}','{}',?2,1,1)",params![f.other,"a".repeat(64)]).unwrap();
    f.reappear(true);
    f.cold();
    assert_eq!(
        operations::listing(f.storage()).unwrap().items[0]
            .error_code
            .as_deref(),
        Some(REAPPEARED)
    );
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    assert!(f.storage().create_meeting("DENIED").is_err());
    let p = operations::Prepared::pending(f.storage(), &f.operation, &|| false).unwrap();
    assert!(!p.preview().execute_allowed && p.preview().reconfirmation_allowed);
    let before = evidence(f.storage());
    let controller = AnalysisController::default();
    for (recording, importing, cancelled) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert!(controller
            .begin_deletion_maintenance(f.storage(), &f.operation, recording, importing, &|| {
                cancelled
            })
            .is_err());
    }
    let export = controller.begin_export().unwrap();
    assert!(controller
        .begin_deletion_maintenance(f.storage(), &f.operation, false, false, &|| false)
        .is_err());
    drop(export);
    let other_work = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(controller
        .begin_deletion_maintenance(f.storage(), &f.operation, false, false, &|| false)
        .is_err());
    drop(other_work);
    assert_eq!(evidence(f.storage()), before);
    finish_pending(&f);
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    assert!(f.storage().restart_required());
    f.cold();
    assert!(!f.storage().data_write_blocked());
    assert_eq!(
        f.storage()
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "interrupted"
    );
    f.kept();
}
