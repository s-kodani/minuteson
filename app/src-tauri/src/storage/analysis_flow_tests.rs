use super::*;
use crate::{analysis_flow, diarization, transcription};
use std::os::unix::fs::PermissionsExt;

pub(in crate::storage) fn fixture(root: &Path) -> (Storage, String) {
    let (s, m, w) = super::super::transcription::tests::fixture(root, 4);
    let mut p = AnalysisPreferences::default();
    p.standard.executable_path = w.executable.path.to_str().unwrap().into();
    p.standard.model_path = w.model.path.to_str().unwrap().into();
    p.standard.threads = 1;
    s.save_analysis_preferences(0, p).unwrap();
    let cli = root.join("sherpa-cli");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nif test -f '{}'; then exit 42; fi\nprintf '0.000 -- 4.000 speaker_00\\n'\n",
            root.join("fail-diarization").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let seg = root.join("seg.onnx");
    let emb = root.join("emb.onnx");
    std::fs::write(&seg, b"segmentation").unwrap();
    std::fs::write(&emb, b"embedding").unwrap();
    let proof = |p: &Path| crate::local_engine::Fingerprint::capture(p, &|| false).unwrap();
    let engine = crate::diarization::sherpa::SherpaConfig {
        executable: proof(&cli),
        segmentation_model: proof(&seg),
        embedding_model: proof(&emb),
        runtime_libraries: vec![],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    };
    let output = format!(
        "{}\nStarted\n0.000 -- 4.000 speaker_00\n",
        engine.expected_header(None)
    );
    let quoted = output.replace('\'', "'\\''");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nif test -f '{}'; then exit 42; fi\nprintf '%s' '{quoted}'\n",
            root.join("fail-diarization").display()
        ),
    )
    .unwrap();
    let mut p = DiarizationPreferences::default();
    p.standard.executable_path = cli.to_str().unwrap().into();
    p.standard.segmentation_model_path = seg.to_str().unwrap().into();
    p.standard.embedding_model_path = emb.to_str().unwrap().into();
    p.standard.threads = 1;
    s.save_diarization_preferences(0, p).unwrap();
    (s, m)
}
fn advance(s: &Storage) {
    analysis_flow::advance(s, &|| false, &|_| {}).unwrap();
}
fn transcribe(s: &Storage) -> String {
    let job = s.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(job.kind, "transcription");
    transcription::process(s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap()
}
fn diarize(s: &Storage) -> String {
    let job = s.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(job.kind, "diarization");
    diarization::process(s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap()
}

#[test]
fn imported_high_accuracy_selection_survives_ai_wait_restart_and_freezes_vocabulary_at_enqueue() {
    let root = tempfile::tempdir().unwrap();
    let (s, old) = fixture(root.path());
    let options = crate::audio_import::ImportOptions {
        profile: "high_accuracy".into(),
        meeting_vocabulary: vec![VocabularyEntry {
            id: Uuid::new_v4().to_string(),
            term: "点検日".into(),
            reading: Some("てんけんび".into()),
            note: None,
            enabled: true,
        }],
        expected_global_version: Some(0),
    };
    let (import, _, _) =
        super::super::imports::tests::pending_configured(&s, "high-import", &options);
    s.finalize_audio_import(&import.id).unwrap();
    let m = import.meeting_id;
    advance(&s); // Only standard is configured; high-accuracy audio is still saved.
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "configuration");
    assert_eq!(flow.profile, "high_accuracy");
    assert!(s.list_jobs(&m).unwrap().is_empty());
    assert_eq!(s.get_meeting(&m).unwrap().unwrap().lifecycle, "imported");
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let mut w = s.analysis_preferences().unwrap().payload;
    w.high_accuracy = w.standard.clone();
    w.high_accuracy.threads = 2;
    s.save_analysis_preferences(1, w).unwrap();
    let mut d = s.diarization_preferences().unwrap().payload;
    d.high_accuracy = d.standard.clone();
    s.save_diarization_preferences(1, d).unwrap();
    // A common term added after import confirmation is fixed at actual enqueue.
    let common = VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: "会議".into(),
        reading: None,
        note: None,
        enabled: true,
    };
    s.save_vocabulary_book(None, 0, vec![common]).unwrap();
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    analysis_flow::continue_flow(&s, &m, flow.lock_version).unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.profile, "high_accuracy");
    assert_eq!(flow.plan.as_ref().unwrap()["profile"], "high_accuracy");
    assert_eq!(flow.plan.as_ref().unwrap()["whisper"]["threads"], 2);
    let text = transcribe(&s);
    let proof = |id: &str| -> (serde_json::Value, serde_json::Value) {
        let (p,c):(String,String) = s.connection.lock().unwrap().query_row("SELECT profile_snapshot_json,config_json FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",params![id,m],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        (
            serde_json::from_str(&p).unwrap(),
            serde_json::from_str(&c).unwrap(),
        )
    };
    let (profile, config) = proof(&text);
    assert_eq!(profile["profile"], "high_accuracy");
    let fixed = config["vocabulary"].clone();
    assert_eq!(fixed["stamp"]["globalVersion"], 1);
    assert_eq!(fixed["stamp"]["meetingVersion"], 1);
    assert_eq!(fixed["entries"].as_array().unwrap().len(), 2);
    assert!(fixed["actualPrompt"].as_str().unwrap().contains("点検日"));
    let book = s.vocabulary_book(Some(&m)).unwrap();
    let mut changed = book.payload;
    changed[0].term = "変更後".into();
    s.save_vocabulary_book(Some(&m), book.lock_version, changed)
        .unwrap();
    advance(&s);
    let final_id = diarize(&s);
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "review");
    assert_eq!(proof(&text).1["vocabulary"], fixed);
    assert_eq!(proof(&final_id).0["profile"], "high_accuracy");
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    assert!(s.list_jobs(&old).unwrap().is_empty());
}

#[test]
fn imported_audio_hands_off_candidates_without_self_labels_and_freezes_explicit_minutes_input() {
    let root = tempfile::tempdir().unwrap();
    let (s, old_meeting) = fixture(root.path());
    let before = s.get_meeting(&old_meeting).unwrap().unwrap();
    let (import, _, _) = super::super::imports::tests::pending(&s, "imported-flow");
    s.finalize_audio_import(&import.id).unwrap();
    let m = import.meeting_id;
    assert_eq!(s.get_meeting(&m).unwrap().unwrap().lifecycle, "imported");
    advance(&s);
    let text = transcribe(&s);
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    advance(&s);
    let final_id = diarize(&s);
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "review");
    assert_eq!(
        flow.transcription_job.unwrap().output_analysis_id,
        Some(text)
    );
    let candidate = s.load_visible_analysis(&m, &final_id).unwrap().unwrap();
    assert_eq!(candidate.utterances[0].effective_text, "点検日です");
    assert_eq!(candidate.speakers.len(), 1);
    assert_eq!(candidate.speakers[0].label, "取り込み音声 話者1");
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    assert_eq!(s.list_jobs(&m).unwrap().len(), 2); // No automatic minutes job.
    let state = s.analysis_versions(&m).unwrap();
    s.adopt_analysis(AdoptRequest {
        meeting_id: m.clone(),
        revision_id: final_id.clone(),
        expected_content_hash: candidate.content_hash.clone(),
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: None,
        confirm_replacing_previous: false,
    })
    .unwrap();
    let current = s.get_meeting(&m).unwrap().unwrap();
    let job = s
        .enqueue_minutes_job(
            super::super::minutes_input::MinutesJobRequest {
                meeting_id: m.clone(),
                operation_id: "imported-minutes".into(),
                expected_analysis_id: final_id.clone(),
                expected_content_hash: candidate.content_hash,
                expected_meeting_lock_version: current.lock_version,
                expected_draft: None,
            },
            serde_json::json!({"engine":"protocol-fixture"}),
            serde_json::json!({}),
            Some(1),
        )
        .unwrap();
    let fixed = s.load_minutes_input(&job.id).unwrap();
    assert_ne!(fixed.analysis.id, final_id); // A private immutable input revision.
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap().id, final_id);
    assert_eq!(fixed.analysis.utterances[0].effective_text, "点検日です");
    assert_eq!(fixed.analysis.speakers[0].label, "取り込み音声 話者1");
    assert_eq!(s.get_meeting(&old_meeting).unwrap().unwrap(), before);
    assert!(s.list_jobs(&old_meeting).unwrap().is_empty());
}

#[test]
fn automatic_flow_hands_off_unadopted_candidate_freezes_conditions_and_requires_explicit_adoption()
{
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let first = s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "transcription");
    let plan = flow.plan.clone();
    let text = transcribe(&s);
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    // Later settings must not change either engine in this flow.
    let mut prefs = s.diarization_preferences().unwrap().payload;
    prefs.standard.threads = 3;
    s.save_diarization_preferences(1, prefs).unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "diarization", "{:?}", flow.error_message);
    assert_eq!(flow.plan, plan);
    let fixed = s
        .load_diarization_input(&flow.diarization_job.as_ref().unwrap().id)
        .unwrap();
    assert_eq!(fixed.parent_id, text);
    assert!(fixed.manual_utterances.is_empty());
    let final_id = diarize(&s);
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "review");
    assert_eq!(s.ensure_analysis_flow(&m).unwrap().id, first.id);
    advance(&s);
    assert_eq!(s.list_jobs(&m).unwrap().len(), 2);
    let view = s.load_visible_analysis(&m, &final_id).unwrap().unwrap();
    assert_eq!(view.utterances[0].effective_text, "点検日です");
    assert_eq!(view.utterances[0].speaker_ids.len(), 1);
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    let state = s.analysis_versions(&m).unwrap();
    s.adopt_analysis(AdoptRequest {
        meeting_id: m.clone(),
        revision_id: final_id.clone(),
        expected_content_hash: view.content_hash,
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: None,
        confirm_replacing_previous: false,
    })
    .unwrap();
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap().id, final_id);
}

#[test]
fn missing_models_preserve_recording_and_stale_continue_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
    let first = s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "configuration");
    assert!(flow.plan.is_none());
    assert!(flow.error_message.is_some());
    assert!(s.list_jobs(&m).unwrap().is_empty());
    assert_eq!(s.get_meeting(&m).unwrap().unwrap().lifecycle, "recorded");
    assert!(s.recorded_audio_plan(&m).is_ok());
    assert!(analysis_flow::continue_flow(&s, &m, first.lock_version).is_err());
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    analysis_flow::continue_flow(&s, &m, flow.lock_version).unwrap();
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "configuration");
}

#[test]
fn failed_diarization_retries_only_that_stage_and_keeps_successful_text() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let text = transcribe(&s);
    advance(&s);
    std::fs::write(root.path().join("fail-diarization"), b"fail").unwrap();
    let job = s.acquire_local_inference_job().unwrap().unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {}).is_err());
    let job = s
        .finish_job(&job.id, 0, "failed", Some("diarization_failed"))
        .unwrap();
    advance(&s);
    assert_eq!(
        s.analysis_flow(&m)
            .unwrap()
            .unwrap()
            .transcription_job
            .unwrap()
            .output_analysis_id
            .as_deref(),
        Some(text.as_str())
    );
    assert_eq!(
        s.load_visible_analysis(&m, &text)
            .unwrap()
            .unwrap()
            .utterances[0]
            .effective_text,
        "点検日です"
    );
    std::fs::remove_file(root.path().join("fail-diarization")).unwrap();
    let next = diarization::RetryRequest {
        meeting_id: m.clone(),
        job_id: job.id,
        operation_id: "user-retry".into(),
        expected_spec_hash: job.spec_hash,
    }
    .enqueue(&s)
    .unwrap();
    advance(&s);
    assert_eq!(
        s.analysis_flow(&m)
            .unwrap()
            .unwrap()
            .diarization_job
            .unwrap()
            .id,
        next.id
    );
    diarize(&s);
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "review");
    assert_eq!(
        s.list_jobs(&m)
            .unwrap()
            .iter()
            .filter(|j| j.kind == "transcription")
            .count(),
        1
    );
}

#[test]
fn model_drift_blocks_handoff_without_losing_text_and_same_plan_can_be_retried() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let text = transcribe(&s);
    std::fs::write(root.path().join("seg.onnx"), b"different model").unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.diarization_job.is_none());
    assert!(s.load_visible_analysis(&m, &text).unwrap().is_some());
    std::fs::write(root.path().join("seg.onnx"), b"segmentation").unwrap();
    analysis_flow::continue_flow(&s, &m, flow.lock_version).unwrap();
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "diarization");
}

#[test]
fn cancelled_coordinator_does_not_start_and_reopen_keeps_interrupted_job_for_explicit_resume() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    analysis_flow::advance(&s, &|| true, &|_| {}).unwrap();
    assert!(s.list_jobs(&m).unwrap().is_empty());
    advance(&s);
    let claimed = s.acquire_local_inference_job().unwrap().unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    advance(&s);
    assert_eq!(
        s.analysis_flow(&m)
            .unwrap()
            .unwrap()
            .transcription_job
            .unwrap()
            .state,
        "interrupted"
    );
    assert_eq!(s.list_jobs(&m).unwrap().len(), 1);
    s.resume_job(&claimed.id).unwrap();
    transcribe(&s);
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "diarization");
}

#[test]
fn flow_plan_is_immutable_hashed_and_job_scope_is_guarded() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert!(s
        .update_analysis_flow(
            &flow,
            "pending",
            Some(serde_json::json!({})),
            None,
            None,
            None
        )
        .is_err());
    let other = s.create_meeting("other").unwrap();
    assert!(s.ensure_analysis_flow(&other.id).is_err());
    let mut wrong = flow.transcription_job.clone().unwrap();
    wrong.meeting_id = other.id;
    assert!(s
        .update_analysis_flow(
            &flow,
            "transcription",
            flow.plan.clone(),
            Some(&wrong),
            None,
            None
        )
        .is_err());
    let db = s.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE analysis_flows SET diarization_job_id=transcription_job_id",
            []
        )
        .is_err());
    db.execute_batch("DROP TRIGGER analysis_flow_plan_fixed; UPDATE analysis_flows SET plan_hash=printf('%064d',0)").unwrap();
    drop(db);
    assert!(s.analysis_flow(&m).is_err());
    assert!(analysis_flow::advance(&s, &|| false, &|_| {}).is_err());
}

#[test]
fn handoff_and_adoption_protect_existing_current_and_saved_edits() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let w = s.analysis_preferences().unwrap().payload.standard;
    let baseline = transcription::StartRequest {
        meeting_id: m.clone(),
        operation_id: "baseline".into(),
        executable_path: w.executable_path.into(),
        model_path: w.model_path.into(),
        vad_model_path: None,
        threads: 1,
        use_gpu: false,
        profile: "standard".into(),
        prompt: String::new(),
        expected_vocabulary: None,
    }
    .enqueue(&s)
    .unwrap();
    let prior = transcribe(&s);
    assert_eq!(s.get_job(&baseline.id).unwrap().state, "succeeded");
    s.commit_analysis_candidate(&m, &prior, 0, false).unwrap();
    let original = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.create_transcript_draft(&m, &prior).unwrap();
    let mut p = draft.payload.clone();
    p.utterances.push(super::super::drafts::UtteranceEdit {
        utterance_id: original.utterances[0].id.clone(),
        effective_text: "大切な手動修正".into(),
        speaker_ids: vec![],
    });
    let saved = s.save_transcript_draft(&draft.id, 0, p).unwrap();
    s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    transcribe(&s);
    advance(&s);
    let candidate = diarize(&s);
    advance(&s);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), original);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), saved);
    let view = s.load_visible_analysis(&m, &candidate).unwrap().unwrap();
    let state = s.analysis_versions(&m).unwrap();
    let request = |stamp| AdoptRequest {
        meeting_id: m.clone(),
        revision_id: candidate.clone(),
        expected_content_hash: view.content_hash.clone(),
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: stamp,
        confirm_replacing_previous: false,
    };
    assert!(s
        .adopt_analysis(request(Some(DraftStamp {
            id: saved.id.clone(),
            lock_version: 0
        })))
        .is_err());
    s.adopt_analysis(request(state.draft)).unwrap();
    assert_eq!(
        s.visible_analysis_draft(&m, &prior).unwrap().unwrap(),
        saved
    );
}

#[test]
fn job_attach_failure_retains_durable_job_and_recovery_does_not_enqueue_a_duplicate() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_flow_attach BEFORE UPDATE ON analysis_flows WHEN NEW.transcription_job_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'attach failed'); END;").unwrap();
    advance(&s);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    assert_eq!(flow.stage, "attention");
    assert!(flow.transcription_job.is_none());
    let jobs = s.list_jobs(&m).unwrap();
    assert_eq!(jobs.len(), 1);
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_flow_attach")
        .unwrap();
    analysis_flow::continue_flow(&s, &m, flow.lock_version).unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    advance(&s);
    assert_eq!(
        s.analysis_flow(&m)
            .unwrap()
            .unwrap()
            .transcription_job
            .unwrap()
            .id,
        jobs[0].id
    );
    assert_eq!(s.list_jobs(&m).unwrap().len(), 1);
}

#[test]
fn migration_from_v17_preserves_data_jobs_and_does_not_autostart_historical_recordings() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let prior = s.get_meeting(&m).unwrap().unwrap();
    let preferences = s.analysis_preferences().unwrap();
    {
        let db = s.connection.lock().unwrap();
        super::imports::remove_import_schema_for_test(&db);
        db.execute_batch("DROP TABLE analysis_flows; DROP TRIGGER input_snapshot_valid; DROP TRIGGER analysis_parent_committed; DELETE FROM schema_migrations WHERE version=18;").unwrap();
        let old = MIGRATION_13
            .split_once("CREATE TRIGGER input_snapshot_valid")
            .unwrap()
            .1
            .split_once("CREATE TRIGGER input_snapshot_fixed_update")
            .unwrap()
            .0;
        db.execute_batch(&format!("CREATE TRIGGER input_snapshot_valid{old}"))
            .unwrap();
        let old = MIGRATION_3
            .split_once("CREATE TRIGGER analysis_parent_committed")
            .unwrap()
            .1
            .split_once("CREATE TRIGGER minutes_parent_hash")
            .unwrap()
            .0;
        db.execute_batch(&format!("CREATE TRIGGER analysis_parent_committed{old}"))
            .unwrap();
    }
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.get_meeting(&m).unwrap().unwrap(), prior);
    assert_eq!(
        s.analysis_preferences().unwrap().payload,
        preferences.payload
    );
    assert!(s.analysis_flow(&m).unwrap().is_none());
    advance(&s);
    assert!(s.list_jobs(&m).unwrap().is_empty());
    let db = s.connection.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        20
    );
    assert!(db
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
}

#[test]
fn unrelated_candidate_cannot_be_a_fixed_input_without_a_flow_and_original_checks_remain() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let w = s.analysis_preferences().unwrap().payload.standard;
    let job = transcription::StartRequest {
        meeting_id: m.clone(),
        operation_id: "standalone".into(),
        executable_path: w.executable_path.into(),
        model_path: w.model_path.into(),
        vad_model_path: None,
        threads: 1,
        use_gpu: false,
        profile: "standard".into(),
        prompt: String::new(),
        expected_vocabulary: None,
    }
    .enqueue(&s)
    .unwrap();
    let id = transcribe(&s);
    let source = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    let request = MinutesJobRequest {
        meeting_id: m.clone(),
        operation_id: "unrelated".into(),
        expected_analysis_id: id,
        expected_content_hash: source.content_hash,
        expected_meeting_lock_version: 0,
        expected_draft: None,
    };
    assert!(s
        .enqueue_diarization_source(request, serde_json::json!({}), Some(&job.id), |_| Ok((
            serde_json::json!({}),
            1
        )))
        .is_err());
    assert_eq!(s.list_jobs(&m).unwrap().len(), 1);
    assert!(s.load_current_analysis(&m).unwrap().is_none());
}
