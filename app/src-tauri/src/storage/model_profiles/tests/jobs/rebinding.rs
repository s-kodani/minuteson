use super::*;
use managed::rebinding::{self, Binding};

fn prepare(f: &Fixture, spec: &JobSpec, target: usize) -> Result<Binding, String> {
    let p = &f.packages[target];
    rebinding::prepare(
        &f.storage,
        &f.models(),
        spec,
        &p.id,
        &p.record.review_hash,
        &p.check,
        &|| false,
    )
}
fn check_execution<T>(spec: &JobSpec, run: impl FnOnce(Execution<'_>) -> T) -> T {
    match spec.kind {
        JobKind::Transcription => {
            let e: WhisperConfig =
                serde_json::from_value(spec.parameters["engine"].clone()).unwrap();
            run(Execution::Whisper(&e))
        }
        JobKind::Diarization => {
            let e: SherpaConfig =
                serde_json::from_value(spec.parameters["diarization"]["engine"].clone()).unwrap();
            run(Execution::Sherpa(&e))
        }
        JobKind::MinutesGeneration => {
            let e: LlamaConfig =
                serde_json::from_value(spec.parameters["generation"]["engine"].clone()).unwrap();
            run(Execution::Llama(
                &e,
                spec.parameters["generation"]["promptBudget"]
                    .as_u64()
                    .unwrap() as usize,
            ))
        }
        _ => unreachable!(),
    }
}
fn value(binding: &Binding) -> serde_json::Value {
    serde_json::to_value(binding).unwrap()
}
fn decode(value: serde_json::Value) -> Binding {
    serde_json::from_value(value).unwrap()
}
fn portable_spec(spec: &JobSpec) -> JobSpec {
    let mut spec = spec.clone();
    let mut refs = BTreeSet::new();
    let bytes = serde_json::to_vec(&spec.parameters).unwrap();
    spec.parameters = serde_json::from_slice(
        &crate::storage::backup::portable_json_for_model_test(&bytes, &mut refs).unwrap(),
    )
    .unwrap();
    spec
}

#[test]
fn model_rebinding_three_engines_resolve_new_identical_packages_without_mutating_fixed_jobs_profiles_or_running_engines(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let specs = jobs
        .iter()
        .map(|j| f.storage.fixed_job_input(&j.id).unwrap())
        .collect::<Vec<_>>();
    let profiles = f.storage.model_profiles().unwrap();
    let before = f.storage.analysis_preferences().unwrap().payload;
    let before_diar = f.storage.diarization_preferences().unwrap().payload;
    let before_minutes = f.storage.minutes_preferences().unwrap().payload;
    let counts = [
        "processing_jobs",
        "job_checkpoints",
        "model_profiles",
        "model_compatibility_checks",
    ]
    .map(|t| f.count(t));
    for (spec, engine) in
        specs
            .iter()
            .map(|s| &s.0)
            .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        let target = f.add(engine, true, false);
        let calls = f.executed(target);
        let origin_calls = f.executed(target - 3);
        // The original installation may no longer exist on a restore target.
        // Keep its non-owning proof, not a dependency on its physical files.
        std::fs::remove_dir_all(f.models().join(&f.packages[target - 3].id)).unwrap();
        check_execution(spec, |e| {
            assert!(managed::verify(&spec.profile_snapshot, e, &|| false).is_err());
        });
        let binding = prepare(&f, spec, target).unwrap();
        let profile = binding.attach(spec).unwrap();
        assert_eq!(
            profile["managedModel"],
            spec.profile_snapshot["managedModel"]
        );
        assert_ne!(
            profile["runtimeModel"]["package"]["packageId"],
            profile["managedModel"]["package"]["packageId"]
        );
        check_execution(spec, |execution| {
            let resolved = binding.resolve(&profile, execution).unwrap();
            binding.verify(&profile, execution, &|| false).unwrap();
            let native = resolved.execution();
            // Configuration and every byte proof stay identical, while every
            // role resolves under the reviewed destination package, even two
            // same-name, same-byte libraries in different subdirectories.
            assert_eq!(
                execution_configuration(execution),
                execution_configuration(native)
            );
            let original = engine_files(execution);
            let actual = engine_files(native);
            assert_eq!(original.len(), actual.len());
            for (old, new) in original.iter().zip(actual) {
                assert_eq!((&old.sha256, old.size), (&new.sha256, new.size));
                assert_ne!(old.path, new.path);
                assert!(new
                    .path
                    .starts_with(f.models().join(&f.packages[target].id)));
                new.verify(&|| false).unwrap();
            }
        });
        let restored = portable_spec(spec);
        check_execution(&restored, |e| {
            binding.resolve(&restored.profile_snapshot, e).unwrap()
        });
        assert_eq!(f.executed(target), calls);
        assert_eq!(f.executed(target - 3), origin_calls);
    }
    for (job, original) in jobs.iter().zip(specs) {
        assert_eq!(f.storage.fixed_job_input(&job.id).unwrap(), original);
        assert_eq!(f.storage.get_job(&job.id).unwrap(), *job);
    }
    assert_eq!(
        serde_json::to_value(f.storage.model_profiles().unwrap()).unwrap(),
        serde_json::to_value(profiles).unwrap()
    );
    assert_eq!(f.storage.analysis_preferences().unwrap().payload, before);
    assert_eq!(
        f.storage.diarization_preferences().unwrap().payload,
        before_diar
    );
    assert_eq!(
        f.storage.minutes_preferences().unwrap().payload,
        before_minutes
    );
    for (i, table) in ["processing_jobs", "job_checkpoints", "model_profiles"]
        .iter()
        .enumerate()
    {
        assert_eq!(f.count(table), counts[i]);
    }
    assert_eq!(f.count("model_compatibility_checks"), counts[3] + 3);
}

fn engine_files(e: Execution<'_>) -> Vec<&Fingerprint> {
    match e {
        Execution::Whisper(w) => {
            let mut files = vec![&w.executable, &w.model];
            files.extend(&w.vad_model);
            files
        }
        Execution::Sherpa(s) => {
            let mut files = vec![&s.executable, &s.segmentation_model, &s.embedding_model];
            files.extend(&s.runtime_libraries);
            files
        }
        Execution::Llama(l, _) => vec![&l.executable, &l.model],
    }
}
fn execution_configuration(e: Execution<'_>) -> serde_json::Value {
    let mut value = match e {
        Execution::Whisper(e) => serde_json::to_value(e).unwrap(),
        Execution::Sherpa(e) => serde_json::to_value(e).unwrap(),
        Execution::Llama(e, budget) => json!({"engine":e,"promptBudget":budget}),
    };
    if let Execution::Llama(_, _) = e {
        let o = value["engine"].as_object_mut().unwrap();
        o.remove("executable");
        o.remove("model");
    } else {
        let o = value.as_object_mut().unwrap();
        for field in [
            "executable",
            "model",
            "vadModel",
            "segmentationModel",
            "embeddingModel",
            "runtimeLibraries",
        ] {
            o.remove(field);
        }
    }
    value
}

#[test]
fn model_rebinding_portable_roundtrip_redacts_all_roles_and_unavailable_binding_cannot_resolve_or_verify(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    for (job, engine) in
        enqueue_all(&f)
            .iter()
            .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
        let target = f.add(engine, true, false);
        let binding = prepare(&f, &spec, target).unwrap();
        let profile = binding.attach(&spec).unwrap();
        let mut refs = BTreeSet::new();
        let portable = crate::storage::backup::portable_json_for_model_test(
            &serde_json::to_vec(&profile).unwrap(),
            &mut refs,
        )
        .unwrap();
        assert!(!String::from_utf8(portable.clone())
            .unwrap()
            .contains(f.app_data.path().to_str().unwrap()));
        let profile: serde_json::Value = serde_json::from_slice(&portable).unwrap();
        let unavailable = rebinding::read(&profile).unwrap().unwrap();
        let restored = portable_spec(&spec);
        check_execution(&restored, |e| {
            unavailable.validate(&profile, e).unwrap();
            managed::validate(&profile, e).unwrap();
            assert!(unavailable.resolve(&profile, e).is_err());
            assert!(unavailable.verify(&profile, e, &|| false).is_err());
        });
        assert!(refs
            .iter()
            .any(|r| r.role == "executable" && r.name == "engine-cli"));
        assert!(refs
            .iter()
            .any(|r| r.role == "artifact" && r.name.ends_with("dylib")));
        if engine == Engine::Whisper {
            assert!(refs
                .iter()
                .any(|r| r.role == "vadModel" && r.name == "vad.bin"));
        }
        let mut second_refs = BTreeSet::new();
        let next =
            crate::storage::backup::portable_json_for_model_test(&portable, &mut second_refs)
                .unwrap();
        assert_eq!(portable, next);
        assert_eq!(refs, second_refs);
        // Decoding history on another supported host is pure, not permission
        // to execute a foreign target. Keep matching original provenance.
        let mut foreign = profile.clone();
        for key in ["managedModel", "runtimeModel"] {
            foreign[key]["package"]["manifest"]["target"] = json!("windows_x86_64");
            let package: PlacementRecord =
                serde_json::from_value(foreign[key]["package"].clone()).unwrap();
            let review = crate::model_inspection::review_hash(
                &package.manifest,
                &package.source_manifest_hash,
                &package.files,
            )
            .unwrap();
            foreign[key]["package"]["reviewHash"] = json!(review);
            foreign[key]["compatibility"]["reviewHash"] = json!(review);
        }
        check_execution(&restored, |e| managed::validate(&foreign, e).unwrap());
    }
}

#[test]
fn model_rebinding_missing_failed_stale_trials_changed_bytes_artifacts_and_cancelled_confirmation_are_read_only_failures(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let (spec, cp) = f.storage.fixed_job_input(&jobs[0].id).unwrap();
    let pending = f.add(Engine::Whisper, false, false);
    assert!(prepare(&f, &spec, pending).is_err());
    let failed = f.add(Engine::Whisper, true, true);
    assert!(prepare(&f, &spec, failed).is_err());
    let target = f.add(Engine::Whisper, true, false);
    let binding = prepare(&f, &spec, target).unwrap();
    let package = &f.packages[target];
    let calls = f.executed(target);
    assert!(rebinding::prepare(
        &f.storage,
        &f.models(),
        &spec,
        &package.id,
        &package.record.review_hash,
        &package.check,
        &|| true
    )
    .is_err());
    let new_check = Uuid::new_v4().to_string();
    f.storage
        .begin_model_check(&new_check, &package.record)
        .unwrap();
    f.storage
        .finish_model_check(
            &new_check,
            &package.record,
            None,
            CheckState::Failed,
            Some("fixture failure"),
        )
        .unwrap();
    assert!(prepare(&f, &spec, target).is_err());
    // A frozen valid binding is independent of later trial selection, but
    // creating a new one always requires the latest explicit success.
    check_execution(&spec, |e| {
        binding
            .verify(&spec.profile_snapshot, e, &|| false)
            .unwrap()
    });
    assert_eq!(f.executed(target), calls);
    for file in [
        "model.bin",
        "vad.bin",
        "lib/whisper-runtime.dylib",
        "engine-cli",
        "mon-model.json",
        "trial",
    ] {
        let target = f.add(Engine::Whisper, true, false);
        let calls = f.executed(target);
        let path = if file == "trial" {
            f.app_data
                .path()
                .join("model-checks")
                .join(&f.packages[target].check)
                .join("stderr.log")
        } else {
            f.models().join(&f.packages[target].id).join(file)
        };
        std::fs::write(path, b"mutated after CPU success").unwrap();
        assert!(prepare(&f, &spec, target).is_err(), "{file}");
        assert_eq!(f.executed(target), calls);
    }
    assert_eq!(f.storage.fixed_job_input(&jobs[0].id).unwrap(), (spec, cp));
    assert_eq!(f.storage.get_job(&jobs[0].id).unwrap(), jobs[0]);
}

#[test]
fn model_rebinding_pure_validation_rejects_mixed_roots_forged_sizes_order_missing_library_and_different_origin_metadata(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let (spec, _) = f.storage.fixed_job_input(&jobs[1].id).unwrap();
    let target = f.add(Engine::Sherpa, true, false);
    let binding = prepare(&f, &spec, target).unwrap();
    let original = value(&binding);
    for case in 0..7 {
        let mut bad = original.clone();
        match case {
            0 => bad["files"][0]["size"] = json!(1),
            1 => bad["files"][1]["sha256"] = json!("a".repeat(64)),
            2 => {
                bad["files"][0]["path"] = json!(format!(
                    "mon-unavailable:{}",
                    bad["files"][0]["sha256"].as_str().unwrap()
                ))
            }
            3 => {
                bad["files"][1]["path"] =
                    json!(f.models().join(Uuid::new_v4().to_string()).join("seg.onnx"))
            }
            4 => {
                bad["files"].as_array_mut().unwrap().pop();
            }
            5 => bad["files"][3]["path"] = bad["files"][4]["path"].clone(),
            6 => bad["schemaVersion"] = json!(2),
            _ => unreachable!(),
        }
        check_execution(&spec, |e| {
            assert!(
                decode(bad).validate(&spec.profile_snapshot, e).is_err(),
                "{case}"
            )
        });
    }
    for field in ["modelVersion", "engineVersion", "licenseRef"] {
        let mut changed = spec.profile_snapshot.clone();
        changed["managedModel"]["package"]["manifest"][field] =
            json!("different historical declaration");
        let package: PlacementRecord =
            serde_json::from_value(changed["managedModel"]["package"].clone()).unwrap();
        let review = crate::model_inspection::review_hash(
            &package.manifest,
            &package.source_manifest_hash,
            &package.files,
        )
        .unwrap();
        changed["managedModel"]["package"]["reviewHash"] = json!(review);
        changed["managedModel"]["compatibility"]["reviewHash"] = json!(review);
        if field == "engineVersion" {
            changed["managedModel"]["compatibility"]["declaredEngineVersion"] =
                json!("different historical declaration");
        }
        check_execution(&spec, |e| {
            assert!(binding.validate(&changed, e).is_err(), "{field}")
        });
    }
    let profile = binding.attach(&spec).unwrap();
    let mut injected = spec.clone();
    injected.operation_id = "no-implicit-rebind".into();
    injected.profile_snapshot = profile;
    let before = f.count("processing_jobs");
    assert!(f.storage.enqueue_job(injected).is_err());
    assert_eq!(f.count("processing_jobs"), before);
    let mut nullable = spec.profile_snapshot.clone();
    nullable["runtimeModel"] = serde_json::Value::Null;
    check_execution(&spec, |e| assert!(managed::validate(&nullable, e).is_err()));
}

#[test]
fn model_rebinding_legacy_jobs_cannot_add_unrecorded_vad_or_runtime_dependencies() {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    for (job, engine) in jobs
        .iter()
        .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
        let target = f.add(engine, true, false);
        let binding = prepare(&f, &spec, target).unwrap();
        let mut legacy = portable_spec(&spec);
        legacy
            .profile_snapshot
            .as_object_mut()
            .unwrap()
            .remove("managedModel");
        let result = prepare(&f, &legacy, target);
        if engine == Engine::Sherpa {
            let accepted = result.unwrap();
            check_execution(&legacy, |e| {
                accepted.resolve(&legacy.profile_snapshot, e).unwrap();
            });
            legacy.parameters["diarization"]["engine"]["runtimeLibraries"] = json!([]);
            assert!(prepare(&f, &legacy, target).is_err());
        } else {
            assert!(
                result.is_err(),
                "legacy CLI config cannot prove undeclared libraries"
            );
            check_execution(&legacy, |e| {
                assert!(binding.resolve(&legacy.profile_snapshot, e).is_err())
            });
        }
    }
}
