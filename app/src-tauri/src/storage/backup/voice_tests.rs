use super::*;
use restore_fixed::workspace::seal::SealedWorkspace;
use restore_fixed::FixedData;
use restore_ids::RestoreIds;
use serde_json::{json, Value};

#[test]
fn voice_threshold_settings_round_trip_without_exporting_other_application_settings() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    archive::tests::meeting(&s, "voice-settings");
    let preferences = crate::storage::voice_preferences::VoiceMatchingPreferences {
        similarity_threshold_milli: 730,
        margin_threshold_milli: 60,
    };
    s.save_voice_matching_preferences(0, preferences).unwrap();
    let path = output.path().join("thresholds.monbackup");
    s.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &|| false).unwrap();
    assert_eq!(
        source.manifest.voice_matching_preferences,
        Some(preferences)
    );
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored.publish_restore(&sealed, &ids, &|| false).unwrap();
    assert_eq!(
        restored.voice_matching_preferences().unwrap().payload,
        preferences
    );
    let version = restored.voice_matching_preferences().unwrap().lock_version;
    restored.publish_restore(&sealed, &ids, &|| false).unwrap();
    assert_eq!(
        restored.voice_matching_preferences().unwrap().lock_version,
        version
    );
}

#[test]
fn speaker_assignment_update_round_trip_preserves_saved_evidence_and_receipt() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "assignment-source");
    let (track, manifest): (String, String) = {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE working_drafts SET active=0 WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
        db.query_row("SELECT t.id,a.audio_manifest_hash FROM meetings m JOIN audio_tracks t ON t.meeting_id=m.id JOIN analysis_revisions a ON a.id=m.current_analysis_id WHERE m.id=?1",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
    };
    let id = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: manifest,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "a".into(),
                    label: "話者A".into(),
                }],
                segments: vec![SpeakerSegmentInput {
                    track_id: track.clone(),
                    speaker_key: Some("a".into()),
                    start_us: 0,
                    end_us: 1_000_000,
                    overlap_group: None,
                    confidence: None,
                }],
                utterances: vec![
                    UtteranceInput {
                        track_id: track.clone(),
                        start_us: 0,
                        end_us: 500000,
                        text: "Hello".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    },
                    UtteranceInput {
                        track_id: track,
                        start_us: 500000,
                        end_us: 1000000,
                        text: "world".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    },
                ],
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(
            &meeting,
            &id,
            storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &id).unwrap();
    let request = crate::storage::transcript_consolidation::Request {
        meeting_id: meeting.clone(),
        analysis_id: id.clone(),
        expected_content_hash: view.content_hash,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: 0,
        }),
        operation_id: Uuid::new_v4().to_string(),
    };
    let next = storage.update_speaker_assignments(&request).unwrap();
    assert_eq!(next.updated_count, 2);
    assert_eq!(next.analysis.utterances.len(), 2);
    let path = output.path().join("consolidated.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let restored_meeting = ids.get("meetings", "", &meeting).unwrap();
    let current = restored
        .load_current_analysis(&restored_meeting)
        .unwrap()
        .unwrap();
    assert_eq!(current.utterances.len(), 2);
    assert_eq!(current.utterances[0].effective_text, "Hello");
    assert!(current.utterances.iter().all(|u| u.speaker_ids.len() == 1));
    let db = restored.connection.lock().unwrap();
    let config: String = db
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [&current.id],
            |r| r.get(0),
        )
        .unwrap();
    let config: Value = serde_json::from_str(&config).unwrap();
    let stamp: crate::storage::DraftStamp =
        serde_json::from_value(config["speakerAssignmentUpdate"]["sourceDraft"].clone()).unwrap();
    assert_eq!(
        stamp.id,
        ids.get(
            "working_drafts",
            &meeting,
            request.expected_draft.as_ref().unwrap().id.as_str()
        )
        .unwrap()
    );
    assert_eq!(config["speakerAssignmentUpdate"]["updatedCount"], 2);
    drop(db);
    restored
        .create_backup_archive(&output.path().join("second-assignment.monbackup"), &|| {
            false
        })
        .unwrap();
}

#[test]
fn transcript_consolidation_round_trip_preserves_records_history_and_lineage_links() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "consolidation-source");
    let (track, manifest): (String, String) = {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE working_drafts SET active=0 WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
        db.query_row("SELECT t.id,a.audio_manifest_hash FROM meetings m JOIN audio_tracks t ON t.meeting_id=m.id JOIN analysis_revisions a ON a.id=m.current_analysis_id WHERE m.id=?1",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
    };
    let id = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: manifest,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "a".into(),
                    label: "話者A".into(),
                }],
                segments: vec![],
                utterances: vec![
                    UtteranceInput {
                        track_id: track.clone(),
                        start_us: 0,
                        end_us: 500000,
                        text: "Hello".into(),
                        speaker_keys: vec!["a".into()],
                        confidence: None,
                    },
                    UtteranceInput {
                        track_id: track,
                        start_us: 500000,
                        end_us: 1000000,
                        text: "world".into(),
                        speaker_keys: vec!["a".into()],
                        confidence: None,
                    },
                ],
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(
            &meeting,
            &id,
            storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &id).unwrap();
    let request = crate::storage::transcript_consolidation::Request {
        meeting_id: meeting.clone(),
        analysis_id: id.clone(),
        expected_content_hash: view.content_hash,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: 0,
        }),
        operation_id: Uuid::new_v4().to_string(),
    };
    let next = storage.consolidate_transcript(&request).unwrap();
    assert!(next.changed);
    assert_eq!(next.analysis.utterances.len(), 1);
    let path = output.path().join("consolidated.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let restored_meeting = ids.get("meetings", "", &meeting).unwrap();
    let current = restored
        .load_current_analysis(&restored_meeting)
        .unwrap()
        .unwrap();
    assert_eq!(current.utterances.len(), 1);
    assert_eq!(current.utterances[0].effective_text, "Hello world");
    let db = restored.connection.lock().unwrap();
    let config: String = db
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [&current.id],
            |r| r.get(0),
        )
        .unwrap();
    let config: Value = serde_json::from_str(&config).unwrap();
    let p: crate::storage::transcript_consolidation::Provenance =
        serde_json::from_value(config["utteranceConsolidation"].clone()).unwrap();
    assert_eq!(p.groups.len(), 1);
    for lineage in p.groups[0]
        .source_lineages
        .iter()
        .chain([&p.groups[0].lineage_id])
    {
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM utterances WHERE lineage_id=?1)",
                [lineage],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
    drop(db);
    restored
        .create_backup_archive(
            &output.path().join("second-consolidated.monbackup"),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
}

#[test]
fn voice_registry_round_trip_remaps_people_and_sources_without_merging_names() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "voice-source");
    let person = Uuid::new_v4().to_string();
    let sample = Uuid::new_v4().to_string();
    let vector = crate::voice::encode(&[3., 4.]).unwrap();
    {
        let db = storage.connection.lock().unwrap();
        let (analysis,track):(String,String)=db.query_row("SELECT m.current_analysis_id,t.id FROM meetings m JOIN audio_tracks t ON t.meeting_id=m.id WHERE m.id=?1",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        db.execute("INSERT INTO voice_people VALUES(?1,'田中',1,0)", [&person])
            .unwrap();
        db.execute(
            "INSERT INTO voice_samples VALUES(?1,?2,?3,?4,2,?5,?6,?7,?8,0,1000000,0)",
            params![
                sample,
                person,
                "a".repeat(64),
                crate::voice::EXTRACTOR,
                vector,
                meeting,
                analysis,
                track
            ],
        )
        .unwrap();
    }
    let path = output.path().join("voices.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(&path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let destination = Storage::open(target.path().into()).unwrap();
    destination
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO voice_people VALUES('destination-person','田中',1,0)",
            [],
        )
        .unwrap();
    destination
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let restored = destination.voice_people().unwrap();
    assert_eq!(restored.len(), 2);
    let restored = restored
        .into_iter()
        .find(|p| !p.samples.is_empty())
        .unwrap();
    assert_eq!(restored.name, "田中");
    assert_ne!(restored.id, person);
    assert_ne!(restored.samples[0].id, sample);
    assert_ne!(
        restored.samples[0].source_meeting_id.as_ref().unwrap(),
        &meeting
    );
    assert!(restored.samples[0].source_available);
    let actual: Vec<u8> = destination
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT vector FROM voice_samples", [], |r| r.get(0))
        .unwrap();
    assert_eq!(actual, vector);
    // A subsequent backup must also accept global registry provenance.
    destination
        .create_backup_archive(
            &output.path().join("second.monbackup"),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
}

#[test]
fn voice_registry_exports_samples_even_when_the_source_meeting_was_deleted() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO voice_people VALUES('person','登録者',1,0)", [])
            .unwrap();
        db.execute("INSERT INTO voice_samples VALUES('sample','person',?1,?2,2,?3,'deleted-meeting','deleted-analysis','deleted-track',0,1000000,0)",params!["a".repeat(64),crate::voice::EXTRACTOR,crate::voice::encode(&[1.,0.]).unwrap()]).unwrap();
    }
    let path = output.path().join("voices.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let row = ids.relational_row("voice_samples", 1).unwrap();
    assert_eq!(row[6], rusqlite::types::Value::Null);
    assert_eq!(row[7], rusqlite::types::Value::Null);
    assert_eq!(row[8], rusqlite::types::Value::Null);
}

#[test]
fn voice_registry_refuses_corrupt_features_before_export() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let output = tempfile::tempdir().unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO voice_people VALUES('person','登録者',1,0)", [])
            .unwrap();
        db.execute("INSERT INTO voice_samples VALUES('sample','person',?1,?2,2,?3,NULL,NULL,NULL,0,1000000,0)",params!["a".repeat(64),crate::voice::EXTRACTOR,vec![0u8;8]]).unwrap();
    }
    assert!(storage
        .create_backup_archive(
            &output.path().join("bad.monbackup"),
            &crate::test_support::never_cancelled
        )
        .is_err());
}

#[test]
fn voice_registration_history_does_not_resurrect_a_deleted_person_or_block_restore() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let m = archive::tests::meeting(&s, "voice-deleted-person");
    let analysis = s.load_current_analysis(&m).unwrap().unwrap();
    let path = output.path().join("source.monbackup");
    s.create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let fp = crate::local_engine::Fingerprint {
        path: PathBuf::from("/source/model"),
        size: 1,
        sha256: "a".repeat(64),
    };
    let plan = crate::voice::Plan {
        resident_runtime: crate::local_engine::resident::prepare(s.root(), "sherpa").unwrap(),
        matching_options: None,
        matching_policy: None,
        schema_version: crate::current_formats::VOICE,
        engine: crate::diarization::sherpa::SherpaConfig {
            executable: fp.clone(),
            embedding_model: fp.clone(),
            segmentation_model: fp.clone(),
            runtime_libraries: vec![fp.clone()],
            threads: 1,
            cluster_threshold_milli: 500,
            window_shift_milli: 100,
            min_duration_on_ms: 0,
            min_duration_off_ms: 0,
            speech_sensitivity_milli: 0,
        },
        extractor: crate::diarization::embedding::ExtractorConfig {
            library_index: 0,
            helper: fp.clone(),
            library: fp,
        },
        book_hash: "b".repeat(64),
        working_hash: "c".repeat(64),
        audio_manifest_hash: "d".repeat(64),
        person_id: Some("intentionally-deleted-person".into()),
        person_name: None,
        windows: vec![],
    };
    let restored = fixed
        .parameters(&m, Some(&analysis.id), &serde_json::json!({"voice":plan}))
        .unwrap();
    assert!(restored["voice"]["personId"].is_null());
    assert!(restored["voiceRequest"].is_null());
}

#[test]
fn speaker_completion_settings_plan_result_and_evidence_survive_additive_restore() {
    let (_root, s, meeting, mut spec, plan) = crate::storage::speaker_completion::tests::fixture();
    let preferences = crate::storage::completion_preferences::CompletionPreferences {
        similarity_threshold_milli: 730,
        margin_threshold_milli: 60,
    };
    s.save_speaker_completion_preferences(0, preferences)
        .unwrap();
    spec.parameters["assignmentRequest"] = json!({"meetingId":meeting,"analysisId":spec.input_analysis_id,"expectedContentHash":spec.input_hash,"expectedDraft":null,"operationId":spec.operation_id});
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let d = crate::speaker_completion::Decision {
        needs_review: true,
        lineage: plan.targets[0].lineage.clone(),
        speaker_id: Some(plan.references[0].speaker_id.clone()),
        body_best: Some(plan.references[0].speaker_id.clone()),
        similarity: Some(0.53),
        runner_up: Some(0.2),
        reason: "nearest_below_threshold".into(),
    };
    s.publish_completion_result(&spec, &plan, &[d], &job.id)
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let path = output.path().join("completion.monbackup");
    s.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(path).unwrap(), output.path(), &|| false).unwrap();
    assert_eq!(
        source.manifest.speaker_completion_preferences,
        Some(preferences)
    );
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored.publish_restore(&sealed, &ids, &|| false).unwrap();
    assert_eq!(
        restored.speaker_completion_preferences().unwrap().payload,
        preferences
    );
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let view = restored
        .load_current_analysis(&new_meeting)
        .unwrap()
        .unwrap();
    assert_eq!(view.utterances[1].speaker_ids.len(), 1);
    let info = view.utterances[1].speaker_identification.as_ref().unwrap();
    assert!(info.needs_review);
    assert_eq!(info.similarity_milli, 530);
    let new_job = ids.get("processing_jobs", &meeting, &job.id).unwrap();
    assert_eq!(
        restored.notification_output(&new_job).unwrap().warnings,
        vec![
            "1件を補完、0件を見送りました",
            "うち1件は低信頼のため要確認です"
        ]
    );
    // Restored source editor hashes cannot authorize writes in a new meeting.
    let (spec, _) = restored.fixed_job_input(&new_job).unwrap();
    let plan = serde_json::from_value(spec.parameters["completion"].clone()).unwrap();
    assert!(restored.check_completion_plan(&spec, &plan).is_err());
}
