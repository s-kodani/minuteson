//! Kept whole-meeting consent is metadata, never a permit after full deletion.
use super::*;

#[path = "kept_meeting_supersession_crash_tests.rs"]
mod crash;

struct KeptMeeting {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    kept: String,
    originals: Vec<(String, String, String)>,
}
fn row(db: &Connection, id: &str) -> String {
    db.query_row("SELECT json_array(rowid,id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at) FROM maintenance_operations WHERE id=?1", [id], |r| r.get(0)).unwrap()
}
fn keep(storage: &Storage, meeting: &str) -> String {
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    intent::prepare(
        storage,
        &id,
        &files::Census::capture(storage, meeting, &crate::test_support::never_cancelled).unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let mut pending =
        operations::Prepared::pending(storage, &id, &crate::test_support::never_cancelled).unwrap();
    assert!(
        pending
            .keep(storage, &lease, &pending.preview().confirmation_id, &|| {
                false
            })
            .unwrap()
            .retained
    );
    id
}
impl KeptMeeting {
    fn new(audio_kept: Option<bool>) -> Self {
        Self::at(tempfile::tempdir().unwrap(), audio_kept)
    }
    fn at(base: tempfile::TempDir, audio_kept: Option<bool>) -> Self {
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("SYNTHETIC KEPT MEETING").unwrap().id;
        let other = storage.create_meeting("KEEP NEIGHBOR").unwrap().id;
        let directory = storage.root().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("source.flac");
        std::fs::write(&file, b"KEEP ORIGINAL AUDIO UNTIL NEW FULL CONSENT").unwrap();
        std::fs::write(directory.join("text.txt"), b"ORIGINAL TEXT").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                file.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        let mut ids = Vec::new();
        if let Some(kept) = audio_kept {
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
                .unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            audio::intent::prepare(
                &storage,
                &id,
                &audio::Census::capture(&storage, &meeting, &crate::test_support::never_cancelled)
                    .unwrap(),
                audio::intent::Policy {
                    include_unregistered_audio: true,
                    retain_unclassified: true,
                },
                &lease,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            if kept {
                audio::intent::keep_unstarted(
                    &storage,
                    &id,
                    &lease,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
            } else {
                audio::intent::removal::start(
                    &storage,
                    &id,
                    &lease,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
                audio::intent::removal::run(
                    &storage,
                    &id,
                    &lease,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
                audio::intent::removal::completion::finish(
                    &storage,
                    &id,
                    &lease,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
            }
            ids.push(id);
        }
        let kept = keep(&storage, &meeting);
        ids.push(kept.clone());
        let originals = {
            let db = storage.connection.lock().unwrap();
            ids.into_iter()
                .map(|id| (id.clone(), row(&db, &id), path_records(&db, &id)))
                .collect()
        };
        Self {
            base,
            storage,
            meeting,
            other,
            kept,
            originals,
        }
    }
    fn begin(&self) -> completion::Receipt {
        let census = files::Census::capture(
            &self.storage,
            &self.meeting,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let reviewed = removal::Reviewed::confirm(&census, true).unwrap();
        let prepared = intent::prepare(
            &self.storage,
            &uuid::Uuid::new_v4().to_string(),
            &census,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let mut files = removal::start(
            &self.storage,
            &prepared,
            &reviewed,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        removal::run(
            &self.storage,
            &mut files,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        completion::start(&self.storage, &files, &crate::test_support::never_cancelled).unwrap()
    }
    fn finish(&self) -> String {
        let mut p = operations::Prepared::fresh(
            &self.storage,
            &self.meeting,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
            .unwrap();
        assert!(
            p.execute(
                &self.storage,
                &lease,
                &p.preview().confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
        );
        self.marker()
    }
    fn marker(&self) -> String {
        self.storage.connection.lock().unwrap().query_row("SELECT id FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r| r.get(0)).unwrap()
    }
    fn unchanged(&self, detached: bool) {
        let db = self.storage.connection.lock().unwrap();
        for (id, original, paths) in &self.originals {
            assert_eq!(row(&db, id), *original);
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
            assert!(intent::read_kept_database(
                &db,
                &self.kept,
                &crate::test_support::never_cancelled
            )
            .is_err());
            assert!(
                intent::read_database(&db, &self.kept, &crate::test_support::never_cancelled)
                    .is_err()
            );
            assert!(
                removal::Receipt::load(&db, &self.kept, &crate::test_support::never_cancelled)
                    .is_err()
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
    fn edit(&self, change: impl FnOnce(&Connection)) {
        let db = self.storage.connection.lock().unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        change(&db);
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    fn mutate_marker(&self, marker: &str, change: impl FnOnce(&mut Header)) {
        self.edit(|db| {
            let json: String = db.query_row("SELECT target_manifest_json FROM maintenance_operations WHERE id=?1", [marker], |r| r.get(0)).unwrap();
            let mut h: Header = serde_json::from_str(&json).unwrap();
            change(&mut h);
            let json = serde_json::to_string(&h).unwrap();
            db.execute("UPDATE maintenance_operations SET target_manifest_json=?2,target_hash=?3 WHERE id=?1", params![marker, json, hash(&json)]).unwrap();
        });
    }
}

#[test]
fn kept_meeting_supersession_only_kept_meeting_is_v3_and_cannot_resume_old_consent() {
    let mut f = KeptMeeting::new(None);
    // A later edit is covered only by the new full deletion, never the kept census.
    std::fs::write(
        f.storage
            .root()
            .join("meetings")
            .join(&f.meeting)
            .join("text.txt"),
        b"LATER TEXT COVERED BY NEW CONSENT",
    )
    .unwrap();
    let marker = f.finish();
    f.unchanged(true);
    {
        let db = f.storage.connection.lock().unwrap();
        let proof = Proof::load(
            &db,
            &f.storage.managed_anchor,
            &marker,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(proof.header.version, 3);
        assert!(proof.header.audio_operations.is_empty());
        assert!(proof.header.kept_audio_operations.is_empty());
        assert_eq!(proof.header.kept_meeting_operations, vec![f.kept.clone()]);
        assert!(proof.authorizes_kept_meeting(&f.kept, &f.meeting));
        assert!(!proof.authorizes(&f.kept, &f.meeting));
        assert!(!proof.authorizes_kept(&f.kept, &f.meeting));
        inspect_kept_meeting(
            &db,
            &f.storage.managed_anchor,
            &f.kept,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    }
    assert!(
        intent::read_keepable(&f.storage, &f.kept, &crate::test_support::never_cancelled).is_err()
    );
    assert!(operations::Prepared::pending(
        &f.storage,
        &f.kept,
        &crate::test_support::never_cancelled
    )
    .is_err());
    f.reopen();
    assert!(!f.storage.data_write_blocked());
    f.unchanged(true);
    f.storage
        .rename_meeting(&f.other, "NEIGHBOR STILL WRITABLE", 0)
        .unwrap();
    assert_eq!(
        f.storage
            .create_backup_archive(
                &f.base.path().join("kept-meeting.monbackup"),
                &crate::test_support::never_cancelled
            )
            .unwrap()
            .deleted_audio_meeting_count,
        0
    );
}

#[test]
fn kept_meeting_supersession_mixed_completed_and_kept_audio_sets_remain_separate() {
    for audio_kept in [false, true] {
        let mut f = KeptMeeting::new(Some(audio_kept));
        let marker = f.finish();
        f.unchanged(true);
        {
            let db = f.storage.connection.lock().unwrap();
            let proof = Proof::load(
                &db,
                &f.storage.managed_anchor,
                &marker,
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            assert_eq!(proof.header.version, 3);
            assert_eq!(proof.header.kept_meeting_operations, vec![f.kept.clone()]);
            assert_eq!(
                proof.header.kept_audio_operations.len(),
                usize::from(audio_kept)
            );
            assert_eq!(
                proof.header.audio_operations.len(),
                usize::from(!audio_kept)
            );
        }
        f.reopen();
        assert!(!f.storage.data_write_blocked());
        f.unchanged(true);
        f.storage
            .create_backup_archive(
                &f.base.path().join("mixed.monbackup"),
                &crate::test_support::never_cancelled,
            )
            .unwrap();
    }
}

#[test]
fn kept_meeting_supersession_cancel_rolls_back_then_cold_rows_committed_purge_resumes() {
    let mut f = KeptMeeting::new(Some(true));
    let mut receipt = f.begin();
    let full = receipt.confirmed().operation().to_owned();
    let cancel = Cell::new(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| cancel.get(), &|phase| {
            if phase == "beforeRowsCommit" {
                cancel.set(true);
            }
        })
        .is_err()
    );
    f.unchanged(false);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(f.storage.connection.lock().unwrap().query_row("SELECT count(*) FROM maintenance_operations WHERE json_extract(target_manifest_json,'$.subtype')='audioHistorySupersession'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    cancel.set(false);
    assert!(
        completion::run_observed(&f.storage, &mut receipt, &|| cancel.get(), &|phase| {
            if phase == "rowsCommitted" {
                cancel.set(true);
            }
        })
        .is_err()
    );
    assert!(receipt.row_phase_committed());
    f.unchanged(true);
    f.reopen();
    assert!(f.storage.data_write_blocked());
    assert_eq!(
        startup::reason(&f.storage.connection.lock().unwrap(), &f.kept).unwrap(),
        None
    );
    let lease = AnalysisController::default()
        .begin_deletion_maintenance(
            &f.storage,
            &full,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let mut p =
        operations::Prepared::pending(&f.storage, &full, &crate::test_support::never_cancelled)
            .unwrap();
    assert!(
        p.execute(
            &f.storage,
            &lease,
            &p.preview().confirmation_id,
            false,
            &crate::test_support::never_cancelled,
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
fn kept_meeting_supersession_rehashed_version_sets_overlap_and_limits_fail_closed() {
    for case in 0..8 {
        let mut f = KeptMeeting::new(None);
        let marker = f.finish();
        f.mutate_marker(&marker, |h| match case {
            0 => h.version = 1,
            1 => h.version = 2,
            2 => h.kept_meeting_operations.clear(),
            3 => h.kept_meeting_operations.push(f.kept.clone()),
            4 => h.audio_operations.push(f.kept.clone()),
            5 => h.kept_audio_operations.push(f.kept.clone()),
            6 => h.kept_meeting_operations[0] = uuid::Uuid::new_v4().to_string(),
            _ => {
                h.kept_meeting_operations =
                    (0..257).map(|_| uuid::Uuid::new_v4().to_string()).collect()
            }
        });
        let db = f.storage.connection.lock().unwrap();
        assert!(
            inspect_kept_meeting(
                &db,
                &f.storage.managed_anchor,
                &f.kept,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{case}"
        );
        assert!(
            audio::backup_deleted(&f.storage, &db, &crate::test_support::never_cancelled).is_err(),
            "{case}"
        );
        drop(db);
        f.reopen();
        assert!(f.storage.data_write_blocked(), "{case}");
        f.unchanged(true);
    }
}

#[test]
fn kept_meeting_supersession_orphan_duplicate_and_mutated_evidence_are_fenced_without_repair() {
    for case in 0..7 {
        let mut f = KeptMeeting::new(None);
        let marker = f.finish();
        f.edit(|db| match case {
            0 => { db.execute("DELETE FROM maintenance_operations WHERE id=?1", [&marker]).unwrap(); }
            1 => {
                let json: String = db.query_row("SELECT target_manifest_json FROM maintenance_operations WHERE id=?1", [&marker], |r| r.get(0)).unwrap();
                let mut h: Header = serde_json::from_str(&json).unwrap(); h.operation = uuid::Uuid::new_v4().to_string();
                let json = serde_json::to_string(&h).unwrap();
                db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','completed',?2,?3,?4,0,0)", params![h.operation, json, hash(&json), PROGRESS]).unwrap();
            }
            2 => { db.execute("UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1", [&f.kept]).unwrap(); }
            3 => { db.execute("DELETE FROM maintenance_paths WHERE operation_id=?1", [&f.kept]).unwrap(); }
            4 => { db.execute("UPDATE maintenance_operations SET consent_at=consent_at+1,created_at=created_at+1,updated_at=updated_at+1 WHERE id=?1", [&f.kept]).unwrap(); }
            5 => { db.execute("UPDATE maintenance_operations SET meeting_id=?2 WHERE id=?1", params![f.kept, f.other]).unwrap(); }
            _ => { db.execute("UPDATE maintenance_operations SET error_code='MUTATED' WHERE id=?1", [&f.kept]).unwrap(); }
        });
        let before = row(&f.storage.connection.lock().unwrap(), &f.kept);
        let db = f.storage.connection.lock().unwrap();
        assert!(
            inspect_kept_meeting(
                &db,
                &f.storage.managed_anchor,
                &f.kept,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{case}"
        );
        assert!(
            audio::backup_deleted(&f.storage, &db, &crate::test_support::never_cancelled).is_err(),
            "{case}"
        );
        drop(db);
        f.reopen();
        assert!(f.storage.data_write_blocked(), "{case}");
        assert!(f.storage.create_meeting("MUST NOT WRITE").is_err());
        let db = f.storage.connection.lock().unwrap();
        assert_eq!(row(&db, &f.kept), before);
        assert!(startup::reason(&db, &f.kept).unwrap().is_some());
        assert!(db
            .execute(
                "UPDATE meetings SET title='MUST NOT WRITE' WHERE id=?1",
                [&f.other]
            )
            .is_err());
    }
}
