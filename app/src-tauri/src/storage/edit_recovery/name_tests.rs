use super::*;
use crate::storage::drafts::{ManualSpeaker, SpeakerNameEdit, UtteranceEdit};
use crate::storage::playback::tests::fixture;

fn setup(storage: &Storage) -> (TranscriptDraft, SaveRequest, String) {
    let (meeting, base, utterance) = fixture(storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let speaker = format!("manual:{}", Uuid::new_v4());
    let mut payload = draft.payload;
    payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    payload.manual_speakers.push(ManualSpeaker {
        id: speaker.clone(),
        label: "名前回復用".into(),
    });
    payload.speaker_names.push(SpeakerNameEdit {
        speaker_id: speaker.clone(),
        display_name: Some("田中".into()),
    });
    let draft = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let request = SaveRequest {
        meeting_id: meeting,
        draft_id: draft.id.clone(),
        base_analysis_id: base,
        operation_id: Uuid::new_v4().to_string(),
        expected_draft_lock_version: draft.lock_version,
        expected_recovery_lock_version: 0,
        buffers: vec![],
        name_buffers: vec![NameBuffer {
            speaker_id: speaker,
            name: Some("田中".into()),
            had_override: true,
            value: "入力途中の名前".into(),
        }],
    };
    (draft, request, utterance)
}

#[test]
fn names_survive_cold_reopen_without_publishing_a_name_or_undo() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, request, _) = setup(&storage);
    let saved = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(saved.payload.schema_version, 4);
    assert!(saved.payload.buffers.is_empty());
    assert_eq!(saved.payload.name_buffers, request.name_buffers);
    assert!(saved.matches_draft && saved.current_context);
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM edit_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let page = storage
        .list_transcript_edit_recoveries(&draft.meeting_id, None)
        .unwrap();
    assert_eq!(page.items[0].buffer_count, 1);
    assert_eq!(page.items[0].content_hash, saved.content_hash);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let loaded = storage
        .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.payload, saved.payload);
    assert_eq!(loaded.content_hash, saved.content_hash);
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM edit_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn mixed_names_and_text_have_one_cas_and_an_empty_clear_blocks_delayed_names() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, mut request, utterance) = setup(&storage);
    request.buffers.push(TextBuffer {
        utterance_id: utterance,
        text: "発言".into(),
        speaker_ids: vec![],
        had_override: false,
        value: "未確定本文".into(),
    });
    let saved = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(
        storage
            .list_transcript_edit_recoveries(&draft.meeting_id, None)
            .unwrap()
            .items[0]
            .buffer_count,
        2
    );
    let retry = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(retry.lock_version, saved.lock_version);
    assert_eq!(retry.updated_at, saved.updated_at);
    for field in 0..4 {
        let mut changed = request.clone();
        match field {
            0 => changed.name_buffers[0].value = "別の名前".into(),
            1 => changed.name_buffers[0].name = None,
            2 => changed.name_buffers[0].had_override = false,
            _ => changed.name_buffers.clear(),
        }
        assert!(storage.save_transcript_edit_recovery(changed).is_err());
    }
    let mut clear = request.clone();
    clear.operation_id = Uuid::new_v4().to_string();
    clear.expected_recovery_lock_version = saved.lock_version;
    clear.buffers.clear();
    clear.name_buffers.clear();
    let empty = storage.save_transcript_edit_recovery(clear).unwrap();
    assert_eq!(empty.payload.schema_version, 4);
    assert_eq!(empty.lock_version, saved.lock_version + 1);
    assert!(empty.payload.buffers.is_empty() && empty.payload.name_buffers.is_empty());
    assert!(storage.save_transcript_edit_recovery(request).is_err());
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
}

#[test]
fn newer_draft_and_undo_keep_names_readable_without_adopting_their_old_context() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, request, utterance) = setup(&storage);
    let saved = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    let mut payload = draft.payload.clone();
    payload.utterances.push(UtteranceEdit {
        utterance_id: utterance,
        effective_text: "確定した本文".into(),
        speaker_ids: vec![],
    });
    let changed = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let old = storage
        .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
        .unwrap()
        .unwrap();
    assert!(!old.matches_draft);
    assert_eq!(old.payload.base_payload, draft.payload);
    assert_eq!(old.payload.name_buffers, request.name_buffers);
    assert!(
        !storage
            .save_transcript_edit_recovery(request.clone())
            .unwrap()
            .matches_draft
    );
    let undone = storage
        .undo_last_transcript_edit(&draft.id, changed.lock_version)
        .unwrap();
    let mut rebase = request.clone();
    rebase.operation_id = Uuid::new_v4().to_string();
    rebase.expected_recovery_lock_version = saved.lock_version;
    assert!(storage
        .save_transcript_edit_recovery(rebase.clone())
        .is_err());
    rebase.expected_draft_lock_version = undone.lock_version;
    let rebound = storage.save_transcript_edit_recovery(rebase).unwrap();
    assert!(rebound.matches_draft);
    assert_eq!(rebound.payload.name_buffers, request.name_buffers);
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        undone
    );
}

#[test]
fn consented_discard_clears_names_with_exact_proof_and_never_publishes_them() {
    use review::DiscardRequest;
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, input, _) = setup(&storage);
    let saved = storage
        .save_transcript_edit_recovery(input.clone())
        .unwrap();
    let request = DiscardRequest {
        meeting_id: draft.meeting_id.clone(),
        draft_id: draft.id.clone(),
        base_analysis_id: draft.base_analysis_id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_draft_lock_version: draft.lock_version,
        expected_draft_content_hash: draft.content_hash.clone(),
        expected_recovery_lock_version: saved.lock_version,
        expected_recovery_content_hash: saved.content_hash.clone(),
    };
    let empty = storage
        .discard_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(empty.payload.schema_version, 4);
    assert!(empty.payload.name_buffers.is_empty() && empty.payload.buffers.is_empty());
    assert_eq!(
        empty.payload.discarded_from,
        Some(DiscardProof {
            content_hash: saved.content_hash,
            lock_version: saved.lock_version
        })
    );
    assert_eq!(
        storage
            .discard_transcript_edit_recovery(request)
            .unwrap()
            .content_hash,
        empty.content_hash
    );
    assert!(storage.save_transcript_edit_recovery(input).is_err());
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    assert_eq!(
        storage
            .list_transcript_edit_recoveries(&draft.meeting_id, None)
            .unwrap()
            .items[0]
            .buffer_count,
        0
    );
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        reopened
            .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
            .unwrap()
            .unwrap()
            .payload,
        empty.payload
    );
}

#[test]
fn foreign_duplicate_and_oversized_names_cannot_replace_an_existing_record() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, request, _) = setup(&storage);
    let saved = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    for field in 0..6 {
        let mut bad = request.clone();
        bad.operation_id = Uuid::new_v4().to_string();
        bad.expected_recovery_lock_version = saved.lock_version;
        match field {
            0 => bad.name_buffers[0].speaker_id = "foreign".into(),
            1 => bad.name_buffers.push(bad.name_buffers[0].clone()),
            2 => bad.name_buffers = vec![bad.name_buffers[0].clone(); 101],
            3 => bad.name_buffers[0].name = Some("😀".repeat(201)),
            4 => bad.name_buffers[0].value = "😀".repeat(201),
            _ => bad.meeting_id = "foreign".into(),
        }
        assert!(
            storage.save_transcript_edit_recovery(bad).is_err(),
            "field {field}"
        );
        assert_eq!(
            storage
                .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
                .unwrap()
                .unwrap()
                .content_hash,
            saved.content_hash
        );
    }
    let mut valid = request;
    valid.operation_id = Uuid::new_v4().to_string();
    valid.expected_recovery_lock_version = saved.lock_version;
    valid.name_buffers[0].name = None;
    valid.name_buffers[0].value = "😀".repeat(200);
    assert_eq!(
        storage
            .save_transcript_edit_recovery(valid)
            .unwrap()
            .payload
            .name_buffers[0]
            .value
            .chars()
            .count(),
        200
    );
}

#[test]
fn adoption_keeps_old_names_readable_and_discard_separate_from_current_analysis_names() {
    use crate::storage::analysis::{AnalysisInput, SpeakerInput, UtteranceInput};
    use crate::storage::analysis_ui::AdoptRequest;
    use review::DiscardRequest;
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (old_draft, old_request, _) = setup(&storage);
    let old = storage
        .save_transcript_edit_recovery(old_request.clone())
        .unwrap();
    let next = storage
        .create_analysis_candidate(
            &old.meeting_id,
            AnalysisInput {
                audio_manifest_hash: storage.canonical_audio_plan(&old.meeting_id).unwrap().manifest_hash,
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![
                    SpeakerInput {
                        key: "A".into(),
                        label: "話者A".into(),
                    },
                    SpeakerInput {
                        key: "B".into(),
                        label: "話者B".into(),
                    },
                ],
                segments: vec![],
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    start_us: 250_000,
                    end_us: 750_000,
                    text: "次の版".into(),
                    speaker_keys: vec!["A".into()],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    let versions = storage.analysis_versions(&old.meeting_id).unwrap();
    let hash = versions
        .versions
        .iter()
        .find(|v| v.id == next)
        .unwrap()
        .content_hash
        .clone();
    storage
        .adopt_analysis(AdoptRequest {
            meeting_id: old.meeting_id.clone(),
            revision_id: next.clone(),
            expected_content_hash: hash,
            expected_meeting_lock_version: versions.meeting_lock_version,
            expected_draft: versions.draft,
            confirm_replacing_previous: true,
        })
        .unwrap();
    let analysis = storage
        .load_current_analysis(&old.meeting_id)
        .unwrap()
        .unwrap();
    let draft = storage
        .create_transcript_draft(&old.meeting_id, &next)
        .unwrap();
    let mut request = old_request.clone();
    request.draft_id = draft.id.clone();
    request.base_analysis_id = next;
    request.expected_draft_lock_version = draft.lock_version;
    request.operation_id = Uuid::new_v4().to_string();
    request.name_buffers = analysis
        .speakers
        .iter()
        .map(|s| NameBuffer {
            speaker_id: s.id.clone(),
            name: s.display_name.clone(),
            had_override: false,
            value: format!("未確定の{}", s.label),
        })
        .collect();
    let current = storage.save_transcript_edit_recovery(request).unwrap();
    let archived = storage
        .transcript_edit_recovery(&old.meeting_id, &old.draft_id, &old.base_analysis_id)
        .unwrap()
        .unwrap();
    assert!(!archived.current_context && archived.matches_draft);
    assert_eq!(archived.payload.name_buffers, old.payload.name_buffers);
    assert!(storage.save_transcript_edit_recovery(old_request).is_err());
    let page = storage
        .list_transcript_edit_recoveries(&old.meeting_id, None)
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.items
            .iter()
            .find(|r| r.draft_id == old.draft_id)
            .unwrap()
            .buffer_count,
        1
    );
    assert_eq!(
        page.items
            .iter()
            .find(|r| r.draft_id == current.draft_id)
            .unwrap()
            .buffer_count,
        2
    );
    let discarded = storage
        .discard_transcript_edit_recovery(DiscardRequest {
            meeting_id: old.meeting_id.clone(),
            draft_id: old.draft_id.clone(),
            base_analysis_id: old.base_analysis_id.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected_draft_lock_version: old.current_draft_lock_version,
            expected_draft_content_hash: old.current_draft_content_hash,
            expected_recovery_lock_version: old.lock_version,
            expected_recovery_content_hash: old.content_hash,
        })
        .unwrap();
    assert!(!discarded.current_context);
    assert!(discarded.payload.name_buffers.is_empty());
    assert_eq!(
        storage
            .load_transcript_draft(&old.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    assert_eq!(
        storage
            .load_current_analysis(&old.meeting_id)
            .unwrap()
            .unwrap(),
        analysis
    );
    assert_eq!(
        storage
            .transcript_edit_recovery(&old.meeting_id, &draft.id, &draft.base_analysis_id)
            .unwrap()
            .unwrap()
            .content_hash,
        current.content_hash
    );
    assert_eq!(
        storage
            .transcript_edit_recovery(&old.meeting_id, &old_draft.id, &old_draft.base_analysis_id)
            .unwrap()
            .unwrap()
            .lock_version,
        old.lock_version + 1
    );
}

#[test]
fn one_hundred_names_at_unicode_limits_are_saved_sorted_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, mut request, _) = setup(&storage);
    let mut payload = draft.payload;
    for n in 1..100 {
        let id = format!("manual:{}", Uuid::new_v4());
        payload.manual_speakers.push(ManualSpeaker {
            id: id.clone(),
            label: format!("話者{n}"),
        });
        request.name_buffers.push(NameBuffer {
            speaker_id: id,
            name: None,
            had_override: false,
            value: "😀".repeat(200),
        });
    }
    request.name_buffers[0].name = Some("😀".repeat(200));
    request.name_buffers[0].value = String::new();
    request.name_buffers.reverse();
    let draft = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    request.expected_draft_lock_version = draft.lock_version;
    let saved = storage
        .save_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(saved.payload.name_buffers.len(), 100);
    assert!(saved
        .payload
        .name_buffers
        .windows(2)
        .all(|pair| pair[0].speaker_id < pair[1].speaker_id));
    assert_eq!(
        saved.payload.name_buffers,
        normalize_names(request.name_buffers).unwrap()
    );
    assert_eq!(
        storage
            .load_transcript_draft(&draft.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    assert_eq!(
        storage
            .list_transcript_edit_recoveries(&draft.meeting_id, None)
            .unwrap()
            .items[0]
            .buffer_count,
        100
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        storage
            .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
            .unwrap()
            .unwrap()
            .payload,
        saved.payload
    );
}

#[test]
fn name_normalization_is_bounded_sorted_and_rejects_unknown_fields() {
    let names: Vec<_> = (0..100)
        .rev()
        .map(|n| NameBuffer {
            speaker_id: format!("speaker-{n:03}"),
            name: None,
            had_override: false,
            value: String::new(),
        })
        .collect();
    let sorted = normalize_names(names.clone()).unwrap();
    assert_eq!(sorted.first().unwrap().speaker_id, "speaker-000");
    assert_eq!(sorted.last().unwrap().speaker_id, "speaker-099");
    let mut too_many = names;
    too_many.push(sorted[0].clone());
    assert!(normalize_names(too_many).is_err());
    let mut json = serde_json::to_value(&sorted[0]).unwrap();
    json["unknown"] = true.into();
    assert!(serde_json::from_value::<NameBuffer>(json).is_err());
}

#[test]
fn invalid_name_payload_formats_fail_read_and_backup_integrity_checks() {
    for case in 0..5 {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (draft, request, _) = setup(&storage);
        let saved = storage.save_transcript_edit_recovery(request).unwrap();
        let mut json = serde_json::to_value(saved.payload).unwrap();
        match case {
            0 => json["schemaVersion"] = 1.into(),
            1 => {
                json.as_object_mut().unwrap().remove("nameBuffers");
            }
            2 => {
                json["schemaVersion"] = 2.into();
                json["discardedFrom"] =
                    serde_json::json!({"contentHash":"a".repeat(64),"lockVersion":1});
            }
            3 => json["nameBuffers"][0]["unknown"] = true.into(),
            _ => json["nameBuffers"][0]["speakerId"] = "foreign".into(),
        }
        let text = serde_json::to_string(&json).unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE transcript_edit_recovery SET payload_json=?1,content_hash=?2",
            params![text, drafts::draft_hash(&text)],
        )
        .unwrap();
        assert!(verify_all(&db, &crate::test_support::never_cancelled).is_err());
        drop(db);
        assert!(storage
            .transcript_edit_recovery(&draft.meeting_id, &draft.id, &draft.base_analysis_id)
            .is_err());
    }
}

#[test]
fn current_recovery_has_explicit_arrays_and_nullable_discard_proof() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (draft, mut request, _) = setup(&storage);
    request.name_buffers.clear();
    let raw = serde_json::to_value(&request).unwrap();
    assert_eq!(raw["nameBuffers"], serde_json::json!([]));
    let saved = storage.save_transcript_edit_recovery(request).unwrap();
    let value = serde_json::to_value(&saved.payload).unwrap();
    assert_eq!(value["schemaVersion"], 4);
    assert_eq!(value["nameBuffers"], serde_json::json!([]));
    assert!(value.get("discardedFrom").unwrap().is_null());
    for version in [1, 2, 3, 5] {
        let mut payload = saved.payload.clone();
        payload.schema_version = version;
        let db = storage.connection.lock().unwrap();
        assert!(validate(&db, &draft.base_analysis_id, &payload).is_err());
    }
}
