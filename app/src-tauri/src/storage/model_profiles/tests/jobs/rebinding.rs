use super::*;
use managed::rebinding::{self, Binding};

fn retry_preview(
    f: &Fixture,
    job: &JobSnapshot,
    target: usize,
) -> Result<crate::test_support::model_retry::Preview, String> {
    let p = &f.packages[target];
    crate::test_support::model_retry::preview(
        &f.storage,
        &f.models(),
        &f.meeting,
        &job.id,
        &job.spec_hash,
        &p.id,
        &p.record.review_hash,
        &p.check,
        &crate::test_support::never_cancelled,
    )
}
fn calls_in(path: &Path) -> usize {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                calls_in(&entry.path())
            } else if entry.file_name() == "executed.marker" {
                std::fs::read(entry.path()).unwrap().len() / 4
            } else {
                0
            }
        })
        .sum()
}

#[test]
fn model_rebinding_reviewed_retries_run_all_three_workers_and_receipts_do_not_recheck_missing_models(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f)
        .into_iter()
        .map(|j| {
            f.storage
                .finish_job(&j.id, j.completed_units, "cancelled", None)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let preferences = (
        f.storage.analysis_preferences().unwrap().payload,
        f.storage.diarization_preferences().unwrap().payload,
        f.storage.minutes_preferences().unwrap().payload,
    );
    for (source, engine) in jobs
        .iter()
        .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        let original = f.storage.fixed_job_input(&source.id).unwrap();
        let target = f.add(engine, true, false);
        std::fs::remove_dir_all(f.models().join(&f.packages[target - 3].id)).unwrap();
        let calls = calls_in(f.root.path()) + calls_in(f.app_data.path());
        let preview = retry_preview(&f, source, target).unwrap();
        assert_eq!(calls_in(f.root.path()) + calls_in(f.app_data.path()), calls);
        let op = Uuid::new_v4().to_string();
        let inspector = crate::model_inspection::Inspector::default();
        let cancelled_op = Uuid::new_v4().to_string();
        let guard = inspector.begin(&cancelled_op).unwrap();
        inspector.cancel(&cancelled_op).unwrap();
        let pending = crate::test_support::model_retry::receipt(
            &f.storage,
            &inspector,
            &cancelled_op,
            &preview,
        )
        .unwrap();
        assert!(pending.active && pending.cancelled && pending.job.is_none());
        drop(guard);
        let ended = crate::test_support::model_retry::receipt(
            &f.storage,
            &inspector,
            &cancelled_op,
            &preview,
        )
        .unwrap();
        assert!(!ended.active && ended.cancelled && ended.job.is_none());
        let next = crate::test_support::model_retry::apply(
            &f.storage,
            &f.models(),
            &op,
            &preview,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let (spec, cp) = f.storage.fixed_job_input(&next.id).unwrap();
        assert_eq!(spec.parameters, original.0.parameters);
        assert_eq!(
            rebinding::base_profile(&spec.profile_snapshot).unwrap(),
            original.0.profile_snapshot
        );
        assert_eq!(cp, original.1);
        assert_eq!(next.retry_of_job_id.as_deref(), Some(source.id.as_str()));
        assert_eq!(next.attempt, 2);
        assert_eq!(calls_in(f.root.path()) + calls_in(f.app_data.path()), calls);
        assert_eq!(
            f.storage.acquire_local_inference_job().unwrap().unwrap().id,
            next.id
        );
        let candidate = match engine {
            Engine::Whisper | Engine::Asr => crate::transcription::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .map_err(|e| e.to_string()),
            Engine::Sherpa => crate::diarization::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            ),
            Engine::Llama => crate::generation::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            ),
        }
        .unwrap();
        assert!(candidate.is_some());
        let completed = f.storage.get_job(&next.id).unwrap();
        assert_eq!(completed.state, "succeeded");
        assert!(calls_in(f.root.path()) + calls_in(f.app_data.path()) > calls);
        assert_eq!(
            f.storage.fixed_job_initial_operation(&next.id).unwrap(),
            source.operation_id
        );
        assert_eq!(f.storage.get_job(&source.id).unwrap(), *source);
        assert_eq!(f.storage.fixed_job_input(&source.id).unwrap(), original);
        // Resolving a lost reply is independent of native files and cannot
        // start a new attempt, even after the destination has disappeared.
        std::fs::remove_dir_all(f.models().join(&f.packages[target].id)).unwrap();
        let counts = (f.count("processing_jobs"), f.count("job_checkpoints"));
        inspector.cancel(&op).unwrap();
        let receipt =
            crate::test_support::model_retry::receipt(&f.storage, &inspector, &op, &preview)
                .unwrap();
        assert!(!receipt.active && receipt.cancelled);
        assert_eq!(receipt.job, Some(completed.clone()));
        assert_eq!(
            crate::test_support::model_retry::result(&f.storage, &op, &preview).unwrap(),
            Some(completed.clone())
        );
        assert_eq!(
            crate::test_support::model_retry::apply(
                &f.storage,
                &f.models(),
                &op,
                &preview,
                &|| true
            )
            .unwrap(),
            completed
        );
        assert_eq!(
            (f.count("processing_jobs"), f.count("job_checkpoints")),
            counts
        );
        assert!(crate::test_support::model_retry::apply(
            &f.storage,
            &f.models(),
            &Uuid::new_v4().to_string(),
            &preview,
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    assert_eq!(
        (
            f.storage.analysis_preferences().unwrap().payload,
            f.storage.diarization_preferences().unwrap().payload,
            f.storage.minutes_preferences().unwrap().payload
        ),
        preferences
    );
}

#[test]
fn model_rebinding_rejects_changed_review_state_cancelled_or_stale_trial_without_enqueuing() {
    for mutation in ["review", "state", "cancel", "trial", "model", "audio"] {
        let mut f = Fixture::new();
        ready(&mut f);
        let job = enqueue_all(&f).remove(0);
        let source = f
            .storage
            .finish_job(&job.id, job.completed_units, "cancelled", None)
            .unwrap();
        let target = f.add(Engine::Whisper, true, false);
        let mut preview = retry_preview(&f, &source, target).unwrap();
        match mutation {
            "review" => {
                let mut v = serde_json::to_value(&preview).unwrap();
                v["source"]["completedUnits"] = json!(1);
                preview = serde_json::from_value(v).unwrap();
            }
            "state" => {
                f.storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE processing_jobs SET updated_at=updated_at+1 WHERE id=?1",
                        params![source.id],
                    )
                    .unwrap();
            }
            "trial" => {
                crate::model_compatibility::check(
                    &f.storage,
                    &f.models(),
                    &Uuid::new_v4().to_string(),
                    &f.packages[target].id,
                    &f.packages[target].record.review_hash,
                    &crate::test_support::never_cancelled,
                )
                .unwrap();
            }
            "model" => {
                std::fs::write(
                    f.models().join(&f.packages[target].id).join("model.bin"),
                    b"changed",
                )
                .unwrap();
            }
            "audio" => {
                std::fs::remove_file({
                    let plan = f.storage.recorded_audio_plan(&f.meeting).unwrap();
                    f.storage
                        .meeting_mix_path(&f.meeting, &plan.tracks[0])
                        .unwrap()
                })
                .unwrap();
            }
            _ => {}
        }
        let jobs = f.count("processing_jobs");
        let cps = f.count("job_checkpoints");
        let calls = calls_in(f.root.path()) + calls_in(f.app_data.path());
        assert!(
            crate::test_support::model_retry::apply(
                &f.storage,
                &f.models(),
                &Uuid::new_v4().to_string(),
                &preview,
                &|| mutation == "cancel"
            )
            .is_err(),
            "{mutation}"
        );
        assert_eq!(
            (
                f.count("processing_jobs"),
                f.count("job_checkpoints"),
                calls_in(f.root.path()) + calls_in(f.app_data.path())
            ),
            (jobs, cps, calls)
        );
    }
}

#[test]
fn model_rebinding_atomic_admission_rolls_back_when_cancelled_after_inserting_retry_and_checkpoint()
{
    let mut f = Fixture::new();
    ready(&mut f);
    let job = enqueue_all(&f).remove(0);
    assert_eq!(
        f.storage.acquire_local_inference_job().unwrap().unwrap().id,
        job.id
    );
    let empty = json!({"outputs":[]});
    f.storage
        .save_job_checkpoint(&job.id, 0, 0, empty.clone())
        .unwrap();
    f.storage.request_job_pause(&job.id, "user").unwrap();
    f.storage.save_job_checkpoint(&job.id, 0, 0, empty).unwrap();
    let source = f.storage.finish_job(&job.id, 0, "cancelled", None).unwrap();
    let (spec, cp) = f.storage.fixed_job_input(&job.id).unwrap();
    let destination = f.add(Engine::Whisper, true, false);
    let binding = prepare(&f, &spec, destination).unwrap();
    let counts = (f.count("processing_jobs"), f.count("job_checkpoints"));
    let checks = std::cell::Cell::new(0usize);
    let op = Uuid::new_v4().to_string();
    assert!(f
        .storage
        .enqueue_rebound_retry_job(&source, &spec, cp.as_ref(), &op, &binding, &|| {
            let count = checks.get();
            checks.set(count + 1);
            count > 0
        })
        .is_err());
    assert_eq!(checks.get(), 2);
    assert_eq!(
        (f.count("processing_jobs"), f.count("job_checkpoints")),
        counts
    );
    assert!(f
        .storage
        .rebound_retry_result(&source, &spec, cp.as_ref(), &op, &binding)
        .unwrap()
        .is_none());
    assert_eq!(f.storage.get_job(&source.id).unwrap(), source);
}

#[test]
fn model_rebinding_restored_partial_history_rechecks_omitted_pcm_and_logs_and_runs_only_unfinished_units(
) {
    let mut f = Fixture::with_tracks(124, true);
    ready(&mut f);
    // Audio duration alone does not make a short transcript require multiple
    // Markdown units. Preserve the draft as actual worker input and force
    // more than one extraction before testing durable partial continuation.
    let analysis = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    let mut draft = f
        .storage
        .create_transcript_draft(&f.meeting, &analysis.id)
        .unwrap();
    draft.payload.utterances = analysis
        .utterances
        .iter()
        .map(|utterance| crate::storage::drafts::UtteranceEdit {
            utterance_id: utterance.id.clone(),
            effective_text: "点検日と異なる確認事項を記録します。".repeat(800),
            speaker_ids: utterance.speaker_ids.clone(),
        })
        .collect();
    f.storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let jobs = enqueue_all(&f);
    assert!(
        jobs[2].total_units.unwrap() > 1,
        "must exercise unfinished Markdown units"
    );
    for (job, engine) in jobs
        .iter()
        .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        assert_eq!(
            f.storage.acquire_local_inference_job().unwrap().unwrap().id,
            job.id
        );
        let progress = |j: JobSnapshot| {
            if j.completed_units == 1 && j.state == "running" {
                f.storage.request_job_pause(&j.id, "user").unwrap();
            }
        };
        let candidate = match engine {
            Engine::Whisper | Engine::Asr => crate::transcription::process(
                &f.storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &progress,
            )
            .map_err(|e| e.to_string()),
            Engine::Sherpa => crate::diarization::process(
                &f.storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &progress,
            ),
            Engine::Llama => crate::generation::process(
                &f.storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &progress,
            ),
        }
        .unwrap();
        assert!(candidate.is_none());
        assert_eq!(f.storage.get_job(&job.id).unwrap().completed_units, 1);
    }
    let new_root = tempfile::tempdir().unwrap();
    let ids = jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
    let (target_storage, target_meeting, target_jobs) =
        crate::storage::backup::published_restore_for_rebinding_test(
            &f.storage,
            new_root.path(),
            &f.meeting,
            &ids,
        );
    f.storage = target_storage;
    f.meeting = target_meeting;
    f.root = new_root;
    for p in &f.packages {
        std::fs::remove_dir_all(f.models().join(&p.id)).unwrap();
    }
    assert!(f.storage.acquire_local_inference_job().unwrap().is_none());
    assert!(f
        .storage
        .model_profiles()
        .unwrap()
        .iter()
        .all(|p| p.id.is_none()));
    for (id, engine) in target_jobs
        .iter()
        .zip([Engine::Whisper, Engine::Sherpa, Engine::Llama])
    {
        let source = f.storage.get_job(id).unwrap();
        assert_eq!(source.state, "interrupted");
        assert_eq!(
            source.error_code.as_deref(),
            Some("restored_preparation_required")
        );
        let original = f.storage.fixed_job_input(id).unwrap();
        assert_eq!(source.completed_units, 1);
        let cp = original.1.as_ref().unwrap();
        let omitted_count: i64 = f
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM restore_omitted_assets WHERE meeting_id=?1",
                params![f.meeting],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(omitted_count, 1); // Native stdout only; canonical audio is retained.
        let destination = f.add(engine, true, false);
        let calls = calls_in(f.root.path()) + calls_in(f.app_data.path());
        if engine != Engine::Llama {
            let wav = cp["outputs"][0]["wavAssetId"].as_str().unwrap();
            let db = f.storage.connection.lock().unwrap();
            let omitted: i64 = db
                .query_row(
                    "SELECT count(*) FROM restore_omitted_assets WHERE asset_id=?1",
                    [wav],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(omitted, 0, "canonical WAV is retained through restore");
            let relative: String = db
                .query_row(
                    "SELECT relative_path FROM file_assets WHERE id=?1",
                    [wav],
                    |r| r.get(0),
                )
                .unwrap();
            drop(db);
            let path = f.storage.managed_path(Path::new(&relative)).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            std::fs::write(&path, b"changed canonical input").unwrap();
            assert!(retry_preview(&f, &source, destination).is_err());
            std::fs::write(path, bytes).unwrap();
        }
        if engine == Engine::Sherpa {
            let log = cp["outputs"][0]["stdoutAssetId"].as_str().unwrap();
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM restore_omitted_assets WHERE asset_id=?1",
                    params![log],
                )
                .unwrap();
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO restore_omitted_assets SELECT id,meeting_id,'recreatable_pcm',size_bytes,content_hash FROM file_assets WHERE id=?1",
                    params![log],
                )
                .unwrap();
            assert!(
                retry_preview(&f, &source, destination).is_err(),
                "a missing log requires its own typed omission proof"
            );
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM restore_omitted_assets WHERE asset_id=?1",
                    params![log],
                )
                .unwrap();
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO restore_omitted_assets SELECT id,meeting_id,'native_log',size_bytes,content_hash FROM file_assets WHERE id=?1",
                    params![log],
                )
                .unwrap();
        }
        assert_eq!(f.storage.get_job(id).unwrap(), source);
        assert_eq!(f.storage.fixed_job_input(id).unwrap(), original);
        assert_eq!(calls_in(f.root.path()) + calls_in(f.app_data.path()), calls);
        let preview = retry_preview(&f, &source, destination).unwrap();
        let next = crate::test_support::model_retry::apply(
            &f.storage,
            &f.models(),
            &Uuid::new_v4().to_string(),
            &preview,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(next.completed_units, 1);
        assert_eq!(
            f.storage.fixed_job_input(&next.id).unwrap().1.as_ref(),
            Some(cp)
        );
        assert_eq!(calls_in(f.root.path()) + calls_in(f.app_data.path()), calls);
        assert!(f.storage.resume_job(id).is_err());
        assert_eq!(
            f.storage.acquire_local_inference_job().unwrap().unwrap().id,
            next.id
        );
        let candidate = match engine {
            Engine::Whisper | Engine::Asr => crate::transcription::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .map_err(|e| e.to_string()),
            Engine::Sherpa => crate::diarization::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            ),
            Engine::Llama => crate::generation::process(
                &f.storage,
                &next.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            ),
        }
        .unwrap();
        assert!(candidate.is_some());
        let after = f.storage.get_job(&next.id).unwrap();
        assert_eq!(after.state, "succeeded");
        let expected = (source.total_units.unwrap() - 1) as usize;
        // The Markdown fixture emits one short body per node, so every
        // unfinished node needs one call. The first extract is not rerun.
        assert_eq!(
            calls_in(f.root.path()) + calls_in(f.app_data.path()) - calls,
            expected,
            "{engine:?}"
        );
        assert_eq!(f.storage.fixed_job_input(id).unwrap(), original);
        assert_eq!(f.storage.get_job(id).unwrap(), source);
        let after_omitted: i64 = f
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM restore_omitted_assets WHERE meeting_id=?1",
                params![f.meeting],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_omitted, omitted_count);
    }
    // The entire retry lineage and all transported runtime proofs must also
    // survive a second real archive/publication, not just JSON redaction.
    let again = tempfile::tempdir().unwrap();
    let all = f
        .storage
        .list_jobs(&f.meeting)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect::<Vec<_>>();
    let (restored, meeting, ids) = crate::storage::backup::published_restore_for_rebinding_test(
        &f.storage,
        again.path(),
        &f.meeting,
        &all,
    );
    assert!(restored.acquire_local_inference_job().unwrap().is_none());
    for id in ids {
        restored.fixed_job_initial_operation(&id).unwrap();
        let (spec, _) = restored.fixed_job_input(&id).unwrap();
        assert!(!serde_json::to_string(&spec.profile_snapshot)
            .unwrap()
            .contains("/models/"));
        assert_eq!(spec.meeting_id, meeting);
    }
}

fn prepare(f: &Fixture, spec: &JobSpec, target: usize) -> Result<Binding, String> {
    let p = &f.packages[target];
    rebinding::prepare(
        &f.storage,
        &f.models(),
        spec,
        &p.id,
        &p.record.review_hash,
        &p.check,
        &crate::test_support::never_cancelled,
    )
}
fn check_execution<T>(spec: &JobSpec, run: impl FnOnce(Execution<'_>) -> T) -> T {
    match spec.kind {
        JobKind::VoiceRegistration | JobKind::VoiceMatching | JobKind::SpeakerCompletion => {
            panic!("not a rebinding fixture")
        }
        JobKind::Transcription => {
            let e: AsrConfig = serde_json::from_value(spec.parameters["engine"].clone()).unwrap();
            run(Execution::Asr(&e))
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
            assert!(managed::verify(
                &spec.profile_snapshot,
                e,
                &crate::test_support::never_cancelled
            )
            .is_err());
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
            binding
                .verify(&profile, execution, &crate::test_support::never_cancelled)
                .unwrap();
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
                new.verify(&crate::test_support::never_cancelled).unwrap();
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
        Execution::Asr(w) => {
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
        Execution::Asr(e) => serde_json::to_value(e).unwrap(),
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
            assert!(unavailable
                .verify(&profile, e, &crate::test_support::never_cancelled)
                .is_err());
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
            .verify(
                &spec.profile_snapshot,
                e,
                &crate::test_support::never_cancelled,
            )
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
