//! Owned historical reconstruction and real publication, not a quality benchmark.
use super::*;
use crate::storage::backup::{
    restore_fixed::workspace::seal::SealedWorkspace, restore_fixed::FixedData,
    restore_ids::RestoreIds,
};

fn historical_ids<'a>(a: &'a VerifiedArchive, output: &Path) -> RestoreIds<'a> {
    RestoreIds::prepare_historical(
        a,
        output,
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
}

fn assert_missing_sources(a: &VerifiedArchive, ids: &RestoreIds<'_>, sealed: &SealedWorkspace) {
    let root = sealed.root_path().unwrap();
    for omitted in &a.manifest.omitted_assets {
        if omitted.reason != OmittedReason::SourceAudioDeleted {
            continue;
        }
        let id = ids
            .get("file_assets", &omitted.meeting_id, &omitted.asset_id)
            .unwrap();
        let (state, size, hash, path): (String, i64, String, String) = sealed
            .connection()
            .query_row(
                "SELECT state,size_bytes,content_hash,relative_path FROM file_assets WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (state, size, hash),
            (
                "missing".into(),
                omitted.source_size as i64,
                omitted.source_sha256.clone()
            )
        );
        assert!(!root.join(path).exists());
    }
    assert_eq!(
        sealed
            .connection()
            .query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn deleted_audio_workspace_publishes_fixed_jobs_drafts_minutes_and_missing_sources_atomically() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("root")).unwrap();
    let meeting = tests::meeting_with_mix(&storage, "deleted-fixed-workspace");
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let request = crate::storage::MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id.clone(),
        expected_content_hash: analysis.content_hash.clone(),
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(crate::storage::DraftStamp {
            id: draft.id.clone(),
            lock_version: draft.lock_version,
        }),
    };
    let generation = crate::generation::markdown_job::enqueue_synthetic(
        &storage,
        request,
        crate::storage::markdown_generation_tests::engine(root.path(), "normal"),
        "standard",
        2400,
        None,
    )
    .unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    crate::storage::markdown_generation_tests::process(
        &storage,
        &generation.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let segmentation = root.path().join("segmentation.onnx");
    let embedding = root.path().join("embedding.onnx");
    std::fs::write(&segmentation, b"owned segmentation").unwrap();
    std::fs::write(&embedding, b"owned embedding").unwrap();
    let library = crate::storage::diarization::tests::extraction_fixture(root.path());
    let diarization_engine = crate::diarization::sherpa::SherpaConfig {
        executable: crate::local_engine::Fingerprint::capture(
            &root.path().join("llama-completion"),
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        segmentation_model: crate::local_engine::Fingerprint::capture(
            &segmentation,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        embedding_model: crate::local_engine::Fingerprint::capture(
            &embedding,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        runtime_libraries: vec![library],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 500,
        min_duration_on_ms: 200,
        min_duration_off_ms: 100,
        speech_sensitivity_milli: 0,
    };
    let request = crate::storage::MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: "deleted-cancelled-diarization".into(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(crate::storage::DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let track = storage
        .ensure_meeting_mix(&meeting, &|| false, &|_, _| {})
        .unwrap()
        .tracks[0]
        .id
        .clone();
    let diarization = crate::diarization::enqueue(
        &storage,
        request,
        diarization_engine,
        "standard",
        vec![crate::diarization::TrackSettings {
            track_id: track,
            num_speakers: Some(3),
        }],
    )
    .unwrap();
    storage
        .finish_job(&diarization.id, 0, "cancelled", None)
        .unwrap();
    let neighbor = tests::meeting(&storage, "available-workspace-neighbor");
    let old_content = storage.get_minutes_markdown(&meeting).unwrap().unwrap();
    let old_plan = storage.recorded_analysis_input(&meeting).unwrap().0;
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (source, _) = write(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    assert_missing_sources(&source, &ids, &sealed);
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    assert!(
        crate::storage::transcription::read_audio_plan(sealed.connection(), &new_meeting).is_err()
    );
    let mut query = sealed.connection().prepare("SELECT parameters_json FROM processing_jobs WHERE meeting_id=?1 AND kind IN ('transcription','diarization')").unwrap();
    for value in query
        .query_map([&new_meeting], |r| r.get::<_, String>(0))
        .unwrap()
    {
        let value: Value = serde_json::from_str(&value.unwrap()).unwrap();
        let plan: crate::storage::transcription::RecordedAudioPlan = serde_json::from_value(
            value
                .get("source")
                .unwrap_or(&value["diarization"]["source"])
                .clone(),
        )
        .unwrap();
        assert_eq!(
            plan.tracks[0].chunks[0].hash,
            old_plan.tracks[0].chunks[0].hash
        );
        assert_eq!(
            plan.tracks[0].chunks[0].frames,
            old_plan.tracks[0].chunks[0].frames
        );
        assert_eq!(
            plan.tracks[0].chunks[0].start_us,
            old_plan.tracks[0].chunks[0].start_us
        );
        assert_ne!(plan.tracks[0].id, old_plan.tracks[0].id);
    }
    let db = Connection::open(sealed.root_path().unwrap().join("fixed.sqlite3")).unwrap();
    let restored: String = db
        .query_row(
            "SELECT markdown FROM minutes_markdown WHERE meeting_id=?1",
            [&new_meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(restored, old_content.markdown);
    drop(db);
    let target = Storage::open(root.path().join("target")).unwrap();
    assert!(matches!(
        target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        crate::storage::backup::publication::Publication::Published { .. }
    ));
    assert!(matches!(
        target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        crate::storage::backup::publication::Publication::AlreadyPublished
    ));
    assert_eq!(target.list_meetings().unwrap().len(), 2);
    assert!(target.recorded_audio_plan(&new_meeting).is_err());
    assert!(target
        .load_transcript_draft(&new_meeting)
        .unwrap()
        .is_some());
    assert_eq!(
        target
            .get_minutes_markdown(&new_meeting)
            .unwrap()
            .unwrap()
            .markdown,
        old_content.markdown
    );
    let mut db = target.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(tx.query_row("SELECT count(*) FROM maintenance_operations WHERE kind='delete_assets' AND state='completed'", [], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(tx.query_row("SELECT count(*) FROM restore_omitted_assets WHERE meeting_id=?1 AND reason='recreatable_pcm'", [&new_meeting], |r|r.get::<_,i64>(0)).unwrap(),0);
    drop(tx);
    drop(db);
    assert!(storage.recorded_audio_plan(&neighbor).is_ok());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
}

#[test]
fn deleted_audio_workspace_rebuilds_saved_whisper_result_and_deleted_pcm_without_engine_reexecution(
) {
    use std::os::unix::fs::PermissionsExt;
    let (root, storage, meeting) = fixture();
    let model = root.path().join("model.bin");
    let cli = root.path().join("fake-whisper");
    let calls = root.path().join("cli-calls");
    std::fs::write(&model, b"owned synthetic model").unwrap();
    let model = model.canonicalize().unwrap();
    let result = json!({"model":{"multilingual":true},"params":{"model":model,"language":"ja","translate":false},"transcription":[{"offsets":{"from":0,"to":500},"text":"点検日です"}]}).to_string();
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nprintf '%s' '{result}' > result.json\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = crate::transcription::whisper::WhisperConfig {
        executable: crate::local_engine::Fingerprint::capture(
            &cli,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        model: crate::local_engine::Fingerprint::capture(
            &model,
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        vad_model: None,
        vad_options: None,
        threads: 1,
        use_gpu: false,
        prompt: "点検日".into(),
    };
    let job = crate::transcription::enqueue(
        &storage,
        &meeting,
        "deleted-source-result",
        engine,
        "standard",
    )
    .unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    crate::transcription::process(
        &storage,
        &job.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let checkpoint = storage.fixed_job_input(&job.id).unwrap().1.unwrap();
    let old_result = checkpoint["outputs"][0]["jsonAssetId"].as_str().unwrap();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (source, _) = write(&storage, output.path());
    assert!(source
        .manifest
        .omitted_assets
        .iter()
        .any(|a| a.reason == OmittedReason::SourceAudioDeleted
            && a.asset_id == checkpoint["outputs"][0]["wavAssetId"].as_str().unwrap()));
    std::fs::remove_file(&cli).unwrap();
    std::fs::remove_file(&model).unwrap();
    let before = std::fs::read(&calls).unwrap();
    let ids = historical_ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    sealed
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    assert_missing_sources(&source, &ids, &sealed);
    let new_result = ids.get("file_assets", &meeting, old_result).unwrap();
    let path: String = sealed
        .connection()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1 AND state='ready'",
            [new_result],
            |r| r.get(0),
        )
        .unwrap();
    let restored: Value =
        serde_json::from_slice(&std::fs::read(sealed.root_path().unwrap().join(path)).unwrap())
            .unwrap();
    let original: Value =
        serde_json::from_slice(&std::fs::read(&source.assets[old_result]).unwrap()).unwrap();
    assert_eq!(restored["transcription"], original["transcription"]);
    let target = Storage::open(storage.root.parent().unwrap().join("target-whisper")).unwrap();
    target
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    assert!(target.recorded_audio_plan(&new_meeting).is_err());
    assert!(target.acquire_local_inference_job().unwrap().is_none());
    assert_eq!(std::fs::read(calls).unwrap(), before);
}

#[test]
fn deleted_audio_workspace_cannot_register_missing_source_as_body_or_rebind_other_meeting() {
    let (_root, storage, meeting) = fixture();
    let neighbor = tests::meeting(&storage, "absent-asset-neighbor");
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (source, _) = write(&storage, output.path());
    let ids = historical_ids(&source, output.path());
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let omitted = source
        .manifest
        .omitted_assets
        .iter()
        .find(|a| a.reason == OmittedReason::SourceAudioDeleted)
        .unwrap();
    fixed
        .absent_source_asset(&meeting, &omitted.asset_id)
        .unwrap();
    assert_eq!(
        fixed
            .asset_hash(&meeting, &omitted.asset_id, &omitted.source_sha256)
            .unwrap(),
        (omitted.source_size as i64, omitted.source_sha256.clone())
    );
    assert!(fixed
        .asset_bytes(&meeting, &omitted.asset_id, b"fake restored audio")
        .is_err());
    assert!(fixed
        .absent_source_asset(&neighbor, &omitted.asset_id)
        .is_err());
}

#[test]
fn deleted_audio_workspace_historical_identity_requires_unchanged_checked_database_and_cancellation(
) {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (source, _) = write(&storage, output.path());
    assert!(RestoreIds::prepare_historical(
        &source,
        output.path(),
        &Uuid::new_v4().to_string(),
        &|| true
    )
    .is_err());
    change_logical(
        &source,
        "UPDATE audio_chunks SET frame_count=frame_count+1 WHERE meeting_id=?1",
        &meeting,
    );
    assert!(RestoreIds::prepare_historical(
        &source,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(storage.get_meeting(&meeting).unwrap().is_some());
}

#[test]
fn deleted_audio_workspace_rejects_mutated_omission_reason_count_owner_and_identity() {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    for attack in ["reason", "drop", "duplicate", "owner", "hash", "size"] {
        let (mut source, _) = write(&storage, output.path());
        let index = source
            .manifest
            .omitted_assets
            .iter()
            .position(|a| a.reason == OmittedReason::SourceAudioDeleted)
            .unwrap();
        match attack {
            "reason" => {
                source.manifest.omitted_assets[index].reason = OmittedReason::RecreatablePcm
            }
            "drop" => {
                source.manifest.omitted_assets.remove(index);
            }
            "duplicate" => source
                .manifest
                .omitted_assets
                .push(source.manifest.omitted_assets[index].clone()),
            "owner" => {
                source.manifest.omitted_assets[index].meeting_id = Uuid::new_v4().to_string()
            }
            "hash" => source.manifest.omitted_assets[index].source_sha256 = "0".repeat(64),
            "size" => source.manifest.omitted_assets[index].source_size += 1,
            _ => unreachable!(),
        }
        assert!(
            RestoreIds::prepare_historical(
                &source,
                output.path(),
                &Uuid::new_v4().to_string(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{attack}"
        );
    }
}
