use super::*;
use crate::local_engine::Fingerprint;
use crate::model_inspection::{PackageFile, PackageManifest, Target};
use std::os::unix::fs::PermissionsExt;

mod jobs;
mod portable;

pub(in crate::storage) fn publication_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Storage,
    String,
    Vec<String>,
) {
    jobs::publication_fixture()
}

struct Package {
    id: String,
    check: String,
    record: PlacementRecord,
    _source: tempfile::TempDir,
}
struct Fixture {
    root: tempfile::TempDir,
    app_data: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    baseline: JobSnapshot,
    packages: Vec<Package>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_seconds(4)
    }
    fn with_seconds(seconds: i64) -> Self {
        Self::with_tracks(seconds, false)
    }
    fn with_tracks(seconds: i64, two: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let (storage, meeting, engine) =
            super::super::transcription::tests::fixture(root.path(), seconds);
        if two {
            let relative = PathBuf::from("meetings")
                .join(&meeting)
                .join("audio")
                .join("system.caf");
            let path = storage.managed_path(&relative).unwrap();
            std::fs::copy(path.parent().unwrap().join("mic.caf"), &path).unwrap();
            let audio = storage
                .register_ready_meeting_asset(&meeting, &relative, "audio/x-caf")
                .unwrap();
            let db = storage.connection.lock().unwrap();
            db.execute("INSERT INTO audio_tracks (id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('track2',?1,'session','system','caf',16000,1,'ready',0,0)",params![meeting]).unwrap();
            db.execute("INSERT INTO audio_chunks (id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) SELECT 'chunk2',meeting_id,'track2',?1,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at FROM audio_chunks WHERE id='chunk'", params![audio.id]).unwrap();
        }
        let baseline = crate::transcription::enqueue(
            &storage,
            &meeting,
            "old-fixed-job",
            engine.clone(),
            "standard",
        )
        .unwrap();
        let mut asr = storage.analysis_preferences().unwrap().payload;
        asr.standard = preferences::EnginePreferences {
            executable_path: engine.executable.path.to_str().unwrap().into(),
            model_path: engine.model.path.to_str().unwrap().into(),
            vad_model_path: None,
            threads: 7,
            use_gpu: true,
        };
        asr.high_accuracy = asr.standard.clone();
        asr.high_accuracy.threads = 3;
        asr.high_accuracy.use_gpu = false;
        storage.save_analysis_preferences(0, asr).unwrap();
        let mut speakers = storage.diarization_preferences().unwrap().payload;
        speakers.standard.threads = 6;
        speakers.standard.cluster_threshold_milli = 700;
        speakers.standard.window_shift_milli = 400;
        speakers.standard.min_duration_on_ms = 200;
        speakers.standard.min_duration_off_ms = 600;
        speakers.high_accuracy.threads = 5;
        speakers.high_accuracy.cluster_threshold_milli = 900;
        storage.save_diarization_preferences(0, speakers).unwrap();
        let mut minutes = storage.minutes_preferences().unwrap().payload;
        minutes.standard.threads = 5;
        minutes.standard.prediction_tokens = 512;
        minutes.standard.prompt_budget = 5000;
        minutes.standard.seed = 777;
        minutes.standard.gpu_layers = 2;
        minutes.high_accuracy.context_tokens = 16384;
        minutes.high_accuracy.prompt_budget = 9000;
        minutes.high_accuracy.seed = 14;
        storage.save_minutes_preferences(0, minutes).unwrap();
        Self {
            root,
            app_data,
            storage,
            meeting,
            baseline,
            packages: Vec::new(),
        }
    }
    fn models(&self) -> PathBuf {
        self.app_data.path().canonicalize().unwrap().join("models")
    }
    fn add(&mut self, engine: Engine, trial: bool, fail: bool) -> usize {
        let source = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4().to_string();
        let check = Uuid::new_v4().to_string();
        let models = self.models();
        let roles = match engine {
            Engine::Whisper => vec![
                ("model.bin", Role::Asr),
                ("vad.bin", Role::Vad),
                ("lib/whisper-runtime.dylib", Role::RuntimeLibrary),
            ],
            Engine::Sherpa => vec![
                ("seg.onnx", Role::Segmentation),
                ("embed.onnx", Role::SpeakerEmbedding),
                ("lib/runtime.dylib", Role::RuntimeLibrary),
                ("lib/sub/runtime.dylib", Role::RuntimeLibrary),
            ],
            Engine::Llama => vec![
                ("model.bin", Role::Llm),
                ("lib/llama-runtime.dylib", Role::RuntimeLibrary),
            ],
        };
        for (name, _) in &roles {
            let file = source.path().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"synthetic fixture only, not real model weights").unwrap();
        }
        let body: String = if fail {
            "exit 42".into()
        } else {
            match engine {
            Engine::Whisper=>"model=; output=\nwhile [ \"$#\" -gt 0 ]; do case \"$1\" in --model) model=\"$2\"; shift;; --output-file) output=\"$2\"; shift;; esac; shift; done\nprintf '{\"model\":{\"multilingual\":true},\"params\":{\"model\":\"%s\",\"language\":\"ja\",\"translate\":false},\"transcription\":[]}' \"$model\" > \"$output.json\"".into(),
            Engine::Llama=>r#"predict=
while [ "$#" -gt 0 ]; do case "$1" in --predict) predict="$2"; shift;; esac; shift; done
if [ "$predict" = 64 ]; then printf '{"ok":true}'; else printf '{"schemaVersion":1,"items":[]}'; fi"#.into(),
            Engine::Sherpa=> {
                // Echo actual CLI arguments rather than embedding an install
                // UUID or CPU defaults. Identical fixture bytes can relocate.
                r#"for arg do
case "$arg" in
--segmentation.pyannote-model=*) seg=${arg#*=};;
--embedding.model=*) embed=${arg#*=};;
--segmentation.num-threads=*) threads=${arg#*=};;
--clustering.num-clusters=*) clusters=${arg#*=};;
--clustering.cluster-threshold=*) threshold=${arg#*=};;
--segmentation.pyannote-window-shift-ratio=*) shift_ratio=${arg#*=};;
--min-duration-on=*) duration_on=${arg#*=};;
--min-duration-off=*) duration_off=${arg#*=};;
esac
done
printf 'OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model="%s", window_shift_ratio=%s), num_threads=%s, debug=False, provider="cpu"), embedding=SpeakerEmbeddingExtractorConfig(model="%s", num_threads=%s, debug=False, provider="cpu"), clustering=FastClusteringConfig(num_clusters=%s, threshold=%s, compute_confidence=False), min_duration_on=%s, min_duration_off=%s)\nStarted\n' "$seg" "$shift_ratio" "$threads" "$embed" "$threads" "$clusters" "$threshold" "$duration_on" "$duration_off"
"#.into()
            },
        }
        };
        std::fs::write(
            source.path().join("engine-cli"),
            format!("#!/bin/sh\necho ran >> executed.marker\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(
            source.path().join("engine-cli"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let mut files = vec![PackageFile {
            relative_path: "engine-cli".into(),
            role: Role::Executable,
            expected_size: None,
            expected_hash: None,
        }];
        files.extend(roles.into_iter().map(|(path, role)| PackageFile {
            relative_path: path.into(),
            role,
            expected_size: None,
            expected_hash: None,
        }));
        let manifest = PackageManifest {
            schema_version: 1,
            engine,
            engine_version: "fixture declaration".into(),
            model_name: format!("fixture {engine:?}"),
            model_version: "1".into(),
            quantization: None,
            license_ref: "fixture only".into(),
            target: Target::current().unwrap(),
            files,
        };
        std::fs::write(
            source.path().join("mon-model.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let inspected = crate::model_inspection::inspect(source.path(), &|| false).unwrap();
        crate::model_placement::place(
            &self.storage,
            &models,
            source.path(),
            &id,
            &inspected.review_hash,
            &|| false,
        )
        .unwrap();
        let record = self
            .storage
            .model_package_for_check(&id, &inspected.review_hash)
            .unwrap();
        if trial {
            let result = crate::model_compatibility::check(
                &self.storage,
                &models,
                &check,
                &id,
                &record.review_hash,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                result.state,
                if fail {
                    CheckState::Failed
                } else {
                    CheckState::Succeeded
                }
            );
        }
        let index = self.packages.len();
        self.packages.push(Package {
            id,
            check,
            record,
            _source: source,
        });
        index
    }
    fn preview(&self, index: usize, name: ProfileName) -> ApplyPreview {
        let p = &self.packages[index];
        crate::model_application::preview(
            &self.storage,
            &self.models(),
            name,
            &p.id,
            &p.record.review_hash,
            &p.check,
            &|| false,
        )
        .unwrap()
    }
    fn apply(&self, index: usize, name: ProfileName) -> ProfileView {
        crate::model_application::apply(
            &self.storage,
            &self.models(),
            &Uuid::new_v4().to_string(),
            &self.preview(index, name),
            &|| false,
        )
        .unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    fn executed(&self, index: usize) -> Vec<u8> {
        std::fs::read(
            self.app_data
                .path()
                .join("model-checks")
                .join(&self.packages[index].check)
                .join("executed.marker"),
        )
        .unwrap()
    }
}

#[test]
fn profile_application_three_engines_keep_parameters_other_profiles_meetings_and_fixed_jobs() {
    let mut f = Fixture::new();
    let baseline = f.storage.fixed_job_input(&f.baseline.id).unwrap();
    let meetings = f.storage.list_meetings().unwrap();
    let original_asr = f.storage.analysis_preferences().unwrap();
    let original_speakers = f.storage.diarization_preferences().unwrap();
    let original_minutes = f.storage.minutes_preferences().unwrap();
    for engine in [Engine::Whisper, Engine::Sherpa, Engine::Llama] {
        let index = f.add(engine, true, false);
        let calls = f.executed(index);
        let preview = f.preview(index, ProfileName::Standard);
        assert_ne!(preview.before, preview.after);
        let result = crate::model_application::apply(
            &f.storage,
            &f.models(),
            &Uuid::new_v4().to_string(),
            &preview,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            f.executed(index),
            calls,
            "preview and apply never run the engine"
        );
        assert_eq!(result.profile_version, index as i64 + 1);
        assert!(result
            .components
            .iter()
            .all(|c| c.settings_match && c.compatibility_current));
        assert_eq!(result.configuration_complete, index == 2);
    }
    let asr = f.storage.analysis_preferences().unwrap();
    let speakers = f.storage.diarization_preferences().unwrap();
    let minutes = f.storage.minutes_preferences().unwrap();
    assert_eq!(asr.payload.standard.threads, 7);
    assert!(asr.payload.standard.use_gpu);
    assert!(asr.payload.standard.vad_model_path.is_some());
    assert_eq!(
        asr.payload.high_accuracy,
        original_asr.payload.high_accuracy
    );
    assert_eq!(speakers.payload.standard.threads, 6);
    assert_eq!(speakers.payload.standard.cluster_threshold_milli, 700);
    assert_eq!(speakers.payload.standard.window_shift_milli, 400);
    assert_eq!(speakers.payload.standard.min_duration_on_ms, 200);
    assert_eq!(speakers.payload.standard.min_duration_off_ms, 600);
    assert_eq!(speakers.payload.standard.runtime_library_paths.len(), 2);
    assert_eq!(
        speakers.payload.high_accuracy,
        original_speakers.payload.high_accuracy
    );
    assert_eq!(minutes.payload.standard.threads, 5);
    assert_eq!(minutes.payload.standard.context_tokens, 8192);
    assert_eq!(minutes.payload.standard.prediction_tokens, 512);
    assert_eq!(minutes.payload.standard.prompt_budget, 5000);
    assert_eq!(minutes.payload.standard.seed, 777);
    assert_eq!(minutes.payload.standard.gpu_layers, 2);
    assert_eq!(
        minutes.payload.high_accuracy,
        original_minutes.payload.high_accuracy
    );
    assert_eq!(
        (
            asr.lock_version,
            speakers.lock_version,
            minutes.lock_version
        ),
        (2, 2, 2)
    );
    assert_eq!(f.storage.fixed_job_input(&f.baseline.id).unwrap(), baseline);
    assert_eq!(f.storage.list_meetings().unwrap(), meetings);
    let saved = f.storage.model_profiles().unwrap();
    assert!(saved[0].configuration_complete);
    assert_eq!(saved[1].profile_version, 0);
    assert_eq!(saved[0].components.iter().flat_map(|c| &c.roles).count(), 4);
    // The existing enqueue path uses the now-selected managed file hashes, not
    // the old job's fixed model or the CPU smoke trial's thread/GPU settings.
    let selected = asr.payload.standard;
    let job = crate::transcription::StartRequest {
        meeting_id: f.meeting.clone(),
        operation_id: "new-managed-model-job".into(),
        executable_path: selected.executable_path.into(),
        model_path: selected.model_path.into(),
        vad_model_path: selected.vad_model_path.map(Into::into),
        threads: selected.threads,
        use_gpu: selected.use_gpu,
        profile: "standard".into(),
        prompt: String::new(),
        expected_vocabulary: None,
    }
    .enqueue(&f.storage)
    .unwrap();
    let spec = f.storage.fixed_job_input(&job.id).unwrap().0;
    assert_eq!(spec.parameters["engine"]["threads"], 7);
    assert_eq!(spec.parameters["engine"]["useGpu"], true);
    assert_eq!(
        spec.parameters["engine"]["model"]["sha256"],
        f.packages[0]
            .record
            .files
            .iter()
            .find(|p| p.role == Role::Asr)
            .unwrap()
            .actual_hash
    );
    assert_eq!(f.storage.fixed_job_input(&f.baseline.id).unwrap(), baseline);
    drop(f.storage);
    let reopened = Storage::open(f.root.path().into()).unwrap();
    assert_eq!(reopened.model_profiles().unwrap(), saved);
    assert_eq!(reopened.fixed_job_input(&f.baseline.id).unwrap(), baseline);
}

#[test]
fn profile_application_high_accuracy_is_separate_and_exact_retry_does_not_duplicate_or_overwrite_newer_head(
) {
    let mut f = Fixture::new();
    let index = f.add(Engine::Llama, true, false);
    let standard = f.storage.minutes_preferences().unwrap().payload.standard;
    let preview = f.preview(index, ProfileName::HighAccuracy);
    let operation = Uuid::new_v4().to_string();
    let first =
        crate::model_application::apply(&f.storage, &f.models(), &operation, &preview, &|| false)
            .unwrap();
    assert_eq!(first.profile_version, 1);
    assert_eq!(f.storage.model_profiles().unwrap()[0].id, None);
    assert_eq!(
        f.storage.minutes_preferences().unwrap().payload.standard,
        standard
    );
    let version = f.storage.minutes_preferences().unwrap().lock_version;
    assert_eq!(
        crate::model_application::apply(&f.storage, &f.models(), &operation, &preview, &|| false)
            .unwrap(),
        first
    );
    assert_eq!(f.count("model_profiles"), 1);
    assert_eq!(
        f.storage.minutes_preferences().unwrap().lock_version,
        version
    );
    let current = f.apply(index, ProfileName::HighAccuracy);
    assert_eq!(current.profile_version, 2);
    assert_eq!(
        f.storage.minutes_preferences().unwrap().lock_version,
        version,
        "same settings need no rewrite, but explicit application gets a new immutable version"
    );
    assert!(crate::model_application::apply(
        &f.storage,
        &f.models(),
        &operation,
        &preview,
        &|| false
    )
    .unwrap_err()
    .contains("古い"));
    assert_eq!(f.storage.model_profiles().unwrap()[1], current);
    assert_eq!(f.count("model_profiles"), 2);
}

#[test]
fn profile_application_stale_settings_head_or_altered_review_cannot_write() {
    for condition in ["preferences", "head", "review", "digest", "identity"] {
        let mut f = Fixture::new();
        let index = f.add(Engine::Whisper, true, false);
        let mut preview = f.preview(index, ProfileName::Standard);
        let mut operation = Uuid::new_v4().to_string();
        match condition {
            "preferences" => {
                let mut p = f.storage.analysis_preferences().unwrap().payload;
                p.high_accuracy.threads += 1;
                f.storage.save_analysis_preferences(1, p).unwrap();
            }
            "head" => {
                f.apply(index, ProfileName::Standard);
            }
            "review" => {
                if let Settings::Whisper(p) = &mut preview.after {
                    p.threads = 1;
                }
                let mut checked = preview.clone();
                checked.review_digest.clear();
                preview.review_digest = digest(&checked).unwrap();
            }
            "digest" => preview.review_digest = "b".repeat(64),
            "identity" => operation = f.meeting.clone(),
            _ => unreachable!(),
        }
        let before = f.storage.analysis_preferences().unwrap();
        let heads = f.storage.model_profiles().unwrap();
        let count = f.count("model_profiles");
        assert!(
            crate::model_application::apply(&f.storage, &f.models(), &operation, &preview, &|| {
                false
            })
            .is_err(),
            "{condition}"
        );
        assert_eq!(
            f.storage.analysis_preferences().unwrap().payload,
            before.payload
        );
        assert_eq!(
            f.storage.analysis_preferences().unwrap().lock_version,
            before.lock_version
        );
        assert_eq!(f.storage.model_profiles().unwrap(), heads);
        assert_eq!(f.count("model_profiles"), count);
    }
}

#[test]
fn profile_application_transaction_failure_and_late_cancellation_roll_back_preferences_components_and_heads(
) {
    for fail in [true, false] {
        let mut f = Fixture::new();
        let index = f.add(Engine::Whisper, true, false);
        let p = &f.packages[index];
        let preview = f.preview(index, ProfileName::Standard);
        let before = f.storage.analysis_preferences().unwrap();
        if fail {
            f.storage.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_profile_head BEFORE INSERT ON app_settings WHEN NEW.key='managed_profile_heads' BEGIN SELECT RAISE(ABORT,'injected head failure'); END;").unwrap();
        }
        let calls = std::cell::Cell::new(0);
        let result = f.storage.apply_model_profile(
            &Uuid::new_v4().to_string(),
            &preview,
            &p.record,
            &f.models().join(&p.id),
            &|| {
                calls.set(calls.get() + 1);
                !fail && calls.get() >= 4
            },
        );
        assert!(result.is_err());
        if !fail {
            assert_eq!(
                calls.get(),
                4,
                "cancel after both preference and head writes, before commit"
            );
        }
        assert_eq!(
            f.storage.analysis_preferences().unwrap().payload,
            before.payload
        );
        assert_eq!(
            f.storage.analysis_preferences().unwrap().lock_version,
            before.lock_version
        );
        assert_eq!(f.count("model_profiles"), 0);
        assert_eq!(f.count("profile_components"), 0);
        assert!(f
            .storage
            .model_profiles()
            .unwrap()
            .iter()
            .all(|p| p.id.is_none()));
        if fail {
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute_batch("DROP TRIGGER fail_profile_head;")
                .unwrap();
        }
        assert_eq!(
            crate::model_application::apply(
                &f.storage,
                &f.models(),
                &Uuid::new_v4().to_string(),
                &preview,
                &|| false
            )
            .unwrap()
            .profile_version,
            1
        );
    }
}

#[test]
fn profile_application_missing_failed_or_nonlatest_compatibility_never_executes_or_becomes_applied()
{
    for condition in ["missing", "failed", "superseded", "cancelled"] {
        let mut f = Fixture::new();
        let index = f.add(Engine::Llama, condition != "missing", condition == "failed");
        let p = &f.packages[index];
        let marker = f
            .app_data
            .path()
            .join("model-checks")
            .join(&p.check)
            .join("executed.marker");
        let before = std::fs::read(&marker).ok();
        if condition == "superseded" || condition == "cancelled" {
            let new = Uuid::new_v4().to_string();
            f.storage.begin_model_check(&new, &p.record).unwrap();
            f.storage
                .finish_model_check(
                    &new,
                    &p.record,
                    None,
                    CheckState::Cancelled,
                    Some("fixture cancellation"),
                )
                .unwrap();
        }
        assert!(crate::model_application::preview(
            &f.storage,
            &f.models(),
            ProfileName::Standard,
            &p.id,
            &p.record.review_hash,
            &p.check,
            &|| false
        )
        .is_err());
        assert_eq!(std::fs::read(marker).ok(), before);
        assert_eq!(f.count("model_profiles"), 0);
        assert!(f
            .storage
            .model_profiles()
            .unwrap()
            .iter()
            .all(|v| v.id.is_none()));
    }
}

#[test]
fn profile_application_rechecks_current_model_and_trial_artifacts_without_changing_original_data() {
    for condition in ["model", "artifact", "link", "pre_cancel"] {
        let mut f = Fixture::new();
        let index = f.add(Engine::Llama, true, false);
        let preview = f.preview(index, ProfileName::Standard);
        let p = &f.packages[index];
        let before = f.storage.minutes_preferences().unwrap();
        let executed = f.executed(index);
        match condition {
            "model" => {
                std::fs::write(f.models().join(&p.id).join("model.bin"), b"changed").unwrap()
            }
            "artifact" => std::fs::write(
                f.app_data
                    .path()
                    .join("model-checks")
                    .join(&p.check)
                    .join("result.json"),
                b"{}",
            )
            .unwrap(),
            "link" => {
                let file = f.models().join(&p.id).join("model.bin");
                std::fs::rename(&file, file.with_extension("kept")).unwrap();
                std::os::unix::fs::symlink(p._source.path().join("model.bin"), &file).unwrap();
            }
            _ => (),
        }
        assert!(crate::model_application::apply(
            &f.storage,
            &f.models(),
            &Uuid::new_v4().to_string(),
            &preview,
            &|| condition == "pre_cancel"
        )
        .is_err());
        assert_eq!(f.count("model_profiles"), 0);
        assert_eq!(f.storage.minutes_preferences().unwrap(), before);
        assert_eq!(f.executed(index), executed);
    }
}

#[test]
fn profile_application_manual_preferences_and_new_trials_mark_configuration_outdated_without_rewriting_history(
) {
    let mut f = Fixture::new();
    let index = f.add(Engine::Llama, true, false);
    let first = f.apply(index, ProfileName::Standard);
    let id = first.id.clone().unwrap();
    let old: Vec<String> = f
        .storage
        .connection
        .lock()
        .unwrap()
        .prepare("SELECT settings_json FROM profile_components WHERE profile_id=?1 ORDER BY role")
        .unwrap()
        .query_map(params![id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut prefs = f.storage.minutes_preferences().unwrap().payload;
    prefs.standard.seed += 1;
    f.storage.save_minutes_preferences(2, prefs).unwrap();
    assert!(!f.storage.model_profiles().unwrap()[0].components[0].settings_match);
    let current = f.apply(index, ProfileName::Standard);
    assert!(current.components[0].settings_match);
    let p = &f.packages[index];
    let new = Uuid::new_v4().to_string();
    f.storage.begin_model_check(&new, &p.record).unwrap();
    f.storage
        .finish_model_check(
            &new,
            &p.record,
            None,
            CheckState::Failed,
            Some("fixture trial failed"),
        )
        .unwrap();
    let view = f.storage.model_profiles().unwrap();
    assert!(!view[0].components[0].compatibility_current);
    assert!(!view[0].configuration_complete);
    let db = f.storage.connection.lock().unwrap();
    let saved: Vec<String> = db
        .prepare("SELECT settings_json FROM profile_components WHERE profile_id=?1 ORDER BY role")
        .unwrap()
        .query_map(params![id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(saved, old);
    for sql in [
        "UPDATE model_profiles SET state='unavailable'",
        "DELETE FROM model_profiles",
        "UPDATE profile_components SET settings_json='{}'",
        "DELETE FROM profile_components",
    ] {
        assert!(db.execute_batch(sql).is_err());
    }
}

#[test]
fn profile_application_corrupted_heads_and_rehashed_invalid_component_are_not_silently_repaired() {
    for condition in [
        "missing_head",
        "head_hash",
        "component_hash",
        "invalid_settings",
    ] {
        let mut f = Fixture::new();
        let index = f.add(Engine::Llama, true, false);
        let first = f.apply(index, ProfileName::Standard);
        let id = first.id.unwrap();
        let db = f.storage.connection.lock().unwrap();
        match condition {
            "missing_head" => {
                db.execute(
                    "DELETE FROM app_settings WHERE key='managed_profile_heads'",
                    [],
                )
                .unwrap();
            }
            "head_hash" => {
                db.execute("UPDATE app_settings SET content_hash=replace(content_hash,'a','b') WHERE key='managed_profile_heads'",[]).unwrap();
                db.execute(
                    "UPDATE app_settings SET content_hash=?1 WHERE key='managed_profile_heads'",
                    params!["c".repeat(64)],
                )
                .unwrap();
            }
            "component_hash" => {
                db.execute_batch("DROP TRIGGER profile_component_fixed;")
                    .unwrap();
                let text: String = db
                    .query_row(
                        "SELECT settings_json FROM profile_components WHERE profile_id=?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .unwrap();
                db.execute(
                    "UPDATE profile_components SET settings_json=?1",
                    params![text.replace("777", "778")],
                )
                .unwrap();
            }
            "invalid_settings" => {
                let (mut heads, version) = heads(&db).unwrap();
                let mut stored =
                    load_profile(&db, ProfileName::Standard, heads.standard.as_ref().unwrap())
                        .unwrap();
                if let Settings::Llama(p) = &mut stored.bindings[0].settings {
                    p.threads = 0;
                }
                db.execute_batch("DROP TRIGGER profile_component_fixed;")
                    .unwrap();
                let text = serde_json::to_string(&ComponentRecord {
                    schema_version: 1,
                    application_hash: stored.application_hash.clone(),
                    binding: stored.bindings[0].clone(),
                })
                .unwrap();
                db.execute(
                    "UPDATE profile_components SET settings_json=?1",
                    params![text],
                )
                .unwrap();
                heads.standard.as_mut().unwrap().content_hash = digest(&stored).unwrap();
                persist(&db, "managed_profile_heads", version + 1, &heads).unwrap();
            }
            _ => unreachable!(),
        }
        drop(db);
        assert!(f.storage.model_profiles().is_err(), "{condition}");
        assert_eq!(f.count("model_profiles"), 1);
    }
}

#[test]
fn profile_application_v24_migration_and_backup_keep_models_and_configuration_local() {
    let mut f = Fixture::new();
    let index = f.add(Engine::Llama, true, false);
    let before = f.storage.list_meetings().unwrap();
    {
        let db = f.storage.connection.lock().unwrap();
        remove_schema_for_test(&db);
    }
    let models = f.models();
    let p = &f.packages[index];
    let package = p.id.clone();
    let review = p.record.review_hash.clone();
    let check = p.check.clone();
    drop(f.storage);
    let storage = Storage::open(f.root.path().into()).unwrap();
    assert_eq!(storage.list_meetings().unwrap(), before);
    assert!(storage.model_package_for_check(&package, &review).is_ok());
    assert!(storage
        .model_profiles()
        .unwrap()
        .iter()
        .all(|p| p.id.is_none()));
    let preview = crate::model_application::preview(
        &storage,
        &models,
        ProfileName::HighAccuracy,
        &package,
        &review,
        &check,
        &|| false,
    )
    .unwrap();
    let operation = Uuid::new_v4().to_string();
    let view = crate::model_application::apply(&storage, &models, &operation, &preview, &|| false)
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let archive = output.path().join("profiles.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let bytes = std::fs::read(archive).unwrap();
    for local in [
        operation.as_bytes(),
        b"managed_profile_heads",
        b"model-checks",
        models.to_str().unwrap().as_bytes(),
    ] {
        assert!(!bytes.windows(local.len()).any(|b| b == local));
    }
    drop(storage);
    let reopened = Storage::open(f.root.path().into()).unwrap();
    assert_eq!(reopened.model_profiles().unwrap()[1], view);
}

#[test]
fn profile_application_backup_maintenance_blocks_publication_and_later_application_preserves_snapshot(
) {
    let mut f = Fixture::new();
    let index = f.add(Engine::Whisper, true, false);
    let preview = f.preview(index, ProfileName::Standard);
    let before = f.storage.analysis_preferences().unwrap();
    let old = f.storage.fixed_job_input(&f.baseline.id).unwrap();
    let snapshot = f.storage.prepare_backup_snapshot(&|| false).unwrap();
    assert!(crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &preview,
        &|| false
    )
    .is_err());
    assert_eq!(f.count("model_profiles"), 0);
    assert_eq!(
        f.storage.analysis_preferences().unwrap().payload,
        before.payload
    );
    assert_eq!(
        f.storage.analysis_preferences().unwrap().lock_version,
        before.lock_version
    );
    assert_eq!(f.storage.fixed_job_input(&f.baseline.id).unwrap(), old);
    drop(snapshot);
    assert_eq!(
        crate::model_application::apply(
            &f.storage,
            &f.models(),
            &Uuid::new_v4().to_string(),
            &preview,
            &|| false
        )
        .unwrap()
        .profile_version,
        1
    );
}
