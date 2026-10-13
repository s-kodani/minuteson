//! Test-only construction of already stored structured job data.
//! No subprocess, retry preflight, or old product worker is used.
use super::*;
use crate::local_engine::Fingerprint;
use serde_json::json;
use std::path::Path;

pub(crate) fn engine(root: &Path, _wide: bool) -> llama::LlamaConfig {
    let executable = root.join("llama-completion");
    let model = root.join("model.gguf");
    std::fs::write(
        &executable,
        b"historical executable fingerprint fixture; never execute",
    )
    .unwrap();
    std::fs::write(
        &model,
        b"historical model fingerprint fixture; never execute",
    )
    .unwrap();
    std::fs::write(root.join("engine.calls"), b"").unwrap();
    llama::LlamaConfig {
        executable: Fingerprint::capture(&executable, &|| false).unwrap(),
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        threads: 1,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    }
}

pub(crate) fn seed_job(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: llama::LlamaConfig,
    name: &str,
    prompt_budget: usize,
    schema_version: u32,
) -> Result<JobSnapshot, String> {
    if !matches!(name, "standard" | "high_accuracy") || prompt_budget > budget(&engine)? {
        return Err("議事録プロファイルまたは入力上限が不正です".into());
    }
    engine.validate()?;
    let profile = managed::attach(profile(&engine, name), None)?;
    storage.enqueue_minutes_job_configured(request, profile, |input| {
        let plan = plan::build_versioned(input, prompt_budget, schema_version)?;
        let total = plan.nodes.len() as i64;
        let parameters = Parameters {
            schema_version,
            template_version: TEMPLATE_VERSION.into(),
            engine,
            prompt_budget,
            plan,
        };
        Ok((
            serde_json::to_value(parameters).map_err(|e| e.to_string())?,
            Some(total),
        ))
    })
}

fn raw(task: &Task, wide: bool) -> Value {
    if let Some(value) = &task.local {
        return value.clone();
    }
    let prompt: Value = serde_json::from_str(&task.prompt).unwrap();
    if let Some(utterances) = prompt["utterances"].as_array() {
        let refs: Vec<_> = utterances
            .iter()
            .map(|u| u["utteranceId"].clone())
            .collect();
        let offset = utterances
            .first()
            .map(|u| u["firstByte"].clone())
            .unwrap_or(json!(0));
        let items: Vec<_> = (0..if wide {12} else {1}).map(|i|json!({
            "kind":"summary", "body": if wide {format!("確認事項{offset}-{i}: {}", "内容の異なる作業を確認します。".repeat(5))} else {"点検日を確認する".into()},
            "assigneeName":null,"assigneeSpeakerId":null,"dueDate":null,"dueText":null,
            "evidenceUtteranceIds":refs})).collect();
        json!({"schemaVersion":1,"items":items})
    } else {
        json!({"schemaVersion":1,"items":prompt["items"].as_array().unwrap().iter().map(|i|
            json!({"body":i["item"]["body"],"sourceItemIds":[i["sourceItemId"]]})).collect::<Vec<_>>()})
    }
}
fn batch(
    storage: &Storage,
    job: &JobSnapshot,
    index: usize,
    task: &Task,
    raw: &Value,
    items: &[GeneratedItem],
) -> SavedBatch {
    let directory = storage
        .analysis_unit_directory(&job.meeting_id, &job.id, index)
        .unwrap();
    for (name, bytes) in [
        ("system.txt", task.system.as_bytes().to_vec()),
        ("prompt.txt", task.prompt.as_bytes().to_vec()),
        ("schema.json", serde_json::to_vec(&task.schema).unwrap()),
        ("result.json", serde_json::to_vec(raw).unwrap()),
    ] {
        write_new(&directory.join(name), &bytes).unwrap();
    }
    save_task_output(storage, &job.meeting_id, &directory, items).unwrap()
}
fn save_items(
    storage: &Storage,
    meeting: &str,
    directory: &std::path::Path,
    items: &[GeneratedItem],
) -> Result<Artifact, String> {
    write_new(
        &directory.join("items.json"),
        &serde_json::to_vec(items).map_err(|e| e.to_string())?,
    )?;
    let asset = storage.register_analysis_artifact(
        meeting,
        &directory.join("items.json"),
        artifact_type("items.json")?,
    )?;
    Ok(Artifact {
        name: "items.json".into(),
        id: asset.id,
        hash: asset.content_hash,
    })
}
fn save_task_output(
    storage: &Storage,
    meeting: &str,
    directory: &std::path::Path,
    items: &[GeneratedItem],
) -> Result<SavedBatch, String> {
    let normalized = save_items(storage, meeting, directory, items)?;
    let mut artifacts = vec![];
    for name in ["system.txt", "prompt.txt", "schema.json", "result.json"] {
        let asset = storage.register_analysis_artifact(
            meeting,
            &directory.join(name),
            artifact_type(name)?,
        )?;
        artifacts.push(Artifact {
            name: name.into(),
            id: asset.id,
            hash: asset.content_hash,
        });
    }
    Ok(SavedBatch {
        artifacts,
        normalized,
    })
}

// Populate complete or partially merged historical artifacts. These are data
// fixtures, not another implementation of external LLM execution.
pub(crate) fn seed_outputs(
    storage: &Storage,
    job: &JobSnapshot,
    partial: bool,
    wide: bool,
) -> Option<String> {
    let input = storage.load_minutes_input(&job.id).unwrap();
    let (spec, _) = storage.fixed_job_input(&job.id).unwrap();
    let p = checked_parameters(
        &spec,
        &input,
        &storage.fixed_job_initial_operation(&job.id).unwrap(),
    )
    .unwrap();
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    let mut checkpoint = Checkpoint::default();
    let mut outputs = vec![];
    for (index, node) in p.plan.nodes.iter().enumerate() {
        let (items, saved) = if p.adaptive_merge() && matches!(node, plan::Node::Merge { .. }) {
            let mut cursor = merge_cursor(node, &outputs, &p).unwrap();
            let mut batches = vec![];
            while cursor.next().is_some() {
                let task = merge_task(&cursor).unwrap();
                let raw = raw(&task, wide);
                let items = merge::parse(raw.clone(), &input, cursor.next().unwrap()).unwrap();
                batches.push(batch(storage, job, index, &task, &raw, &items));
                cursor.accept(items).unwrap();
                if partial && batches.len() == 2 {
                    checkpoint.pending = Some(PendingMerge { index, batches });
                    storage
                        .save_job_checkpoint(
                            &job.id,
                            index as i64,
                            index as i64,
                            serde_json::to_value(&checkpoint).unwrap(),
                        )
                        .unwrap();
                    storage.request_job_pause(&job.id, "user").unwrap();
                    storage
                        .save_job_checkpoint(
                            &job.id,
                            index as i64,
                            index as i64,
                            serde_json::to_value(&checkpoint).unwrap(),
                        )
                        .unwrap();
                    return None;
                }
            }
            let items =
                result::normalize_versioned(cursor.finish().unwrap(), p.schema_version).unwrap();
            let directory = storage
                .analysis_unit_directory(&job.meeting_id, &job.id, index)
                .unwrap();
            let normalized = save_items(storage, &job.meeting_id, &directory, &items).unwrap();
            (
                items,
                UnitOutput {
                    index,
                    artifacts: vec![],
                    normalized,
                    merge_batches: batches,
                },
            )
        } else {
            let task = task(&input, node, &outputs, &p).unwrap();
            let raw = raw(&task, wide);
            let items = parse_node(raw.clone(), &input, node, &outputs, p.schema_version).unwrap();
            let saved = batch(storage, job, index, &task, &raw, &items);
            (
                items,
                UnitOutput {
                    index,
                    artifacts: saved.artifacts,
                    normalized: saved.normalized,
                    merge_batches: vec![],
                },
            )
        };
        checkpoint.outputs.push(saved);
        storage
            .save_job_checkpoint(
                &job.id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).unwrap(),
            )
            .unwrap();
        outputs.push(items);
    }
    assert!(
        !partial,
        "partial fixture requires at least two merge batches"
    );
    let content = from_generated(&input, outputs.pop().unwrap()).unwrap();
    Some(
        storage
            .seed_stored_candidate_for_job(&job.id, checkpoint.outputs.len() as i64, content)
            .unwrap(),
    )
}
pub(crate) fn seed_retry(storage: &Storage, source: &JobSnapshot) -> JobSnapshot {
    let (spec, cp) = storage.fixed_job_input(&source.id).unwrap();
    storage
        .enqueue_retry_job(
            source,
            &spec,
            cp.as_ref(),
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
}
