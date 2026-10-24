use super::*;
use crate::analysis_worker::AnalysisController;
use crate::storage::{deletion::operations::Prepared, JobKind, JobSpec};

fn job(storage: &Storage, meeting: &str, operation: &str) -> JobSnapshot {
    storage
        .enqueue_job(JobSpec {
            meeting_id: meeting.into(),
            kind: JobKind::Transcription,
            operation_id: operation.into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({"engine":"test"}),
            parameters: serde_json::json!({"schemaVersion":4,"window_seconds":30}),
            total_units: Some(2),
        })
        .unwrap()
}

#[test]
fn scoped_deletion_removes_stale_recording_while_other_job_saves_and_finishes() {
    for lifecycle in ["recording", "recorded"] {
        let root = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(Storage::open(root.path().join("data")).unwrap());
        let target = storage.create_meeting("削除する会議").unwrap().id;
        let other = super::analysis::tests::saved_meeting(&storage).id;
        let keep_job = job(&storage, &other, "other-job");
        let (running, _owner) = storage.acquire_owned_inference_job().unwrap().unwrap();
        assert_eq!(running.id, keep_job.id);
        let queued = job(&storage, &target, "target-queued");
        let relative = std::path::PathBuf::from(format!("meetings/{target}/audio/raw.caf"));
        let audio = storage.root().join(&relative);
        let stale_analysis = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
        std::fs::write(&audio, b"synthetic recording").unwrap();
        storage
            .register_ready_meeting_asset(&target, &relative, "audio/x-caf")
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE meetings SET lifecycle=?2,source_type='recording' WHERE id=?1",
                params![target, lifecycle],
            )
            .unwrap();
            db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES(?1,?2,'recording','system','{}',?1,0,0)",params![uuid::Uuid::new_v4().to_string(),target]).unwrap();
            db.execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES(?1,?2,1,'engine','building',?3,'{}','{}',?3,1,0)",params![stale_analysis,target,"a".repeat(64)]).unwrap();
        }
        let controller = AnalysisController::default();
        let lease = controller
            .begin_meeting_deletion(&storage, &target)
            .unwrap();
        assert!(controller
            .begin_data_maintenance(false, false, &|| false)
            .is_err());
        assert!(storage
            .begin_meeting_work(&target, "recording", None)
            .is_err());
        assert!(!storage.data_write_blocked());
        let mut prepared = Prepared::fresh(&storage, &target, &|| false).unwrap();
        let preview = prepared.preview();
        storage
            .save_job_checkpoint(&keep_job.id, 0, 1, serde_json::json!({"next":1}))
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            assert!(db
                .execute(
                    "UPDATE meetings SET title='FORBIDDEN' WHERE id=?1",
                    [&target]
                )
                .is_err());
            assert!(db
                .execute(
                    "UPDATE processing_jobs SET state='running' WHERE id=?1",
                    [&queued.id]
                )
                .is_err());
            assert!(db.execute("INSERT INTO job_checkpoints(id,job_id,sequence,completed_units,checkpoint_json,checkpoint_hash,created_at) VALUES('late',?1,1,0,'{}',?2,0)",params![queued.id,"a".repeat(64)]).is_err());
            assert!(db
                .execute("DELETE FROM temp.mon_meeting_deletion_guard", [])
                .is_err());
            assert!(db
                .execute_batch("DROP TRIGGER temp.mon_meeting_guard_meetings_UPDATE")
                .is_err());
            let foreign = db
                .execute(
                    "UPDATE meetings SET previous_analysis_id=?1 WHERE id=?2",
                    params![stale_analysis, other],
                )
                .unwrap_err();
            assert!(
                foreign.to_string().contains("meeting_deletion_reserved"),
                "{foreign}"
            );
            assert!(db
                .execute(
                    "UPDATE meetings SET title='別会議は保存可能' WHERE id=?1",
                    [&other]
                )
                .is_ok());
        }
        let (send, receive) = std::sync::mpsc::channel();
        let writer_storage = storage.clone();
        let writer_id = keep_job.id.clone();
        let writer_meeting = other.clone();
        let writer = std::thread::spawn(move || {
            receive
                .recv_timeout(std::time::Duration::from_secs(30))
                .unwrap();
            writer_storage
                .save_job_checkpoint(&writer_id, 1, 2, serde_json::json!({"next":2}))
                .unwrap();
            let candidate = writer_storage
                .create_analysis_candidate(&writer_meeting, super::analysis::tests::input())
                .unwrap();
            let version = writer_storage
                .get_meeting(&writer_meeting)
                .unwrap()
                .unwrap()
                .lock_version;
            writer_storage
                .commit_analysis_candidate(&writer_meeting, &candidate, version, false)
                .unwrap();
            candidate
        });
        let sent = std::cell::Cell::new(false);
        assert!(
            prepared
                .execute(
                    &storage,
                    &lease,
                    &preview.confirmation_id,
                    false,
                    &|| false,
                    &|phase| {
                        if phase == "deleting_files" && !sent.replace(true) {
                            send.send(()).unwrap();
                        }
                    }
                )
                .unwrap()
                .deleted
        );
        let published = writer.join().unwrap();
        storage
            .finish_job(&keep_job.id, 2, "succeeded", None)
            .unwrap();
        storage.release_meeting_deletion(&target).unwrap();
        assert!(storage.get_meeting(&target).unwrap().is_none());
        assert!(!audio.exists());
        assert_eq!(storage.get_job(&keep_job.id).unwrap().state, "succeeded");
        assert_eq!(
            storage.load_current_analysis(&other).unwrap().unwrap().id,
            published
        );
        assert_eq!(
            storage.get_meeting(&other).unwrap().unwrap().title,
            "別会議は保存可能"
        );
    }
}

#[test]
fn interrupted_scoped_deletion_keeps_its_fence_and_can_resume() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("data")).unwrap();
    let target = storage.create_meeting("削除を中断する会議").unwrap().id;
    let other = storage.create_meeting("影響しない会議").unwrap().id;
    let controller = AnalysisController::default();
    let lease = controller
        .begin_meeting_deletion(&storage, &target)
        .unwrap();
    let mut prepared = Prepared::fresh(&storage, &target, &|| false).unwrap();
    let preview = prepared.preview();
    let stop = std::cell::Cell::new(false);
    assert!(prepared
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| stop.get(),
            &|phase| {
                if phase == "deleting_files" {
                    stop.set(true);
                }
            }
        )
        .is_err());
    storage.release_meeting_deletion(&target).unwrap();
    drop(lease);
    assert!(!storage.data_write_blocked());
    assert!(storage.require_global_data_writable().is_err());
    assert!(storage.begin_meeting_work(&target, "job", None).is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET title='保持した別会議' WHERE id=?1",
            [&other],
        )
        .unwrap();
    let lease = controller
        .begin_meeting_deletion(&storage, &target)
        .unwrap();
    let mut resumed = Prepared::pending(&storage, &preview.operation_id, &|| false).unwrap();
    resumed.refresh(&storage, &|| false).unwrap();
    let p = resumed.preview();
    assert!(
        resumed
            .execute(
                &storage,
                &lease,
                &p.confirmation_id,
                false,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    storage.release_meeting_deletion(&target).unwrap();
    assert_eq!(
        storage.get_meeting(&other).unwrap().unwrap().title,
        "保持した別会議"
    );
}

#[test]
fn cold_restart_keeps_legacy_global_recovery_for_a_scoped_intent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data");
    let storage = Storage::open(path.clone()).unwrap();
    let target = storage.create_meeting("再起動して保持").unwrap().id;
    let controller = AnalysisController::default();
    let lease = controller
        .begin_meeting_deletion(&storage, &target)
        .unwrap();
    let mut prepared = Prepared::fresh(&storage, &target, &|| false).unwrap();
    let p = prepared.preview();
    let stop = std::cell::Cell::new(false);
    assert!(prepared
        .execute(
            &storage,
            &lease,
            &p.confirmation_id,
            false,
            &|| stop.get(),
            &|phase| {
                if phase == "delete_scope_fixed" {
                    stop.set(true);
                }
            }
        )
        .is_err());
    drop(prepared);
    drop(lease);
    drop(storage);
    let reopened = Storage::open(path.clone()).unwrap();
    assert!(reopened.data_write_blocked());
    assert!(controller
        .begin_meeting_deletion(&reopened, &target)
        .is_err());
    let lease = controller
        .begin_deletion_maintenance(&reopened, &p.operation_id, false, false, &|| false)
        .unwrap();
    let mut pending = Prepared::pending(&reopened, &p.operation_id, &|| false).unwrap();
    let next = pending.preview();
    assert!(
        pending
            .keep(&reopened, &lease, &next.confirmation_id, &|| false)
            .unwrap()
            .retained
    );
    assert!(reopened.restart_required());
    drop(lease);
    drop(pending);
    drop(reopened);
    let clean = Storage::open(path).unwrap();
    assert!(!clean.data_write_blocked());
    assert!(clean.get_meeting(&target).unwrap().is_some());
}

#[test]
fn reserved_target_jobs_do_not_starve_other_meetings() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("data")).unwrap();
    let target = storage.create_meeting("削除する待機会議").unwrap().id;
    let other = super::analysis::tests::saved_meeting(&storage).id;
    let target_job = job(&storage, &target, "first");
    let other_job = job(&storage, &other, "second");
    let controller = AnalysisController::default();
    let _lease = controller
        .begin_meeting_deletion(&storage, &target)
        .unwrap();
    let (claimed, _owner) = storage.acquire_owned_inference_job().unwrap().unwrap();
    assert_eq!(claimed.id, other_job.id);
    assert_eq!(storage.get_job(&target_job.id).unwrap().state, "queued");
    storage.release_meeting_deletion(&target).unwrap();
}

#[test]
fn live_owner_blocks_only_its_meeting_until_cleanup_releases_it() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("data")).unwrap();
    let target = storage.create_meeting("対象").unwrap().id;
    let other = storage.create_meeting("別会議").unwrap().id;
    for kind in ["recording", "job", "export", "import"] {
        let owner = storage
            .begin_meeting_work(&target, kind, Some("job-id"))
            .unwrap();
        let status = storage
            .meeting_deletion_availability(&[target.clone(), other.clone()])
            .unwrap();
        assert!(!status[0].available);
        assert_eq!(status[0].blockers[0].kind, kind);
        assert!(status[1].available);
        assert!(AnalysisController::default()
            .begin_meeting_deletion(&storage, &target)
            .is_err());
        drop(owner);
        let controller = AnalysisController::default();
        let lease = controller
            .begin_meeting_deletion(&storage, &target)
            .unwrap();
        assert!(storage.begin_meeting_work(&target, kind, None).is_err());
        storage.release_meeting_deletion(&target).unwrap();
        drop(lease);
    }
}

#[test]
fn confirmation_does_not_authorize_deleting_a_newly_started_job() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("data")).unwrap();
    let target = storage.create_meeting("対象").unwrap().id;
    let controller = AnalysisController::default();
    let lease = controller
        .begin_meeting_deletion(&storage, &target)
        .unwrap();
    let _preview = Prepared::fresh(&storage, &target, &|| false).unwrap();
    storage.release_meeting_deletion(&target).unwrap();
    drop(lease);
    let _owner = storage.begin_meeting_work(&target, "job", None).unwrap();
    assert!(controller
        .begin_meeting_deletion(&storage, &target)
        .is_err());
    assert!(storage.get_meeting(&target).unwrap().is_some());
}
