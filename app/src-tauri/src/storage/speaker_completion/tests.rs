use super::*;
use crate::local_engine::Fingerprint;
use serde_json::json;

pub(in crate::storage) fn fixture() -> (tempfile::TempDir, Storage, String, JobSpec, Plan) {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let meeting = s.create_meeting("話者補完テスト").unwrap().id;
    {
        let db = s.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type='recording',lifecycle='recorded' WHERE id=?1",
            [&meeting],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,audio_manifest_hash,created_at,updated_at) VALUES('session',?1,'saved','system','{}','start',?2,0,0)",params![meeting,"a".repeat(64)]).unwrap();
        db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES('track',?1,'session','processed','wav',16000,1,'ready',0,0)",[&meeting]).unwrap();
    }
    let dir = PathBuf::from("meetings").join(&meeting).join("audio");
    std::fs::create_dir_all(s.managed_path(&dir).unwrap()).unwrap();
    let manifest_path = dir.join("manifest.json");
    std::fs::write(
        s.managed_path(&manifest_path).unwrap(),
        b"{\"schemaVersion\":4,\"inputDiagnosticsState\":\"unavailable\",\"state\":\"saved\"}",
    )
    .unwrap();
    let manifest = s
        .register_ready_meeting_asset(&meeting, &manifest_path, "application/json")
        .unwrap();
    let audio = dir.join("meeting.wav");
    let mut bytes=crate::audio::wave::header(320000).unwrap(); bytes.extend(vec![0;640000]);
    std::fs::write(s.managed_path(&audio).unwrap(), bytes).unwrap();
    let asset = s
        .register_ready_meeting_asset(&meeting, &audio, "audio/wav")
        .unwrap();
    {
        let db = s.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET duration_us=20000000 WHERE id=?1",
            [&meeting],
        )
        .unwrap();
        db.execute(
            "UPDATE recording_sessions SET audio_manifest_hash=?1 WHERE meeting_id=?2",
            params![manifest.content_hash, meeting],
        )
        .unwrap();
        db.execute("DELETE FROM audio_chunks WHERE meeting_id=?1", [&meeting])
            .unwrap();
        db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES('long',?1,'track',?2,0,0,20000000,320000,16000,1,'ready',0)",params![meeting,asset.id]).unwrap();
        db.execute("INSERT INTO meeting_audio_commits VALUES(?1,'track',?2,?3,320000,'{}','[]','cleaned',0)",params![meeting,manifest.id,manifest.content_hash]).unwrap();
    }
    let id = s
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: manifest.content_hash,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![
                    SpeakerInput {
                        key: "a".into(),
                        label: "話者A".into(),
                    },
                    SpeakerInput {
                        key: "b".into(),
                        label: "話者B".into(),
                    },
                ],
                segments: vec![
                    SpeakerSegmentInput {
                        track_id: "track".into(),
                        speaker_key: Some("a".into()),
                        start_us: 0,
                        end_us: 5_000_000,
                        overlap_group: None,
                        confidence: None,
                    },
                    SpeakerSegmentInput {
                        track_id: "track".into(),
                        speaker_key: Some("b".into()),
                        start_us: 10_000_000,
                        end_us: 15_000_000,
                        overlap_group: None,
                        confidence: None,
                    },
                ],
                utterances: vec![
                    UtteranceInput {
                        track_id: "track".into(),
                        start_us: 1_000_000,
                        end_us: 4_000_000,
                        text: "参照A".into(),
                        speaker_keys: vec!["a".into()],
                        confidence: None,
                    },
                    UtteranceInput {
                        track_id: "track".into(),
                        start_us: 6_000_000,
                        end_us: 8_000_000,
                        text: "未検出".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    },
                    UtteranceInput {
                        track_id: "track".into(),
                        start_us: 11_000_000,
                        end_us: 14_000_000,
                        text: "参照B".into(),
                        speaker_keys: vec!["b".into()],
                        confidence: None,
                    },
                ],
            },
        )
        .unwrap();
    s.commit_analysis_candidate(
        &meeting,
        &id,
        s.get_meeting(&meeting).unwrap().unwrap().lock_version,
        false,
    )
    .unwrap();
    let (source, _) = s.recorded_analysis_input(&meeting).unwrap();
    let mut db = s.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    let (view, _, working_hash) = working(&tx, &meeting).unwrap();
    let known = transcription::known::read(&tx, &meeting, &source).unwrap();
    let options = super::super::completion_preferences::CompletionPreferences {
        ..Default::default()
    };
    let (references, targets, excluded) = evidence(&tx, &view, &known, &source).unwrap();
    assert_eq!(references.len(), 2);
    assert_eq!(targets.len(), 1);
    let fp = Fingerprint {
        path: PathBuf::from(crate::test_support::absolute("/test-only")),
        size: 1,
        sha256: "a".repeat(64),
    };
    let engine = crate::diarization::sherpa::SherpaConfig {
        executable: fp.clone(),
        segmentation_model: fp.clone(),
        embedding_model: fp.clone(),
        runtime_libraries: vec![fp.clone()],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
        speech_sensitivity_milli: 0,
    };
    let plan = Plan {
        resident_runtime: crate::local_engine::resident::prepare(s.root(), "sherpa").unwrap(),
        schema_version: 2,
        algorithm: ALGORITHM.into(),
        options,
        engine,
        extractor: crate::diarization::embedding::ExtractorConfig {
            library_index: 0,
            helper: fp.clone(),
            library: fp,
        },
        source,
        known,
        working_hash,
        references,
        targets,
        excluded,
    };
    let spec = JobSpec {
        meeting_id: meeting.clone(),
        kind: JobKind::SpeakerCompletion,
        operation_id: Uuid::new_v4().to_string(),
        input_analysis_id: Some(view.id),
        input_hash: view.content_hash,
        profile_snapshot: json!({"profile":"test"}),
        parameters: json!({"completion":plan}),
        total_units: Some(1),
    };
    drop(tx);
    drop(db);
    (root, s, meeting, spec, plan)
}
fn decision(plan: &Plan, apply: bool) -> Decision {
    Decision {
        needs_review: false,
        lineage: plan.targets[0].lineage.clone(),
        speaker_id: apply.then(|| plan.references[0].speaker_id.clone()),
        body_best: Some(plan.references[0].speaker_id.clone()),
        similarity: Some(if apply { 0.9 } else { 0.6 }),
        runner_up: Some(0.2),
        reason: if apply { "applied" } else { "below_threshold" }.into(),
    }
}
#[test]
fn atomic_publication_preserves_detector_and_round_trip_receipt() {
    let (_root, s, m, spec, plan) = fixture();
    s.check_completion_plan(&spec, &plan).unwrap();
    let before = s.load_current_analysis(&m).unwrap().unwrap();
    let job = s.enqueue_job(spec.clone()).unwrap();
    assert_eq!(s.acquire_local_inference_job().unwrap().unwrap().id, job.id);
    let done = s
        .publish_completion_result(&spec, &plan, &[decision(&plan, true)], &job.id)
        .unwrap();
    assert_eq!(done.state, "succeeded");
    let after = s.load_current_analysis(&m).unwrap().unwrap();
    assert_ne!(before.id, after.id);
    assert_eq!(after.utterances[1].speaker_ids.len(), 1);
    assert_eq!(
        after.utterances[1].effective_text,
        before.utterances[1].effective_text
    );
    assert_eq!(
        s.notification_output(&job.id).unwrap().warnings,
        vec!["1件を補完、0件を見送りました"]
    );
    assert!(s
        .publish_completion_result(&spec, &plan, &[decision(&plan, true)], &job.id)
        .is_err());
    let db = s.connection.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM speaker_segments WHERE analysis_revision_id=?1",
            [after.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    let (spec2, cp) = jobs::read_fixed_input(&db, &job.id).unwrap();
    assert_eq!(spec2, spec);
    assert_eq!(cp.unwrap()["updatedCount"], 1);
}
#[test]
fn zero_changes_pause_resume_invalid_result_and_stale_editor() {
    let (_root, s, m, spec, plan) = fixture();
    let before = s.load_current_analysis(&m).unwrap().unwrap();
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    s.request_job_pause(&job.id, "user").unwrap();
    s.save_job_checkpoint(&job.id, 0, 0, json!({"completionResult":false}))
        .unwrap();
    s.resume_job(&job.id).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let mut invalid = decision(&plan, true);
    invalid.similarity = Some(0.1);
    assert!(s
        .publish_completion_result(&spec, &plan, &[invalid], &job.id)
        .is_err());
    assert_eq!(s.get_job(&job.id).unwrap().state, "running");
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap().id, before.id);
    s.publish_completion_result(&spec, &plan, &[decision(&plan, false)], &job.id)
        .unwrap();
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap().id, before.id);
    assert_eq!(
        s.notification_output(&job.id).unwrap().warnings[0],
        "0件を補完、1件を見送りました"
    );
    s.connection
        .lock()
        .unwrap()
        .execute("UPDATE meetings SET title='edited' WHERE id=?1", [&m])
        .unwrap();
    assert!(s.check_completion_plan(&spec, &plan).is_err());
}
#[test]
fn contexts_clip_at_runs_and_never_cross_missing_audio() {
    let (_root, s, m, _, plan) = fixture();
    let mut db = s.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    let (view, _, _) = working(&tx, &m).unwrap();
    let (refs, targets, _) = evidence(&tx, &view, &plan.known, &plan.source).unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(targets[0].body.start_us, 6_000_000);
    let (_, body_targets, _) = evidence(&tx, &view, &plan.known, &plan.source).unwrap();
    assert_eq!(body_targets[0].body, targets[0].body);
    let (refs, targets, _) = evidence(&tx, &view, &plan.known, &plan.source).unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(targets[0].body.start_us, 6_000_000);
    tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES('missing','track',7000000,7500000,'unknown',0)",[]).unwrap();
    let known = transcription::known::read(&tx, &m, &plan.source).unwrap();
    let (_, targets, excluded) = evidence(&tx, &view, &known, &plan.source).unwrap();
    assert!(targets.is_empty());
    assert_eq!(excluded["missing_audio"], 1);
}
#[test]
fn preferences_are_versioned_frozen_and_validated() {
    let (_root, s, _, spec, plan) = fixture();
    let p = super::super::completion_preferences::CompletionPreferences {
        similarity_threshold_milli: 730,
        margin_threshold_milli: 60,
    };
    let initial = s.speaker_completion_preferences().unwrap();
    assert_eq!(initial.payload.similarity_threshold_milli, 800);
    s.save_speaker_completion_preferences(0, p).unwrap();
    assert!(s.save_speaker_completion_preferences(0, p).is_err());
    assert!(s
        .save_speaker_completion_preferences(
            1,
            super::super::completion_preferences::CompletionPreferences {
                margin_threshold_milli: 1001,
                ..p
            }
        )
        .is_err());
    s.check_completion_plan(&spec, &plan).unwrap();
    assert_eq!(0, 0);
}

#[test]
fn manual_clearing_and_manual_splits_are_excluded_while_text_edits_remain_eligible() {
    let (_root, s, m, _, plan) = fixture();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.create_transcript_draft(&m, &view.id).unwrap();
    let mut p = draft.payload.clone();
    p.utterances.push(super::super::drafts::UtteranceEdit {
        utterance_id: view.utterances[1].id.clone(),
        effective_text: "本文だけ変更".into(),
        speaker_ids: vec![],
    });
    let saved = s.save_transcript_draft(&draft.id, 0, p.clone()).unwrap();
    {
        let mut db = s.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        let (v, _, _) = working(&tx, &m).unwrap();
        assert_eq!(
            evidence(&tx, &v, &plan.known, &plan.source)
                .unwrap()
                .1
                .len(),
            1
        );
    }
    p.utterances.push(super::super::drafts::UtteranceEdit {
        utterance_id: view.utterances[0].id.clone(),
        effective_text: view.utterances[0].effective_text.clone(),
        speaker_ids: vec![],
    });
    p.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    p.utterance_splits
        .push(super::super::draft_splits::UtteranceSplit {
            source_utterance_id: view.utterances[1].id.clone(),
            source_effective_text: "本文だけ変更".into(),
            parts: vec![
                super::super::draft_splits::SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: 6_000_000,
                    end_us: 7_000_000,
                    effective_text: "本文だけ".into(),
                    speaker_ids: vec![],
                },
                super::super::draft_splits::SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: 7_000_000,
                    end_us: 8_000_000,
                    effective_text: "変更".into(),
                    speaker_ids: vec![],
                },
            ],
        });
    s.save_transcript_draft(&draft.id, saved.lock_version, p)
        .unwrap();
    let mut db = s.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    let (v, _, _) = working(&tx, &m).unwrap();
    let (_, targets, excluded) = evidence(&tx, &v, &plan.known, &plan.source).unwrap();
    assert!(targets.is_empty());
    assert_eq!(excluded["manual"], 3);
}
#[test]
fn publication_failure_rolls_back_all_rows_and_receipts() {
    let (_root, s, m, spec, plan) = fixture();
    let before = s.load_current_analysis(&m).unwrap().unwrap();
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    s.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_completion BEFORE UPDATE OF current_analysis_id ON meetings BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(s
        .publish_completion_result(&spec, &plan, &[decision(&plan, true)], &job.id)
        .is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), before);
    assert_eq!(s.get_job(&job.id).unwrap().state, "running");
    assert!(s.fixed_job_input(&job.id).unwrap().1.is_none());
}
#[test]
fn lost_assignment_reply_reads_back_the_same_job_even_while_busy() {
    let (_root, s, m, mut spec, plan) = fixture();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let request = transcript_consolidation::Request {
        meeting_id: m,
        analysis_id: view.id.clone(),
        expected_content_hash: view.content_hash,
        expected_draft: None,
        operation_id: spec.operation_id.clone(),
    };
    spec.parameters["assignmentRequest"] = json!(request);
    let job = s.enqueue_job(spec).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let reply = s.assignment_completion_reply(&request).unwrap().unwrap();
    assert_eq!(reply.completion_job_id.as_deref(), Some(job.id.as_str()));
    assert_eq!(reply.updated_count, 0);
    let mut changed = request.clone();
    changed.expected_content_hash = "b".repeat(64);
    assert!(s.assignment_completion_reply(&changed).is_err());
    s.publish_completion_result(
        &s.fixed_job_input(&job.id).unwrap().0,
        &plan,
        &[decision(&plan, true)],
        &job.id,
    )
    .unwrap();
    assert!(
        s.assignment_completion_reply(&request)
            .unwrap()
            .unwrap()
            .analysis
            .utterances[1]
            .speaker_ids
            .len()
            == 1
    );
}

#[test]
fn saved_editor_version_and_text_changes_invalidate_frozen_completion() {
    let (_root, s, m, _, plan) = fixture();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let before_hash = plan.working_hash;
    let draft = s.create_transcript_draft(&m, &view.id).unwrap();
    let read = || {
        let mut db = s.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        working(&tx, &m).unwrap().2
    };
    assert_ne!(read(), before_hash);
    let empty_hash = read();
    let mut p = draft.payload.clone();
    p.utterances.push(super::super::drafts::UtteranceEdit {
        utterance_id: view.utterances[1].id.clone(),
        effective_text: "変更後".into(),
        speaker_ids: vec![],
    });
    let saved = s
        .save_transcript_draft(&draft.id, draft.lock_version, p)
        .unwrap();
    assert_ne!(read(), empty_hash);
    s.save_transcript_draft(&draft.id, saved.lock_version, draft.payload)
        .unwrap();
    assert_ne!(read(), empty_hash);
}

#[test]
fn pending_manual_speaker_merges_match_only_the_surviving_identity() {
    let (_root, s, m, mut spec, mut plan) = fixture();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.create_transcript_draft(&m, &view.id).unwrap();
    let mut payload = draft.payload;
    payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    payload
        .speaker_merges
        .push(super::super::drafts::SpeakerMerge {
            source_id: view.speakers[0].id.clone(),
            target_id: view.speakers[1].id.clone(),
        });
    s.save_transcript_draft(&draft.id, 0, payload).unwrap();
    {
        let mut db = s.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        let (v, _, hash) = working(&tx, &m).unwrap();
        let (refs, targets, excluded) = evidence(&tx, &v, &plan.known, &plan.source).unwrap();
        assert!(refs.iter().all(|w| w.speaker_id == view.speakers[1].id));
        assert_eq!(refs.len(), 2);
        plan.working_hash = hash;
        plan.references = refs;
        plan.targets = targets;
        plan.excluded = excluded;
    }
    spec.parameters["completion"] = json!(plan);
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    s.publish_completion_result(&spec, &plan, &[decision(&plan, true)], &job.id)
        .unwrap();
    let after = s.load_current_analysis(&m).unwrap().unwrap();
    assert_eq!(after.speakers.len(), 1);
    assert_eq!(
        after.utterances[1].speaker_ids,
        vec![after.speakers[0].id.clone()]
    );
}

#[test]
fn display_progress_survives_reads_without_changing_durable_receipts() {
    let (_root, s, m, spec, plan) = fixture();
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let value = ExecutionProgress {
        phase: "matching".into(),
        completed: 2,
        total: 4,
        phase_completed: 0,
        phase_total: 1,
    };
    let event = s.set_completion_progress(&job.id, value.clone()).unwrap();
    assert_eq!(event.execution_progress, Some(value.clone()));
    assert_eq!((event.completed_units, event.total_units), (0, Some(1)));
    assert_eq!(
        s.get_job(&job.id).unwrap().execution_progress,
        Some(value.clone())
    );
    assert_eq!(
        s.list_jobs(&m).unwrap()[0].execution_progress,
        Some(value.clone())
    );
    assert_eq!(
        s.notification_jobs(&[job.id.clone()]).unwrap()[0]
            .job
            .execution_progress,
        Some(value.clone())
    );
    assert!(s
        .set_completion_progress(
            &job.id,
            ExecutionProgress {
                completed: 1,
                ..value.clone()
            }
        )
        .is_err());
    assert!(s
        .set_completion_progress(
            &job.id,
            ExecutionProgress {
                completed: 4,
                ..value.clone()
            }
        )
        .is_err());
    s.request_job_pause(&job.id, "user").unwrap();
    assert!(s.get_job(&job.id).unwrap().execution_progress.is_none());
    s.save_job_checkpoint(&job.id, 0, 0, json!({"completionResult":false}))
        .unwrap();
    s.clear_completion_progress(&job.id);
    s.resume_job(&job.id).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let reset = ExecutionProgress {
        phase: "references".into(),
        completed: 0,
        phase_completed: 0,
        ..value
    };
    s.set_completion_progress(&job.id, reset).unwrap();
    // No matching decision changes; the completion proof still uses durable 1/1.
    s.publish_completion_result(&spec, &plan, &[decision(&plan, false)], &job.id)
        .unwrap();
    let done = s.get_job(&job.id).unwrap();
    assert!(done.execution_progress.is_none());
    assert_eq!((done.completed_units, done.total_units), (1, Some(1)));
    assert_eq!(
        s.notification_output(&job.id).unwrap().publication_state,
        "completion"
    );
    s.clear_completion_progress(&job.id);
    assert!(s.execution_progress.lock().unwrap().is_empty());
}

#[test]
fn nearest_publication_keeps_review_scores_and_text_edits_but_clears_changed_assignments() {
    let (_root, s, m, spec, plan) = fixture();
    let job = s.enqueue_job(spec.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let mut d = decision(&plan, true);
    d.needs_review = true;
    d.reason = "nearest_below_threshold".into();
    d.similarity = Some(0.53);
    s.publish_completion_result(&spec, &plan, &[d], &job.id)
        .unwrap();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let info = view.utterances[1].speaker_identification.as_ref().unwrap();
    assert!(info.needs_review);
    assert_eq!(info.similarity_milli, 530);
    assert_eq!(info.runner_up_milli, Some(200));
    assert!(s
        .notification_output(&job.id)
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains("1件は低信頼")));
    let mut payload = TranscriptDraftPayload::default();
    payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: view.utterances[1].id.clone(),
            effective_text: "本文のみ修正".into(),
            speaker_ids: view.utterances[1].speaker_ids.clone(),
        });
    let mut projected = view.clone();
    minutes_input::apply_edits(&mut projected, &payload);
    assert_eq!(
        projected.utterances[1].speaker_identification,
        view.utterances[1].speaker_identification
    );
    {
        let mut db = s.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        let copied = minutes_input::copy_snapshot_with_update(
            &tx,
            &projected,
            &payload,
            "会議",
            0,
            None,
            &"c".repeat(64),
            None,
            None,
            None,
        )
        .unwrap();
        let reread =
            super::super::analysis::read_analysis_content(&tx, &m, &copied, "hash").unwrap();
        assert_eq!(
            reread.utterances[1].speaker_identification,
            view.utterances[1].speaker_identification
        );
    }
    payload.utterances[0].speaker_ids.clear();
    minutes_input::apply_edits(&mut projected, &payload);
    assert!(projected.utterances[1].speaker_identification.is_none());
}

#[test]
fn previous_completion_algorithms_are_rejected_without_publication() {
    let (_root, s, _m, mut spec, mut plan) = fixture();
    for old in [
        "missing_turn_context_cosine_v1",
        "missing_turn_body_nearest_v2",
    ] {
        plan.algorithm = old.into();
        spec.parameters["completion"] = serde_json::to_value(&plan).unwrap();
        assert!(s.check_completion_plan(&spec, &plan).is_err());
    }
}

#[cfg(unix)]
#[test]
fn feature_session_reuses_worker_vectors_and_checks_source_on_cache_hits() {
    use crate::speaker_features::Session;
    use std::os::unix::fs::PermissionsExt;
    let (_root, s, meeting, _spec, mut plan) = fixture();
    let helper = s.root().join("fake-embedding");
    // Both old one-shot and resident protocols return the same exact vector.
    let script = r#"#!/usr/bin/python3
import sys,json,os
out=lambda args: dict(schemaVersion=1,engine='sherpa-onnx',engineVersion='1.13.8',sampleRate=16000,firstFrame=int(args[args.index('--first-frame')+1]),endFrame=int(args[args.index('--end-frame')+1]),dimension=2,status='ready',reason=None,vectorBits=['3e800000','bf400000'])
if '--mon-resident-v1' not in sys.argv:
 print(json.dumps(out(sys.argv[1:])))
else:
 for line in sys.stdin:
  r=json.loads(line)
  if r['op']=='shutdown': break
  if r['op']=='infer':
   with open(r['directory']+'/'+r['stdout'],'w') as f: json.dump(out(r['args']),f)
   with open(r['directory']+'/'+r['stderr'],'w') as f: f.write('')
   with open(os.path.dirname(os.path.dirname(os.path.dirname(sys.argv[0])))+'/calls','a') as f: f.write(str(os.getpid())+'\n')
  print(json.dumps(dict(protocol=1,id=r['id'],op='ready' if r['op']=='init' else 'done')),flush=True)
"#;
    std::fs::write(&helper, script).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let proof = Fingerprint::capture(&helper, &|| false).unwrap();
    plan.engine.executable = proof.clone();
    plan.engine.segmentation_model = proof.clone();
    plan.engine.embedding_model = proof.clone();
    plan.engine.runtime_libraries = vec![proof.clone()];
    plan.extractor.helper = proof.clone();
    plan.extractor.library = proof.clone();
    let runtime = crate::local_engine::resident::Snapshot {
        gpu: None,
        protocol: 1,
        engine: "sherpa".into(),
        name: "mon-sherpa-worker".into(),
        bytes: proof.size,
        sha256: proof.sha256.clone(),
    };
    let dir = s.root().join("inference-runtimes").join(&runtime.sha256);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&helper, dir.join(&runtime.name)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let window = crate::voice::Window {
        speaker_id: "a".into(),
        track_id: "track".into(),
        start_us: 0,
        end_us: 3_000_000,
    };
    let context = crate::internal_logs::Context {
        category: "voice_matching".into(),
        meeting_id: Some(meeting.clone()),
        job_id: None,
        operation_id: None,
    };
    let mut legacy = Session::new(
        &s,
        &meeting,
        &plan.engine,
        &plan.extractor,
        &plan.source,
        None,
        context.clone(),
        &|| false,
    )
    .unwrap();
    let expected = legacy
        .extract(&window, &temp.path().join("legacy"), &|| false)
        .unwrap()
        .unwrap();
    drop(legacy);
    let new_session = || {
        Session::new(
            &s,
            &meeting,
            &plan.engine,
            &plan.extractor,
            &plan.source,
            Some(&runtime),
            context.clone(),
            &|| false,
        )
        .unwrap()
    };
    let mut first = new_session();
    assert_eq!(
        first
            .extract(&window, &temp.path().join("first"), &|| false)
            .unwrap(),
        Some(expected.clone())
    );
    let mut other = window.clone();
    other.start_us = 1_000_000;
    first
        .extract(&other, &temp.path().join("second"), &|| false)
        .unwrap();
    first.finish(&|| false).unwrap();
    drop(first);
    let calls = std::fs::read_to_string(s.root().join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 2);
    assert_eq!(calls.lines().next(), calls.lines().nth(1));
    let mut second = new_session();
    let mut renamed = window.clone();
    renamed.speaker_id = "renamed".into();
    assert_eq!(
        second
            .extract(&renamed, &temp.path().join("cache"), &|| false)
            .unwrap(),
        Some(expected)
    );
    assert!(!temp.path().join("cache").exists());
    assert_eq!(
        second
            .extract(&window, &temp.path().join("cancelled"), &|| true)
            .unwrap(),
        None
    );
    assert_eq!(
        std::fs::read_to_string(s.root().join("calls")).unwrap(),
        calls
    );
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_chunks SET end_us=end_us-1000 WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    assert!(second
        .extract(&window, &temp.path().join("changed"), &|| false)
        .is_err());
    assert!(second.finish(&|| false).is_err());
}
