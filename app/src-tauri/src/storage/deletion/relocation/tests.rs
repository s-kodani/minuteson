//! Owned synthetic audio, fake model files and temporary SQLite only.
use super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::{
        backup::relocation::NativeSnapshot,
        deletion::{audio, completion, files, removal},
    },
};
use rusqlite::{config::DbConfig, OpenFlags};
use std::{cell::Cell, os::unix::fs::symlink};

struct Fixture {
    _base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    audio: PathBuf,
    operation: String,
}
impl Fixture {
    fn new(completed: bool) -> Self {
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage
            .create_meeting("SYNTHETIC PRIVATE TITLE")
            .unwrap()
            .id;
        let other = storage.create_meeting("KEEP NEIGHBOR").unwrap().id;
        let directory = storage.root().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("audio.flac");
        std::fs::write(&path, b"SYNTHETIC AUDIO BODY").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                path.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        std::fs::write(directory.join("retained.md"), b"SYNTHETIC PRIVATE TEXT").unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        audio::intent::prepare(
            &storage,
            &operation,
            &audio::Census::capture(&storage, &meeting, &|| false).unwrap(),
            audio::intent::Policy {
                include_unregistered_audio: true,
                retain_unclassified: true,
            },
            &lease,
            &|| false,
        )
        .unwrap();
        if completed {
            audio::intent::removal::start(&storage, &operation, &lease, &|| false).unwrap();
            audio::intent::removal::run(&storage, &operation, &lease, &|| false).unwrap();
            audio::intent::removal::completion::finish(&storage, &operation, &lease, &|| false)
                .unwrap();
        } else {
            audio::intent::keep_unstarted(&storage, &operation, &lease, &|| false).unwrap();
        }
        Self {
            _base: base,
            storage,
            meeting,
            other,
            audio: path,
            operation,
        }
    }
    fn full_delete(&self) {
        let c = files::Census::capture(&self.storage, &self.meeting, &|| false).unwrap();
        let review = removal::Reviewed::confirm(&c, true).unwrap();
        let p = intent::prepare(
            &self.storage,
            &uuid::Uuid::new_v4().to_string(),
            &c,
            &|| false,
        )
        .unwrap();
        let mut files = removal::start(&self.storage, &p, &review, &|| false).unwrap();
        removal::run(&self.storage, &mut files, &|| false).unwrap();
        let mut rows = completion::start(&self.storage, &files, &|| false).unwrap();
        completion::run(&self.storage, &mut rows, &|| false).unwrap();
    }
    fn original(&self) -> String {
        native_rows(&self.storage.connection.lock().unwrap())
    }
}
fn native_rows(db: &Connection) -> String {
    db.query_row("SELECT json_array((SELECT json_group_array(json_array(rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at)) FROM (SELECT rowid,* FROM maintenance_operations ORDER BY id)),(SELECT json_group_array(json_array(operation_id,relative_path,kind,size_bytes,content_hash,identity_json,identity_hash)) FROM (SELECT * FROM maintenance_paths ORDER BY operation_id,relative_path)))", [], |r|r.get(0)).unwrap()
}
fn image(snapshot: &NativeSnapshot<'_>) -> Connection {
    let db = Connection::open_with_flags(
        snapshot.image().join("meetings.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .unwrap();
    db
}
fn capture(storage: &Storage) -> Source {
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    Source::capture(storage, &snapshot, &|| false)
        .unwrap()
        .unwrap()
}

#[test]
fn deletion_relocation_immutable_stamp_retains_consent_and_target_but_not_typed_progress_or_fk() {
    let f = Fixture::new(true);
    let db = f.storage.connection.lock().unwrap();
    let row = db.query_row("SELECT rowid,id,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at FROM maintenance_operations WHERE id=?1", [&f.operation], operation_row).unwrap();
    let expected = immutable_hash(&row).unwrap();
    for field in [
        "rowid", "id", "kind", "restore", "target", "sha", "consent", "created",
    ] {
        let mut changed = row.clone();
        match field {
            "rowid" => changed.0 += 1,
            "id" => changed.1 = uuid::Uuid::new_v4().to_string(),
            "kind" => changed.3 = "delete_meeting".into(),
            "restore" => changed.5 = Some(uuid::Uuid::new_v4().to_string()),
            "target" => changed.6.push(' '),
            "sha" => changed.7 = "0".repeat(64),
            "consent" => changed.8 = None,
            "created" => changed.11 += 1,
            _ => unreachable!(),
        }
        assert_ne!(immutable_hash(&changed).unwrap(), expected, "{field}");
    }
    let mut changed = row.clone();
    changed.2 = None;
    changed.9 = "{}".into();
    changed.10 = Some("typed inspection still required".into());
    changed.12 += 1;
    assert_eq!(immutable_hash(&changed).unwrap(), expected);
    assert_ne!(row_hash(&changed).unwrap(), row_hash(&row).unwrap());
}

#[test]
fn deletion_relocation_completed_audio_seals_every_row_and_proof_without_consent_rewrite() {
    let f = Fixture::new(true);
    let original = f.original();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    handoff.verify(&|| false).unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(handoff.bytes()).unwrap();
    assert_eq!(envelope["version"], 4);
    let sealed = &envelope["deletion_history"];
    assert_eq!(sealed["sourceRoot"], f.storage.root().to_str().unwrap());
    assert_eq!(sealed["records"].as_array().unwrap().len(), 1);
    let record = &sealed["records"][0];
    assert_eq!(record["operationId"], f.operation);
    assert_eq!(record["validation"], "completedAudio");
    assert!(record["pathCount"].as_u64().unwrap() > 3);
    assert_eq!(record["rowHash"].as_str().unwrap().len(), 64);
    assert_eq!(record["pathsHash"].as_str().unwrap().len(), 64);
    assert!(sealed["directories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["relativePath"] == format!("meetings/{}", f.meeting)));
    assert_eq!(native_rows(&image(&snapshot)), original);
    assert_eq!(f.original(), original);
    for body in [
        "SYNTHETIC PRIVATE TITLE",
        "SYNTHETIC PRIVATE TEXT",
        "SYNTHETIC AUDIO BODY",
    ] {
        assert!(!std::str::from_utf8(handoff.bytes()).unwrap().contains(body));
    }
    assert!(!f.audio.exists());
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
    assert_eq!(
        image(&snapshot)
            .query_row("SELECT count(*) FROM sqlite_temp_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn deletion_relocation_kept_audio_and_meeting_are_preserved_and_not_resumable_consent() {
    let f = Fixture::new(false);
    let c = files::Census::capture(&f.storage, &f.meeting, &|| false).unwrap();
    let p = intent::prepare(&f.storage, &uuid::Uuid::new_v4().to_string(), &c, &|| false).unwrap();
    intent::keep_unstarted(&f.storage, &p, &|| false).unwrap();
    let before = f.original();
    let source = capture(&f.storage);
    assert_eq!(source.records.len(), 2);
    assert!(source.records.iter().any(|r| r.validation == "keptAudio"));
    assert!(source.records.iter().any(|r| r.validation == "keptMeeting"));
    let report = super::receiver::inspect_synthetic_copy(&f.storage);
    assert_eq!(report.kept_records, 2);
    assert_eq!(report.completed_records, 0);
    assert_eq!(f.original(), before);
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"SYNTHETIC AUDIO BODY");
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(audio::intent::removal::start(&f.storage, &f.operation, &lease, &|| false).is_err());
}

#[test]
fn deletion_relocation_full_deletion_retains_supersession_and_detached_original_audio() {
    let f = Fixture::new(true);
    f.full_delete();
    let before = f.original();
    let source = capture(&f.storage);
    assert_eq!(source.records.len(), 3);
    for kind in [
        "completedAudio",
        "completedMeeting",
        "audioHistorySupersession",
    ] {
        assert!(source.records.iter().any(|r| r.validation == kind));
    }
    assert_eq!(f.original(), before);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
}

#[test]
fn deletion_relocation_refuses_reappearance_after_completed_audio_or_full_deletion() {
    for full in [false, true] {
        let f = Fixture::new(true);
        if full {
            f.full_delete();
        }
        let before = f.original();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        std::fs::create_dir_all(f.audio.parent().unwrap()).unwrap();
        std::fs::write(&f.audio, b"REAPPEARED NEVER DELETE").unwrap();
        let db = image(&snapshot);
        assert!(!startup::completed_attention(
            &db,
            &f.storage.managed_anchor,
            &f.storage.native_models_root
        )
        .unwrap()
        .is_empty());
        assert!(Source::capture(&f.storage, &snapshot, &|| false).is_err());
        assert!(snapshot.prepare_control_handoff(&|| false).is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM sqlite_temp_schema", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(f.original(), before);
        assert_eq!(std::fs::read(&f.audio).unwrap(), b"REAPPEARED NEVER DELETE");
    }
}

#[test]
fn deletion_relocation_unknown_recovered_or_corrupt_completed_record_is_not_a_success_hint() {
    for recovered in [false, true] {
        let f = Fixture::new(true);
        {
            let db = f.storage.connection.lock().unwrap();
            if recovered {
                db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,'delete_assets','recovered','{}',?2,'{}',0,0)", params![uuid::Uuid::new_v4().to_string(), "a".repeat(64)]).unwrap();
            } else {
                db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
                    .unwrap();
                db.execute(
                    "UPDATE maintenance_operations SET consent_at=consent_at+1 WHERE id=?1",
                    [&f.operation],
                )
                .unwrap();
                db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
                    .unwrap();
            }
        }
        let before = f.original();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        assert!(snapshot.prepare_control_handoff(&|| false).is_err());
        assert_eq!(f.original(), before);
        assert!(!f.audio.exists());
    }
}

#[test]
fn deletion_relocation_cancel_and_source_reappearance_invalidate_without_removing_new_files() {
    let f = Fixture::new(true);
    let before = f.original();
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    let n = Cell::new(0);
    assert!(Source::capture(&f.storage, &snapshot, &|| {
        n.set(n.get() + 1);
        n.get() > 4
    })
    .is_err());
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    assert!(handoff.verify(&|| true).is_err());
    std::fs::write(&f.audio, b"NEW BODY MUST REMAIN").unwrap();
    assert!(handoff.verify(&|| false).is_err());
    assert_eq!(f.original(), before);
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"NEW BODY MUST REMAIN");
}

#[test]
fn deletion_relocation_over_limit_is_rejected_before_typed_scan_without_temp_or_row_changes() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    s.connection.lock().unwrap().execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10001) INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) SELECT printf('00000000-0000-4000-8000-%012x',x),'delete_assets','completed','{}',printf('%064d',0),'{}',0,0 FROM n;").unwrap();
    let snapshot = s.prepare_relocation_snapshot(&|| false).unwrap();
    let error = Source::capture(&s, &snapshot, &|| false).unwrap_err();
    assert!(error.contains("件数・容量上限"));
    let db = image(&snapshot);
    assert_eq!(
        db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        10001
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM sqlite_temp_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        10001
    );
}

#[test]
fn deletion_relocation_envelope_refuses_missing_changed_or_version_downgraded_native_seal() {
    let f = Fixture::new(true);
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    let h = snapshot.prepare_control_handoff(&|| false).unwrap();
    h.check_compatible(h.bytes()).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(h.bytes()).unwrap();
    for mode in 0..4 {
        let mut changed = saved.clone();
        match mode {
            0 => {
                changed.as_object_mut().unwrap().remove("deletion_history");
            }
            1 => changed["version"] = 3.into(),
            2 => changed["deletion_history"]["records"][0]["rowHash"] = "b".repeat(64).into(),
            _ => changed["deletion_history"]["directories"][0]["identity"]["inode"] = 0.into(),
        }
        assert!(h
            .check_compatible(&serde_json::to_vec(&changed).unwrap())
            .is_err());
    }
}

#[test]
fn deletion_relocation_no_native_history_keeps_exact_version_three_shape_and_rejects_fake_seal() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let snapshot = s.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(Source::capture(&s, &snapshot, &|| false).unwrap().is_none());
    let h = snapshot.prepare_control_handoff(&|| false).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(h.bytes()).unwrap();
    assert_eq!(value["version"], 3);
    assert!(value.get("deletion_history").is_none());
    value["deletion_history"] = serde_json::Value::Null;
    assert!(h
        .check_compatible(&serde_json::to_vec(&value).unwrap())
        .is_err());
}

#[test]
fn deletion_relocation_row_and_path_hashes_bind_nulls_consent_and_logical_proof_names() {
    let f = Fixture::new(false);
    let source = capture(&f.storage);
    let db = f.storage.connection.lock().unwrap();
    let a = paths(&db, &f.operation, &|| false).unwrap();
    assert_eq!(
        a,
        (
            source.records[0].path_count,
            source.records[0].paths_hash.clone()
        )
    );
    let header: String = db
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&f.operation],
            |r| r.get(0),
        )
        .unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute("UPDATE maintenance_paths SET relative_path='audio-deletion-proof/z00000' WHERE operation_id=?1 AND relative_path='audio-deletion-proof/000000'", [&f.operation]).unwrap();
    let changed = paths(&db, &f.operation, &|| false).unwrap();
    assert_eq!(changed.0, a.0);
    assert_ne!(changed.1, a.1);
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            [&f.operation],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        header
    );
    drop(db);
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(Source::capture(&f.storage, &snapshot, &|| false).is_err());
    let mut none = Sha256::new();
    let mut empty = Sha256::new();
    frame(&mut none, &Option::<i64>::None).unwrap();
    frame(&mut empty, &Some(0_i64)).unwrap();
    assert_ne!(none.finalize(), empty.finalize());
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"SYNTHETIC AUDIO BODY");
}

#[test]
fn deletion_relocation_source_root_or_kept_parent_symlink_cannot_be_adopted() {
    let f = Fixture::new(false);
    let before = f.original();
    let directory = f.audio.parent().unwrap();
    let moved = directory.with_extension("retained");
    std::fs::rename(directory, &moved).unwrap();
    symlink(&moved, directory).unwrap();
    assert!(f.storage.prepare_relocation_snapshot(&|| false).is_err());
    assert_eq!(f.original(), before);
    assert_eq!(
        std::fs::read(moved.join("audio.flac")).unwrap(),
        b"SYNTHETIC AUDIO BODY"
    );
}

#[test]
fn deletion_relocation_approved_model_root_is_required_for_kept_and_completed_history() {
    for completed in [false, true] {
        let mut f = super::super::models::tests::Fixture::new();
        let actual_root = f.storage.root().to_owned();
        let placeholder = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut f.storage,
            Storage::open(placeholder.path().into()).unwrap(),
        );
        drop(old);
        f.storage = Storage::open_with_models(actual_root, f.root.clone()).unwrap();
        let p = super::super::models::preview(
            &f.storage,
            &f.root,
            &uuid::Uuid::new_v4().to_string(),
            &f.package,
            &f.review,
            &|| false,
        )
        .unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        super::super::models::intent::prepare(
            &f.storage,
            &f.root,
            &op,
            &f.package,
            &f.review,
            &p.census_hash,
            false,
            &lease,
            &|| false,
        )
        .unwrap();
        if completed {
            super::super::models::intent::removal::start(&f.storage, &op, &lease, &|| false)
                .unwrap();
            super::super::models::intent::removal::run(&f.storage, &op, &lease, &|| false).unwrap();
        } else {
            super::super::models::intent::keep_unstarted(&f.storage, &op, &lease, &|| false)
                .unwrap();
        }
        drop(lease);
        let source = capture(&f.storage);
        assert_eq!(
            source.records[0].validation,
            if completed {
                "completedModel"
            } else {
                "keptModel"
            }
        );
        assert_eq!(source.models_root, f.root);
        assert!(source.models_identity.is_some());
        let report = super::receiver::inspect_synthetic_copy(&f.storage);
        assert_eq!(report.checked_records, 1);
        assert_eq!(report.kept_records, usize::from(!completed));
        assert_eq!(report.completed_records, usize::from(completed));
        let unapproved = tempfile::tempdir().unwrap();
        f.storage.native_models_root = unapproved.path().to_owned();
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        assert!(Source::capture(&f.storage, &snapshot, &|| false).is_err());
    }
}

#[test]
fn deletion_repeat_moves_flat_identity_history_has_per_record_and_aggregate_caps() {
    let f = Fixture::new(true);
    let before = f.original();
    let mut source = capture(&f.storage);
    let root = HistoricalRoot {
        root: source.root.clone(),
        database: source.database.clone(),
    };
    source.records[0].historical_roots = vec![root.clone(); MAX_ROOTS_PER_RECORD];
    assert!(source.verify_size().is_ok());
    source.records[0].historical_roots.push(root);
    assert!(source.verify_size().is_err());
    source.records[0].historical_roots.clear();
    let actual = &source.directories[0];
    let history = HistoricalDirectory {
        relative_path: actual.relative_path.clone(),
        historical: actual.identity.clone(),
        current: actual.identity.clone(),
    };
    source.historical_directories = vec![history; MAX_PATHS];
    assert!(source.verify_size().is_err());
    assert_eq!(f.original(), before);
}
