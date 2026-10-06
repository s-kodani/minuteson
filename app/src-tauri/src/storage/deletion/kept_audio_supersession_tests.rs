//! Private synthetic recordings; retained history never becomes consent.
use super::*;

#[path = "kept_audio_supersession_crash_tests.rs"]
mod crash;

struct Kept {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    operation: String,
    originals: Vec<(String, String, String)>,
}
fn original(db: &Connection, id: &str) -> String {
    db.query_row("SELECT json_array(rowid,id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at) FROM maintenance_operations WHERE id=?1", [id], |r|r.get(0)).unwrap()
}
impl Kept {
    fn new(ancestor: bool) -> Self {
        Self::at(tempfile::tempdir().unwrap(), ancestor)
    }
    fn at(base: tempfile::TempDir, ancestor: bool) -> Self {
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("SYNTHETIC KEPT AUDIO").unwrap().id;
        let other = storage.create_meeting("KEEP NEIGHBOR").unwrap().id;
        let directory = storage.root().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("source.flac");
        std::fs::write(&file, b"ORIGINAL KEPT AUDIO").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                file.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let parent = uuid::Uuid::new_v4().to_string();
        let policy = audio::intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        };
        audio::intent::prepare(
            &storage,
            &parent,
            &audio::Census::capture(&storage, &meeting, &|| false).unwrap(),
            policy.clone(),
            &lease,
            &|| false,
        )
        .unwrap();
        let operation = if ancestor {
            audio::intent::removal::start(&storage, &parent, &lease, &|| false).unwrap();
            audio::intent::removal::run(&storage, &parent, &lease, &|| false).unwrap();
            audio::intent::removal::completion::finish(&storage, &parent, &lease, &|| false)
                .unwrap();
            std::fs::write(&file, b"RETURNED AUDIO KEPT WITHOUT STARTING").unwrap();
            let review =
                audio::intent::reconfirmation::Review::capture(&storage, &parent, &lease, &|| {
                    false
                })
                .unwrap();
            let child = uuid::Uuid::new_v4().to_string();
            audio::intent::reconfirmation::accept(
                &storage,
                &child,
                &review,
                policy,
                &lease,
                &|| false,
            )
            .unwrap();
            child
        } else {
            parent.clone()
        };
        audio::intent::keep_unstarted(&storage, &operation, &lease, &|| false).unwrap();
        drop(lease);
        let db = storage.connection.lock().unwrap();
        let ids = if ancestor {
            vec![parent, operation.clone()]
        } else {
            vec![operation.clone()]
        };
        let originals = ids
            .iter()
            .map(|id| (id.clone(), original(&db, id), path_records(&db, id)))
            .collect();
        drop(db);
        Self {
            base,
            storage,
            meeting,
            other,
            operation,
            originals,
        }
    }
    fn begin(&self) -> completion::Receipt {
        let census = files::Census::capture(&self.storage, &self.meeting, &|| false).unwrap();
        let review = removal::Reviewed::confirm(&census, true).unwrap();
        let prepared = intent::prepare(
            &self.storage,
            &uuid::Uuid::new_v4().to_string(),
            &census,
            &|| false,
        )
        .unwrap();
        let mut removed = removal::start(&self.storage, &prepared, &review, &|| false).unwrap();
        removal::run(&self.storage, &mut removed, &|| false).unwrap();
        completion::start(&self.storage, &removed, &|| false).unwrap()
    }
    fn finish(&self) -> String {
        let mut prepared =
            operations::Prepared::fresh(&self.storage, &self.meeting, &|| false).unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        assert!(
            prepared
                .execute(
                    &self.storage,
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
        self.marker()
    }
    fn marker(&self) -> String {
        self.storage.connection.lock().unwrap().query_row("SELECT id FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r|r.get(0)).unwrap()
    }
    fn unchanged(&self, detached: bool) {
        let db = self.storage.connection.lock().unwrap();
        for (id, before, paths) in &self.originals {
            assert_eq!(original(&db, id), *before);
            assert_eq!(path_records(&db, id), *paths);
            let fk: Option<String> = db
                .query_row(
                    "SELECT meeting_id FROM maintenance_operations WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                fk.as_deref(),
                if detached {
                    None
                } else {
                    Some(self.meeting.as_str())
                }
            );
        }
        if detached {
            assert!(audio::intent::read(&db, &self.operation, &|| false).is_err());
            assert!(
                audio::intent::removal::Receipt::load(&db, &self.operation, &|| false).is_err()
            );
        }
    }
    fn reopen(&mut self) {
        let root = self.storage.root().to_owned();
        let spare = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut self.storage,
            Storage::open(spare.path().to_owned()).unwrap(),
        );
        drop(old);
        self.storage = Storage::open(root).unwrap();
    }
    fn edit(&self, id: &str, edit: impl FnOnce(&Connection)) {
        let db = self.storage.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        assert!(!id.is_empty());
        edit(&db);
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    fn corrupt_marker(&self, id: &str, change: impl FnOnce(&mut Header)) {
        self.edit(id, |db| {
            let json: String = db.query_row("SELECT target_manifest_json FROM maintenance_operations WHERE id=?1", [id], |r|r.get(0)).unwrap();
            let mut h: Header = serde_json::from_str(&json).unwrap();
            change(&mut h);
            let json = serde_json::to_string(&h).unwrap();
            db.execute("UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1", params![id,json,hash(&json)]).unwrap();
        });
    }
}

#[test]
fn kept_audio_supersession_kept_only_is_v2_and_never_becomes_completed_or_executable() {
    let mut f = Kept::new(false);
    let marker = f.finish();
    f.unchanged(true);
    let db = f.storage.connection.lock().unwrap();
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&marker],
            |r| r.get(0),
        )
        .unwrap();
    let h: Header = serde_json::from_str(&json).unwrap();
    assert_eq!(h.version, 2);
    assert!(h.audio_operations.is_empty());
    assert_eq!(h.kept_audio_operations, vec![f.operation.clone()]);
    inspect(&db, &f.storage.managed_anchor, &marker, &|| false).unwrap();
    inspect_kept(&db, &f.storage.managed_anchor, &f.operation, &|| false).unwrap();
    assert!(!superseded(&db, &f.storage.managed_anchor, &f.operation, &|| false).unwrap());
    drop(db);
    assert!(audio::intent::authorize_review(&f.storage, &f.operation).is_err());
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(audio::intent::removal::start(&f.storage, &f.operation, &lease, &|| false).is_err());
    assert!(audio::intent::reconfirmation::Review::capture(
        &f.storage,
        &f.operation,
        &lease,
        &|| false
    )
    .is_err());
    drop(lease);
    f.reopen();
    assert!(!f.storage.data_write_blocked());
    f.unchanged(true);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    f.storage
        .rename_meeting(&f.other, "NEIGHBOR STILL WRITABLE", 0)
        .unwrap();
    let backup = f
        .storage
        .create_backup_archive(&f.base.path().join("kept.monbackup"), &|| false)
        .unwrap();
    assert_eq!(backup.deleted_audio_meeting_count, 0);
}

#[test]
fn kept_audio_supersession_kept_v2_child_preserves_completed_ancestor_and_all_evidence() {
    let mut f = Kept::new(true);
    let marker = f.finish();
    f.unchanged(true);
    let db = f.storage.connection.lock().unwrap();
    let json: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&marker],
            |r| r.get(0),
        )
        .unwrap();
    let h: Header = serde_json::from_str(&json).unwrap();
    assert_eq!(h.audio_operations, vec![f.originals[0].0.clone()]);
    assert_eq!(h.kept_audio_operations, vec![f.operation.clone()]);
    inspect(&db, &f.storage.managed_anchor, &marker, &|| false).unwrap();
    drop(db);
    f.reopen();
    assert!(!f.storage.data_write_blocked());
    f.unchanged(true);
    assert!(f
        .storage
        .create_backup_archive(&f.base.path().join("child.monbackup"), &|| false)
        .is_ok());
}

#[test]
fn kept_audio_supersession_cancel_rolls_back_marker_and_fks_then_cold_purge_resumes() {
    let mut f = Kept::new(false);
    let mut receipt = f.begin();
    let full = receipt.confirmed().operation().to_owned();
    let stop = Cell::new(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| stop.get(), &|event| {
            if event == "beforeRowsCommit" {
                stop.set(true);
            }
        })
        .is_err()
    );
    assert!(stop.get());
    f.unchanged(false);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(f.storage.connection.lock().unwrap().query_row("SELECT count(*) FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r|r.get::<_, i64>(0)).unwrap(), 0);
    stop.set(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| stop.get(), &|event| {
            if event == "rowsCommitted" {
                stop.set(true);
            }
        })
        .is_err()
    );
    assert!(receipt.row_phase_committed());
    f.unchanged(true);
    let marker = f.marker();
    f.reopen();
    assert!(f.storage.data_write_blocked());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(startup::reason(&db, &marker).unwrap(), None);
    assert_eq!(startup::reason(&db, &f.operation).unwrap(), None);
    inspect_kept(&db, &f.storage.managed_anchor, &f.operation, &|| false).unwrap();
    drop(db);
    let lease = AnalysisController::default()
        .begin_deletion_maintenance(&f.storage, &full, false, false, &|| false)
        .unwrap();
    let mut pending = operations::Prepared::pending(&f.storage, &full, &|| false).unwrap();
    assert!(
        pending
            .execute(
                &f.storage,
                &lease,
                &pending.preview().confirmation_id,
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
    f.unchanged(true);
}

#[test]
fn kept_audio_supersession_rehashed_version_membership_overlap_and_boundaries_fail_closed() {
    for case in 0..7 {
        let mut f = Kept::new(false);
        let marker = f.finish();
        f.corrupt_marker(&marker, |h| match case {
            0 => h.version = 1,
            1 => h.kept_audio_operations.clear(),
            2 => h.kept_audio_operations.push(f.operation.clone()),
            3 => h.audio_operations.push(f.operation.clone()),
            4 => h.kept_audio_operations[0] = uuid::Uuid::new_v4().to_string(),
            5 => h.maintenance_ids.push(uuid::Uuid::new_v4().to_string()),
            _ => {
                h.kept_audio_operations =
                    (0..257).map(|_| uuid::Uuid::new_v4().to_string()).collect()
            }
        });
        let db = f.storage.connection.lock().unwrap();
        assert!(
            inspect_kept(&db, &f.storage.managed_anchor, &f.operation, &|| false).is_err(),
            "{case}"
        );
        assert!(
            audio::backup_deleted(&f.storage, &db, &|| false).is_err(),
            "{case}"
        );
        drop(db);
        f.reopen();
        assert!(f.storage.data_write_blocked(), "{case}");
        f.unchanged(true);
    }
}

#[test]
fn kept_audio_supersession_missing_duplicate_or_mutated_history_blocks_startup_without_repair() {
    for case in 0..5 {
        let mut f = Kept::new(false);
        let marker = f.finish();
        f.edit(&marker, |db| match case {
            0 => { db.execute("DELETE FROM maintenance_operations WHERE id=?1", [&marker]).unwrap(); }
            1 => {
                let json: String = db.query_row("SELECT target_manifest_json FROM maintenance_operations WHERE id=?1", [&marker], |r|r.get(0)).unwrap();
                let mut h: Header = serde_json::from_str(&json).unwrap();
                h.operation = uuid::Uuid::new_v4().to_string();
                let json = serde_json::to_string(&h).unwrap();
                db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','completed',?2,?3,?4,0,0)", params![h.operation,json,hash(&json),PROGRESS]).unwrap();
            }
            2 => { db.execute("UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1", [&f.operation]).unwrap(); }
            3 => { db.execute("DELETE FROM maintenance_paths WHERE operation_id=?1", [&f.operation]).unwrap(); }
            _ => { db.execute("UPDATE maintenance_operations SET consent_at=consent_at+1,created_at=created_at+1,updated_at=updated_at+1 WHERE id=?1", [&f.operation]).unwrap(); }
        });
        let before = original(&f.storage.connection.lock().unwrap(), &f.operation);
        let db = f.storage.connection.lock().unwrap();
        assert!(
            inspect_kept(&db, &f.storage.managed_anchor, &f.operation, &|| false).is_err(),
            "{case}"
        );
        assert!(
            audio::backup_deleted(&f.storage, &db, &|| false).is_err(),
            "{case}"
        );
        drop(db);
        f.reopen();
        assert!(f.storage.data_write_blocked(), "{case}");
        assert!(f.storage.create_meeting("MUST NOT WRITE").is_err());
        let db = f.storage.connection.lock().unwrap();
        assert!(startup::reason(&db, &f.operation).unwrap().is_some());
        assert!(db
            .execute(
                "UPDATE meetings SET title='MUST NOT WRITE' WHERE id=?1",
                [&f.other]
            )
            .is_err());
        assert_eq!(original(&db, &f.operation), before);
        assert!(audio::intent::read(&db, &f.operation, &|| false).is_err());
    }
}
