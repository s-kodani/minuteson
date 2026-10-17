use super::*;
use crate::storage::voice::tests::{enqueue, enqueue_selected, fixture};

fn assign_manual(s: &Storage, m: &str, merge: bool) -> String {
    let view = s.load_current_analysis(m).unwrap().unwrap();
    let draft = s.create_transcript_draft(m, &view.id).unwrap();
    let mut payload = draft.payload;
    let manual = format!("manual:{}", Uuid::new_v4());
    payload.schema_version = 2;
    payload.manual_speakers.push(drafts::ManualSpeaker {
        id: manual.clone(),
        label: "手動話者".into(),
    });
    if merge {
        payload.speaker_merges.push(drafts::SpeakerMerge {
            source_id: view.speakers[0].id.clone(),
            target_id: manual.clone(),
        });
    } else {
        payload.utterances.push(drafts::UtteranceEdit {
            utterance_id: view.utterances[0].id.clone(),
            effective_text: view.utterances[0].effective_text.clone(),
            speaker_ids: vec![manual.clone()],
        });
    }
    s.save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    manual
}

#[test]
fn manual_assignment_and_merge_are_registration_candidates_but_not_matching_windows() {
    for merge in [false, true] {
        let (_root, s, m, _) = fixture();
        let manual = assign_manual(&s, &m, merge);
        let c = s.voice_context(&m).unwrap();
        assert!(c.windows.is_empty());
        assert_eq!(c.registration_candidates.len(), 1);
        assert_eq!(c.registration_candidates[0].window.speaker_id, manual);
        assert_eq!(c.registration_candidates[0].text, "登録用の発言");
    }
}

#[test]
fn splitting_a_second_into_two_short_utterances_does_not_join_registration_candidates() {
    let (_root, s, m, speaker) = fixture();
    let view = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.create_transcript_draft(&m, &view.id).unwrap();
    let mut payload = draft.payload;
    payload.schema_version = 3;
    payload
        .utterance_splits
        .push(crate::storage::draft_splits::UtteranceSplit {
            source_utterance_id: view.utterances[0].id.clone(),
            source_effective_text: view.utterances[0].effective_text.clone(),
            parts: vec![
                crate::storage::draft_splits::SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: 0,
                    end_us: 500_000,
                    effective_text: "登録用の".into(),
                    speaker_ids: vec![speaker.clone()],
                },
                crate::storage::draft_splits::SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: 500_000,
                    end_us: 1_000_000,
                    effective_text: "発言".into(),
                    speaker_ids: vec![speaker],
                },
            ],
        });
    s.save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let c = s.voice_context(&m).unwrap();
    assert_eq!(c.windows.len(), 1); // v1 matching policy remains unchanged.
    assert!(c.registration_candidates.is_empty());
}

#[test]
fn enrollment_respects_duration_coverage_gaps_unknown_and_overlapping_audio() {
    let (_root, s, m, _) = fixture();
    // Physical segments belong to an uncommitted candidate. Never weaken the
    // immutable-current-analysis triggers merely to create a test interval.
    let id = s
        .create_analysis_candidate(
            &m,
            AnalysisInput {
                audio_manifest_hash: s.recorded_analysis_input(&m).unwrap().0.manifest_hash,
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![SpeakerInput {
                    key: "source".into(),
                    label: "source".into(),
                }],
                segments: vec![SpeakerSegmentInput {
                    track_id: "track".into(),
                    speaker_key: Some("source".into()),
                    start_us: 0,
                    end_us: 1_000_000,
                    overlap_group: None,
                    confidence: None,
                }],
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    speaker_keys: vec!["source".into()],
                    start_us: 0,
                    end_us: 1_000_000,
                    text: "候補".into(),
                    confidence: None,
                }],
            },
        )
        .unwrap();
    let mut view = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    let speaker = view.speakers[0].id.clone();
    let mut db = s.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(candidates(&tx, &view).unwrap().len(), 1); // inclusive 1s
    view.utterances[0].end_us = 999_999;
    assert!(candidates(&tx, &view).unwrap().is_empty());
    view.utterances[0].end_us = 40_000_000;
    tx.execute(
        "UPDATE speaker_segments SET end_us=40000000 WHERE analysis_revision_id=?1",
        [&view.id],
    )
    .unwrap();
    tx.execute(
        "UPDATE audio_chunks SET end_us=40000000 WHERE id='chunk-1'",
        [],
    )
    .unwrap();
    let c = candidates(&tx, &view).unwrap();
    assert_eq!(
        (c[0].window.start_us, c[0].window.end_us),
        (5_000_000, 35_000_000)
    );
    tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES('gap','track',10000000,20000000,'missing')", []).unwrap();
    let c = candidates(&tx, &view).unwrap();
    assert_eq!(
        c.iter()
            .map(|c| (c.window.start_us, c.window.end_us))
            .collect::<Vec<_>>(),
        vec![(0, 10_000_000), (20_000_000, 40_000_000)]
    );
    tx.execute("UPDATE audio_chunks SET state='missing'", [])
        .unwrap();
    assert!(candidates(&tx, &view).unwrap().is_empty());
    tx.execute("UPDATE audio_chunks SET state='ready'", [])
        .unwrap();
    tx.execute("UPDATE file_assets SET state='corrupt'", [])
        .unwrap();
    assert!(candidates(&tx, &view).unwrap().is_empty());
    tx.execute("UPDATE file_assets SET state='ready'", [])
        .unwrap();
    tx.execute(
        "UPDATE speaker_segments SET speaker_id=NULL WHERE analysis_revision_id=?1",
        [&view.id],
    )
    .unwrap();
    assert!(candidates(&tx, &view).unwrap().is_empty());
    tx.execute(
        "UPDATE speaker_segments SET speaker_id=?1 WHERE analysis_revision_id=?2",
        params![speaker, view.id],
    )
    .unwrap();
    let second = format!("manual:{}", Uuid::new_v4());
    tx.execute("INSERT INTO speakers(id,analysis_revision_id,lineage_id,label,input_scope,identification_state,created_at) VALUES(?1,?2,?1,'second','imported','manual',0)", params![second,view.id]).unwrap();
    tx.execute("INSERT INTO speaker_segments(id,meeting_id,analysis_revision_id,track_id,speaker_id,start_us,end_us) VALUES('overlap',?1,?2,'track',?3,0,40000000)", params![m,view.id,second]).unwrap();
    assert!(candidates(&tx, &view).unwrap().is_empty());
    tx.execute("DELETE FROM speaker_segments WHERE id='overlap'", [])
        .unwrap();
    view.utterances[0].speaker_ids.push(second);
    assert!(candidates(&tx, &view).unwrap().is_empty());
    view.utterances[0].speaker_ids.clear();
    assert!(candidates(&tx, &view).unwrap().is_empty());
}

#[test]
fn selections_reject_empty_excess_duplicate_wrong_speaker_and_stale_boundaries() {
    let (_root, s, m, speaker) = fixture();
    let c = s.voice_context(&m).unwrap().registration_candidates;
    let w = c[0].window.clone();
    assert!(validate_selection(&c, &speaker, std::slice::from_ref(&w)).is_ok());
    for selected in [
        vec![],
        vec![w.clone(), w.clone()],
        vec![w.clone(), w.clone(), w.clone()],
    ] {
        assert!(validate_selection(&c, &speaker, &selected).is_err());
    }
    assert!(validate_selection(&c, "another", std::slice::from_ref(&w)).is_err());
    let mut wrong = w;
    wrong.end_us -= 1;
    assert!(validate_selection(&c, &speaker, &[wrong]).is_err());
}

fn pause_and_resume(s: &Storage, job: &JobSnapshot, spec: &JobSpec) {
    s.request_job_pause(&job.id, "user").unwrap();
    assert_eq!(
        s.save_job_checkpoint(&job.id, 0, 0, serde_json::json!({"voiceResult":false}))
            .unwrap()
            .state,
        "paused"
    );
    assert_eq!(s.resume_job(&job.id).unwrap().state, "queued");
    assert_eq!(s.acquire_local_inference_job().unwrap().unwrap().id, job.id);
    assert_eq!(s.fixed_job_input(&job.id).unwrap().0, *spec);
}

#[test]
fn selected_manual_window_is_fixed_published_and_rejected_after_a_saved_edit() {
    let (_root, s, m, _) = fixture();
    let manual = assign_manual(&s, &m, false);
    let c = s.voice_context(&m).unwrap();
    let selected = vec![c.registration_candidates[0].window.clone()];
    let (job, spec, plan) = enqueue_selected(&s, &m, &manual, true, Some(selected.clone()));
    assert_eq!(plan.schema_version, 2);
    assert_eq!(plan.windows, selected);
    s.check_voice_plan(&spec, &plan).unwrap();
    // Even read-back of a lost acceptance reply must retain the exact selection.
    let request: Start = serde_json::from_value(spec.parameters["voiceRequest"].clone()).unwrap();
    assert_eq!(s.start_voice(request.clone()).unwrap().id, job.id);
    let mut changed = request;
    changed.selected_windows = Some(vec![]);
    assert!(s.start_voice(changed).is_err());
    pause_and_resume(&s, &job, &spec);
    s.publish_voice_result(
        &spec,
        &plan,
        vec![(selected[0].clone(), vec![1., 0.])],
        &job.id,
    )
    .unwrap();
    let person = &s.voice_people().unwrap()[0];
    assert_eq!(person.samples.len(), 1);
    assert!(s.voice_context(&m).unwrap().matches.is_empty());
    assert!(s
        .load_transcript_draft(&m)
        .unwrap()
        .unwrap()
        .payload
        .speaker_names
        .iter()
        .any(|n| n.speaker_id == manual && n.display_name.as_deref() == Some("田中")));
    assert_eq!(
        (person.samples[0].start_us, person.samples[0].end_us),
        (selected[0].start_us, selected[0].end_us)
    );
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let mut payload = draft.payload;
    payload.utterances[0].speaker_ids.clear();
    s.save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    assert!(s.check_voice_plan(&spec, &plan).is_err());
}

#[test]
fn legacy_request_json_and_version_one_plan_remain_compatible() {
    let (_root, s, m, speaker) = fixture();
    let (job, spec, plan) = enqueue(&s, &m, &speaker, true);
    assert_eq!(plan.schema_version, 1);
    assert!(spec.parameters["voiceRequest"]
        .get("selectedWindows")
        .is_none());
    let request: Start = serde_json::from_value(spec.parameters["voiceRequest"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        spec.parameters["voiceRequest"]
    );
    assert_eq!(s.start_voice(request).unwrap().id, job.id);
    pause_and_resume(&s, &job, &spec);
    s.check_voice_plan(&spec, &plan).unwrap();
    s.publish_voice_result(
        &spec,
        &plan,
        vec![(plan.windows[0].clone(), vec![1., 0.])],
        &job.id,
    )
    .unwrap();
}

#[test]
fn a_materialized_corrected_speaker_registers_and_keeps_a_confirmed_match() {
    let (_root, s, m, _) = fixture();
    let id = s
        .create_analysis_candidate(
            &m,
            AnalysisInput {
                audio_manifest_hash: s.recorded_analysis_input(&m).unwrap().0.manifest_hash,
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![
                    SpeakerInput {
                        key: "physical".into(),
                        label: "元の話者".into(),
                    },
                    SpeakerInput {
                        key: "corrected".into(),
                        label: "修正した話者".into(),
                    },
                ],
                segments: vec![SpeakerSegmentInput {
                    track_id: "track".into(),
                    speaker_key: Some("physical".into()),
                    start_us: 0,
                    end_us: 1_000_000,
                    overlap_group: None,
                    confidence: None,
                }],
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    speaker_keys: vec!["corrected".into()],
                    start_us: 0,
                    end_us: 1_000_000,
                    text: "修正済みの発言".into(),
                    confidence: None,
                }],
            },
        )
        .unwrap();
    s.commit_analysis_candidate(
        &m,
        &id,
        s.get_meeting(&m).unwrap().unwrap().lock_version,
        true,
    )
    .unwrap();
    let c = s.voice_context(&m).unwrap();
    assert!(c.windows.is_empty());
    let w = c.registration_candidates[0].window.clone();
    let (job, spec, plan) = enqueue_selected(&s, &m, &w.speaker_id, true, Some(vec![w.clone()]));
    s.check_voice_plan(&spec, &plan).unwrap();
    s.publish_voice_result(&spec, &plan, vec![(w.clone(), vec![1., 0.])], &job.id)
        .unwrap();
    let c = s.voice_context(&m).unwrap();
    assert_eq!(c.matches[0].speaker_id, w.speaker_id);
    assert_eq!(c.matches[0].state, "confirmed");
    assert_eq!(c.people[0].samples.len(), 1);
}

#[test]
#[ignore = "Read-only inspection of an explicitly supplied investigation snapshot"]
fn inspect_saved_manual_registration_candidates() {
    let path =
        std::env::var("MON_VOICE_INSPECT_SNAPSHOT").expect("supply investigation snapshot path");
    let mut db =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tx = db.transaction().unwrap();
    let title = std::env::var("MON_VOICE_INSPECT_MEETING").expect("supply meeting title");
    let name = std::env::var("MON_VOICE_INSPECT_SPEAKER").expect("supply speaker name");
    let m: String = tx
        .query_row("SELECT id FROM meetings WHERE title=?1", [&title], |r| {
            r.get(0)
        })
        .unwrap();
    let (view, _, _) = working(&tx, &m).unwrap();
    let speaker = &view
        .speakers
        .iter()
        .find(|s| s.display_name.as_deref() == Some(name.as_str()))
        .unwrap()
        .id;
    let c = candidates(&tx, &view).unwrap();
    let selected: Vec<_> = c
        .iter()
        .filter(|c| &c.window.speaker_id == speaker)
        .collect();
    assert!(!selected.is_empty());
    assert!(windows(&tx, &view)
        .unwrap()
        .iter()
        .all(|w| &w.speaker_id != speaker));
    println!(
        "{} registration candidates; existing matching policy unchanged",
        selected.len()
    );
}
