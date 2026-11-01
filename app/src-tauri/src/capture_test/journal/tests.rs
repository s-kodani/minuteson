use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn request() -> Request {
    Request {
        run_id: uuid::Uuid::new_v4().to_string(),
        microphone_uid: "USB input/名前 with spaces".into(),
        capture_scope: crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: None,
        },
        expected_import: super::super::ImportExpectation {
            token: None,
            revision: 0,
            busy: false,
        },
    }
}
fn remove_empty(area: &Area, quiet: &Quiet) {
    std::fs::remove_dir(area.path()).unwrap();
    area.finish(quiet).unwrap();
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Pause an actual owned child after fork but before exec. Only descriptor
// syscalls run there; no allocation, Rust locks, GUI or capture initialization.
struct PendingExec {
    ready: std::os::unix::net::UnixStream,
    release: Option<std::os::unix::net::UnixStream>,
    task: Option<std::thread::JoinHandle<std::io::Result<std::process::ExitStatus>>>,
}
impl PendingExec {
    fn new(retained: File) -> Self {
        use std::os::unix::net::UnixStream;
        let (notify, ready) = UnixStream::pair().unwrap();
        let (release, gate) = UnixStream::pair().unwrap();
        ready
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let task = std::thread::spawn(move || {
            let notify_fd = notify.as_raw_fd();
            let gate_fd = gate.as_raw_fd();
            let mut command = Command::new("/usr/bin/true");
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            unsafe {
                command.pre_exec(move || {
                    let flags = libc::fcntl(retained.as_raw_fd(), libc::F_GETFD);
                    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
                        return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
                    }
                    let ready = 1_u8;
                    if libc::write(notify_fd, (&ready as *const u8).cast(), 1) != 1 {
                        return Err(std::io::Error::from_raw_os_error(libc::EPIPE));
                    }
                    let mut poll = libc::pollfd {
                        fd: gate_fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    if libc::poll(&mut poll, 1, 8000) != 1 || poll.revents & libc::POLLIN == 0 {
                        return Err(std::io::Error::from_raw_os_error(libc::ETIMEDOUT));
                    }
                    let mut byte = 0_u8;
                    if libc::read(gate_fd, (&mut byte as *mut u8).cast(), 1) != 1 || byte != 7 {
                        return Err(std::io::Error::from_raw_os_error(libc::ECANCELED));
                    }
                    Ok(())
                });
            }
            let result = command.spawn().and_then(|mut child| child.wait());
            drop(command);
            drop(notify);
            drop(gate);
            result
        });
        Self {
            ready,
            release: Some(release),
            task: Some(task),
        }
    }
    fn wait_ready(&mut self) {
        let mut byte = [0];
        self.ready.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [1]);
    }
    fn finish(&mut self) {
        self.release.take().unwrap().write_all(&[7]).unwrap();
        assert!(self.task.take().unwrap().join().unwrap().unwrap().success());
    }
}
impl Drop for PendingExec {
    fn drop(&mut self) {
        if let Some(mut release) = self.release.take() {
            let _ = release.write_all(&[7]);
        }
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[test]
fn quiet_drop_preexec_copy_ends_collection_ownership_but_not_capture_ownership() {
    // Other parallel tests deliberately pause unrelated pre-exec children.
    // They legitimately retain every current capture-writer description too.
    // Create these leases only after exec in an isolated, single-test process
    // so this check controls every fork copy instead of relaxing collection.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "capture_test::journal::tests::quiet_drop_preexec_child",
            "--ignored",
            "--test-threads=1",
        ])
        .env("MON_QUIET_PREEXEC_FIXTURE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated pre-exec fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "owned pre-exec lease fixture; invoked by the ordinary test without capture"]
fn quiet_drop_preexec_child() {
    assert_eq!(
        std::env::var("MON_QUIET_PREEXEC_FIXTURE").as_deref(),
        Ok("1")
    );
    for collection in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let path = area.path();
        let record = registry.pending().unwrap().pop().unwrap();
        let quiet = if collection {
            Some(area.quiet().unwrap())
        } else {
            None
        };
        let retained = if let Some(quiet) = &quiet {
            quiet.lease.try_clone().unwrap()
        } else {
            area.lease
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .try_clone()
                .unwrap()
        };
        let mut child = PendingExec::new(retained);
        child.wait_ready();
        area.release_parent_lease();
        if collection {
            // Exact failed boundary: payload removed, allocated row durable,
            // Quiet closes while an unrelated pre-exec child retains a copy.
            std::fs::remove_dir(&path).unwrap();
            drop(quiet);
            registry.collect(record.clone()).unwrap();
            assert!(registry.pending().unwrap().is_empty());
        } else {
            assert!(area.quiet().is_err());
            assert!(registry.collect(record.clone()).is_err());
            assert!(path.exists());
            assert_eq!(registry.pending().unwrap().len(), 1);
        }
        child.finish();
        if !collection {
            registry.collect(record).unwrap();
            assert!(registry.pending().unwrap().is_empty());
        }
        assert!(!path.exists());
    }
}
#[test]
fn durable_test_ownership_is_single_allocated_private_and_never_replays_removed_ids() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    assert!(Registry::open(fixture.path()).is_err());
    let request = request();
    let area = registry.allocate(request.clone()).unwrap();
    assert!(registry.allocate(super::tests::request()).is_err());
    assert!(registry.allocate(request.clone()).is_err());
    assert_eq!(
        registry.pending().unwrap()[0].request.run_id,
        request.run_id
    );
    assert_eq!(
        std::fs::metadata(area.path()).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(registry.root.join(lease_name(&request.run_id)))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    let quiet = area.quiet().unwrap();
    assert!(area.finish(&quiet).is_err());
    remove_empty(&area, &quiet);
    assert!(registry.pending().unwrap().is_empty());
    assert!(registry.allocate(request).is_err());
    assert!(registry.allocate(super::tests::request()).is_ok());
}
#[test]
fn registry_drop_releases_only_the_non_capture_lock_after_closing_sqlite() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    // Models the same open description briefly inherited by another thread's
    // fork before exec. It must not outlive the global registry owner.
    let inherited = registry._lock.try_clone().unwrap();
    drop(registry);
    let reopened = Registry::open(fixture.path());
    assert!(
        reopened.is_ok(),
        "a retained non-capture duplicate held the registry lock: {:?}",
        reopened.err()
    );
    assert!(regular(&inherited, 0).is_ok());
}
#[test]
fn quiet_drop_releases_collection_lock_without_adopting_or_removing_audio() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let area = registry.allocate(request()).unwrap();
    let path = area.path().join("microphone.caf");
    std::fs::write(&path, b"synthetic owned audio").unwrap();
    let quiet = area.quiet().unwrap();
    assert!(area.quiet().is_err(), "live Quiet must remain exclusive");
    // dup and a concurrent fork retain the same open description. Quiet is
    // never delegated to a writer; its lock must end with the actual owner.
    let inherited = quiet.lease.try_clone().unwrap();
    drop(quiet);
    let next = area.quiet().unwrap();
    assert!(regular(&inherited, 0).is_ok());
    assert!(area.quiet().is_err(), "the new Quiet still owns its lock");
    drop(inherited);
    assert!(
        area.quiet().is_err(),
        "closing an old copy cannot unlock the new Quiet"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"synthetic owned audio");
    assert_eq!(registry.pending().unwrap().len(), 1);
    drop(next);
    drop(area.quiet().unwrap());
}
#[test]
fn quiet_drop_never_unlocks_a_retained_capture_writer_description() {
    const CHILD: &str = "MON_QUIET_RETAINED_WRITER_FIXTURE";
    const TEST: &str = "capture_test::journal::tests::quiet_drop_never_unlocks_a_retained_capture_writer_description";
    if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
        // Unrelated pre-exec children in the parallel suite can legitimately
        // retain this writer's description. Create the lease only after exec
        // in a single-test child, where this test controls every writer copy.
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "isolated retained-writer fixture failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("1 passed; 0 failed") && stdout.contains(TEST),
            "the isolated retained-writer test did not run: {stdout}"
        );
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let area = registry.allocate(request()).unwrap();
    let path = area.path();
    let writer = area
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .try_clone()
        .unwrap();
    area.release_parent_lease();
    assert!(area.quiet().is_err());
    assert!(registry
        .collect(registry.pending().unwrap().pop().unwrap())
        .is_err());
    assert!(path.exists());
    drop(writer);
    let quiet = area.quiet().unwrap();
    let retained_quiet = quiet.lease.try_clone().unwrap();
    drop(quiet);
    // Collection requires a new exclusive proof, not a PID or old Quiet.
    registry
        .collect(registry.pending().unwrap().pop().unwrap())
        .unwrap();
    assert!(!path.exists());
    assert!(registry.pending().unwrap().is_empty());
    assert!(regular(&retained_quiet, 0).is_ok());
}
#[test]
fn restored_record_keeps_exact_inputs_and_cannot_restart_capture() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let original = registry.allocate(request()).unwrap();
    let record = registry.pending().unwrap().pop().unwrap();
    let path = original.path();
    drop(original);
    drop(registry);
    assert!(
        path.exists(),
        "drop must not collect a potentially live helper"
    );
    let registry = Registry::open(fixture.path()).unwrap();
    let area = registry.restore(record.clone()).unwrap();
    assert_eq!(area.path(), path);
    let mut command = Command::new("/bin/true");
    assert!(area.inherit(&mut command).is_err());
    let mut changed = record;
    changed.request.microphone_uid = "another microphone".into();
    assert!(registry.restore(changed).is_err());
    remove_empty(&area, &area.quiet().unwrap());
}
#[test]
fn completion_requires_the_same_live_quiet_guard_and_an_absent_payload() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = Registry::open(first.path())
        .unwrap()
        .allocate(request())
        .unwrap();
    let b = Registry::open(second.path())
        .unwrap()
        .allocate(request())
        .unwrap();
    let qa = a.quiet().unwrap();
    assert!(
        a.quiet().is_err(),
        "separate descriptions must not share a held exclusive lease"
    );
    let qb = b.quiet().unwrap();
    std::fs::remove_dir(a.path()).unwrap();
    assert!(a.finish(&qb).is_err());
    a.finish(&qa).unwrap();
    remove_empty(&b, &qb);
}
#[test]
fn payload_replacement_and_symlinks_are_preserved_not_adopted() {
    for linked in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let original = area.path().with_extension("original");
        std::fs::rename(area.path(), &original).unwrap();
        let foreign = tempfile::tempdir().unwrap();
        if linked {
            symlink(foreign.path(), area.path()).unwrap();
        } else {
            std::fs::create_dir(area.path()).unwrap();
            std::fs::set_permissions(area.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(area.path().join("keep"), b"unowned").unwrap();
        }
        assert!(area.quiet().is_err());
        assert!(original.exists());
        assert!(foreign.path().exists());
        assert!(registry.pending().unwrap().len() == 1);
    }
}
#[test]
fn lease_replacement_hardlink_permissions_and_contents_never_authorize_collection() {
    for alteration in ["replace", "link", "hardlink", "permissions", "contents"] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let lease = registry.root.join(lease_name(&area.record.request.run_id));
        let foreign = tempfile::NamedTempFile::new().unwrap();
        match alteration {
            "replace" | "link" => {
                std::fs::rename(&lease, lease.with_extension("original")).unwrap();
                if alteration == "link" {
                    symlink(foreign.path(), &lease).unwrap();
                } else {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&lease)
                        .unwrap();
                }
            }
            "hardlink" => std::fs::hard_link(&lease, lease.with_extension("duplicate")).unwrap(),
            "permissions" => {
                std::fs::set_permissions(&lease, std::fs::Permissions::from_mode(0o644)).unwrap()
            }
            "contents" => std::fs::write(&lease, b"changed").unwrap(),
            _ => unreachable!(),
        }
        assert!(area.quiet().is_err(), "{alteration}");
        assert!(area.path().exists());
        assert!(foreign.path().exists());
    }
}
#[test]
fn journal_root_and_database_replacement_are_not_repaired_or_reclaimed() {
    for node in ["root", "database", "lock"] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let original = match node {
            "root" => registry.root.clone(),
            "database" => registry.root.join("ownership.sqlite3"),
            "lock" => registry.root.join("owner.lock"),
            _ => unreachable!(),
        };
        let moved = original.with_extension("original");
        std::fs::rename(&original, &moved).unwrap();
        if node == "root" {
            std::fs::create_dir(&original).unwrap();
            std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&original)
                .unwrap();
        }
        assert!(registry.pending().is_err());
        assert!(area.quiet().is_err());
        assert!(moved.exists());
        assert!(original.exists());
    }
}
#[test]
fn journal_rejects_schema_corruption_and_oversized_or_changed_record_hashes() {
    for alteration in ["schema", "oversized", "digest", "identity"] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let db = Connection::open(registry.root.join("ownership.sqlite3")).unwrap();
        if alteration == "schema" {
            db.execute_batch("CREATE TABLE unknown(value TEXT)")
                .unwrap();
        } else {
            db.execute_batch("DROP TRIGGER runs_fixed").unwrap();
            let (proof, digest) = match alteration {
                "oversized" => {
                    let p = "x".repeat(RECORD_LIMIT + 1);
                    (p.clone(), hash(&p))
                }
                "digest" => {
                    let (p, _) = encode(&area.record).unwrap();
                    (p, "wrong".into())
                }
                "identity" => {
                    let mut changed = area.record.clone();
                    changed.directory.inode += 1;
                    encode(&changed).unwrap()
                }
                _ => unreachable!(),
            };
            db.execute("UPDATE runs SET proof=?1,hash=?2", params![proof, digest])
                .unwrap();
            // Restore the exact schema to isolate record integrity validation.
            let trigger = registry
                .expected_schema
                .iter()
                .find(|(_, name, _)| name == "runs_fixed")
                .unwrap()
                .2
                .clone();
            db.execute_batch(&trigger).unwrap();
        }
        if alteration == "identity" {
            let restored = registry
                .restore(registry.pending().unwrap().pop().unwrap())
                .unwrap();
            assert!(restored.quiet().is_err());
        } else {
            assert!(registry.pending().is_err());
        }
        assert!(area.quiet().is_err());
        assert!(area.path().exists());
    }
}
#[test]
fn journal_never_scans_or_adopts_unregistered_empty_nodes() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let id = request().run_id;
    let existing = registry.root.join(payload_name(&id));
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("microphone.caf"), b"keep").unwrap();
    let mut input = request();
    input.run_id = id;
    assert!(registry.allocate(input).is_err());
    assert!(registry.pending().unwrap().is_empty());
    assert_eq!(
        std::fs::read(existing.join("microphone.caf")).unwrap(),
        b"keep"
    );
    assert!(registry.allocate(request()).is_ok());
}
#[test]
fn ownership_rows_and_completed_history_are_immutable() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let area = registry.allocate(request()).unwrap();
    registry
        .with_db(|db| {
            for sql in [
                "DELETE FROM metadata",
                "UPDATE metadata SET hash='changed'",
                "DELETE FROM runs",
                "UPDATE runs SET proof='changed'",
            ] {
                assert!(db.execute(sql, []).is_err());
            }
            Ok(())
        })
        .unwrap();
    remove_empty(&area, &area.quiet().unwrap());
    registry
        .with_db(|db| {
            assert!(db.execute("UPDATE runs SET state='allocated'", []).is_err());
            Ok(())
        })
        .unwrap();
}
#[test]
fn restarting_never_mutates_a_replaced_database_or_identical_new_certificate() {
    for node in ["database", "certificate"] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let path = registry.root.join(if node == "database" {
            "ownership.sqlite3"
        } else {
            "root-proof.json"
        });
        let contents = std::fs::read(&path).unwrap();
        std::fs::rename(&path, path.with_extension("retained")).unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        if node == "database" {
            file.write_all(b"unowned database; do not initialize")
                .unwrap();
        } else {
            file.write_all(&contents).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        assert!(area.quiet().is_err());
        drop(area);
        drop(registry);
        assert!(Registry::open(fixture.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
#[test]
fn restarting_preserves_untrusted_sqlite_sidecars_without_following_links() {
    for name in [
        "ownership.sqlite3-journal",
        "ownership.sqlite3-wal",
        "ownership.sqlite3-shm",
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let root = registry.root.clone();
        drop(registry);
        let external = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(external.path(), b"keep external bytes").unwrap();
        symlink(external.path(), root.join(name)).unwrap();
        assert!(Registry::open(fixture.path()).is_err());
        assert_eq!(
            std::fs::read(external.path()).unwrap(),
            b"keep external bytes"
        );
        assert!(std::fs::symlink_metadata(root.join(name))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
#[test]
fn invalid_request_never_creates_a_lease_or_payload() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let before = std::fs::read_dir(&registry.root).unwrap().count();
    for change in ["id", "microphone", "revision", "import"] {
        let mut input = request();
        match change {
            "id" => input.run_id = "../../unowned".into(),
            "microphone" => input.microphone_uid = "".into(),
            "revision" => input.expected_import.revision = 9_007_199_254_740_992,
            "import" => input.expected_import.token = Some("another process".into()),
            _ => unreachable!(),
        }
        assert!(registry.allocate(input).is_err());
        assert_eq!(std::fs::read_dir(&registry.root).unwrap().count(), before);
    }
}
#[test]
fn owned_audio_cleanup_collects_only_fixed_files_then_commits_its_journal() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let directory = super::super::files::TestDirectory::new_owned(&registry, request()).unwrap();
    for name in [
        "manifest.json",
        "microphone.caf",
        "system.caf",
        "input-levels.json",
        "stop.request",
    ] {
        std::fs::write(directory.path.join(name), b"").unwrap();
    }
    let path = directory.path.clone();
    directory.cleanup().unwrap();
    assert!(!path.exists());
    assert!(registry.pending().unwrap().is_empty());
    assert!(
        std::fs::read_dir(&registry.root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("lease-")),
        "completed ownership evidence must remain"
    );
}
#[test]
fn owned_audio_unknown_or_linked_payload_keeps_the_record_for_safe_retry() {
    for linked in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let directory =
            super::super::files::TestDirectory::new_owned(&registry, request()).unwrap();
        let foreign = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(foreign.path(), b"keep").unwrap();
        let node = directory
            .path
            .join(if linked { "microphone.caf" } else { "unknown" });
        if linked {
            symlink(foreign.path(), &node).unwrap();
        } else {
            std::fs::write(&node, b"keep").unwrap();
        }
        assert!(directory.cleanup().is_err());
        assert_eq!(registry.pending().unwrap().len(), 1);
        assert_eq!(std::fs::read(foreign.path()).unwrap(), b"keep");
        std::fs::remove_file(&node).unwrap();
        directory.cleanup().unwrap();
        assert!(registry.pending().unwrap().is_empty());
    }
}
#[test]
fn restarted_collection_uses_only_durable_records_and_retains_completed_start_ids() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let request = request();
    let area = registry.allocate(request.clone()).unwrap();
    let path = area.path();
    std::fs::write(path.join("system.caf"), b"owned synthetic PCM").unwrap();
    let unrelated = registry.root.join("mon-owned-capture-test-unregistered");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(unrelated.join("system.caf"), b"unregistered PCM").unwrap();
    drop(area);
    drop(registry);
    let registry = Registry::open(fixture.path()).unwrap();
    let record = registry.pending().unwrap().pop().unwrap();
    registry.collect(record).unwrap();
    assert!(!path.exists());
    assert!(registry.pending().unwrap().is_empty());
    assert!(registry.allocate(request).is_err());
    assert_eq!(
        std::fs::read(unrelated.join("system.caf")).unwrap(),
        b"unregistered PCM"
    );
}
#[test]
fn collection_restarts_after_audio_removal_before_the_completion_commit() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = Registry::open(fixture.path()).unwrap();
    let area = registry.allocate(request()).unwrap();
    let quiet = area.quiet().unwrap();
    // Model exactly the durable crash boundary: directory removed, immutable
    // allocated row remains. Restart must not need an existing directory pin.
    std::fs::remove_dir(area.path()).unwrap();
    assert_eq!(registry.pending().unwrap().len(), 1);
    drop(quiet);
    drop(area);
    drop(registry);
    let registry = Registry::open(fixture.path()).unwrap();
    registry
        .collect(registry.pending().unwrap().pop().unwrap())
        .unwrap();
    assert!(registry.pending().unwrap().is_empty());
}
#[test]
fn restarted_collection_preserves_unknown_audio_until_explicit_fixture_repair() {
    for node in ["unknown", "microphone.caf"] {
        let fixture = tempfile::tempdir().unwrap();
        let registry = Registry::open(fixture.path()).unwrap();
        let area = registry.allocate(request()).unwrap();
        let path = area.path();
        let external = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(external.path(), b"keep external PCM").unwrap();
        let untrusted = path.join(node);
        if node == "unknown" {
            std::fs::write(&untrusted, b"unknown bytes").unwrap();
        } else {
            symlink(external.path(), &untrusted).unwrap();
        }
        std::fs::write(
            path.join("system.caf"),
            b"keep owned PCM until the set is validated",
        )
        .unwrap();
        drop(area);
        drop(registry);
        let registry = Registry::open(fixture.path()).unwrap();
        let record = registry.pending().unwrap().pop().unwrap();
        assert!(registry.collect(record.clone()).is_err());
        assert_eq!(
            std::fs::read(external.path()).unwrap(),
            b"keep external PCM"
        );
        assert_eq!(
            std::fs::read(path.join("system.caf")).unwrap(),
            b"keep owned PCM until the set is validated"
        );
        assert_eq!(registry.pending().unwrap().len(), 1);
        // Explicit fixture repair, not a production recovery operation.
        std::fs::remove_file(untrusted).unwrap();
        registry.collect(record).unwrap();
        assert!(registry.pending().unwrap().is_empty());
    }
}
#[test]
fn inherited_lease_survives_real_parent_sigkill_until_the_helper_exits() {
    inherited_lease_crash_check(None);
}
#[test]
#[ignore = "explicit Rust-to-Swift lease fixture; no microphone or native app"]
fn swift_inherited_lease_survives_parent_sigkill_and_allows_only_quiet_collection() {
    let bundle = PathBuf::from(std::env::var_os("MON_CAPTURE_TEST_SWIFT_LEASE_BUNDLE").unwrap());
    assert!(bundle.is_dir());
    inherited_lease_crash_check(Some(&bundle));
}
fn inherited_lease_crash_check(swift_bundle: Option<&Path>) {
    let fixture = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "capture_test::journal::tests::journal_crash_child",
            "--ignored",
        ])
        .env("MON_CAPTURE_TEST_JOURNAL_FIXTURE", fixture.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(bundle) = swift_bundle {
        command.env("MON_CAPTURE_TEST_JOURNAL_SWIFT_BUNDLE", bundle);
    } else {
        command.env_remove("MON_CAPTURE_TEST_JOURNAL_SWIFT_BUNDLE");
    }
    let status = command.status().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(fixture.path().join("helper-ready").exists());
    let registry = Registry::open(fixture.path()).unwrap();
    let record = registry.pending().unwrap().pop().unwrap();
    let area = registry.restore(record.clone()).unwrap();
    assert!(
        area.quiet().is_err(),
        "dead parent must not mean the helper stopped writing"
    );
    assert!(area.path().exists());
    assert!(registry.collect(record.clone()).is_err());
    assert_eq!(
        std::fs::read(area.path().join("microphone.caf")).unwrap(),
        b"owned synthetic PCM"
    );
    std::fs::write(fixture.path().join("helper-stop"), b"").unwrap();
    wait_until(|| registry.collect(record.clone()).is_ok());
    assert!(!area.path().exists());
    assert!(registry.pending().unwrap().is_empty());
}
#[test]
#[ignore = "synthetic parent crash fixture only; no microphone or native app"]
fn journal_crash_child() {
    let fixture = PathBuf::from(std::env::var_os("MON_CAPTURE_TEST_JOURNAL_FIXTURE").unwrap());
    let registry = Registry::open(&fixture).unwrap();
    let area = registry.allocate(request()).unwrap();
    std::fs::write(area.path().join("microphone.caf"), b"owned synthetic PCM").unwrap();
    let mut command =
        if let Some(bundle) = std::env::var_os("MON_CAPTURE_TEST_JOURNAL_SWIFT_BUNDLE") {
            let result = Command::new("/usr/bin/xcrun")
                .args(["--find", "xctest"])
                .output()
                .unwrap();
            assert!(result.status.success());
            let runner = String::from_utf8(result.stdout).unwrap();
            let mut command = Command::new(runner.trim());
            command.args([
            "-XCTest",
            "MONMacOSCaptureTests.CaptureTestLeaseTests/testInheritedLeaseFixtureWithoutCapture",
        ]).arg(bundle);
            command
        } else {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.args([
                "--exact",
                "capture_test::journal::tests::journal_lease_helper",
                "--ignored",
            ]);
            command
        };
    command
        .env("MON_CAPTURE_TEST_JOURNAL_FIXTURE", &fixture)
        .env(
            "MON_CAPTURE_TEST_JOURNAL_OWNER",
            &area.record.request.run_id,
        )
        .env("MON_CAPTURE_TEST_JOURNAL_OUTPUT", area.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let descriptor = area.inherit(&mut command).unwrap();
    command.env("MON_CAPTURE_TEST_JOURNAL_LEASE_FD", descriptor.to_string());
    let _helper = command.spawn().unwrap();
    drop(command);
    wait_until(|| fixture.join("helper-ready").exists());
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
    panic!("SIGKILL unexpectedly returned");
}
#[test]
#[ignore = "synthetic inherited lease helper only; no capture or audio"]
fn journal_lease_helper() {
    let fixture = PathBuf::from(std::env::var_os("MON_CAPTURE_TEST_JOURNAL_FIXTURE").unwrap());
    let descriptor = std::env::var("MON_CAPTURE_TEST_JOURNAL_LEASE_FD")
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let lease = unsafe { File::from_raw_fd(descriptor) };
    assert!(regular(&lease, 0).is_ok());
    std::fs::write(fixture.join("helper-ready"), b"").unwrap();
    wait_until(|| fixture.join("helper-stop").exists());
    drop(lease);
}
