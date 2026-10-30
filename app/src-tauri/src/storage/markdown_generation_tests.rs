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
if kind=='custom':
    system=pathlib.Path(arg('--system-prompt-file')).read_text()
    assert system.startswith('CUSTOM_MERGE_' if 'drafts' in p else 'CUSTOM_EXTRACT_')
    assert '簡潔' not in system and '500文字' not in system
    with (base/'systems').open('a') as f: f.write(json.dumps(system,ensure_ascii=False)+'\n')
    text='# 自由な文書\n\n| 項目 | 内容 |\n| --- | --- |\n| 状況 | 確認中 |\n\n## 要約\n'+('長い文章。'*140)
    print(json.dumps({{'markdown':text}},ensure_ascii=False));sys.exit(0)
if kind=='concise':
    system=pathlib.Path(arg('--system-prompt-file')).read_text()
    assert '500文字以内' in system
    link=p['utterances'][0]['audioLink']
    text='# 議事録\n\n## 要約\n'+('確認する。'+link)*160+'\n\n## 決定事項\n- 決定事項はそのまま保持する。'+link+'\n\n## ToDo\n- 作業を確認する。'+link
    print(json.dumps({{'markdown':text}},ensure_ascii=False));sys.exit(0)
if kind=='invalid': print(json.dumps({{'items':[]}}));sys.exit(0)
if kind=='truncated': print('{{"markdown":"unfinished');sys.exit(0)
if kind=='budget':
    system=pathlib.Path(arg('--system-prompt-file')).read_bytes()
    prompt=pathlib.Path(arg('--file')).read_bytes()
    expected=min(8192,int(arg('--ctx-size'))-len(system)-len(prompt)-512)
    assert int(arg('--predict'))==expected and expected>2048
    assert 'maxLength' not in schema['properties']['markdown']
    print(json.dumps({{'markdown':'# 点検会議\n'+'- 確認する事項\n'*100}},ensure_ascii=False));sys.exit(0)
if kind=='model_maximum':
    system=pathlib.Path(arg('--system-prompt-file')).read_bytes()
    prompt=pathlib.Path(arg('--file')).read_bytes()
    assert int(arg('--ctx-size'))==32768
    assert int(arg('--predict'))==8192
    assert len(system)+len(prompt)+512+8192<=32768
    print(json.dumps({{'markdown':'# モデル上限で生成\n- 確認事項を統合しました。'}},ensure_ascii=False));sys.exit(0)
if kind=='model_adaptive':
    system=pathlib.Path(arg('--system-prompt-file')).read_bytes()
    prompt=pathlib.Path(arg('--file')).read_bytes()
    expected=min(32768,40960-len(system)-len(prompt)-512)
    assert int(arg('--ctx-size'))==40960
    assert int(arg('--predict'))==expected and expected>=8192
    assert 'maxLength' not in schema['properties']['markdown']
    with (base/'allocations').open('a') as f:
        f.write(json.dumps({{'output':expected,'input':len(system)+len(prompt)+512,'merge':'drafts' in p}})+'\n')
    left='左'*1092
    right='右'*2196+'x'
    if 'utterances' in p:
        text=left if len((base/'calls').read_text().splitlines())==1 else right
    else:
        assert p['drafts']==[left,right]
        text='# 統合した議事録\n- 左右の確認事項を統合しました。'
    print(json.dumps({{'markdown':text}},ensure_ascii=False));sys.exit(0)
if kind in ('inline_evidence','compact_evidence'):
    system=pathlib.Path(arg('--system-prompt-file')).read_text()
    assert '要約（続）' in system and '直後' in system and '再生区間セクション' in system
    if 'utterances' in p:
        for u in p['utterances']:
            if kind=='inline_evidence': assert u['audioLink']=='['+u['start']+'–'+u['end']+'](audio:'+u['start']+'-'+u['end']+')'
            else: assert 'start' not in u and 'end' not in u and u.get('partial',True) is True and '.' not in u['audioLink']
        text='# 議事録\n\n## 要約\n範囲を確認する。'+p['utterances'][0]['audioLink']
    else:
        assert all('## 要約\n' in d and '](audio:' in d for d in p['drafts'])
        text=p['drafts'][0]
    print(json.dumps({{'markdown':text}},ensure_ascii=False));sys.exit(0)
if kind=='wide' and 'utterances' in p: text='異なる確認事項\n'*500
else: text=' \n# 点検会議\n- 点検日\n[区間](audio:00:00:00.100001-00:00:00.799999)\n'
print(json.dumps({{'markdown':text}},ensure_ascii=False))
"#
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&model, b"synthetic GGUF").unwrap();
    LlamaConfig {
        executable: Fingerprint::capture(&executable, &crate::test_support::never_cancelled)
            .unwrap(),
        model: Fingerprint::capture(&model, &crate::test_support::never_cancelled).unwrap(),
        threads: 1,
        context_tokens: if kind == "wide" { 16384 } else { 8192 },
        prediction_tokens: if kind == "wide" { 8192 } else { 2048 },
        gpu_layers: 0,
        seed: 1,
    }
}
fn request(storage: &Storage, long: bool) -> MinutesJobRequest {
    request_text_units(
        storage,
        if long {
            "点検日・異なる作業を確認します。".repeat(500)
        } else {
            "点検日を確認します".into()
        },
        if long { 5 } else { 1 },
    )
}
fn request_text(storage: &Storage, text: String) -> MinutesJobRequest {
    request_text_units(storage, text, 1)
}
fn request_text_units(storage: &Storage, text: String, copies: usize) -> MinutesJobRequest {
    let meeting = analysis::tests::saved_meeting(storage);
    std::fs::create_dir_all(storage.root.join("meetings").join(&meeting.id)).unwrap();
    let mut input = analysis::tests::input();
    input.utterances[0].text = text;
    for _ in 1..copies {
        input.utterances.push(input.utterances[0].clone());
    }
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

#[test]
fn incomplete_markdown_preserves_human_body_and_never_commits_a_partial_unit() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let human = save(&storage, &req.meeting_id, None, "保持する人間の本文");
    let job = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        engine(root.path(), "truncated"),
        "standard",
        2400,
        Some(human.stamp()),
    )
    .unwrap();
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(spec.parameters["generation"]["schemaVersion"], 9);
    claim(&storage, &job);
    let error = markdown_job::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap_err();
    assert!(error.contains("minutes_output_incomplete"));
    assert_eq!(
        storage
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap(),
        human
    );
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 0);
    assert!(storage.fixed_job_input(&job.id).unwrap().1.is_none());
}

#[test]
fn model_budget_drives_actual_cli_and_saves_more_than_480_characters_without_truncation() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        engine(root.path(), "budget"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        spec.parameters["generation"]["capacity"]["maxInputTokens"],
        32768
    );
    assert_eq!(
        spec.parameters["generation"]["capacity"]["maxOutputTokens"],
        8192
    );
    claim(&storage, &job);
    assert!(markdown_job::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .unwrap()
    .is_some());
    let body = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        body.markdown,
        format!("# 点検会議\n{}", "- 確認する事項\n".repeat(100))
    );
    assert!(body.markdown.chars().count() > 480);
    // Reopening represents a restart, not a second simultaneous DB owner.
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        reopened
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap(),
        body
    );
}

#[test]
fn model_adaptive_cli_merges_both_drafts_after_restart_with_variable_output_and_human_body_kept() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request_text(&storage, "長い議題を確認する。".repeat(1500));
    let human = save(&storage, &req.meeting_id, None, "生成前の本文を保持");
    let job = markdown_job::enqueue_synthetic_model_adaptive(
        &storage,
        req.clone(),
        engine(root.path(), "model_adaptive"),
        Some(human.stamp()),
    )
    .unwrap();
    assert_eq!(job.total_units, Some(3));
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        spec.parameters["generation"]["capacity"]["allocation"],
        "modelAdaptive"
    );
    claim(&storage, &job);
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" && j.completed_units == 1 {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        }
    )
    .unwrap()
    .is_none());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(calls(root.path()), 1);
    assert_eq!(
        storage.get_minutes_markdown(&req.meeting_id).unwrap(),
        Some(human.clone())
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(storage.fixed_job_input(&job.id).unwrap().0, spec);
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .unwrap()
    .is_some());
    assert_eq!(calls(root.path()), 3); // No extraction replay or singleton merge loop.
    let allocations: Vec<serde_json::Value> =
        std::fs::read_to_string(root.path().join("allocations"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(allocations.len(), 3);
    assert_eq!(allocations[2]["merge"], true);
    assert!(allocations[2]["output"].as_u64().unwrap() < 32768);
    assert!(allocations
        .iter()
        .all(|a| a["input"].as_u64().unwrap() + a["output"].as_u64().unwrap() <= 40960));
    let body = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        body.markdown,
        "# 統合した議事録\n- 左右の確認事項を統合しました。"
    );
    assert_eq!(body.lock_version, human.lock_version + 1);
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
    drop(storage);
    assert_eq!(
        Storage::open(root.path().into())
            .unwrap()
            .get_minutes_markdown(&req.meeting_id)
            .unwrap(),
        Some(body)
    );
}

#[test]
fn unknown_selected_model_is_rejected_before_enqueuing_or_running_an_engine() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let error = markdown_job::enqueue(
        &storage,
        req,
        engine(root.path(), "unknown"),
        "standard",
        2400,
        None,
    )
    .unwrap_err();
    assert!(error.contains("minutes_model_limits_unknown"));
    assert!(!root.path().join("calls").exists());
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

// Only the opt-in real-asset test calls this. Its entire Storage and package
// are test-owned; no user's meeting, setting or original model is changed.
pub(crate) fn real_managed_model_markdown_roundtrip(
    storage: Storage,
    models: PathBuf,
    expected_model_hash: &str,
    label: &str,
) {
    let root = storage.root().to_owned();
    let preferences = storage.minutes_preferences().unwrap();
    let profiles = storage.model_profiles().unwrap();
    let engine = &preferences.payload.standard;
    let input = request(&storage, false);
    let enqueue = |input: MinutesJobRequest, expected: Option<MarkdownStamp>| {
        generation::StartRequest {
            input,
            executable_path: engine.executable_path.clone().into(),
            model_path: engine.model_path.clone().into(),
            threads: engine.threads,
            context_tokens: engine.context_tokens,
            prediction_tokens: engine.prediction_tokens,
            gpu_layers: engine.gpu_layers,
            seed: engine.seed,
            profile: "standard".into(),
            prompt_budget: engine.prompt_budget,
        }
        .enqueue_markdown(&storage, expected)
        .unwrap()
    };
    let first = enqueue(input.clone(), None);
    let (spec, _) = storage.fixed_job_input(&first.id).unwrap();
    assert!(
        super::model_profiles::jobs::read(&spec.profile_snapshot)
            .unwrap()
            .is_some(),
        "managed model proof must not be bypassed"
    );
    assert_eq!(
        spec.parameters["generation"]["capacity"]["modelSha256"],
        expected_model_hash
    );
    assert_eq!(
        spec.parameters["generation"]["capacity"]["maxOutputTokens"],
        8192
    );
    let started = std::time::Instant::now();
    claim(&storage, &first);
    assert_eq!(
        process(
            &storage,
            &first.id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap(),
        Some(input.meeting_id.clone())
    );
    let generated = storage
        .get_minutes_markdown(&input.meeting_id)
        .unwrap()
        .unwrap();
    assert!(!generated.markdown.trim().is_empty());
    assert_eq!(
        generated.content_hash,
        super::minutes_markdown::content_hash(&generated.markdown)
    );
    assert_eq!(storage.get_job(&first.id).unwrap().state, "succeeded");
    assert_eq!(storage.get_job(&first.id).unwrap().output_minutes_id, None);
    println!(
        "{label} real managed Markdown generation: {:.3}s; {} bytes",
        started.elapsed().as_secs_f64(),
        generated.markdown.len()
    );
    let human = save(
        &storage,
        &input.meeting_id,
        Some(generated.stamp()),
        "# QA human edit\n人間が明示的に保存した点検日\n",
    );
    assert_eq!(
        storage
            .notification_output(&first.id)
            .unwrap()
            .publication_state,
        "superseded"
    );
    let mut again = input.clone();
    again.operation_id = Uuid::new_v4().to_string();
    again.expected_meeting_lock_version = storage
        .get_meeting(&input.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    let second = enqueue(again, Some(human.stamp()));
    assert_eq!(
        storage
            .get_minutes_markdown(&input.meeting_id)
            .unwrap()
            .unwrap(),
        human,
        "enqueue cannot overwrite human text"
    );
    claim(&storage, &second);
    let started = std::time::Instant::now();
    assert_eq!(
        process(
            &storage,
            &second.id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap(),
        Some(input.meeting_id.clone())
    );
    let regenerated = storage
        .get_minutes_markdown(&input.meeting_id)
        .unwrap()
        .unwrap();
    assert!(regenerated.lock_version > human.lock_version);
    assert_eq!(storage.get_job(&second.id).unwrap().state, "succeeded");
    println!(
        "{label} real managed explicit regeneration: {:.3}s; {} bytes",
        started.elapsed().as_secs_f64(),
        regenerated.markdown.len()
    );
    for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(storage.minutes_preferences().unwrap(), preferences);
    assert_eq!(storage.model_profiles().unwrap(), profiles);
    drop(storage);
    let reopened = Storage::open_with_models(root, models).unwrap();
    assert_eq!(
        reopened
            .get_minutes_markdown(&input.meeting_id)
            .unwrap()
            .unwrap(),
        regenerated
    );
    assert_eq!(reopened.get_job(&first.id).unwrap().state, "succeeded");
    assert_eq!(reopened.get_job(&second.id).unwrap().state, "succeeded");
    assert_eq!(reopened.minutes_preferences().unwrap(), preferences);
    assert_eq!(reopened.model_profiles().unwrap(), profiles);
    println!("real managed generation, human edit, explicit overwrite and cold reopen retained; no structured revision or evidence rows");
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
    let job = markdown_job::enqueue_synthetic(
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
        process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap(),
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
    let first = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &first);
    process(
        &storage,
        &first.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
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
    let second = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        Some(human.stamp()),
    )
    .unwrap();
    claim(&storage, &second);
    process(
        &storage,
        &second.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
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
    let job = markdown_job::enqueue_synthetic(
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
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
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
        let job = markdown_job::enqueue_synthetic(
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
        assert!(process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
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
    let job = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    claim(&storage, &job);
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        }
    )
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
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
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
        let job = markdown_job::enqueue_synthetic(
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
        assert!(process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|j| {
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
            }
        )
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
        let result = process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {
                if !first_new_batch.get() {
                    assert_eq!(
                        calls(root.path()),
                        before + 1,
                        "saved merge batch was run again"
                    );
                    first_new_batch.set(true);
                }
            },
        );
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
    let job = markdown_job::enqueue_synthetic(
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
        let job = markdown_job::enqueue_synthetic(
            &storage,
            req.clone(),
            engine(root.path(), "normal"),
            "standard",
            2400,
            Some(human.stamp()),
        )
        .unwrap();
        claim(&storage, &job);
        assert!(process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|j| {
                if j.state == "running" && j.completed_units == 1 {
                    storage.request_job_pause(&job.id, "user").unwrap();
                }
            }
        )
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
        assert!(process(
            &storage,
            &next.id,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
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

#[test]
fn inline_evidence_generation_keeps_sentence_links_through_merge_pause_and_cold_reopen() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, true);
    let human = save(&storage, &req.meeting_id, None, "人間の本文を完了まで保持");
    let job = markdown_job::enqueue_synthetic_inline(
        &storage,
        req.clone(),
        engine(root.path(), "compact_evidence"),
        Some(human.stamp()),
    )
    .unwrap();
    assert!(job.total_units.unwrap() > 1);
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        spec.parameters["generation"]["promptVersion"],
        "customPromptsV5"
    );
    claim(&storage, &job);
    assert!(process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        }
    )
    .unwrap()
    .is_none());
    assert_eq!(calls(root.path()), 1);
    assert_eq!(
        storage.get_minutes_markdown(&req.meeting_id).unwrap(),
        Some(human)
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls(root.path()), job.total_units.unwrap() as usize);
    let document = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert!(storage
        .notification_output(&job.id)
        .unwrap()
        .warnings
        .is_empty());
    assert_eq!(document.markdown.matches("## 要約").count(), 1);
    assert!(!document.markdown.contains("要約（続）"));
    assert!(!document.markdown.contains("## 再生区間"));
    let input = storage.load_minutes_input(&job.id).unwrap();
    let first = &input.analysis.utterances[0];
    let stamp = |us: i64| {
        format!(
            "{:02}:{:02}:{:02}",
            us / 3_600_000_000,
            us / 60_000_000 % 60,
            us / 1_000_000 % 60
        )
    };
    assert!(document.markdown.contains(&format!(
        "範囲を確認する。[{}–{}](audio:{}-{})",
        stamp(first.start_us),
        stamp((first.end_us / 1_000_000 + i64::from(first.end_us % 1_000_000 != 0)) * 1_000_000),
        stamp(first.start_us),
        stamp((first.end_us / 1_000_000 + i64::from(first.end_us % 1_000_000 != 0)) * 1_000_000)
    )));
    drop(storage);
    assert_eq!(
        Storage::open(root.path().into())
            .unwrap()
            .get_minutes_markdown(&req.meeting_id)
            .unwrap(),
        Some(document)
    );
}

#[test]
fn arbitrary_format_saves_exact_output_without_warning_after_reopen() {
    for long in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let req = request(&storage, long);
        let human = save(&storage, &req.meeting_id, None, "既存の議事録");
        let job = markdown_job::enqueue_synthetic_inline(
            &storage,
            req.clone(),
            engine(root.path(), "normal"),
            Some(human.stamp()),
        )
        .unwrap();
        if long {
            assert!(job.total_units.unwrap() > 1);
        }
        claim(&storage, &job);
        assert_eq!(
            process(
                &storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap(),
            Some(req.meeting_id.clone())
        );
        let done = storage.get_job(&job.id).unwrap();
        assert_eq!(done.state, "succeeded");
        assert_eq!(done.error_code, None);
        assert_eq!(done.total_units, Some(done.completed_units));
        let document = storage
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            document.markdown,
            " \n# 点検会議\n- 点検日\n[区間](audio:00:00:00.100001-00:00:00.799999)\n"
        );
        assert_eq!(document.lock_version, human.lock_version + 1);
        let proof = storage.notification_output(&job.id).unwrap();
        assert_eq!(proof.publication_state, "markdown");
        assert_eq!(proof.content_hash, document.content_hash);
        assert!(proof.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(&proof).unwrap()["warnings"],
            serde_json::json!([])
        );
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        assert_eq!(
            storage.get_minutes_markdown(&req.meeting_id).unwrap(),
            Some(document.clone())
        );
        assert_eq!(
            storage.notification_output(&job.id).unwrap().warnings,
            Vec::<String>::new()
        );
        save(
            &storage,
            &req.meeting_id,
            Some(document.stamp()),
            "後から編集した本文",
        );
        let proof = storage.notification_output(&job.id).unwrap();
        assert_eq!(proof.publication_state, "superseded");
        assert!(proof.warnings.is_empty());
    }
}

#[test]
fn compact_generation_replays_across_pause_and_cold_reopen_and_checks_second_links() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, true);
    let model = engine(root.path(), "compact_evidence");
    let job = markdown_job::enqueue_synthetic_compact(&storage, req.clone(), model, None).unwrap();
    assert!(job.total_units.unwrap() > 1);
    assert_eq!(
        storage.fixed_job_input(&job.id).unwrap().0.parameters["generation"]["promptVersion"],
        "customPromptsV5"
    );
    claim(&storage, &job);
    let events = std::cell::RefCell::new(Vec::new());
    markdown_job::process_observed(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        },
        &|v| events.borrow_mut().push(serde_json::to_value(v).unwrap()),
    )
    .unwrap();
    assert_eq!(calls(root.path()), 1);
    assert!(events.borrow().iter().all(|v| v["jobId"] == job.id
        && v["specHash"] == job.spec_hash
        && v["attempt"] == job.attempt));
    assert!(events.borrow().iter().any(|v| v["phase"] == "input"));
    assert!(events.borrow().iter().any(|v| v["phase"] == "saving"));
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls(root.path()), job.total_units.unwrap() as usize);
    assert!(storage
        .notification_output(&job.id)
        .unwrap()
        .warnings
        .is_empty());
    let doc = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert!(doc.markdown.contains("](audio:"));
    assert!(!doc.markdown.contains(".000000"));
    let folder = storage
        .root
        .join("meetings")
        .join(&req.meeting_id)
        .join("analysis")
        .join(&job.id);
    let mut attempts = Vec::new();
    for unit in std::fs::read_dir(folder).unwrap() {
        for file in std::fs::read_dir(unit.unwrap().path()).unwrap() {
            let file = file.unwrap();
            if file
                .file_name()
                .to_string_lossy()
                .starts_with("attempt-performance-")
            {
                let value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
                assert_eq!(value["jobId"], job.id);
                assert_eq!(value["specHash"], job.spec_hash);
                attempts.push(value);
            }
        }
    }
    assert!(attempts.iter().any(|v| v["outcome"] == "stopped"));
    let success = attempts
        .iter()
        .find(|v| v["outcome"] == "succeeded")
        .unwrap();
    assert!(success["publicationMs"].as_u64().is_some());
    assert!(success["finalVerificationMs"].as_u64().is_some());
    assert_eq!(success["completedUnits"], job.total_units.unwrap());
}

#[test]
#[ignore = "requires an idle Mac and MON_LLAMA_COMPLETION / MON_LLAMA_MODEL; runs actual GPU inference"]
fn real_metal_generation_pins_choice_before_first_unit_and_resumes_after_reopen() {
    let executable = std::env::var("MON_LLAMA_COMPLETION").unwrap();
    let model = std::env::var("MON_LLAMA_MODEL").unwrap();
    let root = tempfile::tempdir().unwrap();
    // Use a test-owned external package, including the adjacent native
    // libraries. A fresh Storage deliberately has no applied managed profile;
    // pointing it at the user's managed directory must remain forbidden.
    let source = Path::new(&executable).parent().unwrap();
    assert_eq!(Path::new(&model).parent(), Some(source));
    let package = root.path().join("external-runtime");
    assert!(std::process::Command::new("/bin/cp")
        .arg("-cR")
        .arg(source)
        .arg(&package)
        .status()
        .unwrap()
        .success());
    let executable = package.join(Path::new(&executable).file_name().unwrap());
    let model = package.join(Path::new(&model).file_name().unwrap());
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let engine = LlamaConfig {
        executable: Fingerprint::capture(&executable, &crate::test_support::never_cancelled)
            .unwrap(),
        model: Fingerprint::capture(&model, &crate::test_support::never_cancelled).unwrap(),
        threads: 4,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    };
    let job = markdown_job::enqueue(&storage, req.clone(), engine, "standard", 5400, None).unwrap();
    claim(&storage, &job);
    markdown_job::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        },
    )
    .unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        spec.parameters["generation"]["accelerationPolicy"],
        "autoMetalV2"
    );
    let pinned = checkpoint.unwrap()["acceleration"].clone();
    assert_eq!(pinned["backend"], "metal");
    let layers = pinned["proof"]["gpuLayers"].as_u64().unwrap();
    assert!((1..=256).contains(&layers));
    assert_eq!(pinned["proof"]["schemaVersion"], 2);
    assert_eq!(storage.get_job(&job.id).unwrap().completed_units, 0);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    markdown_job::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        storage.fixed_job_input(&job.id).unwrap().1.unwrap()["acceleration"],
        pinned
    );
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
    let folder = storage
        .root
        .join("meetings")
        .join(&req.meeting_id)
        .join("analysis")
        .join(&job.id);
    let mut actual_gpu = 0;
    for unit in std::fs::read_dir(folder).unwrap() {
        let path = unit.unwrap().path();
        if path.join("performance.json").exists() {
            let stats: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path.join("performance.json")).unwrap())
                    .unwrap();
            assert_eq!(stats["metalDevice"], "MTL0");
            assert_eq!(stats["offloadedLayers"], layers);
            actual_gpu += 1;
        }
    }
    assert_eq!(
        actual_gpu, 2,
        "one synthetic GPU trial and one extraction, with no second trial on resume"
    );
    println!(
        "actual Metal generation: completed {}, pinned GPU{}, resumed without another trial",
        job.id, layers
    );
}

#[test]
fn concise_summary_replays_after_final_checkpoint_and_never_caps_human_markdown() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue_synthetic_concise(
        &storage,
        req.clone(),
        engine(root.path(), "concise"),
        None,
    )
    .unwrap();
    claim(&storage, &job);
    process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert_eq!(calls(root.path()), 1);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    claim(&storage, &job);
    process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls(root.path()), 1);
    let saved = storage
        .get_minutes_markdown(&req.meeting_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .markdown
            .split("## 要約\n")
            .nth(1)
            .unwrap()
            .split("## 決定事項")
            .next()
            .unwrap()
            .matches("確認する。")
            .count(),
        160
    );
    assert!(saved
        .markdown
        .contains("## 決定事項\n- 決定事項はそのまま保持する。"));
    assert!(saved.markdown.contains("## ToDo\n- 作業を確認する。"));
    assert_eq!(
        storage.notification_output(&job.id).unwrap().warnings,
        Vec::<String>::new()
    );
    let human = format!(
        "# 人間の編集\n\n## 要約\n{}",
        "自由に編集した本文。".repeat(150)
    );
    assert_eq!(
        save(
            &storage,
            &req.meeting_id,
            Some(MarkdownStamp {
                lock_version: saved.lock_version,
                content_hash: saved.content_hash
            }),
            &human
        )
        .markdown,
        human
    );
}

#[test]
fn custom_prompts_are_frozen_for_admission_replay_resume_and_new_generation() {
    use super::minutes_prompt_preferences::MinutesPromptPreferences;
    for long in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let custom = |suffix: &str| MinutesPromptPreferences {
            schema_version: 1,
            extract_prompt: format!("CUSTOM_EXTRACT_{suffix}"),
            merge_prompt: format!("CUSTOM_MERGE_{suffix}"),
        };
        storage
            .save_minutes_prompt_preferences(0, custom("A"))
            .unwrap();
        let req = request(&storage, long);
        let engine = engine(root.path(), "custom");
        let job =
            markdown_job::enqueue_synthetic_custom(&storage, req.clone(), engine.clone(), None)
                .unwrap();
        let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
        assert_eq!(
            spec.parameters["generation"]["promptVersion"],
            "customPromptsV5"
        );
        assert_eq!(
            spec.parameters["generation"]["promptSnapshot"]["settingsLockVersion"],
            1
        );
        storage
            .save_minutes_prompt_preferences(1, custom("B"))
            .unwrap();
        // A lost admission response is idempotent despite changing the common setting.
        assert_eq!(
            markdown_job::enqueue_synthetic_custom(&storage, req.clone(), engine.clone(), None)
                .unwrap()
                .id,
            job.id
        );
        claim(&storage, &job);
        process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|j| {
                if j.state == "running" {
                    storage.request_job_pause(&job.id, "user").unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
        drop(storage);
        let storage = Storage::open(root.path().into()).unwrap();
        storage.resume_job(&job.id).unwrap();
        claim(&storage, &job);
        process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        let document = storage
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap();
        assert!(document.markdown.contains("| 状況 | 確認中 |"));
        assert_eq!(document.markdown.matches("長い文章。").count(), 140);
        assert!(storage
            .notification_output(&job.id)
            .unwrap()
            .warnings
            .is_empty());
        let systems = std::fs::read_to_string(root.path().join("systems")).unwrap();
        assert!(systems.contains("CUSTOM_EXTRACT_A"));
        assert!(!systems.contains("CUSTOM_EXTRACT_B"));
        if long {
            assert!(systems.contains("CUSTOM_MERGE_A"));
        }
        let mut new_req = req.clone();
        new_req.operation_id = Uuid::new_v4().to_string();
        let new_job = markdown_job::enqueue_synthetic_custom(
            &storage,
            new_req,
            engine,
            Some(document.stamp()),
        )
        .unwrap();
        claim(&storage, &new_job);
        process(
            &storage,
            &new_job.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        assert!(std::fs::read_to_string(root.path().join("systems"))
            .unwrap()
            .contains("CUSTOM_EXTRACT_B"));
    }
}

#[test]
fn repeated_admission_keeps_saved_capacity_when_new_defaults_change() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let model = engine(root.path(), "custom");
    let original = markdown_job::enqueue_synthetic(
        &storage,
        req.clone(),
        model.clone(),
        "standard",
        5400,
        None,
    )
    .unwrap();
    let saved = storage.fixed_job_input(&original.id).unwrap();
    assert_eq!(
        saved.0.parameters["generation"]["capacity"]["maxContextTokens"],
        32768
    );
    // The new admission helper proposes the profile's smaller capacity. A
    // repeated operation must return the already fixed plan and capacity.
    let repeated = markdown_job::enqueue_synthetic_custom(&storage, req, model, None).unwrap();
    assert_eq!(repeated.id, original.id);
    assert_eq!(storage.fixed_job_input(&repeated.id).unwrap(), saved);
}

#[test]
fn custom_prompt_read_errors_and_oversized_instructions_never_enqueue() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let req = request(&storage, false);
    let engine = engine(root.path(), "custom");
    let mut prompts = storage.minutes_prompt_preferences().unwrap().payload;
    prompts.merge_prompt = "あ".repeat(5000);
    storage.save_minutes_prompt_preferences(0, prompts).unwrap();
    assert!(
        markdown_job::enqueue_synthetic_custom(&storage, req.clone(), engine.clone(), None)
            .unwrap_err()
            .contains("統合用プロンプトが長すぎ")
    );
    assert!(storage.list_jobs(&req.meeting_id).unwrap().is_empty());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE app_settings SET value_json='{}' WHERE key='minutes_prompt_preferences'",
            [],
        )
        .unwrap();
    assert!(markdown_job::enqueue_synthetic_custom(&storage, req.clone(), engine, None).is_err());
    assert!(storage.list_jobs(&req.meeting_id).unwrap().is_empty());
    assert_eq!(calls(root.path()), 0);
}

#[test]
fn custom_retry_keeps_original_prompts_and_durable_output() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let mut prompts = storage.minutes_prompt_preferences().unwrap().payload;
    prompts.extract_prompt = "CUSTOM_EXTRACT_ORIGINAL".into();
    prompts.merge_prompt = "CUSTOM_MERGE_ORIGINAL".into();
    storage
        .save_minutes_prompt_preferences(0, prompts.clone())
        .unwrap();
    let req = request(&storage, false);
    let job = markdown_job::enqueue_synthetic_custom(
        &storage,
        req.clone(),
        engine(root.path(), "custom"),
        None,
    )
    .unwrap();
    claim(&storage, &job);
    process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|j| {
            if j.state == "running" {
                storage.request_job_pause(&job.id, "user").unwrap();
            }
        },
    )
    .unwrap();
    let failed = storage
        .finish_job(&job.id, 1, "failed", Some("synthetic_failure"))
        .unwrap();
    prompts.extract_prompt = "CUSTOM_EXTRACT_CHANGED".into();
    storage.save_minutes_prompt_preferences(1, prompts).unwrap();
    let retry = generation::RetryRequest {
        meeting_id: req.meeting_id.clone(),
        job_id: job.id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_spec_hash: failed.spec_hash,
    }
    .enqueue(&storage)
    .unwrap();
    let (spec, _) = storage.fixed_job_input(&retry.id).unwrap();
    assert_eq!(
        spec.parameters["generation"]["promptSnapshot"]["preferences"]["extractPrompt"],
        "CUSTOM_EXTRACT_ORIGINAL"
    );
    claim(&storage, &retry);
    process(
        &storage,
        &retry.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls(root.path()), 1);
    assert_eq!(
        storage
            .get_minutes_markdown(&req.meeting_id)
            .unwrap()
            .unwrap()
            .markdown
            .matches("長い文章。")
            .count(),
        140
    );
}
