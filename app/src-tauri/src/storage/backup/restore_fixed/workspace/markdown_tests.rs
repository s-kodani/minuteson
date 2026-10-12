use super::*;
use crate::storage::backup::publication::Publication;
use crate::storage::backup::restore_fixed::workspace::seal::SealedWorkspace;
use crate::storage::markdown_generation_tests as synthetic;
use crate::storage::minutes_markdown::{MarkdownDocument, SaveMarkdownRequest};
use crate::storage::{DraftStamp, MinutesJobRequest};

fn save(storage: &Storage, meeting: &str, text: String) -> MarkdownDocument {
    let expected = storage
        .get_minutes_markdown(meeting)
        .unwrap()
        .map(|d| d.stamp());
    storage
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: meeting.into(),
            operation_id: Uuid::new_v4().to_string(),
            expected,
            markdown: text,
        })
        .unwrap()
}
fn queued(storage: &Storage, root: &Path, partial: bool) -> (String, JobSnapshot) {
    let meeting = archive::tests::meeting(storage, "markdown-generation");
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    if partial {
        draft.payload.utterances = vec![crate::storage::drafts::UtteranceEdit {
            utterance_id: analysis.utterances[0].id.clone(),
            effective_text: "点検日を確認する。".repeat(130),
            speaker_ids: vec![],
        }];
        draft = storage
            .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
            .unwrap();
        save(storage, &meeting, "生成を待つ間も保持する人間の本文".into());
    }
    let req = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let expected = storage
        .get_minutes_markdown(&meeting)
        .unwrap()
        .map(|d| d.stamp());
    let job = crate::generation::markdown_job::enqueue_synthetic(
        storage,
        req,
        synthetic::engine(root, if partial { "wide" } else { "normal" }),
        "standard",
        2400,
        expected,
    )
    .unwrap();
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    if partial {
        assert!(synthetic::process(storage, &job.id, &|| false, &|j| {
            if j.state == "running"
                && storage
                    .fixed_job_input(&job.id)
                    .unwrap()
                    .1
                    .as_ref()
                    .is_some_and(|c| {
                        c["pending"]["batches"]
                            .as_array()
                            .is_some_and(|b| b.len() == 1)
                    })
            {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap()
        .is_none());
    } else {
        assert!(synthetic::process(storage, &job.id, &|| false, &|_| {})
            .unwrap()
            .is_some());
    }
    let job = storage.get_job(&job.id).unwrap();
    (meeting, job)
}

#[test]
fn markdown_generation_archive_publication_reopen_and_rebackup_keep_human_body_and_fixed_progress()
{
    // One callback type throughout the generic restore stack keeps test builds
    // bounded; these are private synthetic files, not installed model assets.
    let check: fn() -> bool = || false;
    for partial in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, mut job) = queued(&storage, root.path(), partial);
        if partial {
            let parent = storage
                .finish_job(
                    &job.id,
                    job.completed_units,
                    "failed",
                    Some("synthetic_failure"),
                )
                .unwrap();
            job = crate::generation::RetryRequest {
                meeting_id: meeting.clone(),
                job_id: parent.id,
                operation_id: Uuid::new_v4().to_string(),
                expected_spec_hash: parent.spec_hash,
            }
            .enqueue(&storage)
            .unwrap();
        } else {
            save(&storage,&meeting,format!("\r\n# 人間が編集した本文\n自然文の旧ID {meeting}\n[区間](audio:00:00:00.100001-00:00:00.799999)\n"));
        }
        let original = storage.get_minutes_markdown(&meeting).unwrap().unwrap();
        let cp = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
        let before = synthetic::calls(root.path());
        let output = tempfile::tempdir().unwrap();
        let archive_path = output.path().join("generation.monbackup");
        storage
            .create_backup_archive(&archive_path, &check)
            .unwrap();
        let source = archive::read(
            &mut File::open(archive_path).unwrap(),
            output.path(),
            &check,
        )
        .unwrap();
        let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &check)
            .unwrap();
        let fixed = FixedData::new(&ids, output.path(), &check).unwrap();
        // Restoration must work with engines absent; never reconstruct local
        // paths or run inference just to read saved Markdown and checkpoints.
        std::fs::remove_file(root.path().join("llama-completion")).unwrap();
        std::fs::remove_file(root.path().join("model.gguf")).unwrap();
        let sealed = SealedWorkspace::prepare(&fixed, output.path(), &check).unwrap();
        let target_root = tempfile::tempdir().unwrap();
        let target = Storage::open(target_root.path().into()).unwrap();
        let untouched = target.create_meeting("既存会議").unwrap().id;
        let kept = save(&target, &untouched, "この本文は変更しない".into());
        assert!(matches!(
            target.publish_restore(&sealed, &ids, &check).unwrap(),
            Publication::Published { .. }
        ));
        let new_meeting = ids.get("meetings", "", &meeting).unwrap();
        let new_job = ids.get("processing_jobs", &meeting, &job.id).unwrap();
        let doc = target.get_minutes_markdown(&new_meeting).unwrap().unwrap();
        assert_eq!(doc.markdown, original.markdown);
        assert_eq!(doc.content_hash, original.content_hash);
        assert_eq!(doc.lock_version, original.lock_version);
        assert_ne!(doc.operation_id, original.operation_id);
        assert_eq!(
            target.get_minutes_markdown(&untouched).unwrap(),
            Some(kept.clone())
        );
        let restored = target.get_job(&new_job).unwrap();
        assert_eq!(restored.completed_units, job.completed_units);
        assert_eq!(restored.total_units, job.total_units);
        let (spec, new_cp) = target.fixed_job_input(&new_job).unwrap();
        let new_cp = new_cp.unwrap();
        assert_eq!(new_cp["markdownHash"], cp["markdownHash"]);
        assert_eq!(
            new_cp["outputs"].as_array().unwrap().len(),
            cp["outputs"].as_array().unwrap().len()
        );
        assert!(spec.parameters["generation"]["engine"]["model"]["path"]
            .as_str()
            .unwrap()
            .starts_with("mon-unavailable:"));
        if partial {
            assert_eq!(restored.state, "interrupted");
            assert_eq!(new_cp["pending"]["batches"].as_array().unwrap().len(), 1);
            assert_eq!(
                restored.retry_of_job_id.as_deref(),
                Some(
                    ids.get(
                        "processing_jobs",
                        &meeting,
                        job.retry_of_job_id.as_deref().unwrap()
                    )
                    .unwrap()
                    .as_str()
                )
            );
            assert!(target.acquire_local_inference_job().unwrap().is_none());
        } else {
            assert_eq!(restored.state, "succeeded");
            assert_eq!(
                target
                    .notification_output(&new_job)
                    .unwrap()
                    .publication_state,
                "superseded"
            );
        }
        assert_eq!(synthetic::calls(root.path()), before);
        assert_eq!(
            storage.get_minutes_markdown(&meeting).unwrap(),
            Some(original)
        );
        assert_eq!(storage.fixed_job_input(&job.id).unwrap().1, Some(cp));
        assert_eq!(storage.get_job(&job.id).unwrap(), job);
        for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
            assert_eq!(
                target
                    .connection
                    .lock()
                    .unwrap()
                    .query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get(0))
                    .unwrap(),
                0
            );
        }
        drop(target);
        let target = Storage::open(target_root.path().into()).unwrap();
        assert_eq!(
            target.get_minutes_markdown(&new_meeting).unwrap(),
            Some(doc.clone())
        );
        assert_eq!(target.get_minutes_markdown(&untouched).unwrap(), Some(kept));
        let path = output.path().join("regeneration.monbackup");
        target.create_backup_archive(&path, &check).unwrap();
        let second = archive::read(&mut File::open(path).unwrap(), output.path(), &check).unwrap();
        let second_ids =
            RestoreIds::prepare(&second, output.path(), &Uuid::new_v4().to_string(), &check)
                .unwrap();
        let second_fixed = FixedData::new(&second_ids, output.path(), &check).unwrap();
        let next = SealedWorkspace::prepare(&second_fixed, output.path(), &check).unwrap();
        assert_eq!(
            next.connection()
                .query_row::<String, _, _>(
                    "SELECT markdown FROM minutes_markdown WHERE meeting_id=?1",
                    [second_ids.get("meetings", "", &new_meeting).unwrap()],
                    |r| r.get(0)
                )
                .unwrap(),
            doc.markdown
        );
        assert_eq!(synthetic::calls(root.path()), before);
    }
}

#[test]
fn markdown_generation_queued_archive_restores_without_a_fake_result_or_inference() {
    let check: fn() -> bool = || false;
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "queued-markdown");
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let req = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let job = crate::generation::markdown_job::enqueue_synthetic(
        &storage,
        req,
        synthetic::engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("queued.monbackup");
    storage.create_backup_archive(&path, &check).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &check).unwrap();
    let ids =
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &check).unwrap();
    let fixed = FixedData::new(&ids, output.path(), &check).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &check).unwrap();
    let db = sealed.connection();
    let new_job = ids.get("processing_jobs", &meeting, &job.id).unwrap();
    assert_eq!(
        crate::storage::jobs::load_job(db, &new_job)
            .unwrap()
            .unwrap()
            .state,
        "interrupted"
    );
    assert!(crate::storage::jobs::read_fixed_input(db, &new_job)
        .unwrap()
        .1
        .is_none());
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM minutes_markdown", [], |r| r.get(0))
            .unwrap(),
        0
    );
    assert_eq!(synthetic::calls(root.path()), 0);
}
