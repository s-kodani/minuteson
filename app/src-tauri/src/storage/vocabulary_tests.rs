use super::*;

pub(crate) fn entry(term: &str) -> VocabularyEntry {
    VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: term.into(),
        reading: None,
        note: None,
        enabled: true,
    }
}
#[test]
fn vocabulary_lists_are_versioned_scoped_durable_and_preserve_row_identity() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("用語").unwrap();
    let other = storage.create_meeting("別会議").unwrap();
    let items = vec![entry("点検日"), entry("MON")];
    let common = storage
        .save_vocabulary_book(None, 0, items.clone())
        .unwrap();
    assert_eq!(common.lock_version, 1);
    assert!(storage
        .save_vocabulary_book(None, 0, items.clone())
        .is_err());
    assert_eq!(
        storage
            .save_vocabulary_book(None, 1, items.clone())
            .unwrap()
            .lock_version,
        1
    );
    assert!(storage
        .save_vocabulary_book(Some(&meeting.id), 0, items.clone())
        .is_err());
    let local = storage
        .save_vocabulary_book(Some(&meeting.id), 0, vec![entry("点検日")])
        .unwrap();
    assert_eq!(
        storage
            .vocabulary_book(Some(&other.id))
            .unwrap()
            .lock_version,
        0
    );
    assert!(storage.vocabulary_book(Some("missing")).is_err());
    let mut changed = items.clone();
    changed[0].reading = Some("てんけんび".into());
    let saved = storage
        .save_vocabulary_book(None, 1, changed.clone())
        .unwrap();
    let db = storage.connection.lock().unwrap();
    let versions: Vec<i64> = db
        .prepare(
            "SELECT lock_version FROM vocabulary_entries WHERE scope_id='global' ORDER BY position",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(versions, [2, 1]);
    drop(db);
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    let view = storage.vocabulary_view(&meeting.id).unwrap();
    assert_eq!(view.global.payload, changed);
    assert_eq!(view.global.lock_version, saved.lock_version);
    assert_eq!(view.meeting.payload, local.payload);
    let reordered = vec![
        view.global.payload[1].clone(),
        view.global.payload[0].clone(),
    ];
    assert_eq!(
        storage
            .save_vocabulary_book(None, 2, reordered.clone())
            .unwrap()
            .payload,
        reordered
    );
    assert_eq!(storage.vocabulary_book(None).unwrap().payload, reordered);
}
#[test]
fn vocabulary_rejects_duplicates_bad_input_tampering_and_atomic_write_failure() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let saved = storage
        .save_vocabulary_book(None, 0, vec![entry("点検日")])
        .unwrap();
    for term in ["", " 点検日", "点検日\n", "x\0", "x".repeat(201).as_str()] {
        assert!(storage
            .save_vocabulary_book(None, 1, vec![entry(term)])
            .is_err());
    }
    assert!(storage
        .save_vocabulary_book(None, 1, vec![entry("MON"), entry("MON")])
        .is_err());
    let mut bad = entry("MON");
    bad.reading = Some("".into());
    assert!(storage.save_vocabulary_book(None, 1, vec![bad]).is_err());
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER fixture_vocab_abort BEFORE INSERT ON vocabulary_entries BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(storage
        .save_vocabulary_book(None, 1, vec![entry("新用語")])
        .is_err());
    assert_eq!(
        storage.vocabulary_book(None).unwrap().payload,
        saved.payload
    );
    storage.connection.lock().unwrap().execute_batch("DROP TRIGGER fixture_vocab_abort; UPDATE vocabulary_entries SET note='tampered' WHERE scope_id='global';").unwrap();
    assert!(storage.vocabulary_book(None).is_err());
}

fn enqueue_terms(
    storage: &Storage,
    meeting: &str,
    operation: &str,
    extra: &str,
    stamp: Option<VocabularyStamp>,
) -> Result<JobSnapshot, String> {
    storage.enqueue_with_vocabulary(meeting, operation, extra, stamp, |snapshot| {
        Ok(JobSpec {
            meeting_id: meeting.into(),
            kind: JobKind::Transcription,
            operation_id: operation.into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({"qualityStatus":"unverified"}),
            parameters: serde_json::json!({"vocabulary":snapshot}),
            total_units: Some(1),
        })
    })
}

#[test]
fn vocabulary_snapshot_selection_is_immutable_idempotent_and_by_value() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("用語").unwrap().id;
    let other = storage.create_meeting("別会議").unwrap().id;
    let mut common = entry("点検日");
    common.reading = Some("てんけんび".into());
    let mut off = entry("無効");
    off.enabled = false;
    storage
        .save_vocabulary_book(None, 0, vec![common, entry("MON"), off])
        .unwrap();
    let mut local = entry("点検日");
    local.note = Some("会議別が優先".into());
    let mut disabled = entry("MON");
    disabled.enabled = false;
    storage
        .save_vocabulary_book(Some(&meeting), 0, vec![local.clone(), disabled])
        .unwrap();
    storage
        .save_vocabulary_book(Some(&other), 0, vec![entry("別会議だけ")])
        .unwrap();
    let stamp = VocabularyStamp {
        global_version: 1,
        meeting_version: 1,
    };
    let job = enqueue_terms(&storage, &meeting, "operation", "追加", Some(stamp.clone())).unwrap();
    let spec = storage.fixed_job_input(&job.id).unwrap().0;
    let snapshot: VocabularySnapshot =
        serde_json::from_value(spec.parameters["vocabulary"].clone()).unwrap();
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].entry, local);
    assert_eq!(snapshot.entries[0].scope, "meeting");
    assert_eq!(snapshot.entries[1].entry.term, "MON");
    assert_eq!(snapshot.entries[1].scope, "global");
    assert_eq!(snapshot.stamp, stamp);
    assert!(snapshot.actual_prompt.starts_with("追加\n"));
    assert!(snapshot.actual_prompt.contains("会議別が優先"));
    assert!(!snapshot.actual_prompt.contains("無効"));
    assert!(!snapshot.actual_prompt.contains("別会議だけ"));
    storage.save_vocabulary_book(None, 1, vec![]).unwrap();
    storage
        .save_vocabulary_book(Some(&meeting), 1, vec![])
        .unwrap();
    drop(storage);
    let storage = Storage::open(root.path().into()).unwrap();
    storage
        .verify_vocabulary_snapshot(&meeting, &job.id, &snapshot)
        .unwrap();
    assert_eq!(
        enqueue_terms(&storage, &meeting, "operation", "追加", Some(stamp))
            .unwrap()
            .id,
        job.id
    );
    assert!(enqueue_terms(&storage, &meeting, "operation", "変更", None).is_err());
    assert!(enqueue_terms(&storage, &other, "operation", "追加", None).is_err());
    let next = enqueue_terms(&storage, &meeting, "new", "", None).unwrap();
    let spec = storage.fixed_job_input(&next.id).unwrap().0;
    assert_eq!(
        spec.parameters["vocabulary"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(spec.parameters["vocabulary"]["actualPrompt"], "");
    assert_ne!(spec.parameters["vocabulary"]["id"], snapshot.id);
}

#[test]
fn vocabulary_enqueue_is_atomic_and_rejects_stale_or_oversized_hints() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let m = storage.create_meeting("用語").unwrap().id;
    storage
        .save_vocabulary_book(None, 0, vec![entry("点検日")])
        .unwrap();
    assert!(enqueue_terms(
        &storage,
        &m,
        "stale",
        "",
        Some(VocabularyStamp {
            global_version: 0,
            meeting_version: 0
        })
    )
    .is_err());
    assert!(enqueue_terms(&storage, &m, "long", &"x".repeat(8000), None).is_err());
    assert!(enqueue_terms(&storage, &m, "nul", "\0", None).is_err());
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER fixture_snapshot_abort BEFORE INSERT ON vocabulary_snapshots BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(enqueue_terms(&storage, &m, "abort", "", None).is_err());
    assert!(storage.list_jobs(&m).unwrap().is_empty());
    let db = storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM vocabulary_snapshots", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(db
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
    drop(db);
    assert_eq!(
        storage.vocabulary_book(None).unwrap().payload[0].term,
        "点検日"
    );
}

#[test]
fn vocabulary_snapshot_integrity_and_original_job_lineage_are_required() {
    for change in [
        "update", "delete", "hash", "payload", "owner", "meeting", "embedded",
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let m = storage.create_meeting("用語").unwrap().id;
        let job = enqueue_terms(&storage, &m, "first", "点検日", None).unwrap();
        let spec = storage.fixed_job_input(&job.id).unwrap().0;
        let mut snapshot: VocabularySnapshot =
            serde_json::from_value(spec.parameters["vocabulary"].clone()).unwrap();
        let mut check_job = job.id.clone();
        match change {
            "update" => {
                assert!(storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE vocabulary_snapshots SET content_hash=?1",
                        params!["f".repeat(64)]
                    )
                    .is_err());
                storage
                    .verify_vocabulary_snapshot(&m, &job.id, &snapshot)
                    .unwrap();
                continue;
            }
            "delete" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute("DELETE FROM vocabulary_snapshots", [])
                    .unwrap();
            }
            "hash" | "payload" => {
                let db = storage.connection.lock().unwrap();
                db.execute_batch("DROP TRIGGER vocabulary_snapshot_no_update;")
                    .unwrap();
                db.execute(
                    if change == "hash" {
                        "UPDATE vocabulary_snapshots SET content_hash=?1"
                    } else {
                        "UPDATE vocabulary_snapshots SET payload_json=?1"
                    },
                    params![if change == "hash" {
                        "f".repeat(64)
                    } else {
                        "{}".into()
                    }],
                )
                .unwrap();
            }
            "owner" => {
                let mut cloned = spec;
                cloned.operation_id = "unrelated".into();
                check_job = storage.enqueue_job(cloned).unwrap().id;
            }
            "meeting" => {
                snapshot.meeting_id = storage.create_meeting("別会議").unwrap().id;
            }
            "embedded" => {
                snapshot.actual_prompt = "changed".into();
            }
            _ => unreachable!(),
        }
        assert!(
            storage
                .verify_vocabulary_snapshot(&m, &check_job, &snapshot)
                .is_err(),
            "{change}"
        );
    }
}
