use super::*;
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    file: PathBuf,
    other_file: PathBuf,
    external: PathBuf,
    operation: String,
}
impl Fixture {
    fn new() -> Self {
        Self::at(tempfile::tempdir().unwrap())
    }
    fn at(base: tempfile::TempDir) -> Self {
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage
            .create_meeting("TARGET DB IS KEPT UNTIL ROW PHASE")
            .unwrap()
            .id;
        let other = storage.create_meeting("KEEP OTHER").unwrap().id;
        let file = storage
            .root()
            .join(format!("meetings/{meeting}/audio/source.flac"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"PRIVATE SOURCE").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                file.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        let other_file = storage
            .root()
            .join(format!("meetings/{other}/audio/source.flac"));
        std::fs::create_dir_all(other_file.parent().unwrap()).unwrap();
        std::fs::write(&other_file, b"KEEP OTHER AUDIO").unwrap();
        let external = base.path().join("external.md");
        std::fs::write(&external, b"KEEP EXTERNAL EXPORT").unwrap();
        let model = base.path().join("models/model.gguf");
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(model, b"KEEP MODEL").unwrap();
        Self {
            base,
            storage,
            meeting,
            other,
            file,
            other_file,
            external,
            operation: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn prepared(&self, include: bool) -> (Prepared, Reviewed) {
        let census = Census::capture(&self.storage, &self.meeting, &|| false).unwrap();
        let review = Reviewed::confirm(&census, include).unwrap();
        let prepared = intent::prepare(&self.storage, &self.operation, &census, &|| false).unwrap();
        (prepared, review)
    }
    fn begin(&self, include: bool) -> Receipt {
        let (p, v) = self.prepared(include);
        start(&self.storage, &p, &v, &|| false).unwrap()
    }
    fn unaffected(&self) {
        assert_eq!(
            std::fs::read(self.base.path().join("models/model.gguf")).unwrap(),
            b"KEEP MODEL"
        );
        assert_eq!(
            std::fs::read(&self.other_file).unwrap(),
            b"KEEP OTHER AUDIO"
        );
        assert_eq!(
            std::fs::read(&self.external).unwrap(),
            b"KEEP EXTERNAL EXPORT"
        );
        assert!(self.storage.get_meeting(&self.other).unwrap().is_some());
        assert!(self.storage.get_meeting(&self.meeting).unwrap().is_some());
        assert!(self.storage.data_write_blocked());
    }
}
#[test]
fn file_removal_erases_fixed_tree_keeps_database_observations_and_other_data_and_replays() {
    let f = Fixture::new();
    let mut receipt = f.begin(false);
    let db = f.storage.connection.lock().unwrap();
    let original:(String,String,i64,i64)=db.query_row("SELECT target_manifest_json,target_hash,consent_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",params![f.operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    drop(db);
    run(&f.storage, &mut receipt, &|| false).unwrap();
    assert!(receipt.removed());
    assert!(!f
        .storage
        .root()
        .join(format!("meetings/{}", f.meeting))
        .exists());
    let db = f.storage.connection.lock().unwrap();
    let after:(String,String,i64,i64)=db.query_row("SELECT target_manifest_json,target_hash,consent_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1) FROM maintenance_operations WHERE id=?1",params![f.operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(original, after);
    drop(db);
    let mut replay = read(&f.storage, &f.operation, &|| false).unwrap();
    run(&f.storage, &mut replay, &|| false).unwrap();
    f.unaffected();
}
#[test]
fn file_removal_retains_missing_registered_observations_without_adopting_new_files() {
    let f = Fixture::new();
    std::fs::remove_file(&f.file).unwrap();
    let mut r = f.begin(false);
    run(&f.storage, &mut r, &|| false).unwrap();
    assert!(r.removed());
    f.unaffected();
}
#[test]
fn file_removal_empty_draft_completes_without_creating_a_meeting_folder() {
    let base = tempfile::tempdir().unwrap();
    let storage = Storage::open(base.path().join("root")).unwrap();
    let meeting = storage.create_meeting("DRAFT").unwrap();
    let census = Census::capture(&storage, &meeting.id, &|| false).unwrap();
    let v = Reviewed::confirm(&census, false).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let p = intent::prepare(&storage, &operation, &census, &|| false).unwrap();
    let mut r = start(&storage, &p, &v, &|| false).unwrap();
    run(&storage, &mut r, &|| false).unwrap();
    assert!(r.removed());
    assert!(!storage.root().join("meetings").exists());
    assert!(storage.get_meeting(&meeting.id).unwrap().is_some());
}
#[test]
fn file_removal_unregistered_objects_require_explicit_review_not_producer_hints() {
    let f = Fixture::new();
    let session = uuid::Uuid::new_v4().to_string();
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE meetings SET source_type='recording' WHERE id=?1",
        params![f.meeting],
    )
    .unwrap();
    db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES(?1,?2,'interrupted','system','{}',?1,1,1)",params![session,f.meeting]).unwrap();
    drop(db);
    let hinted = f
        .storage
        .root()
        .join(format!("meetings/{}/audio/{session}/system.caf", f.meeting));
    std::fs::create_dir_all(hinted.parent().unwrap()).unwrap();
    std::fs::write(&hinted, b"UNREGISTERED RECORDING").unwrap();
    let registered = hinted.parent().unwrap().join("microphone.caf");
    std::fs::write(&registered, b"REGISTERED HINTED SOURCE").unwrap();
    f.storage
        .register_ready_meeting_asset(
            &f.meeting,
            registered.strip_prefix(f.storage.root()).unwrap(),
            "audio/caf",
        )
        .unwrap();
    let unknown = f.file.parent().unwrap().join("unknown.txt");
    std::fs::write(&unknown, b"EXPLICITLY REVIEWED UNKNOWN").unwrap();
    let census = Census::capture(&f.storage, &f.meeting, &|| false).unwrap();
    assert!(census
        .manifest()
        .entries
        .iter()
        .any(|e| e.stage_hint.as_deref() == Some("recording")));
    assert!(Reviewed::confirm(&census, false).is_err());
    let v = Reviewed::confirm(&census, true).unwrap();
    let p = intent::prepare(&f.storage, &f.operation, &census, &|| false).unwrap();
    let mut r = start(&f.storage, &p, &v, &|| false).unwrap();
    run(&f.storage, &mut r, &|| false).unwrap();
    assert!(!unknown.exists());
    assert!(!hinted.exists());
    f.unaffected();
}
#[test]
fn file_removal_refuses_changed_review_or_live_files_before_start() {
    let f = Fixture::new();
    let (p, v) = f.prepared(false);
    let foreign = Fixture::new();
    let other_census = Census::capture(&foreign.storage, &foreign.meeting, &|| false).unwrap();
    let other_review = Reviewed::confirm(&other_census, false).unwrap();
    assert!(start(&f.storage, &p, &other_review, &|| false).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    std::fs::write(&f.file, b"CHANGED BEFORE START").unwrap();
    assert!(start(&f.storage, &p, &v, &|| false).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"CHANGED BEFORE START");
    f.unaffected();
}
#[test]
fn file_removal_refuses_changes_after_start_before_any_mutation() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    std::fs::write(&f.file, b"CHANGED AFTER START").unwrap();
    assert!(run(&f.storage, &mut r, &|| false).is_err());
    assert_eq!(r.progress.next, 0);
    assert_eq!(std::fs::read(&f.file).unwrap(), b"CHANGED AFTER START");
    f.unaffected();
}
#[test]
fn file_removal_refuses_another_pending_operation_before_resuming_file_io() {
    for state in ["prepared", "installing", "cleanup", "attention"] {
        let f = Fixture::new();
        let mut receipt = f.begin(false);
        let db = f.storage.connection.lock().unwrap();
        let _writer =
            super::super::fence::write_removal(&f.storage, &db, &receipt, &|| false).unwrap();
        db.execute(
            "INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets',?3,'{}',?4,'{}',1,1)",
            params![uuid::Uuid::new_v4().to_string(), f.other, state, "0".repeat(64)],
        )
        .unwrap();
        drop(_writer);
        drop(db);
        assert!(run(&f.storage, &mut receipt, &|| false).is_err());
        assert_eq!(receipt.progress.next, 0);
        assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
        f.unaffected();
    }
}
#[test]
fn file_removal_cancel_at_each_durable_boundary_cold_reopens_and_finishes_remaining_only() {
    for phase in ["armed", "renamed", "staged", "unlinked", "advanced"] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        let stop = Cell::new(false);
        assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|p| {
            if p == phase {
                stop.set(true);
            }
        })
        .is_err());
        assert!(!r.removed());
        let root = f.storage.root().to_owned();
        let operation = f.operation.clone();
        let other_file = f.other_file.clone();
        let external = f.external.clone();
        drop(f.storage);
        let storage = Storage::open(root).unwrap();
        let mut saved = read(&storage, &operation, &|| false).unwrap();
        run(&storage, &mut saved, &|| false).unwrap();
        assert!(saved.removed());
        assert!(storage.data_write_blocked());
        assert_eq!(std::fs::read(other_file).unwrap(), b"KEEP OTHER AUDIO");
        assert_eq!(std::fs::read(external).unwrap(), b"KEEP EXTERNAL EXPORT");
    }
}
#[test]
fn file_removal_cancellation_after_rename_retains_staged_source_instead_of_false_restore() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|p| {
        if p == "renamed" {
            stop.set(true);
        }
    })
    .is_err());
    assert!(!f.file.exists());
    let staged = f.file.parent().unwrap().join(r.reserved());
    assert_eq!(std::fs::read(&staged).unwrap(), b"PRIVATE SOURCE");
    assert!(r.progress.pending.as_ref().unwrap().staged.is_none());
    let mut saved = read(&f.storage, &f.operation, &|| false).unwrap();
    run(&f.storage, &mut saved, &|| false).unwrap();
    assert!(saved.removed());
    f.unaffected();
}
#[test]
fn file_removal_preserves_unknown_added_during_partial_removal() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let unknown = f.file.parent().unwrap().join("new.txt");
    let added = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| false, &|p| {
        if p == "advanced" && !added.replace(true) {
            std::fs::write(&unknown, b"KEEP NEW UNKNOWN").unwrap();
        }
    })
    .is_err());
    assert!(!r.removed());
    assert!(!f.file.exists());
    assert_eq!(std::fs::read(&unknown).unwrap(), b"KEEP NEW UNKNOWN");
    let mut saved = read(&f.storage, &f.operation, &|| false).unwrap();
    assert!(run(&f.storage, &mut saved, &|| false).is_err());
    f.unaffected();
}

#[test]
fn file_removal_reserved_collision_never_overwrites_observed_data() {
    let f = Fixture::new();
    let reserved = f
        .file
        .parent()
        .unwrap()
        .join(format!(".mon-delete-{}-0", f.operation));
    std::fs::write(&reserved, b"KEEP COLLISION").unwrap();
    let (p, v) = f.prepared(true);
    assert!(start(&f.storage, &p, &v, &|| false).is_err());
    assert_eq!(std::fs::read(reserved).unwrap(), b"KEEP COLLISION");
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}
#[test]
fn file_removal_source_name_race_moves_back_wrong_object_without_deleting_it() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let saved = f.base.path().join("kept-original.flac");
    let raced = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| false, &|p| {
        if p == "beforeRename" && !raced.replace(true) {
            std::fs::rename(&f.file, &saved).unwrap();
            std::fs::write(&f.file, b"KEEP WRONG OBJECT").unwrap();
        }
    })
    .is_err());
    assert_eq!(std::fs::read(saved).unwrap(), b"PRIVATE SOURCE");
    assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP WRONG OBJECT");
    assert!(!r.removed());
    f.unaffected();
}
#[test]
fn file_removal_changed_reserved_object_before_unlink_is_retained() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let reserved = f.file.parent().unwrap().join(r.reserved());
    let saved = f.base.path().join("kept-staged.flac");
    let raced = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| false, &|p| {
        if p == "beforeUnlink" && !raced.replace(true) {
            std::fs::rename(&reserved, &saved).unwrap();
            std::fs::write(&reserved, b"KEEP WRONG RESERVED").unwrap();
        }
    })
    .is_err());
    assert_eq!(std::fs::read(saved).unwrap(), b"PRIVATE SOURCE");
    assert_eq!(std::fs::read(reserved).unwrap(), b"KEEP WRONG RESERVED");
    assert!(r.progress.pending.as_ref().unwrap().staged.is_some());
    f.unaffected();
}
#[test]
fn file_removal_late_links_hardlinks_and_fifo_are_not_followed_or_removed() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    for kind in ["symlink", "hardlink", "fifo"] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        let saved = f.base.path().join("keep-source.flac");
        std::fs::rename(&f.file, &saved).unwrap();
        match kind {
            "symlink" => symlink(&f.external, &f.file).unwrap(),
            "hardlink" => std::fs::hard_link(&saved, &f.file).unwrap(),
            _ => {
                let p = std::ffi::CString::new(f.file.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
            }
        }
        assert!(run(&f.storage, &mut r, &|| false).is_err());
        assert_eq!(std::fs::read(saved).unwrap(), b"PRIVATE SOURCE");
        assert!(std::fs::symlink_metadata(&f.file).is_ok());
        f.unaffected();
    }
}
#[test]
fn file_removal_parent_replacement_does_not_adopt_same_named_copies() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    let saved = f.base.path().join("keep-audio");
    std::fs::rename(f.file.parent().unwrap(), &saved).unwrap();
    std::fs::create_dir(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"PRIVATE SOURCE").unwrap();
    assert!(run(&f.storage, &mut r, &|| false).is_err());
    assert_eq!(
        std::fs::read(saved.join("source.flac")).unwrap(),
        b"PRIVATE SOURCE"
    );
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}
#[test]
fn file_removal_progress_corruption_and_skipped_cursor_never_grant_removal() {
    for (key, value) in [
        ("version", serde_json::json!(9)),
        ("manifestHash", serde_json::json!("a".repeat(64))),
        ("planHash", serde_json::json!("a".repeat(64))),
        ("next", serde_json::json!(usize::MAX)),
        ("pending", serde_json::json!({"unexpected":true})),
        ("extra", serde_json::json!(true)),
        ("filePhase", serde_json::json!("removed")),
    ] {
        let f = Fixture::new();
        let r = f.begin(false);
        let db = f.storage.connection.lock().unwrap();
        let _writer = super::super::fence::write_removal(&f.storage, &db, &r, &|| false).unwrap();
        let mut json = serde_json::to_value(&r.progress).unwrap();
        json[key] = value;
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![f.operation, serde_json::to_string(&json).unwrap()],
        )
        .unwrap();
        drop(_writer);
        drop(db);
        assert!(read(&f.storage, &f.operation, &|| false).is_err());
        assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
        f.unaffected();
    }
    let f = Fixture::new();
    let r = f.begin(false);
    let db = f.storage.connection.lock().unwrap();
    let _writer = super::super::fence::write_removal(&f.storage, &db, &r, &|| false).unwrap();
    let mut progress = r.progress.clone();
    progress.next = 1;
    db.execute(
        "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
        params![f.operation, progress.encode().unwrap()],
    )
    .unwrap();
    drop(_writer);
    drop(db);
    let mut saved = read(&f.storage, &f.operation, &|| false).unwrap();
    assert!(run(&f.storage, &mut saved, &|| false).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"PRIVATE SOURCE");
    f.unaffected();
}
#[test]
fn file_removal_completed_receipt_refuses_recreated_meeting_tree() {
    let f = Fixture::new();
    let mut r = f.begin(false);
    run(&f.storage, &mut r, &|| false).unwrap();
    std::fs::create_dir_all(f.file.parent().unwrap()).unwrap();
    std::fs::write(&f.file, b"KEEP RECREATED").unwrap();
    assert!(run(&f.storage, &mut r, &|| false).is_err());
    assert_eq!(std::fs::read(&f.file).unwrap(), b"KEEP RECREATED");
    f.unaffected();
}

#[test]
fn file_removal_database_replacement_between_checkpoints_stops_before_next_filesystem_mutation() {
    for phase in ["beforeRename", "beforeUnlink"] {
        let f = Fixture::new();
        let mut r = f.begin(false);
        let database = f.storage.root().join("meetings.sqlite3");
        let kept = f.base.path().join("kept.sqlite3");
        let changed = Cell::new(false);
        assert!(run_observed(&f.storage, &mut r, &|| false, &|point| {
            if point == phase && !changed.replace(true) {
                std::fs::rename(&database, &kept).unwrap();
                std::fs::write(&database, b"KEEP REPLACEMENT DB WITHOUT SQLITE INIT").unwrap();
            }
        })
        .is_err());
        assert!(kept.is_file());
        assert_eq!(
            std::fs::read(database).unwrap(),
            b"KEEP REPLACEMENT DB WITHOUT SQLITE INIT"
        );
        let source = if phase == "beforeRename" {
            f.file.clone()
        } else {
            f.file.parent().unwrap().join(r.reserved())
        };
        assert_eq!(std::fs::read(source).unwrap(), b"PRIVATE SOURCE");
        assert!(f.storage.data_write_blocked());
        assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
        assert_eq!(std::fs::read(&f.external).unwrap(), b"KEEP EXTERNAL EXPORT");
    }
}
#[test]
fn file_removal_long_original_path_cold_reopens_reserved_path_beyond_census_limit() {
    let f = Fixture::new();
    let file = f
        .storage
        .root()
        .join(format!("meetings/{}/{}/f", f.meeting, "z".repeat(185)));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"LONG PATH SOURCE").unwrap();
    f.storage
        .register_ready_meeting_asset(
            &f.meeting,
            file.strip_prefix(f.storage.root()).unwrap(),
            "audio/flac",
        )
        .unwrap();
    let mut r = f.begin(false);
    assert_eq!(
        r.target().unwrap().path,
        file.strip_prefix(f.storage.root())
            .unwrap()
            .to_str()
            .unwrap()
    );
    let stop = Cell::new(false);
    assert!(run_observed(&f.storage, &mut r, &|| stop.get(), &|p| {
        if p == "staged" {
            stop.set(true);
        }
    })
    .is_err());
    let staged = file.parent().unwrap().join(r.reserved());
    assert!(
        staged
            .strip_prefix(f.storage.root())
            .unwrap()
            .as_os_str()
            .len()
            > 256
    );
    assert_eq!(std::fs::read(staged).unwrap(), b"LONG PATH SOURCE");
    let root = f.storage.root().to_owned();
    drop(f.storage);
    let storage = Storage::open(root).unwrap();
    let mut r = read(&storage, &f.operation, &|| false).unwrap();
    run(&storage, &mut r, &|| false).unwrap();
    assert!(r.removed());
    assert_eq!(std::fs::read(&f.other_file).unwrap(), b"KEEP OTHER AUDIO");
}
#[test]
fn file_removal_many_explicit_files_keep_progress_bounded_and_complete_all_observed_paths() {
    let f = Fixture::new();
    for n in 0..200 {
        std::fs::write(
            f.file.parent().unwrap().join(format!("z-{n:04}.txt")),
            b"EXPLICIT REVIEW",
        )
        .unwrap();
    }
    let mut r = f.begin(true);
    assert!(serde_json::to_vec(r.confirmed.manifest()).unwrap().len() > 65536);
    assert!(r.progress.encode().unwrap().len() < 4096);
    run(&f.storage, &mut r, &|| false).unwrap();
    assert!(r.removed());
    assert_eq!(r.progress.next, 203);
    assert!(r.progress.encode().unwrap().len() < 4096);
    assert!(!f.file.parent().unwrap().exists());
    f.unaffected();
}

#[test]
#[ignore = "subprocess fixture; invoked by file removal SIGKILL parent"]
fn file_removal_checkpoint_child_fixture() {
    let base = PathBuf::from(std::env::var("MON_REMOVE_TEST_BASE").unwrap());
    let phase = std::env::var("MON_REMOVE_TEST_PHASE").unwrap();
    let f = Fixture::at(tempfile::tempdir_in(&base).unwrap());
    let mut r = f.begin(false);
    let renamed = Cell::new(0usize);
    run_observed(&f.storage, &mut r, &|| false, &|point| {
        if point == "renamed" {
            renamed.set(renamed.get() + 1);
        }
        let matches = phase == point
            || (phase == "directoryRenamed" && point == "renamed" && renamed.get() == 2)
            || (phase == "rootRenamed" && point == "renamed" && renamed.get() == 3);
        if matches {
            let receipt = serde_json::to_vec(&(
                f.storage.root(),
                &f.operation,
                &f.meeting,
                &f.other,
                &f.other_file,
                &f.external,
            ))
            .unwrap();
            std::fs::write(base.join("ready.tmp"), receipt).unwrap();
            std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    })
    .unwrap();
    panic!("removal checkpoint not reached");
}
#[test]
fn file_removal_sigkill_at_eight_boundaries_reopens_and_finishes_without_touching_other_data() {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "armed",
        "renamed",
        "staged",
        "unlinked",
        "advanced",
        "filesRemoved",
        "directoryRenamed",
        "rootRenamed",
    ] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::removal::tests::file_removal_checkpoint_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_REMOVE_TEST_BASE", base.path())
            .env("MON_REMOVE_TEST_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let ready = base.path().join("ready.json");
        let start = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || start.elapsed() > std::time::Duration::from_secs(20)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("removal checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (root, operation, meeting, other, other_file, external): (
            PathBuf,
            String,
            String,
            String,
            PathBuf,
            PathBuf,
        ) = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        assert!(root
            .canonicalize()
            .unwrap()
            .starts_with(base.path().canonicalize().unwrap()));
        let storage = Storage::open(root).unwrap();
        assert!(storage.data_write_blocked());
        let mut r = read(&storage, &operation, &|| false).unwrap();
        run(&storage, &mut r, &|| false).unwrap();
        assert!(r.removed());
        assert!(!storage.root().join(format!("meetings/{meeting}")).exists());
        assert!(storage.get_meeting(&meeting).unwrap().is_some());
        assert!(storage.get_meeting(&other).unwrap().is_some());
        assert_eq!(std::fs::read(other_file).unwrap(), b"KEEP OTHER AUDIO");
        assert_eq!(std::fs::read(external).unwrap(), b"KEEP EXTERNAL EXPORT");
        assert!(storage.data_write_blocked());
    }
}
