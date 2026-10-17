use super::*;
use std::cell::Cell;

fn entry(term: &str) -> VocabularyEntry {
    VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: term.into(),
        reading: None,
        note: None,
        enabled: true,
    }
}
fn storage() -> (tempfile::TempDir, Storage) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    (root, storage)
}
fn apply(
    plan: &GlobalMerge,
    target: &Storage,
    check: &impl Fn() -> bool,
) -> Result<MergeReceipt, String> {
    let mut db = target.connection.lock().unwrap();
    let mut tx = db.transaction().unwrap();
    let result = plan.apply(&mut tx, check);
    tx.commit().unwrap();
    result
}
fn plan(source: &Storage, target: &Storage) -> Result<GlobalMerge, String> {
    GlobalMerge::plan(
        &source.connection.lock().unwrap(),
        &target.connection.lock().unwrap(),
        &crate::test_support::never_cancelled,
    )
}

#[test]
fn restore_global_vocabulary_deduplicates_exact_content_keeps_conflicts_and_appends_new_terms() {
    let (_s, source) = storage();
    let (_t, target) = storage();
    let same = entry("点検日");
    let conflict = entry("MON");
    let existing = vec![same.clone(), conflict.clone(), entry("既存のみ")];
    target
        .save_vocabulary_book(None, 0, existing.clone())
        .unwrap();
    let old_rows = stored_entries(&target.connection.lock().unwrap(), "global").unwrap();
    let mut duplicate = same;
    duplicate.id = Uuid::new_v4().to_string();
    let mut changed = conflict.clone();
    changed.id = Uuid::new_v4().to_string();
    changed.reading = Some("もん".into());
    let incoming = vec![duplicate, changed.clone(), entry("追加用語")];
    source
        .save_vocabulary_book(None, 0, incoming.clone())
        .unwrap();
    let source_before = stored_entries(&source.connection.lock().unwrap(), "global").unwrap();
    let plan = plan(&source, &target).unwrap();
    assert_eq!(
        plan.receipt(),
        &MergeReceipt {
            added: 1,
            deduplicated: 1,
            conflicts: vec![VocabularyConflict {
                term: "MON".into(),
                existing_id: conflict.id,
                restored_id: changed.id
            }]
        }
    );
    let result = apply(&plan, &target, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(&result, plan.receipt());
    let book = target.vocabulary_book(None).unwrap();
    assert_eq!(book.lock_version, 2);
    assert_eq!(&book.payload[..existing.len()], existing);
    assert_eq!(book.payload.last().unwrap(), &incoming[2]);
    let rows = stored_entries(&target.connection.lock().unwrap(), "global").unwrap();
    assert_eq!(&rows[..old_rows.len()], old_rows);
    assert_eq!(rows.last().unwrap(), source_before.last().unwrap());
    assert_eq!(
        stored_entries(&source.connection.lock().unwrap(), "global").unwrap(),
        source_before
    );
}

#[test]
fn restore_global_vocabulary_compares_reading_note_and_enabled_without_normalizing_text() {
    for field in ["reading", "note", "enabled"] {
        let (_s, source) = storage();
        let (_t, target) = storage();
        let original = entry("MON");
        target
            .save_vocabulary_book(None, 0, vec![original.clone()])
            .unwrap();
        let mut changed = entry("MON");
        match field {
            "reading" => changed.reading = Some("読み".into()),
            "note" => changed.note = Some("補足".into()),
            "enabled" => changed.enabled = false,
            _ => unreachable!(),
        }
        source.save_vocabulary_book(None, 0, vec![changed]).unwrap();
        let plan = plan(&source, &target).unwrap();
        assert_eq!(plan.receipt().conflicts.len(), 1);
        let before = target.vocabulary_book(None).unwrap();
        apply(&plan, &target, &crate::test_support::never_cancelled).unwrap();
        let after = target.vocabulary_book(None).unwrap();
        assert_eq!(after.payload, before.payload);
        assert_eq!(after.lock_version, before.lock_version);
    }
}

#[test]
fn restore_global_vocabulary_empty_or_only_duplicates_never_creates_or_updates_scope() {
    let (_s, source) = storage();
    let (_t, target) = storage();
    apply(
        &plan(&source, &target).unwrap(),
        &target,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        target
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM vocabulary_scopes", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let original = entry("点検日");
    target
        .save_vocabulary_book(None, 0, vec![original.clone()])
        .unwrap();
    let before: (String, i64, i64) = target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT content_hash,lock_version,updated_at FROM vocabulary_scopes WHERE id='global'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let mut incoming = original;
    incoming.id = Uuid::new_v4().to_string();
    source
        .save_vocabulary_book(None, 0, vec![incoming])
        .unwrap();
    let plan = plan(&source, &target).unwrap();
    assert_eq!(plan.receipt().deduplicated, 1);
    apply(&plan, &target, &crate::test_support::never_cancelled).unwrap();
    apply(&plan, &target, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(target.connection.lock().unwrap().query_row("SELECT content_hash,lock_version,updated_at FROM vocabulary_scopes WHERE id='global'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap(),before);
}

#[test]
fn restore_global_vocabulary_cancellation_or_insert_failure_rolls_back_its_savepoint() {
    let (_s, source) = storage();
    source
        .save_vocabulary_book(None, 0, vec![entry("先頭"), entry("末尾")])
        .unwrap();
    for failure in ["cancel", "sql"] {
        let (_t, target) = storage();
        let plan = plan(&source, &target).unwrap();
        if failure == "sql" {
            target.connection.lock().unwrap().execute_batch("CREATE TRIGGER fixture_restore_fail BEFORE INSERT ON vocabulary_entries WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        }
        let calls = Cell::new(0);
        let result = apply(&plan, &target, &|| {
            calls.set(calls.get() + 1);
            failure == "cancel" && calls.get() == 3
        });
        assert!(result.is_err());
        let db = target.connection.lock().unwrap();
        for table in ["vocabulary_entries", "vocabulary_scopes"] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}

#[test]
fn restore_global_vocabulary_rechecks_destination_after_confirmation_and_rejects_id_collisions() {
    let (_s, source) = storage();
    let (_t, target) = storage();
    let a = entry("追加");
    source
        .save_vocabulary_book(None, 0, vec![a.clone()])
        .unwrap();
    let planned = plan(&source, &target).unwrap();
    target
        .save_vocabulary_book(None, 0, vec![entry("確認後の変更")])
        .unwrap();
    assert!(
        apply(&planned, &target, &crate::test_support::never_cancelled)
            .unwrap_err()
            .contains("確認後")
    );
    let local = target.create_meeting("別会議").unwrap();
    target
        .save_vocabulary_book(Some(&local.id), 0, vec![a])
        .unwrap();
    assert!(plan(&source, &target).err().unwrap().contains("ID"));
    assert_eq!(
        target.vocabulary_book(None).unwrap().payload[0].term,
        "確認後の変更"
    );
}

#[test]
fn restore_global_vocabulary_rejects_capacity_overflow_and_corrupted_books_without_partial_additions(
) {
    let (_s, source) = storage();
    let (_t, target) = storage();
    source
        .save_vocabulary_book(None, 0, vec![entry("追加")])
        .unwrap();
    let existing: Vec<_> = (0..500).map(|i| entry(&format!("既存{i}"))).collect();
    target
        .save_vocabulary_book(None, 0, existing.clone())
        .unwrap();
    assert!(plan(&source, &target).err().unwrap().contains("500件"));
    assert_eq!(target.vocabulary_book(None).unwrap().payload, existing);
    source
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE vocabulary_entries SET note='改変'", [])
        .unwrap();
    assert!(plan(&source, &target).is_err());
}

#[test]
fn restore_global_vocabulary_keeps_meeting_books_and_both_fixed_job_snapshots_unchanged() {
    let (_s, source) = storage();
    let (_t, target) = storage();
    fn enqueue(storage: &Storage, term: &str) -> (String, JobSnapshot) {
        let meeting = storage.create_meeting("固定条件").unwrap();
        storage
            .save_vocabulary_book(None, 0, vec![entry(term)])
            .unwrap();
        storage
            .save_vocabulary_book(Some(&meeting.id), 0, vec![entry("会議別")])
            .unwrap();
        let job = storage
            .enqueue_with_vocabulary(&meeting.id, "fixed", "", None, |snapshot| {
                Ok(JobSpec {
                    meeting_id: meeting.id.clone(),
                    kind: JobKind::Transcription,
                    operation_id: "fixed".into(),
                    input_analysis_id: None,
                    input_hash: "a".repeat(64),
                    profile_snapshot: serde_json::json!({"qualityStatus":"unverified"}),
                    parameters: serde_json::json!({"vocabulary":snapshot}),
                    total_units: Some(1),
                })
            })
            .unwrap();
        (meeting.id, job)
    }
    let (s_meeting, s_job) = enqueue(&source, "追加用語");
    let (t_meeting, t_job) = enqueue(&target, "既存用語");
    let s_before = source.fixed_job_input(&s_job.id).unwrap();
    let t_before = target.fixed_job_input(&t_job.id).unwrap();
    let s_terms = source.vocabulary_book(Some(&s_meeting)).unwrap();
    let t_terms = target.vocabulary_book(Some(&t_meeting)).unwrap();
    apply(
        &plan(&source, &target).unwrap(),
        &target,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(source.fixed_job_input(&s_job.id).unwrap(), s_before);
    assert_eq!(target.fixed_job_input(&t_job.id).unwrap(), t_before);
    assert_eq!(
        source.vocabulary_book(Some(&s_meeting)).unwrap().payload,
        s_terms.payload
    );
    assert_eq!(
        target.vocabulary_book(Some(&t_meeting)).unwrap().payload,
        t_terms.payload
    );
}
