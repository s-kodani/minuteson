use super::*;
use serde_json::json;

fn engine(
    root: &Path,
    seconds: i64,
    fail_plain: bool,
) -> (Storage, String, WhisperConfig, PathBuf) {
    let (storage, meeting, mut config) = fixture(root, seconds);
    let calls = root.join("calls.jsonl");
    let model = &config.model.path;
    let bad=json!({"model":{"multilingual":true},"params":{"model":model,"language":"ja","translate":false},"transcription":[{"offsets":{"from":2500,"to":3000},"text":"無条件な本文置換ではありません。".repeat(4)}]}).to_string();
    let good=json!({"model":{"multilingual":true},"params":{"model":model,"language":"ja","translate":false},"transcription":[{"offsets":{"from":2500,"to":3000},"text":"通常の会議内容を確認しています。"}]}).to_string();
    let script=format!("#!/usr/bin/env python3\nimport json,sys\nwith open({:?},'a') as f: f.write(json.dumps(sys.argv[1:])+'\\n')\nresult={bad:?} if '--prompt' in sys.argv or {fail_plain:?} else {good:?}\nwith open('result.json','w') as f: f.write(result)\n",calls.to_str().unwrap()).replace("or false", "or False").replace("or true", "or True");
    std::fs::write(&config.executable.path, script).unwrap();
    config.executable = Fingerprint::capture(&config.executable.path, &|| false).unwrap();
    storage
        .save_vocabulary_book(None, 0, vec![vocabulary_entry("点検日")])
        .unwrap();
    (storage, meeting, config, calls)
}
fn call_args(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
#[test]
fn guarded_transcription_fallback_is_durable_and_resume_keeps_the_original_vocabulary() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config, calls) = engine(root.path(), 124, false);
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(&storage, &job.id, &|| false, &|s| {
        if s.completed_units == 1 && s.state == "running" {
            storage.request_job_pause(&job.id, "user").unwrap();
        }
    })
    .unwrap()
    .is_none());
    let (_, checkpoint) = storage.fixed_job_input(&job.id).unwrap();
    let first = &checkpoint.as_ref().unwrap()["outputs"][0];
    assert_eq!(first["recognition"]["mode"], "fallback");
    assert_eq!(first["recognition"]["reasons"], json!(["repetition"]));
    assert!(first["recognition"]["rejected"]["jsonAssetId"].is_string());
    assert_ne!(
        first["jsonAssetId"],
        first["recognition"]["rejected"]["jsonAssetId"]
    );
    storage.save_vocabulary_book(None, 1, vec![]).unwrap();
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage.resume_job(&job.id).unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    let candidate = process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let args = call_args(&calls);
    assert_eq!(args.len(), 4);
    for pair in args.chunks_exact(2) {
        let p = pair[0].iter().position(|a| a == "--prompt").unwrap();
        assert!(pair[0][p + 1].contains("点検日"));
        assert!(!pair[1].iter().any(|a| a == "--prompt"));
        assert!(pair.iter().all(|a| a.iter().any(|v| v == "--suppress-nst")));
    }
    let visible = storage
        .load_visible_analysis(&meeting, &candidate)
        .unwrap()
        .unwrap();
    assert!(visible
        .utterances
        .iter()
        .all(|u| u.recognized_text == "通常の会議内容を確認しています。"));
    let checkpoint2 = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    assert_eq!(checkpoint2["outputs"][0], checkpoint.unwrap()["outputs"][0]);
}
#[test]
fn guarded_transcription_finishes_with_review_ranges_instead_of_publishing_repetition() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config, calls) = engine(root.path(), 4, true);
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    let candidate = process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(call_args(&calls).len(), 3);
    let view = storage
        .load_visible_analysis(&meeting, &candidate)
        .unwrap()
        .unwrap();
    assert!(view.utterances.is_empty());
    assert_eq!(view.review_ranges.len(), 1);
    assert_eq!(storage.get_job(&job.id).unwrap().state, "succeeded");
}

#[test]
fn guarded_transcription_checks_rejected_artifacts_before_resume_or_retry() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config, calls) = engine(root.path(), 124, false);
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    process(&storage, &job.id, &|| false, &|s| {
        if s.completed_units == 1 && s.state == "running" {
            storage.request_job_pause(&job.id, "user").unwrap();
        }
    })
    .unwrap();
    let cp = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    let id = cp["outputs"][0]["recognition"]["rejected"]["jsonAssetId"]
        .as_str()
        .unwrap();
    let path: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(storage.root.join(path), b"tampered").unwrap();
    storage.resume_job(&job.id).unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(&storage, &job.id, &|| false, &|_| {}).is_err());
    let failed = storage
        .finish_job(&job.id, 1, "failed", Some("test"))
        .unwrap();
    assert!(RetryRequest {
        meeting_id: meeting,
        job_id: job.id,
        operation_id: "retry".into(),
        expected_spec_hash: failed.spec_hash
    }
    .enqueue(&storage)
    .is_err());
    assert_eq!(call_args(&calls).len(), 2);
}

#[test]
#[ignore = "requires a private MON_VOCABULARY_CHECK_ROOT prepared from copied meeting data"]
fn real_guarded_vocabulary_worker_checks_all_units_against_unhinted_audio() {
    use crate::transcription::whisper;
    use std::time::{Duration, Instant};
    let root = PathBuf::from(std::env::var("MON_VOCABULARY_CHECK_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("verification-input.json")).unwrap())
            .unwrap();
    let meeting = marker["meetingId"].as_str().unwrap();
    let engine: WhisperConfig = serde_json::from_value(marker["engine"].clone()).unwrap();
    engine.verify(&|| false).unwrap();
    let storage = Storage::open_with_models(
        root.clone(),
        PathBuf::from(marker["modelsRoot"].as_str().unwrap()),
    )
    .unwrap();
    let original = storage.load_current_analysis(meeting).unwrap().unwrap();
    let started = Instant::now();
    let job = enqueue(
        &storage,
        meeting,
        "vocabulary-v2-real-verification",
        WhisperConfig {
            prompt: String::new(),
            ..engine.clone()
        },
        "standard",
    )
    .unwrap();
    assert_eq!(job.total_units, Some(19));
    assert_eq!(
        storage
            .acquire_job_kind(Some(JobKind::Transcription))
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    let outcome = process(&storage, &job.id, &|| false, &|s| {
        eprintln!(
            "real vocabulary worker: {}/{} units",
            s.completed_units,
            s.total_units.unwrap_or(0)
        );
    });
    if let Err(error) = &outcome {
        std::fs::write(root.join("worker-error.txt"), error).unwrap();
    }
    let candidate = outcome.unwrap().unwrap();
    let (spec, cp) = storage.fixed_job_input(&job.id).unwrap();
    let cp = cp.unwrap();
    std::fs::write(
        root.join("worker-spec.json"),
        serde_json::to_vec_pretty(&spec.parameters).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("worker-checkpoint.json"),
        serde_json::to_vec_pretty(&cp).unwrap(),
    )
    .unwrap();
    let view = storage
        .load_visible_analysis(meeting, &candidate)
        .unwrap()
        .unwrap();
    std::fs::write(
        root.join("worker-transcript.json"),
        serde_json::to_vec_pretty(&view).unwrap(),
    )
    .unwrap();
    assert_eq!(
        storage.load_current_analysis(meeting).unwrap().unwrap(),
        original
    );
    assert!(!view.utterances.is_empty());
    for u in &view.utterances {
        assert!(!u.recognized_text.contains("無条件な本文置換ではありません"));
        assert!(!u.recognized_text.contains("表記・読みの認識ヒント"));
        assert!(
            u.start_us >= 0
                && u.end_us > u.start_us
                && u.end_us
                    <= spec.parameters["source"]["tracks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|t| t["endUs"].as_i64().unwrap())
                        .max()
                        .unwrap()
        );
    }
    let worker_seconds = started.elapsed().as_secs_f64();
    let mut comparisons = vec![];
    let mut plain = engine;
    plain.prompt.clear();
    plain.use_gpu = cfg!(target_os = "macos");
    for output in cp["outputs"].as_array().unwrap() {
        let index = output["index"].as_u64().unwrap();
        let asset = output["wavAssetId"].as_str().unwrap();
        let (path, hash): (String, String) = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT relative_path,content_hash FROM file_assets WHERE id=?1",
                [asset],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let source = root.join(path);
        assert_eq!(
            Fingerprint::capture(&source, &|| false).unwrap().sha256,
            hash
        );
        let d = root.join(format!("unhinted-reference-{index}"));
        std::fs::create_dir(&d).unwrap();
        std::fs::copy(source, d.join("input.wav")).unwrap();
        let unit = &spec.parameters["units"][index as usize];
        let rate = spec.parameters["source"]["tracks"][0]["sampleRate"]
            .as_u64()
            .unwrap();
        let duration = ((unit["windowEnd"].as_u64().unwrap()
            - unit["windowFirst"].as_u64().unwrap())
            * 1_000_000
            / rate) as i64;
        let now = Instant::now();
        let reference = whisper::run_unit_with_suppression(
            &plain,
            &d,
            duration,
            Duration::from_secs(1800),
            true,
            &|| false,
        )
        .unwrap()
        .unwrap();
        let selected_id = output["jsonAssetId"].as_str().unwrap();
        let selected_hash = output["jsonHash"].as_str().unwrap();
        let selected = storage
            .read_analysis_result(meeting, selected_id, selected_hash)
            .unwrap();
        let selected = whisper::parse_output(&selected, &plain.model.path, duration).unwrap();
        let coverage = |s: &[whisper::Segment]| -> i64 {
            let mut last = 0;
            let mut total = 0;
            for seg in s {
                total += (seg.end_us - seg.start_us.max(last)).max(0);
                last = last.max(seg.end_us);
            }
            total
        };
        let reference_usable = !crate::transcription::reference_is_repetitive(&reference);
        let item = json!({"index":index,"recognition":output["recognition"],"referenceSeconds":now.elapsed().as_secs_f64(),"selectedSegments":selected.len(),"referenceSegments":reference.len(),"referenceUsable":reference_usable,"selectedCoveredUs":coverage(&selected),"referenceCoveredUs":coverage(&reference),"textExactMatch":selected==reference});
        comparisons.push(item);
        std::fs::write(root.join("real-comparison.json"),serde_json::to_vec_pretty(&json!({"jobId":job.id,"candidateId":candidate,"workerSeconds":worker_seconds,"comparisons":comparisons})).unwrap()).unwrap();
        eprintln!("real vocabulary reference: {}/19 units", index + 1);
    }
    assert_eq!(comparisons.len(), 19);
    assert!(comparisons.iter().all(|c|c["referenceUsable"] == false || c["selectedCoveredUs"].as_i64().unwrap()*100>=c["referenceCoveredUs"].as_i64().unwrap()*85),"A unit lost more than 15% of reference speech coverage; inspect private comparison artifacts");
    assert_eq!(
        storage.load_current_analysis(meeting).unwrap().unwrap(),
        original
    );
}

#[test]
fn guarded_transcription_pause_during_fallback_does_not_checkpoint_initial_result() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, mut config, calls) = engine(root.path(), 4, false);
    let marker = root.path().join("fallback-started");
    let script=std::fs::read_to_string(&config.executable.path).unwrap().replace("import json,sys",&format!("import json,sys,time\nif '--prompt' not in sys.argv:\n open({:?},'w').close()\n time.sleep(5)",marker.to_str().unwrap()));
    std::fs::write(&config.executable.path, script).unwrap();
    config.executable = Fingerprint::capture(&config.executable.path, &|| false).unwrap();
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    assert!(process(&storage, &job.id, &|| marker.exists(), &|_| {})
        .unwrap()
        .is_none());
    assert_eq!(call_args(&calls).len(), 1); // paused inside the second CLI before it records output
    let job = storage.get_job(&job.id).unwrap();
    assert_eq!(job.state, "paused");
    assert_eq!(job.completed_units, 0);
    assert!(storage.load_current_analysis(&meeting).unwrap().is_none());
}

#[test]
fn guarded_transcription_restore_maps_and_validates_both_selected_and_rejected_results() {
    use crate::saved_artifact::{RestoreAssets, Role};
    struct Assets<'a> {
        storage: &'a Storage,
        meeting: &'a str,
        writes: Vec<(String, Vec<u8>)>,
    }
    impl RestoreAssets for Assets<'_> {
        fn read(&mut self, role: Role, id: &str, hash: &str) -> Result<Vec<u8>, String> {
            assert_eq!(role, Role::TranscriptionResult);
            self.storage.read_analysis_result(self.meeting, id, hash)
        }
        fn write(
            &mut self,
            role: Role,
            id: &str,
            hash: &str,
            bytes: &[u8],
        ) -> Result<(String, String), String> {
            assert_eq!(role, Role::TranscriptionResult);
            self.writes.push((id.into(), bytes.into()));
            Ok((id.into(), hash.into()))
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config, _) = engine(root.path(), 4, false);
    let model = config.model.path.to_string_lossy().into_owned();
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let job = storage.get_job(&job.id).unwrap();
    let (spec, cp) = storage.fixed_job_input(&job.id).unwrap();
    let cp = cp.unwrap();
    let mut mapped_results = 0;
    let mapped =
        crate::transcription::restore_checkpoint_references(&job, cp.clone(), |role, id, hash| {
            if role == Role::TranscriptionResult {
                mapped_results += 1;
            }
            Ok((format!("restored-{id}"), hash.into()))
        })
        .unwrap();
    assert_eq!(mapped_results, 2);
    assert!(
        mapped["outputs"][0]["recognition"]["rejected"]["jsonAssetId"]
            .as_str()
            .unwrap()
            .starts_with("restored-")
    );
    let mut assets = Assets {
        storage: &storage,
        meeting: &meeting,
        writes: vec![],
    };
    crate::transcription::restore_saved_results(
        &job,
        &spec,
        &spec,
        cp.clone(),
        &[model.clone()],
        &mut assets,
        &|| false,
    )
    .unwrap();
    assert_eq!(assets.writes.len(), 2);
    for (id, bytes) in &assets.writes {
        let hash = if id == cp["outputs"][0]["jsonAssetId"].as_str().unwrap() {
            cp["outputs"][0]["jsonHash"].as_str().unwrap()
        } else {
            cp["outputs"][0]["recognition"]["rejected"]["jsonHash"]
                .as_str()
                .unwrap()
        };
        let original = storage.read_analysis_result(&meeting, id, hash).unwrap();
        let before: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let after: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(before, after);
    }
    let mut forged = cp;
    forged["outputs"][0]["recognition"]["reasons"] = json!([]);
    assets.writes.clear();
    assert!(crate::transcription::restore_saved_results(
        &job,
        &spec,
        &spec,
        forged,
        &[model],
        &mut assets,
        &|| false
    )
    .is_err());
    assert!(assets.writes.is_empty());
}

#[test]
fn guarded_transcription_retries_a_hint_induced_timestamp_gap_without_clipping_pauses() {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, mut config, calls) = engine(root.path(), 40, false);
    let gap=json!({"model":{"multilingual":true},"params":{"model":config.model.path,"language":"ja","translate":false},"transcription":[{"offsets":{"from":0,"to":2000},"text":"お待ちください"},{"offsets":{"from":31000,"to":38000},"text":"次の内容をご説明します"}]}).to_string();
    let mut script = std::fs::read_to_string(&config.executable.path).unwrap();
    let from = script.find("result=").unwrap();
    let to = script[from..].find(" if '--prompt'").unwrap() + from;
    script.replace_range(from..to, &format!("result={gap:?}"));
    std::fs::write(&config.executable.path, script).unwrap();
    config.executable = Fingerprint::capture(&config.executable.path, &|| false).unwrap();
    let job = enqueue(&storage, &meeting, "guard", config, "standard").unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let cp = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    assert_eq!(cp["outputs"][0]["recognition"]["mode"], "fallback");
    assert_eq!(
        cp["outputs"][0]["recognition"]["reasons"],
        json!(["timestampGap"])
    );
    assert_eq!(call_args(&calls).len(), 2);
}
