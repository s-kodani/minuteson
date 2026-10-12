use super::*;
use serde_json::json;

#[derive(Default)]
struct MemoryAssets {
    source: BTreeMap<String, (Role, String, Vec<u8>)>,
    target: BTreeMap<String, Vec<u8>>,
}
impl MemoryAssets {
    fn batch(&mut self, task: &Task, text: &str) -> Vec<Artifact> {
        [
            ("system.txt", task.system.as_bytes().to_vec()),
            ("prompt.txt", task.prompt.as_bytes().to_vec()),
            ("schema.json", serde_json::to_vec(&task.schema).unwrap()),
            (
                "result.json",
                serde_json::to_vec(&json!({"markdown":text})).unwrap(),
            ),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            let a = Artifact {
                name: name.into(),
                id: format!("a{}", self.source.len()),
                hash: format!("{:x}", Sha256::digest(&bytes)),
            };
            self.source
                .insert(a.id.clone(), (role(&a).unwrap(), a.hash.clone(), bytes));
            a
        })
        .collect()
    }
}
impl Assets for MemoryAssets {
    fn read(&mut self, role: Role, id: &str, hash: &str) -> Result<Vec<u8>, String> {
        let (expected, sha, bytes) = self.source.get(id).ok_or("missing")?;
        if expected != &role || sha != hash {
            return Err("wrong ownership/hash".into());
        }
        Ok(bytes.clone())
    }
    fn write(
        &mut self,
        role: Role,
        id: &str,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(String, String), String> {
        self.read(role, id, hash)?;
        let id = format!("new-{id}");
        if self.target.get(&id).is_some_and(|old| old != bytes) {
            return Err("conflicting write".into());
        }
        self.target.insert(id.clone(), bytes.to_vec());
        Ok((id, format!("{:x}", Sha256::digest(bytes))))
    }
    fn map(&mut self, _: &str, _: &str) -> Result<String, String> {
        panic!("Markdown does not map utterance/evidence IDs");
    }
}
struct Fixture {
    old_input: MinutesInput,
    new_input: MinutesInput,
    old_spec: JobSpec,
    new_spec: JobSpec,
    old_job: JobSnapshot,
    new_job: JobSnapshot,
    value: Value,
    assets: MemoryAssets,
}
impl Fixture {
    fn run(&mut self, cancelled: &impl Fn() -> bool) -> Result<Value, String> {
        crate::generation::restore::checkpoint(
            Context {
                job: &self.old_job,
                spec: &self.old_spec,
                input: &self.old_input,
                initial_operation: "original-operation",
            },
            Context {
                job: &self.new_job,
                spec: &self.new_spec,
                input: &self.new_input,
                initial_operation: "mapped-operation",
            },
            self.value.clone(),
            &mut self.assets,
            cancelled,
        )
    }
    fn replace(&mut self, position: usize, bytes: Vec<u8>) {
        let a = &mut self.value["outputs"][0]["batches"][0][position];
        let id = a["id"].as_str().unwrap().to_string();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        a["hash"] = json!(hash);
        let saved = self.assets.source.get_mut(&id).unwrap();
        saved.1 = hash;
        saved.2 = bytes;
    }
}
const EXACT:&str=" \r\n# 点検日\n自然文のutteranceや /Users/fictional は保持\n[区間](audio:00:00:00.100001-00:00:00.799999)\n";
fn fixture(long: bool, pending: bool, queued: bool) -> Fixture {
    let mut old_input = crate::minutes_document::tests::input();
    if long {
        old_input.analysis.utterances[0].effective_text = "点検日について確認する。".repeat(150);
    }
    let mut new_input = old_input.clone();
    new_input.analysis.id = "mapped-analysis".into();
    new_input.analysis.meeting_id = "mapped-meeting".into();
    new_input.analysis.speakers[0].id = "mapped-speaker".into();
    new_input.analysis.utterances[0].id = "mapped-utterance".into();
    new_input.analysis.utterances[0].speaker_ids = vec!["mapped-speaker".into()];
    let engine = super::super::super::llama::LlamaConfig {
        executable: crate::local_engine::Fingerprint {
            path: "mon-unavailable:exe".into(),
            size: 1,
            sha256: "a".repeat(64),
        },
        model: crate::local_engine::Fingerprint {
            path: "mon-unavailable:model".into(),
            size: 2,
            sha256: "b".repeat(64),
        },
        threads: 1,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    };
    let p = Parameters {
        schema_version: 5,
        engine,
        prompt_budget: 2400,
        plan: plan::build(&old_input, 2400).unwrap(),
        markdown_target: GenerationTarget {
            schema_version: 1,
            expected: Some(MarkdownStamp {
                lock_version: 7,
                content_hash: content_hash("以前の人間の本文"),
            }),
        },
    };
    let spec = |input: &MinutesInput, operation: &str| JobSpec {
        kind: JobKind::MinutesGeneration,
        meeting_id: input.analysis.meeting_id.clone(),
        operation_id: operation.into(),
        input_analysis_id: Some(input.analysis.id.clone()),
        input_hash: input.analysis.content_hash.clone(),
        profile_snapshot: super::super::super::profile(&p.engine, "standard"),
        parameters: json!({"schemaVersion":1,"request":MinutesJobRequest{meeting_id:input.analysis.meeting_id.clone(),operation_id:operation.into(),expected_analysis_id:input.analysis.id.clone(),expected_content_hash:input.analysis.content_hash.clone(),expected_meeting_lock_version:0,expected_draft:None},"generation":p}),
        total_units: Some(p.plan.nodes.len() as i64),
    };
    let old_spec = spec(&old_input, "original-operation");
    let mut new_spec = spec(&new_input, "mapped-operation");
    new_spec.parameters["generation"]["engine"]["model"]["path"] =
        json!("mon-unavailable:relocated-model");
    let mut c = Checkpoint::default();
    let mut assets = MemoryAssets::default();
    let mut bodies = vec![];
    if !queued {
        for (index, node) in p.plan.nodes.iter().enumerate() {
            let (text, batches) = match node {
                plan::Node::Extract { slices } => {
                    let text = if long {
                        "異なる事項をすべて確認する。\n".repeat(180)
                    } else {
                        EXACT.into()
                    };
                    let batch = assets.batch(&extract_task(&old_input, slices, &p).unwrap(), &text);
                    (text, vec![batch])
                }
                _ => {
                    let mut cursor = cursor(node, &bodies, p.prompt_budget).unwrap();
                    let mut batches = vec![];
                    while cursor.result().is_none() {
                        let text = EXACT.to_string();
                        batches.push(assets.batch(&merge_task(&cursor, &p).unwrap(), &text));
                        cursor.accept(text).unwrap();
                        if pending {
                            assert!(cursor.result().is_none());
                            c.pending = Some(Output {
                                index,
                                batches: std::mem::take(&mut batches),
                            });
                            break;
                        }
                    }
                    if pending {
                        break;
                    }
                    (cursor.result().unwrap().to_string(), batches)
                }
            };
            bodies.push(text);
            c.outputs.push(Output { index, batches });
        }
        if c.outputs.len() == p.plan.nodes.len() {
            c.markdown_hash = Some(content_hash(bodies.last().unwrap()));
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
        completed_units: c.outputs.len() as i64,
        total_units: spec.total_units,
        error_code: None,
        created_at: 1,
        updated_at: 1,
        output_analysis_id: None,
        output_minutes_id: None,
        pause_reason: Some("user".into()),
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
        value: serde_json::to_value(c).unwrap(),
        assets,
    }
}

#[test]
fn markdown_restore_keeps_exact_text_intervals_and_four_files_without_models_or_evidence_mapping() {
    for (long, pending, queued) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (false, false, true),
    ] {
        let mut f = fixture(long, pending, queued);
        let original = f.value.clone();
        let restored = f.run(&|| false).unwrap();
        assert_eq!(restored["markdownHash"], original["markdownHash"]);
        if !queued {
            assert!(f
                .assets
                .target
                .values()
                .any(|b| super::super::super::llama::parse_output(b)
                    .ok()
                    .is_some_and(|v| v["markdown"] == EXACT)));
        }
        for (id, (_, _, bytes)) in &f.assets.source {
            assert_eq!(&f.assets.target[&format!("new-{id}")], bytes);
        }
        assert_eq!(f.run(&|| false).unwrap(), restored);
        assert_eq!(f.value, original);
        let mapped = crate::generation::restore_checkpoint_references(
            &f.new_job,
            f.new_spec.parameters["generation"].clone(),
            original,
            |role, id, hash| {
                let bytes = f.assets.read(role, id, hash)?;
                Ok((format!("new-{id}"), format!("{:x}", Sha256::digest(bytes))))
            },
        )
        .unwrap();
        assert_eq!(mapped, restored);
    }
}
#[test]
fn markdown_restore_rejects_changed_fixed_input_target_progress_and_strict_output_before_writing() {
    for case in 0..13 {
        let mut f = fixture(false, false, false);
        match case {
            0 => f.new_input.analysis.utterances[0].effective_text.push('変'),
            1 => f.new_input.analysis.speakers[0].display_name = Some("別名".into()),
            2 => f.new_input.analysis.utterances[0].end_us += 1,
            3 => {
                f.new_spec.parameters["generation"]["markdownTarget"]["expected"]["contentHash"] =
                    json!("d".repeat(64))
            }
            4 => f.new_spec.parameters["generation"]["engine"]["seed"] = json!(2),
            5 => f.new_job.completed_units = 0,
            6 => f.replace(0, b"changed system".to_vec()),
            7 => f.replace(1, b"{}".to_vec()),
            8 => f.replace(2, b"{}".to_vec()),
            9 => f.replace(3, br#"{"items":[]}"#.to_vec()),
            10 => f.replace(3, br#"{"markdown":"first","markdown":"second"}"#.to_vec()),
            11 => f.value["markdownHash"] = json!("e".repeat(64)),
            12 => {
                f.new_spec.parameters["generation"]["markdownTarget"]["expected"]["lockVersion"] =
                    json!(-1)
            }
            _ => unreachable!(),
        }
        assert!(f.run(&|| false).is_err(), "accepted changed case {case}");
        assert!(f.assets.target.is_empty());
    }
}
#[test]
fn markdown_restore_cancellation_and_invalid_artifact_sets_never_publish_dummy_units() {
    let mut f = fixture(true, true, false);
    assert!(f.run(&|| true).is_err());
    assert!(f.assets.target.is_empty());
    let checks = std::cell::Cell::new(0);
    assert!(f
        .run(&|| {
            checks.set(checks.get() + 1);
            checks.get() > 4
        })
        .is_err());
    assert!(f.assets.target.is_empty());
    for case in 0..4 {
        let mut f = fixture(false, false, false);
        match case {
            0 => {
                f.value["outputs"][0]["batches"][0]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            1 => f.value["outputs"][0]["batches"][0][3]["name"] = json!("items.json"),
            2 => f.value["outputs"][0]["batches"][0]
                .as_array_mut()
                .unwrap()
                .swap(0, 1),
            3 => f.value["outputs"][0]["index"] = json!(100),
            _ => unreachable!(),
        }
        assert!(f.run(&|| false).is_err());
        assert!(crate::generation::restore_checkpoint_references(
            &f.old_job,
            f.old_spec.parameters["generation"].clone(),
            f.value,
            |_, _, _| Ok(("new".into(), "a".repeat(64)))
        )
        .is_err());
    }
}
