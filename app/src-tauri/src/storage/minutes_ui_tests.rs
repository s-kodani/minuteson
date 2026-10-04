use super::*;
use crate::minutes_document::{
    from_generated, Block, GeneratedItem, Inline, ItemKind, MinutesContent, SourceType,
};
use serde_json::json;

pub(in crate::storage) fn meeting(storage: &Storage) -> String {
    let m = super::super::analysis::tests::saved_meeting(storage);
    let mut input = super::super::analysis::tests::input();
    input.utterances[0].text = "点検日を確認することに決まりました。".into();
    let id = storage.create_analysis_candidate(&m.id, input).unwrap();
    storage
        .commit_analysis_candidate(&m.id, &id, 0, false)
        .unwrap();
    storage.create_transcript_draft(&m.id, &id).unwrap();
    m.id
}
pub(in crate::storage) fn candidate(storage: &Storage, meeting: &str) -> String {
    let analysis = storage.load_current_analysis(meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(meeting).unwrap().unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.into(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let job = storage
        .enqueue_minutes_job(
            request,
            json!({"engine":"fixture"}),
            json!({"templateVersion":"standard-v1"}),
            Some(1),
        )
        .unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    storage
        .save_job_checkpoint(&job.id, 0, 1, json!({"fixture":true}))
        .unwrap();
    let input = storage.load_minutes_input(&job.id).unwrap();
    let content = from_generated(
        &input,
        vec![GeneratedItem {
            kind: ItemKind::Decision,
            body: "点検日を確認する".into(),
            assignee_name: None,
            assignee_speaker_id: None,
            due_date: None,
            due_text: None,
            evidence_utterance_ids: vec![input.analysis.utterances[0].id.clone()],
        }],
    )
    .unwrap();
    storage
        .finish_minutes_candidate(&job.id, 1, content)
        .unwrap()
}
pub(in crate::storage) fn request(
    storage: &Storage,
    meeting: &str,
    id: &str,
) -> MinutesAdoptRequest {
    let state = storage.minutes_versions(meeting).unwrap();
    let view = storage.load_minutes_content(meeting, id).unwrap().unwrap();
    MinutesAdoptRequest {
        meeting_id: meeting.into(),
        revision_id: id.into(),
        expected_content_hash: view.content_hash,
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: state.draft,
        confirm_replacing_previous: state.previous_id.is_some(),
    }
}
fn append(mut content: MinutesContent, text: &str) -> MinutesContent {
    content.blocks.push(Block::Paragraph {
        item_id: None,
        content: vec![Inline::text(text)],
    });
    content
}
fn stamp(draft: &MinutesDraft) -> Option<DraftStamp> {
    Some(DraftStamp {
        id: draft.id.clone(),
        lock_version: draft.lock_version,
    })
}

#[test]
fn long_document_drafts_save_export_and_undo_every_row_without_a_display_page_limit() {
    use crate::minutes_document::{TableRow, TextBlock};
    let root = tempfile::tempdir().unwrap();
    let mut s = Storage::open(root.path().into()).unwrap();
    let m = meeting(&s);
    let a = candidate(&s, &m);
    s.adopt_minutes(request(&s, &m, &a)).unwrap();
    let original = s.load_minutes_content(&m, &a).unwrap().unwrap();
    let initial = s.create_minutes_draft(&m, &a).unwrap();
    let mut payload = initial.payload.clone();
    payload.blocks.push(Block::List {
        ordered: true,
        items: (1..=121)
            .map(|i| TextBlock {
                item_id: None,
                content: vec![Inline::text(format!("長い項目{i}"))],
            })
            .collect(),
    });
    payload.blocks.push(Block::Table {
        headers: vec![vec![Inline::text("列1")], vec![Inline::text("列2")]],
        rows: (1..=125)
            .map(|i| TableRow {
                item_id: None,
                cells: vec![
                    vec![Inline::text(format!("長い行{i}"))],
                    vec![Inline::text("メモ")],
                ],
            })
            .collect(),
    });
    let long = s
        .save_minutes_draft(&initial.id, 0, payload.clone())
        .unwrap();
    let list_index = payload.blocks.len() - 2;
    let table_index = payload.blocks.len() - 1;
    if let Block::List { items, .. } = &mut payload.blocks[list_index] {
        items[120].content = vec![Inline::text("末尾項目を修正")];
    }
    if let Block::Table { rows, .. } = &mut payload.blocks[table_index] {
        rows[124].cells[1] = vec![Inline::text("末尾セルを修正")];
    }
    let edited = s
        .save_minutes_draft(&long.id, long.lock_version, payload.clone())
        .unwrap();
    drop(s);
    s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.load_minutes_draft(&m, None).unwrap().unwrap(), edited);
    let state = s.minutes_versions(&m).unwrap();
    let export = s
        .prepare_export(super::super::exports::ExportRequest {
            operation_id: Uuid::new_v4().to_string(),
            meeting_id: m.clone(),
            expected_minutes_id: a.clone(),
            expected_content_hash: original.content_hash.clone(),
            expected_meeting_lock_version: state.meeting_lock_version,
            expected_draft: state.draft,
        })
        .unwrap();
    for i in 1..=120 {
        assert!(export.markdown.contains(&format!("{i}. 長い項目{i}\n")));
    }
    for i in 1..=125 {
        assert!(export.markdown.contains(&format!("| 長い行{i} |")));
    }
    assert!(export.markdown.contains("121. 末尾項目を修正\n"));
    assert!(export.markdown.contains("| 長い行125 | 末尾セルを修正 |"));
    assert_eq!(
        s.undo_minutes_edit(&edited.id, edited.lock_version)
            .unwrap()
            .payload,
        long.payload
    );
    assert_eq!(s.load_minutes_content(&m, &a).unwrap().unwrap(), original);
}

#[test]
fn explicit_evidence_verification_is_durable_undoable_and_keeps_the_generated_base_frozen() {
    use crate::minutes_document::VerificationState;
    let root = tempfile::tempdir().unwrap();
    let mut s = Storage::open(root.path().into()).unwrap();
    let m = meeting(&s);
    let a = candidate(&s, &m);
    s.adopt_minutes(request(&s, &m, &a)).unwrap();
    let original = s.load_minutes_content(&m, &a).unwrap().unwrap();
    let initial = s.create_minutes_draft(&m, &a).unwrap();
    let mut payload = initial.payload.clone();
    payload.evidence[0].verification_state = VerificationState::UserVerified;
    let checked = s.save_minutes_draft(&initial.id, 0, payload).unwrap();
    assert_eq!(checked.payload.items, initial.payload.items);
    assert_eq!(checked.payload.blocks, initial.payload.blocks);
    assert_eq!(checked.payload.evidence[0].source_type, SourceType::Engine);
    assert_eq!(s.load_minutes_content(&m, &a).unwrap().unwrap(), original);
    let reference = MinutesEvidenceRequest {
        meeting_id: m.clone(),
        minutes_id: a.clone(),
        expected_content_hash: original.content_hash.clone(),
        expected_input_content_hash: original.input_content_hash.clone(),
        expected_evidence: checked.payload.evidence[0].clone(),
    };
    assert_eq!(
        s.minutes_evidence_source(&reference)
            .unwrap()
            .evidence
            .verification_state,
        VerificationState::UserVerified
    );
    assert_eq!(
        s.save_minutes_draft(&checked.id, 1, checked.payload.clone())
            .unwrap(),
        checked
    );
    assert!(s
        .save_minutes_draft(&checked.id, 0, initial.payload.clone())
        .is_err());
    let mut wrong_anchor = checked.payload.clone();
    wrong_anchor.evidence[0].start_us += 1;
    assert!(s.save_minutes_draft(&checked.id, 1, wrong_anchor).is_err());
    assert_eq!(s.load_minutes_draft(&m, None).unwrap().unwrap(), checked);
    let mut payload = checked.payload.clone();
    payload.evidence[0].verification_state = VerificationState::Unverified;
    let cleared = s.save_minutes_draft(&checked.id, 1, payload).unwrap();
    drop(s);
    s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.load_minutes_draft(&m, None).unwrap().unwrap(), cleared);
    let undone = s
        .undo_minutes_edit(&cleared.id, cleared.lock_version)
        .unwrap();
    assert_eq!(undone.payload, checked.payload);
    let b = candidate(&s, &m);
    s.adopt_minutes(request(&s, &m, &b)).unwrap();
    assert!(s
        .save_minutes_draft(&undone.id, undone.lock_version, initial.payload)
        .is_err());
    let restore = request(&s, &m, &a);
    s.restore_minutes_checked(
        &m,
        restore.expected_meeting_lock_version,
        &a,
        &restore.expected_content_hash,
        restore.expected_draft,
    )
    .unwrap();
    assert_eq!(s.load_minutes_draft(&m, None).unwrap().unwrap(), undone);
    assert_eq!(s.load_minutes_content(&m, &a).unwrap().unwrap(), original);
}

#[test]
fn multiple_saved_references_keep_independent_verification_and_resolve_their_own_quotes() {
    use crate::minutes_document::VerificationState;
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let m = meeting(&s);
    let a = candidate(&s, &m);
    s.adopt_minutes(request(&s, &m, &a)).unwrap();
    let original = s.load_minutes_content(&m, &a).unwrap().unwrap();
    let initial = s.create_minutes_draft(&m, &a).unwrap();
    let mut payload = initial.payload.clone();
    payload.items[0].source_type = SourceType::Manual;
    payload.evidence[0].source_type = SourceType::Manual;
    let mut second = payload.evidence[0].clone();
    second.id = Uuid::new_v4().to_string();
    second.quote_text = "点検日".into();
    for b in &mut payload.blocks {
        if let Block::List { items, .. } = b {
            if let Some(row) = items
                .iter_mut()
                .find(|row| row.item_id.as_ref() == Some(&second.minutes_item_id))
            {
                row.content.push(Inline::Evidence {
                    minutes_item_id: second.minutes_item_id.clone(),
                    evidence_link_id: second.id.clone(),
                });
            }
        }
    }
    payload.evidence.push(second);
    let multiple = s.save_minutes_draft(&initial.id, 0, payload).unwrap();
    let mut payload = multiple.payload;
    payload.evidence[0].verification_state = VerificationState::UserVerified;
    let checked = s.save_minutes_draft(&multiple.id, 1, payload).unwrap();
    for e in &checked.payload.evidence {
        let r = MinutesEvidenceRequest {
            meeting_id: m.clone(),
            minutes_id: a.clone(),
            expected_content_hash: original.content_hash.clone(),
            expected_input_content_hash: original.input_content_hash.clone(),
            expected_evidence: e.clone(),
        };
        assert_eq!(s.minutes_evidence_source(&r).unwrap().evidence, *e);
    }
    assert_eq!(
        checked.payload.evidence[0].verification_state,
        VerificationState::UserVerified
    );
    assert_eq!(
        checked.payload.evidence[1].verification_state,
        VerificationState::Unverified
    );
    assert_ne!(
        checked.payload.evidence[0].quote_text,
        checked.payload.evidence[1].quote_text
    );
    let undone = s
        .undo_minutes_edit(&checked.id, checked.lock_version)
        .unwrap();
    assert!(undone
        .payload
        .evidence
        .iter()
        .all(|e| e.verification_state == VerificationState::Unverified));
    assert_eq!(undone.payload.evidence.len(), 2);
}

#[test]
fn minutes_adoption_restore_and_multi_undo_preserve_each_base_and_generated_originals() {
    let root = tempfile::tempdir().unwrap();
    let mut storage = Storage::open(root.path().into()).unwrap();
    let m = meeting(&storage);
    let a = candidate(&storage, &m);
    storage.adopt_minutes(request(&storage, &m, &a)).unwrap();
    let a_original = storage.load_minutes_content(&m, &a).unwrap().unwrap();
    let initial = storage.create_minutes_draft(&m, &a).unwrap();
    let d1 = storage
        .save_minutes_draft(
            &initial.id,
            0,
            append(initial.payload.clone(), "手動で追記"),
        )
        .unwrap();
    let d2 = storage
        .save_minutes_draft(&d1.id, d1.lock_version, append(d1.payload.clone(), "追記2"))
        .unwrap();
    let noop = storage
        .save_minutes_draft(&d2.id, d2.lock_version, d2.payload.clone())
        .unwrap();
    assert_eq!(noop, d2);
    assert_eq!(
        storage
            .undo_minutes_edit(&d2.id, d2.lock_version)
            .unwrap()
            .payload,
        d1.payload
    );
    let undone = storage
        .undo_minutes_edit(&d2.id, d2.lock_version + 1)
        .unwrap();
    assert_eq!(undone.payload, initial.payload);
    assert!(storage
        .undo_minutes_edit(&undone.id, undone.lock_version)
        .is_err());
    let mut a_edited = storage
        .save_minutes_draft(
            &undone.id,
            undone.lock_version,
            append(undone.payload, "新しい追記"),
        )
        .unwrap();
    let undo_branch = storage
        .undo_minutes_edit(&a_edited.id, a_edited.lock_version)
        .unwrap();
    assert_eq!(undo_branch.payload, initial.payload);
    assert!(storage
        .undo_minutes_edit(&undo_branch.id, undo_branch.lock_version)
        .is_err());
    a_edited = storage
        .save_minutes_draft(&undo_branch.id, undo_branch.lock_version, a_edited.payload)
        .unwrap();
    let b = candidate(&storage, &m);
    storage.adopt_minutes(request(&storage, &m, &b)).unwrap();
    assert!(storage.load_minutes_draft(&m, None).unwrap().is_none());
    assert_eq!(
        storage.load_minutes_draft(&m, Some(&a)).unwrap(),
        Some(a_edited.clone())
    );
    let b_draft = storage.create_minutes_draft(&m, &b).unwrap();
    let b_edited = storage
        .save_minutes_draft(&b_draft.id, 0, append(b_draft.payload.clone(), "B版の修正"))
        .unwrap();
    assert!(storage
        .save_minutes_draft(
            &a_edited.id,
            a_edited.lock_version,
            a_edited.payload.clone()
        )
        .is_err());
    let state = storage.minutes_versions(&m).unwrap();
    storage
        .restore_minutes_checked(
            &m,
            state.meeting_lock_version,
            &a,
            &a_original.content_hash,
            stamp(&b_edited),
        )
        .unwrap();
    assert_eq!(
        storage.load_minutes_draft(&m, None).unwrap(),
        Some(a_edited.clone())
    );
    let state = storage.minutes_versions(&m).unwrap();
    let b_view = storage.load_minutes_content(&m, &b).unwrap().unwrap();
    storage
        .restore_minutes_checked(
            &m,
            state.meeting_lock_version,
            &b,
            &b_view.content_hash,
            stamp(&a_edited),
        )
        .unwrap();
    assert_eq!(
        storage.load_minutes_draft(&m, None).unwrap(),
        Some(b_edited.clone())
    );
    assert_eq!(
        storage.load_minutes_content(&m, &a).unwrap().unwrap(),
        a_original
    );
    let c = candidate(&storage, &m);
    let mut no_confirm = request(&storage, &m, &c);
    no_confirm.confirm_replacing_previous = false;
    assert!(storage.adopt_minutes(no_confirm).is_err());
    storage.adopt_minutes(request(&storage, &m, &c)).unwrap();
    assert!(storage.load_minutes_content(&m, &a).unwrap().is_none());
    assert!(storage.load_minutes_draft(&m, Some(&a)).is_err());
    assert!(storage
        .minutes_versions(&m)
        .unwrap()
        .versions
        .iter()
        .all(|v| v.id != a));
    drop(storage);
    storage = Storage::open(root.path().into()).unwrap();
    let state = storage.minutes_versions(&m).unwrap();
    assert_eq!(state.current_id, Some(c));
    assert_eq!(state.previous_id, Some(b.clone()));
    assert_eq!(
        storage.load_minutes_draft(&m, Some(&b)).unwrap(),
        Some(b_edited)
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM working_drafts WHERE meeting_id=?1 AND target_kind='minutes'",
                params![m],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn stale_preview_draft_parent_and_wrong_scope_cannot_switch_minutes() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let m = meeting(&storage);
    let a = candidate(&storage, &m);
    let mut bad = request(&storage, &m, &a);
    bad.expected_content_hash = "0".repeat(64);
    assert!(storage.adopt_minutes(bad).is_err());
    let mut bad = request(&storage, &m, &a);
    bad.meeting_id = storage.create_meeting("別会議").unwrap().id;
    assert!(storage.adopt_minutes(bad).is_err());
    storage.adopt_minutes(request(&storage, &m, &a)).unwrap();
    let draft = storage.create_minutes_draft(&m, &a).unwrap();
    let b = candidate(&storage, &m);
    let c = candidate(&storage, &m);
    let stale = request(&storage, &m, &b);
    let edited = storage
        .save_minutes_draft(&draft.id, 0, append(draft.payload.clone(), "確認後に編集"))
        .unwrap();
    assert!(storage.adopt_minutes(stale).is_err());
    assert_eq!(
        storage.minutes_versions(&m).unwrap().current_id,
        Some(a.clone())
    );
    assert_eq!(storage.load_minutes_draft(&m, None).unwrap(), Some(edited));
    let stale_parent = request(&storage, &m, &c);
    storage.adopt_minutes(request(&storage, &m, &b)).unwrap();
    assert!(storage.adopt_minutes(stale_parent).is_err());
    // Even a new meeting stamp cannot make a candidate's stale parent current.
    assert!(storage.adopt_minutes(request(&storage, &m, &c)).is_err());
    let state = storage.minutes_versions(&m).unwrap();
    assert!(storage
        .restore_minutes_checked(&m, state.meeting_lock_version, &b, "0", None)
        .is_err());
    assert_eq!(storage.minutes_versions(&m).unwrap().current_id, Some(b));
}

#[test]
fn draft_rejects_changed_evidence_engine_claims_stale_saves_and_tampered_history() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let m = meeting(&storage);
    let a = candidate(&storage, &m);
    storage.adopt_minutes(request(&storage, &m, &a)).unwrap();
    let d = storage.create_minutes_draft(&m, &a).unwrap();
    let mut bad = d.payload.clone();
    bad.evidence[0].start_us += 1000;
    assert!(storage.save_minutes_draft(&d.id, 0, bad).is_err());
    let mut bad = d.payload.clone();
    bad.evidence[0].quote_text = "点検日".into();
    assert!(storage.save_minutes_draft(&d.id, 0, bad).is_err());
    let mut changed = d.payload.clone();
    changed.items[0].body = "点検日を再確認する".into();
    for block in &mut changed.blocks {
        if let Block::List { items, .. } = block {
            for item in items {
                if item.item_id.as_deref() == Some(&changed.items[0].id) {
                    item.content[0] = Inline::text("点検日を再確認する");
                }
            }
        }
    }
    assert!(storage
        .save_minutes_draft(&d.id, 0, changed.clone())
        .is_err());
    changed.items[0].source_type = SourceType::Manual;
    for e in &mut changed.evidence {
        e.source_type = SourceType::Manual;
    }
    let corrected = storage.save_minutes_draft(&d.id, 0, changed).unwrap();
    assert!(storage
        .save_minutes_draft(&d.id, 0, corrected.payload.clone())
        .is_err());
    assert!(storage.undo_minutes_edit(&d.id, 0).is_err());
    let db = storage.connection.lock().unwrap();
    db.execute(
        "UPDATE edit_operations SET inverse_json='{}' WHERE draft_id=?1",
        params![d.id],
    )
    .unwrap();
    drop(db);
    assert!(storage
        .undo_minutes_edit(&d.id, corrected.lock_version)
        .is_err());
    assert_eq!(
        storage.load_minutes_draft(&m, None).unwrap(),
        Some(corrected)
    );
    let db = storage.connection.lock().unwrap();
    db.execute(
        "UPDATE working_drafts SET payload_json='{}' WHERE id=?1",
        params![d.id],
    )
    .unwrap();
    drop(db);
    assert!(storage.load_minutes_draft(&m, None).is_err());
}

#[test]
fn adoption_rolls_back_draft_switch_and_input_update_does_not_rewrite_fixed_quotes() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let m = meeting(&storage);
    let a = candidate(&storage, &m);
    storage.adopt_minutes(request(&storage, &m, &a)).unwrap();
    let d = storage.create_minutes_draft(&m, &a).unwrap();
    let b = candidate(&storage, &m);
    let preview = storage.load_minutes_content(&m, &b).unwrap().unwrap();
    let mut td = storage.load_transcript_draft(&m).unwrap().unwrap();
    let current = storage.load_current_analysis(&m).unwrap().unwrap();
    td.payload
        .utterances
        .push(super::super::drafts::UtteranceEdit {
            utterance_id: current.utterances[0].id.clone(),
            effective_text: "生成後に点検日を修正".into(),
            speaker_ids: vec![],
        });
    storage
        .save_transcript_draft(&td.id, td.lock_version, td.payload)
        .unwrap();
    assert!(storage
        .minutes_versions(&m)
        .unwrap()
        .versions
        .iter()
        .all(|v| v.input_updated));
    assert_eq!(
        storage.load_minutes_content(&m, &b).unwrap().unwrap(),
        preview
    );
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_minutes_switch BEFORE UPDATE OF current_minutes_id ON meetings BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(storage.adopt_minutes(request(&storage, &m, &b)).is_err());
    assert_eq!(
        storage.load_minutes_draft(&m, None).unwrap(),
        Some(d.clone())
    );
    assert_eq!(storage.minutes_versions(&m).unwrap().current_id, Some(a));
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT publication_state FROM minutes_revisions WHERE id=?1",
                params![b],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "candidate"
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_minutes_switch;")
        .unwrap();
    storage.adopt_minutes(request(&storage, &m, &b)).unwrap();
    assert!(storage.load_minutes_draft(&m, None).unwrap().is_none());
    assert_eq!(
        storage.load_minutes_content(&m, &b).unwrap().unwrap(),
        preview
    );
}

#[test]
fn failed_draft_update_rolls_back_the_document_and_inverse_operation_together() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let m = meeting(&storage);
    let a = candidate(&storage, &m);
    storage.adopt_minutes(request(&storage, &m, &a)).unwrap();
    let d = storage.create_minutes_draft(&m, &a).unwrap();
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_minutes_draft BEFORE UPDATE OF payload_json ON working_drafts WHEN NEW.target_kind='minutes' BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(storage
        .save_minutes_draft(&d.id, 0, append(d.payload.clone(), "保存に失敗する変更"))
        .is_err());
    assert_eq!(
        storage.load_minutes_draft(&m, None).unwrap(),
        Some(d.clone())
    );
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM edit_operations WHERE draft_id=?1",
                params![d.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_minutes_draft;")
        .unwrap();
    let saved = storage
        .save_minutes_draft(&d.id, 0, append(d.payload.clone(), "保存できる変更"))
        .unwrap();
    assert_eq!(
        storage
            .undo_minutes_edit(&d.id, saved.lock_version)
            .unwrap()
            .payload,
        d.payload
    );
}
