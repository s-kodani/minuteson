use super::super::analysis::tests::{input, saved_meeting};
use super::*;

fn request(storage: &Storage, meeting: &str, candidate: &str) -> AdoptRequest {
    let state = storage.analysis_versions(meeting).unwrap();
    AdoptRequest {
        meeting_id: meeting.into(),
        revision_id: candidate.into(),
        expected_content_hash: state
            .versions
            .iter()
            .find(|version| version.id == candidate)
            .unwrap()
            .content_hash
            .clone(),
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: state.draft,
        confirm_replacing_previous: true,
    }
}

#[test]
fn adoption_and_restore_preserve_each_base_draft_and_undo_chain() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let meeting = saved_meeting(&storage);
    let first = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    storage
        .adopt_analysis(request(&storage, &meeting.id, &first))
        .unwrap();
    let draft = storage
        .create_transcript_draft(&meeting.id, &first)
        .unwrap();
    let view = storage.load_current_analysis(&meeting.id).unwrap().unwrap();
    let payload = TranscriptDraftPayload {
        schema_version: 1,
        speaker_names: vec![],
        utterances: vec![super::super::drafts::UtteranceEdit {
            utterance_id: view.utterances[0].id.clone(),
            effective_text: "手動修正を保持".into(),
            speaker_ids: vec![],
        }],
    };
    let edited = storage
        .save_transcript_draft(&draft.id, 0, payload)
        .unwrap();
    let second = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    storage
        .adopt_analysis(request(&storage, &meeting.id, &second))
        .unwrap();
    assert!(storage
        .load_transcript_draft(&meeting.id)
        .unwrap()
        .is_none());
    assert_eq!(
        storage.visible_analysis_draft(&meeting.id, &first).unwrap(),
        Some(edited.clone())
    );
    assert!(storage
        .save_transcript_draft(&edited.id, 1, edited.payload.clone())
        .is_err());
    let next = storage
        .create_transcript_draft(&meeting.id, &second)
        .unwrap();
    let state = storage.analysis_versions(&meeting.id).unwrap();
    let previous = state
        .versions
        .iter()
        .find(|v| Some(&v.id) == state.previous_id.as_ref())
        .unwrap();
    assert!(storage
        .restore_analysis_checked(
            &meeting.id,
            state.meeting_lock_version,
            "wrong-id",
            &previous.content_hash,
            state.draft.clone()
        )
        .is_err());
    assert!(storage
        .restore_analysis_checked(
            &meeting.id,
            state.meeting_lock_version,
            &previous.id,
            &"f".repeat(64),
            state.draft.clone()
        )
        .is_err());
    assert_eq!(
        storage.analysis_versions(&meeting.id).unwrap().current_id,
        Some(second.clone())
    );
    storage
        .restore_analysis_checked(
            &meeting.id,
            state.meeting_lock_version,
            &previous.id,
            &previous.content_hash,
            state.draft,
        )
        .unwrap();
    assert_eq!(
        storage.load_transcript_draft(&meeting.id).unwrap(),
        Some(edited.clone())
    );
    let undone = storage.undo_last_transcript_edit(&edited.id, 1).unwrap();
    assert_eq!(undone.payload, draft.payload);
    assert_eq!(
        storage
            .visible_analysis_draft(&meeting.id, &second)
            .unwrap(),
        Some(next)
    );
    let state = storage.analysis_versions(&meeting.id).unwrap();
    let previous = state
        .versions
        .iter()
        .find(|v| Some(&v.id) == state.previous_id.as_ref())
        .unwrap();
    storage
        .restore_analysis_checked(
            &meeting.id,
            state.meeting_lock_version,
            &previous.id,
            &previous.content_hash,
            state.draft,
        )
        .unwrap();
    drop(storage);
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    assert_eq!(
        storage
            .visible_analysis_draft(&meeting.id, &first)
            .unwrap()
            .unwrap(),
        undone
    );
}

#[test]
fn stale_preview_stale_draft_and_wrong_meeting_leave_heads_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let meeting = saved_meeting(&storage);
    let first = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    storage
        .adopt_analysis(request(&storage, &meeting.id, &first))
        .unwrap();
    let candidate = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    let stale = request(&storage, &meeting.id, &candidate);
    storage
        .create_transcript_draft(&meeting.id, &first)
        .unwrap();
    assert!(storage.adopt_analysis(stale).is_err());
    let mut wrong = request(&storage, &meeting.id, &candidate);
    wrong.expected_content_hash = "f".repeat(64);
    assert!(storage.adopt_analysis(wrong).is_err());
    assert!(storage
        .load_visible_analysis("other", &candidate)
        .unwrap()
        .is_none());
    let stale = request(&storage, &meeting.id, &candidate);
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE utterances SET effective_text = '候補を改変' WHERE analysis_revision_id = ?1",
            params![candidate],
        )
        .unwrap();
    assert!(storage
        .load_visible_analysis(&meeting.id, &candidate)
        .is_err());
    assert!(storage.adopt_analysis(stale).is_err());
    assert_eq!(
        storage
            .load_current_analysis(&meeting.id)
            .unwrap()
            .unwrap()
            .id,
        first
    );
}

#[test]
fn candidate_from_old_head_cannot_overwrite_new_adoption() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let meeting = saved_meeting(&storage);
    let first = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    let stale = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    storage
        .adopt_analysis(request(&storage, &meeting.id, &first))
        .unwrap();
    assert!(storage
        .adopt_analysis(request(&storage, &meeting.id, &stale))
        .is_err());
    let second = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    storage
        .adopt_analysis(request(&storage, &meeting.id, &second))
        .unwrap();
    let third = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    let mut needs_confirmation = request(&storage, &meeting.id, &third);
    needs_confirmation.confirm_replacing_previous = false;
    assert!(storage.adopt_analysis(needs_confirmation).is_err());
    storage
        .adopt_analysis(request(&storage, &meeting.id, &third))
        .unwrap();
    assert!(storage
        .load_visible_analysis(&meeting.id, &first)
        .unwrap()
        .is_none());
    assert!(storage.visible_analysis_draft(&meeting.id, &first).is_err());
}

#[test]
fn migration_from_v9_preserves_draft_ids_payload_and_edit_records() {
    let directory = tempfile::tempdir().unwrap();
    let db = Connection::open(directory.path().join("meetings.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
    for (index, sql) in [
        MIGRATION_1,
        MIGRATION_2,
        MIGRATION_3,
        MIGRATION_4,
        MIGRATION_5,
        MIGRATION_6,
        MIGRATION_7,
        MIGRATION_8,
        MIGRATION_9,
    ]
    .iter()
    .enumerate()
    {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES (?1, ?2, 0)",
            params![
                index as i64 + 1,
                format!("{:x}", Sha256::digest(sql.as_bytes()))
            ],
        )
        .unwrap();
    }
    db.execute("INSERT INTO meetings (id, title, lifecycle, created_at, updated_at) VALUES ('meeting', '旧データ', 'draft', 0, 0)", []).unwrap();
    db.execute("INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES ('analysis', 'meeting', 1, 'engine', 'committed', ?1, '{}', '{}', ?1, 1, 0)", params!["a".repeat(64)]).unwrap();
    db.execute(
        "UPDATE meetings SET current_analysis_id = 'analysis' WHERE id = 'meeting'",
        [],
    )
    .unwrap();
    let json = "{\"schema_version\":1,\"speaker_names\":[],\"utterances\":[]}";
    let mut hasher = Sha256::new();
    hasher.update(b"mon-transcript-draft-v1");
    hasher.update(json.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    db.execute("INSERT INTO working_drafts (id, meeting_id, target_kind, base_analysis_id, payload_json, schema_version, content_hash, lock_version, created_at, updated_at) VALUES ('draft', 'meeting', 'transcript', 'analysis', ?1, 1, ?2, 1, 0, 0)", params![json, hash]).unwrap();
    db.execute("INSERT INTO edit_operations VALUES ('edit', 'draft', 1, 'replace_payload', ?1, ?1, ?2, ?2, 0)", params![json, hash]).unwrap();
    drop(db);
    let storage = Storage::open(directory.path().to_owned()).unwrap();
    let draft = storage.load_transcript_draft("meeting").unwrap().unwrap();
    assert_eq!(draft.id, "draft");
    assert_eq!(draft.lock_version, 1);
    assert_eq!(draft.content_hash, hash);
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM edit_operations WHERE draft_id = 'draft'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "edit"
    );
    assert!(storage
        .connection
        .lock()
        .unwrap()
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    storage.undo_last_transcript_edit("draft", 1).unwrap();
}
