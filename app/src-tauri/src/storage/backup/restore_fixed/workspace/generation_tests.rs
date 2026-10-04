use super::*;
use crate::storage::{DraftStamp, MinutesJobRequest};

struct NativeFixture {
    root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    job: JobSnapshot,
}
fn fixture(long: bool, partial: bool) -> NativeFixture {
    let mut f = queued(long);
    if partial {
        crate::storage::generation_tests::pause_in_merge(&f.storage, &f.job);
    } else {
        f.storage.acquire_local_inference_job().unwrap();
        crate::generation::process(&f.storage, &f.job.id, &|| false, &|_| {})
            .unwrap()
            .unwrap();
    }
    f.job = f.storage.get_job(&f.job.id).unwrap();
    f
}
fn queued(long: bool) -> NativeFixture {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = crate::storage::backup::archive::tests::meeting(&storage, "generation-workspace");
    let current = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    if long {
        draft.payload.utterances = vec![crate::storage::drafts::UtteranceEdit {
            utterance_id: current.utterances[0].id.clone(),
            effective_text: "点検日を確認することに決まりました。".repeat(45),
            speaker_ids: vec![],
        }];
        draft = storage
            .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
            .unwrap();
    }
    let request = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: current.id,
        expected_content_hash: current.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let engine = if long {
        crate::storage::generation_tests::wide_engine(root.path())
    } else {
        crate::storage::generation_tests::normal_engine(root.path())
    };
    let job = crate::generation::enqueue(&storage, request, engine, "standard", 2400).unwrap();
    NativeFixture {
        root,
        storage,
        meeting,
        job,
    }
}
fn calls(f: &NativeFixture) -> String {
    std::fs::read_to_string(f.root.path().join("engine.calls")).unwrap()
}
fn archive(f: &NativeFixture, dir: &Path) -> archive::VerifiedArchive {
    crate::storage::backup::restore_fixed::tests::read(&f.storage, dir)
}
fn prepared<'a>(source: &'a archive::VerifiedArchive, output: &Path) -> RestoreIds<'a> {
    RestoreIds::prepare(source, output, &Uuid::new_v4().to_string(), &|| false).unwrap()
}

#[test]
fn generation_workspace_rebuilds_worker_files_and_all_checkpoint_hashes_without_inference() {
    let f = fixture(false, false);
    let output = tempfile::tempdir().unwrap();
    let old_draft = f.storage.load_transcript_draft(&f.meeting).unwrap();
    let before = calls(&f);
    let source = archive(&f, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    // These are owned synthetic fixture files, not installed engines/models.
    std::fs::remove_file(f.root.path().join("model.gguf")).unwrap();
    std::fs::remove_file(f.root.path().join("llama-completion")).unwrap();
    let mut work = FixedWorkspace::prepare(&fixed, output.path(), &|| false)
        .unwrap()
        .with_generation_artifacts(&fixed, &|| false)
        .unwrap();
    let stage = work.stage.path().to_path_buf();
    let new_job = ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap();
    let tx = work.database.transaction().unwrap();
    let (spec, checkpoint) = crate::storage::jobs::read_fixed_input(&tx, &new_job).unwrap();
    let input = crate::storage::minutes_input::read_input(&tx, &spec).unwrap();
    let checkpoint = checkpoint.unwrap();
    let normalized = checkpoint["outputs"][0]["normalized"]["id"]
        .as_str()
        .unwrap();
    let path: String = tx
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            params![normalized],
            |r| r.get(0),
        )
        .unwrap();
    let items: Vec<crate::minutes_document::GeneratedItem> =
        serde_json::from_slice(&std::fs::read(stage.join(path)).unwrap()).unwrap();
    assert_eq!(items[0].body, "点検日を確認する");
    assert_eq!(
        items[0].evidence_utterance_ids,
        vec![input.analysis.utterances[0].id.clone()]
    );
    let prompt_id = checkpoint["outputs"][0]["artifacts"][1]["id"]
        .as_str()
        .unwrap();
    let path: String = tx
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            params![prompt_id],
            |r| r.get(0),
        )
        .unwrap();
    let prompt: Value = serde_json::from_slice(&std::fs::read(stage.join(path)).unwrap()).unwrap();
    assert_eq!(
        prompt["utterances"][0]["utteranceId"],
        input.analysis.utterances[0].id
    );
    assert!(spec.parameters["generation"]["engine"]["model"]["path"]
        .as_str()
        .unwrap()
        .starts_with("mon-unavailable:"));
    tx.commit().unwrap();
    let assets: Vec<(String, i64, String)> = work
        .database
        .prepare(
            "SELECT relative_path,size_bytes,content_hash FROM file_assets WHERE state='ready'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (relative, size, hash) in assets {
        assert_eq!(
            digest_file(&stage.join(relative), &|| false).unwrap(),
            (size as u64, hash)
        );
    }
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert_eq!(calls(&f), before);
    assert_eq!(
        f.storage.load_transcript_draft(&f.meeting).unwrap(),
        old_draft
    );
    assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    drop(work);
    assert!(!stage.exists());
}

#[test]
fn generation_workspace_replays_historical_partial_merge_and_shared_retry_files() {
    let f = fixture(true, true);
    let cp = f.storage.fixed_job_input(&f.job.id).unwrap().1.unwrap();
    let parent = f
        .storage
        .finish_job(&f.job.id, f.job.completed_units, "failed", Some("fixture"))
        .unwrap();
    let child = crate::generation::RetryRequest {
        meeting_id: f.meeting.clone(),
        job_id: parent.id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_spec_hash: parent.spec_hash.clone(),
    }
    .enqueue(&f.storage)
    .unwrap();
    let before = calls(&f);
    let output = tempfile::tempdir().unwrap();
    let source = archive(&f, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut work = FixedWorkspace::prepare(&fixed, output.path(), &|| false)
        .unwrap()
        .with_generation_artifacts(&fixed, &|| false)
        .unwrap();
    let tx = work.database.transaction().unwrap();
    let new_child = ids.get("processing_jobs", &f.meeting, &child.id).unwrap();
    let (_, checkpoint) = crate::storage::jobs::read_fixed_input(&tx, &new_child).unwrap();
    let checkpoint = checkpoint.unwrap();
    assert_eq!(checkpoint["pending"]["index"], cp["pending"]["index"]);
    assert_eq!(
        checkpoint["pending"]["batches"].as_array().unwrap().len(),
        2
    );
    let new_parent = ids.get("processing_jobs", &f.meeting, &parent.id).unwrap();
    let restored = crate::storage::jobs::load_job(&tx, &new_child)
        .unwrap()
        .unwrap();
    assert_eq!(restored.retry_of_job_id, Some(new_parent));
    assert_eq!(restored.completed_units, child.completed_units);
    assert_eq!(restored.state, "queued"); // Unsealed: publication must change runnable state.
    let count: i64 = tx
        .query_row("SELECT count(*) FROM job_checkpoints", [], |r| r.get(0))
        .unwrap();
    assert!(count > 3);
    tx.commit().unwrap();
    crate::storage::backup::validate_database(&mut work.database, &|| false).unwrap();
    assert_eq!(calls(&f), before);
}

#[test]
fn generation_workspace_rejects_changed_verified_result_and_cleans_only_its_stage() {
    let f = fixture(false, false);
    let output = tempfile::tempdir().unwrap();
    let source = archive(&f, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let stage = work.stage.path().to_path_buf();
    let old_cp = f.storage.fixed_job_input(&f.job.id).unwrap().1.unwrap();
    let raw = old_cp["outputs"][0]["artifacts"][3]["id"].as_str().unwrap();
    std::fs::write(&source.assets[raw], b"{}").unwrap();
    assert!(work.with_generation_artifacts(&fixed, &|| false).is_err());
    assert!(!stage.exists());
    assert!(source.assets[raw].is_file());
    assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
}

#[test]
fn generation_workspace_cancellation_at_entry_and_after_private_write_cleans_only_owned_files() {
    let f = fixture(false, false);
    let output = tempfile::tempdir().unwrap();
    let source = archive(&f, output.path());
    let checkpoint = f.storage.fixed_job_input(&f.job.id).unwrap().1.unwrap();
    let system = checkpoint["outputs"][0]["artifacts"][0]["id"]
        .as_str()
        .unwrap();
    let marker = output.path().join("keep.txt");
    std::fs::write(&marker, b"keep").unwrap();
    for after_write in [false, true] {
        let ids = prepared(&source, output.path());
        let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
        let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
        let stage = work.stage.path().to_path_buf();
        let file = stage.join(format!(
            "meetings/{}/restored/{}",
            ids.get("meetings", "", &f.meeting).unwrap(),
            ids.get("file_assets", &f.meeting, system).unwrap()
        ));
        let observed = std::cell::Cell::new(false);
        assert!(work
            .with_generation_artifacts(&fixed, &|| {
                if file.exists() {
                    observed.set(true);
                }
                !after_write || observed.get()
            })
            .is_err());
        assert_eq!(observed.get(), after_write);
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
        assert!(source.assets.values().all(|p| p.is_file()));
    }
    assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
}

#[test]
fn generation_workspace_rejects_source_file_and_target_directory_symlinks_without_following_them() {
    for source_link in [true, false] {
        let f = fixture(false, false);
        let output = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = archive(&f, output.path());
        let ids = prepared(&source, output.path());
        let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
        let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
        let stage = work.stage.path().to_path_buf();
        let marker = outside.path().join("keep.txt");
        std::fs::write(&marker, b"keep").unwrap();
        if source_link {
            let checkpoint = f.storage.fixed_job_input(&f.job.id).unwrap().1.unwrap();
            let raw = checkpoint["outputs"][0]["artifacts"][3]["id"]
                .as_str()
                .unwrap();
            let file = &source.assets[raw];
            let target = outside.path().join("result.json");
            std::fs::copy(file, &target).unwrap();
            std::fs::remove_file(file).unwrap();
            std::os::unix::fs::symlink(&target, file).unwrap();
        } else {
            let directory = stage
                .join("meetings")
                .join(ids.get("meetings", "", &f.meeting).unwrap())
                .join("restored");
            std::fs::rename(&directory, stage.join("held-assets")).unwrap();
            std::os::unix::fs::symlink(outside.path(), &directory).unwrap();
        }
        assert!(work.with_generation_artifacts(&fixed, &|| false).is_err());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
        assert_eq!(
            std::fs::read_dir(outside.path()).unwrap().count(),
            if source_link { 2 } else { 1 }
        );
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    }
}

#[test]
fn generation_workspace_rejects_mutated_source_and_target_fixed_settings() {
    for source_settings in [true, false] {
        let f = fixture(false, false);
        let output = tempfile::tempdir().unwrap();
        let source = archive(&f, output.path());
        let ids = prepared(&source, output.path());
        let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
        let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
        let stage = work.stage.path().to_path_buf();
        if source_settings {
            // Simulate an external mutation of this test-owned validation file;
            // retain query_only on the production source connection.
            let path: String = source
                .database
                .query_row(
                    "SELECT file FROM pragma_database_list WHERE name='main'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(Path::new(&path).starts_with(output.path().canonicalize().unwrap()));
            let writer = Connection::open(path).unwrap();
            writer.execute("UPDATE processing_jobs SET parameters_json=json_set(parameters_json,'$.generation.engine.seed',99) WHERE id=?1",params![f.job.id]).unwrap();
        } else {
            let job = ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap();
            work.database.execute("UPDATE processing_jobs SET parameters_json=json_set(parameters_json,'$.generation.engine.seed',99) WHERE id=?1",params![job]).unwrap();
        }
        assert!(work.with_generation_artifacts(&fixed, &|| false).is_err());
        assert!(!stage.exists());
        assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
    }
}

#[test]
fn generation_workspace_rejects_invalid_retry_parent_state_using_runtime_lineage_policy() {
    let f = fixture(true, true);
    let output = tempfile::tempdir().unwrap();
    let parent = f
        .storage
        .finish_job(&f.job.id, f.job.completed_units, "failed", Some("fixture"))
        .unwrap();
    crate::generation::RetryRequest {
        meeting_id: f.meeting.clone(),
        job_id: parent.id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_spec_hash: parent.spec_hash.clone(),
    }
    .enqueue(&f.storage)
    .unwrap();
    let source = archive(&f, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let stage = work.stage.path().to_path_buf();
    let path: String = source
        .database
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Path::new(&path).starts_with(output.path().canonicalize().unwrap()));
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='paused' WHERE id=?1",
            params![parent.id],
        )
        .unwrap();
    assert_eq!(
        work.with_generation_artifacts(&fixed, &|| false)
            .err()
            .unwrap(),
        "再試行の固定条件または由来が一致しません"
    );
    assert!(!stage.exists());
    assert_eq!(f.storage.get_job(&parent.id).unwrap(), parent);
}

#[test]
fn generation_workspace_rejects_unknown_worker_conditions_without_publishing_partial_image() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting =
        crate::storage::backup::restore_fixed::tests::fixture(&storage, "unsupported-generation");
    let source = crate::storage::backup::restore_fixed::tests::read(&storage, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let stage = work.stage.path().to_path_buf();
    assert!(work.with_generation_artifacts(&fixed, &|| false).is_err());
    assert!(!stage.exists());
    assert!(storage.get_meeting(&meeting).unwrap().is_some());
}

#[test]
fn generation_workspace_validates_unstarted_job_without_creating_a_checkpoint_or_running_model() {
    let f = queued(false);
    let output = tempfile::tempdir().unwrap();
    let source = archive(&f, output.path());
    let ids = prepared(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = FixedWorkspace::prepare(&fixed, output.path(), &|| false)
        .unwrap()
        .with_generation_artifacts(&fixed, &|| false)
        .unwrap();
    let job = ids.get("processing_jobs", &f.meeting, &f.job.id).unwrap();
    let (spec, checkpoint) = crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
    assert!(checkpoint.is_none());
    assert_eq!(spec.total_units, Some(1));
    assert!(!f.root.path().join("engine.calls").exists());
    assert_eq!(
        work.database
            .query_row("SELECT count(*) FROM job_checkpoints", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        work.database
            .query_row("SELECT count(*) FROM file_assets", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(f.storage.get_job(&f.job.id).unwrap(), f.job);
}
