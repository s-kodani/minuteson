use super::*;
use crate::minutes_document::{Block, Inline};

fn generation_source(storage: &Storage) -> super::super::minutes_input::MinutesJobRequest {
    let meeting = super::super::analysis::tests::saved_meeting(storage);
    let analysis = storage
        .create_analysis_candidate(&meeting.id, super::super::analysis::tests::input())
        .unwrap();
    let current = storage
        .commit_analysis_candidate(&meeting.id, &analysis, 0, false)
        .unwrap();
    super::super::minutes_input::MinutesJobRequest {
        meeting_id: meeting.id,
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis,
        expected_content_hash: current.content_hash,
        expected_meeting_lock_version: current.meeting_lock_version,
        expected_draft: None,
    }
}
fn generation_job(
    storage: &Storage,
    source: &super::super::minutes_input::MinutesJobRequest,
    expected: Option<MarkdownStamp>,
) -> JobSnapshot {
    storage
        .enqueue_markdown_minutes_job_configured(
            source.clone(),
            serde_json::json!({"engine":"synthetic-markdown"}),
            GenerationTarget {
                schema_version: 1,
                expected,
            },
            |_| Ok((serde_json::json!({"format":"markdown"}), Some(1))),
        )
        .unwrap()
}
fn completed_generation(
    storage: &Storage,
    source: &super::super::minutes_input::MinutesJobRequest,
    expected: Option<MarkdownStamp>,
    text: &str,
) -> JobSnapshot {
    let job = generation_job(storage, source, expected);
    assert_eq!(
        storage
            .acquire_job_kind(Some(JobKind::MinutesGeneration))
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    storage
        .save_job_checkpoint(
            &job.id,
            0,
            1,
            serde_json::json!({"markdownHash":content_hash(text)}),
        )
        .unwrap();
    storage.get_job(&job.id).unwrap()
}

#[test]
fn generation_publishes_exact_markdown_and_success_without_candidates_evidence_or_backups() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let mut source = generation_source(&storage);
    let first = " \n# 初回\n- 決定事項\n[区間](audio:00:00:00.000001-00:00:00.999999)\n";
    let job = completed_generation(&storage, &source, None, first);
    let published = storage
        .finish_minutes_markdown(&job.id, 1, first.into())
        .unwrap();
    assert_eq!(published.markdown, first);
    assert_eq!(published.operation_id, source.operation_id);
    let succeeded = storage.get_job(&job.id).unwrap();
    assert_eq!(succeeded.state, "succeeded");
    assert_eq!(succeeded.output_minutes_id, None);
    let human = storage
        .save_minutes_markdown(request(
            &source.meeting_id,
            Some(published.stamp()),
            "人間が編集した本文",
        ))
        .unwrap();
    source.operation_id = Uuid::new_v4().to_string();
    let job = completed_generation(&storage, &source, Some(human.stamp()), "# 明示再生成\n");
    let next = storage
        .finish_minutes_markdown(&job.id, 1, "# 明示再生成\n".into())
        .unwrap();
    assert_eq!(next.lock_version, 3);
    assert_eq!(
        storage.get_minutes_markdown(&source.meeting_id).unwrap(),
        Some(next.clone())
    );
    assert!(storage
        .finish_minutes_markdown(&job.id, 1, "# 明示再生成\n".into())
        .is_err());
    assert_eq!(
        generation_job(&storage, &source, Some(human.stamp())).id,
        job.id
    ); // lost admission response is read back
    let db = storage.connection.lock().unwrap();
    for table in ["minutes_revisions", "minutes_items", "evidence_links"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM working_drafts WHERE target_kind='minutes'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id WHERE d.target_kind='minutes'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM minutes_markdown", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    drop(storage);
    assert_eq!(
        Storage::open(root.path().into())
            .unwrap()
            .get_minutes_markdown(&source.meeting_id)
            .unwrap(),
        Some(next)
    );
}

#[test]
fn human_save_during_generation_defeats_publication_without_failing_or_replacing_the_human_body() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let source = generation_source(&storage);
    let base = storage
        .save_minutes_markdown(request(&source.meeting_id, None, "生成前"))
        .unwrap();
    let job = completed_generation(&storage, &source, Some(base.stamp()), "生成後");
    let human = storage
        .save_minutes_markdown(request(
            &source.meeting_id,
            Some(base.stamp()),
            "生成中に人間が保存",
        ))
        .unwrap();
    assert!(storage
        .finish_minutes_markdown(&job.id, 1, "生成後".into())
        .unwrap_err()
        .contains("更新"));
    assert_eq!(
        storage.get_minutes_markdown(&source.meeting_id).unwrap(),
        Some(human)
    );
    assert_eq!(storage.get_job(&job.id).unwrap(), job);
}

#[test]
fn admission_rejects_stale_target_or_bad_uuid_before_copying_fixed_input() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let mut source = generation_source(&storage);
    let base = storage
        .save_minutes_markdown(request(&source.meeting_id, None, "現在の本文"))
        .unwrap();
    let before = storage.list_jobs(&source.meeting_id).unwrap();
    for target in [
        GenerationTarget {
            schema_version: 1,
            expected: None,
        },
        GenerationTarget {
            schema_version: 2,
            expected: Some(base.stamp()),
        },
        GenerationTarget {
            schema_version: 1,
            expected: Some(MarkdownStamp {
                lock_version: base.lock_version + 1,
                content_hash: base.content_hash.clone(),
            }),
        },
    ] {
        assert!(storage
            .enqueue_markdown_minutes_job_configured(
                source.clone(),
                serde_json::json!({}),
                target,
                |_| Ok((serde_json::json!({}), Some(1)))
            )
            .is_err());
    }
    source.operation_id = "bad-uuid".into();
    assert!(storage
        .enqueue_markdown_minutes_job_configured(
            source.clone(),
            serde_json::json!({}),
            GenerationTarget {
                schema_version: 1,
                expected: Some(base.stamp())
            },
            |_| Ok((serde_json::json!({}), Some(1)))
        )
        .is_err());
    assert_eq!(storage.list_jobs(&source.meeting_id).unwrap(), before);
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM analysis_revisions WHERE publication_state='internal_snapshot'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn incomplete_paused_or_mismatched_output_never_publishes_generated_body() {
    for mode in [
        "queued",
        "incomplete",
        "paused",
        "wrong_hash",
        "empty",
        "nul",
        "oversize",
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let source = generation_source(&storage);
        let base = storage
            .save_minutes_markdown(request(&source.meeting_id, None, "保持する本文"))
            .unwrap();
        let job = generation_job(&storage, &source, Some(base.stamp()));
        if mode != "queued" {
            storage
                .acquire_job_kind(Some(JobKind::MinutesGeneration))
                .unwrap();
            if mode != "incomplete" {
                storage
                    .save_job_checkpoint(
                        &job.id,
                        0,
                        1,
                        serde_json::json!({"markdownHash":content_hash("生成結果")}),
                    )
                    .unwrap();
            }
            if mode == "paused" {
                storage.request_job_pause(&job.id, "user").unwrap();
                storage
                    .save_job_checkpoint(
                        &job.id,
                        1,
                        1,
                        serde_json::json!({"markdownHash":content_hash("生成結果")}),
                    )
                    .unwrap();
            }
        }
        let text = match mode {
            "wrong_hash" => "別の結果".into(),
            "empty" => " \n".into(),
            "nul" => "a\0b".into(),
            "oversize" => "あ".repeat(MAX_BYTES / 3 + 1),
            _ => "生成結果".into(),
        };
        let before = storage.get_job(&job.id).unwrap();
        assert!(
            storage.finish_minutes_markdown(&job.id, 1, text).is_err(),
            "{mode}"
        );
        assert_eq!(storage.get_job(&job.id).unwrap(), before);
        assert_eq!(
            storage.get_minutes_markdown(&source.meeting_id).unwrap(),
            Some(base)
        );
    }
}

#[test]
fn failure_to_mark_job_succeeded_rolls_back_the_generated_markdown_update() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let source = generation_source(&storage);
    let base = storage
        .save_minutes_markdown(request(&source.meeting_id, None, "保持する本文"))
        .unwrap();
    let job = completed_generation(&storage, &source, Some(base.stamp()), "生成結果");
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER synthetic_generation_commit_failure BEFORE UPDATE OF state ON processing_jobs WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT,'synthetic job failure'); END;").unwrap();
    assert!(storage
        .finish_minutes_markdown(&job.id, 1, "生成結果".into())
        .unwrap_err()
        .contains("synthetic job failure"));
    assert_eq!(
        storage.get_minutes_markdown(&source.meeting_id).unwrap(),
        Some(base)
    );
    assert_eq!(storage.get_job(&job.id).unwrap(), job);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER synthetic_generation_commit_failure;")
        .unwrap();
    storage
        .finish_minutes_markdown(&job.id, 1, "生成結果".into())
        .unwrap();
}

#[test]
fn generation_commit_failure_rolls_back_body_and_success_after_both_sql_updates() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let source = generation_source(&storage);
    let base = storage
        .save_minutes_markdown(request(&source.meeting_id, None, "保持する本文"))
        .unwrap();
    let job = completed_generation(&storage, &source, Some(base.stamp()), "生成結果");
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER synthetic_generation_deferred_failure AFTER UPDATE OF state ON processing_jobs WHEN NEW.state='succeeded' BEGIN INSERT INTO minutes_markdown SELECT 'absent-generated-meeting',markdown,content_hash,lock_version,operation_id,base_lock_version,base_content_hash,updated_at FROM minutes_markdown WHERE meeting_id=NEW.meeting_id; END; PRAGMA defer_foreign_keys=ON;").unwrap();
    assert!(storage
        .finish_minutes_markdown(&job.id, 1, "生成結果".into())
        .is_err());
    assert_eq!(
        storage.get_minutes_markdown(&source.meeting_id).unwrap(),
        Some(base)
    );
    assert_eq!(storage.get_job(&job.id).unwrap(), job);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER synthetic_generation_deferred_failure;")
        .unwrap();
    storage
        .finish_minutes_markdown(&job.id, 1, "生成結果".into())
        .unwrap();
}

#[test]
fn duplicate_generation_target_and_unread_existing_minutes_do_not_enqueue_or_publish() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let source = generation_source(&storage);
    assert!(storage
        .enqueue_markdown_minutes_job_configured(
            source.clone(),
            serde_json::json!({}),
            GenerationTarget {
                schema_version: 1,
                expected: None
            },
            |_| Ok((
                serde_json::json!({"markdownTarget":{"schemaVersion":1,"expected":null}}),
                Some(1)
            ))
        )
        .is_err());
    assert!(storage.list_jobs(&source.meeting_id).unwrap().is_empty());
    // The saved-meeting fixture has a fixed recording-start operation ID.
    // Use an independent synthetic DB, not a second recording with that ID.
    let legacy_root = tempfile::tempdir().unwrap();
    let storage = Storage::open(legacy_root.path().into()).unwrap();
    let (legacy_meeting, _) = legacy(&storage);
    let existing = storage.get_meeting(&legacy_meeting).unwrap().unwrap();
    let versions = storage.analysis_versions(&legacy_meeting).unwrap();
    let version = versions
        .versions
        .iter()
        .find(|v| Some(&v.id) == versions.current_id.as_ref())
        .unwrap();
    let request = super::super::minutes_input::MinutesJobRequest {
        meeting_id: legacy_meeting,
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: version.id.clone(),
        expected_content_hash: version.content_hash.clone(),
        expected_meeting_lock_version: existing.lock_version,
        expected_draft: versions.draft,
    };
    assert!(storage
        .enqueue_markdown_minutes_job_configured(
            request.clone(),
            serde_json::json!({}),
            GenerationTarget {
                schema_version: 1,
                expected: None
            },
            |_| Ok((serde_json::json!({}), Some(1)))
        )
        .unwrap_err()
        .contains("読み込"));
    assert!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM minutes_markdown WHERE meeting_id=?1)",
                [request.meeting_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
            == false
    );
}

fn meeting(storage: &Storage) -> String {
    storage.create_meeting("Markdown").unwrap().id
}
fn request(meeting: &str, expected: Option<MarkdownStamp>, text: &str) -> SaveMarkdownRequest {
    SaveMarkdownRequest {
        meeting_id: meeting.into(),
        operation_id: Uuid::new_v4().to_string(),
        expected,
        markdown: text.into(),
    }
}
fn legacy(storage: &Storage) -> (String, String) {
    let (source, job, content) = super::super::minutes::tests::fixture(storage, 1);
    let revision = storage
        .finish_minutes_candidate(&job.id, 1, content)
        .unwrap();
    let lock = storage
        .get_meeting(&source.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    storage
        .commit_minutes_candidate(&source.meeting_id, &revision, lock, false)
        .unwrap();
    (source.meeting_id, revision)
}

#[test]
fn exact_markdown_utf8_whitespace_html_and_empty_human_text_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    assert_eq!(storage.get_minutes_markdown(&meeting).unwrap(), None);
    let text = "\r\n# 人間の議事録\r\n\r\n**太字**  \n\t<script>x()</script>\n\n[00:01:00–00:01:30](audio:00:01:00-00:01:30)\n ";
    let first = storage
        .save_minutes_markdown(request(&meeting, None, text))
        .unwrap();
    assert_eq!(first.markdown, text);
    assert_eq!(first.content_hash, content_hash(text));
    assert_eq!(first.lock_version, 1);
    let cleared = storage
        .save_minutes_markdown(request(&meeting, Some(first.stamp()), ""))
        .unwrap();
    assert_eq!(cleared.markdown, "");
    assert_eq!(cleared.lock_version, 2);
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        reopened.get_minutes_markdown(&meeting).unwrap(),
        Some(cleared)
    );
}

#[test]
fn lost_response_retry_is_idempotent_only_for_the_same_operation_body_and_base() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let original = request(&meeting, None, "\n# 点検日\n");
    let first = storage.save_minutes_markdown(original.clone()).unwrap();
    assert_eq!(
        storage.save_minutes_markdown(original.clone()).unwrap(),
        first
    );
    let mut changed = original.clone();
    changed.markdown.push_str("変更");
    assert!(storage
        .save_minutes_markdown(changed)
        .unwrap_err()
        .contains("同じ"));
    let mut changed = original.clone();
    changed.expected = Some(first.stamp());
    assert!(storage
        .save_minutes_markdown(changed)
        .unwrap_err()
        .contains("同じ"));
    let later = storage
        .save_minutes_markdown(request(&meeting, Some(first.stamp()), "人間が後で編集"))
        .unwrap();
    assert!(storage
        .save_minutes_markdown(original)
        .unwrap_err()
        .contains("更新"));
    assert_eq!(storage.get_minutes_markdown(&meeting).unwrap(), Some(later));
}

#[test]
fn stale_generation_or_human_save_cannot_replace_newer_human_editing() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let first = storage
        .save_minutes_markdown(request(&meeting, None, "初回生成"))
        .unwrap();
    let generated = request(&meeting, Some(first.stamp()), "遅れて完成した生成結果");
    let edited = storage
        .save_minutes_markdown(request(&meeting, Some(first.stamp()), "人間の新しい編集\n"))
        .unwrap();
    assert!(storage
        .save_minutes_markdown(generated)
        .unwrap_err()
        .contains("更新"));
    let mut wrong = request(&meeting, Some(edited.stamp()), "別の保存");
    wrong.expected.as_mut().unwrap().content_hash = "b".repeat(64);
    assert!(storage.save_minutes_markdown(wrong).is_err());
    assert!(storage
        .save_minutes_markdown(request(&meeting, None, "勝手な初期化"))
        .is_err());
    assert_eq!(
        storage.get_minutes_markdown(&meeting).unwrap(),
        Some(edited)
    );
}

#[test]
fn missing_meeting_and_invalid_operation_or_stamp_do_not_create_documents() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let mut wrong = request(&meeting, None, "本文");
    wrong.operation_id = "not-a-canonical-operation".into();
    assert!(storage.save_minutes_markdown(wrong).is_err());
    for version in [0, -1, MAX_LOCK] {
        let wrong = request(
            &meeting,
            Some(MarkdownStamp {
                lock_version: version,
                content_hash: "a".repeat(64),
            }),
            "本文",
        );
        assert!(storage.save_minutes_markdown(wrong).is_err());
    }
    assert!(storage
        .save_minutes_markdown(request("missing", None, "本文"))
        .is_err());
    assert_eq!(storage.get_minutes_markdown(&meeting).unwrap(), None);
}

#[test]
fn nul_and_utf8_byte_limit_are_rejected_without_losing_saved_text() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let saved = storage
        .save_minutes_markdown(request(&meeting, None, "保持する人間の編集"))
        .unwrap();
    for text in ["途中\0NUL".into(), "あ".repeat(MAX_BYTES / 3 + 1)] {
        assert!(storage
            .save_minutes_markdown(request(&meeting, Some(saved.stamp()), &text))
            .is_err());
        assert_eq!(
            storage.get_minutes_markdown(&meeting).unwrap(),
            Some(saved.clone())
        );
    }
    let at_limit = "a".repeat(MAX_BYTES);
    let next = storage
        .save_minutes_markdown(request(&meeting, Some(saved.stamp()), &at_limit))
        .unwrap();
    assert_eq!(next.markdown.len(), MAX_BYTES);
}

#[test]
fn db_write_failure_rolls_back_document_and_keeps_retry_identity() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let first = storage
        .save_minutes_markdown(request(&meeting, None, "保存済み"))
        .unwrap();
    let pending = request(&meeting, Some(first.stamp()), "未保存入力");
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER synthetic_minutes_failure BEFORE UPDATE ON minutes_markdown BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(storage
        .save_minutes_markdown(pending.clone())
        .unwrap_err()
        .contains("synthetic failure"));
    assert_eq!(storage.get_minutes_markdown(&meeting).unwrap(), Some(first));
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER synthetic_minutes_failure;")
        .unwrap();
    let saved = storage.save_minutes_markdown(pending.clone()).unwrap();
    assert_eq!(storage.save_minutes_markdown(pending).unwrap(), saved);
}

#[test]
fn db_deferred_commit_failure_rolls_back_the_successful_write_and_keeps_retry_identity() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let first = storage
        .save_minutes_markdown(request(&meeting, None, "保存済み"))
        .unwrap();
    let pending = request(&meeting, Some(first.stamp()), "まだ確定していない本文");
    {
        let mut db = storage.connection.lock().unwrap();
        db.execute_batch("CREATE TRIGGER synthetic_minutes_commit_failure AFTER UPDATE ON minutes_markdown BEGIN INSERT INTO minutes_markdown SELECT 'absent-meeting-for-commit-test',markdown,content_hash,lock_version,operation_id,base_lock_version,base_content_hash,updated_at FROM minutes_markdown WHERE meeting_id=NEW.meeting_id; END; PRAGMA defer_foreign_keys=ON;").unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let written = save(&tx, &pending).unwrap();
        assert_eq!(written.markdown, pending.markdown);
        assert!(tx.commit().unwrap_err().to_string().contains("FOREIGN KEY"));
        db.execute_batch("PRAGMA defer_foreign_keys=ON;").unwrap();
    }
    assert!(storage
        .save_minutes_markdown(pending.clone())
        .unwrap_err()
        .contains("FOREIGN KEY"));
    assert_eq!(storage.get_minutes_markdown(&meeting).unwrap(), Some(first));
    {
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM minutes_markdown", [], |r| r.get(0))
                .unwrap(),
            1
        );
        db.execute_batch("DROP TRIGGER synthetic_minutes_commit_failure;")
            .unwrap();
    }
    let next = storage.save_minutes_markdown(pending.clone()).unwrap();
    assert_eq!(storage.save_minutes_markdown(pending).unwrap(), next);
}

#[test]
fn replacing_text_creates_no_minutes_revisions_evidence_drafts_or_old_text_copy() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let mut saved = storage
        .save_minutes_markdown(request(&meeting, None, "初回"))
        .unwrap();
    for index in 0..12 {
        saved = storage
            .save_minutes_markdown(request(
                &meeting,
                Some(saved.stamp()),
                &format!("現在の本文 {index}"),
            ))
            .unwrap();
    }
    let db = storage.connection.lock().unwrap();
    for table in [
        "minutes_revisions",
        "minutes_items",
        "evidence_links",
        "working_drafts",
        "edit_operations",
    ] {
        assert_eq!(
            db.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM minutes_markdown", [], |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        read(&db, &meeting).unwrap().unwrap().markdown,
        "現在の本文 11"
    );
}

#[test]
fn meeting_scope_isolated_and_deleting_a_meeting_cascades_only_its_document() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = meeting(&storage);
    let b = meeting(&storage);
    let first = storage
        .save_minutes_markdown(request(&a, None, "会議A"))
        .unwrap();
    let second = storage
        .save_minutes_markdown(request(&b, None, "会議B"))
        .unwrap();
    assert!(storage
        .save_minutes_markdown(request(&b, Some(first.stamp()), "Aから更新"))
        .is_err());
    let db = storage.connection.lock().unwrap();
    db.execute("DELETE FROM meetings WHERE id=?1", [&a])
        .unwrap();
    assert_eq!(read(&db, &a).unwrap(), None);
    assert_eq!(read(&db, &b).unwrap(), Some(second));
}

#[test]
fn tampered_hash_is_rejected_by_read_save_and_document_validation() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let saved = storage
        .save_minutes_markdown(request(&meeting, None, "保存済み"))
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE minutes_markdown SET markdown='改変' WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    assert!(storage.get_minutes_markdown(&meeting).is_err());
    assert!(storage
        .save_minutes_markdown(request(&meeting, Some(saved.stamp()), "再保存"))
        .is_err());
    assert!(verify_all(&storage.connection.lock().unwrap()).is_err());
}

#[test]
fn first_read_imports_current_effective_draft_without_source_ids_and_reopen_keeps_human_text() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base) = legacy(&storage);
    let draft = storage.create_minutes_draft(&meeting, &base).unwrap();
    let mut payload = draft.payload;
    payload.blocks.push(Block::Paragraph {
        item_id: None,
        content: vec![Inline::text("人間の保存済み追記")],
    });
    storage
        .save_minutes_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let before = storage
        .load_minutes_content(&meeting, &base)
        .unwrap()
        .unwrap();
    let job_id = storage
        .connection
        .lock()
        .unwrap()
        .query_row::<String, _, _>(
            "SELECT id FROM processing_jobs WHERE output_minutes_id=?1",
            [&base],
            |r| r.get(0),
        )
        .unwrap();
    let input = storage.load_minutes_input(&job_id).unwrap();
    let legacy_hash = before.content_hash;
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    let migrated = reopened.get_minutes_markdown(&meeting).unwrap().unwrap();
    assert!(migrated.markdown.contains("人間の保存済み追記"));
    assert!(migrated.markdown.contains("](audio:"));
    assert!(!migrated.markdown.contains("[^根拠"));
    assert!(!migrated.markdown.contains(&input.analysis.id));
    assert_eq!(
        reopened
            .load_minutes_content(&meeting, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        legacy_hash
    );
    let edited = reopened
        .save_minutes_markdown(request(
            &meeting,
            Some(migrated.stamp()),
            "人間だけが管理する新本文",
        ))
        .unwrap();
    drop(reopened);
    let twice = Storage::open(root.path().into()).unwrap();
    assert_eq!(twice.get_minutes_markdown(&meeting).unwrap(), Some(edited));
}

#[test]
fn failed_import_does_not_partially_migrate_or_delete_existing_documents() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base) = legacy(&storage);
    let draft = storage.create_minutes_draft(&meeting, &base).unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE working_drafts SET content_hash=?1 WHERE id=?2",
            params!["b".repeat(64), draft.id],
        )
        .unwrap();
    drop(storage);
    let reopened = Storage::open(root.path().into()).unwrap();
    assert!(reopened.get_minutes_markdown(&meeting).is_err());
    let unrelated = reopened.create_meeting("別の会議は利用できる").unwrap().id;
    let kept = reopened
        .save_minutes_markdown(request(&unrelated, None, "別会議の人間の本文"))
        .unwrap();
    assert_eq!(
        reopened.get_minutes_markdown(&unrelated).unwrap(),
        Some(kept)
    );
    drop(reopened);
    let db = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM minutes_markdown WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    assert!(db
        .query_row::<bool, _, _>(
            "SELECT EXISTS(SELECT 1 FROM minutes_revisions WHERE id=?1 AND meeting_id=?2)",
            params![base, meeting],
            |r| r.get(0)
        )
        .unwrap());
}

#[test]
fn schema_30_upgrade_imports_effective_text_once_and_preserves_source() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base) = legacy(&storage);
    let before = storage
        .load_minutes_content(&meeting, &base)
        .unwrap()
        .unwrap();
    {
        // Construct the previous trusted schema in this private fixture only.
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM minutes_markdown", [], |r| r.get(0))
                .unwrap(),
            0
        );
        db.execute_batch(
            "DROP TABLE minutes_markdown; DELETE FROM schema_migrations WHERE version=31;",
        )
        .unwrap();
        assert_eq!(super::super::verify_migration_history(&db).unwrap(), 30);
    }
    drop(storage);
    let upgraded = Storage::open(root.path().into()).unwrap();
    let body = upgraded.get_minutes_markdown(&meeting).unwrap().unwrap();
    assert_eq!(body.lock_version, 1);
    assert!(!body.markdown.is_empty());
    assert_eq!(
        upgraded
            .load_minutes_content(&meeting, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        before.content_hash
    );
    assert_eq!(
        super::super::verify_migration_history(&upgraded.connection.lock().unwrap()).unwrap(),
        31
    );
    drop(upgraded);
    let again = Storage::open(root.path().into()).unwrap();
    assert_eq!(again.get_minutes_markdown(&meeting).unwrap(), Some(body));
}

#[test]
fn exact_fractional_intervals_replace_legacy_evidence_without_utterance_metadata() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (_request, job, mut content) = super::super::minutes::tests::fixture(&storage, 1);
    let input = storage.load_minutes_input(&job.id).unwrap();
    content.evidence[0].start_us = 100_001;
    content.evidence[0].end_us = 799_999;
    let markdown = content.plain_markdown(&input).unwrap();
    assert!(markdown.contains("](audio:00:00:00.100001-00:00:00.799999)"));
    assert!(!markdown.contains("[^根拠"));
    assert!(!markdown.contains(&content.evidence[0].utterance_id));
    assert!(!markdown.contains(&content.evidence[0].analysis_revision_id));
}

#[test]
fn malformed_retry_base_hash_and_lock_overflow_are_not_accepted() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage);
    let first = storage
        .save_minutes_markdown(request(&meeting, None, "1"))
        .unwrap();
    let second = storage
        .save_minutes_markdown(request(&meeting, Some(first.stamp()), "2"))
        .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE minutes_markdown SET base_content_hash=?1 WHERE meeting_id=?2",
            params!["G".repeat(64), meeting],
        )
        .unwrap();
    }
    assert!(storage.get_minutes_markdown(&meeting).is_err());
    {
        let db = storage.connection.lock().unwrap();
        db.execute("UPDATE minutes_markdown SET lock_version=?1,base_lock_version=?2,base_content_hash=?3 WHERE meeting_id=?4",params![MAX_LOCK,MAX_LOCK-1,first.content_hash,meeting]).unwrap();
    }
    let ceiling = storage.get_minutes_markdown(&meeting).unwrap().unwrap();
    assert_eq!(ceiling.markdown, second.markdown);
    assert!(storage
        .save_minutes_markdown(request(&meeting, Some(ceiling.stamp()), "overflow"))
        .is_err());
    assert_eq!(
        storage.get_minutes_markdown(&meeting).unwrap(),
        Some(ceiling)
    );
}

#[test]
fn absent_base_cannot_overwrite_unread_saved_legacy_minutes() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, base) = legacy(&storage);
    let original = storage
        .load_minutes_content(&meeting, &base)
        .unwrap()
        .unwrap();
    assert!(storage
        .save_minutes_markdown(request(&meeting, None, "既存本文を見ずに上書き"))
        .unwrap_err()
        .contains("読み込"));
    assert_eq!(
        read(&storage.connection.lock().unwrap(), &meeting).unwrap(),
        None
    );
    assert_eq!(
        storage
            .load_minutes_content(&meeting, &base)
            .unwrap()
            .unwrap()
            .content_hash,
        original.content_hash
    );
    let imported = storage.get_minutes_markdown(&meeting).unwrap().unwrap();
    let edited = storage
        .save_minutes_markdown(request(
            &meeting,
            Some(imported.stamp()),
            "確認後の人間の編集",
        ))
        .unwrap();
    assert_eq!(
        storage.get_minutes_markdown(&meeting).unwrap(),
        Some(edited)
    );
}
