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
    fixture_version(
        long,
        pending,
        queued,
        markdown::PromptVersion::CustomPromptsV5,
    )
}
fn fixture_version(
    long: bool,
    pending: bool,
    queued: bool,
    version: markdown::PromptVersion,
) -> Fixture {
    let mut old_input = crate::storage::current_minutes_fixture::input();
    if long {
        old_input.analysis.utterances[0].effective_text = "点検日について確認する。".repeat(1500);
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
    let capacity = crate::generation::model_budget::Capacity::synthetic(&engine, 32768, 8192);
    let prompt_budget = if version == markdown::PromptVersion::CustomPromptsV5 {
        5400
    } else {
        2400
    };
    let limit = plan_limit_for(&capacity, &engine, prompt_budget, version).unwrap();
    let prompt_snapshot =
        (version == markdown::PromptVersion::CustomPromptsV5).then(|| PromptSnapshot {
            settings_lock_version: 12,
            preferences: crate::storage::minutes_prompt_preferences::MinutesPromptPreferences {
                schema_version: 1,
                extract_prompt: "保存した作成指示".into(),
                merge_prompt: "保存した統合指示".into(),
            },
        });
    let extract_system = prompt_snapshot.as_ref().map_or_else(
        || version.extract_system(),
        |s| s.preferences.extract_prompt.as_str(),
    );
    let plan = plan::build_with_system(&old_input, limit, version, extract_system).unwrap();
    let p = Parameters {
        resident_runtime: Some(serde_json::from_value(json!({"protocol":1,"engine":"llama","name":"mon-llama-worker","bytes":1234,"sha256":"a".repeat(64)})).unwrap()),
        schema_version: crate::current_formats::MINUTES,
        optimization_version: Some(1),
        optimization: None,
        prompt_version: version,
        prompt_snapshot,
        acceleration_policy: None,
        capacity,
        engine,
        prompt_budget,
        plan,
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
                        "異なる事項をすべて確認する。\n".repeat(500)
                    } else {
                        EXACT.into()
                    };
                    let batch = assets.batch(&extract_task(&old_input, slices, &p).unwrap(), &text);
                    (text, vec![batch])
                }
                _ => {
                    let mut cursor = cursor(node, &bodies, plan_limit(&p).unwrap(), &p).unwrap();
                    let mut batches = vec![];
                    while cursor.result().is_none() {
                        let text = EXACT.to_string();
                        batches.push(assets.batch(&merge_task(&cursor).unwrap(), &text));
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
        execution_progress: None,
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
        (false, false, true),
    ] {
        let mut f = fixture(long, pending, queued);
        let original = f.value.clone();
        let restored = f.run(&crate::test_support::never_cancelled).unwrap();
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
        assert_eq!(
            f.run(&crate::test_support::never_cancelled).unwrap(),
            restored
        );
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
fn markdown_restore_preserves_model_capacity_and_schema_even_before_first_output() {
    for source in [false, true] {
        for change in [
            "output", "missing", "null", "version5", "version6", "version8", "model",
        ] {
            let mut f = fixture(false, false, true);
            assert!(f.run(&crate::test_support::never_cancelled).is_ok());
            let spec = if source {
                &mut f.old_spec
            } else {
                &mut f.new_spec
            };
            match change {
                "output" => {
                    spec.parameters["generation"]["capacity"]["maxOutputTokens"] = json!(4096)
                }
                "missing" => {
                    spec.parameters["generation"]
                        .as_object_mut()
                        .unwrap()
                        .remove("capacity");
                }
                "null" => spec.parameters["generation"]["capacity"] = Value::Null,
                "version5" => spec.parameters["generation"]["schemaVersion"] = json!(5),
                "version6" => spec.parameters["generation"]["schemaVersion"] = json!(6),
                "version8" => spec.parameters["generation"]["schemaVersion"] = json!(8),
                "model" => {
                    spec.parameters["generation"]["capacity"]["modelSha256"] = json!("d".repeat(64))
                }
                _ => unreachable!(),
            }
            assert!(f.run(&crate::test_support::never_cancelled).is_err());
            assert!(f.assets.target.is_empty());
        }
    }
}
#[test]
fn markdown_restore_rejects_changed_fixed_input_target_progress_and_strict_output_before_writing() {
    for case in 0..15 {
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
            13 => {
                f.new_spec.parameters["generation"]["promptSnapshot"]["preferences"]
                    ["extractPrompt"] = json!("別の設定")
            }
            14 => {
                f.new_spec.parameters["generation"]["promptSnapshot"]["preferences"]
                    ["mergePrompt"] = json!("別の設定")
            }
            _ => unreachable!(),
        }
        assert!(
            f.run(&crate::test_support::never_cancelled).is_err(),
            "accepted changed case {case}"
        );
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
        assert!(f.run(&crate::test_support::never_cancelled).is_err());
        assert!(crate::generation::restore_checkpoint_references(
            &f.old_job,
            f.old_spec.parameters["generation"].clone(),
            f.value,
            |_, _, _| Ok(("new".into(), "a".repeat(64)))
        )
        .is_err());
    }
}

#[test]
fn prompt_version_is_fixed_across_restore_and_old_revisions_are_rejected() {
    let mut f = fixture(false, false, false);
    assert!(f.run(&|| false).is_ok());
    for version in [
        "legacy",
        "inlineEvidenceV1",
        "inlineEvidenceV2",
        "compactEvidenceV3",
        "conciseSummaryV4",
        "futureVersion",
    ] {
        f.new_spec.parameters["generation"]["promptVersion"] = json!(version);
        assert!(f.run(&|| false).is_err());
    }
}

#[test]
fn restoring_format_mismatched_minutes_keeps_exact_output_without_treating_format_as_corruption() {
    let mut f = fixture_version(
        false,
        false,
        false,
        markdown::PromptVersion::CustomPromptsV5,
    );
    let text = format!(
        "# 議事録\n\n## 要約\n{}",
        "根拠リンクのない生成本文。".repeat(160)
    );
    f.replace(3, serde_json::to_vec(&json!({"markdown":text})).unwrap());
    f.value["markdownHash"] = json!(content_hash(&text));
    let result = f.run(&crate::test_support::never_cancelled).unwrap();
    assert_eq!(result["markdownHash"], content_hash(&text));
    assert!(f
        .assets
        .target
        .values()
        .any(|bytes| super::super::super::llama::parse_output(bytes)
            .ok()
            .is_some_and(|v| v["markdown"] == text)));
}

fn acceleration_fixture() -> Fixture {
    let mut f = fixture_version(
        false,
        false,
        false,
        markdown::PromptVersion::CustomPromptsV5,
    );
    for spec in [&mut f.old_spec, &mut f.new_spec] {
        let mut p: Parameters =
            serde_json::from_value(spec.parameters["generation"].clone()).unwrap();
        p.engine.executable.sha256 =
            "e3d5205729cb4c1e176d5c8e39796aefbe361a2300d0593586db9503dd0d63d1".into();
        p.engine.executable.size = 33472;
        p.engine.model.sha256 =
            "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785".into();
        p.engine.model.size = 5_027_783_488;
        let mut capacity = serde_json::to_value(&p.capacity).unwrap();
        capacity["modelSha256"] = json!(p.engine.model.sha256);
        p.capacity = serde_json::from_value(capacity).unwrap();
        p.acceleration_policy = Some(super::super::super::acceleration::Policy::AutoMetalV2);
        spec.profile_snapshot = super::super::super::profile(&p.engine, "standard");
        spec.parameters["generation"] = serde_json::to_value(p).unwrap();
    }
    f.value["acceleration"] = json!({"backend":"cpu","reason":"trialFailed"});
    f
}
#[test]
fn acceleration_choice_survives_portable_replay_and_cannot_be_removed_or_added_to_old_jobs() {
    let mut f = acceleration_fixture();
    let result = f.run(&crate::test_support::never_cancelled).unwrap();
    assert_eq!(result["acceleration"], f.value["acceleration"]);
    let mut missing = acceleration_fixture();
    missing
        .value
        .as_object_mut()
        .unwrap()
        .remove("acceleration");
    assert!(missing.run(&crate::test_support::never_cancelled).is_err());
    assert!(missing.assets.target.is_empty());
    let mut old = fixture_version(
        false,
        false,
        false,
        markdown::PromptVersion::CustomPromptsV5,
    );
    old.value["acceleration"] = json!({"backend":"cpu","reason":"trialFailed"});
    assert!(old.run(&crate::test_support::never_cancelled).is_err());
    let mut changed = acceleration_fixture();
    changed.new_spec.parameters["generation"]
        .as_object_mut()
        .unwrap()
        .remove("accelerationPolicy");
    assert!(changed.run(&crate::test_support::never_cancelled).is_err());
}

#[test]
fn metal_proof_restores_as_historical_bytes_and_rejects_changed_gpu_evidence() {
    let mut f = acceleration_fixture();
    let p: Parameters =
        serde_json::from_value(f.old_spec.parameters["generation"].clone()).unwrap();
    f.value["acceleration"] = json!({"backend":"metal","proof":{"schemaVersion":2,"engineSha256":p.engine.executable.sha256,"modelSha256":p.engine.model.sha256,"contextTokens":acceleration_context(&p).unwrap(),"gpuLayers":37,"device":"MTL0","result":"{\"ok\":true}","log":"llama_prepare_model_devices: using device MTL0 (Apple M1 Max)\nload_tensors: offloaded 37/37 layers to GPU\nhistorical model path: /old-machine/model.gguf"}});
    let proof = f.value["acceleration"].clone();
    assert_eq!(
        f.run(&crate::test_support::never_cancelled).unwrap()["acceleration"],
        proof
    );
    let mut corrupt = acceleration_fixture();
    corrupt.value["acceleration"] = proof;
    corrupt.value["acceleration"]["proof"]["log"] =
        json!("using device MTL0\nload_tensors: offloaded 0/37 layers to GPU");
    assert!(corrupt.run(&crate::test_support::never_cancelled).is_err());
    assert!(corrupt.assets.target.is_empty());
}

fn automatic_acceleration_fixture() -> Fixture {
    let mut f = acceleration_fixture();
    for spec in [&mut f.old_spec, &mut f.new_spec] {
        let mut p: Parameters =
            serde_json::from_value(spec.parameters["generation"].clone()).unwrap();
        p.engine.model.sha256 =
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5".into();
        p.engine.model.size = 2_497_280_256;
        let mut capacity = serde_json::to_value(&p.capacity).unwrap();
        capacity["modelSha256"] = json!(p.engine.model.sha256);
        p.capacity = serde_json::from_value(capacity).unwrap();
        p.acceleration_policy = Some(super::super::super::acceleration::Policy::AutoMetalV2);
        spec.profile_snapshot = super::super::super::profile(&p.engine, "standard");
        spec.parameters["generation"] = serde_json::to_value(p).unwrap();
    }
    f
}
#[test]
fn automatic_gpu_for_a_different_model_keeps_actual_layers_and_historical_proof_after_restore() {
    let mut f = automatic_acceleration_fixture();
    let p: Parameters =
        serde_json::from_value(f.old_spec.parameters["generation"].clone()).unwrap();
    f.value["acceleration"] = json!({"backend":"metal","proof":{"schemaVersion":2,"engineSha256":p.engine.executable.sha256,"modelSha256":p.engine.model.sha256,"contextTokens":acceleration_context(&p).unwrap(),"gpuLayers":16,"device":"MTL0","result":"{\"ok\":true}","log":"using device MTL0 (Apple GPU)\nload_tensors: offloaded 16/37 layers to GPU\nhistorical path: /old-machine/model.gguf"}});
    let pinned = f.value["acceleration"].clone();
    assert_eq!(
        f.run(&crate::test_support::never_cancelled).unwrap()["acceleration"],
        pinned
    );
    let mut corrupt = automatic_acceleration_fixture();
    corrupt.value["acceleration"] = pinned.clone();
    corrupt.value["acceleration"]["proof"]["modelSha256"] = json!("a".repeat(64));
    assert!(corrupt.run(&crate::test_support::never_cancelled).is_err());
    assert!(corrupt.assets.target.is_empty());
    let mut corrupt = automatic_acceleration_fixture();
    corrupt.value["acceleration"] = pinned;
    corrupt.value["acceleration"]["proof"]["gpuLayers"] = json!(32);
    assert!(corrupt.run(&crate::test_support::never_cancelled).is_err());
    assert!(corrupt.assets.target.is_empty());
}
#[test]
fn automatic_cpu_choice_survives_restore_but_the_pinned_policy_cannot_change() {
    let mut f = automatic_acceleration_fixture();
    assert_eq!(
        f.run(&crate::test_support::never_cancelled).unwrap()["acceleration"],
        f.value["acceleration"]
    );
    let mut f = acceleration_fixture();
    f.new_spec.parameters["generation"]["accelerationPolicy"] = json!("metalTrialV1");
    assert!(f.run(&crate::test_support::never_cancelled).is_err());
    assert!(f.assets.target.is_empty());
}

#[test]
fn resident_markdown_restore_preserves_worker_identity_and_rejects_substitution() {
    let mut f = fixture(false, false, false);
    let runtime = json!({"protocol":1,"engine":"llama","name":"mon-llama-worker","bytes":1234,"sha256":"a".repeat(64)});
    f.old_spec.parameters["generation"]["residentRuntime"] = runtime.clone();
    f.new_spec.parameters["generation"]["residentRuntime"] = runtime.clone();
    let before = f.value.clone();
    assert!(f.run(&|| false).is_ok());
    assert_eq!(
        f.old_spec.parameters["generation"]["residentRuntime"],
        runtime
    );
    assert_eq!(
        f.new_spec.parameters["generation"]["residentRuntime"],
        runtime
    );
    assert_eq!(f.value, before);
    f.new_spec.parameters["generation"]["residentRuntime"]["sha256"] = json!("b".repeat(64));
    assert!(f.run(&|| false).is_err());
}

#[test]
fn current_restore_retains_optimization_policy_without_loading_models() {
    let mut f = fixture(false, false, false);
    let runtime = json!({"protocol":1,"engine":"llama","name":"mon-llama-worker","bytes":1234,"sha256":"a".repeat(64)});
    let policy =
        json!({"version":1,"tokenPacking":false,"prefixReuse":true,"evidenceHash":"b".repeat(64)});
    for spec in [&mut f.old_spec, &mut f.new_spec] {
        let p = &mut spec.parameters["generation"];
        p["schemaVersion"] = json!(9);
        p["optimizationVersion"] = json!(1);
        p["optimization"] = policy.clone();
        p["residentRuntime"] = runtime.clone();
    }
    assert!(f.run(&|| false).is_ok());
    f.new_spec.parameters["generation"]["optimization"]["prefixReuse"] = json!(false);
    assert!(f.run(&|| false).is_err());
    f.new_spec.parameters["generation"]["optimization"] = policy;
    f.value["tokenCounts"] = json!({"bad-key":1});
    assert!(f.run(&|| false).is_err());
}
