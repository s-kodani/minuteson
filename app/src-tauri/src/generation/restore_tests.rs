use super::*;
use crate::minutes_document::{tests::input, ItemKind};
use std::collections::BTreeMap;

#[derive(Default, Clone)]
struct MemoryAssets {
    source: BTreeMap<String, (Role, String, Vec<u8>)>,
    target: BTreeMap<String, Vec<u8>>,
}
impl MemoryAssets {
    fn replace(&mut self, artifact: &mut Artifact, bytes: Vec<u8>) {
        artifact.hash = format!("{:x}", Sha256::digest(&bytes));
        let saved = self.source.get_mut(&artifact.id).unwrap();
        assert_eq!(saved.0, role(artifact).unwrap());
        saved.1 = artifact.hash.clone();
        saved.2 = bytes;
    }
    fn add(&mut self, name: &str, bytes: Vec<u8>) -> Artifact {
        let artifact = Artifact {
            name: name.into(),
            id: format!("asset-{}", self.source.len()),
            hash: format!("{:x}", Sha256::digest(&bytes)),
        };
        self.source.insert(
            artifact.id.clone(),
            (role(&artifact).unwrap(), artifact.hash.clone(), bytes),
        );
        artifact
    }
    fn saved(&mut self, task: &Task, raw: Value, items: &[GeneratedItem]) -> SavedBatch {
        let artifacts = vec![
            self.add("system.txt", task.system.as_bytes().to_vec()),
            self.add("prompt.txt", task.prompt.as_bytes().to_vec()),
            self.add("schema.json", serde_json::to_vec(&task.schema).unwrap()),
            self.add("result.json", serde_json::to_vec(&raw).unwrap()),
        ];
        let normalized = self.add("items.json", serde_json::to_vec(items).unwrap());
        SavedBatch {
            artifacts,
            normalized,
        }
    }
}
impl Assets for MemoryAssets {
    fn read(&mut self, kind: Role, id: &str, hash: &str) -> Result<Vec<u8>, String> {
        let (expected, stored, bytes) = self.source.get(id).ok_or("missing")?;
        if expected != &kind || stored != hash {
            return Err("wrong provenance".into());
        }
        Ok(bytes.clone())
    }
    fn write(
        &mut self,
        kind: Role,
        id: &str,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(String, String), String> {
        self.read(kind, id, hash)?;
        let id = format!("new:{id}");
        if self.target.get(&id).is_some_and(|prior| prior != bytes) {
            return Err("conflicting historical artifact".into());
        }
        self.target.insert(id.clone(), bytes.to_vec());
        Ok((id, format!("{:x}", Sha256::digest(bytes))))
    }
    fn map(&mut self, domain: &str, id: &str) -> Result<String, String> {
        match (domain, id) {
            ("utterances", "utterance") => Ok("new-u-id!".into()),
            ("speakers", "speaker") => Ok("newspkr".into()),
            _ => Err("unowned reference".into()),
        }
    }
}
struct Fixture {
    old_input: MinutesInput,
    new_input: MinutesInput,
    old_spec: JobSpec,
    new_spec: JobSpec,
    old_job: JobSnapshot,
    new_job: JobSnapshot,
    checkpoint: Value,
    assets: MemoryAssets,
}
impl Fixture {
    fn run(&mut self, check: &impl Fn() -> bool) -> Result<Value, String> {
        checkpoint(
            Context {
                job: &self.old_job,
                spec: &self.old_spec,
                input: &self.old_input,
                initial_operation: "op",
            },
            Context {
                job: &self.new_job,
                spec: &self.new_spec,
                input: &self.new_input,
                initial_operation: "new-op",
            },
            self.checkpoint.clone(),
            &mut self.assets,
            check,
        )
    }
}
fn fixture(version: u32, long: bool, pending: bool) -> Fixture {
    fixture_stop(
        version,
        long,
        if pending {
            Stop::FirstPass
        } else {
            Stop::Complete
        },
    )
}
#[derive(Clone, Copy)]
enum Stop {
    Complete,
    FirstPass,
    CrossPass,
}
fn fixture_stop(version: u32, long: bool, stop: Stop) -> Fixture {
    let mut old_input = input();
    if long {
        old_input.analysis.utterances[0].effective_text = "田中が点検日を確認します。".repeat(160);
    }
    let mut new_input = old_input.clone();
    new_input.analysis.id = "newanalysis".into();
    new_input.analysis.meeting_id = "newmeet".into();
    new_input.analysis.speakers[0].id = "newspkr".into();
    new_input.analysis.utterances[0].id = "new-u-id!".into();
    new_input.analysis.utterances[0].speaker_ids = vec!["newspkr".into()];
    let fingerprint = crate::local_engine::Fingerprint {
        path: "mon-unavailable:test".into(),
        size: 1,
        sha256: "a".repeat(64),
    };
    let parameters = Parameters {
        schema_version: version,
        template_version: TEMPLATE_VERSION.into(),
        engine: llama::LlamaConfig {
            executable: fingerprint.clone(),
            model: fingerprint,
            threads: 1,
            context_tokens: 32768,
            prediction_tokens: 1024,
            gpu_layers: 0,
            seed: 1,
        },
        prompt_budget: 2400,
        plan: plan::build_versioned(&old_input, 2400, version).unwrap(),
    };
    assert_eq!(
        parameters.plan,
        plan::build_versioned(&new_input, 2400, version).unwrap()
    );
    let spec = |meeting: &str, operation: &str, input: &MinutesInput| JobSpec {
        meeting_id: meeting.into(),
        kind: JobKind::MinutesGeneration,
        operation_id: operation.into(),
        input_analysis_id: Some(input.analysis.id.clone()),
        input_hash: input.analysis.content_hash.clone(),
        profile_snapshot: profile(&parameters.engine, "standard"),
        parameters: json!({"schemaVersion":1,"request":MinutesJobRequest {
            meeting_id:meeting.into(),operation_id:operation.into(),expected_analysis_id:input.analysis.id.clone(),
            expected_content_hash:input.analysis.content_hash.clone(),expected_meeting_lock_version:0,expected_draft:None,
        },"generation":parameters}),
        total_units: Some(parameters.plan.nodes.len() as i64),
    };
    let old_spec = spec("meeting", "op", &old_input);
    let new_spec = spec("newmeet", "new-op", &new_input);
    let mut assets = MemoryAssets::default();
    let mut checkpoint = Checkpoint::default();
    let mut outputs = vec![];
    for (index, node) in parameters.plan.nodes.iter().enumerate() {
        if parameters.adaptive_merge() && matches!(node, plan::Node::Merge { .. }) {
            let mut cursor = merge_cursor(node, &outputs, &parameters).unwrap();
            let mut batches = vec![];
            while cursor.next().is_some() {
                let cross_pass = cursor
                    .next()
                    .unwrap()
                    .first()
                    .is_some_and(|(id, _)| id.starts_with("r1-"));
                let task = merge_task(&cursor).unwrap();
                let raw = adaptive::identity(cursor.next().unwrap());
                let items = merge::parse(raw.clone(), &old_input, cursor.next().unwrap()).unwrap();
                batches.push(assets.saved(&task, raw, &items));
                cursor.accept(items).unwrap();
                if (matches!(stop, Stop::FirstPass) && batches.len() == 2)
                    || (matches!(stop, Stop::CrossPass) && cross_pass)
                {
                    assert!(cursor.next().is_some());
                    checkpoint.pending = Some(PendingMerge {
                        index,
                        batches: std::mem::take(&mut batches),
                    });
                    break;
                }
            }
            if checkpoint.pending.is_some() {
                break;
            }
            let items = result::normalize_versioned(cursor.finish().unwrap(), version).unwrap();
            let normalized = assets.add("items.json", serde_json::to_vec(&items).unwrap());
            checkpoint.outputs.push(UnitOutput {
                index,
                artifacts: vec![],
                normalized,
                merge_batches: batches,
            });
            outputs.push(items);
        } else {
            let task = task(&old_input, node, &outputs, &parameters).unwrap();
            let raw = match node {
                plan::Node::Extract { .. } => {
                    let items: Vec<_> = (0..if long { 12 } else { 1 })
                        .map(|i| GeneratedItem {
                            kind: ItemKind::Todo,
                            body: format!("原文 utterance {i} 点検日を確認する"),
                            assignee_name: Some("田中".into()),
                            assignee_speaker_id: Some("speaker".into()),
                            due_date: None,
                            due_text: None,
                            evidence_utterance_ids: vec!["utterance".into()],
                        })
                        .collect();
                    json!({"schemaVersion":1,"items":items})
                }
                plan::Node::Merge { left, right } => {
                    adaptive::identity(&merge::sources(*left, *right, &outputs).unwrap())
                }
            };
            let items = parse_node(raw.clone(), &old_input, node, &outputs, version).unwrap();
            let batch = assets.saved(&task, raw, &items);
            checkpoint.outputs.push(UnitOutput {
                index,
                artifacts: batch.artifacts,
                normalized: batch.normalized,
                merge_batches: vec![],
            });
            outputs.push(items);
        }
    }
    let job = |spec: &JobSpec| JobSnapshot {
        id: format!("job-{}", spec.meeting_id),
        meeting_id: spec.meeting_id.clone(),
        kind: "minutes_generation".into(),
        state: "paused".into(),
        operation_id: spec.operation_id.clone(),
        input_hash: spec.input_hash.clone(),
        spec_hash: "c".repeat(64),
        completed_units: checkpoint.outputs.len() as i64,
        total_units: spec.total_units,
        error_code: None,
        created_at: 1,
        updated_at: 1,
        output_analysis_id: None,
        output_minutes_id: None,
        pause_reason: Some("recording".into()),
        retry_of_job_id: None,
        attempt: 1,
    };
    let old_job = job(&old_spec);
    let new_job = job(&new_spec);
    Fixture {
        old_input,
        new_input,
        old_spec,
        new_spec,
        old_job,
        new_job,
        checkpoint: serde_json::to_value(checkpoint).unwrap(),
        assets,
    }
}

#[test]
fn restore_generation_replays_references_not_natural_language_or_models() {
    let mut f = fixture(1, false, false);
    let value = f.run(&|| false).unwrap();
    let items: Vec<GeneratedItem> = serde_json::from_slice(
        &f.assets.target[value["outputs"][0]["normalized"]["id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(items[0].body, "原文 utterance 0 点検日を確認する");
    assert_eq!(items[0].evidence_utterance_ids, vec!["new-u-id!"]);
    assert_eq!(items[0].assignee_speaker_id.as_deref(), Some("newspkr"));
    let prompt = &f.assets.target[value["outputs"][0]["artifacts"][1]["id"].as_str().unwrap()];
    assert!(String::from_utf8_lossy(prompt).contains("new-u-id!"));
    assert_eq!(f.run(&|| false).unwrap(), value); // Historical reuse is idempotent.
}
#[test]
fn restore_generation_replays_adaptive_completed_and_pending_cross_pass_batches() {
    for version in [2, 3, 4] {
        for pending in [false, true] {
            let mut f = fixture(version, true, pending);
            let value = f.run(&|| false).unwrap();
            assert_eq!(
                value["outputs"].as_array().unwrap().len(),
                f.old_job.completed_units as usize
            );
            assert_eq!(value.get("pending").is_some(), pending);
            if !pending {
                assert!(value["outputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| o["mergeBatches"].as_array().is_some_and(|b| b.len() > 2)));
            }
        }
    }
}

#[test]
fn restore_generation_never_upgrades_fixed_prompt_policy_or_accepts_substituted_system_bytes() {
    for version in [1, 2, 3, 4] {
        let mut f = fixture(version, false, false);
        let expected = plan::extract_system(version).unwrap();
        let value = f.run(&|| false).unwrap();
        let restored_id = value["outputs"][0]["artifacts"][0]["id"].as_str().unwrap();
        assert_eq!(f.assets.target[restored_id], expected.as_bytes());
        let mut changed_version = fixture(version, false, false);
        changed_version.new_spec.parameters["generation"]["schemaVersion"] =
            json!(if version == 3 { 2 } else { 3 });
        assert!(changed_version.run(&|| false).is_err());
        let mut substituted = fixture(version, false, false);
        let id = substituted.checkpoint["outputs"][0]["artifacts"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        substituted.assets.source.get_mut(&id).unwrap().2 =
            plan::extract_system(if version >= 3 { 2 } else { 3 })
                .unwrap()
                .into_bytes();
        assert!(substituted.run(&|| false).is_err());
    }
}
#[test]
fn restore_repeated_extraction_preserves_raw_output_and_versioned_normalization() {
    for version in [3, 4] {
        let mut f = fixture(version, false, false);
        let p: Parameters =
            serde_json::from_value(f.old_spec.parameters["generation"].clone()).unwrap();
        let mut cp: Checkpoint = serde_json::from_value(f.checkpoint.clone()).unwrap();
        assert_eq!(cp.outputs.len(), 1);
        let source: Vec<GeneratedItem> =
            serde_json::from_slice(&f.assets.source[&cp.outputs[0].normalized.id].2).unwrap();
        let distinct: Vec<_> = (0..3)
            .map(|i| {
                let mut item = source[0].clone();
                item.body = format!("原文 utterance {i} 点検日を確認する");
                item
            })
            .collect();
        let raw_items: Vec<_> = distinct.iter().chain(distinct.iter()).cloned().collect();
        let raw = json!({"schemaVersion":1,"items":raw_items});
        let parsed = parse_node(raw.clone(), &f.old_input, &p.plan.nodes[0], &[], version).unwrap();
        f.assets.replace(
            &mut cp.outputs[0].artifacts[3],
            serde_json::to_vec(&raw).unwrap(),
        );
        f.assets.replace(
            &mut cp.outputs[0].normalized,
            serde_json::to_vec(&parsed).unwrap(),
        );
        f.checkpoint = serde_json::to_value(cp).unwrap();
        let value = f.run(&|| false).unwrap();
        let saved_raw: Value = serde_json::from_slice(
            &f.assets.target[value["outputs"][0]["artifacts"][3]["id"].as_str().unwrap()],
        )
        .unwrap();
        let saved_items: Vec<GeneratedItem> = serde_json::from_slice(
            &f.assets.target[value["outputs"][0]["normalized"]["id"].as_str().unwrap()],
        )
        .unwrap();
        assert_eq!(saved_raw["items"].as_array().unwrap().len(), 6);
        assert_eq!(saved_items.len(), if version == 4 { 3 } else { 6 });
        assert!(saved_items
            .iter()
            .all(|i| i.evidence_utterance_ids == vec!["new-u-id!"]));
        assert_eq!(saved_items, mapped(&mut f.assets, &parsed).unwrap());
        assert_eq!(f.run(&|| false).unwrap(), value);
        // Correct hashes alone must not authorize a different normalization.
        let mut cp: Checkpoint = serde_json::from_value(f.checkpoint.clone()).unwrap();
        let wrong = if version == 4 { raw_items } else { distinct };
        f.assets.replace(
            &mut cp.outputs[0].normalized,
            serde_json::to_vec(&wrong).unwrap(),
        );
        f.checkpoint = serde_json::to_value(cp).unwrap();
        f.assets.target.clear();
        assert!(f.run(&|| false).is_err());
    }
}
#[test]
fn restore_generation_rejects_changed_prompt_schema_raw_normalized_plan_and_lineage() {
    for name in [
        "system.txt",
        "prompt.txt",
        "schema.json",
        "result.json",
        "items.json",
    ] {
        let mut f = fixture(1, false, false);
        let key = f
            .assets
            .source
            .iter()
            .find(|(_, (_, _, bytes))| match name {
                "system.txt" => bytes.starts_with(plan::EXTRACT_SYSTEM.as_bytes()),
                "prompt.txt" => String::from_utf8_lossy(bytes).contains("meetingTitle"),
                "schema.json" => String::from_utf8_lossy(bytes).contains("additionalProperties"),
                "result.json" => bytes.starts_with(b"{\"items\""),
                "items.json" => bytes.starts_with(b"["),
                _ => false,
            })
            .unwrap()
            .0
            .clone();
        f.assets.source.get_mut(&key).unwrap().2 = b"{}".to_vec();
        assert!(f.run(&|| false).is_err(), "{name}");
    }
    let mut f = fixture(1, false, false);
    f.new_spec.parameters["generation"]["plan"]["nodes"] = json!([]);
    assert!(f.run(&|| false).is_err());
    let mut f = fixture(1, false, false);
    f.old_spec.parameters["request"]["operationId"] = json!("another-root");
    assert!(f.run(&|| false).is_err());
    let mut f = fixture(1, false, false);
    f.new_spec.parameters["generation"]["engine"]["seed"] = json!(99);
    assert!(f.run(&|| false).is_err());
    let mut f = fixture(1, false, false);
    f.new_job.input_hash = "d".repeat(64);
    assert!(f.run(&|| false).is_err());
}
#[test]
fn restore_generation_cancellation_writes_nothing_at_entry() {
    let mut f = fixture(2, true, true);
    assert!(f.run(&|| true).is_err());
    assert!(f.assets.target.is_empty());
}

#[test]
fn restore_generation_preserves_historical_progress_and_legacy_merge_sources() {
    let mut f = fixture(1, true, false);
    let full = f.run(&|| false).unwrap();
    assert!(full["outputs"].as_array().unwrap().len() > 2);
    for (old, new) in f.checkpoint["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .zip(full["outputs"].as_array().unwrap())
    {
        let id = old["artifacts"][3]["id"].as_str().unwrap();
        let old_raw: Value = serde_json::from_slice(&f.assets.source[id].2).unwrap();
        if old_raw["items"][0].get("sourceItemIds").is_some() {
            let id = new["artifacts"][3]["id"].as_str().unwrap();
            let new_raw: Value = serde_json::from_slice(&f.assets.target[id]).unwrap();
            assert_eq!(old_raw, new_raw);
        }
    }
    f.checkpoint["outputs"].as_array_mut().unwrap().truncate(1);
    f.old_job.completed_units = 1;
    f.new_job.completed_units = 1;
    let historical = f.run(&|| false).unwrap();
    assert_eq!(historical["outputs"].as_array().unwrap().len(), 1);
    assert_eq!(historical["outputs"][0], full["outputs"][0]);
}

#[test]
fn restore_generation_rejects_semantically_invalid_result_even_with_matching_asset_hash() {
    let mut f = fixture(1, false, false);
    let id = f.checkpoint["outputs"][0]["artifacts"][3]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let asset = f.assets.source.get_mut(&id).unwrap();
    let mut raw: Value = serde_json::from_slice(&asset.2).unwrap();
    raw["items"][0]["evidenceUtteranceIds"] = json!(["another-meeting-utterance"]);
    asset.2 = serde_json::to_vec(&raw).unwrap();
    asset.1 = format!("{:x}", Sha256::digest(&asset.2));
    f.checkpoint["outputs"][0]["artifacts"][3]["hash"] = json!(asset.1);
    assert!(f.run(&|| false).is_err());
    assert!(f.assets.target.is_empty());
}

#[test]
fn restore_generation_interrupted_replay_returns_no_checkpoint() {
    let mut f = fixture(2, true, true);
    let visits = std::cell::Cell::new(0);
    assert!(f
        .run(&|| {
            visits.set(visits.get() + 1);
            visits.get() >= 4
        })
        .is_err());
    assert!(!f.assets.target.is_empty()); // Private partial files, never ready output.
    assert!(f.assets.target.keys().all(|id| id.starts_with("new:")));
}

#[test]
fn restore_generation_preserves_pending_second_pass_position() {
    let mut f = fixture_stop(2, true, Stop::CrossPass);
    let batches = f.checkpoint["pending"]["batches"].as_array().unwrap();
    assert!(batches.len() > 2);
    let batch_count = batches.len();
    let id = batches.last().unwrap()["artifacts"][3]["id"]
        .as_str()
        .unwrap();
    let raw: Value = serde_json::from_slice(&f.assets.source[id].2).unwrap();
    assert!(raw["items"][0]["sourceItemIds"][0]
        .as_str()
        .unwrap()
        .starts_with("r1-"));
    let value = f.run(&|| false).unwrap();
    assert_eq!(value["pending"]["index"], f.checkpoint["pending"]["index"]);
    assert_eq!(
        value["pending"]["batches"].as_array().unwrap().len(),
        batch_count
    );
    let id = value["pending"]["batches"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["artifacts"][3]["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&f.assets.target[id]).unwrap(),
        raw
    );
}
