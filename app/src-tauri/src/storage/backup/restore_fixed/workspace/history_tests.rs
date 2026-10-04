use super::*;
use crate::storage::analysis_flows::AnalysisFlow;

struct Fixture {
    root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    flow: AnalysisFlow,
}
fn fixture(complete: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting) = crate::storage::analysis_flows::tests::fixture(root.path());
    storage.ensure_analysis_flow(&meeting).unwrap();
    crate::analysis_flow::advance(&storage, &|| false, &|_| {}).unwrap();
    if complete {
        let trans = storage.acquire_local_inference_job().unwrap().unwrap();
        crate::transcription::process(&storage, &trans.id, &|| false, &|_| {})
            .unwrap()
            .unwrap();
        crate::analysis_flow::advance(&storage, &|| false, &|_| {}).unwrap();
        let diar = storage.acquire_local_inference_job().unwrap().unwrap();
        crate::diarization::process(&storage, &diar.id, &|| false, &|_| {})
            .unwrap()
            .unwrap();
        crate::analysis_flow::advance(&storage, &|| false, &|_| {}).unwrap();
    }
    let flow = storage.analysis_flow(&meeting).unwrap().unwrap();
    Fixture {
        root,
        storage,
        meeting,
        flow,
    }
}
fn read(f: &Fixture, output: &Path) -> archive::VerifiedArchive {
    crate::storage::backup::restore_fixed::tests::read(&f.storage, output)
}
fn ids<'a>(source: &'a archive::VerifiedArchive, output: &Path) -> RestoreIds<'a> {
    RestoreIds::prepare(source, output, &Uuid::new_v4().to_string(), &|| false).unwrap()
}
fn prepared(fixed: &FixedData<'_, '_>, output: &Path) -> FixedWorkspace {
    FixedWorkspace::prepare(fixed, output, &|| false)
        .unwrap()
        .with_speech_artifacts(fixed, &|| false)
        .unwrap()
        .with_generation_artifacts(fixed, &|| false)
        .unwrap()
}
fn root_operations(db: &Connection, flow: &AnalysisFlow) {
    for (kind, job) in [
        ("transcription", &flow.transcription_job),
        ("diarization", &flow.diarization_job),
    ] {
        if let Some(job) = job {
            let spec = crate::storage::jobs::read_fixed_input(db, &job.id)
                .unwrap()
                .0;
            let operation = crate::storage::jobs::initial_operation(job.clone(), &spec, |parent| {
                Ok((
                    crate::storage::jobs::load_job(db, parent)?.unwrap(),
                    crate::storage::jobs::read_fixed_input(db, parent)?.0,
                ))
            })
            .unwrap();
            assert_eq!(operation, format!("flow:{}:{kind}", flow.id));
        }
    }
}

#[test]
fn processing_history_restores_completed_flow_operation_plan_hash_and_unadopted_candidate_chain() {
    let f = fixture(true);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    for name in [
        "fake-cli",
        "model.bin",
        "sherpa-cli",
        "seg.onnx",
        "emb.onnx",
    ] {
        std::fs::remove_file(f.root.path().join(name)).unwrap();
    }
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    let stage = work.stage.path().to_path_buf();
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(&work.database, &meeting)
        .unwrap()
        .unwrap();
    assert_eq!(flow.stage, "review");
    assert_ne!(flow.id, f.flow.id);
    assert_eq!(flow.lock_version, f.flow.lock_version);
    assert_eq!(
        flow.plan,
        serde_json::from_str::<Value>(
            &source
                .database
                .query_row(
                    "SELECT plan_json FROM analysis_flows WHERE id=?1",
                    params![f.flow.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap()
        )
        .ok()
    );
    root_operations(&work.database, &flow);
    let candidate = flow
        .diarization_job
        .as_ref()
        .unwrap()
        .output_analysis_id
        .as_ref()
        .unwrap();
    let tx = work.database.transaction().unwrap();
    assert!(crate::storage::analysis_flows::adoption_parent_matches(
        &tx, &meeting, candidate, None
    )
    .unwrap());
    tx.commit().unwrap();
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert!(f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .is_none());
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().plan,
        f.flow.plan
    );
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "review"
    );
    drop(work);
    assert!(!stage.exists());
}

#[test]
fn processing_history_interrupts_queued_retry_and_prepared_flow_without_losing_conditions() {
    let mut f = fixture(false);
    let parent = f.flow.transcription_job.as_ref().unwrap();
    assert_eq!(
        f.storage.acquire_local_inference_job().unwrap().unwrap().id,
        parent.id
    );
    let parent = f
        .storage
        .finish_job(&parent.id, 0, "failed", Some("fixture"))
        .unwrap();
    let child = crate::transcription::RetryRequest {
        meeting_id: f.meeting.clone(),
        job_id: parent.id.clone(),
        operation_id: "flow-retry".into(),
        expected_spec_hash: parent.spec_hash.clone(),
    }
    .enqueue(&f.storage)
    .unwrap();
    f.flow = f
        .storage
        .update_analysis_flow(
            &f.flow,
            "transcription",
            f.flow.plan.clone(),
            Some(&child),
            None,
            None,
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(&work.database, &meeting)
        .unwrap()
        .unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.error_message.as_ref().unwrap().contains("明示的"));
    let rebuilt = flow.transcription_job.as_ref().unwrap();
    assert_eq!(rebuilt.state, "interrupted");
    assert_eq!(
        rebuilt.error_code.as_deref(),
        Some("restored_preparation_required")
    );
    assert_eq!(rebuilt.attempt, child.attempt);
    assert_eq!(
        rebuilt.retry_of_job_id,
        Some(ids.get("processing_jobs", &f.meeting, &parent.id).unwrap())
    );
    root_operations(&work.database, &flow);
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert_eq!(f.storage.get_job(&child.id).unwrap(), child);
    assert_eq!(f.storage.get_job(&parent.id).unwrap(), parent);
}

#[test]
fn processing_history_blocks_pending_flow_with_no_models_or_jobs() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = crate::storage::transcription::tests::fixture(root.path(), 4);
    let old = storage.ensure_analysis_flow(&meeting).unwrap();
    assert_eq!(old.stage, "pending");
    let f = Fixture {
        root,
        storage,
        meeting,
        flow: old,
    };
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    let flow = crate::storage::analysis_flows::read_flow(
        &work.database,
        &ids.get("meetings", "", &f.meeting).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(flow.stage, "configuration");
    assert!(flow.plan.is_none());
    assert!(flow.transcription_job.is_none());
    assert_eq!(
        work.database
            .query_row("SELECT count(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "pending"
    );
}

fn exported_fixture() -> (Fixture, tempfile::TempDir, PathBuf) {
    let f = fixture(true);
    let candidate = f
        .flow
        .diarization_job
        .as_ref()
        .unwrap()
        .output_analysis_id
        .as_ref()
        .unwrap();
    let view = f
        .storage
        .load_visible_analysis(&f.meeting, candidate)
        .unwrap()
        .unwrap();
    let state = f.storage.analysis_versions(&f.meeting).unwrap();
    f.storage
        .adopt_analysis(crate::storage::AdoptRequest {
            meeting_id: f.meeting.clone(),
            revision_id: candidate.clone(),
            expected_content_hash: view.content_hash,
            expected_meeting_lock_version: state.meeting_lock_version,
            expected_draft: None,
            confirm_replacing_previous: false,
        })
        .unwrap();
    let draft = f
        .storage
        .create_transcript_draft(&f.meeting, candidate)
        .unwrap();
    let input = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    let request = crate::storage::MinutesJobRequest {
        meeting_id: f.meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: input.id,
        expected_content_hash: input.content_hash,
        expected_meeting_lock_version: f
            .storage
            .get_meeting(&f.meeting)
            .unwrap()
            .unwrap()
            .lock_version,
        expected_draft: Some(crate::storage::DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let engine = crate::storage::generation_tests::normal_engine(f.root.path());
    let job = crate::generation::enqueue(&f.storage, request, engine, "standard", 2400).unwrap();
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    let revision = crate::generation::process(&f.storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    f.storage
        .adopt_minutes(crate::storage::minutes_ui::tests::request(
            &f.storage, &f.meeting, &revision,
        ))
        .unwrap();
    let draft = f
        .storage
        .create_minutes_draft(&f.meeting, &revision)
        .unwrap();
    let mut payload = draft.payload;
    payload
        .blocks
        .push(crate::minutes_document::Block::Paragraph {
            item_id: None,
            content: vec![crate::minutes_document::Inline::text(
                "出力時だけの手動追記",
            )],
        });
    f.storage
        .save_minutes_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("保存済み議事録.md");
    let export = || {
        let state = f.storage.minutes_versions(&f.meeting).unwrap();
        let content = f
            .storage
            .load_minutes_content(&f.meeting, &revision)
            .unwrap()
            .unwrap();
        f.storage
            .prepare_export(crate::storage::exports::ExportRequest {
                operation_id: Uuid::new_v4().to_string(),
                meeting_id: f.meeting.clone(),
                expected_minutes_id: revision.clone(),
                expected_content_hash: content.content_hash,
                expected_meeting_lock_version: state.meeting_lock_version,
                expected_draft: state.draft,
            })
            .unwrap()
    };
    assert_eq!(
        f.storage.write_export(export(), &path).unwrap().state,
        "succeeded"
    );
    let failure = f
        .storage
        .write_export(export(), &f.root.path().join("blocked.md"))
        .unwrap();
    assert_eq!(failure.state, "failed");
    assert!(!f.root.path().join("blocked.md").exists());
    // External files may be edited or disappear after export. Restore never
    // opens them and must not treat old evidence as new-ID/current-draft bytes.
    std::fs::write(&path, b"external edit").unwrap();
    let draft = f
        .storage
        .load_minutes_draft(&f.meeting, None)
        .unwrap()
        .unwrap();
    let mut payload = draft.payload;
    payload
        .blocks
        .push(crate::minutes_document::Block::Paragraph {
            item_id: None,
            content: vec![crate::minutes_document::Inline::text("出力後の追加編集")],
        });
    f.storage
        .save_minutes_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    (f, outside, path)
}

#[test]
fn processing_history_keeps_external_export_evidence_and_new_references_without_touching_external_files(
) {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let original = f.storage.exports(&f.meeting).unwrap();
    let draft = f.storage.load_minutes_draft(&f.meeting, None).unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    for old in &original {
        let new = ids.get("export_records", &f.meeting, &old.id).unwrap();
        let saved: (String,String,String,i64,String,String,Option<i64>) = work.database.query_row("SELECT meeting_id,exported_content_hash,output_content_hash,output_size_bytes,destination_path,state,exported_at FROM export_records WHERE id=?1",params![new],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
        assert_eq!(
            saved,
            (
                ids.get("meetings", "", &f.meeting).unwrap(),
                old.exported_content_hash.clone(),
                old.output_content_hash.clone(),
                old.output_size_bytes,
                portable::basename(&old.destination_path).unwrap(),
                old.state.clone(),
                old.exported_at
            )
        );
        let minutes: Option<String> = work
            .database
            .query_row(
                "SELECT minutes_revision_id FROM export_records WHERE id=?1",
                params![new],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            minutes,
            old.minutes_revision_id
                .as_deref()
                .map(|id| ids.get("minutes_revisions", &f.meeting, id).unwrap())
        );
        assert_ne!(old.id, new);
    }
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert_eq!(
        f.storage.load_minutes_draft(&f.meeting, None).unwrap(),
        draft
    );
    assert_eq!(f.storage.exports(&f.meeting).unwrap().len(), 2);
}

fn source_writer(source: &archive::VerifiedArchive, output: &Path) -> Connection {
    let path: String = source
        .database
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Path::new(&path).starts_with(output.canonicalize().unwrap()));
    Connection::open(path).unwrap()
}

#[test]
fn processing_history_rejects_altered_target_plan_job_state_and_export_evidence() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    for change in ["plan", "state", "export"] {
        let ids = ids(&source, output.path());
        let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
        let work = prepared(&fixed, output.path());
        let stage = work.stage.path().to_path_buf();
        match change {
            "plan" => {
                work.database.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.threads',2)",[]).unwrap();
            }
            "state" => {
                work.database.execute("UPDATE processing_jobs SET state='interrupted' WHERE kind='minutes_generation'",[]).unwrap();
            }
            "export" => {
                work.database
                    .execute(
                        "UPDATE export_records SET output_content_hash=?1",
                        params!["f".repeat(64)],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(work.with_processing_history(&fixed, &|| false).is_err());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    }
}

#[test]
fn processing_history_rejects_unsupported_or_inconsistent_source_plan_and_root_operations() {
    for change in ["schema", "engine", "model", "root-kind"] {
        let f = fixture(false);
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let writer = source_writer(&source, output.path());
        match change {
            "schema" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.schemaVersion',2)",[]).unwrap();
            }
            "engine" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.threads',2)",[]).unwrap();
            }
            "model" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.model.size',99)",[]).unwrap();
            }
            "root-kind" => {
                writer
                    .execute("UPDATE processing_jobs SET kind='diarization'", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        if change == "root-kind" {
            assert!(RestoreIds::prepare(
                &source,
                output.path(),
                &Uuid::new_v4().to_string(),
                &|| false
            )
            .is_err());
        } else {
            let ids = ids(&source, output.path());
            let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
            let work = prepared(&fixed, output.path());
            let stage = work.stage.path().to_path_buf();
            assert!(work.with_processing_history(&fixed, &|| false).is_err());
            assert!(!stage.exists());
        }
        assert_eq!(
            source
                .database
                .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.storage.analysis_flow(&f.meeting).unwrap().unwrap().plan,
            f.flow.plan
        );
    }
}

#[test]
fn processing_history_cancellation_after_flow_update_cleans_only_the_owned_stage() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let marker = output.path().join("keep.txt");
    std::fs::write(&marker, b"keep").unwrap();
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = prepared(&fixed, output.path());
    let stage = work.stage.path().to_path_buf();
    let image = stage.join("fixed.sqlite3");
    let inspect = Connection::open(&image).unwrap();
    let cancelled = std::cell::Cell::new(false);
    let result = work.with_processing_history(&fixed, &|| {
        let state: String = inspect
            .query_row("SELECT stage FROM analysis_flows", [], |r| r.get(0))
            .unwrap();
        if state == "attention" {
            cancelled.set(true);
        }
        cancelled.get()
    });
    drop(inspect);
    assert!(result.is_err());
    assert!(cancelled.get());
    assert!(!stage.exists());
    assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "transcription"
    );
}

#[test]
fn processing_history_preserves_unlinked_root_job_after_a_lost_handoff_without_auto_start() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    source_writer(&source, output.path())
        .execute("UPDATE analysis_flows SET transcription_job_id=NULL", [])
        .unwrap();
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    let flow = crate::storage::analysis_flows::read_flow(
        &work.database,
        &ids.get("meetings", "", &f.meeting).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.transcription_job.is_none());
    let job = ids
        .get(
            "processing_jobs",
            &f.meeting,
            &f.flow.transcription_job.as_ref().unwrap().id,
        )
        .unwrap();
    let job = crate::storage::jobs::load_job(&work.database, &job)
        .unwrap()
        .unwrap();
    assert_eq!(job.state, "interrupted");
    assert_eq!(job.operation_id, format!("flow:{}:transcription", flow.id));
}

#[test]
fn processing_history_keeps_detached_revision_and_historical_deleted_draft_export_metadata() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let historical = Uuid::new_v4().to_string();
    source_writer(&source,output.path()).execute("UPDATE export_records SET minutes_revision_id=NULL,source_draft_id=?1,source_draft_lock_version=7",params![historical]).unwrap();
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &|| false)
        .unwrap();
    let mapped = ids.get("working_drafts", &f.meeting, &historical).unwrap();
    let count: i64 = work.database.query_row("SELECT count(*) FROM export_records WHERE minutes_revision_id IS NULL AND source_draft_id=?1 AND source_draft_lock_version=7",params![mapped],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        work.database
            .query_row(
                "SELECT count(*) FROM working_drafts WHERE id=?1",
                params![mapped],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
}

#[test]
fn processing_history_rejects_bad_export_hash_path_draft_kind_or_unsettled_state() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let mut counter = 0;
    for change in ["hash", "path", "draft", "writing"] {
        // Each archive owns its source DB, keeping production query_only intact.
        let dir = output.path().join(format!("case-{counter}"));
        counter += 1;
        std::fs::create_dir(&dir).unwrap();
        let source = read(&f, &dir);
        let writer = source_writer(&source, &dir);
        match change {
            "hash" => {
                writer
                    .execute(
                        "UPDATE export_records SET request_hash=?1",
                        params!["z".repeat(64)],
                    )
                    .unwrap();
            }
            "path" => {
                writer
                    .execute(
                        "UPDATE export_records SET destination_path='/Users/source/other.md'",
                        [],
                    )
                    .unwrap();
            }
            "draft" => {
                writer.execute("UPDATE export_records SET source_draft_id=(SELECT id FROM working_drafts WHERE target_kind='transcript' LIMIT 1)",[]).unwrap();
            }
            "writing" => {
                writer
                    .execute(
                        "UPDATE export_records SET state='writing',exported_at=NULL",
                        [],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let ids = ids(&source, &dir);
        let fixed = FixedData::new(&ids, &dir, &|| false).unwrap();
        let work = prepared(&fixed, &dir);
        let stage = work.stage.path().to_path_buf();
        let error = work
            .with_processing_history(&fixed, &|| false)
            .err()
            .unwrap();
        assert!(error.contains("書き出し履歴"), "{change}: {error}");
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
        assert_eq!(
            source
                .database
                .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn processing_history_interrupts_each_runnable_or_interrupted_state_without_changing_fixed_input() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    for state in ["queued", "running", "pausing", "paused", "interrupted"] {
        let dir = output.path().join(state);
        std::fs::create_dir(&dir).unwrap();
        let source = read(&f, &dir);
        source_writer(&source, &dir)
            .execute(
                "UPDATE processing_jobs SET state=?1,pause_reason='user'",
                params![state],
            )
            .unwrap();
        let ids = ids(&source, &dir);
        let fixed = FixedData::new(&ids, &dir, &|| false).unwrap();
        let work = prepared(&fixed, &dir);
        let job = ids
            .get(
                "processing_jobs",
                &f.meeting,
                &f.flow.transcription_job.as_ref().unwrap().id,
            )
            .unwrap();
        let before = crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
        let work = work.with_processing_history(&fixed, &|| false).unwrap();
        let after = crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
        assert_eq!(before, after);
        let restored = crate::storage::jobs::load_job(&work.database, &job)
            .unwrap()
            .unwrap();
        assert_eq!(restored.state, "interrupted");
        assert!(restored.pause_reason.is_none());
        assert_eq!(restored.completed_units, 0);
        assert_eq!(
            f.storage
                .get_job(&f.flow.transcription_job.as_ref().unwrap().id)
                .unwrap()
                .state,
            "queued"
        );
    }
}
