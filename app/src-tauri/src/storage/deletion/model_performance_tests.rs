//! Private synthetic packages; no downloaded engine is executed or removed.
use super::*;
use crate::analysis_worker::AnalysisController;

#[test]
fn model_delete_and_resume_keep_notification_owners_active() {
    let f = Fixture::new();
    let c = AnalysisController::default();
    let history = c.begin_notice_storage().unwrap();
    let owner = c.begin_model_deletion(false, false, &|| false).unwrap();
    let mut p =
        operations::Prepared::fresh(&f.storage, &f.root, &f.package, &f.review, &owner, &|| {
            false
        })
        .unwrap();
    let preview = p.preview();
    let stop = Cell::new(false);
    assert!(p
        .execute(
            &f.storage,
            &owner,
            &preview.confirmation_id,
            false,
            &|| stop.get(),
            &|phase| if phase == "model_delete_scope_fixed" {
                stop.set(true);
            },
        )
        .is_err());
    history.validate_active().unwrap();
    assert!(f.path("cli").exists());
    drop(owner);
    let owner = c
        .begin_model_deletion_maintenance(&f.storage, &preview.operation_id, false, false, &|| {
            false
        })
        .unwrap();
    let next_history = c.begin_notice_storage().unwrap();
    let mut p =
        operations::Prepared::pending(&f.storage, &f.root, &preview.operation_id, &owner, &|| {
            false
        })
        .unwrap();
    let receipt = p
        .execute(
            &f.storage,
            &owner,
            &p.preview().confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.body_deleted && receipt.history_retained);
    assert!(!f.path("cli").exists());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    history.validate_active().unwrap();
    next_history.validate_active().unwrap();
    drop(owner);
    assert!(c.begin_model_deletion(false, false, &|| false).is_ok());
}

#[test]
fn global_maintenance_cannot_authorize_model_body_removal() {
    let f = Fixture::new();
    let c = AnalysisController::default();
    let global = c.begin_data_maintenance(false, false, &|| false).unwrap();
    assert!(operations::Prepared::fresh(
        &f.storage,
        &f.root,
        &f.package,
        &f.review,
        &global,
        &|| false,
    )
    .is_err());
    assert!(f.path("cli").exists());
    assert_eq!(f.operation_count(), 0);
}

#[test]
fn model_review_hashes_outside_database_lock_and_retains_exact_consent() {
    let f = Fixture::new();
    let owner = AnalysisController::default()
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let _measure = crate::operation_observation::Scope::enter("test", |_| {});
    let observed = Cell::new(false);
    let mut prepared =
        operations::Prepared::fresh(&f.storage, &f.root, &f.package, &f.review, &owner, &|| {
            if crate::operation_observation::measurements().1 > 0 {
                if let Ok(db) = f.storage.connection.try_lock() {
                    assert_eq!(
                        db.query_row("SELECT count(*) FROM meetings", [], |r| r.get::<_, i64>(0))
                            .unwrap(),
                        1
                    );
                    observed.set(true);
                }
            }
            false
        })
        .unwrap();
    assert!(observed.get(), "file hashing monopolized the database");
    let preview = prepared.preview();
    std::fs::write(f.path("weights/model.bin"), b"changed after preview").unwrap();
    assert!(prepared
        .execute(
            &f.storage,
            &owner,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {}
        )
        .is_err());
    assert!(f.path("weights/model.bin").is_file());
    assert!(f.storage.list_meetings().is_ok());
}

#[test]
fn model_removal_releases_database_and_write_bypass_at_every_boundary() {
    for inject_unknown in [false, true] {
        let f = Fixture::new();
        let _cache = crate::storage::deletion::verification::Scope::enter();
        let owner = AnalysisController::default()
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        let p = operations::Prepared::fresh(
            &f.storage,
            &f.root,
            &f.package,
            &f.review,
            &owner,
            &|| false,
        )
        .unwrap()
        .preview();
        intent::prepare(
            &f.storage,
            &f.root,
            &p.operation_id,
            &f.package,
            &f.review,
            &f.preview().unwrap().census_hash,
            false,
            &owner,
            &|| false,
        )
        .unwrap();
        intent::removal::start(&f.storage, &p.operation_id, &owner, &|| false).unwrap();
        let boundaries = Cell::new(0usize);
        let result = intent::removal::run_observed(
            &f.storage,
            &p.operation_id,
            &owner,
            &|| false,
            &|event| {
                let db = f
                    .storage
                    .connection
                    .try_lock()
                    .expect("I/O retained the shared DB lock");
                let ordinary_write = db.execute("UPDATE meetings SET title=title", []);
                if event != "completed" {
                    assert!(ordinary_write.is_err(), "write bypass survived at {event}");
                }
                drop(db);
                boundaries.set(boundaries.get() + 1);
                if inject_unknown && event == "beforeUnlink" {
                    std::fs::write(f.path("unknown"), b"retain unknown").unwrap();
                }
            },
        );
        assert!(boundaries.get() > 0);
        if inject_unknown {
            assert!(result.is_err());
            assert_eq!(std::fs::read(f.path("unknown")).unwrap(), b"retain unknown");
            assert!(f.storage.data_write_blocked());
        } else {
            assert!(result.unwrap().completed());
            assert!(!f.path("").exists());
        }
        assert_eq!(f.storage.list_meetings().unwrap()[0].title, "KEEP");
    }
}

#[cfg(windows)]
#[test]
fn windows_hash_pin_denies_writes_allows_journaled_rename_and_unlink_and_expires() {
    let f = Fixture::new();
    let directory = crate::storage::deletion::fs::RootAnchor::directory(&f.root)
        .unwrap()
        .child(&f.package)
        .unwrap()
        .unwrap();
    {
        let _scope = crate::storage::deletion::verification::Scope::enter();
        let (_, sha) =
            crate::storage::deletion::verification::hash(&directory, "cli", &|| false).unwrap();
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(f.path("cli"))
            .is_err());
        directory
            .rename_exclusive("cli", ".mon-delete-model-test")
            .unwrap();
        let (proof, after) = crate::storage::deletion::verification::hash(
            &directory,
            ".mon-delete-model-test",
            &|| false,
        )
        .unwrap();
        assert_eq!(sha, after);
        assert!(crate::storage::deletion::verification::hash(
            &directory,
            ".mon-delete-model-test",
            &|| true
        )
        .is_err());
        directory
            .unlink_staged(".mon-delete-model-test", false, &proof)
            .unwrap();
        assert!(!f.path(".mon-delete-model-test").exists());
    }
    std::fs::write(f.path("cli"), b"new generation").unwrap();
    let (_, sha) =
        crate::storage::deletion::verification::hash(&directory, "cli", &|| false).unwrap();
    assert_eq!(sha, format!("{:x}", Sha256::digest(b"new generation")));
}

#[test]
#[ignore = "explicit private QA root only; creates synthetic packages without executing them"]
fn seed_private_model_qa() {
    use std::io::Write;
    let root = PathBuf::from(std::env::var_os("MON_OPERATION_QA_ROOT").expect("private QA root"));
    let app = root
        .file_name()
        .is_some_and(|s| s == "jp.mon.desktop.qa.operation-performance-20261026");
    let benchmark = root.ends_with("windows-parity/model-validation")
        && root.starts_with(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../output")
                .canonicalize()
                .unwrap(),
        );
    assert!(app || benchmark, "never seed an existing user application");
    assert!(!root.exists(), "private root must be new");
    crate::native_fs::private_directories(&root, true).unwrap();
    let root = root.canonicalize().unwrap();
    let storage = Storage::open(if app { root.clone() } else { root.join("data") }).unwrap();
    let f = Fixture::new();
    let bytes: usize = std::env::var("MON_OPERATION_QA_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150000);
    assert!(bytes <= 2 * 1024 * 1024 * 1024);
    let mut state = serde_json::Map::new();
    for (index, key) in ["windows-whisper-194-tiny", "windows-whisper-194-base"]
        .into_iter()
        .enumerate()
    {
        let mut file = std::fs::File::create(f.source.join("weights/model.bin")).unwrap();
        let block = vec![b'm'; 1024 * 1024];
        let mut remaining = if index == 0 { bytes } else { 150000 };
        while remaining > 0 {
            let n = remaining.min(block.len());
            file.write_all(&block[..n]).unwrap();
            remaining -= n;
        }
        file.sync_all().unwrap();
        drop(file);
        let report = model_inspection::inspect(&f.source, &|| false).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        model_placement::place(
            &storage,
            &root.join("models"),
            &f.source,
            &id,
            &report.review_hash,
            &|| false,
        )
        .unwrap();
        state.insert(
            key.into(),
            serde_json::json!({"package":id,"review":report.review_hash}),
        );
    }
    std::fs::write(
        root.join("model-state.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    println!("privateQaRoot={}", root.display());
}

#[test]
#[ignore = "explicit synthetic benchmark; writes and removes only private temporary packages"]
fn benchmark_model_review_and_removal() {
    use std::io::Write;
    let bytes: usize = std::env::var("MON_OPERATION_BENCH_BYTES")
        .unwrap_or("268435456".into())
        .parse()
        .unwrap();
    assert!(matches!(bytes, 268435456 | 2147483648));
    for run in 0..3 {
        let f = Fixture::new();
        let mut file = std::fs::File::create(f.path("weights/model.bin")).unwrap();
        let block = vec![b'm'; 1024 * 1024];
        for _ in 0..bytes / block.len() {
            file.write_all(&block).unwrap();
        }
        file.sync_all().unwrap();
        drop(file);
        let owner = AnalysisController::default()
            .begin_model_deletion(false, false, &|| false)
            .unwrap();
        let _cache = (std::env::var("MON_OPERATION_BENCH_UNCACHED").as_deref() != Ok("1"))
            .then(crate::storage::deletion::verification::Scope::enter);
        let _measure = crate::operation_observation::Scope::enter("benchmark", |_| {});
        let started = std::time::Instant::now();
        let mut p = operations::Prepared::fresh(
            &f.storage,
            &f.root,
            &f.package,
            &f.review,
            &owner,
            &|| false,
        )
        .unwrap();
        let review_ms = started.elapsed().as_millis();
        let review_reads = crate::operation_observation::measurements();
        // Preview and execution are distinct native workers. Never reuse pins
        // across the human confirmation boundary in the measured path.
        drop(_cache);
        let _cache = (std::env::var("MON_OPERATION_BENCH_UNCACHED").as_deref() != Ok("1"))
            .then(crate::storage::deletion::verification::Scope::enter);
        let started = std::time::Instant::now();
        let result = p
            .execute(
                &f.storage,
                &owner,
                &p.preview().confirmation_id,
                false,
                &|| false,
                &|_| {},
            )
            .unwrap();
        assert!(result.body_deleted && !f.path("").exists());
        println!(
            "{}",
            serde_json::json!({"bytes":bytes,"run":run,"reviewMs":review_ms,"removalMs":started.elapsed().as_millis(),
            "reviewReadBytes":review_reads.0,"reviewChecks":review_reads.1,"totalReadBytes":crate::operation_observation::measurements().0,
            "totalChecks":crate::operation_observation::measurements().1})
        );
    }
}

#[test]
fn meeting_and_audio_review_release_database_during_payload_reads_and_detect_changes() {
    for audio_only in [false, true] {
        let f = Fixture::new();
        let meeting = f.storage.list_meetings().unwrap()[0].id.clone();
        let folder = f.storage.root().join("meetings").join(&meeting);
        crate::native_fs::private_directories(&folder, true).unwrap();
        std::fs::write(folder.join("unclassified.bin"), vec![b'x'; 150000]).unwrap();
        let observed = Cell::new(false);
        let _measure = crate::operation_observation::Scope::enter("test", |_| {});
        let capture = |change: bool| {
            let changed = Cell::new(false);
            let check = || {
                if crate::operation_observation::measurements().1 > 0 {
                    if let Ok(db) = f.storage.connection.try_lock() {
                        observed.set(true);
                        if change && !changed.replace(true) {
                            db.execute(
                                "UPDATE meetings SET title='changed during review' WHERE id=?1",
                                [&meeting],
                            )
                            .unwrap();
                        }
                    }
                }
                false
            };
            if audio_only {
                crate::storage::deletion::audio::Census::capture(&f.storage, &meeting, &check)
                    .map(|_| ())
            } else {
                crate::storage::deletion::files::Census::capture(&f.storage, &meeting, &check)
                    .map(|_| ())
            }
        };
        capture(false).unwrap();
        assert!(
            observed.get(),
            "payload reads retained the shared connection"
        );
        // Reset measurements so the mutation occurs inside the next payload read.
        let _measure = crate::operation_observation::Scope::enter("test", |_| {});
        assert!(capture(true).is_err());
        assert!(folder.join("unclassified.bin").is_file());
    }
}

#[cfg(windows)]
#[test]
fn windows_native_appdata_spelling_reviews_and_deletes_only_the_confirmed_model() {
    let f = Fixture::new();
    let plain = PathBuf::from(f.root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert_ne!(plain, plain.canonicalize().unwrap());
    let opened = crate::storage::deletion::fs::RootAnchor::directory(&plain).unwrap();
    let canonical = crate::storage::deletion::fs::RootAnchor::directory(&f.root).unwrap();
    assert_eq!(opened.proof().unwrap(), canonical.proof().unwrap());
    assert!(
        crate::storage::deletion::fs::RootAnchor::directory(&plain.join("..").join("models"))
            .is_err()
    );
    // Parent pins intentionally protect this model root until the assertion ends.
    drop(opened);
    drop(canonical);
    let owner = AnalysisController::default()
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let mut p =
        operations::Prepared::fresh(&f.storage, &plain, &f.package, &f.review, &owner, &|| false)
            .unwrap();
    let receipt = p
        .execute(
            &f.storage,
            &owner,
            &p.preview().confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.body_deleted && !f.path("").exists());
    assert_eq!(f.storage.list_meetings().unwrap()[0].title, "KEEP");
}
