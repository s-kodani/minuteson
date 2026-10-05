use super::*;
use std::sync::atomic::Ordering;

#[test]
fn retired_source_keeps_reads_but_rejects_writes_and_new_stages_after_snapshot_drop() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    let stage = saved.stage.path().to_owned();
    saved.retire_source_for_restart(&|| false).unwrap();
    drop(saved);
    assert!(!stage.exists());
    assert!(f.storage.retired.load(Ordering::Acquire));
    assert!(f.storage.maintenance.load(Ordering::Acquire));
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert!(f.storage.create_meeting("旧保存先へは追加しない").is_err());
    assert!(f.storage.backup_stage().is_err());
    assert!(f.storage.prepare_backup_snapshot(&|| false).is_err());
    assert!(f.storage.prepare_relocation_snapshot(&|| false).is_err());
    let db = f.storage.connection.lock().unwrap();
    for sql in [
        "UPDATE meetings SET title='変更しない'",
        "DELETE FROM meetings",
        "CREATE TABLE forbidden(value TEXT)",
        "PRAGMA user_version=123",
    ] {
        assert!(db.execute_batch(sql).is_err(), "{sql}");
    }
    drop(db);
    assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
}

#[test]
fn retired_source_exits_without_mutating_idle_or_queued_jobs() {
    let f = Fixture::new();
    let job = f
        .storage
        .enqueue_job(crate::storage::JobSpec {
            meeting_id: f.meeting.clone(),
            kind: crate::storage::JobKind::Transcription,
            operation_id: "queued-before-retirement".into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({"engine": "test"}),
            parameters: serde_json::json!({}),
            total_units: Some(10),
        })
        .unwrap();
    let saved = snapshot(&f);
    let before = native_tables(&f.storage.connection.lock().unwrap());
    saved.retire_source_for_restart(&|| false).unwrap();
    f.storage.pause_for_exit().unwrap();
    drop(saved);
    f.storage.pause_for_exit().unwrap();
    assert_eq!(before, native_tables(&f.storage.connection.lock().unwrap()));
    assert_eq!(f.storage.get_job(&job.id).unwrap(), job);
}

#[test]
fn cancellation_or_invalid_image_does_not_retire_a_valid_source() {
    for failure in ["cancel", "image"] {
        let f = Fixture::new();
        let saved = snapshot(&f);
        if failure == "image" {
            std::fs::write(saved.image.join("unexpected"), b"not sealed").unwrap();
        }
        assert!(saved
            .retire_source_for_restart(&|| failure == "cancel")
            .is_err());
        assert!(!f.storage.retired.load(Ordering::Acquire));
        drop(saved);
        f.storage
            .create_meeting("切替前の拒否なら旧保存先を継続")
            .unwrap();
    }
}

#[test]
fn changed_source_files_prevent_retirement_without_authorizing_the_copy() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    std::fs::write(f.path(), b"changed audio").unwrap();
    assert!(saved.retire_source_for_restart(&|| false).is_err());
    assert!(!f.storage.retired.load(Ordering::Acquire));
    drop(saved);
    f.storage
        .create_meeting("元ルートは有効・コピーは不採用")
        .unwrap();
    assert_eq!(std::fs::read(f.path()).unwrap(), b"changed audio");
}

#[test]
fn cancellation_at_the_final_locked_check_leaves_source_writable() {
    let checks = {
        let f = Fixture::new();
        let saved = snapshot(&f);
        let calls = Cell::new(0usize);
        saved
            .retire_source_for_restart(&|| {
                calls.set(calls.get() + 1);
                false
            })
            .unwrap();
        calls.get()
    };
    assert!(checks > 1);
    let f = Fixture::new();
    let saved = snapshot(&f);
    let calls = Cell::new(0usize);
    assert!(saved
        .retire_source_for_restart(&|| {
            calls.set(calls.get() + 1);
            calls.get() == checks
        })
        .is_err());
    assert_eq!(calls.get(), checks);
    assert!(!f.storage.retired.load(Ordering::Acquire));
    drop(saved);
    f.storage.create_meeting("停止確定直前の取消").unwrap();
}

#[test]
fn retirement_is_idempotent_and_unwinding_does_not_release_the_read_only_fence() {
    let f = Fixture::new();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let saved = snapshot(&f);
        saved.retire_source_for_restart(&|| false).unwrap();
        saved.retire_source_for_restart(&|| false).unwrap();
        panic!("publication attempt failed after source retirement");
    }));
    assert!(failure.is_err());
    assert!(f.storage.create_meeting("解除されない").is_err());
    assert!(f.storage.backup_stage().is_err());
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
    f.storage.pause_for_exit().unwrap();
}

#[test]
fn uncertain_database_binding_overrides_retired_read_access() {
    let f = Fixture::new();
    let saved = snapshot(&f);
    saved.retire_source_for_restart(&|| false).unwrap();
    let db = f.storage.root.join("meetings.sqlite3");
    let held = f.storage.root.join("retained-database");
    std::fs::rename(&db, &held).unwrap();
    assert!(saved.verify_source(&|| false).is_err());
    drop(saved);
    assert!(f.storage.list_meetings().is_err());
    assert!(f.storage.create_meeting("不確定DBには書かない").is_err());
    assert!(f.storage.pause_for_exit().is_err());
    assert!(held.exists());
    assert!(!db.exists());
    assert_eq!(std::fs::read(f.path()).unwrap(), b"original audio");
}
