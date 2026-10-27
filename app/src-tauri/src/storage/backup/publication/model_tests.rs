use super::*;
use crate::storage::model_metadata::{retained_files, ModelFileMetadata};

#[test]
fn exact_minutes_markdown_survives_archive_additive_restore_reopen_and_rebackup() {
    use crate::storage::minutes_markdown::SaveMarkdownRequest;
    // Reuse one callback type through the generic reconstruction stack.
    // Separate closures at each call needlessly duplicate its code generation.
    let check: fn() -> bool = || false;
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let source_storage = Storage::open(root.path().into()).unwrap();
    let meeting = source_storage.create_meeting("Markdown backup").unwrap().id;
    let text="\r\n# 人間が編集した議事録\r\n\t**そのまま**  \n[区間](audio:00:00:01.000001-00:00:02.999999)\n<script>表示だけ</script>\n ";
    let original = source_storage
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: meeting.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown: text.into(),
        })
        .unwrap();
    let path = output.path().join("markdown.monbackup");
    source_storage.create_backup_archive(&path, &check).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &check).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let untouched = target.create_meeting("Existing human text").unwrap().id;
    let kept = target
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: untouched.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown: "この本文は変えない".into(),
        })
        .unwrap();
    let ids =
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &check).unwrap();
    let fixed = restore_fixed::FixedData::new(&ids, output.path(), &check).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &check).unwrap();
    target.publish_restore(&sealed, &ids, &check).unwrap();
    let restored_id = ids.get("meetings", "", &meeting).unwrap();
    let restored = target.get_minutes_markdown(&restored_id).unwrap().unwrap();
    assert_eq!(restored.markdown, text);
    assert_eq!(restored.content_hash, original.content_hash);
    assert_eq!(restored.lock_version, original.lock_version);
    assert_ne!(restored.operation_id, original.operation_id);
    assert_eq!(
        target.get_minutes_markdown(&untouched).unwrap(),
        Some(kept.clone())
    );
    assert_eq!(
        source_storage.get_minutes_markdown(&meeting).unwrap(),
        Some(original)
    );
    drop(target);
    let target = Storage::open(target_root.path().into()).unwrap();
    assert_eq!(
        target.get_minutes_markdown(&restored_id).unwrap(),
        Some(restored)
    );
    assert_eq!(target.get_minutes_markdown(&untouched).unwrap(), Some(kept));
    let next = output.path().join("markdown-rebackup.monbackup");
    target.create_backup_archive(&next, &check).unwrap();
    let next = archive::read(&mut File::open(next).unwrap(), output.path(), &check).unwrap();
    let next_ids =
        RestoreIds::prepare(&next, output.path(), &Uuid::new_v4().to_string(), &check).unwrap();
    let next_fixed = restore_fixed::FixedData::new(&next_ids, output.path(), &check).unwrap();
    let next_sealed = SealedWorkspace::prepare(&next_fixed, output.path(), &check).unwrap();
    assert_eq!(
        next_sealed
            .connection()
            .query_row::<String, _, _>(
                "SELECT markdown FROM minutes_markdown WHERE markdown=?1",
                [text],
                |r| r.get(0)
            )
            .unwrap(),
        text
    );
    let db = target.connection.lock().unwrap();
    for table in [
        "minutes_revisions",
        "minutes_items",
        "evidence_links",
        "working_drafts",
        "edit_operations",
    ] {
        assert_eq!(
            db.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
}

#[test]
fn archive_rejects_tampered_minutes_markdown_without_repairing_source() {
    use crate::storage::minutes_markdown::SaveMarkdownRequest;
    let check: fn() -> bool = || false;
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("Markdown corruption").unwrap().id;
    storage
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: meeting.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown: "保存済み".into(),
        })
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE minutes_markdown SET markdown='改変' WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    assert!(storage
        .create_backup_archive(&output.path().join("invalid.monbackup"), &check)
        .is_err());
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row::<String, _, _>(
                "SELECT markdown FROM minutes_markdown WHERE meeting_id=?1",
                [meeting],
                |r| r.get(0)
            )
            .unwrap(),
        "改変"
    );
}

#[test]
fn restore_managed_model_jobs_and_flow_preserve_all_engine_proofs_without_installing_models_and_rebackup_again(
) {
    let (_root, app_data, storage, meeting, jobs) =
        crate::storage::model_profiles::tests::publication_fixture();
    let snapshots = jobs
        .iter()
        .map(|id| storage.fixed_job_input(id).unwrap().0.profile_snapshot)
        .collect::<Vec<_>>();
    let plan = storage
        .analysis_flow(&meeting)
        .unwrap()
        .unwrap()
        .plan
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("managed.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    for library in [
        "whisper-runtime.dylib",
        "llama-runtime.dylib",
        "runtime.dylib",
    ] {
        assert!(source
            .manifest
            .file_references
            .iter()
            .any(|f| f.name == library && f.role == "artifact"));
    }
    // Source package roots can disappear before the pure reconstruction.
    drop(app_data);
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed =
        restore_fixed::FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    target
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let restored = target.analysis_flow(&new_meeting).unwrap().unwrap();
    assert_eq!(restored.stage, "attention");
    assert_eq!(
        restored.plan.as_ref().unwrap()["whisperManaged"],
        plan["whisperManaged"]
    );
    assert_eq!(
        restored.plan.as_ref().unwrap()["sherpaManaged"],
        plan["sherpaManaged"]
    );
    for (id, snapshot) in jobs.iter().zip(&snapshots) {
        let new = ids.get("processing_jobs", &meeting, id).unwrap();
        let (spec, _) = target.fixed_job_input(&new).unwrap();
        assert_eq!(&spec.profile_snapshot, snapshot);
        assert!(!serde_json::to_string(&spec.parameters)
            .unwrap()
            .contains("/models/"));
    }
    let views = target.model_profiles().unwrap();
    assert!(views
        .iter()
        .all(|v| v.id.is_none() && !v.configuration_complete));
    assert!(target.acquire_local_inference_job().unwrap().is_none());
    let next = output.path().join("managed-rebackup.monbackup");
    target
        .create_backup_archive(&next, &crate::test_support::never_cancelled)
        .unwrap();
    let next = archive::read(
        &mut File::open(next).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        next.manifest.file_references,
        source.manifest.file_references
    );
    let next_ids = RestoreIds::prepare(
        &next,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let next_fixed = restore_fixed::FixedData::new(
        &next_ids,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    SealedWorkspace::prepare(
        &next_fixed,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
}

#[test]
fn restore_model_metadata_diarization_rebackup_preserves_omitted_log_reason_and_reconstructs_again()
{
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let (storage, meeting, engine, request) =
        crate::storage::diarization::tests::fixture(root.path(), true);
    let engine_paths = [
        engine.executable.path.clone(),
        engine.segmentation_model.path.clone(),
        engine.embedding_model.path.clone(),
    ];
    let job = crate::storage::diarization::tests::enqueue(&storage, request, engine);
    storage.acquire_local_inference_job().unwrap().unwrap();
    let revision = crate::diarization::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let original_view = storage
        .load_visible_analysis(&meeting, &revision)
        .unwrap()
        .unwrap();
    let path = output.path().join("diarization.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(source
        .manifest
        .omitted_assets
        .iter()
        .any(|a| matches!(a.reason, archive::OmittedReason::NativeLog)));
    for path in engine_paths {
        std::fs::remove_file(path).unwrap();
    }
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed =
        restore_fixed::FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let work =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    target
        .publish_restore(&work, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let new_revision = ids.get("analysis_revisions", &meeting, &revision).unwrap();
    let view = target
        .load_visible_analysis(&new_meeting, &new_revision)
        .unwrap()
        .unwrap();
    assert_eq!(view.utterances.len(), original_view.utterances.len());
    for (new, old) in view.utterances.iter().zip(&original_view.utterances) {
        assert_eq!(
            (&new.recognized_text, new.start_us, new.end_us),
            (&old.recognized_text, old.start_us, old.end_us)
        );
    }
    let backup = output.path().join("diarization-rebackup.monbackup");
    target
        .create_backup_archive(&backup, &crate::test_support::never_cancelled)
        .unwrap();
    let next = archive::read(
        &mut File::open(backup).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        next.manifest.file_references,
        source.manifest.file_references
    );
    assert_eq!(
        next.manifest
            .omitted_assets
            .iter()
            .filter(|a| matches!(a.reason, archive::OmittedReason::NativeLog))
            .count(),
        source
            .manifest
            .omitted_assets
            .iter()
            .filter(|a| matches!(a.reason, archive::OmittedReason::NativeLog))
            .count()
    );
    let next_ids = RestoreIds::prepare(
        &next,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let next_fixed = restore_fixed::FixedData::new(
        &next_ids,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let _next_work = SealedWorkspace::prepare(
        &next_fixed,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(target.acquire_local_inference_job().unwrap().is_none());
}

fn metadata(name: &str, hash: char, role: &str) -> ModelFileMetadata {
    ModelFileMetadata {
        name: name.into(),
        sha256: hash.to_string().repeat(64),
        size: 5,
        role: role.into(),
    }
}
#[test]
fn restore_model_metadata_redaction_reuses_only_exact_unavailable_fingerprints_and_not_unused_names(
) {
    let known = vec![
        metadata("model.bin", 'a', "model"),
        metadata("alias.bin", 'a', "model"),
        metadata("wrong-size", 'a', "model"),
        metadata("other.bin", 'b', "model"),
        metadata("cli", 'a', "executable"),
    ];
    let mut known = known;
    known[2].size = 9;
    let bytes=serde_json::to_vec(&serde_json::json!({"model":{"path":format!("mon-unavailable:{}","a".repeat(64)),"size":5,"sha256":"a".repeat(64)}})).unwrap();
    let mut refs = std::collections::BTreeSet::new();
    assert_eq!(
        portable::portable_json_with_metadata(&bytes, false, &mut refs, &known).unwrap(),
        bytes
    );
    assert_eq!(
        refs.into_iter().collect::<Vec<_>>(),
        vec![known[1].clone(), known[0].clone()]
    );
    let local=serde_json::to_vec(&serde_json::json!({"model":{"path":"/local/current.bin","size":5,"sha256":"a".repeat(64)}})).unwrap();
    let mut refs = std::collections::BTreeSet::new();
    portable::portable_json_with_metadata(&local, false, &mut refs, &known).unwrap();
    assert_eq!(
        refs.into_iter().collect::<Vec<_>>(),
        vec![metadata("current.bin", 'a', "model")]
    );
}

#[test]
fn restore_model_metadata_rebackup_rerestore_keeps_native_transcription_name_text_times_and_unavailability(
) {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let (storage, meeting, engine) = crate::storage::transcription::tests::fixture(root.path(), 10);
    let model = engine.model.path.clone();
    let cli = engine.executable.path.clone();
    let job =
        crate::transcription::enqueue(&storage, &meeting, "metadata-roundtrip", engine, "standard")
            .unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    let revision = crate::transcription::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let expected = storage
        .load_visible_analysis(&meeting, &revision)
        .unwrap()
        .unwrap();
    let archive_path = output.path().join("source.monbackup");
    storage
        .create_backup_archive(&archive_path, &crate::test_support::never_cancelled)
        .unwrap();
    let mut source = archive::read(
        &mut File::open(&archive_path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let original = source.manifest.file_references.clone();
    assert!(original
        .iter()
        .any(|r| r.name == "model.bin" && r.role == "model"));
    std::fs::remove_file(model).unwrap();
    std::fs::remove_file(cli).unwrap(); // Fixture-owned, no real models.
    let mut old_meeting = meeting;
    let mut old_revision = revision;
    let mut roots = vec![];
    for generation in 0..3 {
        let target_root = tempfile::tempdir().unwrap();
        let target = Storage::open(target_root.path().into()).unwrap();
        let ids = match restore_ids::RestoreIds::prepare_for_batch(
            &source,
            output.path(),
            &Uuid::new_v4().to_string(),
            &target.connection.lock().unwrap(),
            &crate::test_support::never_cancelled,
        )
        .unwrap()
        {
            restore_ids::BatchPreparation::Pending(ids) => ids,
            _ => panic!(),
        };
        let fixed = restore_fixed::FixedData::new(
            &ids,
            output.path(),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        let work =
            SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
                .unwrap();
        let new_meeting = ids.get("meetings", "", &old_meeting).unwrap();
        let new_revision = ids
            .get("analysis_revisions", &old_meeting, &old_revision)
            .unwrap();
        target
            .publish_restore(&work, &ids, &crate::test_support::never_cancelled)
            .unwrap();
        drop(work);
        drop(fixed);
        drop(ids);
        let restored = target
            .load_visible_analysis(&new_meeting, &new_revision)
            .unwrap()
            .unwrap();
        assert_eq!(
            restored.utterances[0].recognized_text,
            expected.utterances[0].recognized_text
        );
        assert_eq!(
            (
                restored.utterances[0].start_us,
                restored.utterances[0].end_us
            ),
            (
                expected.utterances[0].start_us,
                expected.utterances[0].end_us
            )
        );
        assert!(target.acquire_local_inference_job().unwrap().is_none());
        {
            let mut db = target.connection.lock().unwrap();
            let assets = super::super::read_assets(&db).unwrap();
            drop(db);
            let canonical = target.recorded_audio_plan(&new_meeting).unwrap();
            let source_id = &canonical.tracks[0].chunks[0].asset_id;
            let source = assets.iter().find(|a| &a.id == source_id).unwrap();
            let mut db = target.connection.lock().unwrap();
            assert!(!portable::temporary_pcm(&db, source).unwrap());
            let omitted: i64 = db
                .query_row(
                    "SELECT count(*) FROM restore_omitted_assets WHERE asset_id=?1",
                    [source_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(omitted, 0);
            let tx = db.transaction().unwrap();
            tx.execute(
                "UPDATE file_assets SET state='missing' WHERE id=?1",
                [source_id],
            )
            .unwrap();
            assert!(
                super::super::read_assets(&tx).is_err(),
                "the sole meeting WAV is never an omitted cache"
            );
            tx.rollback().unwrap();
        }
        assert_eq!(
            retained_files(&target.connection.lock().unwrap())
                .unwrap()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            original.iter().cloned().collect()
        );
        let path = output
            .path()
            .join(format!("generation-{generation}.monbackup"));
        target
            .create_backup_archive(&path, &crate::test_support::never_cancelled)
            .unwrap();
        let managed = target.root.clone();
        drop(target);
        let reopened = Storage::open(managed).unwrap();
        assert!(reopened
            .load_visible_analysis(&new_meeting, &new_revision)
            .unwrap()
            .is_some());
        drop(reopened);
        source = archive::read(
            &mut File::open(&path).unwrap(),
            output.path(),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(source.manifest.file_references, original);
        assert_eq!(source.manifest.tables.len(), portable::TABLES.len());
        for table in crate::storage::model_metadata::TABLES {
            assert_eq!(
                source
                    .database
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            source
                .database
                .query_row("SELECT count(*) FROM restore_omitted_assets", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        old_meeting = new_meeting;
        old_revision = new_revision;
        roots.push(target_root);
    }
}
