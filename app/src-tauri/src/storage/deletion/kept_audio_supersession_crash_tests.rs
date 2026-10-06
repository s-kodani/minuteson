//! Actual SIGKILL before/after the kept-history and FK-detachment transaction.
use super::*;
use std::os::unix::process::ExitStatusExt;

#[test]
#[ignore = "child only; private kept-history fixture supplied by parent"]
fn kept_audio_supersession_checkpoint_child() {
    let base = PathBuf::from(std::env::var_os("MON_KEPT_AUDIO_CRASH_BASE").unwrap());
    let phase = std::env::var("MON_KEPT_AUDIO_CRASH_PHASE").unwrap();
    let f = Kept::at(tempfile::tempdir_in(&base).unwrap(), true);
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut receipt = f.begin();
    let full = receipt.confirmed().operation().to_owned();
    completion::run_observed(&f.storage, &mut receipt, &|| false, &|point| {
        if point == phase {
            lease.validate_active().unwrap();
            let data = serde_json::to_vec(&(
                f.storage.root(),
                &full,
                &f.meeting,
                &f.other,
                &f.operation,
                &f.originals,
            ))
            .unwrap();
            std::fs::write(base.join("ready.tmp"), data).unwrap();
            std::fs::rename(base.join("ready.tmp"), base.join("ready.json")).unwrap();
            loop {
                std::thread::park();
            }
        }
    })
    .unwrap();
    panic!("kept-audio checkpoint not reached");
}

#[test]
fn kept_audio_supersession_sigkill_on_both_sides_of_rows_commit_preserves_consent_and_resumes() {
    for phase in ["beforeRowsCommit", "rowsCommitted"] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::deletion::audio_supersession::tests::kept_history::crash::kept_audio_supersession_checkpoint_child", "--ignored", "--nocapture"])
            .env("MON_KEPT_AUDIO_CRASH_BASE", base.path())
            .env("MON_KEPT_AUDIO_CRASH_PHASE", phase)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit())
            .spawn().unwrap();
        let ready = base.path().join("ready.json");
        let start = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || start.elapsed() > std::time::Duration::from_secs(60)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("kept-audio checkpoint not reached: {phase}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
        let (root, full, meeting, other, kept, originals): (
            PathBuf,
            String,
            String,
            String,
            String,
            Vec<(String, String, String)>,
        ) = serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
        let storage = Storage::open(root.clone()).unwrap();
        assert!(storage.data_write_blocked());
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().is_none(),
            phase == "rowsCommitted"
        );
        {
            let db = storage.connection.lock().unwrap();
            for (id, row, paths) in &originals {
                assert_eq!(original(&db, id), *row);
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
                    if phase == "rowsCommitted" {
                        None
                    } else {
                        Some(meeting.as_str())
                    }
                );
            }
            assert_eq!(startup::reason(&db, &kept).unwrap(), None);
        }
        let lease = AnalysisController::default()
            .begin_deletion_maintenance(&storage, &full, false, false, &|| false)
            .unwrap();
        let mut pending = operations::Prepared::pending(&storage, &full, &|| false).unwrap();
        assert!(
            pending
                .execute(
                    &storage,
                    &lease,
                    &pending.preview().confirmation_id,
                    false,
                    &|| false,
                    &|_| {}
                )
                .unwrap()
                .deleted
        );
        drop(lease);
        drop(storage);
        let storage = Storage::open(root).unwrap();
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&meeting).unwrap().is_none());
        assert!(storage.get_meeting(&other).unwrap().is_some());
        let db = storage.connection.lock().unwrap();
        for (id, row, paths) in &originals {
            assert_eq!(original(&db, id), *row);
            assert_eq!(path_records(&db, id), *paths);
        }
        inspect_kept(&db, &storage.managed_anchor, &kept, &|| false).unwrap();
        assert!(audio::intent::read(&db, &kept, &|| false).is_err());
    }
}
