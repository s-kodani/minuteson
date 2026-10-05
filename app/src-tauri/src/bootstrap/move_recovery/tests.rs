use super::*;
use std::os::unix::fs::DirBuilderExt;

struct Fixture {
    _base: tempfile::TempDir,
    bootstrap: Bootstrap,
    storage: Storage,
    target: PathBuf,
    operation: String,
}
impl Fixture {
    fn new(copy: bool) -> Self {
        let base = tempfile::tempdir().unwrap();
        let parent = base.path().canonicalize().unwrap();
        let b = Bootstrap::open(&parent.join("config"), &parent.join("app-data")).unwrap();
        let storage = Storage::open(b.root().into()).unwrap();
        b.initialize(&storage).unwrap();
        storage.create_meeting("original").unwrap();
        let target = parent.join("target");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .unwrap();
        let operation = Uuid::new_v4().to_string();
        {
            let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
            b.begin_move(
                &operation,
                &Uuid::new_v4().to_string(),
                &target,
                snapshot.manifest_hash(),
                0,
            )
            .unwrap();
            if copy {
                let mut p = b.pending_placement(&operation, &snapshot).unwrap();
                // Cancel after the DB inode is durably claimed, before its bytes.
                assert!(p
                    .copy(&snapshot, &|| target.join("meetings.sqlite3").exists())
                    .is_err());
            }
        }
        Self {
            _base: base,
            bootstrap: b,
            storage,
            target,
            operation,
        }
    }
    fn review(&self) -> Review {
        self.bootstrap
            .prepare_pending_move(&self.storage, &|| false, &|_| {})
            .unwrap()
    }
}

#[test]
fn pending_review_resumes_claimed_db_without_replacing_inode_and_retires_old_writes() {
    let f = Fixture::new(true);
    let before = Identity::read(&f.target.join("meetings.sqlite3").metadata().unwrap()).unwrap();
    let review = f.review();
    assert!(
        review.preview().resume_allowed,
        "{:?}",
        review.preview().resume_reason
    );
    assert_eq!(review.preview().copied_paths, Some(0));
    let receipt = review
        .resume(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(!receipt.retained);
    assert!(receipt.restart_required);
    assert!(before.matches(&f.target.join("meetings.sqlite3").metadata().unwrap()));
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.target);
    assert!(f.storage.create_meeting("old root write").is_err());
}

#[test]
fn pending_review_missing_owner_can_resume_only_exact_empty_target() {
    let f = Fixture::new(false);
    let review = f.review();
    assert!(
        review.preview().resume_allowed,
        "{:?}",
        review.preview().resume_reason
    );
    review
        .resume(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.target);
}

#[test]
fn pending_review_source_edits_block_resume_but_keep_preserves_both_roots_and_replays() {
    let f = Fixture::new(true);
    let bytes = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    f.storage.create_meeting("newer edits").unwrap();
    let review = f.review();
    assert!(!review.preview().resume_allowed);
    assert!(review
        .resume(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false,
            &|_| {}
        )
        .is_err());
    let receipt = review
        .keep(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false,
        )
        .unwrap();
    assert!(receipt.retained && !receipt.restart_required);
    assert!(
        review
            .keep(
                &f.bootstrap,
                &f.storage,
                &review.preview().confirmation_id,
                &|| true
            )
            .unwrap()
            .retained
    );
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        bytes
    );
    assert_eq!(f.storage.list_meetings().unwrap().len(), 2);
    let view = f.bootstrap.view().unwrap();
    assert!(view.pending_operation_id.is_none());
    assert_eq!(view.kept_moves.len(), 1);
    assert!(!view.kept_moves[0].decision_pending);
    assert_eq!(view.meeting_root, f.storage.root());
    assert!(f
        .bootstrap
        .prepare_move(&f.storage, f._base.path(), &|| false, &|_| {})
        .is_ok());
}

#[test]
fn pending_review_unknown_objects_and_missing_target_are_retained_never_adopted() {
    for failure in ["unknown", "missing", "foreign-owner", "foreign-lock"] {
        let f = Fixture::new(false);
        match failure {
            "unknown" => std::fs::write(f.target.join("foreign"), b"do not delete").unwrap(),
            "missing" => std::fs::rename(&f.target, f.target.with_extension("retained")).unwrap(),
            "foreign-owner" => std::fs::write(
                f.bootstrap
                    .config
                    .join(format!(".relocation-{}.sqlite3", f.operation)),
                b"foreign owner",
            )
            .unwrap(),
            "foreign-lock" => std::fs::write(
                f.bootstrap
                    .config
                    .join(format!(".relocation-{}.lock", f.operation)),
                b"foreign lock",
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let review = f.review();
        assert!(!review.preview().resume_allowed, "{failure}");
        review
            .keep(
                &f.bootstrap,
                &f.storage,
                &review.preview().confirmation_id,
                &|| false,
            )
            .unwrap();
        assert!(f.bootstrap.view().unwrap().pending_operation_id.is_none());
        if failure == "unknown" {
            assert_eq!(
                std::fs::read(f.target.join("foreign")).unwrap(),
                b"do not delete"
            );
        }
        if failure == "missing" {
            assert!(f.target.with_extension("retained").is_dir());
            assert!(!f.target.exists());
        }
    }
}

#[test]
fn pending_keep_wrong_confirmation_cancel_and_changed_target_cannot_publish_decision() {
    let f = Fixture::new(true);
    let review = f.review();
    assert!(review
        .keep(
            &f.bootstrap,
            &f.storage,
            &Uuid::new_v4().to_string(),
            &|| false
        )
        .is_err());
    assert!(review
        .keep(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| true
        )
        .is_err());
    std::fs::write(f.target.join("new external file"), b"retain").unwrap();
    assert!(review
        .keep(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false
        )
        .is_err());
    assert!(f.bootstrap.read_kept(&f.operation).unwrap().is_none());
    assert!(f.bootstrap.view().unwrap().pending_operation_id.is_some());
}

#[test]
fn pending_saved_keep_requires_explicit_resolution_and_cannot_resume_or_overwrite() {
    let f = Fixture::new(true);
    let review = f.review();
    f.bootstrap
        .write_kept(&Decision {
            pending: review.pending.clone(),
            revision: review.revision,
        })
        .unwrap();
    assert!(f.bootstrap.view().unwrap().kept_moves[0].decision_pending);
    let fresh = f.review();
    assert!(!fresh.preview().resume_allowed);
    fresh
        .keep(
            &f.bootstrap,
            &f.storage,
            &fresh.preview().confirmation_id,
            &|| false,
        )
        .unwrap();
    assert!(!f.bootstrap.view().unwrap().kept_moves[0].decision_pending);
}

#[test]
fn pending_review_cold_restart_uses_durable_owner_and_same_original_intent() {
    let f = Fixture::new(true);
    let config = f.bootstrap.config.clone();
    let data = f.bootstrap.models_root.parent().unwrap().to_path_buf();
    let op = f.operation.clone();
    let target = f.target.clone();
    drop(f.storage);
    drop(f.bootstrap);
    let b = Bootstrap::open(&config, &data).unwrap();
    let s = Storage::open(b.root().into()).unwrap();
    b.initialize(&s).unwrap();
    let review = b.prepare_pending_move(&s, &|| false, &|_| {}).unwrap();
    assert_eq!(review.preview().operation_id, op);
    assert!(
        review.preview().resume_allowed,
        "{:?}",
        review.preview().resume_reason
    );
    review
        .resume(
            &b,
            &s,
            &review.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(b.view().unwrap().meeting_root, target);
}

#[test]
fn pending_review_full_copy_installed_history_is_rechecked_and_resumable() {
    let f = Fixture::new(true);
    {
        let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
        let mut p = f
            .bootstrap
            .pending_placement(&f.operation, &snapshot)
            .unwrap();
        p.copy(&snapshot, &|| false).unwrap();
        let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
        p.install_history(&handoff, &|| false).unwrap();
    }
    let review = f.review();
    assert!(
        review.preview().resume_allowed,
        "{:?}",
        review.preview().resume_reason
    );
    assert_eq!(
        review.preview().copied_paths,
        review.preview().expected_paths
    );
    review
        .resume(
            &f.bootstrap,
            &f.storage,
            &review.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.target);
}

#[test]
fn pending_review_changed_copied_bytes_and_control_file_are_never_overwritten() {
    for failure in ["payload", "history"] {
        let f = Fixture::new(true);
        {
            let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
            let mut p = f
                .bootstrap
                .pending_placement(&f.operation, &snapshot)
                .unwrap();
            p.copy(&snapshot, &|| false).unwrap();
            let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
            p.install_history(&handoff, &|| false).unwrap();
        }
        let altered = f.target.join(if failure == "payload" {
            "meetings.sqlite3"
        } else {
            ".mon-handoff.sqlite3"
        });
        std::fs::write(&altered, b"external modification").unwrap();
        let review = f.review();
        assert!(!review.preview().resume_allowed, "{failure}");
        review
            .keep(
                &f.bootstrap,
                &f.storage,
                &review.preview().confirmation_id,
                &|| false,
            )
            .unwrap();
        assert_eq!(std::fs::read(altered).unwrap(), b"external modification");
        assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    }
}

#[test]
#[ignore = "subprocess fixture; invoked by the pending keep SIGKILL test"]
fn pending_keep_checkpoint_fixture() {
    let parent = PathBuf::from(std::env::var("MON_MOVE_KEEP_BASE").unwrap());
    let b = Bootstrap::open(&parent.join("config"), &parent.join("app-data")).unwrap();
    let s = Storage::open(b.root().into()).unwrap();
    b.initialize(&s).unwrap();
    let review = b.prepare_pending_move(&s, &|| false, &|_| {}).unwrap();
    review
        .keep(&b, &s, &review.preview().confirmation_id, &|| false)
        .unwrap();
    panic!("keep checkpoint not reached");
}

#[test]
fn pending_keep_sigkill_after_durable_decision_retains_files_and_needs_explicit_completion() {
    let f = Fixture::new(true);
    let parent = f._base.path().canonicalize().unwrap();
    let ready = parent.join("keep-ready");
    let target = f.target.clone();
    let operation = f.operation.clone();
    let copied_before = std::fs::read(target.join("meetings.sqlite3")).unwrap();
    drop(f.storage);
    drop(f.bootstrap);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bootstrap::move_recovery::tests::pending_keep_checkpoint_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("MON_MOVE_KEEP_BASE", &parent)
        .env("MON_MOVE_KEEP_CHECKPOINT", "saved")
        .env("MON_MOVE_KEEP_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !ready.exists() {
        if child.try_wait().unwrap().is_some()
            || started.elapsed() > std::time::Duration::from_secs(20)
        {
            let _ = child.kill();
            let _ = child.wait();
            panic!("keep checkpoint was not reached");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let b = Bootstrap::open(&parent.join("config"), &parent.join("app-data")).unwrap();
    let s = Storage::open(b.root().into()).unwrap();
    b.initialize(&s).unwrap();
    let view = b.view().unwrap();
    assert_eq!(
        view.pending_operation_id.as_deref(),
        Some(operation.as_str())
    );
    assert!(view.kept_moves[0].decision_pending);
    assert_eq!(view.meeting_root, s.root());
    let review = b.prepare_pending_move(&s, &|| false, &|_| {}).unwrap();
    assert!(!review.preview().resume_allowed);
    review
        .keep(&b, &s, &review.preview().confirmation_id, &|| false)
        .unwrap();
    assert!(b.view().unwrap().pending_operation_id.is_none());
    assert_eq!(
        std::fs::read(target.join("meetings.sqlite3")).unwrap(),
        copied_before
    );
    assert!(s.create_meeting("old root remains writable").is_ok());
}
