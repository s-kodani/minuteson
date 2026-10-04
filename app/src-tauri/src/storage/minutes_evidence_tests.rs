use super::*;
use crate::minutes_document::{from_generated, Block, GeneratedItem, Inline, ItemKind, SourceType};
use serde_json::json;

fn candidate(s: &Storage, m: &str) -> String {
    let analysis = s.load_current_analysis(m).unwrap().unwrap();
    let draft = s.load_transcript_draft(m).unwrap().unwrap();
    let job = s
        .enqueue_minutes_job(
            MinutesJobRequest {
                meeting_id: m.into(),
                operation_id: Uuid::new_v4().to_string(),
                expected_analysis_id: analysis.id,
                expected_content_hash: analysis.content_hash,
                expected_meeting_lock_version: s.get_meeting(m).unwrap().unwrap().lock_version,
                expected_draft: Some(DraftStamp {
                    id: draft.id,
                    lock_version: draft.lock_version,
                }),
            },
            json!({"fixture":true}),
            json!({"templateVersion":"standard-v1"}),
            Some(1),
        )
        .unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    s.save_job_checkpoint(&job.id, 0, 1, json!({"fixture":true}))
        .unwrap();
    let input = s.load_minutes_input(&job.id).unwrap();
    let index = input.analysis.utterances.len() / 2;
    let content = from_generated(
        &input,
        vec![GeneratedItem {
            kind: ItemKind::Decision,
            body: input.analysis.utterances[index].effective_text.clone(),
            assignee_name: None,
            assignee_speaker_id: None,
            due_date: None,
            due_text: None,
            evidence_utterance_ids: vec![input.analysis.utterances[index].id.clone()],
        }],
    )
    .unwrap();
    s.finish_minutes_candidate(&job.id, 1, content).unwrap()
}
fn request(s: &Storage, m: &str, id: &str) -> MinutesEvidenceRequest {
    let view = s.load_minutes_content(m, id).unwrap().unwrap();
    MinutesEvidenceRequest {
        meeting_id: m.into(),
        minutes_id: id.into(),
        expected_content_hash: view.content_hash,
        expected_input_content_hash: view.input_content_hash,
        expected_evidence: view.content.evidence[0].clone(),
    }
}
fn fixture(s: &Storage) -> (String, String, String) {
    let (m, analysis, utterance) = super::super::playback::tests::fixture(s);
    s.create_transcript_draft(&m, &analysis).unwrap();
    let id = candidate(s, &m);
    (m, id, utterance)
}

#[test]
fn fixed_reference_survives_source_edits_and_minutes_adoption_and_restore() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, a, u) = fixture(&s);
    let r = request(&s, &m, &a);
    // Candidate references work before adoption, but arbitrary snapshot playback does not.
    let fixed = s.minutes_evidence_source(&r).unwrap();
    assert_eq!(fixed.evidence.quote_text, "発言");
    assert!(s
        .prepare_utterance_audio(&m, &fixed.analysis_id, &fixed.evidence.utterance_id)
        .is_err());
    let audio = s.prepare_minutes_evidence_audio(&r).unwrap();
    assert_eq!((audio.start_us, audio.end_us), (250_000, 750_000));
    let store = crate::playback::PlaybackStore::default();
    let info = store.insert(audio).unwrap();
    assert_eq!(info.total_frames, 24_000);
    assert!(!store.read(&info.token, 0).unwrap().is_empty());
    store.close(&info.token).unwrap();
    s.adopt_minutes(super::super::minutes_ui::tests::request(&s, &m, &a))
        .unwrap();
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let mut payload = draft.payload.clone();
    payload
        .utterances
        .push(super::super::drafts::UtteranceEdit {
            utterance_id: u,
            effective_text: "後から修正した発言".into(),
            speaker_ids: vec![],
        });
    s.save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let b = candidate(&s, &m);
    let rb = request(&s, &m, &b);
    assert_eq!(
        s.minutes_evidence_source(&rb).unwrap().evidence.quote_text,
        "後から修正した発言"
    );
    s.adopt_minutes(super::super::minutes_ui::tests::request(&s, &m, &b))
        .unwrap();
    assert_eq!(
        s.minutes_evidence_source(&r).unwrap().evidence,
        fixed.evidence
    );
    assert_ne!(
        r.expected_evidence.analysis_revision_id,
        rb.expected_evidence.analysis_revision_id
    );
    let restore = super::super::minutes_ui::tests::request(&s, &m, &a);
    s.restore_minutes_checked(
        &m,
        restore.expected_meeting_lock_version,
        &a,
        &restore.expected_content_hash,
        restore.expected_draft,
    )
    .unwrap();
    assert_eq!(
        s.minutes_evidence_source(&r).unwrap().utterances,
        fixed.utterances
    );
    assert!(s.prepare_minutes_evidence_audio(&r).is_ok());
}

#[test]
fn mismatched_references_changed_input_and_missing_audio_are_not_redirected() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id, _) = fixture(&s);
    let r = request(&s, &m, &id);
    let mut variants = vec![];
    let mut x = r.clone();
    x.meeting_id = "other".into();
    variants.push(x);
    let mut x = r.clone();
    x.expected_content_hash = "0".repeat(64);
    variants.push(x);
    let mut x = r.clone();
    x.expected_evidence.quote_text = "改変".into();
    variants.push(x);
    let mut x = r.clone();
    x.expected_evidence.start_us += 1;
    variants.push(x);
    let mut x = r.clone();
    x.expected_evidence.analysis_revision_id = "current".into();
    variants.push(x);
    let mut x = r.clone();
    x.expected_evidence.utterance_id = "another".into();
    variants.push(x);
    let mut x = r.clone();
    x.expected_evidence.id = "missing".into();
    variants.push(x);
    for x in variants {
        assert!(s.minutes_evidence_source(&x).is_err());
        assert!(s.prepare_minutes_evidence_audio(&x).is_err());
    }
    s.connection.lock().unwrap().execute("INSERT INTO audio_gaps (id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',300000,400000,'test',0)", []).unwrap();
    assert!(s.minutes_evidence_source(&r).is_ok());
    assert!(s
        .prepare_minutes_evidence_audio(&r)
        .err()
        .unwrap()
        .contains("欠損"));
    s.connection
        .lock()
        .unwrap()
        .execute("DELETE FROM audio_gaps", [])
        .unwrap();
    let path = root.path().join("meetings").join(&m).join("audio/0.caf");
    std::fs::write(path, b"changed").unwrap();
    assert!(s.prepare_minutes_evidence_audio(&r).is_err());
    assert!(s
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE utterances SET effective_text='changed' WHERE id=?1",
            params![r.expected_evidence.utterance_id],
        )
        .is_err());
    // Simulate corruption beyond the normal immutable SQL boundary.
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER utterances_internal_update")
        .unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE utterances SET effective_text='changed' WHERE id=?1",
            params![r.expected_evidence.utterance_id],
        )
        .unwrap();
    assert!(s.minutes_evidence_source(&r).is_err());
}

#[test]
fn saved_manual_references_use_the_same_fixed_input_without_becoming_verified() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id, _) = fixture(&s);
    s.adopt_minutes(super::super::minutes_ui::tests::request(&s, &m, &id))
        .unwrap();
    let draft = s.create_minutes_draft(&m, &id).unwrap();
    let mut payload = draft.payload.clone();
    let mut item = payload.items[0].clone();
    item.id = Uuid::new_v4().to_string();
    item.source_type = SourceType::Manual;
    let mut e = payload.evidence[0].clone();
    e.id = Uuid::new_v4().to_string();
    e.minutes_item_id = item.id.clone();
    e.source_type = SourceType::Manual;
    e.quote_text = "発".into();
    e.start_us = 350_000;
    e.end_us = 650_000;
    payload.blocks.push(Block::Paragraph {
        item_id: Some(item.id.clone()),
        content: vec![
            Inline::text(&item.body),
            Inline::Evidence {
                minutes_item_id: item.id.clone(),
                evidence_link_id: e.id.clone(),
            },
        ],
    });
    payload.items.push(item);
    payload.evidence.push(e.clone());
    s.save_minutes_draft(&draft.id, 0, payload).unwrap();
    let mut r = request(&s, &m, &id);
    r.expected_evidence = e;
    assert_eq!(
        s.minutes_evidence_source(&r)
            .unwrap()
            .evidence
            .verification_state,
        crate::minutes_document::VerificationState::Unverified
    );
    let audio = s.prepare_minutes_evidence_audio(&r).unwrap();
    assert_eq!((audio.start_us, audio.end_us), (350_000, 650_000));
}

#[test]
fn source_context_is_bounded_to_two_neighbors_and_uses_the_selected_fixed_utterance() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, _, _) = super::super::playback::tests::fixture(&s);
    let input = AnalysisInput {
        audio_manifest_hash: "a".repeat(64),
        profile_snapshot: json!({}),
        config: json!({}),
        speakers: vec![],
        segments: vec![],
        utterances: (0..7)
            .map(|i| UtteranceInput {
                track_id: "track".into(),
                start_us: i * 100_000,
                end_us: (i + 1) * 100_000,
                text: format!("発言{i}"),
                speaker_keys: vec![],
                confidence: None,
            })
            .collect(),
    };
    let a = s.create_analysis_candidate(&m, input).unwrap();
    s.commit_analysis_candidate(
        &m,
        &a,
        s.get_meeting(&m).unwrap().unwrap().lock_version,
        false,
    )
    .unwrap();
    s.create_transcript_draft(&m, &a).unwrap();
    let id = candidate(&s, &m);
    let r = request(&s, &m, &id);
    let source = s.minutes_evidence_source(&r).unwrap();
    assert_eq!(source.total_utterances, 7);
    assert_eq!(source.first_sequence, 2);
    assert_eq!(source.utterances.len(), 5);
    assert_eq!(source.utterances[2].effective_text, "発言3");
    assert_eq!(source.utterances[2].id, r.expected_evidence.utterance_id);
}
