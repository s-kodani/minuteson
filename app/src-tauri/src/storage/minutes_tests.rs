use super::super::minutes_input::MinutesJobRequest;
use super::*;
use crate::minutes_document::{from_generated, GeneratedItem, ItemKind, SourceType, VerificationState};
use serde_json::json;

pub(in crate::storage) fn fixture(
    storage: &Storage,
    units: i64,
) -> (MinutesJobRequest, JobSnapshot, MinutesContent) {
    let meeting = super::super::analysis::tests::saved_meeting(storage);
    let mut source = super::super::analysis::tests::input();
    source.utterances[0].text = "点検日は2026-10-09。担当者は未確認です。".into();
    let analysis = storage
        .create_analysis_candidate(&meeting.id, source)
        .unwrap();
    let committed = storage
        .commit_analysis_candidate(&meeting.id, &analysis, 0, false)
        .unwrap();
    let draft = storage
        .create_transcript_draft(&meeting.id, &analysis)
        .unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.id,
        operation_id: "minutes-operation".into(),
        expected_analysis_id: analysis,
        expected_content_hash: committed.content_hash,
        expected_meeting_lock_version: committed.meeting_lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let job = enqueue(storage, request.clone(), units);
    assert_eq!(
        storage
            .acquire_job_kind(Some(JobKind::MinutesGeneration))
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    storage
        .save_job_checkpoint(
            &job.id,
            0,
            1,
            json!({"stage":"protocol-fixture","result":"parsed"}),
        )
        .unwrap();
    let content = make_content(storage, &job.id);
    (request, job, content)
}
fn enqueue(storage: &Storage, request: MinutesJobRequest, units: i64) -> JobSnapshot {
    storage
        .enqueue_minutes_job(
            request,
            json!({"engine":"protocol-fixture","modelHash":"c".repeat(64)}),
            json!({"templateVersion":TEMPLATE_VERSION}),
            Some(units),
        )
        .unwrap()
}
fn make_content(storage: &Storage, job: &str) -> MinutesContent {
    let input = storage.load_minutes_input(job).unwrap();
    let raw = [ItemKind::Decision, ItemKind::Todo]
        .into_iter()
        .map(|kind| GeneratedItem {
            kind,
            body: "点検日を確認する".into(),
            assignee_name: None,
            assignee_speaker_id: None,
            due_date: None,
            due_text: None,
            evidence_utterance_ids: vec![input.analysis.utterances[0].id.clone()],
        })
        .collect();
    from_generated(&input, raw).unwrap()
}

#[test]
fn minutes_result_and_job_success_are_atomic_without_adopting_or_replacing_edits() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, job, content) = fixture(&storage, 1);
    let before = storage.get_meeting(&request.meeting_id).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&request.meeting_id).unwrap();
    let source = storage.load_current_analysis(&request.meeting_id).unwrap();
    let id = storage
        .seed_stored_candidate_for_job(&job.id, 1, content.clone())
        .unwrap();
    let snapshot = storage.get_job(&job.id).unwrap();
    assert_eq!(snapshot.state, "succeeded");
    assert_eq!(snapshot.output_minutes_id.as_deref(), Some(id.as_str()));
    assert_eq!(snapshot.output_analysis_id, None);
    let notification = storage.notification_output(&job.id).unwrap();
    assert_eq!(notification.job.id, job.id);
    assert_eq!(notification.publication_state, "candidate");
    let view = storage
        .load_minutes_content(&request.meeting_id, &id)
        .unwrap()
        .unwrap();
    assert_eq!(view.content, content);
    assert_eq!(view.generated_content, content);
    assert_eq!(notification.content_hash, view.content_hash);
    assert_eq!(
        storage.get_meeting(&request.meeting_id).unwrap().unwrap(),
        before
    );
    assert_eq!(
        storage.load_transcript_draft(&request.meeting_id).unwrap(),
        draft
    );
    assert_eq!(
        storage.load_current_analysis(&request.meeting_id).unwrap(),
        source
    );
    assert!(storage
        .load_minutes_content("other-meeting", &id)
        .unwrap()
        .is_none());
    assert!(storage
        .seed_stored_candidate_for_job(&job.id, 1, content)
        .is_err());
    let lock = storage
        .get_meeting(&request.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    storage
        .seed_stored_minutes_head(&request.meeting_id, &id)
        .unwrap();
    assert!(storage.verify_minutes_content_hash(&id).unwrap());
    assert_eq!(
        storage
            .notification_output(&job.id)
            .unwrap()
            .publication_state,
        "committed"
    );
    let markdown = view
        .content
        .markdown(&storage.load_minutes_input(&job.id).unwrap())
        .unwrap();
    assert!(markdown.contains("未確認"));
    assert!(markdown.contains(&view.input_analysis_id.replace('-', "\\-")));
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(
        storage
            .load_minutes_content(&request.meeting_id, &id)
            .unwrap()
            .unwrap(),
        view
    );
    assert_eq!(storage.get_job(&job.id).unwrap(), snapshot);
}

#[test]
fn minutes_finalize_rolls_back_everything_when_success_transition_fails() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, job, content) = fixture(&storage, 1);
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER simulate_minutes_success_failure BEFORE UPDATE OF output_minutes_id ON processing_jobs WHEN NEW.output_minutes_id IS NOT NULL BEGIN SELECT RAISE(ABORT, 'simulated success failure'); END;").unwrap();
    assert!(storage
        .seed_stored_candidate_for_job(&job.id, 1, content.clone())
        .is_err());
    let db = storage.connection.lock().unwrap();
    for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    db.execute_batch("DROP TRIGGER simulate_minutes_success_failure;")
        .unwrap();
    drop(db);
    let current = storage.get_job(&job.id).unwrap();
    assert_eq!(current.state, "running");
    assert_eq!(current.completed_units, 1);
    assert_eq!(current.output_minutes_id, None);
    assert!(
        storage
            .get_meeting(&request.meeting_id)
            .unwrap()
            .unwrap()
            .lock_version
            > 0
    );
    assert!(storage
        .seed_stored_candidate_for_job(&job.id, 1, content)
        .is_ok());
}

#[test]
fn incomplete_invalid_and_tampered_results_leave_the_job_running_without_a_candidate() {
    for change in [
        "incomplete",
        "content",
        "evidence",
        "reviewed",
        "manual",
        "checkpoint",
        "duplicate_ids",
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (_, job, mut content) = fixture(&storage, if change == "incomplete" { 2 } else { 1 });
        match change {
            "content" => content.items[0].body = "文書にない本文".into(),
            "evidence" => content.evidence[0].utterance_id = "missing".into(),
            "reviewed" => content.evidence[0].verification_state = VerificationState::UserVerified,
            "manual" => {
                content.items[0].source_type = SourceType::Manual;
                content.evidence[0].source_type = SourceType::Manual;
            }
            "checkpoint" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE job_checkpoints SET checkpoint_json='{}' WHERE job_id=?1",
                        params![job.id],
                    )
                    .unwrap();
            }
            "duplicate_ids" => content.items.push(content.items[0].clone()),
            "incomplete" => {}
            _ => unreachable!(),
        }
        assert!(
            storage
                .seed_stored_candidate_for_job(&job.id, 1, content)
                .is_err(),
            "{change}"
        );
        assert_eq!(storage.get_job(&job.id).unwrap().state, "running");
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM minutes_revisions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn regenerated_candidate_keeps_current_minutes_draft_and_fixed_evidence() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (mut request, job, content) = fixture(&storage, 1);
    let first = storage
        .seed_stored_candidate_for_job(&job.id, 1, content)
        .unwrap();
    let lock = storage
        .get_meeting(&request.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    storage
        .seed_stored_minutes_head(&request.meeting_id, &first)
        .unwrap();
    let original = storage
        .load_minutes_content(&request.meeting_id, &first)
        .unwrap()
        .unwrap();
    storage.connection.lock().unwrap().execute("INSERT INTO working_drafts (id,meeting_id,target_kind,base_minutes_id,payload_json,schema_version,content_hash,lock_version,created_at,updated_at) VALUES ('minutes-draft',?1,'minutes',?2,?3,1,?4,0,0,0)",params![request.meeting_id,first,serde_json::to_string(&original.content).unwrap(),"d".repeat(64)]).unwrap();
    request.operation_id = "regenerate".into();
    request.expected_meeting_lock_version = storage
        .get_meeting(&request.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    let second_job = enqueue(&storage, request.clone(), 1);
    storage
        .acquire_job_kind(Some(JobKind::MinutesGeneration))
        .unwrap()
        .unwrap();
    storage
        .save_job_checkpoint(&second_job.id, 0, 1, json!({"result":"parsed"}))
        .unwrap();
    let second = storage
        .seed_stored_candidate_for_job(&second_job.id, 1, make_content(&storage, &second_job.id))
        .unwrap();
    assert_eq!(
        storage
            .load_minutes_content(&request.meeting_id, &first)
            .unwrap()
            .unwrap(),
        original
    );
    let db = storage.connection.lock().unwrap();
    let (head,parent,draft):(String,String,String)=db.query_row("SELECT m.current_minutes_id,r.parent_revision_id,d.payload_json FROM meetings m JOIN minutes_revisions r ON r.meeting_id=m.id AND r.id=?1 JOIN working_drafts d ON d.meeting_id=m.id AND d.target_kind='minutes' WHERE m.id=?2",params![second,request.meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(head, first);
    assert_eq!(parent, first);
    assert_eq!(draft, serde_json::to_string(&original.content).unwrap());
    drop(db);
    assert!(storage.verify_minutes_content_hash(&first).unwrap());
    assert!(storage
        .load_minutes_content(&request.meeting_id, &second)
        .unwrap()
        .is_some());
}

#[test]
fn sql_minutes_result_pointer_cannot_cross_scope_or_change_after_success() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, job, content) = fixture(&storage, 1);
    let id = storage
        .seed_stored_candidate_for_job(&job.id, 1, content)
        .unwrap();
    let db = storage.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE processing_jobs SET output_minutes_id=NULL WHERE id=?1",
            params![job.id]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE processing_jobs SET state='failed' WHERE id=?1",
            params![job.id]
        )
        .is_err());
    drop(db);
    let mut spec = storage.fixed_job_input(&job.id).unwrap().0;
    spec.operation_id = "different-operation".into();
    let second = storage.enqueue_job(spec).unwrap();
    let db = storage.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE processing_jobs SET output_minutes_id=?1 WHERE id=?2",
            params![id, second.id]
        )
        .is_err());
    assert!(db.execute("INSERT INTO processing_jobs (id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,output_minutes_id,created_at,updated_at) VALUES ('bad',?1,'transcription','succeeded','bad',?2,'{}','{}',?2,?3,0,0)",params![request.meeting_id,"a".repeat(64),id]).is_err());
}

#[test]
fn read_checks_both_content_hash_and_canonical_projections() {
    for rehash in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (request, job, content) = fixture(&storage, 1);
        let id = storage
            .seed_stored_candidate_for_job(&job.id, 1, content)
            .unwrap();
        let mut db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE minutes_items SET body='変えた項目' WHERE minutes_revision_id=?1",
            params![id],
        )
        .unwrap();
        if rehash {
            let tx = db.transaction().unwrap();
            let hash = super::super::revisions::minutes_content_hash(&tx, &id).unwrap();
            tx.execute(
                "UPDATE minutes_revisions SET content_hash=?1 WHERE id=?2",
                params![hash, id],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        drop(db);
        assert!(storage
            .load_minutes_content(&request.meeting_id, &id)
            .is_err());
    }
}
