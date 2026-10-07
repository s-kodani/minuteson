use super::*;
use crate::storage::analysis::{AnalysisInput, UtteranceInput};
use crate::storage::analysis_ui::AdoptRequest;
use crate::storage::drafts::UtteranceEdit;
use crate::storage::playback::tests::fixture;

fn save_request(draft: &TranscriptDraft, utterance: &str) -> SaveRequest {
    SaveRequest {
        meeting_id: draft.meeting_id.clone(),
        draft_id: draft.id.clone(),
        base_analysis_id: draft.base_analysis_id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_draft_lock_version: draft.lock_version,
        expected_recovery_lock_version: 0,
        name_buffers: vec![],
        buffers: vec![TextBuffer {
            utterance_id: utterance.into(),
            text: "発言".into(),
            speaker_ids: vec![],
            had_override: false,
            value: "旧版の未確定入力".into(),
        }],
    }
}

fn discard_request(record: &Recovery) -> DiscardRequest {
    DiscardRequest {
        meeting_id: record.meeting_id.clone(),
        draft_id: record.draft_id.clone(),
        base_analysis_id: record.base_analysis_id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_draft_lock_version: record.current_draft_lock_version,
        expected_draft_content_hash: record.current_draft_content_hash.clone(),
        expected_recovery_lock_version: record.lock_version,
        expected_recovery_content_hash: record.content_hash.clone(),
    }
}

// Exercise real adoption, not a test-only UPDATE of active flags or heads.
fn adopt_next(storage: &Storage, meeting: &str) -> (TranscriptDraft, String) {
    let base = storage
        .create_analysis_candidate(
            meeting,
            AnalysisInput {
                audio_manifest_hash: "a".repeat(64),
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![],
                segments: vec![],
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    start_us: 250_000,
                    end_us: 750_000,
                    text: "発言".into(),
                    speaker_keys: vec![],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    let versions = storage.analysis_versions(meeting).unwrap();
    let hash = versions
        .versions
        .iter()
        .find(|v| v.id == base)
        .unwrap()
        .content_hash
        .clone();
    storage
        .adopt_analysis(AdoptRequest {
            meeting_id: meeting.into(),
            revision_id: base.clone(),
            expected_content_hash: hash,
            expected_meeting_lock_version: versions.meeting_lock_version,
            expected_draft: versions.draft,
            confirm_replacing_previous: true,
        })
        .unwrap();
    let utterance = storage
        .load_current_analysis(meeting)
        .unwrap()
        .unwrap()
        .utterances[0]
        .id
        .clone();
    (
        storage.create_transcript_draft(meeting, &base).unwrap(),
        utterance,
    )
}

// Keep every column, including timestamps, head pointers and undo JSON.
fn published_snapshot(storage: &Storage) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let db = storage.connection.lock().unwrap();
    [
        "meetings",
        "analysis_revisions",
        "utterances",
        "working_drafts",
        "edit_operations",
    ]
    .into_iter()
    .map(|table| {
        let mut stmt = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            (0..columns)
                .map(|i| row.get(i))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    })
    .collect()
}

#[test]
fn review_lists_archived_and_current_records_without_body_or_state_mutations() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let old = storage.create_transcript_draft(&meeting, &base).unwrap();
    let saved = storage
        .save_transcript_edit_recovery(save_request(&old, &utterance))
        .unwrap();
    let (current, utterance) = adopt_next(&storage, &meeting);
    storage
        .save_transcript_edit_recovery(save_request(&current, &utterance))
        .unwrap();
    let before = published_snapshot(&storage);
    let page = storage
        .list_transcript_edit_recoveries(&meeting, None)
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_none());
    let archived = page.items.iter().find(|r| r.draft_id == old.id).unwrap();
    assert_eq!(archived.content_hash, saved.content_hash);
    assert!(archived.matches_draft);
    assert!(!archived.current_context);
    assert_eq!(archived.buffer_count, 1);
    assert!(
        page.items
            .iter()
            .find(|r| r.draft_id == current.id)
            .unwrap()
            .current_context
    );
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("旧版の未確定入力"));
    assert!(!json.contains("basePayload"));
    assert!(!json.contains("buffers"));
    let detail = storage
        .transcript_edit_recovery(&meeting, &old.id, &base)
        .unwrap()
        .unwrap();
    assert!(!detail.current_context);
    assert_eq!(detail.current_draft_content_hash, old.content_hash);
    assert_eq!(published_snapshot(&storage), before);
}

#[test]
fn review_pagination_is_bounded_sorted_meeting_scoped_and_keeps_empty_records() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let mut draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let mut utterance = utterance;
    let mut expected = vec![];
    for index in 0..41 {
        let mut request = save_request(&draft, &utterance);
        if index % 2 == 0 {
            request.buffers.clear();
        }
        storage.save_transcript_edit_recovery(request).unwrap();
        expected.push(draft.id.clone());
        if index != 40 {
            (draft, utterance) = adopt_next(&storage, &meeting);
        }
    }
    let before = published_snapshot(&storage);
    let mut cursor = None;
    let mut found = vec![];
    let mut counts = vec![];
    let mut empty = 0;
    loop {
        let page = storage
            .list_transcript_edit_recoveries(&meeting, cursor.as_deref())
            .unwrap();
        counts.push(page.items.len());
        assert!(page.items.len() <= PAGE_SIZE);
        empty += page.items.iter().filter(|r| r.buffer_count == 0).count();
        found.extend(page.items.into_iter().map(|r| r.draft_id));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    expected.sort();
    assert_eq!(found, expected);
    assert_eq!(counts, [20, 20, 1]);
    assert_eq!(empty, 21);
    assert_eq!(published_snapshot(&storage), before);
    let foreign = storage.create_meeting("別会議").unwrap();
    assert!(storage
        .list_transcript_edit_recoveries(&foreign.id, None)
        .unwrap()
        .items
        .is_empty());
    for cursor in ["invalid", "", "00000000000000000000000000000000"] {
        assert!(storage
            .list_transcript_edit_recoveries(&meeting, Some(cursor))
            .is_err());
    }
}

#[test]
fn reviewed_inactive_discard_only_writes_tombstone_and_survives_cold_start() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let old = storage.create_transcript_draft(&meeting, &base).unwrap();
    let old_request = save_request(&old, &utterance);
    let saved = storage
        .save_transcript_edit_recovery(old_request.clone())
        .unwrap();
    let (current, utterance) = adopt_next(&storage, &meeting);
    let current_saved = storage
        .save_transcript_edit_recovery(save_request(&current, &utterance))
        .unwrap();
    let detail = storage
        .transcript_edit_recovery(&meeting, &old.id, &base)
        .unwrap()
        .unwrap();
    let request = discard_request(&detail);
    let before = published_snapshot(&storage);
    let cleared = storage
        .discard_transcript_edit_recovery(request.clone())
        .unwrap();
    assert_eq!(published_snapshot(&storage), before);
    assert_eq!(cleared.lock_version, 2);
    assert_eq!(cleared.payload.schema_version, 2);
    assert_eq!(
        cleared.payload.discarded_from,
        Some(DiscardProof {
            content_hash: saved.content_hash,
            lock_version: 1
        })
    );
    assert!(cleared.payload.buffers.is_empty());
    assert!(!cleared.current_context);
    assert!(storage.save_transcript_edit_recovery(old_request).is_err());
    assert_eq!(
        storage
            .transcript_edit_recovery(&meeting, &current.id, &current.base_analysis_id)
            .unwrap()
            .unwrap()
            .content_hash,
        current_saved.content_hash
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let read = storage
        .transcript_edit_recovery(&meeting, &old.id, &base)
        .unwrap()
        .unwrap();
    assert_eq!(read.content_hash, cleared.content_hash);
    let retry = storage.discard_transcript_edit_recovery(request).unwrap();
    assert_eq!(retry.updated_at, read.updated_at);
    assert_eq!(retry.lock_version, read.lock_version);
    assert_eq!(published_snapshot(&storage), before);
}

#[test]
fn discard_requires_exact_review_hash_version_and_ownership_even_for_same_operation() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let saved = storage
        .save_transcript_edit_recovery(save_request(&draft, &utterance))
        .unwrap();
    let req = discard_request(&saved);
    for already_discarded in [false, true] {
        if already_discarded {
            storage
                .discard_transcript_edit_recovery(req.clone())
                .unwrap();
        }
        let before = storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap();
        for field in 0..12 {
            let mut bad = req.clone();
            match field {
                0 => bad.meeting_id = Uuid::new_v4().to_string(),
                1 => bad.draft_id = Uuid::new_v4().to_string(),
                2 => bad.base_analysis_id = Uuid::new_v4().to_string(),
                3 => bad.operation_id = "invalid".into(),
                4 => bad.expected_draft_lock_version += 1,
                5 => bad.expected_draft_content_hash = "f".repeat(64),
                6 => bad.expected_recovery_lock_version += 1,
                7 => bad.expected_recovery_content_hash = "f".repeat(64),
                8 => bad.expected_recovery_lock_version = 0,
                9 => bad.expected_draft_lock_version = -1,
                10 => bad.expected_recovery_content_hash = "A".repeat(64),
                _ => bad.expected_draft_content_hash.clear(),
            }
            assert!(
                storage.discard_transcript_edit_recovery(bad).is_err(),
                "field {field}, after {already_discarded}"
            );
            let read = storage
                .transcript_edit_recovery(&meeting, &draft.id, &base)
                .unwrap()
                .unwrap();
            assert_eq!(read.content_hash, before.content_hash);
            assert_eq!(read.lock_version, before.lock_version);
        }
    }
}

#[test]
fn new_input_after_preview_is_not_discarded_by_old_confirmation_or_retry() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let original = save_request(&draft, &utterance);
    let saved = storage
        .save_transcript_edit_recovery(original.clone())
        .unwrap();
    let old_confirmation = discard_request(&saved);
    let mut new_input = original;
    new_input.operation_id = Uuid::new_v4().to_string();
    new_input.expected_recovery_lock_version = 1;
    new_input.buffers[0].value = "確認後の新しい入力".into();
    let next = storage
        .save_transcript_edit_recovery(new_input.clone())
        .unwrap();
    assert!(storage
        .discard_transcript_edit_recovery(old_confirmation)
        .is_err());
    let req = discard_request(&next);
    let tombstone = storage
        .discard_transcript_edit_recovery(req.clone())
        .unwrap();
    assert!(storage
        .save_transcript_edit_recovery(new_input.clone())
        .is_err());
    new_input.operation_id = Uuid::new_v4().to_string();
    new_input.expected_recovery_lock_version = tombstone.lock_version;
    new_input.buffers[0].value = "破棄後に始めた入力".into();
    let after = storage
        .save_transcript_edit_recovery(new_input.clone())
        .unwrap();
    assert_eq!(after.payload.schema_version, 1);
    assert!(after.payload.discarded_from.is_none());
    assert!(storage.discard_transcript_edit_recovery(req).is_err());
    assert_eq!(
        storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap()
            .payload
            .buffers,
        new_input.buffers
    );
}

#[test]
fn draft_change_requires_new_review_but_discard_retry_only_proves_old_result() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let saved = storage
        .save_transcript_edit_recovery(save_request(&draft, &utterance))
        .unwrap();
    let old_req = discard_request(&saved);
    let mut payload = draft.payload.clone();
    payload.utterances.push(UtteranceEdit {
        utterance_id: utterance,
        effective_text: "確定済み本文".into(),
        speaker_ids: vec![],
    });
    let advanced = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    assert!(storage.discard_transcript_edit_recovery(old_req).is_err());
    let detail = storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .unwrap();
    assert!(!detail.matches_draft);
    assert!(detail.current_context);
    assert_eq!(detail.current_draft_lock_version, advanced.lock_version);
    assert_eq!(detail.current_draft_content_hash, advanced.content_hash);
    let req = discard_request(&detail);
    let before = published_snapshot(&storage);
    let cleared = storage
        .discard_transcript_edit_recovery(req.clone())
        .unwrap();
    assert_eq!(published_snapshot(&storage), before);
    assert_eq!(cleared.payload.base_payload, advanced.payload);
    assert!(cleared.matches_draft);
    storage
        .undo_last_transcript_edit(&draft.id, advanced.lock_version)
        .unwrap();
    let before = published_snapshot(&storage);
    let retry = storage.discard_transcript_edit_recovery(req).unwrap();
    assert_eq!(retry.content_hash, cleared.content_hash);
    assert!(!retry.matches_draft);
    assert_eq!(published_snapshot(&storage), before);
}

#[test]
fn operation_identity_cannot_switch_between_normal_save_and_reviewed_discard() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let normal = save_request(&draft, &utterance);
    let saved = storage
        .save_transcript_edit_recovery(normal.clone())
        .unwrap();
    let mut req = discard_request(&saved);
    req.operation_id = normal.operation_id;
    assert!(storage
        .discard_transcript_edit_recovery(req.clone())
        .is_err());
    req.operation_id = Uuid::new_v4().to_string();
    let cleared = storage
        .discard_transcript_edit_recovery(req.clone())
        .unwrap();
    let mut disguised = save_request(&draft, &utterance);
    disguised.operation_id = req.operation_id;
    disguised.expected_recovery_lock_version = saved.lock_version;
    disguised.buffers.clear();
    assert!(storage.save_transcript_edit_recovery(disguised).is_err());
    assert_eq!(
        storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        cleared.content_hash
    );
}

#[test]
fn retirement_keeps_review_read_only_and_refuses_discard_including_ack_retry() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let saved = storage
        .save_transcript_edit_recovery(save_request(&draft, &utterance))
        .unwrap();
    let req = discard_request(&saved);
    let cleared = storage
        .discard_transcript_edit_recovery(req.clone())
        .unwrap();
    storage
        .retired
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(storage.discard_transcript_edit_recovery(req).is_err());
    assert!(storage
        .discard_transcript_edit_recovery(discard_request(&cleared))
        .is_err());
    let page = storage
        .list_transcript_edit_recoveries(&meeting, None)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].buffer_count, 0);
    assert_eq!(page.items[0].content_hash, cleared.content_hash);
}

#[test]
fn corrupt_empty_or_inactive_records_are_not_hidden_or_automatically_fixed() {
    for corruption in 0..8 {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, base, utterance) = fixture(&storage);
        let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
        let saved = storage
            .save_transcript_edit_recovery(save_request(&draft, &utterance))
            .unwrap();
        let cleared = storage
            .discard_transcript_edit_recovery(discard_request(&saved))
            .unwrap();
        adopt_next(&storage, &meeting);
        let mut json = serde_json::to_value(&cleared.payload).unwrap();
        match corruption {
            0 => json["schemaVersion"] = 1.into(),
            1 => json["schemaVersion"] = 3.into(),
            2 => json["discardedFrom"] = serde_json::Value::Null,
            3 => json["discardedFrom"]["lockVersion"] = 0.into(),
            4 => json["discardedFrom"]["lockVersion"] = 2.into(),
            5 => json["discardedFrom"]["contentHash"] = "not-a-hash".into(),
            6 => json["discardedFrom"]["unknown"] = true.into(),
            _ => json["buffers"] = serde_json::to_value(&saved.payload.buffers).unwrap(),
        }
        let text = serde_json::to_string(&json).unwrap();
        storage.connection.lock().unwrap().execute("UPDATE transcript_edit_recovery SET payload_json=?1,content_hash=?2 WHERE draft_id=?3",params![text,drafts::draft_hash(&text),draft.id]).unwrap();
        let before = published_snapshot(&storage);
        assert!(
            storage
                .list_transcript_edit_recoveries(&meeting, None)
                .is_err(),
            "corruption {corruption}"
        );
        assert!(storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .is_err());
        assert!(storage
            .discard_transcript_edit_recovery(discard_request(&cleared))
            .is_err());
        assert_eq!(published_snapshot(&storage), before);
        let retained: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload_json FROM transcript_edit_recovery WHERE draft_id=?1",
                params![draft.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retained, text);
    }
}

#[test]
fn schema_one_serialization_stays_byte_compatible_and_unknown_discard_request_is_refused() {
    let payload = Payload {
        schema_version: 1,
        base_payload: TranscriptDraftPayload::default(),
        buffers: vec![],
        name_buffers: vec![],
        discarded_from: None,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(!json.contains("discardedFrom"));
    let legacy = format!(
        "{{\"schemaVersion\":1,\"basePayload\":{},\"buffers\":[]}}",
        serde_json::to_string(&payload.base_payload).unwrap()
    );
    assert_eq!(json, legacy);
    let mut request = serde_json::json!({"meetingId":"m","draftId":"d","baseAnalysisId":"b","operationId":"o","expectedDraftLockVersion":0,"expectedDraftContentHash":"h","expectedRecoveryLockVersion":1,"expectedRecoveryContentHash":"h"});
    assert!(serde_json::from_value::<DiscardRequest>(request.clone()).is_ok());
    request["adoptOldDraft"] = true.into();
    assert!(serde_json::from_value::<DiscardRequest>(request).is_err());
}

#[test]
fn competing_discard_and_new_input_have_one_cas_winner_without_publishing_body() {
    let root = tempfile::tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::open(root.path().into()).unwrap());
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let mut next = save_request(&draft, &utterance);
    let saved = storage.save_transcript_edit_recovery(next.clone()).unwrap();
    let discard = discard_request(&saved);
    next.operation_id = Uuid::new_v4().to_string();
    next.expected_recovery_lock_version = 1;
    next.buffers[0].value = "競合する新入力".into();
    let before = published_snapshot(&storage);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let clear_job = {
        let storage = storage.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            storage.discard_transcript_edit_recovery(discard)
        })
    };
    let save_job = {
        let storage = storage.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            storage.save_transcript_edit_recovery(next)
        })
    };
    let results = [clear_job.join().unwrap(), save_job.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(winner.lock_version, 2);
    assert_eq!(
        storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        winner.content_hash
    );
    assert_eq!(published_snapshot(&storage), before);
}
