use super::*;
use std::io::Write;

fn fixture() -> (tempfile::TempDir, Storage) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    (root, storage)
}
fn state(registry: &Registry, id: &str) -> String {
    registry
        .db
        .lock()
        .unwrap()
        .query_row("SELECT state FROM areas WHERE id=?1", params![id], |r| {
            r.get(0)
        })
        .unwrap()
}
#[test]
fn scratch_reopen_recovers_owned_partial_files_nested_stages_and_not_the_source_or_meetings() {
    let (root, storage) = fixture();
    let meeting = storage.create_meeting("既存の会議").unwrap();
    let source = root.path().join("external.monbackup");
    std::fs::write(&source, b"untouched source").unwrap();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    let id = stage.owner.as_ref().unwrap().1.clone();
    directories(&path.join("assets")).unwrap();
    private_file(&path.join("assets/partial"))
        .unwrap()
        .write_all(b"partial bytes")
        .unwrap();
    let child = Stage::create(&path).unwrap();
    private_file(&child.path().join("validation.sqlite3"))
        .unwrap()
        .write_all(b"unfinished sqlite")
        .unwrap();
    child.abandon();
    stage.abandon();
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert!(!path.exists());
    assert_eq!(state(reopened.scratch.as_ref().unwrap(), &id), "recovered");
    assert_eq!(
        reopened.get_meeting(&meeting.id).unwrap().unwrap().title,
        "既存の会議"
    );
    assert_eq!(std::fs::read(source).unwrap(), b"untouched source");
}
#[test]
fn scratch_normal_drop_only_removes_registered_objects_and_unrecorded_creation_gap_is_retained() {
    let (_root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    let id = stage.owner.as_ref().unwrap().1.clone();
    private_file(&path.join("owned"))
        .unwrap()
        .write_all(b"owned")
        .unwrap();
    std::fs::write(path.join("unknown"), b"keep unknown").unwrap();
    drop(stage);
    assert!(!path.join("owned").exists());
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"keep unknown"
    );
    assert_eq!(state(storage.scratch.as_ref().unwrap(), &id), "attention");
    storage.scratch.as_ref().unwrap().recover().unwrap();
    assert!(path.join("unknown").exists());
    std::fs::remove_file(path.join("unknown")).unwrap();
    storage.scratch.as_ref().unwrap().recover().unwrap();
    assert!(!path.exists());
    assert_eq!(state(storage.scratch.as_ref().unwrap(), &id), "recovered");
}
#[test]
fn scratch_replaced_leaf_and_parent_are_not_adopted_or_followed() {
    use std::os::unix::fs::symlink;
    let (_root, storage) = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("foreign"), b"foreign").unwrap();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    directories(&path.join("assets")).unwrap();
    private_file(&path.join("assets/foreign")).unwrap();
    let saved = path.join("assets-original");
    std::fs::rename(path.join("assets"), &saved).unwrap();
    symlink(outside.path(), path.join("assets")).unwrap();
    drop(stage);
    assert_eq!(
        std::fs::read(outside.path().join("foreign")).unwrap(),
        b"foreign"
    );
    assert!(saved.join("foreign").exists());
    assert!(path
        .join("assets")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
}
#[test]
fn scratch_hard_links_and_same_name_replacements_preserve_the_foreign_object() {
    let (_root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    let outside = tempfile::tempdir().unwrap();
    private_file(&path.join("linked"))
        .unwrap()
        .write_all(b"linked bytes")
        .unwrap();
    std::fs::hard_link(path.join("linked"), outside.path().join("link")).unwrap();
    private_file(&path.join("replaced")).unwrap();
    std::fs::rename(path.join("replaced"), outside.path().join("old")).unwrap();
    std::fs::write(path.join("replaced"), b"foreign replacement").unwrap();
    drop(stage);
    assert_eq!(std::fs::read(path.join("linked")).unwrap(), b"linked bytes");
    assert_eq!(
        std::fs::read(path.join("replaced")).unwrap(),
        b"foreign replacement"
    );
    assert_eq!(
        std::fs::read(outside.path().join("link")).unwrap(),
        b"linked bytes"
    );
}
#[test]
fn scratch_live_prepared_copy_survives_storage_reopen_and_legacy_named_trees_are_not_adopted() {
    let (root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    private_file(&path.join("live"))
        .unwrap()
        .write_all(b"still live")
        .unwrap();
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(std::fs::read(path.join("live")).unwrap(), b"still live");
    drop(stage);
    assert!(!path.exists());
    drop(reopened);
    let stage_root = root
        .path()
        .join(".mon-stage-00000000-0000-0000-0000-000000000000");
    std::fs::create_dir(&stage_root).unwrap();
    std::fs::write(stage_root.join("unregistered"), b"legacy copy").unwrap();
    let again = Storage::open(root.path().into()).unwrap();
    assert!(stage_root.join("unregistered").exists());
    drop(again);
}
#[test]
fn scratch_changed_schema_and_invalid_manifest_preserve_owned_material() {
    let (_root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_path_buf();
    let id = stage.owner.as_ref().unwrap().1.clone();
    private_file(&path.join("owned")).unwrap();
    let registry = storage.scratch.as_ref().unwrap();
    registry
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER entries_fixed;")
        .unwrap();
    registry
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE entries SET hash=?1 WHERE area=?2 AND kind='file'",
            params!["0".repeat(64), id],
        )
        .unwrap();
    drop(stage);
    assert!(path.join("owned").exists());
    assert_eq!(state(registry, &id), "active");
    let definition = registry
        .expected_schema
        .iter()
        .find(|(_, name, _)| name == "entries_fixed")
        .unwrap()
        .2
        .clone();
    registry
        .db
        .lock()
        .unwrap()
        .execute_batch(&definition)
        .unwrap();
    registry.recover().unwrap();
    assert!(path.join("owned").exists());
    assert_eq!(state(registry, &id), "attention");
}

#[test]
fn scratch_removed_entry_replay_never_adopts_a_same_name_foreign_file() {
    let (_root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_owned();
    let id = stage.owner.as_ref().unwrap().1.clone();
    private_file(&path.join("owned")).unwrap();
    std::fs::write(path.join("unknown"), b"unknown").unwrap();
    drop(stage);
    assert!(!path.join("owned").exists());
    std::fs::write(path.join("owned"), b"foreign replacement after unlink").unwrap();
    storage.scratch.as_ref().unwrap().recover().unwrap();
    assert_eq!(
        std::fs::read(path.join("owned")).unwrap(),
        b"foreign replacement after unlink"
    );
    assert_eq!(state(storage.scratch.as_ref().unwrap(), &id), "attention");
}

#[test]
fn scratch_parent_replacement_stops_file_and_directory_creation_before_touching_the_outside() {
    use std::os::unix::fs::symlink;
    let (_root, storage) = fixture();
    let outside = tempfile::tempdir().unwrap();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_owned();
    directories(&path.join("assets")).unwrap();
    std::fs::rename(path.join("assets"), path.join("old-assets")).unwrap();
    symlink(outside.path(), path.join("assets")).unwrap();
    assert!(private_file(&path.join("assets/new-file")).is_err());
    assert!(directories(&path.join("assets/new-directory")).is_err());
    assert!(Stage::create(&path.join("assets")).is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn scratch_new_storage_root_at_same_path_is_never_used_for_cleanup() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("root");
    let saved = outer.path().join("original-root");
    let storage = Storage::open(root.clone()).unwrap();
    let stage = storage.backup_stage().unwrap();
    let name = stage.path().file_name().unwrap().to_owned();
    private_file(&stage.path().join("owned")).unwrap();
    std::fs::rename(&root, &saved).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join(&name)).unwrap();
    std::fs::write(root.join(&name).join("owned"), b"foreign root").unwrap();
    drop(stage);
    assert!(saved.join(&name).join("owned").exists());
    assert_eq!(
        std::fs::read(root.join(&name).join("owned")).unwrap(),
        b"foreign root"
    );
    assert!(Registry::open(&root).is_err());
    drop(storage);
}

// Executed only by the subprocess test below. A real SIGKILL does not run any
// Stage destructor, unlike the in-process abandonment fixtures above.
#[test]
fn scratch_crash_child() {
    let Some(root) = std::env::var_os("MON_SCRATCH_TEST_ROOT") else {
        return;
    };
    let storage = Storage::open(PathBuf::from(root)).unwrap();
    let mode = std::env::var("MON_SCRATCH_TEST_MODE").unwrap();
    fn ready(storage: &Storage) -> ! {
        let relative: String = storage
            .scratch
            .as_ref()
            .unwrap()
            .db
            .lock()
            .unwrap()
            .query_row("SELECT path FROM areas WHERE state='active'", [], |r| {
                r.get(0)
            })
            .unwrap();
        println!(
            "MON_SCRATCH_READY={}",
            storage.root.join(relative).display()
        );
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    match mode.as_str() {
        "snapshot" => {
            let _snapshot = storage.prepare_backup_snapshot(&|| false).unwrap();
            ready(&storage);
        }
        "restore" => {
            let source = PathBuf::from(std::env::var_os("MON_SCRATCH_TEST_SOURCE").unwrap());
            let _selected = storage
                .prepare_additive_restore(&source, &|| false, &|_| {})
                .unwrap();
            ready(&storage);
        }
        "partial" | "gap" => {
            let stage = storage.backup_stage().unwrap();
            let child = Stage::create(stage.path()).unwrap();
            let mut file = private_file(&child.path().join("unfinished-payload")).unwrap();
            file.write_all(&vec![7; 512 * 1024]).unwrap();
            file.sync_all().unwrap();
            if mode == "gap" {
                std::fs::write(stage.path().join("unregistered-gap"), b"keep unowned bytes")
                    .unwrap();
            }
            ready(&storage);
        }
        _ => panic!("unexpected test mode"),
    }
}

#[test]
fn scratch_sigkill_reopen_recovers_real_snapshot_restore_and_partial_copies_but_keeps_creation_gaps(
) {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (source_root, source) = fixture();
    let source_meeting = archive::tests::meeting(&source, "source fixture");
    let output = tempfile::tempdir().unwrap();
    let archive_path = output.path().join("source.monbackup");
    source
        .create_backup_archive(&archive_path, &|| false)
        .unwrap();
    let pending: i64 = source
        .scratch
        .as_ref()
        .unwrap()
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM areas WHERE state!='recovered'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pending, 0,
        "normal backup leaves no unresolved construction area"
    );
    let archive_digest = digest_file(&archive_path, &|| false).unwrap();
    for mode in ["snapshot", "restore", "partial", "gap"] {
        let (root, target) = fixture();
        let existing = archive::tests::meeting(&target, "existing fixture");
        let before = super::super::publication::tests::all_rows(&target.connection.lock().unwrap());
        drop(target);
        let mut child = Child(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::backup::scratch::tests::scratch_crash_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("MON_SCRATCH_TEST_ROOT", root.path())
                .env("MON_SCRATCH_TEST_SOURCE", &archive_path)
                .env("MON_SCRATCH_TEST_MODE", mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if let Some((_, path)) = line.split_once("MON_SCRATCH_READY=") {
                    let _ = sender.send(PathBuf::from(path));
                    break;
                }
            }
        });
        let stage = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("child failed to prepare its private copy");
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        assert!(!status.success());
        reader.join().unwrap();
        let reopened = Storage::open(root.path().into()).unwrap();
        assert_eq!(
            super::super::publication::tests::all_rows(&reopened.connection.lock().unwrap()),
            before
        );
        assert!(reopened.get_meeting(&existing).unwrap().is_some());
        let registry = reopened.scratch.as_ref().unwrap();
        let statuses: Vec<String> = registry
            .db
            .lock()
            .unwrap()
            .prepare("SELECT state FROM areas")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        if mode == "gap" {
            assert_eq!(
                std::fs::read(stage.join("unregistered-gap")).unwrap(),
                b"keep unowned bytes"
            );
            assert_eq!(std::fs::read_dir(&stage).unwrap().count(), 1);
            assert_eq!(statuses, vec!["attention"]);
        } else {
            assert!(!stage.exists(), "{mode}: owned tree remains");
            assert_eq!(statuses, vec!["recovered"]);
        }
        assert_eq!(
            digest_file(&archive_path, &|| false).unwrap(),
            archive_digest
        );
    }
    assert!(source.get_meeting(&source_meeting).unwrap().is_some());
    assert!(source_root.path().exists());
}

#[test]
fn scratch_empty_unproven_journal_is_not_initialized_or_used_to_adopt_named_trees() {
    let root = tempfile::tempdir().unwrap();
    let journal = root.path().join(".mon-scratch.sqlite3");
    private_file(&journal).unwrap();
    let tree = root
        .path()
        .join(".mon-stage-00000000-0000-0000-0000-000000000000");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("unknown"), b"unknown bootstrap gap").unwrap();
    assert!(Storage::open(root.path().into()).is_err());
    assert_eq!(journal.metadata().unwrap().len(), 0);
    assert_eq!(
        std::fs::read(tree.join("unknown")).unwrap(),
        b"unknown bootstrap gap"
    );
}

#[test]
fn scratch_changed_private_directory_permissions_hold_the_tree_for_attention() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, storage) = fixture();
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_owned();
    let id = stage.owner.as_ref().unwrap().1.clone();
    private_file(&path.join("owned")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    drop(stage);
    assert!(path.join("owned").exists());
    assert_eq!(state(storage.scratch.as_ref().unwrap(), &id), "attention");
}
