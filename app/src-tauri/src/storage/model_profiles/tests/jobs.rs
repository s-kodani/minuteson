use super::*;
use crate::diarization::sherpa::SherpaConfig;
use crate::generation::llama::LlamaConfig;
use crate::storage::model_profiles::jobs::{self as managed, Execution};
use crate::transcription::asr::AsrConfig;
use serde_json::json;

mod rebinding;

fn whisper(f: &Fixture) -> AsrConfig {
    let p = f.storage.analysis_preferences().unwrap().payload.standard;
    let capture = |s: &str| {
        Fingerprint::capture(Path::new(s), &crate::test_support::never_cancelled).unwrap()
    };
    AsrConfig {
        backend: None,
        executable: capture(&p.executable_path),
        model: capture(&p.model_path),
        vad_model: p.vad_model_path.as_deref().map(capture),
        vad_options: p.vad_options,
        threads: p.threads,
        use_gpu: p.use_gpu,
        prompt: String::new(),
    }
}
fn sherpa(f: &Fixture) -> SherpaConfig {
    let p = f
        .storage
        .diarization_preferences()
        .unwrap()
        .payload
        .standard;
    let capture = |s: &str| {
        Fingerprint::capture(Path::new(s), &crate::test_support::never_cancelled).unwrap()
    };
    SherpaConfig {
        executable: capture(&p.executable_path),
        segmentation_model: capture(&p.segmentation_model_path),
        embedding_model: capture(&p.embedding_model_path),
        runtime_libraries: p.runtime_library_paths.iter().map(|s| capture(s)).collect(),
        threads: p.threads,
        cluster_threshold_milli: p.cluster_threshold_milli,
        window_shift_milli: p.window_shift_milli,
        min_duration_on_ms: p.min_duration_on_ms,
        min_duration_off_ms: p.min_duration_off_ms,
        speech_sensitivity_milli: p.speech_sensitivity_milli,
    }
}
fn llama(f: &Fixture) -> (LlamaConfig, usize) {
    let p = f.storage.minutes_preferences().unwrap().payload.standard;
    let capture = |s: &str| {
        Fingerprint::capture(Path::new(s), &crate::test_support::never_cancelled).unwrap()
    };
    (
        LlamaConfig {
            executable: capture(&p.executable_path),
            model: capture(&p.model_path),
            threads: p.threads,
            context_tokens: p.context_tokens,
            prediction_tokens: p.prediction_tokens,
            gpu_layers: p.gpu_layers,
            seed: p.seed,
        },
        p.prompt_budget,
    )
}
fn ready(f: &mut Fixture) {
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    let id = crate::transcription::process(
        &f.storage,
        &f.baseline.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    f.storage
        .commit_analysis_candidate(&f.meeting, &id, 0, false)
        .unwrap();
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let index = f.add(engine, true, false);
        f.apply(index, ProfileName::Standard);
    }
}

fn minutes_start(f: &Fixture) -> crate::generation::StartRequest {
    let p = f.storage.minutes_preferences().unwrap().payload.standard;
    crate::generation::StartRequest {
        input: request(f, &Uuid::new_v4().to_string()),
        executable_path: p.executable_path.into(),
        model_path: p.model_path.into(),
        threads: p.threads,
        context_tokens: p.context_tokens,
        prediction_tokens: p.prediction_tokens,
        gpu_layers: p.gpu_layers,
        seed: p.seed,
        profile: "standard".into(),
        prompt_budget: p.prompt_budget,
    }
}

#[test]
fn minutes_admission_uses_sealed_metadata_and_worker_rejects_missing_model_before_inference() {
    let mut f = Fixture::new();
    ready(&mut f);
    let expected = llama(&f).0;
    let start = minutes_start(&f);
    // A removed large file must not be opened/scanned during admission. The
    // worker must still reject it before starting a CLI or publishing output.
    std::fs::remove_file(&expected.model.path).unwrap();
    let (engine, selection) = f.storage.prepare_minutes_admission(&start).unwrap();
    assert_eq!(engine, expected);
    let job = crate::generation::markdown_job::enqueue_prepared_synthetic(
        &f.storage,
        start.input.clone(),
        engine,
        start.prompt_budget,
        selection,
    )
    .unwrap();
    assert_eq!(job.state, "queued");
    let (again, snapshot) = f.storage.prepare_minutes_admission(&start).unwrap();
    let replay = crate::generation::markdown_job::enqueue_prepared_synthetic(
        &f.storage,
        start.input,
        again,
        start.prompt_budget,
        snapshot,
    )
    .unwrap();
    assert_eq!(replay.id, job.id);
    let claimed = f.storage.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(claimed.id, job.id);
    assert!(crate::generation::markdown_job::process(
        &f.storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
    assert_eq!(f.storage.get_job(&job.id).unwrap().completed_units, 0);
    assert!(f
        .storage
        .get_minutes_markdown(&f.meeting)
        .unwrap()
        .is_none());
}

#[test]
fn minutes_admission_rejects_unselected_paths_and_changed_settings_without_file_reads() {
    let mut f = Fixture::new();
    ready(&mut f);
    let mut start = minutes_start(&f);
    start.model_path = "/unselected/model.gguf".into();
    assert!(f.storage.prepare_minutes_admission(&start).is_err());
    let mut start = minutes_start(&f);
    start.threads = if start.threads == 1 { 2 } else { 1 };
    assert!(f.storage.prepare_minutes_admission(&start).is_err());
    let mut start = minutes_start(&f);
    start.profile = "unknown".into();
    assert!(f.storage.prepare_minutes_admission(&start).is_err());
}
fn request(f: &Fixture, op: &str) -> MinutesJobRequest {
    let a = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    MinutesJobRequest {
        meeting_id: f.meeting.clone(),
        operation_id: if op == "managed-minutes" {
            Uuid::new_v4().to_string()
        } else {
            op.into()
        },
        expected_analysis_id: a.id,
        expected_content_hash: a.content_hash,
        expected_meeting_lock_version: f
            .storage
            .get_meeting(&f.meeting)
            .unwrap()
            .unwrap()
            .lock_version,
        expected_draft: f
            .storage
            .load_transcript_draft(&f.meeting)
            .unwrap()
            .map(|d| crate::storage::DraftStamp {
                id: d.id,
                lock_version: d.lock_version,
            }),
    }
}
fn tracks(f: &Fixture) -> Vec<crate::diarization::TrackSettings> {
    f.storage
        .ensure_meeting_mix(&f.meeting, &|| false, &|_, _| {})
        .unwrap()
        .tracks
        .into_iter()
        .map(|t| crate::diarization::TrackSettings {
            track_id: t.id,
            num_speakers: None,
        })
        .collect()
}
fn enqueue_all(f: &Fixture) -> Vec<JobSnapshot> {
    let w = crate::transcription::enqueue(
        &f.storage,
        &f.meeting,
        "managed-asr",
        whisper(f),
        "standard",
    )
    .unwrap();
    let d = crate::diarization::enqueue(
        &f.storage,
        request(f, "managed-diar"),
        sherpa(f),
        "standard",
        tracks(f),
    )
    .unwrap();
    let (l, b) = llama(f);
    let m = crate::generation::markdown_job::enqueue_synthetic(
        &f.storage,
        request(f, "managed-minutes"),
        l,
        "standard",
        b,
        None,
    )
    .unwrap();
    vec![w, d, m]
}

pub(super) fn publication_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Storage,
    String,
    Vec<String>,
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &jobs[0].id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    f.storage.ensure_analysis_flow(&f.meeting).unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    let mut ids = jobs.into_iter().map(|j| j.id).collect::<Vec<_>>();
    ids.push(flow.transcription_job.unwrap().id);
    (f.root, f.app_data, f.storage, f.meeting, ids)
}

#[test]
fn managed_jobs_all_three_engines_bind_partial_or_complete_profile_without_running_a_trial() {
    let mut f = Fixture::new();
    ready(&mut f);
    let calls = (0..3).map(|i| f.executed(i)).collect::<Vec<_>>();
    let jobs = enqueue_all(&f);
    let w = whisper(&f);
    let d = sherpa(&f);
    let (l, b) = llama(&f);
    for (job, engine) in jobs.iter().zip([
        Execution::Asr(&w),
        Execution::Sherpa(&d),
        Execution::Llama(&l, b),
    ]) {
        let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
        assert_eq!(spec.profile_snapshot["managedModel"]["profileVersion"], 3);
        assert_eq!(spec.profile_snapshot["qualityStatus"], "unverified");
        managed::validate(&spec.profile_snapshot, engine).unwrap();
        assert!(!serde_json::to_string(&spec.profile_snapshot)
            .unwrap()
            .contains(f.models().to_str().unwrap()));
    }
    assert_eq!((0..3).map(|i| f.executed(i)).collect::<Vec<_>>(), calls);
    // Missing unrelated engines do not prevent an explicitly applied ASR.
    let mut partial = Fixture::new();
    let i = partial.add(Engine::Whisper, true, false);
    partial.apply(i, ProfileName::Standard);
    let w = whisper(&partial);
    assert!(partial
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&w),
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .is_some());
}

#[test]
fn managed_jobs_current_model_vad_executable_library_manifest_and_trial_artifact_mutations_cannot_enqueue(
) {
    for target in [
        "model.bin",
        "vad.bin",
        "engine-cli",
        "lib/whisper-runtime.dylib",
        "mon-model.json",
        "trial",
    ] {
        let mut f = Fixture::new();
        let i = f.add(Engine::Whisper, true, false);
        f.apply(i, ProfileName::Standard);
        let before = f.count("processing_jobs");
        let path = if target == "trial" {
            f.app_data
                .path()
                .join("model-checks")
                .join(&f.packages[i].check)
                .join("stderr.log")
        } else {
            f.models().join(&f.packages[i].id).join(target)
        };
        std::fs::write(path, b"changed after approval").unwrap();
        let result = crate::transcription::enqueue(
            &f.storage,
            &f.meeting,
            "changed",
            whisper(&f),
            "standard",
        );
        assert!(result.is_err(), "{target}");
        assert_eq!(f.count("processing_jobs"), before);
    }
    let mut f = Fixture::new();
    let i = f.add(Engine::Sherpa, true, false);
    f.apply(i, ProfileName::Standard);
    std::fs::write(
        f.models()
            .join(&f.packages[i].id)
            .join("lib/libsherpa-onnx-c-api.dylib"),
        b"changed library",
    )
    .unwrap();
    assert!(f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Sherpa(&sherpa(&f)),
            &crate::test_support::never_cancelled
        )
        .is_err());
}

#[test]
fn managed_jobs_stale_numeric_settings_manual_paths_other_profile_and_cancellation_cannot_bypass_application(
) {
    let mut f = Fixture::new();
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    let mut w = whisper(&f);
    w.threads += 1;
    assert!(f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&w),
            &crate::test_support::never_cancelled
        )
        .is_err());
    let w = whisper(&f);
    assert!(f
        .storage
        .prepare_model_selection(
            "high_accuracy",
            Execution::Asr(&w),
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(f
        .storage
        .prepare_model_selection("standard", Execution::Asr(&w), &|| true)
        .is_err());
    let original = f.storage.analysis_preferences().unwrap();
    let mut prefs = original.payload;
    prefs.standard.threads += 1;
    f.storage
        .save_analysis_preferences(original.lock_version, prefs)
        .unwrap();
    assert!(f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&whisper(&f)),
            &crate::test_support::never_cancelled
        )
        .is_err());
    f.apply(i, ProfileName::Standard);
    assert!(f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&whisper(&f)),
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .is_some());
}

#[test]
fn managed_jobs_transaction_gate_rejects_removed_or_forged_proof_and_head_changes_for_all_three_engines(
) {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let specs = jobs
        .iter()
        .map(|j| f.storage.fixed_job_input(&j.id).unwrap().0)
        .collect::<Vec<_>>();
    for original in &specs {
        for mode in ["missing", "forged"] {
            let mut s = original.clone();
            s.operation_id = Uuid::new_v4().to_string();
            if mode == "missing" {
                s.profile_snapshot
                    .as_object_mut()
                    .unwrap()
                    .remove("managedModel");
            } else {
                s.profile_snapshot["managedModel"]["profileVersion"] = json!(99);
            }
            let before = f.count("processing_jobs");
            assert!(f.storage.enqueue_job(s).is_err());
            assert_eq!(f.count("processing_jobs"), before);
        }
    }
    let index = f.add(Engine::Whisper, true, false);
    f.apply(index, ProfileName::Standard);
    for original in specs {
        let mut s = original;
        s.operation_id = Uuid::new_v4().to_string();
        let before = f.count("processing_jobs");
        assert!(f.storage.enqueue_job(s).is_err());
        assert_eq!(f.count("processing_jobs"), before);
    }
    // Producer-side snapshot creation must roll back too, not leave an orphan.
    for (job, kind) in jobs.iter().skip(1).zip(["diarization", "generation"]) {
        let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
        let profile = spec.profile_snapshot;
        let mut p = spec.parameters[kind].clone();
        let count = f.count("analysis_input_snapshots");
        let produced = std::cell::Cell::new(false);
        let result = if kind == "generation" {
            let target = serde_json::from_value(
                p.as_object_mut().unwrap().remove("markdownTarget").unwrap(),
            )
            .unwrap();
            let mut request = request(&f, "late-minutes");
            request.operation_id = Uuid::new_v4().to_string();
            f.storage
                .enqueue_markdown_minutes_job_configured(request, profile, target, |_| {
                    produced.set(true);
                    Ok((p, spec.total_units))
                })
        } else {
            f.storage
                .enqueue_diarization_job(request(&f, "late-diar"), profile, |_| {
                    produced.set(true);
                    Ok((p, spec.total_units.unwrap()))
                })
        };
        assert!(
            produced.get(),
            "must reach snapshot production, not an earlier ID/target guard"
        );
        assert!(result
            .unwrap_err()
            .contains("管理モデルの構成版が変わっています"));
        assert_eq!(f.count("analysis_input_snapshots"), count);
    }
}

#[test]
fn managed_jobs_fixed_execution_and_same_operation_survive_head_settings_and_trial_changes() {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let original = f.storage.fixed_job_input(&jobs[0].id).unwrap();
    let w = whisper(&f);
    let d = sherpa(&f);
    let (l, b) = llama(&f);
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    let duplicate =
        crate::transcription::enqueue(&f.storage, &f.meeting, "managed-asr", w.clone(), "standard")
            .unwrap();
    assert_eq!(duplicate, jobs[0]);
    let check = Uuid::new_v4().to_string();
    f.storage
        .begin_model_check(&check, &f.packages[1].record)
        .unwrap();
    f.storage
        .finish_model_check(
            &check,
            &f.packages[1].record,
            None,
            CheckState::Failed,
            Some("synthetic failure"),
        )
        .unwrap();
    assert!(f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Sherpa(&d),
            &crate::test_support::never_cancelled
        )
        .is_err());
    for (job, e) in jobs.iter().zip([
        Execution::Asr(&w),
        Execution::Sherpa(&d),
        Execution::Llama(&l, b),
    ]) {
        let (s, _) = f.storage.fixed_job_input(&job.id).unwrap();
        managed::verify(
            &s.profile_snapshot,
            e,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    }
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &jobs[0].id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        f.storage.fixed_job_input(&jobs[0].id).unwrap().0,
        original.0
    );
}

#[test]
fn managed_jobs_runtime_refuses_mutated_package_before_any_child_process_and_preserves_fixed_spec()
{
    let mut f = Fixture::new();
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    let w = whisper(&f);
    let job =
        crate::transcription::enqueue(&f.storage, &f.meeting, "runtime-managed", w, "standard")
            .unwrap();
    let original = f.storage.fixed_job_input(&job.id).unwrap();
    // Drain the older fixture job; it stays executable under its original proof.
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &f.baseline.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    std::fs::write(
        f.models().join(&f.packages[i].id).join("vad.bin"),
        b"new bytes",
    )
    .unwrap();
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    assert!(crate::transcription::process(
        &f.storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
    assert!(!f
        .models()
        .join(&f.packages[i].id)
        .join("executed.marker")
        .exists());
    assert_eq!(f.storage.fixed_job_input(&job.id).unwrap(), original);
}

#[test]
fn managed_jobs_flow_freezes_both_engines_and_handoffs_use_old_proof_after_new_application() {
    let mut f = Fixture::new();
    ready(&mut f);
    f.storage.ensure_analysis_flow(&f.meeting).unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    let original = flow.plan.clone().unwrap();
    assert!(original["whisperManaged"].is_object());
    assert!(original["sherpaManaged"].is_object());
    let old = crate::analysis_flow::saved_plan(original.clone(), "standard").unwrap();
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &flow.transcription_job.unwrap().id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let next = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    assert_eq!(next.plan, Some(original));
    let job = next.diarization_job.as_ref().unwrap();
    let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        managed::read(&spec.profile_snapshot).unwrap(),
        old.sherpa_managed
    );
    let actual: SherpaConfig =
        serde_json::from_value(spec.parameters["diarization"]["engine"].clone()).unwrap();
    assert_eq!(actual, old.sherpa);
    // Durable plans are immutable even when a valid replacement is supplied.
    let mut changed = next.plan.clone().unwrap();
    changed["sherpa"]["threads"] = json!(1);
    assert!(f
        .storage
        .update_analysis_flow(&next, "diarization", Some(changed), None, None, None)
        .is_err());
}

#[test]
fn cpu_thread_tuning_keeps_started_flow_conditions_and_new_jobs_use_new_threads() {
    let mut f = Fixture::new();
    ready(&mut f);
    f.storage.ensure_analysis_flow(&f.meeting).unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    let original = flow.plan.clone().unwrap();
    let old = crate::analysis_flow::saved_plan(original.clone(), "standard").unwrap();
    let package = f
        .packages
        .iter()
        .find(|p| p.record.manifest.engine == Engine::Sherpa)
        .unwrap();
    let preview = crate::model_application::preview_with_threads(
        &f.storage,
        &f.models(),
        ProfileName::Standard,
        &package.id,
        &package.record.review_hash,
        &package.check,
        Some(8),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &preview,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let new = crate::analysis_flow::prepare(
        &f.storage,
        "standard",
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(new.sherpa.threads, 8);
    let mut expected = old.sherpa.clone();
    expected.threads = 8;
    assert_eq!(new.sherpa, expected);
    assert_ne!(new.sherpa_managed, old.sherpa_managed);
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &flow.transcription_job.unwrap().id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let next = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    assert_eq!(next.plan, Some(original));
    let (spec, _) = f
        .storage
        .fixed_job_input(&next.diarization_job.unwrap().id)
        .unwrap();
    let frozen: SherpaConfig =
        serde_json::from_value(spec.parameters["diarization"]["engine"].clone()).unwrap();
    assert_eq!(frozen, old.sherpa);
    let job = crate::diarization::enqueue(
        &f.storage,
        request(&f, &Uuid::new_v4().to_string()),
        new.sherpa,
        "standard",
        tracks(&f),
    )
    .unwrap();
    let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(spec.parameters["diarization"]["engine"]["threads"], 8);
}

#[test]
fn managed_jobs_portable_decoder_accepts_other_supported_target_but_rejects_tampered_hash_roles_and_mixed_paths(
) {
    let mut f = Fixture::new();
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    let w = whisper(&f);
    let selected = f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&w),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let profile = managed::attach(json!({"profile":"standard"}), selected).unwrap();
    let mut portable = w.clone();
    for p in [
        &mut portable.executable,
        &mut portable.model,
        portable.vad_model.as_mut().unwrap(),
    ] {
        p.path = PathBuf::from(format!("mon-unavailable:{}", p.sha256));
    }
    managed::validate(&profile, Execution::Asr(&portable)).unwrap();
    let mut invalid = portable.clone();
    invalid.threads = 0;
    let mut invalid_profile = profile.clone();
    invalid_profile["managedModel"]["configuration"]["threads"] = json!(0);
    assert!(managed::validate(&invalid_profile, Execution::Asr(&invalid)).is_err());
    let mut other = profile.clone();
    let mut record: PlacementRecord =
        serde_json::from_value(other["managedModel"]["package"].clone()).unwrap();
    record.manifest.target = Target::WindowsX86_64;
    record.source_manifest_hash = digest(&record.manifest).unwrap();
    record.review_hash = crate::model_inspection::review_hash(
        &record.manifest,
        &record.source_manifest_hash,
        &record.files,
    )
    .unwrap();
    other["managedModel"]["compatibility"]["reviewHash"] = json!(record.review_hash);
    other["managedModel"]["package"] = serde_json::to_value(record).unwrap();
    managed::validate(&other, Execution::Asr(&portable)).unwrap();
    assert!(managed::verify(
        &other,
        Execution::Asr(&portable),
        &crate::test_support::never_cancelled
    )
    .is_err());
    let mut mixed = portable.clone();
    mixed.executable = w.executable.clone();
    assert!(managed::validate(&profile, Execution::Asr(&mixed)).is_err());
    for field in ["profileHash", "configuration", "compatibility"] {
        let mut p = profile.clone();
        p["managedModel"][field] = json!(null);
        assert!(managed::validate(&p, Execution::Asr(&portable)).is_err());
    }
    let mut p = profile.clone();
    p["managedModel"]["package"]["files"][1]["actualHash"] = json!("a".repeat(64));
    assert!(managed::validate(&p, Execution::Asr(&portable)).is_err());
    // Native availability remains a separate operation, never a restore gate.
    assert!(managed::verify(
        &profile,
        Execution::Asr(&portable),
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn managed_jobs_retry_all_three_engines_keeps_old_conditions_after_profile_changes() {
    let mut f = Fixture::new();
    ready(&mut f);
    let jobs = enqueue_all(&f);
    let originals = jobs
        .iter()
        .map(|j| f.storage.fixed_job_input(&j.id).unwrap().0)
        .collect::<Vec<_>>();
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    let mut retried = vec![];
    for job in &jobs {
        assert_eq!(
            f.storage.acquire_local_inference_job().unwrap().unwrap().id,
            job.id
        );
        let failed = f
            .storage
            .finish_job(
                &job.id,
                0,
                "failed",
                Some("fixture_failed_before_execution"),
            )
            .unwrap();
        let result = match job.kind.as_str() {
            "transcription" => crate::transcription::RetryRequest {
                meeting_id: f.meeting.clone(),
                job_id: job.id.clone(),
                operation_id: "retry-managed-asr".into(),
                expected_spec_hash: failed.spec_hash,
            }
            .enqueue(&f.storage),
            "diarization" => crate::diarization::RetryRequest {
                meeting_id: f.meeting.clone(),
                job_id: job.id.clone(),
                operation_id: "retry-managed-diar".into(),
                expected_spec_hash: failed.spec_hash,
            }
            .enqueue(&f.storage),
            "minutes_generation" => crate::generation::RetryRequest {
                meeting_id: f.meeting.clone(),
                job_id: job.id.clone(),
                operation_id: Uuid::new_v4().to_string(),
                expected_spec_hash: failed.spec_hash,
            }
            .enqueue(&f.storage),
            _ => unreachable!(),
        };
        retried.push(result.unwrap());
    }
    for (job, original) in retried.iter().zip(originals) {
        let spec = f.storage.fixed_job_input(&job.id).unwrap().0;
        assert_eq!(spec.profile_snapshot, original.profile_snapshot);
        assert_eq!(spec.parameters, original.parameters);
        assert_eq!(job.attempt, 2);
    }
}

#[test]
fn managed_jobs_plan_save_rechecks_current_head_and_never_commits_a_stale_plan() {
    let mut f = Fixture::new();
    ready(&mut f);
    let flow = f.storage.ensure_analysis_flow(&f.meeting).unwrap();
    let w = whisper(&f);
    let d = sherpa(&f);
    let ws = f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Asr(&w),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let ds = f
        .storage
        .prepare_model_selection(
            "standard",
            Execution::Sherpa(&d),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let plan = json!({"schemaVersion":1,"profile":"standard","whisper":w,"sherpa":d,"whisperManaged":ws,"sherpaManaged":ds});
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    assert!(f
        .storage
        .update_analysis_flow(&flow, "transcription", Some(plan), None, None, None)
        .is_err());
    let saved = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    assert_eq!(saved.stage, "pending");
    assert!(saved.plan.is_none());
    assert_eq!(saved.lock_version, flow.lock_version);
}

#[test]
fn managed_jobs_diarization_and_minutes_runtime_reject_untracked_library_mutation_before_launch() {
    for index in [1, 2] {
        let mut f = Fixture::new();
        ready(&mut f);
        let jobs = enqueue_all(&f);
        for earlier in jobs.iter().take(index) {
            assert_eq!(
                f.storage.acquire_local_inference_job().unwrap().unwrap().id,
                earlier.id
            );
            f.storage
                .finish_job(
                    &earlier.id,
                    0,
                    "failed",
                    Some("fixture_failed_before_execution"),
                )
                .unwrap();
        }
        let job = &jobs[index];
        let spec = f.storage.fixed_job_input(&job.id).unwrap();
        let library = if index == 1 {
            "lib/libsherpa-onnx-c-api.dylib"
        } else {
            "lib/llama-runtime.dylib"
        };
        std::fs::write(
            f.models().join(&f.packages[index].id).join(library),
            b"changed dependency",
        )
        .unwrap();
        assert_eq!(
            f.storage.acquire_local_inference_job().unwrap().unwrap().id,
            job.id
        );
        let result = if index == 1 {
            crate::diarization::process(
                &f.storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
        } else {
            crate::generation::process(
                &f.storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
        };
        assert!(result.is_err());
        assert_eq!(f.storage.fixed_job_input(&job.id).unwrap(), spec);
        assert!(!f
            .models()
            .join(&f.packages[index].id)
            .join("executed.marker")
            .exists());
    }
}

#[test]
fn automatic_merge_tuning_keeps_started_flow_conditions_and_new_jobs_use_new_threshold() {
    let mut f = Fixture::new();
    ready(&mut f);
    f.storage.ensure_analysis_flow(&f.meeting).unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let flow = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    let original = flow.plan.clone().unwrap();
    let old = crate::analysis_flow::saved_plan(original.clone(), "standard").unwrap();
    let package = f
        .packages
        .iter()
        .find(|p| p.record.manifest.engine == Engine::Sherpa)
        .unwrap();
    let preview = crate::model_application::preview_with_tuning(
        &f.storage,
        &f.models(),
        ProfileName::Standard,
        &package.id,
        &package.record.review_hash,
        &package.check,
        None,
        Some(1300),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &preview,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let new = crate::analysis_flow::prepare(
        &f.storage,
        "standard",
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(new.sherpa.cluster_threshold_milli, 1300);
    let mut expected = old.sherpa.clone();
    expected.cluster_threshold_milli = 1300;
    assert_eq!(new.sherpa, expected);
    assert_ne!(new.sherpa_managed, old.sherpa_managed);
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &f.storage,
        &flow.transcription_job.unwrap().id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let next = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
    assert_eq!(next.plan, Some(original));
    let (spec, _) = f
        .storage
        .fixed_job_input(&next.diarization_job.unwrap().id)
        .unwrap();
    let frozen: SherpaConfig =
        serde_json::from_value(spec.parameters["diarization"]["engine"].clone()).unwrap();
    assert_eq!(frozen, old.sherpa);
    let job = crate::diarization::enqueue(
        &f.storage,
        request(&f, &Uuid::new_v4().to_string()),
        new.sherpa,
        "standard",
        tracks(&f),
    )
    .unwrap();
    let (spec, _) = f.storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        spec.parameters["diarization"]["engine"]["clusterThresholdMilli"],
        1300
    );
}
