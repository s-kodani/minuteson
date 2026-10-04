use super::*;
use crate::generation;
use crate::generation::llama::LlamaConfig;
use crate::local_engine::Fingerprint;
use std::os::unix::fs::PermissionsExt;

fn engine(root: &Path, body: &str) -> LlamaConfig {
    let path = root.join("llama-completion");
    let model = root.join("model.gguf");
    let script=format!("#!/usr/bin/python3\nimport sys,json,pathlib,os\nargs=sys.argv[1:]\ndef arg(name): return args[args.index(name)+1]\nbase=pathlib.Path(arg('--model')).parent\nwith (base/'engine.calls').open('a') as f: f.write('call\\n')\n{body}\n");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&model, b"fixture GGUF").unwrap();
    LlamaConfig {
        executable: Fingerprint::capture(&path, &|| false).unwrap(),
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        threads: 1,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    }
}
fn normal_engine(root: &Path) -> LlamaConfig {
    engine(
        root,
        r#"p=json.loads(pathlib.Path(arg('--file')).read_text())
if 'utterances' in p:
    refs=[u['utteranceId'] for u in p['utterances']]
    items=[{'kind':'summary','body':'点検日を確認する','assigneeName':None,'assigneeSpeakerId':None,'dueDate':None,'dueText':None,'evidenceUtteranceIds':refs}] if refs else []
else:
    items=[{'body':'点検日を確認する','sourceItemIds':[i['sourceItemId'] for i in p['items']]}] if p['items'] else []
print(json.dumps({'schemaVersion':1,'items':items},ensure_ascii=False))"#,
    )
}
fn request(storage: &Storage, long: bool) -> MinutesJobRequest {
    let meeting = analysis::tests::saved_meeting(storage);
    std::fs::create_dir_all(storage.root.join("meetings").join(&meeting.id)).unwrap();
    let mut source = analysis::tests::input();
    source.utterances[0].text = if long {
        "点検日を確認することに決まりました。".repeat(45)
    } else {
        "点検日を確認することに決まりました。".into()
    };
    let id = storage
        .create_analysis_candidate(&meeting.id, source)
        .unwrap();
    let current = storage
        .commit_analysis_candidate(&meeting.id, &id, 0, false)
        .unwrap();
    let draft = storage.create_transcript_draft(&meeting.id, &id).unwrap();
    MinutesJobRequest {
        meeting_id: meeting.id,
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: id,
        expected_content_hash: current.content_hash,
        expected_meeting_lock_version: current.meeting_lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    }
}
fn calls(root: &Path) -> usize {
    std::fs::read_to_string(root.join("engine.calls"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}
fn current_minutes(storage: &Storage, meeting: &str) -> Option<String> {
    storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT current_minutes_id FROM meetings WHERE id=?1",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap()
}

fn retry_request(job: &JobSnapshot, operation: &str) -> generation::RetryRequest {
    generation::RetryRequest {
        meeting_id: job.meeting_id.clone(),
        job_id: job.id.clone(),
        operation_id: operation.into(),
        expected_spec_hash: job.spec_hash.clone(),
    }
}
fn pause_at(storage: &Storage, job: &JobSnapshot, units: i64) {
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    assert!(generation::process(storage, &job.id, &|| false, &|s| {
        if s.completed_units == units && s.state == "running" {
            storage.request_job_pause(&job.id, "user").unwrap();
        }
    })
    .unwrap()
    .is_none());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
}

#[test]
fn minutes_retry_chain_preserves_fixed_inputs_edits_and_durable_units_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let engine = normal_engine(root.path());
    let mut request = request(&storage, true);
    let baseline = minutes_ui::tests::candidate(&storage, &request.meeting_id);
    storage
        .adopt_minutes(minutes_ui::tests::request(
            &storage,
            &request.meeting_id,
            &baseline,
        ))
        .unwrap();
    let draft = storage
        .create_minutes_draft(&request.meeting_id, &baseline)
        .unwrap();
    let mut content = draft.payload;
    content
        .blocks
        .push(crate::minutes_document::Block::Paragraph {
            item_id: None,
            content: vec![crate::minutes_document::Inline::text("保持する手動編集")],
        });
    let minutes_edit = storage
        .save_minutes_draft(&draft.id, draft.lock_version, content)
        .unwrap();
    request.expected_meeting_lock_version = storage
        .get_meeting(&request.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    let original =
        generation::enqueue(&storage, request.clone(), engine, "standard", 2400).unwrap();
    let fixed_input = storage.load_minutes_input(&original.id).unwrap();
    pause_at(&storage, &original, 1);
    let source = storage
        .finish_job(&original.id, 1, "failed", Some("fixture_failure"))
        .unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&source.id).unwrap();
    let mut transcript = storage
        .load_transcript_draft(&request.meeting_id)
        .unwrap()
        .unwrap();
    transcript.payload.utterances.push(drafts::UtteranceEdit {
        utterance_id: storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap()
            .utterances[0]
            .id
            .clone(),
        effective_text: "unused".into(),
        speaker_ids: vec![],
    });
    let transcript_edit = storage
        .save_transcript_draft(&transcript.id, transcript.lock_version, transcript.payload)
        .unwrap();
    let child = retry_request(&source, "retry-2").enqueue(&storage).unwrap();
    assert_eq!(child.completed_units, 1);
    assert_eq!(child.attempt, 2);
    assert_eq!(child.retry_of_job_id.as_deref(), Some(source.id.as_str()));
    assert_ne!(child.spec_hash, source.spec_hash);
    let (child_spec, inherited) = storage.fixed_job_input(&child.id).unwrap();
    assert_eq!(child_spec.parameters, spec.parameters);
    assert_eq!(child_spec.profile_snapshot, spec.profile_snapshot);
    assert_eq!(child_spec.input_analysis_id, spec.input_analysis_id);
    assert_eq!(inherited, checkpoint);
    assert_eq!(storage.load_minutes_input(&child.id).unwrap(), fixed_input);
    assert_eq!(
        storage.fixed_job_initial_operation(&child.id).unwrap(),
        request.operation_id
    );
    assert_eq!(
        retry_request(&source, "retry-2").enqueue(&storage).unwrap(),
        child
    );
    assert!(retry_request(&source, "competing-retry")
        .enqueue(&storage)
        .is_err());
    assert_eq!(calls(root.path()), 1);
    pause_at(&storage, &child, 2);
    let child = storage
        .finish_job(&child.id, 2, "failed", Some("fixture_failure"))
        .unwrap();
    let third = retry_request(&child, "retry-3").enqueue(&storage).unwrap();
    assert_eq!(third.attempt, 3);
    assert_eq!(third.completed_units, 2);
    assert_eq!(
        storage.fixed_job_initial_operation(&third.id).unwrap(),
        request.operation_id
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        third.id
    );
    let result = generation::process(&storage, &third.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(storage.get_job(&third.id).unwrap().state, "succeeded");
    assert_eq!(calls(root.path()) as i64, original.total_units.unwrap());
    assert_eq!(storage.get_job(&source.id).unwrap(), source);
    assert_eq!(storage.get_job(&child.id).unwrap(), child);
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        transcript_edit
    );
    assert_eq!(
        storage
            .load_minutes_draft(&request.meeting_id, Some(&baseline))
            .unwrap()
            .unwrap(),
        minutes_edit
    );
    assert_eq!(
        current_minutes(&storage, &request.meeting_id),
        Some(baseline)
    );
    assert!(storage
        .minutes_versions(&request.meeting_id)
        .unwrap()
        .versions
        .iter()
        .any(|v| v.id == result && v.publication_state == "candidate" && v.input_updated));
}

#[test]
fn cancelled_fully_checkpointed_minutes_retry_does_not_rerun_engine() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let job = generation::enqueue(
        &storage,
        request(&storage, false),
        normal_engine(root.path()),
        "standard",
        2400,
    )
    .unwrap();
    pause_at(&storage, &job, job.total_units.unwrap());
    let source = storage
        .finish_job(&job.id, job.total_units.unwrap(), "cancelled", None)
        .unwrap();
    let count = calls(root.path());
    let next = retry_request(&source, "retry-complete")
        .enqueue(&storage)
        .unwrap();
    storage.acquire_local_inference_job().unwrap();
    assert!(generation::process(&storage, &next.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    assert_eq!(calls(root.path()), count);
    assert!(current_minutes(&storage, &job.meeting_id).is_none());
}

#[test]
fn minutes_retry_rejects_changed_models_artifacts_and_stale_identity_without_writes() {
    for change in [
        "model",
        "executable",
        "normalized",
        "result",
        "prompt",
        "meeting",
        "spec",
        "state",
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let job = generation::enqueue(
            &storage,
            request(&storage, true),
            normal_engine(root.path()),
            "standard",
            2400,
        )
        .unwrap();
        pause_at(&storage, &job, 1);
        let source = if change == "state" {
            storage.get_job(&job.id).unwrap()
        } else {
            storage
                .finish_job(&job.id, 1, "failed", Some("fixture_failure"))
                .unwrap()
        };
        let mut retry = retry_request(&source, "rejected-retry");
        match change {
            "model" => std::fs::write(root.path().join("model.gguf"), b"changed").unwrap(),
            "executable" => {
                std::fs::write(root.path().join("llama-completion"), b"changed").unwrap()
            }
            "meeting" => retry.meeting_id = storage.create_meeting("wrong meeting").unwrap().id,
            "spec" => retry.expected_spec_hash = "changed".into(),
            "state" => {}
            _ => {
                let (_, cp) = storage.fixed_job_input(&job.id).unwrap();
                let cp = cp.unwrap();
                let id = match change {
                    "normalized" => &cp["outputs"][0]["normalized"]["id"],
                    "result" => &cp["outputs"][0]["artifacts"][3]["id"],
                    _ => &cp["outputs"][0]["artifacts"][1]["id"],
                }
                .as_str()
                .unwrap();
                let path: String = storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT relative_path FROM file_assets WHERE id=?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .unwrap();
                std::fs::write(root.path().join(path), b"[]").unwrap();
            }
        }
        let jobs = storage.list_jobs(&job.meeting_id).unwrap();
        assert!(retry.enqueue(&storage).is_err(), "{change}");
        assert_eq!(storage.list_jobs(&job.meeting_id).unwrap(), jobs);
        assert_eq!(calls(root.path()), 1);
        assert!(current_minutes(&storage, &job.meeting_id).is_none());
    }
}

#[test]
fn minutes_retry_checkpoint_copy_failure_is_atomic() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let job = generation::enqueue(
        &storage,
        request(&storage, true),
        normal_engine(root.path()),
        "standard",
        2400,
    )
    .unwrap();
    pause_at(&storage, &job, 1);
    let source = storage
        .finish_job(&job.id, 1, "failed", Some("fixture_failure"))
        .unwrap();
    let jobs = storage.list_jobs(&job.meeting_id).unwrap();
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER fixture_retry_abort BEFORE INSERT ON job_checkpoints BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;").unwrap();
    assert!(retry_request(&source, "atomic-retry")
        .enqueue(&storage)
        .is_err());
    assert_eq!(storage.list_jobs(&job.meeting_id).unwrap(), jobs);
    assert_eq!(calls(root.path()), 1);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fixture_retry_abort;")
        .unwrap();
    assert_eq!(
        retry_request(&source, "atomic-retry")
            .enqueue(&storage)
            .unwrap()
            .completed_units,
        1
    );
}

#[test]
fn generation_reopens_and_resumes_only_durable_units_and_preserves_source_edits() {
    let root = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(root.path().into()).unwrap();
    let engine = normal_engine(root.path());
    let request = request(&storage, true);
    let job =
        generation::enqueue(&storage, request.clone(), engine.clone(), "standard", 2400).unwrap();
    assert!(job.total_units.unwrap() > 3);
    let fixed = storage.load_minutes_input(&job.id).unwrap();
    assert_eq!(
        generation::enqueue(&storage, request.clone(), engine.clone(), "standard", 2400)
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    assert!(
        generation::process(&storage, &job.id, &|| false, &|snapshot| {
            if snapshot.completed_units == 1 && snapshot.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap()
        .is_none()
    );
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(calls(root.path()), 1);
    let mut draft = storage
        .load_transcript_draft(&request.meeting_id)
        .unwrap()
        .unwrap();
    draft.payload.utterances.push(drafts::UtteranceEdit {
        utterance_id: storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap()
            .utterances[0]
            .id
            .clone(),
        effective_text: "生成開始後の修正".into(),
        speaker_ids: vec![],
    });
    let edited = storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    assert_eq!(
        generation::enqueue(&storage, request.clone(), engine.clone(), "standard", 2400)
            .unwrap()
            .id,
        job.id
    );
    storage.resume_job(&job.id).unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    drop(storage);
    storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "interrupted");
    storage.resume_job(&job.id).unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    let id = generation::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let final_job = storage.get_job(&job.id).unwrap();
    assert_eq!(final_job.state, "succeeded");
    assert_eq!(final_job.completed_units, job.total_units.unwrap());
    assert_eq!(final_job.output_minutes_id, Some(id.clone()));
    assert_eq!(calls(root.path()) as i64, job.total_units.unwrap());
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        edited
    );
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .map(|a| a.id),
        Some(request.expected_analysis_id.clone())
    );
    let view = storage
        .load_minutes_content(&request.meeting_id, &id)
        .unwrap()
        .unwrap();
    assert_eq!(view.content.items.len(), 1);
    assert!(view
        .content
        .evidence
        .iter()
        .all(|e| e.analysis_revision_id == fixed.analysis.id));
    assert!(view
        .content
        .markdown(&fixed)
        .unwrap()
        .contains("点検日を確認する"));
    assert!(current_minutes(&storage, &request.meeting_id).is_none());
}

#[test]
fn generated_artifact_or_model_changes_block_resume_without_running_more_units() {
    for change in ["artifact", "model"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let engine = normal_engine(root.path());
        let request = request(&storage, true);
        let job = generation::enqueue(&storage, request.clone(), engine, "standard", 2400).unwrap();
        storage.acquire_local_inference_job().unwrap();
        generation::process(&storage, &job.id, &|| false, &|s| {
            if s.completed_units == 1 && s.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap();
        if change == "model" {
            std::fs::write(root.path().join("model.gguf"), b"changed").unwrap();
        } else {
            let (_, checkpoint) = storage.fixed_job_input(&job.id).unwrap();
            let cp = checkpoint.unwrap();
            let id = cp["outputs"][0]["normalized"]["id"].as_str().unwrap();
            let path: String = storage
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT relative_path FROM file_assets WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap();
            std::fs::write(root.path().join(path), b"[]").unwrap();
        }
        storage.resume_job(&job.id).unwrap();
        storage.acquire_local_inference_job().unwrap();
        assert!(generation::process(&storage, &job.id, &|| false, &|_| {}).is_err());
        assert_eq!(calls(root.path()), 1);
        assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 1);
        assert!(current_minutes(&storage, &request.meeting_id).is_none());
    }
}

#[test]
fn recording_pause_and_exit_kill_unfinished_generation_without_advancing_checkpoint() {
    for recording in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let engine = engine(
            root.path(),
            "(base/'engine.pid').write_text(str(os.getpid()))\nwhile True: pass",
        );
        let request = request(&storage, false);
        let job = generation::enqueue(&storage, request, engine, "standard", 2400).unwrap();
        storage.acquire_local_inference_job().unwrap();
        let stopping = || {
            if root.path().join("engine.pid").exists() {
                if recording {
                    storage.request_pause_for_recording().unwrap();
                    false
                } else {
                    true
                }
            } else {
                false
            }
        };
        assert!(generation::process(&storage, &job.id, &stopping, &|_| {})
            .unwrap()
            .is_none());
        let current = storage.get_job(&job.id).unwrap();
        assert_eq!(current.state, "paused");
        assert_eq!(current.completed_units, 0);
        assert_eq!(
            current.pause_reason.as_deref(),
            Some(if recording { "recording" } else { "exit" })
        );
        let pid = std::fs::read_to_string(root.path().join("engine.pid")).unwrap();
        assert!(!std::process::Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
    }
}

#[test]
fn bad_generation_result_and_bad_plan_builder_do_not_create_a_candidate_or_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let engine = engine(root.path(), "print('{')");
    let request = request(&storage, false);
    let count = || {
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = count();
    assert!(
        generation::enqueue(&storage, request.clone(), engine.clone(), "standard", 1000).is_err()
    );
    assert_eq!(count(), before);
    let job = generation::enqueue(&storage, request.clone(), engine, "standard", 2400).unwrap();
    storage.acquire_local_inference_job().unwrap();
    assert!(generation::process(&storage, &job.id, &|| false, &|_| {}).is_err());
    assert_eq!(calls(root.path()), 1);
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 0);
    assert!(current_minutes(&storage, &request.meeting_id).is_none());
    let candidates: i64 = storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM minutes_revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(candidates, 0);
}

#[test]
fn blank_generation_is_local_and_scheduler_skips_unsupported_engines() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let engine = normal_engine(root.path());
    let mut request = request(&storage, false);
    let mut draft = storage
        .load_transcript_draft(&request.meeting_id)
        .unwrap()
        .unwrap();
    draft.payload.utterances.push(drafts::UtteranceEdit {
        utterance_id: storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap()
            .utterances[0]
            .id
            .clone(),
        effective_text: " ".into(),
        speaker_ids: vec![],
    });
    let edited = storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    request.expected_draft = Some(DraftStamp {
        id: edited.id,
        lock_version: edited.lock_version,
    });
    let unsupported = storage
        .enqueue_job(JobSpec {
            meeting_id: request.meeting_id.clone(),
            kind: JobKind::Diarization,
            operation_id: Uuid::new_v4().to_string(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({}),
            parameters: serde_json::json!({}),
            total_units: Some(1),
        })
        .unwrap();
    let job = generation::enqueue(&storage, request.clone(), engine, "standard", 2400).unwrap();
    assert_eq!(job.total_units, Some(1));
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    let id = generation::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(calls(root.path()), 0);
    assert!(storage
        .load_minutes_content(&request.meeting_id, &id)
        .unwrap()
        .unwrap()
        .content
        .items
        .is_empty());
    assert_eq!(storage.get_job(&unsupported.id).unwrap().state, "queued");
    assert!(storage.acquire_local_inference_job().unwrap().is_none());
}

#[test]
#[ignore = "requires MON_LLAMA_COMPLETION and MON_LLAMA_MODEL"]
fn real_llama_worker_saves_candidate_and_reuses_completed_results() {
    let root = tempfile::tempdir().unwrap().keep();
    let storage = Storage::open(root.clone()).unwrap();
    let request = request(&storage, false);
    let engine = LlamaConfig {
        executable: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_COMPLETION").unwrap()),
            &|| false,
        )
        .unwrap(),
        model: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_MODEL").unwrap()),
            &|| false,
        )
        .unwrap(),
        threads: 4,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    };
    let job =
        generation::enqueue(&storage, request.clone(), engine.clone(), "standard", 5400).unwrap();
    assert_eq!(job.total_units, Some(1));
    storage.acquire_local_inference_job().unwrap();
    println!("real llama worker synthetic fixture: {}", root.display());
    generation::process(&storage, &job.id, &|| false, &|s| {
        if s.completed_units == 1 && s.state == "running" {
            storage.request_job_pause(&job.id, "user").unwrap();
        }
    })
    .unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 1);
    let units = root
        .join("meetings")
        .join(&request.meeting_id)
        .join("analysis")
        .join(&job.id);
    assert_eq!(std::fs::read_dir(&units).unwrap().count(), 1);
    storage.resume_job(&job.id).unwrap();
    storage.acquire_local_inference_job().unwrap();
    let id = generation::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read_dir(&units).unwrap().count(), 1);
    let view = storage
        .load_minutes_content(&request.meeting_id, &id)
        .unwrap()
        .unwrap();
    assert!(!view.content.items.is_empty());
    assert_eq!(
        storage.get_job(&job.id).unwrap().output_minutes_id,
        Some(id)
    );
    assert!(current_minutes(&storage, &request.meeting_id).is_none());
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap()
            .id,
        request.expected_analysis_id
    );
    let retry_source = generation::enqueue(
        &storage,
        MinutesJobRequest {
            operation_id: Uuid::new_v4().to_string(),
            ..request.clone()
        },
        engine,
        "standard",
        5400,
    )
    .unwrap();
    pause_at(&storage, &retry_source, 1);
    let cancelled = storage
        .finish_job(&retry_source.id, 1, "cancelled", None)
        .unwrap();
    let checkpoint = storage.fixed_job_input(&cancelled.id).unwrap().1;
    let retry = retry_request(&cancelled, &Uuid::new_v4().to_string())
        .enqueue(&storage)
        .unwrap();
    assert_eq!(storage.fixed_job_input(&retry.id).unwrap().1, checkpoint);
    storage.acquire_local_inference_job().unwrap();
    assert!(generation::process(&storage, &retry.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    let retry_units = root
        .join("meetings")
        .join(&request.meeting_id)
        .join("analysis")
        .join(&retry.id);
    assert!(!retry_units.exists());
    assert_eq!(storage.get_job(&cancelled.id).unwrap(), cancelled);
    assert!(current_minutes(&storage, &request.meeting_id).is_none());
    println!("real llama worker: {} unverified items, durable unit reused without another CLI result directory",view.content.items.len());
}
