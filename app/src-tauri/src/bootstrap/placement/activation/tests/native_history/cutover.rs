//! Production folder confirmation/publication, crash recovery and admission.
use super::super::ChildGuard;
use super::*;
use crate::storage::deletion::relocation::receiver::{
    synthetic_all_deletion_evidence as evidence, synthetic_keep_meeting as keep,
};
use std::{io::BufRead, os::unix::process::ExitStatusExt, sync::mpsc, time::Duration};

fn mixed(completed_model: bool) -> (NativeFixture, PathBuf) {
    let f = NativeFixture::new(false, false);
    let storage = f.storage.as_ref().unwrap();
    let archive = f._base.path().join("cutover.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let restore = storage
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let restored = restore
        .publish(
            storage,
            &restore.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap()
        .meeting_ids[0]
        .clone();
    drop(restore);
    keep(storage, &restored);
    let mut full = operations::Prepared::fresh(storage, &restored, &|| false).unwrap();
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(
        full.execute(
            storage,
            &lease,
            &full.preview().confirmation_id,
            true,
            &|| false,
            &|_| {}
        )
        .unwrap()
        .deleted
    );
    drop(lease);
    keep(storage, &f.meeting);
    let (_, model) = synthetic_model(&f, completed_model);
    (f, model)
}

#[test]
fn deletion_cutover_public_folder_mixed_history_stale_command_refusal_and_repeat_move() {
    for completed_model in [false, true] {
        let (mut f, model) = mixed(completed_model);
        let b = f.bootstrap.as_ref().unwrap();
        let storage = f.storage.as_ref().unwrap();
        let original = evidence(storage);
        let old = storage.root().to_owned();
        let mut prepared = b
            .prepare_move(storage, f._base.path(), &|| false, &|_| {})
            .unwrap();
        let view = prepared.preview();
        assert!(!view.destination.exists());
        let result = prepared
            .publish(b, storage, &view.confirmation_id, &|| false, &|_| {})
            .unwrap();
        assert!(result.restart_required && storage.restart_required());
        assert_eq!(result.destination, view.destination);
        assert_eq!(b.view().unwrap().active_meeting_root, old);
        assert!(storage
            .create_meeting("OLD WRITES MUST STAY RETIRED")
            .is_err());
        assert_eq!(evidence(storage), original);
        // The controller can replay its receipt, but a retired lower-level
        // folder command must never create another copy or publish again.
        let config = std::fs::read(f.config.join(NAME)).unwrap();
        assert!(prepared
            .publish(b, storage, &view.confirmation_id, &|| false, &|_| {})
            .is_err());
        assert_eq!(std::fs::read(f.config.join(NAME)).unwrap(), config);
        drop(prepared);
        drop(f.storage.take());
        drop(f.bootstrap.take());
        let (b, storage) = f.open();
        assert_eq!(storage.root(), result.destination);
        assert!(!storage.data_write_blocked());
        assert_eq!(evidence(&storage), original);
        assert_eq!(model.exists(), !completed_model);
        assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
        storage
            .create_meeting("CURRENT WRITES AFTER OWNED ADMISSION")
            .unwrap();
        let next = f._base.path().join("next-public");
        std::fs::create_dir(&next).unwrap();
        let mut again = b.prepare_move(&storage, &next, &|| false, &|_| {}).unwrap();
        let second = again
            .publish(
                &b,
                &storage,
                &again.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap();
        drop(again);
        drop(storage);
        drop(b);
        for _ in 0..2 {
            let (b, storage) = f.open();
            assert_eq!(storage.root(), second.destination);
            assert!(!storage.data_write_blocked());
            assert_eq!(evidence(&storage), original);
            assert_eq!(model.exists(), !completed_model);
            assert!(old.join("meetings.sqlite3").exists());
            drop(storage);
            drop(b);
        }
    }
}

#[test]
fn deletion_cutover_publication_errors_before_selection_and_after_rename_keep_old_writes_retired() {
    for boundary in ["before_selection", "after_rename"] {
        let (mut f, model) = mixed(false);
        let b = f.bootstrap.as_ref().unwrap();
        let storage = f.storage.as_ref().unwrap();
        let old = storage.root().to_owned();
        let original = evidence(storage);
        let mut p = b
            .prepare_move(storage, f._base.path(), &|| false, &|_| {})
            .unwrap();
        let view = p.preview();
        // Arm only after pending selection/copy/history publication. Otherwise
        // after_rename would fault the initial begin_move, before retirement.
        let result = p.publish(b, storage, &view.confirmation_id, &|| false, &|phase| {
            if phase == "switching" {
                FAULT.with(|fault| fault.set(Some(boundary)));
            }
        });
        FAULT.with(|fault| fault.set(None));
        assert!(result.unwrap_err().contains("injected activation failure"));
        assert!(storage.restart_required());
        assert!(storage
            .create_meeting("NO OLD WRITES AFTER PUBLICATION ATTEMPT")
            .is_err());
        assert_eq!(evidence(storage), original);
        drop(p);
        drop(f.storage.take());
        drop(f.bootstrap.take());
        let (b, storage) = f.open();
        assert_eq!(
            storage.root(),
            if boundary == "before_selection" {
                &old
            } else {
                &view.destination
            }
        );
        assert!(!storage.data_write_blocked());
        assert_eq!(evidence(&storage), original);
        assert!(model.exists());
        if boundary == "before_selection" {
            let review = b
                .prepare_pending_move(&storage, &|| false, &|_| {})
                .unwrap();
            assert!(review.preview().resume_allowed);
            assert!(
                review
                    .resume(
                        &b,
                        &storage,
                        &review.preview().confirmation_id,
                        &|| false,
                        &|_| {}
                    )
                    .unwrap()
                    .restart_required
            );
            drop(storage);
            drop(b);
            let (b, storage) = f.open();
            assert_eq!(storage.root(), view.destination);
            assert!(!storage.data_write_blocked());
            assert_eq!(evidence(&storage), original);
            drop(storage);
            drop(b);
        }
    }
}

#[test]
#[ignore = "child only; actual parent-owned mixed deletion histories and cutover paths required"]
fn deletion_cutover_checkpoint_child() {
    let (config, legacy, parent, ready): (PathBuf, PathBuf, PathBuf, PathBuf) =
        serde_json::from_str(&std::env::var("MON_DELETION_CUTOVER_PATHS").unwrap()).unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let mut p = b
        .prepare_move(&storage, &parent, &|| false, &|_| {})
        .unwrap();
    std::fs::write(ready, serde_json::to_vec(&p.preview().destination).unwrap()).unwrap();
    p.publish(
        &b,
        &storage,
        &p.preview().confirmation_id,
        &|| false,
        &|_| {},
    )
    .unwrap();
    drop(p);
    drop(storage);
    drop(b);
    let _b = Bootstrap::open(&config, &legacy).unwrap();
    panic!("deletion cutover checkpoint was not reached");
}

#[test]
fn deletion_cutover_sigkill_selection_admission_and_tamper_use_only_durable_owned_state() {
    for phase in [
        "before_selection",
        "after_selection",
        "after_admission",
        "after_selection_tamper",
    ] {
        let (mut f, model) = mixed(false);
        let original = evidence(f.storage.as_ref().unwrap());
        let old = f.storage.as_ref().unwrap().root().to_owned();
        drop(f.storage.take());
        drop(f.bootstrap.take());
        let ready = f._base.path().join("cutover-ready.json");
        let checkpoint = if phase == "after_selection_tamper" {
            "after_selection"
        } else {
            phase
        };
        let (tx, rx) = mpsc::channel();
        let mut process = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bootstrap::placement::activation::tests::native_history::cutover::deletion_cutover_checkpoint_child", "--ignored", "--nocapture"])
            .env("MON_DELETION_CUTOVER_PATHS", serde_json::to_string(&(&f.config, &f.legacy, f._base.path(), &ready)).unwrap())
            .env("MON_ACTIVATION_TEST_CHECKPOINT", checkpoint)
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit()).spawn().unwrap();
        let stdout = process.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                match line {
                    Ok(s) if s.starts_with("MON_ACTIVATION_READY=") => {
                        let _ = tx.send(Ok(s));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                    _ => {}
                }
            }
            let _ = tx.send(Err("child exited before cutover checkpoint".into()));
        });
        let mut child = ChildGuard {
            child: process,
            reader: Some(reader),
        };
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(60)).unwrap().unwrap(),
            format!("MON_ACTIVATION_READY={checkpoint}")
        );
        child.child.kill().unwrap();
        assert_eq!(child.child.wait().unwrap().signal(), Some(libc::SIGKILL));
        if let Some(reader) = child.reader.take() {
            reader.join().unwrap();
        }
        drop(child);
        let target: PathBuf = serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap();
        let document: Document =
            serde_json::from_slice(&std::fs::read(f.config.join(NAME)).unwrap()).unwrap();
        assert_eq!(
            document.record.current.path,
            if phase == "before_selection" {
                old.clone()
            } else {
                target.clone()
            }
        );
        let operation = document
            .record
            .activation
            .as_ref()
            .map(|a| &a.pending.operation_id)
            .or_else(|| {
                document
                    .record
                    .pending_move
                    .as_ref()
                    .map(|p| &p.operation_id)
            })
            .unwrap();
        let owner =
            Connection::open(f.config.join(format!(".relocation-{operation}.sqlite3"))).unwrap();
        assert_eq!(
            owner
                .query_row("SELECT count(*) FROM admission", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            i64::from(phase == "after_admission")
        );
        drop(owner);
        if phase == "after_selection_tamper" {
            let before = std::fs::read(target.join("meetings.sqlite3")).unwrap();
            let file = target
                .join("meetings")
                .join(&f.meeting)
                .join("audio/retained.txt");
            std::fs::write(&file, b"CHANGED BEFORE FIRST ADMISSION").unwrap();
            assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
            assert_eq!(
                std::fs::read(target.join("meetings.sqlite3")).unwrap(),
                before
            );
            assert!(model.exists());
            assert!(old.join("meetings.sqlite3").exists());
            continue;
        }
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(evidence(&storage), original);
        assert!(model.exists());
        if phase == "before_selection" {
            let review = b
                .prepare_pending_move(&storage, &|| false, &|_| {})
                .unwrap();
            assert!(review.preview().resume_allowed);
            assert!(
                review
                    .resume(
                        &b,
                        &storage,
                        &review.preview().confirmation_id,
                        &|| false,
                        &|_| {}
                    )
                    .unwrap()
                    .restart_required
            );
            drop(storage);
            drop(b);
        } else {
            storage
                .create_meeting("WRITABLE AFTER OWNED CRASH RECOVERY")
                .unwrap();
            drop(storage);
            drop(b);
        }
        for _ in 0..2 {
            let (b, storage) = f.open();
            assert_eq!(storage.root(), target);
            assert!(!storage.data_write_blocked());
            assert_eq!(evidence(&storage), original);
            assert!(model.exists() && old.join("meetings.sqlite3").exists());
            drop(storage);
            drop(b);
        }
    }
}
