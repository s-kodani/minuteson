use super::*;
struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    config: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new(overlap: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let data = base.join("app-data");
        crate::native_fs::private_directories(&data, false).unwrap();
        let config = if overlap {
            data.clone()
        } else {
            base.join("config")
        };
        if !overlap {
            crate::native_fs::private_directories(&config, false).unwrap();
        }
        // A pre-bootstrap legacy image is rejected before Storage admission.
        let root = if overlap || cfg!(windows) {
            data.clone()
        } else {
            data.join("data")
        };
        if root != data {
            crate::native_fs::private_directories(&root, false).unwrap();
        }
        write_new(
            &root.join(DB),
            b"unrecognized old database, intentionally not SQLite",
        )
        .unwrap();
        for p in [
            root.join("meetings/old"),
            data.join("models/large"),
            data.join("model-downloads/partial"),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(root.join("meetings/old/audio.flac"), b"old audio").unwrap();
        std::fs::write(
            data.join("models/large/model.bin"),
            b"MODEL BODY MUST NOT BE COPIED",
        )
        .unwrap();
        std::fs::write(
            data.join("model-downloads/partial/payload"),
            b"PARTIAL MODEL MUST NOT BE COPIED",
        )
        .unwrap();
        std::fs::write(root.join("existing.monbackup"), b"external backup").unwrap();
        std::fs::create_dir_all(data.join("logs")).unwrap();
        std::fs::write(data.join("logs/running.log"), b"active log").unwrap();
        Self {
            _temp: temp,
            data,
            config,
            root,
        }
    }
    fn prepare(&self) -> (Bootstrap, Selection, String) {
        let b = open(&self.config, &self.data).unwrap();
        let s = inspect(&b, &self.data).unwrap();
        assert_eq!(s.root(), self.root);
        (b, s, Uuid::new_v4().to_string())
    }
    fn fresh(&self) {
        let (storage, b) = crate::startup_attention::admit_storage(&self.config, &self.data)
            .map_err(|e| format!("{e:?}"))
            .unwrap();
        assert_eq!(storage.root(), self.root);
        assert!(storage.list_meetings().unwrap().is_empty());
        assert!(!self.data.join("models/large/model.bin").exists());
        assert!(!self.data.join("model-downloads").exists());
        assert!(self.root.join("existing.monbackup").is_file());
        assert_eq!(
            std::fs::read(self.data.join("logs/running.log")).unwrap(),
            b"active log"
        );
        assert!(!self.config.join(JOURNAL).exists());
        drop(storage);
        drop(b);
    }
    fn old(&self) {
        assert_eq!(
            std::fs::read(self.root.join(DB)).unwrap(),
            b"unrecognized old database, intentionally not SQLite"
        );
        assert!(self.root.join("meetings/old/audio.flac").is_file());
        assert!(self.data.join("models/large/model.bin").is_file());
        assert!(!self.config.join(JOURNAL).exists());
    }
}
#[test]
fn legacy_reset_with_and_without_archive_excludes_model_payloads_even_with_overlapping_roots() {
    for overlap in [false, true] {
        for archive in [false, true] {
            let f = Fixture::new(overlap);
            assert!(crate::startup_attention::admit_storage(&f.config, &f.data).is_err());
            let (b, s, id) = f.prepare();
            let parent = f._temp.path().canonicalize().unwrap();
            let to = destination(&s, &parent, &id).unwrap();
            execute(b, &s, &id, archive.then_some(to.as_path()), &|_| {}).unwrap();
            f.fresh();
            if archive {
                let mut found_audio = false;
                for item in std::fs::read_dir(&to).unwrap() {
                    let p = item.unwrap().path();
                    let n = p.file_name().unwrap().to_str().unwrap();
                    assert!(!n.contains("models") && !n.contains("model-downloads"));
                    if p.join("old/audio.flac").is_file() {
                        found_audio = true;
                    }
                    if p.is_file() {
                        let bytes = std::fs::read(p).unwrap();
                        assert!(!bytes.windows(10).any(|b| b == b"MODEL BODY"));
                    }
                }
                assert!(found_audio);
                assert!(!to.extension().is_some_and(|x| x == "monbackup"));
            } else {
                assert!(!to.exists());
            }
        }
    }
}
#[test]
fn interruption_recovers_before_normal_admission_at_every_transaction_stage() {
    for stage in [
        "prepared",
        "move",
        "staged",
        "fresh_reserved",
        "fresh_installed",
        "database",
        "published",
        "committed",
        "cleanup",
    ] {
        let f = Fixture::new(false);
        let (b, s, id) = f.prepare();
        FAIL.with(|p| *p.borrow_mut() = Some(stage.into()));
        let e = execute(b, &s, &id, None, &|_| {}).unwrap_err();
        assert_eq!(e, "reset test interruption", "{stage}");
        FAIL.with(|p| *p.borrow_mut() = None);
        assert!(f.config.join(JOURNAL).is_file());
        assert!(Storage::open_with_models(f.root.clone(), f.data.join("models")).is_err());
        recover(&f.config, &f.data).unwrap_or_else(|e| panic!("{stage}: {e}"));
        if ["published", "committed", "cleanup"].contains(&stage) {
            f.fresh();
        } else {
            f.old();
        }
        recover(&f.config, &f.data).unwrap();
    }
}
#[test]
fn archive_error_and_bad_destinations_leave_the_old_data_untouched() {
    let f = Fixture::new(true);
    let (b, s, id) = f.prepare();
    for bad in [&f.root, &f.data.join("models"), &f.config] {
        assert!(destination(&s, bad, &id).is_err());
    }
    let to = destination(&s, &f._temp.path().canonicalize().unwrap(), &id).unwrap();
    std::fs::create_dir(&to).unwrap();
    assert!(execute(b, &s, &id, Some(&to), &|_| {}).is_err());
    f.old();
}
#[test]
fn changed_source_and_competing_storage_are_rejected() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    let lease = RootLease::acquire(&f.root, true).unwrap();
    assert!(RootLease::acquire(&f.root, true).is_err());
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    drop(lease);
    f.old();
    let (b, s, id) = f.prepare();
    let old = f.root.join("preserved-db");
    std::fs::rename(f.root.join(DB), &old).unwrap();
    std::fs::write(f.root.join(DB), b"replacement database").unwrap();
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    assert_eq!(
        std::fs::read(f.root.join(DB)).unwrap(),
        b"replacement database"
    );
    assert!(old.is_file());
}
#[cfg(unix)]
#[test]
fn symlinked_archive_data_is_rejected_but_model_contents_are_never_read() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    symlink(
        f.data.join("models/large/model.bin"),
        f.root.join("meetings/link"),
    )
    .unwrap();
    let to = destination(&s, &f._temp.path().canonicalize().unwrap(), &id).unwrap();
    assert!(execute(b, &s, &id, Some(&to), &|_| {}).is_err());
    f.old();
    std::fs::remove_file(f.root.join("meetings/link")).unwrap();
    let (b, s, id) = f.prepare();
    std::fs::set_permissions(
        f.data.join("models/large/model.bin"),
        std::fs::Permissions::from_mode(0),
    )
    .unwrap();
    let to = destination(&s, &f._temp.path().canonicalize().unwrap(), &id).unwrap();
    execute(b, &s, &id, Some(&to), &|_| {}).unwrap();
    f.fresh();
}
#[test]
fn recovery_never_deletes_a_replacement_database() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    FAIL.with(|p| *p.borrow_mut() = Some("committed".into()));
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    FAIL.with(|p| *p.borrow_mut() = None);
    std::fs::rename(f.root.join(DB), f.root.join("new-db-kept")).unwrap();
    std::fs::write(f.root.join(DB), b"new data after interrupted reset").unwrap();
    assert!(recover(&f.config, &f.data).is_err());
    assert_eq!(
        std::fs::read(f.root.join(DB)).unwrap(),
        b"new data after interrupted reset"
    );
    assert!(f.root.join("new-db-kept").is_file());
}
#[test]
fn registered_rejected_schema_resets_settings_and_keeps_the_selected_root() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let data = base.join("data");
    let config = base.join("config");
    crate::native_fs::private_directories(&data, false).unwrap();
    let b = Bootstrap::open(&config, &data).unwrap();
    let storage = b.open_storage().unwrap();
    let root = storage.root().to_owned();
    storage.create_meeting("消える会議").unwrap();
    storage
        .save_ui_preferences(
            0,
            serde_json::from_value(
                serde_json::json!({"schemaVersion":1,"theme":"dark","colorTheme":"purple"}),
            )
            .unwrap(),
        )
        .unwrap();
    crate::storage::with_test_database(&storage, |db| {
        db.execute("INSERT INTO model_packages(id,engine,model_name,model_version,source_type,manifest_json,manifest_hash,state) VALUES('old-model','whisper_cpp','old','1','download','{}',?1,'unavailable')", ["a".repeat(64)]).unwrap();
    });
    drop(storage);
    drop(b);
    let db = rusqlite::Connection::open(root.join(DB)).unwrap();
    db.execute_batch("DELETE FROM schema_migrations WHERE version>33")
        .unwrap();
    drop(db);
    assert!(crate::startup_attention::admit_storage(&config, &data).is_err());
    let b = open(&config, &data).unwrap();
    let s = inspect(&b, &data).unwrap();
    let id = Uuid::new_v4().to_string();
    let to = destination(&s, &base, &id).unwrap();
    execute(b, &s, &id, Some(&to), &|_| {}).unwrap();
    let (fresh, b) = crate::startup_attention::admit_storage(&config, &data)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    assert_eq!(fresh.root(), root);
    assert!(fresh.list_meetings().unwrap().is_empty());
    assert_eq!(
        fresh.ui_preferences().unwrap().payload,
        crate::storage::UiPreferences::default()
    );
    assert!(to.join("bootstrap.json").is_file());
    let old_image = std::fs::read_dir(&to)
        .unwrap()
        .map(|d| d.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-meetings.sqlite3")
        })
        .unwrap();
    let old = rusqlite::Connection::open_with_flags(
        old_image,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        old.query_row("SELECT count(*) FROM model_packages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(fresh);
    drop(b);
}
#[test]
fn archive_parent_and_workspace_replacements_are_never_adopted() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    let parent = f._temp.path().canonicalize().unwrap().join("archive");
    std::fs::create_dir(&parent).unwrap();
    let archive = Archive::select(&s, &parent, &id).unwrap();
    std::fs::rename(&parent, parent.with_file_name("kept-archive")).unwrap();
    std::fs::create_dir(&parent).unwrap();
    assert!(archive.verify().is_err());
    drop(b);
    f.old();
    let (b, s, id) = f.prepare();
    FAIL.with(|p| *p.borrow_mut() = Some("staged".into()));
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    FAIL.with(|p| *p.borrow_mut() = None);
    let (_, j): (String, Journal) =
        serde_json::from_slice(&read_private(&f.config.join(JOURNAL), MAX_RECORD).unwrap())
            .unwrap();
    let workspace = j.workspace(j.workspaces[0].root);
    let kept = workspace.with_file_name("preserved-workspace");
    std::fs::rename(&workspace, &kept).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    assert!(recover(&f.config, &f.data).is_err());
    assert!(kept.is_dir());
}
#[test]
fn rollback_itself_can_be_interrupted_without_deleting_restored_data() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    FAIL.with(|p| *p.borrow_mut() = Some("database".into()));
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    FAIL.with(|p| *p.borrow_mut() = None);
    let (_, j): (String, Journal) =
        serde_json::from_slice(&read_private(&f.config.join(JOURNAL), MAX_RECORD).unwrap())
            .unwrap();
    // Simulate a crash after new file cleanup and the old DB's restore rename.
    for fresh in &j.fresh {
        let path = f.root.join(&fresh.name);
        if exists(&path).unwrap() {
            remove_owned(&path, &fresh.identity).unwrap();
        }
    }
    let (i, entry) = j
        .entries
        .iter()
        .enumerate()
        .find(|(_, e)| j.source(e) == f.root.join(DB))
        .unwrap();
    rename(&j.target(i, entry), &j.source(entry), false).unwrap();
    recover(&f.config, &f.data).unwrap();
    f.old();
}

#[test]
fn disk_full_and_interrupted_copy_do_not_change_any_source_data() {
    for fault in ["disk_full", "copy_failure"] {
        let f = Fixture::new(false);
        let (b, s, id) = f.prepare();
        let to = destination(&s, &f._temp.path().canonicalize().unwrap(), &id).unwrap();
        FAIL.with(|p| *p.borrow_mut() = Some(fault.into()));
        assert!(execute(b, &s, &id, Some(&to), &|_| {}).is_err());
        FAIL.with(|p| *p.borrow_mut() = None);
        f.old();
        assert!(to.join("MON-reset-incomplete.json").is_file());
        assert!(!to.join("MON-reset-info.json").exists());
    }
}
#[cfg(windows)]
#[test]
fn windows_locked_file_rolls_back_precommit_renames() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.root.join(DB))
        .unwrap();
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    drop(locked);
    f.old();
}

#[test]
fn reset_refuses_a_live_capture_lease_without_reading_its_ownership_database() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    let area = f.data.join(if cfg!(windows) {
        ".mon-capture-tests-windows-v1"
    } else {
        ".mon-capture-tests"
    });
    create_directory(&area).unwrap();
    let lease = area.join(format!("lease-{}", Uuid::new_v4()));
    write_new(&lease, b"").unwrap();
    let owner = open_file(&lease, libc::O_RDWR).unwrap();
    owner.try_lock().unwrap();
    assert!(execute(b, &s, &id, None, &|_| {}).is_err());
    drop(owner);
    f.old();
    let (b, s, id) = f.prepare();
    execute(b, &s, &id, None, &|_| {}).unwrap();
    f.fresh();
    assert!(!area.exists());
}

#[test]
fn interrupted_archive_has_a_durable_receipt_and_recovers_without_changing_sources() {
    let f = Fixture::new(false);
    let (b, s, id) = f.prepare();
    let to = destination(&s, &f._temp.path().canonicalize().unwrap(), &id).unwrap();
    FAIL.with(|p| *p.borrow_mut() = Some("archiving".into()));
    assert!(execute(b, &s, &id, Some(&to), &|_| {}).is_err());
    FAIL.with(|p| *p.borrow_mut() = None);
    let (_, j): (String, Journal) =
        serde_json::from_slice(&read_private(&f.config.join(JOURNAL), MAX_RECORD).unwrap())
            .unwrap();
    assert!(j.phase == Phase::Archiving);
    assert!(j.workspaces.is_empty());
    recover(&f.config, &f.data).unwrap();
    f.old();
}

#[cfg(unix)]
#[test]
#[ignore = "child process for the duplicate-open lock regression"]
fn sqlite_lock_child() {
    let root = std::env::var_os("MON_RESET_SQLITE_LOCK_ROOT").expect("synthetic child root");
    let db = rusqlite::Connection::open(PathBuf::from(root).join(DB)).unwrap();
    db.busy_timeout(std::time::Duration::ZERO).unwrap();
    let error = db
        .query_row("SELECT count(*) FROM meetings", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect_err("the original SQLite connection must remain exclusive");
    assert!(
        matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        ),
        "{error}"
    );
}
#[cfg(unix)]
#[test]
fn duplicate_storage_open_preserves_the_original_sqlite_process_locks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let storage = Storage::open(root.clone()).unwrap();
    storage.create_meeting("retained exclusive owner").unwrap();
    assert!(Storage::open(root.clone()).is_err());
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bootstrap::reset::tests::sqlite_lock_child",
            "--ignored",
            "--test-threads=1",
        ])
        .env("MON_RESET_SQLITE_LOCK_ROOT", &root)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}
