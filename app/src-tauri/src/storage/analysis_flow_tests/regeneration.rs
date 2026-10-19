use super::*;
use crate::storage::analysis_flows::regeneration::Request;

pub(in crate::storage) fn complete_initial(s: &Storage, meeting: &str) {
    s.ensure_analysis_flow(meeting).unwrap();
    advance(s);
    transcribe(s);
    advance(s);
    diarize(s);
    advance(s);
    assert_eq!(s.analysis_flow(meeting).unwrap().unwrap().stage, "review");
}
fn request(s: &Storage, meeting: &str) -> Request {
    Request {
        meeting_id: meeting.into(),
        operation_id: Uuid::new_v4().to_string(),
        expected: s.regeneration_admission(meeting).unwrap(),
        speaker_counts: vec![],
    }
}

#[test]
fn regeneration_explicit_speaker_count_is_frozen_and_replay_cannot_change_it() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let before = s.diarization_preferences().unwrap();
    let track = s.recorded_analysis_input(&m).unwrap().0.tracks[0]
        .id
        .clone();
    let mut r = request(&s, &m);
    r.speaker_counts = vec![crate::diarization::TrackSettings {
        track_id: track,
        num_speakers: Some(5),
    }];
    let flow = s
        .regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        flow.plan.as_ref().unwrap()["speakerCounts"][0]["numSpeakers"],
        5
    );
    assert_eq!(s.regeneration_replay(&r).unwrap().unwrap().id, flow.id);
    r.speaker_counts[0].num_speakers = Some(4);
    assert!(s.regeneration_replay(&r).is_err());
    advance(&s);
    transcribe(&s);
    advance(&s);
    let job = s
        .analysis_flow(&m)
        .unwrap()
        .unwrap()
        .diarization_job
        .unwrap();
    assert_eq!(
        s.fixed_job_input(&job.id).unwrap().0.parameters["diarization"]["tracks"][0]["numSpeakers"],
        5
    );
    diarize(&s);
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "review");
    assert_eq!(s.diarization_preferences().unwrap().payload, before.payload);
    assert_eq!(
        s.diarization_preferences().unwrap().lock_version,
        before.lock_version
    );
    let next = s
        .regenerate_analysis(&request(&s, &m), &crate::test_support::never_cancelled)
        .unwrap();
    assert!(next.plan.as_ref().unwrap().get("speakerCounts").is_none());
}

#[test]
fn regeneration_invalid_speaker_counts_and_foreign_tracks_never_start_a_flow() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    let track = s.recorded_analysis_input(&m).unwrap().0.tracks[0]
        .id
        .clone();
    for values in [
        vec![(track.clone(), Some(0))],
        vec![(track.clone(), Some(101))],
        vec![("foreign".into(), Some(5))],
        vec![(track.clone(), Some(5)), (track.clone(), Some(4))],
    ] {
        let mut r = request(&s, &m);
        r.speaker_counts = values
            .into_iter()
            .map(
                |(track_id, num_speakers)| crate::diarization::TrackSettings {
                    track_id,
                    num_speakers,
                },
            )
            .collect();
        assert!(s
            .regenerate_analysis(&r, &crate::test_support::never_cancelled)
            .is_err());
        assert!(s.analysis_flow(&m).unwrap().is_none());
    }
}

#[test]
fn failed_first_transcription_starts_a_new_global_settings_flow_without_retrying_the_old_job() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    s.ensure_analysis_flow(&m).unwrap();
    advance(&s);
    let failed = s.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(failed.kind, "transcription");
    s.finish_job(&failed.id, 0, "failed", Some("synthetic_failure"))
        .unwrap();
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "attention");
    assert!(s.load_current_analysis(&m).unwrap().is_none());
    let fixed = s.fixed_job_input(&failed.id).unwrap();
    let mut preferences = s.analysis_preferences().unwrap().payload;
    preferences.standard.threads = 2;
    s.save_analysis_preferences(1, preferences).unwrap();
    let r = request(&s, &m);
    assert!(r.expected.target.is_none());
    let next = s
        .regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(next.id, r.operation_id);
    assert_eq!(next.plan.as_ref().unwrap()["whisper"]["threads"], 2);
    assert_eq!(
        s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
            .unwrap()
            .id,
        next.id
    );
    advance(&s);
    let job = s
        .analysis_flow(&m)
        .unwrap()
        .unwrap()
        .transcription_job
        .unwrap();
    assert_ne!(job.id, failed.id);
    assert_eq!(job.attempt, 1);
    assert!(job.retry_of_job_id.is_none());
    transcribe(&s);
    advance(&s);
    diarize(&s);
    advance(&s);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "review");
    assert!(s.load_current_analysis(&m).unwrap().is_some());
    assert_eq!(s.fixed_job_input(&failed.id).unwrap(), fixed);
    assert_eq!(s.get_job(&failed.id).unwrap().state, "failed");
}
fn finish(s: &Storage, meeting: &str) -> AnalysisFlow {
    advance(s);
    transcribe(s);
    advance(s);
    diarize(s);
    advance(s);
    let flow = s.analysis_flow(meeting).unwrap().unwrap();
    assert!(flow.ready_to_publish);
    flow
}
fn edit(s: &Storage, meeting: &str, text: &str) -> super::super::super::drafts::TranscriptDraft {
    let base = s.load_current_analysis(meeting).unwrap().unwrap();
    let draft = s.create_transcript_draft(meeting, &base.id).unwrap();
    let mut payload = draft.payload.clone();
    payload.utterances = vec![super::super::super::drafts::UtteranceEdit {
        utterance_id: base.utterances[0].id.clone(),
        effective_text: text.into(),
        speaker_ids: base.utterances[0].speaker_ids.clone(),
    }];
    s.save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap()
}

#[test]
fn regeneration_uses_start_time_global_models_and_waits_for_edit_barrier() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let saved = edit(&s, &m, "再生成前の人間の修正");
    let mut preferences = s.analysis_preferences().unwrap().payload;
    preferences.standard.threads = 2;
    s.save_analysis_preferences(1, preferences).unwrap();
    let r = request(&s, &m);
    let accepted = s
        .regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(accepted.id, r.operation_id);
    assert_eq!(accepted.plan.as_ref().unwrap()["whisper"]["threads"], 2);
    let plan = accepted.plan.clone();
    let mut preferences = s.analysis_preferences().unwrap().payload;
    preferences.standard.threads = 3;
    s.save_analysis_preferences(2, preferences).unwrap();
    let flow = finish(&s, &m);
    assert_eq!(flow.plan, plan);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), saved);
    assert_eq!(s.ready_analysis_flows().unwrap().len(), 1);
    let published = s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    let published = published.published().unwrap();
    assert_eq!(published.stage, "review");
    assert!(!published.ready_to_publish);
    let new = s.load_current_analysis(&m).unwrap().unwrap();
    assert_ne!(new.id, prior.id);
    assert!(s.load_transcript_draft(&m).unwrap().is_none());
    assert_eq!(s.list_jobs(&m).unwrap().len(), 4);
    assert_eq!(
        s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
            .unwrap()
            .id,
        r.operation_id
    );
    assert_eq!(
        s.publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
            .unwrap()
            .published()
            .unwrap()
            .stage,
        "review"
    );
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), new);
}

#[test]
fn regeneration_later_edits_require_confirmation_and_publish_existing_output() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    edit(&s, &m, "開始前の修正");
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let r = request(&s, &m);
    s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    let later = edit(&s, &m, "処理中に保存した修正");
    let mut payload = later.payload.clone();
    payload
        .speaker_names
        .push(super::super::super::drafts::SpeakerNameEdit {
            speaker_id: prior.speakers[0].id.clone(),
            display_name: Some("人間が変更した話者名".into()),
        });
    payload.utterances[0].speaker_ids.clear();
    let later = s
        .save_transcript_draft(&later.id, later.lock_version, payload)
        .unwrap();
    let flow = finish(&s, &m);
    let pending = s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    let super::super::super::analysis_flows::regeneration::Publication::ConfirmationRequired {
        target,
        meeting_title,
        ..
    } = pending
    else {
        panic!("expected confirmation");
    };
    assert!(!meeting_title.is_empty());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), later);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "diarization");
    assert_eq!(s.ready_analysis_flows().unwrap().len(), 1);
    let jobs = s.list_jobs(&m).unwrap();
    let plan = flow.plan.clone();
    // Cancel/no response retains both the public draft and the generated output,
    // including across restart. Acceptance publishes those exact existing jobs.
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    s.publish_regenerated_flow(&m, &flow.id, flow.lock_version, Some(&target))
        .unwrap()
        .published()
        .unwrap();
    assert_eq!(s.list_jobs(&m).unwrap(), jobs);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().plan, plan);
    assert_eq!(
        s.load_current_analysis(&m).unwrap().unwrap().id,
        flow.diarization_job.unwrap().output_analysis_id.unwrap()
    );
    assert!(s.load_transcript_draft(&m).unwrap().is_none());
}

#[test]
fn regeneration_missing_models_cancel_and_stale_confirmation_leave_public_text_and_flow_intact() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let r = request(&s, &m);
    let flow = s.analysis_flow(&m).unwrap().unwrap();
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    assert!(s.regenerate_analysis(&r, &|| true).is_err());
    let later = edit(&s, &m, "確認後に変更した本文");
    assert!(s
        .regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .is_err());
    let current = request(&s, &m);
    std::fs::remove_file(root.path().join("model.bin")).unwrap();
    assert!(s
        .regenerate_analysis(&current, &crate::test_support::never_cancelled)
        .is_err());
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().id, flow.id);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), later);
    assert_eq!(s.list_jobs(&m).unwrap().len(), 2);
}

#[test]
fn regeneration_operation_identity_replay_and_competing_starts_are_checked() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let r = request(&s, &m);
    let competing = request(&s, &m);
    let first = s
        .regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(s
        .regenerate_analysis(&competing, &crate::test_support::never_cancelled)
        .is_err());
    let mut changed = r.clone();
    changed.expected.target.as_mut().unwrap().content_hash = "0".repeat(64);
    assert!(s.regeneration_replay(&changed).is_err());
    let mut invalid = r.clone();
    invalid.operation_id = "not-a-uuid".into();
    assert!(s
        .regenerate_analysis(&invalid, &crate::test_support::never_cancelled)
        .is_err());
    advance(&s);
    // Read-only replay needs neither model availability nor idle inference.
    std::fs::remove_file(root.path().join("model.bin")).unwrap();
    assert_eq!(s.regeneration_replay(&r).unwrap().unwrap().id, first.id);
    assert_eq!(s.list_jobs(&m).unwrap().len(), 3);
}

#[test]
fn regeneration_restart_preserves_pending_publication_and_wrong_flow_cannot_publish() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let r = request(&s, &m);
    let before = s.load_current_analysis(&m).unwrap().unwrap();
    s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    let flow = finish(&s, &m);
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.ready_analysis_flows().unwrap().len(), 1);
    assert!(s
        .publish_regenerated_flow(&m, &Uuid::new_v4().to_string(), flow.lock_version, None)
        .is_err());
    assert!(s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version + 1, None)
        .is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), before);
    s.publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    assert!(s.ready_analysis_flows().unwrap().is_empty());
}

#[test]
fn regeneration_publication_failure_rolls_back_head_and_draft_then_stops_automatic_retry() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let saved = edit(&s, &m, "上書き失敗でも残す");
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let r = request(&s, &m);
    s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    let flow = finish(&s, &m);
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_regeneration_head BEFORE UPDATE OF current_analysis_id ON meetings BEGIN SELECT RAISE(ABORT,'synthetic publication failure'); END;").unwrap();
    assert!(s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), saved);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage, "attention");
    let db = s.connection.lock().unwrap();
    let state: String = db
        .query_row(
            "SELECT publication_state FROM analysis_revisions WHERE id=?1",
            params![flow.diarization_job.unwrap().output_analysis_id.unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "candidate");
}

#[test]
fn regeneration_failed_worker_and_success_never_change_human_minutes() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = edit(&s, &m, "再生成が失敗しても残す本文");
    let minutes = s
        .save_minutes_markdown(crate::storage::minutes_markdown::SaveMarkdownRequest {
            meeting_id: m.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown: "# 人間の議事録\r\n\n修正をそのまま保持する。\n".into(),
        })
        .unwrap();
    let r = request(&s, &m);
    s.regenerate_analysis(&r, &crate::test_support::never_cancelled)
        .unwrap();
    advance(&s);
    transcribe(&s);
    advance(&s);
    std::fs::write(root.path().join("fail-diarization"), b"fail").unwrap();
    let job = s.acquire_local_inference_job().unwrap().unwrap();
    assert!(
        diarization::process(&s, &job.id, &crate::test_support::never_cancelled, &|_| {}).is_err()
    );
    s.finish_job(&job.id, 0, "failed", Some("synthetic diarization failure"))
        .unwrap();
    advance(&s);
    assert!(!s.analysis_flow(&m).unwrap().unwrap().ready_to_publish);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), draft);
    assert_eq!(s.get_minutes_markdown(&m).unwrap(), Some(minutes.clone()));
    std::fs::remove_file(root.path().join("fail-diarization")).unwrap();
    let next = request(&s, &m);
    s.regenerate_analysis(&next, &crate::test_support::never_cancelled)
        .unwrap();
    let flow = finish(&s, &m);
    s.publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    assert_ne!(s.load_current_analysis(&m).unwrap().unwrap().id, prior.id);
    assert_eq!(s.get_minutes_markdown(&m).unwrap(), Some(minutes.clone()));
    assert_eq!(
        s.list_jobs(&m)
            .unwrap()
            .iter()
            .filter(|j| j.kind == "minutes_generation")
            .count(),
        0
    );
    drop(s);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(reopened.get_minutes_markdown(&m).unwrap(), Some(minutes));
}

#[test]
fn regeneration_stale_replacement_consent_preserves_newer_edits_and_can_be_confirmed_again() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    s.regenerate_analysis(&request(&s, &m), &crate::test_support::never_cancelled)
        .unwrap();
    edit(&s, &m, "確認対象の本文");
    let flow = finish(&s, &m);
    let pending = s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    let crate::storage::analysis_flows::regeneration::Publication::ConfirmationRequired {
        target,
        ..
    } = pending
    else {
        panic!("expected confirmation");
    };
    let newer = edit(&s, &m, "確認後に保存した本文");
    assert!(s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, Some(&target))
        .is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), newer);
    assert!(s.analysis_flow(&m).unwrap().unwrap().replacement_ready);
    let mut wrong_head = target;
    wrong_head.analysis_id = Uuid::new_v4().to_string();
    assert!(s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, Some(&wrong_head))
        .is_err());
    let pending = s
        .publish_regenerated_flow(&m, &flow.id, flow.lock_version, None)
        .unwrap();
    let crate::storage::analysis_flows::regeneration::Publication::ConfirmationRequired {
        target,
        ..
    } = pending
    else {
        panic!("expected confirmation");
    };
    s.publish_regenerated_flow(&m, &flow.id, flow.lock_version, Some(&target))
        .unwrap()
        .published()
        .unwrap();
    assert_ne!(s.load_current_analysis(&m).unwrap().unwrap().id, prior.id);
    assert_eq!(s.list_jobs(&m).unwrap().len(), 4);
}

#[test]
fn regeneration_previously_stopped_replacement_can_publish_without_new_jobs() {
    let root = tempfile::tempdir().unwrap();
    let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    s.regenerate_analysis(&request(&s, &m), &crate::test_support::never_cancelled)
        .unwrap();
    edit(&s, &m, "旧バージョンで保持された修正");
    let flow = finish(&s, &m);
    let stopped = s
        .update_analysis_flow(
            &flow,
            "attention",
            flow.plan.clone(),
            flow.transcription_job.as_ref(),
            flow.diarization_job.as_ref(),
            Some("再生成の開始後に文字起こしが編集されました"),
        )
        .unwrap();
    assert!(stopped.replacement_ready);
    assert!(!stopped.ready_to_publish);
    let pending = s
        .publish_regenerated_flow(&m, &stopped.id, stopped.lock_version, None)
        .unwrap();
    let crate::storage::analysis_flows::regeneration::Publication::ConfirmationRequired {
        target,
        ..
    } = pending
    else {
        panic!("expected confirmation");
    };
    s.publish_regenerated_flow(&m, &stopped.id, stopped.lock_version, Some(&target))
        .unwrap()
        .published()
        .unwrap();
    assert_eq!(s.list_jobs(&m).unwrap().len(), 4);
    assert!(s
        .analysis_flow(&m)
        .unwrap()
        .unwrap()
        .error_message
        .is_none());
}
