use super::super::minutes_markdown::SaveMarkdownRequest;
use super::*;

fn fixture(storage: &Storage, text: &str) -> (MarkdownExportRequest, MarkdownDocument) {
    let meeting = storage.create_meeting("../点検:会議").unwrap();
    let document = storage
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: meeting.id.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: None,
            markdown: text.into(),
        })
        .unwrap();
    (
        MarkdownExportRequest {
            operation_id: Uuid::new_v4().to_string(),
            meeting_id: meeting.id,
            expected: document.stamp(),
        },
        document,
    )
}

#[test]
fn exact_human_bytes_empty_unicode_html_and_interval_have_no_added_metadata() {
    for text in [
        "",
        "\r\n# 点検日　\r\n<script>literal</script>\n[再生](mon-audio:0.125-1.25)  \n\n",
        "末尾改行なし",
    ] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (r, before) = fixture(&s, text);
        let prepared = s.prepare_markdown_export(r.clone()).unwrap();
        assert!(prepared.filename.ends_with("_議事録.md"));
        assert!(!prepared.filename.contains('/'));
        let path = outside.path().join("exact.md");
        let record = s.write_markdown_export(prepared, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
        assert_eq!(record.output_size_bytes, text.len() as i64);
        assert_eq!(
            record.output_content_hash,
            format!("{:x}", Sha256::digest(text.as_bytes()))
        );
        assert_eq!(record.exported_content_hash, before.content_hash);
        assert_eq!(record.state, "succeeded");
        assert!(record.minutes_revision_id.is_none());
        assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(before));
        let db = s.connection.lock().unwrap();
        for table in [
            "minutes_revisions",
            "working_drafts",
            "edit_operations",
            "minutes_items",
        ] {
            assert_eq!(
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}
#[test]
fn cancelled_preparation_and_stale_selection_never_publish_or_make_receipts() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, before) = fixture(&s, "initial");
    drop(s.prepare_markdown_export(r.clone()).unwrap());
    assert!(s.exports(&r.meeting_id).unwrap().is_empty());
    let prepared = s.prepare_markdown_export(r.clone()).unwrap();
    let edited = s
        .save_minutes_markdown(SaveMarkdownRequest {
            meeting_id: r.meeting_id.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected: Some(before.stamp()),
            markdown: "human changed".into(),
        })
        .unwrap();
    let path = outside.path().join("keep.md");
    std::fs::write(&path, "external").unwrap();
    assert!(s.write_markdown_export(prepared, &path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
    assert!(s.prepare_markdown_export(r.clone()).is_err());
    assert!(s.exports(&r.meeting_id).unwrap().is_empty());
    assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(edited));
}
#[test]
fn receipt_replay_after_response_loss_never_rewrites_even_after_source_changes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, before) = fixture(&s, "source");
    let prepared = s.prepare_markdown_export(r.clone()).unwrap();
    let duplicate = s.prepare_markdown_export(r.clone()).unwrap();
    let path = outside.path().join("a.md");
    let record = s.write_markdown_export(prepared, &path).unwrap();
    s.save_minutes_markdown(SaveMarkdownRequest {
        meeting_id: r.meeting_id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected: Some(before.stamp()),
        markdown: "later".into(),
    })
    .unwrap();
    std::fs::write(&path, "external changed").unwrap();
    assert_eq!(
        s.write_markdown_export(duplicate, &path).unwrap().id,
        record.id
    );
    assert_eq!(
        s.existing_markdown_export(&r).unwrap().unwrap().id,
        record.id
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "external changed");
    let mut changed = r.clone();
    changed.expected.lock_version += 1;
    assert!(s.existing_markdown_export(&changed).is_err());
    changed = r;
    changed.meeting_id = "other".into();
    assert!(s.existing_markdown_export(&changed).is_err());
}
#[test]
fn corrupt_prepared_bytes_and_stored_hash_never_publish() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, _) = fixture(&s, "verified");
    let mut p = s.prepare_markdown_export(r.clone()).unwrap();
    p.document.markdown = "tampered".into();
    let path = outside.path().join("a.md");
    assert!(s.write_markdown_export(p, &path).is_err());
    assert!(!path.exists());
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE minutes_markdown SET markdown='corrupt' WHERE meeting_id=?1",
            [&r.meeting_id],
        )
        .unwrap();
    assert!(s.prepare_markdown_export(r.clone()).is_err());
    assert!(s.exports(&r.meeting_id).unwrap().is_empty());
}
#[test]
fn unsafe_destinations_are_failed_and_retained_without_modifying_source_or_managed_files() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, before) = fixture(&s, "human");
    let path = root.path().join("managed.md");
    std::fs::write(&path, "keep").unwrap();
    let result = s
        .write_markdown_export(s.prepare_markdown_export(r.clone()).unwrap(), &path)
        .unwrap();
    assert_eq!(result.state, "failed");
    assert!(result.error_message.is_some());
    assert!(result.exported_at.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
    assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(before));
    assert_eq!(
        s.existing_markdown_export(&r).unwrap().unwrap().state,
        "failed"
    );
}
#[test]
fn durable_write_failures_and_published_uncertainty_are_not_success_or_retried() {
    for published in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (r, before) = fixture(&s, "human");
        let p = s.prepare_markdown_export(r.clone()).unwrap();
        let path = outside.path().join("a.md");
        let result = s
            .write_markdown_export_using(p, &path, |_, _| {
                Err(crate::exports::PublishError {
                    published,
                    message: "injected I/O failure".into(),
                })
            })
            .unwrap();
        assert_eq!(
            result.state,
            if published { "interrupted" } else { "failed" }
        );
        assert!(result.exported_at.is_none());
        assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(before));
        let p = s.prepare_markdown_export(r.clone()).unwrap();
        assert_eq!(
            s.write_markdown_export_using(p, &path, |_, _| panic!("must not republish"))
                .unwrap()
                .state,
            result.state
        );
    }
}
#[test]
fn cold_open_marks_incomplete_write_uncertain_and_receipt_still_matches_original_request() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, before) = fixture(&s, "human");
    let path = outside.path().join("a.md");
    s.write_markdown_export(s.prepare_markdown_export(r.clone()).unwrap(), &path)
        .unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE export_records SET state='writing',exported_at=NULL WHERE id=?1",
            [&r.operation_id],
        )
        .unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let record = s.existing_markdown_export(&r).unwrap().unwrap();
    assert_eq!(record.state, "interrupted");
    assert!(record.exported_at.is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "human");
    assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(before));
}
#[test]
fn result_commit_failure_never_republishes_and_cold_open_keeps_the_human_body() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, before) = fixture(&s, "current human Markdown");
    let first = s.prepare_markdown_export(r.clone()).unwrap();
    let duplicate = s.prepare_markdown_export(r.clone()).unwrap();
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_plain_export_result BEFORE UPDATE ON export_records WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT,'simulated result commit failure');END;").unwrap();
    let path = outside.path().join("existing.md");
    assert!(s.write_markdown_export(first, &path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before.markdown);
    assert_eq!(
        s.existing_markdown_export(&r).unwrap().unwrap().state,
        "writing"
    );
    std::fs::write(&path, "external human change").unwrap();
    assert_eq!(
        s.write_markdown_export(duplicate, &path).unwrap().state,
        "writing"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "external human change"
    );
    assert_eq!(
        s.get_minutes_markdown(&r.meeting_id).unwrap(),
        Some(before.clone())
    );
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let receipt = s.existing_markdown_export(&r).unwrap().unwrap();
    assert_eq!(receipt.state, "interrupted");
    assert!(receipt.exported_at.is_none());
    assert_eq!(s.get_minutes_markdown(&r.meeting_id).unwrap(), Some(before));
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "external human change"
    );
}

#[test]
fn invalid_requests_and_missing_documents_are_rejected_before_a_dialog() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (r, _) = fixture(&s, "human");
    for value in ["invalid", "00000000-0000-1000-8000-000000000001"] {
        let mut bad = r.clone();
        bad.operation_id = value.into();
        assert!(s.existing_markdown_export(&bad).is_err());
        assert!(s.prepare_markdown_export(bad).is_err());
    }
    let mut bad = r.clone();
    bad.expected.content_hash = "A".repeat(64);
    assert!(s.prepare_markdown_export(bad).is_err());
    let mut bad = r;
    bad.meeting_id = s.create_meeting("no document").unwrap().id;
    assert!(s.prepare_markdown_export(bad).is_err());
}
