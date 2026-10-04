use super::*;
use crate::storage::model_metadata::{historical, ModelFileMetadata};

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
    let revision = crate::diarization::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let original_view = storage
        .load_visible_analysis(&meeting, &revision)
        .unwrap()
        .unwrap();
    let path = output.path().join("diarization.monbackup");
    storage.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &|| false).unwrap();
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
    let fixed = restore_fixed::FixedData::new(&ids, output.path(), &|| false).unwrap();
    let work = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    target.publish_restore(&work, &ids, &|| false).unwrap();
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
    target.create_backup_archive(&backup, &|| false).unwrap();
    let next = archive::read(&mut File::open(backup).unwrap(), output.path(), &|| false).unwrap();
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
    let next_ids =
        RestoreIds::prepare(&next, output.path(), &Uuid::new_v4().to_string(), &|| false).unwrap();
    let next_fixed = restore_fixed::FixedData::new(&next_ids, output.path(), &|| false).unwrap();
    let _next_work = SealedWorkspace::prepare(&next_fixed, output.path(), &|| false).unwrap();
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
    let revision = crate::transcription::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let expected = storage
        .load_visible_analysis(&meeting, &revision)
        .unwrap()
        .unwrap();
    let archive_path = output.path().join("source.monbackup");
    storage
        .create_backup_archive(&archive_path, &|| false)
        .unwrap();
    let mut source = archive::read(
        &mut File::open(&archive_path).unwrap(),
        output.path(),
        &|| false,
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
            &|| false,
        )
        .unwrap()
        {
            restore_ids::BatchPreparation::Pending(ids) => ids,
            _ => panic!(),
        };
        let fixed = restore_fixed::FixedData::new(&ids, output.path(), &|| false).unwrap();
        let work = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
        let new_meeting = ids.get("meetings", "", &old_meeting).unwrap();
        let new_revision = ids
            .get("analysis_revisions", &old_meeting, &old_revision)
            .unwrap();
        target.publish_restore(&work, &ids, &|| false).unwrap();
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
            let omitted: String = db
                .query_row(
                    "SELECT asset_id FROM restore_omitted_assets LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let tx = db.transaction().unwrap();
            tx.execute(
                "DELETE FROM restore_omitted_assets WHERE asset_id=?1",
                params![omitted],
            )
            .unwrap();
            assert!(
                super::super::read_assets(&tx).is_err(),
                "missing cache without its saved exclusion must not be silently omitted"
            );
            tx.rollback().unwrap();
            let assets = super::super::read_assets(&db).unwrap();
            let cache = assets.iter().find(|a| a.id == omitted).unwrap();
            assert!(portable::temporary_pcm(&db, cache).unwrap());
            let mut changed = cache.clone();
            changed.content_hash = "f".repeat(64);
            assert!(!portable::temporary_pcm(&db, &changed).unwrap());
            let mut original_audio = assets
                .iter()
                .find(|a| a.media_type == "audio/x-caf")
                .unwrap()
                .clone();
            original_audio.state = "missing".into();
            original_audio.media_type = "audio/wav".into();
            assert!(!portable::temporary_pcm(&db, &original_audio).unwrap());
            assert!(db
                .execute(
                    "UPDATE restore_omitted_assets SET reason='native_log' WHERE asset_id=?1",
                    params![omitted]
                )
                .is_err());
        }
        assert_eq!(
            historical(&target.connection.lock().unwrap())
                .unwrap()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            original.iter().cloned().collect()
        );
        let path = output
            .path()
            .join(format!("generation-{generation}.monbackup"));
        target.create_backup_archive(&path, &|| false).unwrap();
        let managed = target.root.clone();
        drop(target);
        let reopened = Storage::open(managed).unwrap();
        assert!(reopened
            .load_visible_analysis(&new_meeting, &new_revision)
            .unwrap()
            .is_some());
        drop(reopened);
        source = archive::read(&mut File::open(&path).unwrap(), output.path(), &|| false).unwrap();
        assert_eq!(source.manifest.file_references, original);
        assert_eq!(source.manifest.tables.len(), 31);
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
