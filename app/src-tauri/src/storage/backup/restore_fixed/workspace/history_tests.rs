use super::*;
use crate::storage::analysis_flows::AnalysisFlow;

struct Fixture {
    root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    flow: AnalysisFlow,
}

#[test]
fn analysis_flow_regeneration_backup_rebuilds_replacement_ids_and_preserves_stale_human_edits() {
    for later_edit in [false, true] {
        let mut f = fixture(true);
        let base = f
            .storage
            .load_current_analysis(&f.meeting)
            .unwrap()
            .unwrap();
        let draft = f
            .storage
            .create_transcript_draft(&f.meeting, &base.id)
            .unwrap();
        let cli = f.root.path().join("sherpa-cli");
        let script = std::fs::read_to_string(&cli)
            .unwrap()
            .replace("num_clusters=-1", "num_clusters=1");
        std::fs::write(&cli, script).unwrap();
        let audio = f.storage.recorded_analysis_input(&f.meeting).unwrap().0;
        let request = crate::storage::analysis_flows::regeneration::Request {
            meeting_id: f.meeting.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: f.storage.regeneration_admission(&f.meeting).unwrap(),
            speaker_counts: vec![crate::diarization::TrackSettings {
                track_id: audio.tracks[0].id.clone(),
                num_speakers: Some(1),
            }],
        };
        f.storage
            .regenerate_analysis(&request, &crate::test_support::never_cancelled)
            .unwrap();
        crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
        let trans = f.storage.acquire_local_inference_job().unwrap().unwrap();
        crate::transcription::process(
            &f.storage,
            &trans.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
        let diar = f.storage.acquire_local_inference_job().unwrap().unwrap();
        crate::diarization::process(
            &f.storage,
            &diar.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        crate::analysis_flow::advance(&f.storage, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
        if later_edit {
            let mut payload = draft.payload.clone();
            payload
                .utterances
                .push(crate::storage::drafts::UtteranceEdit {
                    utterance_id: base.utterances[0].id.clone(),
                    effective_text: "後続の手動修正".into(),
                    speaker_ids: vec![],
                });
            f.storage
                .save_transcript_draft(&draft.id, draft.lock_version, payload)
                .unwrap();
        }
        f.flow = f.storage.analysis_flow(&f.meeting).unwrap().unwrap();
        assert!(f.flow.ready_to_publish);
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let mut work = prepared(&fixed, output.path())
            .with_processing_history(&fixed, &crate::test_support::never_cancelled)
            .unwrap();
        let meeting = ids.get("meetings", "", &f.meeting).unwrap();
        let flow = crate::storage::analysis_flows::read_flow(&work.database, &meeting)
            .unwrap()
            .unwrap();
        assert_eq!(flow.stage, "attention");
        assert!(!flow.ready_to_publish);
        let plan = crate::analysis_flow::saved_plan(flow.plan.unwrap(), "standard").unwrap();
        assert_eq!(plan.speaker_counts[0].num_speakers, Some(1));
        assert_eq!(
            plan.speaker_counts[0].track_id,
            ids.get("audio_tracks", &f.meeting, &audio.tracks[0].id)
                .unwrap()
        );
        assert_eq!(
            plan.speaker_counts_manifest.as_deref(),
            Some(
                fixed
                    .audio_manifest_hash(&f.meeting, &audio.manifest_hash)
                    .unwrap()
                    .as_str()
            )
        );
        let target = plan.replacement.unwrap();
        assert_eq!(
            target.analysis_id,
            ids.get("analysis_revisions", &f.meeting, &base.id).unwrap()
        );
        let stamp = target.draft.unwrap();
        assert_eq!(
            stamp.id,
            ids.get("working_drafts", &f.meeting, &draft.id).unwrap()
        );
        assert_eq!(stamp.lock_version, draft.lock_version);
        let saved_version: i64 = work
            .database
            .query_row(
                "SELECT lock_version FROM working_drafts WHERE id=?1",
                params![stamp.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(saved_version, draft.lock_version + i64::from(later_edit));
        let head: String = work
            .database
            .query_row(
                "SELECT current_analysis_id FROM meetings WHERE id=?1",
                params![meeting],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(head, target.analysis_id);
        crate::storage::backup::validate_database(
            &mut work.database,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        drop(work);
        crate::storage::backup::restore_fixed::workspace::seal::SealedWorkspace::prepare(
            &fixed,
            output.path(),
            &crate::test_support::never_cancelled,
        )
        .unwrap()
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
        assert_eq!(
            f.storage
                .load_current_analysis(&f.meeting)
                .unwrap()
                .unwrap(),
            base
        );
    }
}
fn fixture(complete: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting) = crate::storage::analysis_flows::tests::fixture(root.path());
    storage.ensure_analysis_flow(&meeting).unwrap();
    crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    if complete {
        let trans = storage.acquire_local_inference_job().unwrap().unwrap();
        crate::transcription::process(
            &storage,
            &trans.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
        let diar = storage.acquire_local_inference_job().unwrap().unwrap();
        crate::diarization::process(
            &storage,
            &diar.id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .unwrap();
        crate::analysis_flow::advance(&storage, &crate::test_support::never_cancelled, &|_| {})
            .unwrap();
    }
    let flow = storage.analysis_flow(&meeting).unwrap().unwrap();
    Fixture {
        root,
        storage,
        meeting,
        flow,
    }
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
fn prepared(fixed: &FixedData<'_, '_>, output: &Path) -> FixedWorkspace {
    FixedWorkspace::prepare(fixed, output, &crate::test_support::never_cancelled)
        .unwrap()
        .with_speech_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_generation_artifacts(fixed, &crate::test_support::never_cancelled)
        .unwrap()
}
fn root_operations(db: &Connection, flow: &AnalysisFlow) {
    for (kind, job) in [
        ("transcription", &flow.transcription_job),
        ("diarization", &flow.diarization_job),
    ] {
        if let Some(job) = job {
            let spec = crate::storage::jobs::read_fixed_input(db, &job.id)
                .unwrap()
                .0;
            let operation = crate::storage::jobs::initial_operation(job.clone(), &spec, |parent| {
                Ok((
                    crate::storage::jobs::load_job(db, parent)?.unwrap(),
                    crate::storage::jobs::read_fixed_input(db, parent)?.0,
                ))
            })
            .unwrap();
            assert_eq!(operation, format!("flow:{}:{kind}", flow.id));
        }
    }
}

#[test]
fn processing_history_restores_completed_flow_operation_plan_hash_and_published_utterances() {
    let f = fixture(true);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    for name in [
        "fake-cli",
        "model.bin",
        "sherpa-cli",
        "seg.onnx",
        "emb.onnx",
    ] {
        std::fs::remove_file(f.root.path().join(name)).unwrap();
    }
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let stage = work.stage.path().to_path_buf();
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(&work.database, &meeting)
        .unwrap()
        .unwrap();
    assert_eq!(flow.stage, "review");
    assert_ne!(flow.id, f.flow.id);
    assert_eq!(flow.lock_version, f.flow.lock_version);
    assert_eq!(
        flow.plan,
        serde_json::from_str::<Value>(
            &source
                .database
                .query_row(
                    "SELECT plan_json FROM analysis_flows WHERE id=?1",
                    params![f.flow.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap()
        )
        .ok()
    );
    root_operations(&work.database, &flow);
    let candidate = flow
        .diarization_job
        .as_ref()
        .unwrap()
        .output_analysis_id
        .as_ref()
        .unwrap();
    let tx = work.database.transaction().unwrap();
    assert!(crate::storage::analysis_flows::adoption_parent_matches(
        &tx, &meeting, candidate, None
    )
    .unwrap());
    tx.commit().unwrap();
    crate::storage::backup::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        f.storage
            .load_current_analysis(&f.meeting)
            .unwrap()
            .unwrap()
            .id,
        f.flow
            .diarization_job
            .as_ref()
            .unwrap()
            .output_analysis_id
            .as_deref()
            .unwrap()
    );
    assert_eq!(
        work.database
            .query_row(
                "SELECT current_analysis_id FROM meetings WHERE id=?1",
                params![meeting],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        *candidate
    );
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().plan,
        f.flow.plan
    );
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "review"
    );
    drop(work);
    assert!(!stage.exists());
}

#[test]
fn persisted_ids_replay_native_speech_generation_history_and_files_after_restart() {
    use crate::storage::backup::ledger::BatchPlan;
    use crate::storage::backup::restore_ids::BatchPreparation;
    fn pending<'a>(result: Result<BatchPreparation<'a>, String>) -> RestoreIds<'a> {
        match result.unwrap() {
            BatchPreparation::Pending(ids) => ids,
            BatchPreparation::AlreadyPublished { .. } => panic!("unpublished batch expected"),
        }
    }
    fn rows(db: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
        let mut statement = db
            .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    fn build(ids: &RestoreIds<'_>, output: &Path) -> FixedWorkspace {
        let fixed = FixedData::new(ids, output, &crate::test_support::never_cancelled).unwrap();
        let mut work = prepared(&fixed, output)
            .with_processing_history(&fixed, &crate::test_support::never_cancelled)
            .unwrap()
            .with_provenance(&fixed, &crate::test_support::never_cancelled)
            .unwrap();
        crate::storage::backup::validate_database(
            &mut work.database,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        work
    }
    let (f, _outside, external) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let existing = target.create_meeting("既存会議").unwrap();
    let batch = Uuid::new_v4().to_string();
    let ids = pending(RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &target.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    ));
    {
        let mut db = target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        BatchPlan::new(&ids, &crate::test_support::never_cancelled)
            .unwrap()
            .reserve(&mut tx, &crate::test_support::never_cancelled)
            .unwrap();
        tx.commit().unwrap();
    }
    // All restore passes must work without invoking the original CLI or model.
    for name in [
        "fake-cli",
        "model.bin",
        "sherpa-cli",
        "seg.onnx",
        "emb.onnx",
    ] {
        std::fs::remove_file(f.root.path().join(name)).unwrap();
    }
    for name in ["llama-completion", "model.gguf"] {
        std::fs::remove_file(f.root.path().join("markdown-worker").join(name)).unwrap();
    }
    let first = build(&ids, output.path());
    let expected: Vec<_> = portable::tables(&first.database)
        .unwrap()
        .into_iter()
        .filter(|table| table.name != "restore_provenance")
        .map(|table| (table.name.clone(), rows(&first.database, &table.name)))
        .collect();
    let mut statement = first
        .database
        .prepare("SELECT relative_path FROM file_assets WHERE state='ready' ORDER BY relative_path")
        .unwrap();
    let files: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let bytes: Vec<_> = files
        .iter()
        .map(|file| std::fs::read(first.stage.path().join(file)).unwrap())
        .collect();
    let missing = first.missing_assets.clone();
    drop(statement);
    drop(first);
    drop(ids);
    drop(source);
    drop(target);
    let source = archive::read(
        &mut File::open(output.path().join("fixed.monbackup")).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let ids = pending(RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &target.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    ));
    let second = build(&ids, output.path());
    for (table, original) in expected {
        assert_eq!(rows(&second.database, &table), original, "{table}");
    }
    for (file, original) in files.iter().zip(bytes) {
        assert_eq!(
            std::fs::read(second.stage.path().join(file)).unwrap(),
            original,
            "{file}"
        );
    }
    assert_eq!(second.missing_assets, missing);
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(&second.database, &meeting)
        .unwrap()
        .unwrap();
    root_operations(&second.database, &flow);
    assert_eq!(target.list_meetings().unwrap(), vec![existing]);
    assert_eq!(std::fs::read(&external).unwrap(), b"external edit");
}

#[test]
fn persisted_ids_reject_wrong_derived_flow_operation_even_with_rehashed_ledger() {
    use crate::storage::backup::ledger::{BatchPlan, IdentityMap};
    use crate::storage::backup::restore_ids::BatchPreparation;
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let batch = Uuid::new_v4().to_string();
    let mut db = portable::schema(MIGRATIONS.len()).unwrap();
    let ids = match RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &db,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    {
        BatchPreparation::Pending(ids) => ids,
        _ => panic!("new batch expected"),
    };
    let mut tx = db.transaction().unwrap();
    BatchPlan::new(&ids, &crate::test_support::never_cancelled)
        .unwrap()
        .reserve(&mut tx, &crate::test_support::never_cancelled)
        .unwrap();
    tx.commit().unwrap();
    drop(ids);
    // Only this destination fixture has its immutable triggers removed.
    db.execute_batch("DROP TRIGGER restore_map_fixed; DROP TRIGGER restore_batch_fixed;")
        .unwrap();
    db.execute("UPDATE restore_identity_maps SET target_key=?1 WHERE domain='operation' AND source_key LIKE 'flow:%:transcription'", params![format!("flow:{}:diarization", Uuid::new_v4())]).unwrap();
    let mut hash = Sha256::new();
    let mut statement = db.prepare("SELECT domain,source_scope,source_key,target_key FROM restore_identity_maps WHERE batch_id=?1 ORDER BY domain,source_scope,source_key").unwrap();
    let maps = statement
        .query_map(params![batch], |r| {
            Ok(IdentityMap {
                domain: r.get(0)?,
                scope: r.get(1)?,
                source: r.get(2)?,
                target: r.get(3)?,
            })
        })
        .unwrap();
    for map in maps {
        hash.update(serde_json::to_vec(&map.unwrap()).unwrap());
        hash.update(b"\n");
    }
    db.execute(
        "UPDATE restore_batches SET mapping_hash=?1",
        params![format!("{:x}", hash.finalize())],
    )
    .unwrap();
    let error = RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &db,
        &crate::test_support::never_cancelled,
    )
    .err()
    .unwrap();
    assert!(error.contains("派生操作ID"), "{error}");
}

#[test]
fn restore_seal_native_speech_generation_history_and_missing_assets_without_models_or_auto_start() {
    use super::super::seal::SealedWorkspace;
    let (f, _outside, external) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    for name in [
        "fake-cli",
        "model.bin",
        "sherpa-cli",
        "seg.onnx",
        "emb.onnx",
    ] {
        std::fs::remove_file(f.root.path().join(name)).unwrap();
    }
    for name in ["llama-completion", "model.gguf"] {
        std::fs::remove_file(f.root.path().join("markdown-worker").join(name)).unwrap();
    }
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let db = sealed.connection();
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(db, &meeting)
        .unwrap()
        .unwrap();
    root_operations(db, &flow);
    assert_eq!(flow.stage, "review");
    assert!(
        db.query_row(
            "SELECT count(*) FROM file_assets WHERE state='missing'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > 0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM processing_jobs WHERE state IN ('queued','running','pausing','paused')", [], |r|r.get::<_,i64>(0)).unwrap(),0);
    let tx = db.unchecked_transaction().unwrap();
    assert!(crate::storage::analysis_flows::adoption_parent_matches(
        &tx,
        &meeting,
        flow.diarization_job
            .as_ref()
            .unwrap()
            .output_analysis_id
            .as_ref()
            .unwrap(),
        None
    )
    .unwrap());
    tx.commit().unwrap();
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(std::fs::read(&external).unwrap(), b"external edit");
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "review"
    );
    let omitted: String = db
        .query_row(
            "SELECT relative_path FROM file_assets WHERE state='missing' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let path = sealed.root_path().unwrap().join(omitted);
    assert!(!path.exists());
    std::fs::write(&path, b"must remain omitted").unwrap();
    assert!(sealed
        .verify(&crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("除外"));
}

#[test]
fn restore_publication_native_history_preserves_existing_edits_files_and_reopens_without_models() {
    use super::super::seal::SealedWorkspace;
    use crate::storage::backup::publication::{tests::all_rows, Publication};
    let (f, _outside, external) = exported_fixture();
    let (existing, _existing_outside, existing_external) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let batch = Uuid::new_v4().to_string();
    let ids = match RestoreIds::prepare_for_batch(
        &source,
        output.path(),
        &batch,
        &existing.storage.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    {
        crate::storage::backup::restore_ids::BatchPreparation::Pending(ids) => ids,
        _ => panic!(),
    };
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    for name in [
        "fake-cli",
        "model.bin",
        "sherpa-cli",
        "seg.onnx",
        "emb.onnx",
    ] {
        std::fs::remove_file(f.root.path().join(name)).unwrap();
    }
    for name in ["llama-completion", "model.gguf"] {
        std::fs::remove_file(f.root.path().join("markdown-worker").join(name)).unwrap();
    }
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let before = all_rows(&existing.storage.connection.lock().unwrap());
    assert!(matches!(
        existing
            .storage
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        Publication::Published { meetings: 1, .. }
    ));
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    {
        let mut db = existing.storage.connection.lock().unwrap();
        crate::storage::backup::validate_database(&mut db, &crate::test_support::never_cancelled)
            .unwrap();
        let after = all_rows(&db);
        for (index, table) in crate::storage::backup::portable::TABLES
            .iter()
            .chain(["app_settings", "restore_batches", "restore_identity_maps"].iter())
            .enumerate()
        {
            assert!(
                after[index].starts_with(&before[index]),
                "existing {table} changed"
            );
        }
        let flow = crate::storage::analysis_flows::read_flow(&db, &meeting)
            .unwrap()
            .unwrap();
        root_operations(&db, &flow);
        assert_eq!(flow.stage, "review");
        assert_eq!(db.query_row("SELECT count(*) FROM processing_jobs WHERE state IN ('queued','running','pausing','paused')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(
            db.query_row(
                "SELECT count(*) FROM file_assets WHERE meeting_id=?1 AND state='missing'",
                params![meeting],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
        let mut q=db.prepare("SELECT relative_path,size_bytes,content_hash,state FROM file_assets WHERE meeting_id=?1").unwrap();
        let mut rows = q.query(params![meeting]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let relative: String = row.get(0).unwrap();
            let path = existing.storage.root.join(relative);
            let state: String = row.get(3).unwrap();
            if state == "missing" {
                assert!(!path.exists());
            } else {
                assert_eq!(
                    crate::storage::backup::digest_file(
                        &path,
                        &crate::test_support::never_cancelled
                    )
                    .unwrap(),
                    (
                        row.get::<_, i64>(1).unwrap() as u64,
                        row.get::<_, String>(2).unwrap()
                    )
                );
            }
        }
    }
    assert!(existing
        .storage
        .load_current_analysis(&meeting)
        .unwrap()
        .is_some());
    assert!(existing
        .storage
        .get_minutes_markdown(&meeting)
        .unwrap()
        .is_some());
    assert!(existing
        .storage
        .acquire_local_inference_job()
        .unwrap()
        .is_none());
    assert_eq!(std::fs::read(&external).unwrap(), b"external edit");
    assert_eq!(std::fs::read(&existing_external).unwrap(), b"external edit");
    let root = existing.root.path().to_path_buf();
    drop(existing.storage);
    let reopened = Storage::open(root).unwrap();
    assert_eq!(reopened.list_meetings().unwrap().len(), 2);
    assert!(reopened.load_current_analysis(&meeting).unwrap().is_some());
    assert!(reopened.get_minutes_markdown(&meeting).unwrap().is_some());
    assert!(reopened.acquire_local_inference_job().unwrap().is_none());
    assert!(matches!(
        RestoreIds::prepare_for_batch(
            &source,
            output.path(),
            &batch,
            &reopened.connection.lock().unwrap(),
            &crate::test_support::never_cancelled
        )
        .unwrap(),
        crate::storage::backup::restore_ids::BatchPreparation::AlreadyPublished { .. }
    ));
}

#[test]
fn processing_history_interrupts_queued_retry_and_prepared_flow_without_losing_conditions() {
    let mut f = fixture(false);
    let parent = f.flow.transcription_job.as_ref().unwrap();
    assert_eq!(
        f.storage.acquire_local_inference_job().unwrap().unwrap().id,
        parent.id
    );
    let parent = f
        .storage
        .finish_job(&parent.id, 0, "failed", Some("fixture"))
        .unwrap();
    let child = crate::transcription::RetryRequest {
        meeting_id: f.meeting.clone(),
        job_id: parent.id.clone(),
        operation_id: "flow-retry".into(),
        expected_spec_hash: parent.spec_hash.clone(),
    }
    .enqueue(&f.storage)
    .unwrap();
    f.flow = f
        .storage
        .update_analysis_flow(
            &f.flow,
            "transcription",
            f.flow.plan.clone(),
            Some(&child),
            None,
            None,
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let meeting = ids.get("meetings", "", &f.meeting).unwrap();
    let flow = crate::storage::analysis_flows::read_flow(&work.database, &meeting)
        .unwrap()
        .unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.error_message.as_ref().unwrap().contains("明示的"));
    let rebuilt = flow.transcription_job.as_ref().unwrap();
    assert_eq!(rebuilt.state, "interrupted");
    assert_eq!(
        rebuilt.error_code.as_deref(),
        Some("restored_preparation_required")
    );
    assert_eq!(rebuilt.attempt, child.attempt);
    assert_eq!(
        rebuilt.retry_of_job_id,
        Some(ids.get("processing_jobs", &f.meeting, &parent.id).unwrap())
    );
    root_operations(&work.database, &flow);
    crate::storage::backup::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(f.storage.get_job(&child.id).unwrap(), child);
    assert_eq!(f.storage.get_job(&parent.id).unwrap(), parent);
}

#[test]
fn processing_history_blocks_pending_flow_with_no_models_or_jobs() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let (storage, meeting, _) = crate::storage::transcription::tests::fixture(root.path(), 4);
    let old = storage.ensure_analysis_flow(&meeting).unwrap();
    assert_eq!(old.stage, "pending");
    let f = Fixture {
        root,
        storage,
        meeting,
        flow: old,
    };
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let flow = crate::storage::analysis_flows::read_flow(
        &work.database,
        &ids.get("meetings", "", &f.meeting).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(flow.stage, "configuration");
    assert!(flow.plan.is_none());
    assert!(flow.transcription_job.is_none());
    assert_eq!(
        work.database
            .query_row("SELECT count(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "pending"
    );
}

fn exported_fixture() -> (Fixture, tempfile::TempDir, PathBuf) {
    let f = fixture(true);
    let candidate = f
        .flow
        .diarization_job
        .as_ref()
        .unwrap()
        .output_analysis_id
        .as_ref()
        .unwrap();
    assert_eq!(
        f.storage
            .load_current_analysis(&f.meeting)
            .unwrap()
            .unwrap()
            .id,
        *candidate
    );
    let draft = f
        .storage
        .create_transcript_draft(&f.meeting, candidate)
        .unwrap();
    let input = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    let request = crate::storage::MinutesJobRequest {
        meeting_id: f.meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: input.id,
        expected_content_hash: input.content_hash,
        expected_meeting_lock_version: f
            .storage
            .get_meeting(&f.meeting)
            .unwrap()
            .unwrap()
            .lock_version,
        expected_draft: Some(crate::storage::DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let model_root = f.root.path().join("markdown-worker");
    std::fs::create_dir(&model_root).unwrap();
    let job = crate::generation::markdown_job::enqueue_synthetic(
        &f.storage,
        request,
        crate::storage::markdown_generation_tests::engine(&model_root, "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    f.storage.acquire_local_inference_job().unwrap().unwrap();
    crate::storage::markdown_generation_tests::process(
        &f.storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let body = f.storage.get_minutes_markdown(&f.meeting).unwrap().unwrap();
    let saved = f
        .storage
        .save_minutes_markdown(crate::storage::minutes_markdown::SaveMarkdownRequest {
            meeting_id: f.meeting.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: Some(body.stamp()),
            markdown: format!("{}\n出力時だけの手動追記", body.markdown),
        })
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("保存済み議事録.md");
    let prepare = || {
        f.storage
            .prepare_markdown_export(crate::storage::markdown_exports::MarkdownExportRequest {
                meeting_id: f.meeting.clone(),
                operation_id: Uuid::new_v4().to_string(),
                expected: saved.stamp(),
            })
            .unwrap()
    };
    f.storage.write_markdown_export(prepare(), &path).unwrap();
    assert_eq!(
        f.storage
            .write_markdown_export(prepare(), &outside.path().join("missing/blocked.md"))
            .unwrap()
            .state,
        "failed"
    );
    assert_eq!(f.storage.exports(&f.meeting).unwrap().len(), 2);
    std::fs::write(&path, b"external edit").unwrap();
    f.storage
        .save_minutes_markdown(crate::storage::minutes_markdown::SaveMarkdownRequest {
            meeting_id: f.meeting.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: Some(saved.stamp()),
            markdown: format!("{}\n出力後の追加編集", saved.markdown),
        })
        .unwrap();
    (f, outside, path)
}

#[test]
fn processing_history_keeps_external_export_evidence_and_new_references_without_touching_external_files(
) {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let original = f.storage.exports(&f.meeting).unwrap();
    let draft = f.storage.get_minutes_markdown(&f.meeting).unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    for old in &original {
        let new = ids.get("export_records", &f.meeting, &old.id).unwrap();
        let saved: (String,String,String,i64,String,String,Option<i64>) = work.database.query_row("SELECT meeting_id,exported_content_hash,output_content_hash,output_size_bytes,destination_path,state,exported_at FROM export_records WHERE id=?1",params![new],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
        assert_eq!(
            saved,
            (
                ids.get("meetings", "", &f.meeting).unwrap(),
                old.exported_content_hash.clone(),
                old.output_content_hash.clone(),
                old.output_size_bytes,
                portable::basename(&old.destination_path).unwrap(),
                old.state.clone(),
                old.exported_at
            )
        );
        let minutes: Option<String> = work
            .database
            .query_row(
                "SELECT minutes_revision_id FROM export_records WHERE id=?1",
                params![new],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            minutes,
            old.minutes_revision_id
                .as_deref()
                .map(|id| ids.get("minutes_revisions", &f.meeting, id).unwrap())
        );
        assert_ne!(old.id, new);
    }
    crate::storage::backup::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert_eq!(f.storage.get_minutes_markdown(&f.meeting).unwrap(), draft);
    assert_eq!(f.storage.exports(&f.meeting).unwrap().len(), 2);
}

fn source_writer(source: &archive::VerifiedArchive, output: &Path) -> Connection {
    let path: String = source
        .database
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(Path::new(&path).starts_with(output.canonicalize().unwrap()));
    Connection::open(path).unwrap()
}

#[test]
fn restore_provenance_native_completed_speech_generation_and_export_keep_normal_readers_valid() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap()
        .with_provenance(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    crate::storage::backup::validate_database(
        &mut work.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    for domain in [
        "processing_jobs",
        "job_checkpoints",
        "analysis_revisions",
        "working_drafts",
        "export_records",
        "operation",
    ] {
        assert!(
            work.database
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM restore_provenance WHERE domain=?1)",
                    params![domain],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap(),
            "{domain}"
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "review"
    );
}

#[test]
fn processing_history_rejects_altered_target_plan_job_state_and_export_evidence() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    for change in ["plan", "state", "export"] {
        let ids = ids(&source, output.path());
        let fixed =
            FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
        let work = prepared(&fixed, output.path());
        let stage = work.stage.path().to_path_buf();
        match change {
            "plan" => {
                work.database.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.threads',2)",[]).unwrap();
            }
            "state" => {
                work.database.execute("UPDATE processing_jobs SET state='interrupted' WHERE kind='minutes_generation'",[]).unwrap();
            }
            "export" => {
                work.database
                    .execute(
                        "UPDATE export_records SET output_content_hash=?1",
                        params!["f".repeat(64)],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(work
            .with_processing_history(&fixed, &crate::test_support::never_cancelled)
            .is_err());
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
    }
}

#[test]
fn processing_history_rejects_unsupported_or_inconsistent_source_plan_and_root_operations() {
    for change in ["schema", "engine", "model", "root-kind"] {
        let f = fixture(false);
        let output = tempfile::tempdir().unwrap();
        let source = read(&f, output.path());
        let writer = source_writer(&source, output.path());
        match change {
            "schema" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.schemaVersion',2)",[]).unwrap();
            }
            "engine" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.threads',2)",[]).unwrap();
            }
            "model" => {
                writer.execute("UPDATE analysis_flows SET plan_json=json_set(plan_json,'$.whisper.model.size',99)",[]).unwrap();
            }
            "root-kind" => {
                writer
                    .execute("UPDATE processing_jobs SET kind='diarization'", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        if change == "root-kind" {
            assert!(RestoreIds::prepare(
                &source,
                output.path(),
                &Uuid::new_v4().to_string(),
                &crate::test_support::never_cancelled
            )
            .is_err());
        } else {
            let ids = ids(&source, output.path());
            let fixed =
                FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
            let work = prepared(&fixed, output.path());
            let stage = work.stage.path().to_path_buf();
            assert!(work
                .with_processing_history(&fixed, &crate::test_support::never_cancelled)
                .is_err());
            assert!(!stage.exists());
        }
        assert_eq!(
            source
                .database
                .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.storage.analysis_flow(&f.meeting).unwrap().unwrap().plan,
            f.flow.plan
        );
    }
}

#[test]
fn processing_history_cancellation_after_flow_update_cleans_only_the_owned_stage() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    let marker = output.path().join("keep.txt");
    std::fs::write(&marker, b"keep").unwrap();
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepared(&fixed, output.path());
    let stage = work.stage.path().to_path_buf();
    let image = stage.join("fixed.sqlite3");
    let inspect = Connection::open(&image).unwrap();
    let cancelled = std::cell::Cell::new(false);
    let result = work.with_processing_history(&fixed, &|| {
        let state: String = inspect
            .query_row("SELECT stage FROM analysis_flows", [], |r| r.get(0))
            .unwrap();
        if state == "attention" {
            cancelled.set(true);
        }
        cancelled.get()
    });
    drop(inspect);
    assert!(result.is_err());
    assert!(cancelled.get());
    assert!(!stage.exists());
    assert_eq!(std::fs::read(&marker).unwrap(), b"keep");
    assert_eq!(
        f.storage.analysis_flow(&f.meeting).unwrap().unwrap().stage,
        "transcription"
    );
}

#[test]
fn processing_history_preserves_unlinked_root_job_after_a_lost_handoff_without_auto_start() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    let source = read(&f, output.path());
    source_writer(&source, output.path())
        .execute("UPDATE analysis_flows SET transcription_job_id=NULL", [])
        .unwrap();
    let ids = ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let work = prepared(&fixed, output.path())
        .with_processing_history(&fixed, &crate::test_support::never_cancelled)
        .unwrap();
    let flow = crate::storage::analysis_flows::read_flow(
        &work.database,
        &ids.get("meetings", "", &f.meeting).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.transcription_job.is_none());
    let job = ids
        .get(
            "processing_jobs",
            &f.meeting,
            &f.flow.transcription_job.as_ref().unwrap().id,
        )
        .unwrap();
    let job = crate::storage::jobs::load_job(&work.database, &job)
        .unwrap()
        .unwrap();
    assert_eq!(job.state, "interrupted");
    assert_eq!(job.operation_id, format!("flow:{}:transcription", flow.id));
}

#[test]
fn processing_history_rejects_bad_export_hash_path_draft_kind_or_unsettled_state() {
    let (f, _outside, path) = exported_fixture();
    let output = tempfile::tempdir().unwrap();
    let mut counter = 0;
    for change in ["hash", "path", "draft", "writing"] {
        // Each archive owns its source DB, keeping production query_only intact.
        let dir = output.path().join(format!("case-{counter}"));
        counter += 1;
        std::fs::create_dir(&dir).unwrap();
        let source = read(&f, &dir);
        let writer = source_writer(&source, &dir);
        match change {
            "hash" => {
                writer
                    .execute(
                        "UPDATE export_records SET request_hash=?1",
                        params!["z".repeat(64)],
                    )
                    .unwrap();
            }
            "path" => {
                writer
                    .execute(
                        "UPDATE export_records SET destination_path='/Users/source/other.md'",
                        [],
                    )
                    .unwrap();
            }
            "draft" => {
                // One successful and one failed export are both retained.
                assert_eq!(writer.execute("UPDATE export_records SET source_draft_id=(SELECT id FROM working_drafts WHERE target_kind='transcript' LIMIT 1),source_draft_lock_version=(SELECT lock_version FROM working_drafts WHERE target_kind='transcript' LIMIT 1)",[]).unwrap(),2);
            }
            "writing" => {
                writer
                    .execute(
                        "UPDATE export_records SET state='writing',exported_at=NULL",
                        [],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let ids = ids(&source, &dir);
        let fixed = FixedData::new(&ids, &dir, &crate::test_support::never_cancelled).unwrap();
        let work = prepared(&fixed, &dir);
        let stage = work.stage.path().to_path_buf();
        let error = work
            .with_processing_history(&fixed, &crate::test_support::never_cancelled)
            .err()
            .unwrap();
        assert!(error.contains("書き出し履歴"), "{change}: {error}");
        assert!(!stage.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
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
fn processing_history_interrupts_each_runnable_or_interrupted_state_without_changing_fixed_input() {
    let f = fixture(false);
    let output = tempfile::tempdir().unwrap();
    for state in ["queued", "running", "pausing", "paused", "interrupted"] {
        let dir = output.path().join(state);
        std::fs::create_dir(&dir).unwrap();
        let source = read(&f, &dir);
        source_writer(&source, &dir)
            .execute(
                "UPDATE processing_jobs SET state=?1,pause_reason='user'",
                params![state],
            )
            .unwrap();
        let ids = ids(&source, &dir);
        let fixed = FixedData::new(&ids, &dir, &crate::test_support::never_cancelled).unwrap();
        let work = prepared(&fixed, &dir);
        let job = ids
            .get(
                "processing_jobs",
                &f.meeting,
                &f.flow.transcription_job.as_ref().unwrap().id,
            )
            .unwrap();
        let before = crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
        let work = work
            .with_processing_history(&fixed, &crate::test_support::never_cancelled)
            .unwrap();
        let after = crate::storage::jobs::read_fixed_input(&work.database, &job).unwrap();
        assert_eq!(before, after);
        let restored = crate::storage::jobs::load_job(&work.database, &job)
            .unwrap()
            .unwrap();
        assert_eq!(restored.state, "interrupted");
        assert!(restored.pause_reason.is_none());
        assert_eq!(restored.completed_units, 0);
        assert_eq!(
            f.storage
                .get_job(&f.flow.transcription_job.as_ref().unwrap().id)
                .unwrap()
                .state,
            "queued"
        );
    }
}
