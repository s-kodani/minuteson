use super::*;
use crate::storage::current_minutes_fixture as minutes_fixture;
use std::cell::Cell;
use std::os::unix::fs::{symlink, PermissionsExt};

mod retirement;

struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    relative: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = storage.create_meeting("移動前の会議").unwrap().id;
        let relative = format!("meetings/{meeting}/audio/original.flac");
        let path = storage.root.join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"original audio").unwrap();
        storage
            .register_ready_meeting_asset(&meeting, Path::new(&relative), "audio/flac")
            .unwrap();
        Self {
            _root: root,
            storage,
            meeting,
            relative,
        }
    }
    fn path(&self) -> PathBuf {
        self.storage.root.join(&self.relative)
    }
}
fn snapshot(f: &Fixture) -> NativeSnapshot<'_> {
    f.storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap()
}
fn native_tables(db: &Connection) -> (String, Vec<Table>) {
    census(db, &crate::test_support::never_cancelled).unwrap()
}

#[test]
fn placed_current_database_is_validated_without_migration_and_keeps_cancellation() {
    let f = Fixture::new();
    let mut db = f.storage.connection.lock().unwrap();
    let before = native_tables(&db);
    validate_database(&mut db, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(native_tables(&db), before);
    verify_database_schema(&db).unwrap();
    assert!(validate_database(&mut db, &|| true).is_err());
    assert_eq!(native_tables(&db), before);
}

#[test]
fn placed_old_database_schema_is_rejected_without_mutation() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);
        INSERT INTO schema_migrations VALUES (38, 'old', 0);
        CREATE TABLE preserved(payload TEXT NOT NULL); INSERT INTO preserved VALUES ('keep');").unwrap();
    assert!(validate_database(&mut db, &crate::test_support::never_cancelled).is_err());
    assert_eq!(
        db.query_row("SELECT payload FROM preserved", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn native_image_preserves_wal_ids_all_tables_settings_and_unregistered_audio() {
    let f = Fixture::new();
    let unregistered = f
        .storage
        .root
        .join(format!("meetings/{}/audio/unregistered.part", f.meeting));
    std::fs::write(&unregistered, b"unregistered recording").unwrap();
    let temp = f.storage.root.join("temp/interrupted-job");
    std::fs::create_dir_all(temp.join("empty")).unwrap();
    std::fs::write(temp.join("processing.log"), b"local log").unwrap();
    let settings = r#"{"path":"/Users/local/models","theme":"dark"}"#;
    {
        let db = f.storage.connection.lock().unwrap();
        db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('local-test',?1,?2,1,1)", params![settings,digest(settings.as_bytes())]).unwrap();
    }
    assert!(
        f.storage
            .root
            .join("meetings.sqlite3-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    let saved = snapshot(&f);
    assert_eq!(saved.manifest.version, 1);
    assert_eq!(saved.manifest.assets[0].meeting_id, f.meeting);
    assert_eq!(saved.manifest.assets[0].relative_path, f.relative);
    let source = f.storage.connection.lock().unwrap();
    let target = read_only(&saved.image.join("meetings.sqlite3")).unwrap();
    assert_eq!(native_tables(&source), native_tables(&target));
    let kept: String = target
        .query_row(
            "SELECT value_json FROM app_settings WHERE key='local-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, settings);
    assert_eq!(
        target
            .query_row("SELECT id FROM meetings", [], |r| r.get::<_, String>(0))
            .unwrap(),
        f.meeting
    );
    drop(source);
    assert_eq!(
        std::fs::read(saved.image.join(&f.relative)).unwrap(),
        b"original audio"
    );
    assert_eq!(
        std::fs::read(
            saved
                .image
                .join(unregistered.strip_prefix(&f.storage.root).unwrap())
        )
        .unwrap(),
        b"unregistered recording"
    );
    assert_eq!(
        std::fs::read(saved.image.join("temp/interrupted-job/processing.log")).unwrap(),
        b"local log"
    );
    assert!(saved.image.join("temp/interrupted-job/empty").is_dir());
    assert!(!saved.image.join("meetings.sqlite3-wal").exists());
    assert!(!saved.image.join("meetings.sqlite3-shm").exists());
    for node in &saved.copied {
        let mode = std::fs::metadata(saved.image.join(&node.entry.relative_path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
    assert_eq!(
        saved.manifest_hash(),
        digest(&serde_json::to_vec(saved.manifest()).unwrap())
    );
    saved.verify(&crate::test_support::never_cancelled).unwrap();
    saved
        .verify_source(&crate::test_support::never_cancelled)
        .unwrap();
}

#[test]
fn native_image_preserves_current_draft_markdown_and_local_model_conditions() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, analysis, _) = crate::storage::playback::tests::fixture(&storage);
    storage
        .create_transcript_draft(&meeting, &analysis)
        .unwrap();
    minutes_fixture::candidate(&storage, &meeting);
    minutes_fixture::save(&storage, &meeting, "# 移動後にも保持する編集\n");
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    draft
        .payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: storage
                .load_current_analysis(&meeting)
                .unwrap()
                .unwrap()
                .utterances[0]
                .id
                .clone(),
            effective_text: "文字起こしの編集".into(),
            speaker_ids: vec![],
        });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let saved = storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        native_tables(&storage.connection.lock().unwrap()),
        native_tables(&read_only(&saved.image.join("meetings.sqlite3")).unwrap())
    );
    let rows = &saved.manifest.tables;
    assert!(rows
        .iter()
        .any(|t| t.name == "working_drafts" && t.rows > 0));
    assert!(rows
        .iter()
        .any(|t| t.name == "minutes_markdown" && t.rows == 1));
}

#[test]
fn copied_image_can_open_same_meeting_after_old_managed_tree_is_unavailable() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    let copy = Storage::open(saved.image.clone()).unwrap();
    let old = f.storage.root.join("meetings");
    let hidden = f.storage.root.join("retained-old-meetings");
    std::fs::rename(&old, &hidden).unwrap();
    assert_eq!(
        copy.get_meeting(&f.meeting).unwrap().unwrap().title,
        "移動前の会議"
    );
    assert_eq!(
        std::fs::read(copy.root.join(&f.relative)).unwrap(),
        b"original audio"
    );
    // Opening Storage changes only the temporary destination (WAL/registry).
    // It is not allowed to substitute for the sealed pre-cutover image.
    assert!(saved.verify(&crate::test_support::never_cancelled).is_err());
    assert!(saved
        .verify_source(&crate::test_support::never_cancelled)
        .is_err());
    drop(copy);
    std::fs::rename(&hidden, &old).unwrap();
}

#[test]
fn write_fence_lasts_through_verification_and_releases_only_on_drop() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    assert!(f.storage.create_meeting("拒否する書込").is_err());
    assert!(f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    assert!(f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    saved
        .verify_source(&crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    let stage = saved.stage.path().to_owned();
    drop(saved);
    assert!(!stage.exists());
    f.storage.create_meeting("解除後の書込").unwrap();
    assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
}

#[test]
fn root_models_bootstrap_and_owned_control_journals_are_not_copied_or_deleted() {
    let f = Fixture::new();
    let names = [
        "models/package/model.bin",
        "bootstrap.json",
        ".bootstrap.lock",
        "foreign.txt",
    ];
    for name in names {
        let path = f.storage.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, name.as_bytes()).unwrap();
    }
    let registry = f.storage.root.join(".mon-scratch.sqlite3");
    let identity = Identity::read(&registry.metadata().unwrap()).unwrap();
    let saved = snapshot(&f);
    for name in names {
        assert!(!saved.image.join(name).exists());
        assert_eq!(
            std::fs::read(f.storage.root.join(name)).unwrap(),
            name.as_bytes()
        );
    }
    assert!(!saved.image.join(".mon-scratch.sqlite3").exists());
    drop(saved);
    assert_eq!(
        Identity::read(&registry.metadata().unwrap()).unwrap(),
        identity
    );
}

#[test]
fn ready_mismatch_or_missing_body_refuses_without_changing_source_or_fence() {
    for body in [Some(b"wrong audio".as_slice()), None] {
        let f = Fixture::new();
        match body {
            Some(bytes) => std::fs::write(f.path(), bytes).unwrap(),
            None => std::fs::remove_file(f.path()).unwrap(),
        }
        assert!(f
            .storage
            .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
            .is_err());
        assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
        f.storage.create_meeting("元DBは利用可能").unwrap();
        assert_eq!(std::fs::read(f.path()).ok(), body.map(Vec::from));
    }
}

#[test]
fn missing_and_corrupt_states_keep_native_metadata_without_portable_cache_exceptions() {
    let f = Fixture::new();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE file_assets SET state='missing'", [])
        .unwrap();
    std::fs::remove_file(f.path()).unwrap();
    let saved = snapshot(&f);
    assert_eq!(saved.manifest.assets[0].state, "missing");
    assert!(!saved.image.join(&f.relative).exists());
    drop(saved);
    std::fs::write(f.path(), b"broken retained audio").unwrap();
    assert!(f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE file_assets SET state='corrupt'", [])
        .unwrap();
    let saved = snapshot(&f);
    assert_eq!(saved.manifest.assets[0].state, "corrupt");
    assert_eq!(
        std::fs::read(saved.image.join(&f.relative)).unwrap(),
        b"broken retained audio"
    );
    assert_eq!(saved.manifest.assets[0].hash, digest(b"original audio"));
}

#[test]
fn links_and_special_files_refuse_without_following_them() {
    for kind in ["symlink", "hardlink", "directory-link", "fifo"] {
        let f = Fixture::new();
        let outside = f.storage.root.join("foreign-secret");
        std::fs::write(&outside, b"not meeting data").unwrap();
        let name = f
            .storage
            .root
            .join(format!("meetings/{}/unknown", f.meeting));
        match kind {
            "symlink" => symlink(&outside, &name).unwrap(),
            "hardlink" => std::fs::hard_link(&outside, &name).unwrap(),
            "directory-link" => symlink(&f.storage.root, &name).unwrap(),
            "fifo" => {
                use std::os::unix::ffi::OsStrExt;
                let path = std::ffi::CString::new(name.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            }
            _ => unreachable!(),
        }
        assert!(
            f.storage
                .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
                .is_err(),
            "{kind}"
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"not meeting data");
        assert!(std::fs::symlink_metadata(name).is_ok());
        f.storage.create_meeting("拒否後も元DB").unwrap();
    }
}

#[test]
fn exact_path_inventory_rejects_added_removed_modified_and_replaced_source_files() {
    for kind in ["add", "remove", "modify", "replace"] {
        let f = Fixture::new();
        let saved = snapshot(&f);
        match kind {
            "add" => std::fs::write(f.path().with_file_name("new.part"), b"new").unwrap(),
            "remove" => std::fs::remove_file(f.path()).unwrap(),
            "modify" => std::fs::write(f.path(), b"changed audio!").unwrap(),
            "replace" => {
                std::fs::rename(f.path(), f.storage.root.join("retained-original")).unwrap();
                std::fs::write(f.path(), b"original audio").unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            saved
                .verify_source(&crate::test_support::never_cancelled)
                .is_err(),
            "{kind}"
        );
        saved.verify(&crate::test_support::never_cancelled).unwrap();
    }
}

#[test]
fn sealed_image_rejects_payload_manifest_db_and_extra_root_changes() {
    for kind in ["payload", "replace", "manifest", "database", "extra"] {
        let f = Fixture::new();
        let saved = snapshot(&f);
        let path = saved.image.join(&f.relative);
        match kind {
            "payload" => std::fs::write(path, b"changed audio!").unwrap(),
            "replace" => {
                std::fs::rename(&path, saved.stage.path().join("retained-file")).unwrap();
                std::fs::write(&path, b"original audio").unwrap();
            }
            "manifest" => {
                std::fs::write(saved.stage.path().join("relocation-manifest.json"), b"{}").unwrap()
            }
            "database" => {
                std::fs::write(saved.image.join("meetings.sqlite3"), b"not a DB").unwrap()
            }
            "extra" => std::fs::write(saved.image.join("unconfirmed"), b"keep").unwrap(),
            _ => unreachable!(),
        }
        assert!(
            saved.verify(&crate::test_support::never_cancelled).is_err(),
            "{kind}"
        );
        saved
            .verify_source(&crate::test_support::never_cancelled)
            .unwrap();
        assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
    }
}

#[test]
fn cancellation_mid_copy_cleans_only_owned_stage_and_keeps_originals() {
    let f = Fixture::new();
    let called = Cell::new(false);
    let cancel = || {
        let during_copy = std::fs::read_dir(&f.storage.root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                e.file_name().to_string_lossy().starts_with(".mon-stage-")
                    && e.path().join("native").join(&f.relative).exists()
            });
        if during_copy {
            called.set(true);
        }
        during_copy
    };
    assert!(f.storage.prepare_relocation_snapshot(&cancel).is_err());
    assert!(called.get());
    assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
    assert!(!std::fs::read_dir(&f.storage.root)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().starts_with(".mon-stage-")));
    f.storage.create_meeting("取消後の元DB").unwrap();
}

#[test]
fn source_changed_during_copy_is_rejected_without_activating_any_destination() {
    let f = Fixture::new();
    let changed = Cell::new(false);
    let inspect = || {
        let copying = std::fs::read_dir(&f.storage.root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                e.file_name().to_string_lossy().starts_with(".mon-stage-")
                    && e.path().join("native").join(&f.relative).exists()
            });
        if copying && !changed.replace(true) {
            std::fs::write(f.path(), b"changed audio!").unwrap();
        }
        false
    };
    assert!(f.storage.prepare_relocation_snapshot(&inspect).is_err());
    assert!(changed.get());
    assert_eq!(std::fs::read(f.path()).unwrap(), b"changed audio!");
    f.storage.create_meeting("元保存先を維持").unwrap();
}

#[test]
fn census_detects_type_content_duplicate_count_and_schema_without_row_order_dependence() {
    let a = Connection::open_in_memory().unwrap();
    let b = Connection::open_in_memory().unwrap();
    for db in [&a, &b] {
        db.execute_batch("CREATE TABLE payload(value); INSERT INTO payload VALUES(1),('1'),(x'31'),(NULL),(1.0);").unwrap();
    }
    assert_eq!(native_tables(&a), native_tables(&b));
    b.execute_batch(
        "DELETE FROM payload; INSERT INTO payload VALUES(NULL),(1.0),(x'31'),('1'),(1);",
    )
    .unwrap();
    assert_eq!(native_tables(&a), native_tables(&b));
    b.execute(
        "UPDATE payload SET value='different' WHERE value IS NULL",
        [],
    )
    .unwrap();
    assert_ne!(native_tables(&a), native_tables(&b));
    b.execute("UPDATE payload SET value=NULL WHERE value='different'", [])
        .unwrap();
    assert_eq!(native_tables(&a), native_tables(&b));
    b.execute("INSERT INTO payload VALUES(1)", []).unwrap();
    assert_ne!(native_tables(&a), native_tables(&b));
    b.execute_batch("CREATE TABLE extra(value);").unwrap();
    assert_ne!(native_tables(&a).0, native_tables(&b).0);
}

#[test]
fn root_escaping_registered_paths_are_rejected_without_reading_external_payloads() {
    for relative in [
        "../foreign",
        "models/model",
        "meetings/wrong/audio.flac",
        "meetings//audio.flac",
    ] {
        let f = Fixture::new();
        f.storage
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE file_assets SET relative_path=?1", params![relative])
            .unwrap();
        assert!(
            f.storage
                .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
                .is_err(),
            "{relative}"
        );
        assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
    }
}

#[test]
fn active_recording_refuses_before_allocating_any_native_copy() {
    let f = Fixture::new();
    f.storage
        .prepare_recording(
            &f.meeting,
            &crate::storage::RecordingConfig {
                mode: "system".into(),
                backend: "screencapturekit".into(),
                target_pids: vec![],
                microphone_device_uid: None,
                target_application_identity: None,
            },
            "fixture-active",
        )
        .unwrap();
    assert!(f
        .storage
        .prepare_relocation_snapshot(&crate::test_support::never_cancelled)
        .err()
        .expect("active recording must be refused")
        .contains("停止"));
    assert!(!std::fs::read_dir(&f.storage.root)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().starts_with(".mon-stage-")));
    assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
}

#[test]
fn source_database_path_substitution_cannot_hide_behind_the_old_open_connection() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    let db = f.storage.root.join("meetings.sqlite3");
    let hidden = f.storage.root.join("retained-original-db");
    std::fs::rename(&db, &hidden).unwrap();
    std::fs::copy(saved.image.join("meetings.sqlite3"), &db).unwrap();
    assert!(saved
        .verify_source(&crate::test_support::never_cancelled)
        .is_err());
    saved.verify(&crate::test_support::never_cancelled).unwrap();
    assert!(hidden.exists());
}

#[test]
fn source_database_same_inode_rewrite_link_and_permissions_are_rejected() {
    for kind in ["rewrite", "hardlink", "permissions"] {
        let f = Fixture::new();
        let saved = snapshot(&f);
        let path = f.storage.root.join("meetings.sqlite3");
        let identity = Identity::read(&path.metadata().unwrap()).unwrap();
        match kind {
            "rewrite" => {
                let bytes = std::fs::read(&path).unwrap();
                // SQLite's cached logical rows may remain unchanged. The native
                // source file itself was touched outside the held write fence.
                std::fs::write(&path, bytes).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&path, f.storage.root.join("foreign-db-link")).unwrap()
            }
            "permissions" => {
                let mode = path.metadata().unwrap().permissions().mode();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode ^ 0o020))
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(Identity::read(&path.metadata().unwrap()).unwrap(), identity);
        assert!(
            saved
                .verify_source(&crate::test_support::never_cancelled)
                .is_err(),
            "{kind}"
        );
        saved.verify(&crate::test_support::never_cancelled).unwrap();
        drop(saved);
        assert!(f
            .storage
            .create_meeting("不確定DBへの書込は再開しない")
            .is_err());
    }
}

#[test]
fn staged_root_identity_and_private_permissions_are_part_of_the_seal() {
    for kind in ["identity", "permissions"] {
        let f = Fixture::new();
        let saved = snapshot(&f);
        if kind == "permissions" {
            std::fs::set_permissions(&saved.image, std::fs::Permissions::from_mode(0o755)).unwrap();
        } else {
            let old = saved.stage.path().join("retained-native");
            std::fs::rename(&saved.image, &old).unwrap();
            std::fs::create_dir(&saved.image).unwrap();
            std::fs::set_permissions(&saved.image, std::fs::Permissions::from_mode(0o700)).unwrap();
            for child in std::fs::read_dir(&old).unwrap() {
                let child = child.unwrap();
                std::fs::rename(child.path(), saved.image.join(child.file_name())).unwrap();
            }
        }
        assert!(
            saved.verify(&crate::test_support::never_cancelled).is_err(),
            "{kind}"
        );
        saved
            .verify_source(&crate::test_support::never_cancelled)
            .unwrap();
    }
}

#[test]
fn native_relocation_crash_child() {
    let Ok(raw) = std::env::var("MON_RELOCATION_CRASH_CHILD") else {
        return;
    };
    let (root, marker, mode): (PathBuf, PathBuf, String) = serde_json::from_str(&raw).unwrap();
    let storage = Storage::open(root).unwrap();
    let asset_path: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT relative_path FROM file_assets LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let ready = |stage: &Path| -> ! {
        let temporary = marker.with_extension("tmp");
        std::fs::write(&temporary, stage.to_string_lossy().as_bytes()).unwrap();
        File::open(&temporary).unwrap().sync_all().unwrap();
        std::fs::rename(temporary, &marker).unwrap();
        loop {
            std::thread::park();
        }
    };
    let cancel = || {
        if mode == "partial" {
            for entry in std::fs::read_dir(&storage.root).unwrap() {
                let entry = entry.unwrap();
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mon-stage-")
                {
                    let path = entry.path().join("native").join(&asset_path);
                    if path.is_file() {
                        ready(&entry.path());
                    }
                }
            }
        }
        false
    };
    let saved = storage.prepare_relocation_snapshot(&cancel).unwrap();
    ready(saved.stage.path());
}

#[test]
fn sigkill_during_native_copy_or_after_sealing_recovers_only_owned_copy_and_keeps_old_db_audio() {
    use std::process::{Command, Stdio};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for mode in ["partial", "sealed"] {
        let f = Fixture::new();
        let before = native_tables(&f.storage.connection.lock().unwrap());
        let root = f.storage.root.clone();
        let meeting = f.meeting.clone();
        let relative = f.relative.clone();
        let marker_root = tempfile::tempdir().unwrap();
        let marker = marker_root.path().join("ready");
        drop(f.storage);
        let mut child = Child(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::backup::relocation::tests::native_relocation_crash_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(
                    "MON_RELOCATION_CRASH_CHILD",
                    serde_json::to_string(&(&root, &marker, mode)).unwrap(),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !marker.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before {mode}"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "child never reached {mode}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let stage = PathBuf::from(std::fs::read_to_string(&marker).unwrap());
        assert!(stage.join("native").exists());
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let reopened = Storage::open(root.clone()).unwrap();
        assert_eq!(native_tables(&reopened.connection.lock().unwrap()), before);
        assert!(reopened.get_meeting(&meeting).unwrap().is_some());
        assert_eq!(
            std::fs::read(root.join(relative)).unwrap(),
            b"original audio"
        );
        assert!(!stage.exists(), "owned {mode} image should be recovered");
        reopened.create_meeting("強制終了後も旧保存先").unwrap();
    }
}

#[test]
fn native_image_copies_registry_owned_voice_clips_and_validates_their_hashes() {
    let (_root, storage, meeting, speaker) = crate::storage::voice::tests::fixture();
    let person = crate::storage::voice::tests::register(&storage, &meeting, &speaker);
    let id = storage.voice_people().unwrap()[0].samples[0].id.clone();
    let relative = crate::storage::voice::audio::relative(&person, &id).unwrap();
    let bytes = std::fs::read(storage.root().join(&relative)).unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    snapshot.verify(&|| false).unwrap();
    assert_eq!(
        std::fs::read(snapshot.image().join(&relative)).unwrap(),
        bytes
    );
    let file = snapshot.image().join(relative);
    let mut changed = bytes;
    changed[44] ^= 1;
    std::fs::write(file, changed).unwrap();
    assert!(snapshot.verify(&|| false).is_err());
}
