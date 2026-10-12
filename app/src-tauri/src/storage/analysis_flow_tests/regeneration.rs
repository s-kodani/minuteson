use super::*;
use crate::storage::analysis_flows::regeneration::Request;

pub(in crate::storage) fn complete_initial(s: &Storage, meeting: &str) {
    s.ensure_analysis_flow(meeting).unwrap();
    advance(s); transcribe(s); advance(s); diarize(s); advance(s);
    assert_eq!(s.analysis_flow(meeting).unwrap().unwrap().stage, "review");
}
fn request(s: &Storage, meeting: &str) -> Request {
    Request { meeting_id: meeting.into(), operation_id: Uuid::new_v4().to_string(), expected: s.regeneration_admission(meeting).unwrap() }
}
fn finish(s: &Storage, meeting: &str) -> AnalysisFlow {
    advance(s); transcribe(s); advance(s); diarize(s); advance(s);
    let flow = s.analysis_flow(meeting).unwrap().unwrap();
    assert!(flow.ready_to_publish);
    flow
}
fn edit(s: &Storage, meeting: &str, text: &str) -> super::super::super::drafts::TranscriptDraft {
    let base = s.load_current_analysis(meeting).unwrap().unwrap();
    let draft = s.create_transcript_draft(meeting, &base.id).unwrap();
    let mut payload = draft.payload.clone();
    payload.utterances = vec![super::super::super::drafts::UtteranceEdit {
        utterance_id: base.utterances[0].id.clone(), effective_text: text.into(), speaker_ids: base.utterances[0].speaker_ids.clone(),
    }];
    s.save_transcript_draft(&draft.id, draft.lock_version, payload).unwrap()
}

#[test]
fn regeneration_uses_start_time_global_models_and_waits_for_edit_barrier() {
    let root = tempfile::tempdir().unwrap(); let (s,m) = fixture(root.path());
    complete_initial(&s,&m); let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let saved = edit(&s,&m,"再生成前の人間の修正");
    let mut preferences = s.analysis_preferences().unwrap().payload;
    preferences.standard.threads = 2; s.save_analysis_preferences(1,preferences).unwrap();
    let r = request(&s,&m); let accepted = s.regenerate_analysis(&r,&||false).unwrap();
    assert_eq!(accepted.id,r.operation_id);
    assert_eq!(accepted.plan.as_ref().unwrap()["whisper"]["threads"],2);
    let plan = accepted.plan.clone();
    let mut preferences = s.analysis_preferences().unwrap().payload;
    preferences.standard.threads = 3; s.save_analysis_preferences(2,preferences).unwrap();
    let flow = finish(&s,&m);
    assert_eq!(flow.plan,plan);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(),saved);
    assert_eq!(s.ready_analysis_flows().unwrap().len(),1);
    let published = s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).unwrap();
    assert_eq!(published.stage,"review"); assert!(!published.ready_to_publish);
    let new = s.load_current_analysis(&m).unwrap().unwrap();
    assert_ne!(new.id,prior.id); assert!(s.load_transcript_draft(&m).unwrap().is_none());
    assert_eq!(s.list_jobs(&m).unwrap().len(),4);
    assert_eq!(s.regenerate_analysis(&r,&||false).unwrap().id,r.operation_id);
    assert_eq!(s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).unwrap().stage,"review");
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),new);
}

#[test]
fn regeneration_later_human_text_and_speaker_edits_win_and_are_not_rebased() {
    let root = tempfile::tempdir().unwrap(); let (s,m) = fixture(root.path()); complete_initial(&s,&m);
    edit(&s,&m,"開始前の修正"); let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let r=request(&s,&m); s.regenerate_analysis(&r,&||false).unwrap();
    let later=edit(&s,&m,"処理中に保存した修正");
    let mut payload = later.payload.clone();
    payload.speaker_names.push(super::super::super::drafts::SpeakerNameEdit {
        speaker_id: prior.speakers[0].id.clone(), display_name: Some("人間が変更した話者名".into()),
    });
    payload.utterances[0].speaker_ids.clear();
    let later = s.save_transcript_draft(&later.id, later.lock_version, payload).unwrap();
    let flow=finish(&s,&m);
    let error=s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).unwrap_err();
    assert!(error.contains("編集されました"),"{error}");
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(),later);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage,"attention");
    assert!(s.ready_analysis_flows().unwrap().is_empty());
    // A new explicit confirmation can replace the newly edited target.
    let next=request(&s,&m); s.regenerate_analysis(&next,&||false).unwrap();
    assert!(s.regenerate_analysis(&r,&||false).is_err());
    let flow=finish(&s,&m); s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).unwrap();
    assert_ne!(s.load_current_analysis(&m).unwrap().unwrap().id,prior.id);
}

#[test]
fn regeneration_missing_models_cancel_and_stale_confirmation_leave_public_text_and_flow_intact() {
    let root=tempfile::tempdir().unwrap(); let(s,m)=fixture(root.path()); complete_initial(&s,&m);
    let r=request(&s,&m); let flow=s.analysis_flow(&m).unwrap().unwrap(); let prior=s.load_current_analysis(&m).unwrap().unwrap();
    assert!(s.regenerate_analysis(&r,&||true).is_err());
    let later=edit(&s,&m,"確認後に変更した本文");
    assert!(s.regenerate_analysis(&r,&||false).is_err());
    let current=request(&s,&m);
    std::fs::remove_file(root.path().join("model.bin")).unwrap();
    assert!(s.regenerate_analysis(&current,&||false).is_err());
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().id,flow.id);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(),later);
    assert_eq!(s.list_jobs(&m).unwrap().len(),2);
}

#[test]
fn regeneration_operation_identity_replay_and_competing_starts_are_checked() {
    let root=tempfile::tempdir().unwrap(); let(s,m)=fixture(root.path()); complete_initial(&s,&m);
    let r=request(&s,&m); let competing=request(&s,&m); let first=s.regenerate_analysis(&r,&||false).unwrap();
    assert!(s.regenerate_analysis(&competing,&||false).is_err());
    let mut changed=r.clone(); changed.expected.target.content_hash="0".repeat(64);
    assert!(s.regeneration_replay(&changed).is_err());
    let mut invalid=r.clone(); invalid.operation_id="not-a-uuid".into(); assert!(s.regenerate_analysis(&invalid,&||false).is_err());
    advance(&s);
    // Read-only replay needs neither model availability nor idle inference.
    std::fs::remove_file(root.path().join("model.bin")).unwrap();
    assert_eq!(s.regeneration_replay(&r).unwrap().unwrap().id,first.id);
    assert_eq!(s.list_jobs(&m).unwrap().len(),3);
}

#[test]
fn regeneration_restart_preserves_pending_publication_and_wrong_flow_cannot_publish() {
    let root=tempfile::tempdir().unwrap(); let(s,m)=fixture(root.path()); complete_initial(&s,&m);
    let r=request(&s,&m); let before=s.load_current_analysis(&m).unwrap().unwrap();
    s.regenerate_analysis(&r,&||false).unwrap(); let flow=finish(&s,&m);
    drop(s); let s=Storage::open(root.path().into()).unwrap();
    assert_eq!(s.ready_analysis_flows().unwrap().len(),1);
    assert!(s.publish_regenerated_flow(&m,&Uuid::new_v4().to_string(),flow.lock_version).is_err());
    assert!(s.publish_regenerated_flow(&m,&flow.id,flow.lock_version+1).is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),before);
    s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).unwrap();
    assert!(s.ready_analysis_flows().unwrap().is_empty());
}

#[test]
fn regeneration_publication_failure_rolls_back_head_and_draft_then_stops_automatic_retry() {
    let root=tempfile::tempdir().unwrap(); let(s,m)=fixture(root.path()); complete_initial(&s,&m);
    let saved=edit(&s,&m,"上書き失敗でも残す"); let prior=s.load_current_analysis(&m).unwrap().unwrap();
    let r=request(&s,&m); s.regenerate_analysis(&r,&||false).unwrap(); let flow=finish(&s,&m);
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_regeneration_head BEFORE UPDATE OF current_analysis_id ON meetings BEGIN SELECT RAISE(ABORT,'synthetic publication failure'); END;").unwrap();
    assert!(s.publish_regenerated_flow(&m,&flow.id,flow.lock_version).is_err());
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(),prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(),saved);
    assert_eq!(s.analysis_flow(&m).unwrap().unwrap().stage,"attention");
    let db=s.connection.lock().unwrap(); let state:String=db.query_row("SELECT publication_state FROM analysis_revisions WHERE id=?1",params![flow.diarization_job.unwrap().output_analysis_id.unwrap()],|r|r.get(0)).unwrap();
    assert_eq!(state,"candidate");
}

#[test]
fn regeneration_failed_worker_and_success_never_change_human_minutes() {
    let root = tempfile::tempdir().unwrap(); let (s, m) = fixture(root.path());
    complete_initial(&s, &m);
    let prior = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = edit(&s, &m, "再生成が失敗しても残す本文");
    let minutes = s.save_minutes_markdown(crate::storage::minutes_markdown::SaveMarkdownRequest {
        meeting_id: m.clone(), operation_id: Uuid::new_v4().to_string(), expected: None,
        markdown: "# 人間の議事録\r\n\n修正をそのまま保持する。\n".into(),
    }).unwrap();
    let r = request(&s, &m); s.regenerate_analysis(&r, &|| false).unwrap();
    advance(&s); transcribe(&s); advance(&s);
    std::fs::write(root.path().join("fail-diarization"), b"fail").unwrap();
    let job = s.acquire_local_inference_job().unwrap().unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {}).is_err());
    s.finish_job(&job.id, 0, "failed", Some("synthetic diarization failure")).unwrap();
    advance(&s);
    assert!(!s.analysis_flow(&m).unwrap().unwrap().ready_to_publish);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), prior);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), draft);
    assert_eq!(s.get_minutes_markdown(&m).unwrap(), Some(minutes.clone()));
    std::fs::remove_file(root.path().join("fail-diarization")).unwrap();
    let next = request(&s, &m); s.regenerate_analysis(&next, &|| false).unwrap();
    let flow = finish(&s, &m);
    s.publish_regenerated_flow(&m, &flow.id, flow.lock_version).unwrap();
    assert_ne!(s.load_current_analysis(&m).unwrap().unwrap().id, prior.id);
    assert_eq!(s.get_minutes_markdown(&m).unwrap(), Some(minutes.clone()));
    assert_eq!(s.list_jobs(&m).unwrap().iter().filter(|j| j.kind == "minutes_generation").count(), 0);
    drop(s);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(reopened.get_minutes_markdown(&m).unwrap(), Some(minutes));
}
