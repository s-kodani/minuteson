use super::*;

fn split_transcript(storage: &Storage) -> String {
    use crate::storage::draft_splits::{SplitPart, UtteranceSplit};
    use crate::storage::drafts::{ManualSpeaker, UtteranceEdit};
    let meeting = archive::tests::meeting(storage, "edit-recovery-roundtrip");
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let utterance = &analysis.utterances[0];
    let speaker = format!("manual:{}", Uuid::new_v4());
    let mut payload = draft.payload.clone();
    payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    payload.manual_speakers.push(ManualSpeaker {
        id: speaker.clone(),
        label: "手動話者".into(),
    });
    payload.utterances.push(UtteranceEdit {
        utterance_id: utterance.id.clone(),
        effective_text: "前半後半".into(),
        speaker_ids: vec![speaker.clone()],
    });
    payload.utterance_splits.push(UtteranceSplit {
        source_utterance_id: utterance.id.clone(),
        source_effective_text: "前半後半".into(),
        parts: vec![
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: 0,
                end_us: 500_000,
                effective_text: "前半".into(),
                speaker_ids: vec![speaker.clone()],
            },
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: 500_000,
                end_us: 1_000_000,
                effective_text: "後半".into(),
                speaker_ids: vec![speaker],
            },
        ],
    });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    meeting
}

#[test]
fn saved_split_transcript_becomes_one_current_baseline_without_edit_history() {
    let root = tempfile::tempdir().unwrap();
    let source = Storage::open(root.path().into()).unwrap();
    let meeting = split_transcript(&source);
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("edited.monbackup");
    source.create_backup_archive(&path, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let prepared = target
        .prepare_additive_restore(&path, &|| false, &|_| {})
        .unwrap();
    let receipt = prepared
        .publish(
            &target,
            &prepared.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    let view = target
        .load_current_analysis(&receipt.meeting_ids[0])
        .unwrap()
        .unwrap();
    assert_eq!(view.utterances.len(), 2);
    assert_eq!(view.utterances[0].effective_text, "前半");
    assert_eq!(view.utterances[1].effective_text, "後半");
    assert_eq!(
        (view.utterances[0].start_us, view.utterances[0].end_us),
        (0, 500_000)
    );
    assert_eq!(view.speakers.len(), 1);
    assert_eq!(view.speakers[0].label, "手動話者");
    assert!(view
        .utterances
        .iter()
        .all(|u| u.speaker_ids == vec![view.speakers[0].id.clone()]));
    assert!(target
        .load_transcript_draft(&receipt.meeting_ids[0])
        .unwrap()
        .is_none());
    let db = target.connection.lock().unwrap();
    for table in [
        "edit_operations",
        "transcript_edit_recovery",
        "processing_jobs",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM analysis_revisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    assert!(source.load_transcript_draft(&meeting).unwrap().is_some());
}
