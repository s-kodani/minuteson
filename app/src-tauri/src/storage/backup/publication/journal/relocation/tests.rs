use super::*;
use crate::bootstrap::Bootstrap;
use crate::storage::backup::publication::tests::{fixture, ids, sealed, Fixture};
use std::os::unix::fs::DirBuilderExt;

struct Pending {
    id: String,
    batch: String,
    file: String,
    directory: String,
}
fn pending(f: &Fixture, mode: &str) -> Pending {
    let batch = Uuid::new_v4().to_string();
    let ids = ids(f, &batch);
    let sealed = sealed(f, &ids);
    let targets = targets(&sealed).unwrap();
    let file = targets
        .iter()
        .find(|t| t.kind == "file")
        .unwrap()
        .path
        .clone();
    let directory = file.split('/').take(2).collect::<Vec<_>>().join("/");
    let db = f.target.connection.lock().unwrap();
    let journal = Journal::begin(&db, &f.target.root, &sealed, &ids).unwrap();
    for relative in [&directory, &format!("{directory}/restored")] {
        let path = f.target.root.join(relative);
        std::fs::create_dir(&path).unwrap();
        journal
            .record(&path, &std::fs::symlink_metadata(&path).unwrap())
            .unwrap();
    }
    let path = f.target.root.join(&file);
    std::fs::write(&path, b"incomplete owned restore").unwrap();
    if mode != "gap" {
        journal
            .record(&path, &std::fs::symlink_metadata(&path).unwrap())
            .unwrap();
    }
    if mode == "substituted" {
        std::fs::rename(&path, f.target.root.join(&directory).join("old-owned-body")).unwrap();
        std::fs::write(&path, b"substituted unowned restore").unwrap();
    }
    if mode == "unknown" {
        std::fs::write(
            f.target.root.join(&directory).join("unknown"),
            b"unknown body",
        )
        .unwrap();
    }
    Pending {
        id: journal.id.clone(),
        batch,
        file,
        directory,
    }
}
fn move_to(f: &Fixture) -> (tempfile::TempDir, PathBuf) {
    move_storage(&f.target)
}
fn move_storage(source: &Storage) -> (tempfile::TempDir, PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let parent = base.path().canonicalize().unwrap();
    let config = parent.join("config");
    let data = parent.join("data");
    let b = Bootstrap::open(&config, &data).unwrap();
    let seed = Storage::open(b.root().into()).unwrap();
    b.initialize(&seed).unwrap();
    drop(seed);
    drop(b);
    crate::bootstrap::set_test_root(&config, &data, &source.root);
    let b = Bootstrap::open(&config, &data).unwrap();
    let target = parent.join("destination");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let native = source.prepare_relocation_snapshot(&|| false).unwrap();
    let op = Uuid::new_v4().to_string();
    b.begin_move(
        &op,
        &Uuid::new_v4().to_string(),
        &target,
        native.manifest_hash(),
        0,
    )
    .unwrap();
    let mut p = b.pending_placement(&op, &native).unwrap();
    p.copy(&native, &|| false).unwrap();
    let h = native.prepare_control_handoff(&|| false).unwrap();
    let before = std::fs::read(target.join("meetings.sqlite3")).unwrap();
    p.install_history(&h, &|| false).unwrap();
    p.verify(&native, &|| false).unwrap();
    assert_eq!(
        std::fs::read(target.join("meetings.sqlite3")).unwrap(),
        before
    );
    assert_eq!(b.view().unwrap().meeting_root, source.root);
    (base, target)
}
fn state(storage: &Storage, id: &str) -> String {
    storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
}
fn count(target: &Path) -> i64 {
    Connection::open(target.join(".mon-scratch.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM relocation_restore_bindings",
            [],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn restore_rebinding_recovers_only_registered_copies_keeps_unknown_and_leaves_source_unchanged() {
    let f = fixture();
    let pending = pending(&f, "unknown");
    let old = f.target.root.join(&pending.file);
    let old_bytes = std::fs::read(&old).unwrap();
    let original_manifest: String = f
        .target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![pending.id],
            |r| r.get(0),
        )
        .unwrap();
    let (_base, target) = move_to(&f);
    assert_eq!(count(&target), 1);
    let s = Storage::open(target.clone()).unwrap();
    assert!(!target.join(&pending.file).exists());
    assert_eq!(
        std::fs::read(target.join(&pending.directory).join("unknown")).unwrap(),
        b"unknown body"
    );
    assert_eq!(std::fs::read(old).unwrap(), old_bytes);
    assert_eq!(state(&s, &pending.id), "attention");
    let manifest: String = s
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT target_manifest_json FROM maintenance_operations WHERE id=?1",
            params![pending.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(manifest, original_manifest);
    drop(s);
    let again = Storage::open(target.clone()).unwrap();
    assert_eq!(state(&again, &pending.id), "attention");
    assert!(f.target.root.join(&pending.file).exists());
}
#[test]
fn restore_rebinding_never_adopts_source_creation_gap_or_substituted_inode() {
    for mode in ["gap", "substituted"] {
        let f = fixture();
        let p = pending(&f, mode);
        let (_base, target) = move_to(&f);
        let before = std::fs::read(target.join(&p.file)).unwrap();
        let s = Storage::open(target.clone()).unwrap();
        assert_eq!(std::fs::read(target.join(&p.file)).unwrap(), before);
        assert_eq!(std::fs::read(f.target.root.join(&p.file)).unwrap(), before);
        assert_eq!(state(&s, &p.id), "attention");
    }
}
#[test]
fn restore_rebinding_refuses_destination_inode_substitution_without_touching_foreign_body() {
    let f = fixture();
    let p = pending(&f, "owned");
    let (_base, target) = move_to(&f);
    let path = target.join(&p.file);
    std::fs::rename(&path, target.join(&p.directory).join("copied-owned-body")).unwrap();
    std::fs::write(&path, b"foreign destination body").unwrap();
    let s = Storage::open(target.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign destination body");
    assert_eq!(state(&s, &p.id), "attention");
    assert!(f.target.root.join(&p.file).exists());
}
#[test]
fn restore_rebinding_second_move_translates_effective_ownership_without_adopting_unknown_files() {
    let f = fixture();
    let p = pending(&f, "unknown");
    let (_first, first) = move_to(&f);
    let source = Storage::open(first.clone()).unwrap();
    assert_eq!(state(&source, &p.id), "attention");
    let (_second, target) = move_storage(&source);
    let destination = Storage::open(target.clone()).unwrap();
    assert_eq!(count(&target), 1);
    assert_eq!(destination.previous_storage_history().unwrap().len(), 2);
    assert_eq!(state(&destination, &p.id), "attention");
    for root in [&f.target.root, &first, &target] {
        assert_eq!(
            std::fs::read(root.join(&p.directory).join("unknown")).unwrap(),
            b"unknown body"
        );
    }
    assert!(f.target.root.join(&p.file).exists());
}
fn retry(f: &Fixture, target: &Storage, p: &Pending) {
    let ids = ids(f, &p.batch);
    let sealed = sealed(f, &ids);
    let db = target.connection.lock().unwrap();
    let journal = Journal::begin(&db, &target.root, &sealed, &ids).unwrap();
    assert_eq!(journal.id, p.id);
    for relative in [&p.directory, &format!("{}/restored", p.directory)] {
        let path = target.root.join(relative);
        std::fs::create_dir(&path).unwrap();
        journal
            .record(&path, &std::fs::symlink_metadata(&path).unwrap())
            .unwrap();
    }
    let file = target.root.join(&p.file);
    std::fs::write(&file, b"new local retry body").unwrap();
    journal
        .record(&file, &std::fs::symlink_metadata(&file).unwrap())
        .unwrap();
    journal.recover().unwrap();
    assert!(!file.exists());
}
#[test]
fn restore_rebinding_reuses_same_batch_with_fresh_local_ownership_after_copied_cleanup() {
    let f = fixture();
    let p = pending(&f, "owned");
    let (_base, target) = move_to(&f);
    let s = Storage::open(target.clone()).unwrap();
    assert_eq!(state(&s, &p.id), "recovered");
    retry(&f, &s, &p);
    assert_eq!(state(&s, &p.id), "recovered");
    assert_eq!(
        std::fs::read(f.target.root.join(&p.file)).unwrap(),
        b"incomplete owned restore"
    );
    let scope: String = Connection::open(target.join(".mon-scratch.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT scope FROM relocation_restore_bindings WHERE operation_id=?1",
            params![p.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope, "local");
    drop(s);
    assert_eq!(state(&Storage::open(target).unwrap(), &p.id), "recovered");
}
#[test]
fn restore_rebinding_invalid_original_or_local_proof_is_preserved_without_cleanup() {
    for corrupt_source in [true, false] {
        let f = fixture();
        let p = pending(&f, "owned");
        if corrupt_source {
            let db = f.target.connection.lock().unwrap();
            let sql: String = db
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name='maintenance_path_fixed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute_batch("DROP TRIGGER maintenance_path_fixed")
                .unwrap();
            db.execute("UPDATE maintenance_paths SET identity_hash=?1 WHERE operation_id=?2 AND kind='file' AND identity_json IS NOT NULL",params!["0".repeat(64),p.id]).unwrap();
            db.execute_batch(&sql).unwrap();
        }
        let (_base, target) = move_to(&f);
        if corrupt_source {
            assert_eq!(count(&target), 0);
        } else {
            let db = Connection::open(target.join(".mon-scratch.sqlite3")).unwrap();
            let sql: String = db
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name='relocation_restore_binding_fixed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute_batch("DROP TRIGGER relocation_restore_binding_fixed")
                .unwrap();
            db.execute(
                "UPDATE relocation_restore_bindings SET proof_hash=?1",
                params!["0".repeat(64)],
            )
            .unwrap();
            db.execute_batch(&sql).unwrap();
        }
        let bytes = std::fs::read(target.join(&p.file)).unwrap();
        let s = Storage::open(target.clone()).unwrap();
        assert_eq!(state(&s, &p.id), "attention");
        assert_eq!(std::fs::read(target.join(&p.file)).unwrap(), bytes);
    }
}
#[test]
fn restore_rebinding_local_scope_crash_child() {
    let Ok(root) = std::env::var("MON_RESTORE_REBINDING_ROOT") else {
        return;
    };
    let id = std::env::var("MON_RESTORE_REBINDING_ID").unwrap();
    let marker = std::env::var("MON_RESTORE_REBINDING_MARKER").unwrap();
    let s = Storage::open(PathBuf::from(root)).unwrap();
    start_local_scope(&s.root, &s.connection.lock().unwrap(), &id).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .unwrap();
    file.write_all(b"ready").unwrap();
    file.sync_all().unwrap();
    loop {
        std::thread::park();
    }
}
#[test]
fn restore_rebinding_sigkill_after_local_scope_commit_preserves_source_and_allows_same_batch_retry()
{
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = fixture();
    let p = pending(&f, "owned");
    let (base, target) = move_to(&f);
    let s = Storage::open(target.clone()).unwrap();
    assert_eq!(state(&s, &p.id), "recovered");
    drop(s);
    let marker = base.path().join("ready");
    let mut child=Reap(Command::new(std::env::current_exe().unwrap()).args(["--exact","storage::backup::publication::journal::relocation::tests::restore_rebinding_local_scope_crash_child","--nocapture"]).env("MON_RESTORE_REBINDING_ROOT",&target).env("MON_RESTORE_REBINDING_ID",&p.id).env("MON_RESTORE_REBINDING_MARKER",&marker).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
    let start = Instant::now();
    while !marker.exists() {
        if let Some(code) = child.0.try_wait().unwrap() {
            panic!("child ended before marker: {code}");
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let s = Storage::open(target.clone()).unwrap();
    assert_eq!(state(&s, &p.id), "recovered");
    retry(&f, &s, &p);
    assert!(f.target.root.join(&p.file).exists());
}
