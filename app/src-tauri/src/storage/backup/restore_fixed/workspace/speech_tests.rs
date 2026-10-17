use super::*;
use crate::storage::JobSnapshot;

struct Fixture {
    root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    job: JobSnapshot,
}
fn transcription(partial: bool, queued: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, mut engine) =
        crate::storage::transcription::tests::fixture(root.path(), if partial { 124 } else { 10 });
    let script = std::fs::read_to_string(&engine.executable.path).unwrap();
    std::fs::write(
        &engine.executable.path,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\n{}",
            root.path().join("cli-calls").display(),
            script.strip_prefix("#!/bin/sh\n").unwrap()
        ),
    )
    .unwrap();
    engine.executable = crate::local_engine::Fingerprint::capture(
        &engine.executable.path,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let job =
        crate::transcription::enqueue(&storage, &meeting, "speech-restore", engine, "standard")
            .unwrap();
    if !queued {
        storage.acquire_local_inference_job().unwrap().unwrap();
        let candidate = crate::transcription::process(
            &storage,
            &job.id,
            &crate::test_support::never_cancelled,
            &|j| {
                if partial && j.completed_units == 1 && j.state == "running" {
                    storage.request_job_pause(&j.id, "user").unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(candidate.is_none(), partial);
    }
    let job = storage.get_job(&job.id).unwrap();
    Fixture {
        root,
        storage,
        meeting,
        job,
    }
}
fn diarization(partial: bool, queued: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, engine, request) =
        crate::storage::diarization::tests::fixture(root.path(), true);
    let job = crate::storage::diarization::tests::enqueue(&storage, request, engine);
    if !queued {
        if partial {
            crate::storage::diarization::tests::pause_after_first(&storage, &job.id);
        } else {
            storage.acquire_local_inference_job().unwrap().unwrap();
            crate::diarization::process(
                &storage,
                &job.id,
                &crate::test_support::never_cancelled,
                &|_| {},
            )
            .unwrap()
            .unwrap();
        }
    }
    let job = storage.get_job(&job.id).unwrap();
    Fixture {
        root,
        storage,
        meeting,
        job,
    }
}
fn calls(f: &Fixture) -> Vec<u8> {
    std::fs::read(f.root.path().join("cli-calls")).unwrap_or_default()
}
fn read(f: &Fixture, output: &Path) -> archive::VerifiedArchive {
    crate::storage::backup::restore_fixed::tests::read(&f.storage, output)
}
fn ids<'a>(source: &'a archive::VerifiedArchive, output: &Path) -> RestoreIds<'a> {
    RestoreIds::prepare(
        source,
        output,
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
}
fn result_id(f: &Fixture) -> String {
    let checkpoint = f.storage.fixed_job_input(&f.job.id).unwrap().1.unwrap();
    checkpoint["outputs"][0][if f.job.kind == "transcription" {
        "jsonAssetId"
    } else {
        "resultAssetId"
    }]
    .as_str()
    .unwrap()
    .into()
}
fn ready_files_and_missing_rows(
    work: &FixedWorkspace,
    source: &archive::VerifiedArchive,
    ids: &RestoreIds<'_>,
) {
    let mut query = work
        .database
        .prepare("SELECT id,relative_path,state,size_bytes,content_hash FROM file_assets")
        .unwrap();
    let mut rows = query.query([]).unwrap();
    let mut missing = std::collections::BTreeSet::new();
    while let Some(row) = rows.next().unwrap() {
        let id: String = row.get(0).unwrap();
        let relative: String = row.get(1).unwrap();
        let state: String = row.get(2).unwrap();
        let size: i64 = row.get(3).unwrap();
        let hash: String = row.get(4).unwrap();
        let path = work.stage.path().join(relative);
        match state.as_str() {
            "ready" => assert_eq!(
                digest_file(&path, &crate::test_support::never_cancelled).unwrap(),
                (size as u64, hash)
            ),
            "missing" => {
                assert!(!path.exists());
                missing.insert(id);
            }
            other => panic!("unexpected file state {other}"),
        }
    }
    let expected = source
        .manifest
        .omitted_assets
        .iter()
        .map(|a| ids.get("file_assets", &a.meeting_id, &a.asset_id).unwrap())
        .collect();
    assert_eq!(missing, expected);
    assert_eq!(work.missing_assets, missing);
}

#[test]
fn speech_workspace_rebuilds_native_results_and_hashes_without_engines_or_missing_pcm_logs() {
    for f in [transcription(false, false), diarization(false, false)] {
        let output = tempfile::tempdir().unwrap();
        let before = calls(&f);
        let draft = f.storage.load_transcript_draft(&f.meeting).unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        // Only fake fixture engines/models are removed, never user resources.
        let names: &[&str] = if f.job.kind == "transcription" {
            &["fake-cli", "model.bin"]
        } else {
            &["diarization-cli", "segmentation.onnx", "embedding.onnx"]
        };
        for name in names {
            std::fs::remove_file(f.root.path().join(name)).unwrap();
        }
        let mut work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap()
                .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
                .unwrap()
                .with_generation_artifacts(&fixed, &crate::test_support::never_cancelled)
                .unwrap();
        let stage = work.stage.path().to_path_buf();
        let job = ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap();
        let (spec, checkpoint) =
            crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
        assert_eq!(
            spec.meeting_id,
            ids.get("meetings", "", &f.meeting).unwrap()
        );
        let old_id = result_id(&f);
        let original: Value =
            serde_json::from_slice(&std::fs::read(&source.assets[&old_id]).unwrap()).unwrap();
        let new_id = ids.get("file_assets", &f.meeting, &old_id).unwrap();
        let path: String = work
            .database
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1",
                params![new_id],
                |r| r.get(0),
            )
            .unwrap();
        let result: Value =
            serde_json::from_slice(&std::fs::read(stage.join(path)).unwrap()).unwrap();
        if f.job.kind == "transcription" {
            assert_eq!(result["transcription"], original["transcription"]);
            assert!(result["params"]["model"]
                .as_str()
                .unwrap()
                .starts_with("mon-unavailable:"));
            assert_eq!(checkpoint.unwrap()["outputs"][0]["jsonAssetId"], new_id);
        } else {
            assert_eq!(result["segments"], original["segments"]);
            assert_eq!(result["trackStartUs"], original["trackStartUs"]);
            assert_eq!(result["durationUs"], original["durationUs"]);
            assert_eq!(
                result["inputAnalysisId"],
                spec.input_analysis_id.clone().unwrap()
            );
            assert_eq!(result["inputHash"], spec.input_hash);
            assert_eq!(
                result["trackId"],
                ids.get(
                    "audio_tracks",
                    &f.meeting,
                    original["trackId"].as_str().unwrap()
                )
                .unwrap()
            );
            assert!(result["engine"]["segmentationModel"]["path"]
                .as_str()
                .unwrap()
                .starts_with("mon-unavailable:"));
            let tx = work.database.transaction().unwrap();
            crate::storage::diarization::read_input(&tx, &spec).unwrap();
            tx.commit().unwrap();
        }
        ready_files_and_missing_rows(&work, &source, &ids);
        crate::storage::backup::validate_database(
            &mut work.database,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(calls(&f), before);
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
        assert_eq!(f.storage.load_transcript_draft(&f.meeting).unwrap(), draft);
        drop(work);
        assert!(!stage.exists());
    }
}

#[test]
fn speech_workspace_rebuilds_partial_history_and_shared_retry_without_reexecuting() {
    for f in [transcription(true, false), diarization(true, false)] {
        let parent = f
            .storage
            .finish_job(&f.job.id, f.job.completed_units, "failed", Some("fixture"))
            .unwrap();
        let child = if f.job.kind == "transcription" {
            crate::transcription::RetryRequest {
                meeting_id: f.meeting.clone(),
                job_id: parent.id.clone(),
                operation_id: "speech-retry".into(),
                expected_spec_hash: parent.spec_hash.clone(),
            }
            .enqueue(&f.storage)
            .unwrap()
        } else {
            crate::diarization::RetryRequest {
                meeting_id: f.meeting.clone(),
                job_id: parent.id.clone(),
                operation_id: "speech-retry".into(),
                expected_spec_hash: parent.spec_hash.clone(),
            }
            .enqueue(&f.storage)
            .unwrap()
        };
        let output = tempfile::tempdir().unwrap();
        let before = calls(&f);
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let mut work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap()
                .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
                .unwrap();
        let mapped = ids.get("processing_jobs", &f.meeting, &child.id).unwrap();
        let (_, checkpoint) =
            crate::storage::jobs::read_fixed_input(&work.database, &mapped).unwrap();
        assert_eq!(checkpoint.unwrap()["outputs"].as_array().unwrap().len(), 1);
        let rebuilt = crate::storage::jobs::load_job(&work.database, &mapped)
            .unwrap()
            .unwrap();
        assert_eq!(
            rebuilt.retry_of_job_id,
            Some(ids.get("processing_jobs", &f.meeting, &parent.id).unwrap())
        );
        assert_eq!(rebuilt.attempt, 2);
        assert_eq!(rebuilt.completed_units, 1);
        let count: i64 = work
            .database
            .query_row("SELECT count(*) FROM job_checkpoints", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 3);
        ready_files_and_missing_rows(&work, &source, &ids);
        crate::storage::backup::validate_database(
            &mut work.database,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(calls(&f), before);
        assert_eq!(f.storage.get_job(&parent.id).unwrap(), parent);
    }
}

#[test]
fn speech_workspace_checks_unstarted_jobs_without_creating_results_or_checkpoints() {
    for f in [transcription(false, true), diarization(false, true)] {
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap()
                .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
                .unwrap();
        let mapped = ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap();
        assert!(
            crate::storage::jobs::read_fixed_input(&work.database, &mapped)
                .unwrap()
                .1
                .is_none()
        );
        assert_eq!(
            work.database
                .query_row("SELECT count(*) FROM job_checkpoints", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        ready_files_and_missing_rows(&work, &source, &ids);
        assert!(calls(&f).is_empty());
    }
}

#[test]
fn speech_workspace_rejects_changed_verified_payload_and_preserves_original_data() {
    for f in [transcription(false, false), diarization(false, false)] {
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap();
        let stage = work.stage.path().to_path_buf();
        let before = calls(&f);
        let asset = result_id(&f);
        std::fs::write(&source.assets[&asset], b"{}").unwrap();
        assert!(work
            .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
            .is_err());
        assert!(!stage.exists());
        assert!(source.assets[&asset].is_file());
        assert_eq!(calls(&f), before);
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    }
}

#[test]
fn speech_workspace_rejects_semantically_invalid_native_results_even_with_valid_payload_digest() {
    for change in [
        "model-name",
        "transcription-time",
        "transcription-start",
        "diarization-track",
        "diarization-time",
        "diarization-engine",
    ] {
        let f = if change.starts_with("diarization") {
            diarization(false, false)
        } else {
            transcription(false, false)
        };
        let output = tempfile::tempdir().unwrap();
        let mut source = read(&f, output.path());
        let asset = result_id(&f);
        let mut result: Value =
            serde_json::from_slice(&std::fs::read(&source.assets[&asset]).unwrap()).unwrap();
        match change {
            "model-name" => result["params"]["model"] = json!("unrelated-model.bin"),
            // A bounded final decoded window is valid padding, not a corrupt
            // result. Reject a window longer than the decoder's 30s context.
            "transcription-time" => {
                let from = result["transcription"][0]["offsets"]["from"]
                    .as_i64()
                    .unwrap();
                result["transcription"][0]["offsets"]["to"] = json!(from + 30_001);
            }
            "transcription-start" => {
                result["transcription"][0]["offsets"]["from"] = json!(10_000);
                result["transcription"][0]["offsets"]["to"] = json!(11_000);
            }
            // Same meeting/duration/settings: a correct native result must also
            // belong to the exact checkpoint position, not just any track.
            "diarization-track" => result["trackId"] = json!("system-track"),
            "diarization-time" => result["segments"][0]["endUs"] = json!(11_000_000),
            "diarization-engine" => result["engine"]["threads"] = json!(2),
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&result).unwrap();
        std::fs::write(&source.assets[&asset], &bytes).unwrap();
        // Test the semantic pass independently of the physical SHA guard. Only
        // this test-owned validated payload descriptor is changed; source SQL
        // remains query_only and original logical identity proofs stay intact.
        let entry = source
            .manifest
            .entries
            .iter_mut()
            .find(|e| e.asset_id.as_deref() == Some(&asset))
            .unwrap();
        entry.size = bytes.len() as u64;
        entry.sha256 = format!("{:x}", Sha256::digest(&bytes));
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap();
        let stage = work.stage.path().to_path_buf();
        let before = calls(&f);
        let error = work
            .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
            .err()
            .unwrap_or_else(|| panic!("invalid {change} was accepted"));
        if change == "diarization-track" {
            assert!(error.contains("チェックポイントのトラック"), "{error}");
        }
        if change == "model-name" {
            assert!(error.contains("モデル名"), "{error}");
        }
        assert!(!stage.exists());
        assert_eq!(calls(&f), before);
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
        assert_eq!(
            source
                .database
                .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn speech_workspace_cancellation_at_entry_and_after_write_removes_only_its_stage() {
    for f in [transcription(false, false), diarization(false, false)] {
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let marker = output.path().join("keep.txt");
        std::fs::write(&marker, b"keep").unwrap();
        for after_write in [false, true] {
            let ids = ids(&source, output.path());
            let fixed =
                FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
            let work = FixedWorkspace::prepare(
                &fixed,
                output.path(),
                &crate::test_support::never_cancelled,
            )
            .unwrap();
            let stage = work.stage.path().to_path_buf();
            let asset = ids.get("file_assets", &f.meeting, &result_id(&f)).unwrap();
            let file = stage.join(format!(
                "meetings/{}/restored/{asset}",
                ids.get("meetings", "", &f.meeting).unwrap()
            ));
            let seen = std::cell::Cell::new(false);
            assert!(work
                .with_speech_artifacts(&fixed, &|| {
                    if file.exists() {
                        seen.set(true);
                    }
                    !after_write || seen.get()
                })
                .is_err());
            assert_eq!(seen.get(), after_write);
            assert!(!stage.exists());
            assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
            assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
        }
    }
}

#[test]
fn speech_workspace_rejects_source_and_destination_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;
    for source_link in [true, false] {
        let f = transcription(false, false);
        let output = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let keep = outside.path().join("keep.txt");
        std::fs::write(&keep, b"keep").unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap();
        let stage = work.stage.path().to_path_buf();
        if source_link {
            let result = &source.assets[&result_id(&f)];
            let body = outside.path().join("result.json");
            std::fs::rename(result, &body).unwrap();
            symlink(body, result).unwrap();
        } else {
            let relative: String = work
                .database
                .query_row(
                    "SELECT relative_path FROM file_assets WHERE id=?1",
                    params![ids.get("file_assets", &f.meeting, &result_id(&f)).unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            symlink(&keep, stage.join(relative)).unwrap();
        }
        assert!(work
            .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
            .is_err());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&keep).unwrap(), b"keep");
        assert_eq!(
            std::fs::read_dir(outside.path()).unwrap().count(),
            if source_link { 2 } else { 1 }
        );
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    }
}

#[test]
fn speech_workspace_rejects_source_and_target_fixed_settings_mutations() {
    for source_settings in [true, false] {
        let f = transcription(false, false);
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work =
            FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap();
        let stage = work.stage.path().to_path_buf();
        let writer;
        let (db, job) = if source_settings {
            let path: String = source
                .database
                .query_row(
                    "SELECT file FROM pragma_database_list WHERE name='main'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(Path::new(&path).starts_with(output.path().canonicalize().unwrap()));
            writer = Connection::open(path).unwrap();
            (&writer, f.job.id.clone())
        } else {
            (
                &work.database,
                ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap(),
            )
        };
        db.execute("UPDATE processing_jobs SET parameters_json=json_set(parameters_json,'$.engine.threads',2) WHERE id=?1", params![job]).unwrap();
        assert!(work
            .with_speech_artifacts(&fixed, &crate::test_support::never_cancelled)
            .is_err());
        assert!(!stage.exists());
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    }
}
