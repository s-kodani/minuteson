use super::*;
use crate::storage::drafts::UtteranceEdit;
use crate::storage::playback::tests::fixture;

fn request(draft: &TranscriptDraft, utterance: &str) -> SaveRequest {
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
            value: "入力途中の本文\n未確定".into(),
        }],
    }
}

#[test]
fn recovery_survives_reopen_without_publishing_text_or_undo_history() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let req = request(&draft, &utterance);
    let saved = storage.save_transcript_edit_recovery(req.clone()).unwrap();
    assert!(saved.matches_draft);
    assert_eq!(saved.lock_version, 1);
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
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
        0
    );
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let loaded = storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.payload.buffers, req.buffers);
    assert_eq!(loaded.content_hash, saved.content_hash);
    assert!(loaded.matches_draft);
    assert_eq!(
        storage
            .load_current_analysis(&meeting)
            .unwrap()
            .unwrap()
            .utterances[0]
            .effective_text,
        "発言"
    );
}

#[test]
fn lost_ack_retry_is_idempotent_but_changed_request_and_stale_clear_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let req = request(&draft, &utterance);
    let saved = storage.save_transcript_edit_recovery(req.clone()).unwrap();
    let retry = storage.save_transcript_edit_recovery(req.clone()).unwrap();
    assert_eq!(saved.lock_version, retry.lock_version);
    assert_eq!(saved.updated_at, retry.updated_at);
    let mut changed = req.clone();
    changed.buffers[0].value = "別の要求".into();
    assert!(storage.save_transcript_edit_recovery(changed).is_err());
    let mut clear = req.clone();
    clear.operation_id = Uuid::new_v4().to_string();
    clear.buffers.clear();
    assert!(storage
        .save_transcript_edit_recovery(clear.clone())
        .is_err());
    clear.expected_recovery_lock_version = saved.lock_version;
    let tombstone = storage.save_transcript_edit_recovery(clear).unwrap();
    assert!(tombstone.payload.buffers.is_empty());
    assert_eq!(tombstone.lock_version, 2);
    assert!(storage.save_transcript_edit_recovery(req).is_err());
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let loaded = storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .unwrap();
    assert!(loaded.payload.buffers.is_empty());
    assert_eq!(loaded.lock_version, 2);
}

#[test]
fn advancing_draft_and_undo_preserve_old_context_without_silent_application() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let req = request(&draft, &utterance);
    storage.save_transcript_edit_recovery(req.clone()).unwrap();
    let mut payload = draft.payload.clone();
    payload.utterances.push(UtteranceEdit {
        utterance_id: utterance.clone(),
        effective_text: "確定版".into(),
        speaker_ids: vec![],
    });
    let saved = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let old = storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .unwrap();
    assert!(!old.matches_draft);
    assert_eq!(old.payload.base_payload, draft.payload);
    assert_eq!(old.payload.buffers, req.buffers);
    assert!(
        !storage
            .save_transcript_edit_recovery(req.clone())
            .unwrap()
            .matches_draft
    );
    let undone = storage
        .undo_last_transcript_edit(&draft.id, saved.lock_version)
        .unwrap();
    assert_eq!(undone.payload, draft.payload);
    assert!(
        !storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap()
            .matches_draft
    );
    let mut stale = req;
    stale.operation_id = Uuid::new_v4().to_string();
    stale.expected_recovery_lock_version = 1;
    assert!(storage
        .save_transcript_edit_recovery(stale.clone())
        .is_err());
    stale.expected_draft_lock_version = undone.lock_version;
    assert!(
        storage
            .save_transcript_edit_recovery(stale)
            .unwrap()
            .matches_draft
    );
}

#[test]
fn invalid_scope_duplicates_limits_and_negative_versions_never_replace_recovery() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let req = request(&draft, &utterance);
    let mut cases = vec![];
    for field in 0..10 {
        let mut bad = req.clone();
        match field {
            0 => bad.meeting_id = "foreign".into(),
            1 => bad.draft_id = "foreign".into(),
            2 => bad.base_analysis_id = "foreign".into(),
            3 => bad.operation_id = "not-a-uuid".into(),
            4 => bad.expected_draft_lock_version = -1,
            5 => bad.expected_recovery_lock_version = -1,
            6 => bad.buffers[0].utterance_id = "foreign".into(),
            7 => bad.buffers[0].speaker_ids.push("foreign".into()),
            8 => bad.buffers.push(bad.buffers[0].clone()),
            _ => bad.buffers[0].value = "あ".repeat(20_001),
        }
        cases.push(bad);
    }
    for bad in cases {
        assert!(storage.save_transcript_edit_recovery(bad).is_err());
    }
    assert!(storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .is_none());
    let saved = storage.save_transcript_edit_recovery(req.clone()).unwrap();
    assert!(storage
        .transcript_edit_recovery("foreign", &draft.id, &base)
        .is_err());
    assert!(storage
        .transcript_edit_recovery(&meeting, "foreign", &base)
        .is_err());
    assert!(storage
        .transcript_edit_recovery(&meeting, &draft.id, "foreign")
        .is_err());
    assert_eq!(
        storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        saved.content_hash
    );
}

#[test]
fn corrupted_payload_hash_future_version_and_unknown_fields_fail_closed() {
    for corruption in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, base, utterance) = fixture(&storage);
        let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
        let req = request(&draft, &utterance);
        let saved = storage.save_transcript_edit_recovery(req.clone()).unwrap();
        let db = storage.connection.lock().unwrap();
        match corruption {
            0 => {
                db.execute(
                    "UPDATE transcript_edit_recovery SET content_hash=?1",
                    params!["f".repeat(64)],
                )
                .unwrap();
            }
            1 => {
                db.execute(
                    "UPDATE transcript_edit_recovery SET base_draft_content_hash=?1",
                    params!["f".repeat(64)],
                )
                .unwrap();
            }
            2 => {
                db.execute(
                    "UPDATE transcript_edit_recovery SET base_draft_lock_version=99",
                    [],
                )
                .unwrap();
            }
            _ => {
                let mut json = serde_json::to_value(saved.payload).unwrap();
                json["unknown"] = serde_json::json!(true);
                let text = serde_json::to_string(&json).unwrap();
                db.execute(
                    "UPDATE transcript_edit_recovery SET payload_json=?1, content_hash=?2",
                    params![text, drafts::draft_hash(&text)],
                )
                .unwrap();
            }
        }
        assert!(verify_all(&db, &|| false).is_err());
        drop(db);
        assert!(storage
            .transcript_edit_recovery(&meeting, &draft.id, &base)
            .is_err());
        assert!(storage.save_transcript_edit_recovery(req).is_err());
    }
}

#[test]
fn retirement_rejects_writes_but_keeps_readable_recovery() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let req = request(&draft, &utterance);
    storage.save_transcript_edit_recovery(req.clone()).unwrap();
    storage
        .retired
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(storage.save_transcript_edit_recovery(req).is_err());
    assert!(storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .is_some());
}

#[test]
fn v27_migration_preserves_existing_draft_hash_version_and_undo_chain() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let mut payload = draft.payload.clone();
    payload.utterances.push(UtteranceEdit {
        utterance_id: utterance.clone(),
        effective_text: "既存の確定本文".into(),
        speaker_ids: vec![],
    });
    let saved = storage
        .save_transcript_draft(&draft.id, 0, payload)
        .unwrap();
    drop(storage);
    let mut db = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
    remove_schema_for_test(&db);
    let old: (String, String, String) = db
        .query_row(
            "SELECT payload_json,inverse_json,after_hash FROM edit_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(verify_migration_history(&db).unwrap(), 27);
    apply_migrations(&mut db).unwrap();
    let new: (String, String, String) = db
        .query_row(
            "SELECT payload_json,inverse_json,after_hash FROM edit_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(old, new);
    drop(db);
    let storage = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        saved
    );
    assert!(storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .is_none());
    let restored = storage
        .undo_last_transcript_edit(&draft.id, saved.lock_version)
        .unwrap();
    assert_eq!(restored.content_hash, draft.content_hash);
    storage
        .save_transcript_edit_recovery(request(&restored, &utterance))
        .unwrap();
}

#[test]
fn competing_requests_at_same_revision_have_one_winner_and_keep_exact_value() {
    let root = tempfile::tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::open(root.path().into()).unwrap());
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let first = request(&draft, &utterance);
    let mut second = first.clone();
    second.operation_id = Uuid::new_v4().to_string();
    second.buffers[0].value = "競合する入力".into();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = [first, second]
        .into_iter()
        .map(|req| {
            let storage = storage.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                storage.save_transcript_edit_recovery(req)
            })
        })
        .collect();
    let results: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let read = storage
        .transcript_edit_recovery(&meeting, &draft.id, &base)
        .unwrap()
        .unwrap();
    assert_eq!(read.payload, winner.payload);
    assert_eq!(read.lock_version, 1);
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        draft
    );
}

#[test]
fn empty_text_and_exact_character_limit_are_recoverable_without_confirming_invalid_text() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    let mut req = request(&draft, &utterance);
    req.buffers[0].value.clear();
    let empty = storage.save_transcript_edit_recovery(req.clone()).unwrap();
    assert_eq!(empty.payload.buffers.len(), 1);
    assert_eq!(empty.payload.buffers[0].value, "");
    req.operation_id = Uuid::new_v4().to_string();
    req.expected_recovery_lock_version = 1;
    req.buffers[0].value = "あ".repeat(20_000);
    let saved = storage.save_transcript_edit_recovery(req).unwrap();
    assert_eq!(saved.payload.buffers[0].value.chars().count(), 20_000);
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        draft
    );
    let invalid = vec![saved.payload.buffers[0].clone(); 129];
    assert!(normalize(invalid).unwrap_err().contains("128"));
}

#[test]
fn verification_rejects_orphan_records_even_when_foreign_key_checks_were_bypassed() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base, utterance) = fixture(&storage);
    let draft = storage.create_transcript_draft(&meeting, &base).unwrap();
    storage
        .save_transcript_edit_recovery(request(&draft, &utterance))
        .unwrap();
    let db = storage.connection.lock().unwrap();
    assert!(db
        .execute("UPDATE transcript_edit_recovery SET draft_id=NULL", [])
        .is_err());
    db.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    db.execute("UPDATE transcript_edit_recovery SET draft_id='orphan'", [])
        .unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    assert!(verify_all(&db, &|| false).unwrap_err().contains("所有"));
}
