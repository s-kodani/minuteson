//! Real WAL recovery across the kept-meeting and FK-detachment transaction.
use super::*;
use std::os::unix::process::ExitStatusExt;

#[test]
#[ignore = "child only; parent supplies private kept-meeting fixture"]
fn kept_meeting_supersession_checkpoint_child() {
    let base = PathBuf::from(std::env::var_os("MON_KEPT_MEETING_CRASH_BASE").unwrap());
    let phase = std::env::var("MON_KEPT_MEETING_CRASH_PHASE").unwrap();
    let f = KeptMeeting::at(tempfile::tempdir_in(&base).unwrap(), Some(true));
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut receipt = f.begin();
    let full = receipt.confirmed().operation().to_owned();
    completion::run_observed(&f.storage, &mut receipt, &|| false, &|event| {
        if event == phase {
            lease.validate_active().unwrap();
            let data = serde_json::to_vec(&(
                f.storage.root(),
                &full,
                &f.meeting,
                &f.other,
                &f.kept,
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
    panic!("kept-meeting checkpoint not reached");
}

#[test]
fn kept_meeting_supersession_sigkill_before_and_after_rows_commit_preserves_original_and_resumes() {
    for phase in ["beforeRowsCommit", "rowsCommitted"] {
        let base = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::deletion::audio_supersession::tests::kept_meeting::crash::kept_meeting_supersession_checkpoint_child", "--ignored", "--nocapture"])
            .env("MON_KEPT_MEETING_CRASH_BASE", base.path()).env("MON_KEPT_MEETING_CRASH_PHASE", phase)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::inherit()).spawn().unwrap();
        let ready = base.path().join("ready.json");
        let start = std::time::Instant::now();
        while !ready.exists() {
            if child.try_wait().unwrap().is_some()
                || start.elapsed() > std::time::Duration::from_secs(60)
            {
                let _ = child.kill();
                let _ = child.wait();
                panic!("kept-meeting checkpoint not reached: {phase}");
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
        ) = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        let storage = Storage::open(root.clone()).unwrap();
        assert!(storage.data_write_blocked());
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().is_none(),
            phase == "rowsCommitted"
        );
        {
            let db = storage.connection.lock().unwrap();
            for (id, original, paths) in &originals {
                assert_eq!(row(&db, id), *original);
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
        let mut p = operations::Prepared::pending(&storage, &full, &|| false).unwrap();
        assert!(
            p.execute(
                &storage,
                &lease,
                &p.preview().confirmation_id,
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
        for (id, original, paths) in &originals {
            assert_eq!(row(&db, id), *original);
            assert_eq!(path_records(&db, id), *paths);
        }
        inspect_kept_meeting(&db, &storage.managed_anchor, &kept, &|| false).unwrap();
        assert!(intent::read_kept_database(&db, &kept, &|| false).is_err());
    }
}
