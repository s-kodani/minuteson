//! Product rejection of retired jobs; historical data seeding is separate.
use super::*;
use crate::generation::{self, stored_fixture};

fn request(storage: &Storage) -> MinutesJobRequest {
    let meeting = stored_minutes_fixture::meeting(storage);
    std::fs::create_dir_all(storage.root.join("meetings").join(&meeting)).unwrap();
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    }
}
#[test]
fn retired_worker_never_executes_or_changes_saved_input_checkpoint_or_human_markdown() {
    for version in [1, 2, 3, 4] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let request = request(&storage);
        let human = storage
            .save_minutes_markdown(minutes_markdown::SaveMarkdownRequest {
                meeting_id: request.meeting_id.clone(),
                operation_id: Uuid::new_v4().to_string(),
                expected: None,
                markdown: "保持する人間本文".into(),
            })
            .unwrap();
        let job = stored_fixture::seed_job(
            &storage,
            request,
            stored_fixture::engine(root.path(), false),
            "standard",
            2400,
            version,
        )
        .unwrap();
        storage.acquire_local_inference_job().unwrap().unwrap();
        let before = storage.get_job(&job.id).unwrap();
        let fixed = storage.fixed_job_input(&job.id).unwrap();
        let error = generation::process(&storage, &job.id, &|| false, &|_| {
            panic!("retired job emitted progress")
        })
        .unwrap_err();
        assert!(error.contains("minutes_structured_generation_removed"));
        assert_eq!(storage.get_job(&job.id).unwrap(), before);
        assert_eq!(storage.fixed_job_input(&job.id).unwrap(), fixed);
        assert_eq!(
            storage.get_minutes_markdown(&job.meeting_id).unwrap(),
            Some(human)
        );
        assert!(std::fs::read(root.path().join("engine.calls"))
            .unwrap()
            .is_empty());
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM minutes_revisions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn retired_retry_is_rejected_before_creating_jobs_or_mutating_past_results() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let request = request(&storage);
    let job = stored_fixture::seed_job(
        &storage,
        request.clone(),
        stored_fixture::engine(root.path(), false),
        "standard",
        2400,
        4,
    )
    .unwrap();
    let revision = stored_fixture::seed_outputs(&storage, &job, false, false).unwrap();
    let saved = storage
        .load_minutes_content(&job.meeting_id, &revision)
        .unwrap();
    assert!(saved.is_some(), "the protected historical result must exist");
    // A cancelled queued fixture is retry-shaped but is never executable.
    let mut req = request;
    req.operation_id = Uuid::new_v4().to_string();
    let pending = stored_fixture::seed_job(
        &storage,
        req,
        stored_fixture::engine(root.path(), false),
        "standard",
        2400,
        4,
    )
    .unwrap();
    let failed = storage
        .finish_job(&pending.id, 0, "cancelled", None)
        .unwrap();
    let fixed = storage.fixed_job_input(&failed.id).unwrap();
    let error = generation::RetryRequest {
        meeting_id: failed.meeting_id.clone(),
        job_id: failed.id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_spec_hash: failed.spec_hash.clone(),
    }
    .enqueue(&storage)
    .unwrap_err();
    assert!(error.contains("minutes_structured_generation_removed"));
    assert_eq!(storage.get_job(&failed.id).unwrap(), failed);
    assert_eq!(storage.fixed_job_input(&failed.id).unwrap(), fixed);
    assert_eq!(
        serde_json::to_value(
            storage
                .load_minutes_content(&job.meeting_id, &revision)
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(saved).unwrap()
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
