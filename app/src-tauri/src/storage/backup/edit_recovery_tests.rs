use super::restore_fixed::{workspace::seal::SealedWorkspace, FixedData};
use super::restore_ids::{BatchPreparation, RestoreIds};
use super::*;
use crate::storage::edit_recovery::{SaveRequest, TextBuffer};

fn split_transcript(storage: &Storage) -> String {
    use crate::storage::draft_splits::{SplitPart, UtteranceSplit};
    use crate::storage::drafts::{ManualSpeaker, UtteranceEdit};
    let meeting = archive::tests::meeting(storage, "edit-recovery-roundtrip");
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let utterance = &analysis.utterances[0];
    let speaker = format!("manual:{}", Uuid::new_v4());
    let mut payload = draft.payload.clone();
    payload.schema_version = 3;
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
fn recovery_round_trip_remaps_split_and_manual_ids_preserving_text_and_stale_context() {
    for stale in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = Storage::open(root.path().into()).unwrap();
        let meeting = split_transcript(&source);
        let draft = source.load_transcript_draft(&meeting).unwrap().unwrap();
        let part = &draft.payload.utterance_splits[0].parts[0];
        let value = format!("未確定の本文に含まれるID {} は置換しない", part.id);
        let saved = source
            .save_transcript_edit_recovery(SaveRequest {
                meeting_id: meeting.clone(),
                draft_id: draft.id.clone(),
                base_analysis_id: draft.base_analysis_id.clone(),
                operation_id: Uuid::new_v4().to_string(),
                expected_draft_lock_version: draft.lock_version,
                expected_recovery_lock_version: 0,
                buffers: vec![TextBuffer {
                    utterance_id: part.id.clone(),
                    text: part.effective_text.clone(),
                    speaker_ids: part.speaker_ids.clone(),
                    had_override: false,
                    value: value.clone(),
                }],
            })
            .unwrap();
        if stale {
            source
                .undo_last_transcript_edit(&draft.id, draft.lock_version)
                .unwrap();
        }
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("recovery.monbackup");
        source.create_backup_archive(&path, &|| false).unwrap();
        let archive =
            archive::read(&mut File::open(path).unwrap(), output.path(), &|| false).unwrap();
        let target_root = tempfile::tempdir().unwrap();
        let target = Storage::open(target_root.path().into()).unwrap();
        let keep = target.create_meeting("既存会議は変更しない").unwrap();
        let ids = match RestoreIds::prepare_for_batch(
            &archive,
            output.path(),
            &Uuid::new_v4().to_string(),
            &target.connection.lock().unwrap(),
            &|| false,
        )
        .unwrap()
        {
            BatchPreparation::Pending(ids) => ids,
            _ => panic!("new batch"),
        };
        let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
        let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
        target.publish_restore(&sealed, &ids, &|| false).unwrap();
        let new_meeting = ids.get("meetings", "", &meeting).unwrap();
        let new_base = ids
            .get("analysis_revisions", &meeting, &draft.base_analysis_id)
            .unwrap();
        let new_draft = ids.get("working_drafts", &meeting, &draft.id).unwrap();
        let loaded = target
            .transcript_edit_recovery(&new_meeting, &new_draft, &new_base)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.matches_draft, !stale);
        assert_eq!(loaded.lock_version, saved.lock_version);
        assert_eq!(
            loaded.base_draft_lock_version,
            saved.base_draft_lock_version
        );
        assert_eq!(loaded.payload.buffers[0].value, value);
        assert_eq!(loaded.payload.buffers[0].text, part.effective_text);
        assert_eq!(
            loaded.payload.buffers[0].utterance_id,
            ids.get("utterances", &meeting, &part.id).unwrap()
        );
        assert_ne!(loaded.payload.buffers[0].utterance_id, part.id);
        assert_ne!(loaded.payload.buffers[0].speaker_ids, part.speaker_ids);
        assert_ne!(loaded.operation_id, saved.operation_id);
        assert_ne!(loaded.content_hash, saved.content_hash);
        assert_ne!(
            loaded.base_draft_content_hash,
            saved.base_draft_content_hash
        );
        assert_eq!(target.get_meeting(&keep.id).unwrap(), Some(keep));
        let again = output.path().join("restored.monbackup");
        target.create_backup_archive(&again, &|| false).unwrap();
        let checked =
            archive::read(&mut File::open(again).unwrap(), output.path(), &|| false).unwrap();
        let payload: String = checked
            .database
            .query_row(
                "SELECT payload_json FROM transcript_edit_recovery WHERE draft_id=?1",
                params![new_draft],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<crate::storage::edit_recovery::Payload>(&payload).unwrap(),
            loaded.payload
        );
        assert_eq!(
            source
                .transcript_edit_recovery(&meeting, &draft.id, &draft.base_analysis_id)
                .unwrap()
                .unwrap()
                .content_hash,
            saved.content_hash
        );
    }
}

#[test]
fn portable_v27_and_v28_have_exact_version_specific_tables() {
    let old = portable::schema(27).unwrap();
    let new = portable::schema(28).unwrap();
    assert_eq!(portable::tables(&old).unwrap().len(), 31);
    assert_eq!(portable::tables(&new).unwrap().len(), 32);
    assert!(!portable::tables(&old)
        .unwrap()
        .iter()
        .any(|t| t.name == "transcript_edit_recovery"));
    crate::storage::edit_recovery::verify_all(&old, &|| false).unwrap();
    crate::storage::edit_recovery::verify_all(&new, &|| false).unwrap();
}
