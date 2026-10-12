use super::minutes_markdown::{MarkdownStamp, SaveMarkdownRequest};
use super::*;
use crate::generation::llama::LlamaConfig;
use crate::generation::{self, markdown_job};
use crate::local_engine::Fingerprint;
use std::os::unix::fs::PermissionsExt;

pub(in crate::storage) fn engine(root: &Path, kind: &str) -> LlamaConfig {
    let executable = root.join("llama-completion");
    let model = root.join("model.gguf");
    let script = format!(
        r#"#!/usr/bin/python3
import sys,json,pathlib
args=sys.argv[1:]
def arg(name): return args[args.index(name)+1]
base=pathlib.Path(arg('--model')).parent
with (base/'calls').open('a') as f: f.write('call\n')
p=json.loads(pathlib.Path(arg('--file')).read_text())
schema=json.loads(pathlib.Path(arg('--json-schema-file')).read_text())
assert list(schema['properties'])==['markdown']
assert 'utteranceId' not in json.dumps(p) and 'speakerId' not in json.dumps(p)
kind={kind:?}
if kind=='invalid': print(json.dumps({{'items':[]}}));sys.exit(0)
if kind=='wide' and 'utterances' in p: text='異なる確認事項\n'*230
else: text=' \n# 点検会議\n- 点検日\n[区間](audio:00:00:00.100001-00:00:00.799999)\n'
print(json.dumps({{'markdown':text}},ensure_ascii=False))
"#
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&model, b"synthetic GGUF").unwrap();
    LlamaConfig {
        executable: Fingerprint::capture(&executable, &|| false).unwrap(),
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        threads: 1,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    }
}
fn request(storage: &Storage, long: bool) -> MinutesJobRequest {
    let meeting = analysis::tests::saved_meeting(storage);
    std::fs::create_dir_all(storage.root.join("meetings").join(&meeting.id)).unwrap();
    let mut input = analysis::tests::input();
    input.utterances[0].text = if long {
        "点検日・異なる作業を確認します。".repeat(120)
    } else {
        "点検日を確認します".into()
    };
    let id = storage
        .create_analysis_candidate(&meeting.id, input)
        .unwrap();
    let committed = storage
        .commit_analysis_candidate(&meeting.id, &id, 0, false)
        .unwrap();
    MinutesJobRequest {
        meeting_id: meeting.id,
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: id,
        expected_content_hash: committed.content_hash,
        expected_meeting_lock_version: committed.meeting_lock_version,
        expected_draft: None,
    }
}
fn save(
    storage: &Storage,
    meeting: &str,
    expected: Option<MarkdownStamp>,
    text: &str,
) -> super::minutes_markdown::MarkdownDocument {
    storage
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: meeting.into(),
            operation_id: Uuid::new_v4().to_string(),
            expected,
            markdown: text.into(),
        })
        .unwrap()
}
pub(in crate::storage) fn calls(root: &Path) -> usize {
    std::fs::read_to_string(root.join("calls"))
        .unwrap_or_default()
        .lines()
        .count()
}
fn claim(storage: &Storage, job: &JobSnapshot) {
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
}
// Share one dispatcher instantiation instead of compiling the full generation
// pipeline again for every synthetic callback type.
pub(in crate::storage) fn process(
    storage: &Storage,
    job: &str,
    stopping: &dyn Fn() -> bool,
    progress: &dyn Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    generation::process(storage, job, &stopping, &progress)
}

#[test]
fn markdown_worker_publishes_exact_model_text_directly_and_reopen_keeps_it_without_candidates() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &job);
    assert_eq!(
        process(&storage, &job.id, &|| false, &|_| {}).unwrap(),
        Some(req.meeting_id.clone())
    );
    let document = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        document.markdown,
        " \n# 点検会議\n- 点検日\n[区間](audio:00:00:00.100001-00:00:00.799999)\n"
    );
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
    assert_eq!(storage.get_job(&job.id).unwrap().output_minutes_id, None);
    assert_eq!(calls(root.path()), 1);
    let proof = storage.notification_output(&job.id).unwrap();
    assert_eq!(proof.content_hash, document.content_hash);
    assert_eq!(proof.publication_state, "markdown");
    let human = save(
        &storage,
        &req.meeting_id,
        Some(document.stamp()),
        "生成後の人間の編集",
    );
    let proof = storage.notification_output(&job.id).unwrap();
    assert_eq!(proof.content_hash, document.content_hash);
    assert_eq!(proof.publication_state, "superseded");
    let db = storage.connection.lock().unwrap();
    for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    drop(db);
    drop(storage);
    assert_eq!(
        Storage::open(root.path().into())
            .unwrap()
            .get_minutes_markdown(&req.meeting_id)
            .unwrap(),
        Some(human)
    );
}
#[test]
fn markdown_regeneration_replaces_one_body_without_structured_versions_or_edit_history() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let mut req = request(&storage, false);
    let first = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &first);
    process(&storage, &first.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let original = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    let human = save(
        &storage,
        &req.meeting_id,
        Some(original.stamp()),
        "上書きを明示確認する人間の本文",
    );
    req.operation_id = Uuid::new_v4().to_string();
    req.expected_meeting_lock_version = storage
        .get_meeting(&req.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    let second = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        Some(human.stamp()),
    )
    .unwrap();
    claim(&storage, &second);
    process(&storage, &second.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let regenerated = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert_eq!(regenerated.markdown, original.markdown);
    assert!(regenerated.lock_version > human.lock_version);
    let db = storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM minutes_markdown WHERE meeting_id=?1",
            [&req.meeting_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM working_drafts WHERE target_kind='minutes'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM edit_operations WHERE operation_kind IN ('replace_minutes','undo_minutes') OR operation_kind LIKE 'undo_minutes:%'", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(db.query_row("SELECT current_minutes_id IS NULL AND previous_minutes_id IS NULL FROM meetings WHERE id=?1", [&req.meeting_id], |r| r.get::<_,bool>(0)).unwrap(), true);
    drop(db);
    drop(storage);
    assert_eq!(
        Storage::open(root.path().into())
            .unwrap()
            .get_minutes_markdown(&req.meeting_id)
            .unwrap(),
        Some(regenerated)
    );
}

#[test]
fn markdown_worker_preserves_human_save_after_start_and_rejects_initial_result_conflict() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let base = save(&storage, &req.meeting_id, None, "開始前の本文");
    let job = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        Some(base.stamp()),
    )
    .unwrap();
    claim(&storage, &job);
    let human = save(
        &storage,
        &req.meeting_id,
        Some(base.stamp()),
        "生成中に人間が保存",
    );
    assert!(process(&storage, &job.id, &|| false, &|_| {})
        .unwrap_err()
        .contains("更新"));
    assert_eq!(
        storage.get_minutes_markdown(&req.meeting_id).unwrap(),
        Some(human)
    );
    assert_eq!(storage.get_job(&job.id).unwrap().state, "running");
}
#[test]
fn markdown_invalid_model_result_and_changed_engine_never_replace_saved_human_text() {
    for mode in ["invalid", "changed"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let req = request(&storage, false);
        let base = save(&storage, &req.meeting_id, None, "保持する本文");
        let config = engine(root.path(), mode);
        let job = markdown_job::enqueue(
            &storage,
            req.clone(),
            config.clone(),
            "standard",
            2400,
            Some(base.stamp()),
        )
        .unwrap();
        claim(&storage, &job);
        if mode == "changed" {
            std::fs::write(&config.model.path, b"changed bytes").unwrap();
        }
        assert!(process(&storage, &job.id, &|| false, &|_| {}).is_err());
        assert_eq!(
            storage.get_minutes_markdown(&req.meeting_id).unwrap(),
            Some(base)
        );
        assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 0);
    }
}
#[test]
fn markdown_completed_unit_pauses_resumes_after_reopen_and_does_not_rerun_the_model() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &job);
    assert!(process(&storage, &job.id, &|| false, &|j| {
        if j.state == "running" {
            storage.request_job_pause(&job.id, "user").unwrap();
        }
    })
    .unwrap()
    .is_none());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(calls(root.path()), 1);
    assert!(storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .is_none());
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    assert!(process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    assert_eq!(calls(root.path()), 1);
}
#[test]
fn markdown_partial_merge_resumes_exact_subunits_and_rejects_tampered_raw_bytes() {
    for tamper in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let req = request(&storage, true);
        let job = markdown_job::enqueue(
            &storage,
            req.clone(),
            engine(root.path(), "wide"),
            "standard",
            2400,
            None,
        )
        .unwrap();
        assert!(job.total_units.unwrap() > 1);
        claim(&storage, &job);
        assert!(process(&storage, &job.id, &|| false, &|j| {
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
        assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
        let (_, checkpoint) = storage.fixed_job_input(&job.id).unwrap();
        let artifact = checkpoint.as_ref().unwrap()["pending"]["batches"][0][3]["id"]
            .as_str()
            .unwrap();
        if tamper {
            let relative: String = storage
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT relative_path FROM file_assets WHERE id=?1",
                    [artifact],
                    |r| r.get(0),
                )
                .unwrap();
            std::fs::write(storage.root.join(relative), b"tampered").unwrap();
        }
        let before = calls(root.path());
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        storage.resume_job(&job.id).unwrap();
        claim(&storage, &job);
        let first_new_batch = std::cell::Cell::new(false);
        let result = process(&storage, &job.id, &|| false, &|_| {
            if !first_new_batch.get() {
                assert_eq!(
                    calls(root.path()),
                    before + 1,
                    "saved merge batch was run again"
                );
                first_new_batch.set(true);
            }
        });
        if tamper {
            assert!(result.is_err());
            assert_eq!(calls(root.path()), before);
            assert!(storage
                .get_minutes_markdown(&req.meeting_id)
                .unwrap()
                .is_none());
        } else {
            assert!(result.unwrap().is_some());
            assert!(first_new_batch.get());
            assert!(calls(root.path()) > before);
            assert!(storage
                .get_minutes_markdown(&req.meeting_id)
                .unwrap()
                .is_some());
        }
    }
}
#[test]
fn markdown_stopping_before_first_unit_pauses_without_running_or_publishing() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &job);
    assert!(process(&storage, &job.id, &|| true, &|_| {})
        .unwrap()
        .is_none());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(calls(root.path()), 0);
    assert!(storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .is_none());
}

#[test]
fn markdown_terminal_retry_reuses_verified_units_without_publishing_on_failure_or_cancel() {
    for outcome in ["failed", "cancelled"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let req = request(&storage, false);
        let human = save(&storage, &req.meeting_id, None, "人間の本文を保持");
        let job = markdown_job::enqueue(
            &storage,
            req.clone(),
            engine(root.path(), "normal"),
            "standard",
            2400,
            Some(human.stamp()),
        )
        .unwrap();
        claim(&storage, &job);
        assert!(process(&storage, &job.id, &|| false, &|j| {
            if j.state == "running" && j.completed_units == 1 {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        })
        .unwrap()
        .is_none());
        let source = storage
            .finish_job(
                &job.id,
                1,
                outcome,
                if outcome == "failed" {
                    Some("synthetic_failure")
                } else {
                    None
                },
            )
            .unwrap();
        assert_eq!(
            storage.get_minutes_markdown(&req.meeting_id).unwrap(),
            Some(human.clone())
        );
        assert_eq!(calls(root.path()), 1);
        assert!(storage.notification_output(&job.id).is_err());
        // An invalid operation cannot queue a retry that would later fail the
        // canonical operation check at final Markdown publication.
        let retry = |op: String| generation::RetryRequest {
            meeting_id: req.meeting_id.clone(),
            job_id: source.id.clone(),
            operation_id: op,
            expected_spec_hash: source.spec_hash.clone(),
        };
        assert!(retry("not-a-uuid".into()).enqueue(&storage).is_err());
        let op = Uuid::new_v4().to_string();
        let next = retry(op.clone()).enqueue(&storage).unwrap();
        assert_eq!(retry(op).enqueue(&storage).unwrap(), next);
        let input = storage.load_minutes_input(&source.id).unwrap();
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        assert_eq!(storage.load_minutes_input(&next.id).unwrap(), input);
        claim(&storage, &next);
        assert!(process(&storage, &next.id, &|| false, &|_| {})
            .unwrap()
            .is_some());
        assert_eq!(
            calls(root.path()),
            1,
            "durable extraction must not run again"
        );
        let document = storage
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap();
        assert_eq!(document.operation_id, next.operation_id);
        assert_eq!(document.lock_version, human.lock_version + 1);
        assert_eq!(storage.get_job(&source.id).unwrap(), source);
        assert_eq!(
            storage
                .notification_output(&next.id)
                .unwrap()
                .publication_state,
            "markdown"
        );
    }
}
