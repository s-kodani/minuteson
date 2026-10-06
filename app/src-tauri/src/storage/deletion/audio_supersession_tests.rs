//! Owned synthetic files/SQLite, never microphones, user recordings or models.
use super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::{
        deletion::{audio, completion, files, intent, operations, removal},
        Storage,
    },
};
use rusqlite::config::DbConfig;
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    audio_operation: String,
    audio_record: String,
    paths: String,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("DELETE PRIVATE MEETING").unwrap().id;
        let other = storage.create_meeting("KEEP NEIGHBOR").unwrap().id;
        let directory = storage.root().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("synthetic.flac");
        std::fs::write(&path, b"SYNTHETIC PRIVATE AUDIO").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                path.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        std::fs::write(
            directory.join("keep-until-full-deletion.md"),
            b"PRIVATE TEXT",
        )
        .unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let audio_operation = uuid::Uuid::new_v4().to_string();
        audio::intent::prepare(
            &storage,
            &audio_operation,
            &audio::Census::capture(&storage, &meeting, &|| false).unwrap(),
            audio::intent::Policy {
                include_unregistered_audio: true,
                retain_unclassified: true,
            },
            &lease,
            &|| false,
        )
        .unwrap();
        audio::intent::removal::start(&storage, &audio_operation, &lease, &|| false).unwrap();
        audio::intent::removal::run(&storage, &audio_operation, &lease, &|| false).unwrap();
        audio::intent::removal::completion::finish(&storage, &audio_operation, &lease, &|| false)
            .unwrap();
        assert!(!path.exists());
        drop(lease);
        let db = storage.connection.lock().unwrap();
        let audio_record = record(&db, &audio_operation);
        let paths = path_records(&db, &audio_operation);
        db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'prune_revisions','recovered','{}',?3,'{}',0,0)", params![uuid::Uuid::new_v4().to_string(),meeting,"a".repeat(64)]).unwrap();
        drop(db);
        Self {
            base,
            storage,
            meeting,
            other,
            audio_operation,
            audio_record,
            paths,
            directory,
        }
    }
    fn begin(&self) -> completion::Receipt {
        let c = files::Census::capture(&self.storage, &self.meeting, &|| false).unwrap();
        let reviewed = removal::Reviewed::confirm(&c, true).unwrap();
        let p = intent::prepare(
            &self.storage,
            &uuid::Uuid::new_v4().to_string(),
            &c,
            &|| false,
        )
        .unwrap();
        let mut files = removal::start(&self.storage, &p, &reviewed, &|| false).unwrap();
        removal::run(&self.storage, &mut files, &|| false).unwrap();
        completion::start(&self.storage, &files, &|| false).unwrap()
    }
    fn finish(&self) -> String {
        let mut receipt = self.begin();
        completion::run(&self.storage, &mut receipt, &|| false).unwrap();
        self.marker()
    }
    fn marker(&self) -> String {
        self.storage.connection.lock().unwrap().query_row("SELECT id FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r|r.get(0)).unwrap()
    }
    fn reopen(&mut self) {
        let root = self.storage.root().to_owned();
        let temp = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut self.storage,
            Storage::open(temp.path().to_owned()).unwrap(),
        );
        drop(old);
        self.storage = Storage::open(root).unwrap();
    }
    fn unchanged_history(&self) {
        let db = self.storage.connection.lock().unwrap();
        assert_eq!(record(&db, &self.audio_operation), self.audio_record);
        assert_eq!(path_records(&db, &self.audio_operation), self.paths);
        assert_eq!(
            db.query_row(
                "SELECT meeting_id FROM maintenance_operations WHERE id=?1",
                [&self.audio_operation],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        // Historical admission never broadens the ordinary native reader.
        assert!(
            audio::intent::removal::Receipt::load(&db, &self.audio_operation, &|| false).is_err()
        );
    }
    fn edit_marker(&self, marker: &str, edit: impl FnOnce(&mut Header)) {
        let db = self.storage.connection.lock().unwrap();
        let json: String = db
            .query_row(
                "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                [marker],
                |r| r.get(0),
            )
            .unwrap();
        let mut header: Header = serde_json::from_str(&json).unwrap();
        edit(&mut header);
        let json = serde_json::to_string(&header).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1",
            params![marker, json, hash(&json)],
        )
        .unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    fn rejected(&mut self, marker: &str) {
        let db = self.storage.connection.lock().unwrap();
        assert!(inspect(&db, &self.storage.managed_anchor, marker, &|| false).is_err());
        assert!(audio::backup_deleted(&self.storage, &db, &|| false).is_err());
        drop(db);
        self.reopen();
        assert!(self.storage.data_write_blocked());
        assert!(
            startup::reason(&self.storage.connection.lock().unwrap(), marker)
                .unwrap()
                .is_some()
        );
        assert!(self.storage.get_meeting(&self.other).unwrap().is_some());
    }
}
fn record(db: &Connection, op: &str) -> String {
    db.query_row("SELECT json_array(id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at) FROM maintenance_operations WHERE id=?1", [op], |r|r.get(0)).unwrap()
}
fn path_records(db: &Connection, op: &str) -> String {
    db.query_row("SELECT json_group_array(json_array(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash)) FROM (SELECT * FROM maintenance_paths WHERE operation_id=?1 ORDER BY relative_path)", [op], |r|r.get(0)).unwrap()
}

#[test]
fn audio_supersession_actual_shared_controller_deletes_rows_without_rewriting_audio_consent_and_restart_is_writable(
) {
    let mut f = Fixture::new();
    let mut prepared = operations::Prepared::fresh(&f.storage, &f.meeting, &|| false).unwrap();
    let preview = prepared.preview();
    assert!(preview.execute_allowed);
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let receipt = prepared
        .execute(
            &f.storage,
            &lease,
            &preview.confirmation_id,
            true,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted);
    drop(lease);
    let marker = f.marker();
    f.unchanged_history();
    assert!(!f.directory.exists());
    let db = f.storage.connection.lock().unwrap();
    inspect(&db, &f.storage.managed_anchor, &marker, &|| false).unwrap();
    assert!(
        superseded(&db, &f.storage.managed_anchor, &f.audio_operation, &|| {
            false
        })
        .unwrap()
    );
    assert!(audio::backup_deleted(&f.storage, &db, &|| false)
        .unwrap()
        .is_empty());
    assert!(inspect(&db, &f.storage.managed_anchor, &marker, &|| true).is_err());
    drop(db);
    assert!(audio::intent::authorize_review(&f.storage, &f.audio_operation).is_err());
    f.reopen();
    assert!(!f.storage.data_write_blocked());
    assert_eq!(
        startup::reason(&f.storage.connection.lock().unwrap(), &marker).unwrap(),
        None
    );
    assert_eq!(
        startup::reason(&f.storage.connection.lock().unwrap(), &f.audio_operation).unwrap(),
        None
    );
    f.storage
        .rename_meeting(&f.other, "NEIGHBOR STILL WRITABLE", 0)
        .unwrap();
    f.unchanged_history();
}

#[test]
fn audio_supersession_cancel_before_row_commit_rolls_back_marker_rows_and_fk_together_then_retries()
{
    let f = Fixture::new();
    let mut receipt = f.begin();
    let stop = Cell::new(false);
    let observed = Cell::new(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| stop.get(), &|e| {
            if e == "beforeRowsCommit" {
                observed.set(true);
                stop.set(true);
            }
        })
        .is_err()
    );
    assert!(observed.get());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(record(&db, &f.audio_operation), f.audio_record);
    assert_eq!(
        db.query_row(
            "SELECT meeting_id FROM maintenance_operations WHERE id=?1",
            [&f.audio_operation],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        f.meeting
    );
    assert_eq!(db.query_row("SELECT count(*) FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
    assert!(f.storage.data_write_blocked());
    drop(db);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    completion::run(&f.storage, &mut receipt, &|| false).unwrap();
    f.unchanged_history();
}

#[test]
fn audio_supersession_committed_rows_survive_cold_restart_before_purge_without_spurious_audio_attention(
) {
    let mut f = Fixture::new();
    let mut receipt = f.begin();
    let op = receipt.confirmed().operation().to_owned();
    let stop = Cell::new(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| stop.get(), &|e| {
            if e == "rowsCommitted" {
                stop.set(true);
            }
        })
        .is_err()
    );
    assert!(receipt.row_phase_committed());
    let marker = f.marker();
    f.reopen();
    assert!(f.storage.data_write_blocked());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(startup::reason(&db, &marker).unwrap(), None);
    assert_eq!(startup::reason(&db, &f.audio_operation).unwrap(), None);
    drop(db);
    let mut pending = operations::Prepared::pending(&f.storage, &op, &|| false).unwrap();
    let preview = pending.preview();
    assert!(preview.row_phase_committed && preview.execute_allowed);
    let lease = AnalysisController::default()
        .begin_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .unwrap();
    assert!(
        pending
            .execute(
                &f.storage,
                &lease,
                &preview.confirmation_id,
                false,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    drop(lease);
    f.reopen();
    assert!(!f.storage.data_write_blocked());
    f.unchanged_history();
}

#[test]
fn audio_supersession_rehashed_scope_membership_order_and_identity_changes_never_admit_detached_history(
) {
    for case in 0..9 {
        let mut f = Fixture::new();
        let marker = f.finish();
        f.edit_marker(&marker, |h| match case {
            0 => h.scope_hash = "a".repeat(64),
            1 => h.audio_operations.clear(),
            2 => h.maintenance_ids.clear(),
            3 => h.maintenance_ids.push(uuid::Uuid::new_v4().to_string()),
            4 => h.audio_operations.push(h.audio_operations[0].clone()),
            5 => h.delete_operation = uuid::Uuid::new_v4().to_string(),
            6 => h.meeting = f.other.clone(),
            7 => h.maintenance_ids.reverse(),
            _ => {
                let mut identity = serde_json::to_value(&h.database).unwrap();
                identity["inode"] = (identity["inode"].as_u64().unwrap() + 1).into();
                h.database = serde_json::from_value(identity).unwrap();
            }
        });
        f.rejected(&marker);
        assert!(!f.directory.exists());
    }
}

#[test]
fn audio_supersession_preserved_native_metadata_or_chunk_changes_block_backup_and_startup() {
    for case in 0..4 {
        let mut f = Fixture::new();
        let marker = f.finish();
        let db = f.storage.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        match case {
            0 => db
                .execute(
                    "UPDATE maintenance_operations SET consent_at=consent_at+1 WHERE id=?1",
                    [&f.audio_operation],
                )
                .unwrap(),
            1 => db
                .execute(
                    "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                    [&f.audio_operation],
                )
                .unwrap(),
            2 => db
                .execute(
                    "UPDATE maintenance_paths SET identity_hash=?2 WHERE operation_id=?1",
                    params![f.audio_operation, "a".repeat(64)],
                )
                .unwrap(),
            _ => db
                .execute(
                    "UPDATE maintenance_operations SET meeting_id=?2 WHERE id=?1",
                    params![f.audio_operation, f.other],
                )
                .unwrap(),
        };
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        drop(db);
        f.rejected(&marker);
    }
}

#[test]
fn audio_supersession_orphan_duplicate_or_path_bearing_markers_fail_closed() {
    for case in 0..3 {
        let mut f = Fixture::new();
        let marker = f.finish();
        let db = f.storage.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        match case {
            0 => {
                db.execute("DELETE FROM maintenance_paths WHERE operation_id=(SELECT json_extract(target_manifest_json,'$.deleteOperation') FROM maintenance_operations WHERE id=?1)", [&marker]).unwrap();
                db.execute("DELETE FROM maintenance_operations WHERE id=(SELECT json_extract(target_manifest_json,'$.deleteOperation') FROM maintenance_operations WHERE id=?1)", [&marker]).unwrap();
            }
            1 => {
                let json: String = db
                    .query_row(
                        "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                        [&marker],
                        |r| r.get(0),
                    )
                    .unwrap();
                let mut h: Header = serde_json::from_str(&json).unwrap();
                h.operation = uuid::Uuid::new_v4().to_string();
                let json = serde_json::to_string(&h).unwrap();
                db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','completed',?2,?3,?4,0,0)", params![h.operation,json,hash(&json),PROGRESS]).unwrap();
            }
            _ => {
                db.execute("INSERT INTO maintenance_paths(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash) VALUES(?1,'forged-target','file',0,?2,'{}',?2)", params![marker,"a".repeat(64)]).unwrap();
            }
        }
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
        drop(db);
        f.rejected(&marker);
    }
}

#[test]
fn audio_supersession_recreated_db_or_logical_or_reserved_root_is_retained_and_never_reclassified()
{
    for case in 0..3 {
        let mut f = Fixture::new();
        let marker = f.finish();
        let outside = f.base.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"KEEP EXTERNAL").unwrap();
        if case == 0 {
            f.storage.connection.lock().unwrap().execute("INSERT INTO meetings(id,title,lifecycle,source_type,created_at,updated_at,lock_version) VALUES(?1,'RECREATED','draft','recording',0,0,0)", [&f.meeting]).unwrap();
        } else if case == 1 {
            std::os::unix::fs::symlink(&outside, &f.directory).unwrap();
        } else {
            let db = f.storage.connection.lock().unwrap();
            let (original, files) = startup::supersession_source(
                &db,
                &serde_json::from_str::<Header>(
                    &db.query_row(
                        "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
                        [&marker],
                        |r| r.get::<_, String>(0),
                    )
                    .unwrap(),
                )
                .unwrap()
                .delete_operation,
                &|| false,
            )
            .unwrap();
            let roots = renewal::reconfirmation_roots(&db, &original, &files, &|| false).unwrap();
            let root = roots
                .iter()
                .find(|r| r.starts_with("meetings/.mon-delete-"))
                .unwrap();
            std::fs::create_dir(f.storage.root().join(root)).unwrap();
        }
        if case == 0 {
            f.rejected(&marker);
        } else {
            let db = f.storage.connection.lock().unwrap();
            assert!(inspect(&db, &f.storage.managed_anchor, &marker, &|| false).is_err());
            assert!(audio::backup_deleted(&f.storage, &db, &|| false).is_err());
            let parent = startup_parent(&db, &f.storage.managed_anchor, &marker)
                .unwrap()
                .unwrap();
            drop(db);
            f.reopen();
            assert!(f.storage.data_write_blocked());
            let db = f.storage.connection.lock().unwrap();
            assert!(startup::reason(&db, &parent).unwrap().is_some());
            assert_eq!(startup::reason(&db, &marker).unwrap(), None);
            assert_eq!(startup::reason(&db, &f.audio_operation).unwrap(), None);
        }
        assert_eq!(
            std::fs::read(outside.join("keep")).unwrap(),
            b"KEEP EXTERNAL"
        );
    }
}

#[test]
fn audio_supersession_parent_reconfirmation_resolves_reappearance_without_new_audio_consent_or_rewritten_history(
) {
    let mut f = Fixture::new();
    let marker = f.finish();
    let parent = startup_parent(
        &f.storage.connection.lock().unwrap(),
        &f.storage.managed_anchor,
        &marker,
    )
    .unwrap()
    .unwrap();
    std::fs::create_dir(&f.directory).unwrap();
    std::fs::write(
        f.directory.join("new-unknown-file"),
        b"ONLY REMOVE AFTER NEW CONSENT",
    )
    .unwrap();
    f.reopen();
    assert!(f.storage.data_write_blocked());
    let mut prepared = operations::Prepared::pending(&f.storage, &parent, &|| false).unwrap();
    assert!(prepared.preview().reconfirmation_allowed);
    let lease = AnalysisController::default()
        .begin_deletion_maintenance(&f.storage, &parent, false, false, &|| false)
        .unwrap();
    prepared
        .review_remaining(
            &f.storage,
            &lease,
            &prepared.preview().confirmation_id,
            &|| false,
        )
        .unwrap();
    let preview = prepared.preview();
    assert!(preview.execute_allowed && preview.row_phase_committed);
    assert!(
        prepared
            .execute(
                &f.storage,
                &lease,
                &preview.confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    drop(lease);
    assert!(!f.directory.exists());
    f.unchanged_history();
    inspect(
        &f.storage.connection.lock().unwrap(),
        &f.storage.managed_anchor,
        &marker,
        &|| false,
    )
    .unwrap();
    f.reopen();
    assert!(!f.storage.data_write_blocked());
}

#[test]
fn audio_supersession_missing_marker_does_not_weaken_normal_detached_fk_rejection() {
    let mut f = Fixture::new();
    let marker = f.finish();
    let db = f.storage.connection.lock().unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute("DELETE FROM maintenance_operations WHERE id=?1", [&marker])
        .unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    drop(db);
    f.reopen();
    assert!(f.storage.data_write_blocked());
    assert!(
        startup::reason(&f.storage.connection.lock().unwrap(), &f.audio_operation)
            .unwrap()
            .is_some()
    );
    assert!(audio::intent::authorize_review(&f.storage, &f.audio_operation).is_err());
}
