use super::*;
use crate::minutes_document::{Block, Inline};

fn fixture(storage: &Storage) -> (String, String) {
    let m = super::super::minutes_ui::tests::meeting(storage);
    let id = super::super::minutes_ui::tests::candidate(storage, &m);
    let req = super::super::minutes_ui::tests::request(storage, &m, &id);
    storage.adopt_minutes(req).unwrap();
    (m, id)
}
fn request(storage: &Storage, m: &str, id: &str) -> ExportRequest {
    let state = storage.minutes_versions(m).unwrap();
    let view = storage.load_minutes_content(m, id).unwrap().unwrap();
    ExportRequest {
        operation_id: Uuid::new_v4().to_string(),
        meeting_id: m.into(),
        expected_minutes_id: id.into(),
        expected_content_hash: view.content_hash,
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: state.draft,
    }
}
#[test]
fn edited_export_is_derived_from_validated_json_and_replay_never_rewrites_the_file() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let initial = s.create_minutes_draft(&m, &id).unwrap();
    let mut payload = initial.payload.clone();
    payload.blocks.push(Block::Paragraph {
        item_id: None,
        content: vec![Inline::text("手動追記 <script> | 改行\n確認")],
    });
    let edited = s.save_minutes_draft(&initial.id, 0, payload).unwrap();
    let r = request(&s, &m, &id);
    let prepared = s.prepare_export(r.clone()).unwrap();
    assert!(prepared.markdown.contains("手動追記"));
    assert!(prepared.markdown.contains("[^根拠1]"));
    assert!(!prepared.markdown.contains("<script>"));
    let bytes = prepared.markdown.as_bytes().to_vec();
    let path = outside.path().join("会議's.md");
    let result = s.write_export(prepared, &path).unwrap();
    assert_eq!(result.state, "succeeded");
    assert!(result.exported_at.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        result.output_content_hash,
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(s.load_minutes_draft(&m, None).unwrap(), Some(edited));
    assert_eq!(s.minutes_versions(&m).unwrap().current_id, Some(id));
    std::fs::write(&path, b"external edits").unwrap();
    assert_eq!(s.existing_export(&r).unwrap().unwrap().id, result.id);
    let prepared = s.prepare_export(r.clone()).unwrap();
    assert_eq!(s.write_export(prepared, &path).unwrap().id, result.id);
    assert_eq!(std::fs::read(&path).unwrap(), b"external edits");
    assert_eq!(s.exports(&m).unwrap().len(), 1);
    let mut changed = r;
    changed.meeting_id = "another".into();
    assert!(s.existing_export(&changed).is_err());
}
#[test]
fn cancel_and_stale_dialog_input_do_not_change_drafts_heads_or_external_files() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let initial = s.create_minutes_draft(&m, &id).unwrap();
    let r = request(&s, &m, &id);
    let cancelled = s.prepare_export(r.clone()).unwrap();
    drop(cancelled);
    assert!(s.exports(&m).unwrap().is_empty());
    assert_eq!(
        s.load_minutes_draft(&m, None).unwrap(),
        Some(initial.clone())
    );
    let prepared = s.prepare_export(r.clone()).unwrap();
    let mut changed = initial.payload.clone();
    changed.blocks.push(Block::Paragraph {
        item_id: None,
        content: vec![Inline::text("保存ダイアログ中の追加編集")],
    });
    s.save_minutes_draft(&initial.id, 0, changed).unwrap();
    let path = outside.path().join("existing.md");
    std::fs::write(&path, b"preserved").unwrap();
    assert!(s.write_export(prepared, &path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"preserved");
    assert!(s.exports(&m).unwrap().is_empty());
    assert!(s.prepare_export(r).is_err());
}
#[test]
fn interrupted_exports_are_not_reported_as_success_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let r = request(&s, &m, &id);
    let p = s.prepare_export(r.clone()).unwrap();
    let path = outside.path().join("a.md");
    s.write_export(p, &path).unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE export_records SET state='writing',exported_at=NULL WHERE id=?1",
            params![r.operation_id],
        )
        .unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let record = s.existing_export(&r).unwrap().unwrap();
    assert_eq!(record.state, "interrupted");
    assert!(record.exported_at.is_none());
    assert!(record.error_message.unwrap().contains("外部ファイル"));
    assert!(path.exists());
}
#[test]
fn filenames_are_portable_and_export_foreign_keys_preserve_external_results() {
    let escaped = safe_filename("a/b:c*?\n<>|\\\"", 0).unwrap();
    assert!(escaped.ends_with("_議事録.md"));
    assert!(!escaped
        .chars()
        .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)));
    let name = safe_filename("定例", 0).unwrap();
    assert!(name.ends_with("_定例_議事録.md"));
    assert!(!safe_filename("../", 0).unwrap().contains("/"));
    assert!(safe_filename(&"会".repeat(200), 0).unwrap().len() <= 240);
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let r = request(&s, &m, &id);
    let p = s.prepare_export(r.clone()).unwrap();
    s.write_export(p, &outside.path().join("a.md")).unwrap();
    let other = s.create_meeting("other").unwrap();
    let db = s.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE export_records SET meeting_id=?1 WHERE id=?2",
            params![other.id, r.operation_id]
        )
        .is_err());
    db.execute(
        "UPDATE export_records SET minutes_revision_id=NULL WHERE id=?1",
        params![r.operation_id],
    )
    .unwrap();
    drop(db);
    assert!(s.exports(&m).unwrap()[0].minutes_revision_id.is_none());
    assert!(outside.path().join("a.md").exists());
}

#[test]
fn failed_publication_and_result_commit_keep_edits_and_do_not_retry_writes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let draft = s.create_minutes_draft(&m, &id).unwrap();
    let r = request(&s, &m, &id);
    let p = s.prepare_export(r.clone()).unwrap();
    let path = outside.path().join("a.md");
    std::fs::write(&path, b"original").unwrap();
    let record = s
        .write_export_using(p, &path, |_, _| {
            Err(crate::exports::PublishError {
                published: false,
                message: "模擬容量不足".into(),
            })
        })
        .unwrap();
    assert_eq!(record.state, "failed");
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(s.load_minutes_draft(&m, None).unwrap(), Some(draft));
    let r = request(&s, &m, &id);
    let mut p = s.prepare_export(r.clone()).unwrap();
    p.markdown.push_str("不正な派生物");
    assert!(s.write_export(p, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    let p = s.prepare_export(r.clone()).unwrap();
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_export_result BEFORE UPDATE ON export_records WHEN NEW.state='succeeded' BEGIN SELECT RAISE(ABORT,'simulated result commit failure');END;").unwrap();
    assert!(s.write_export(p, &path).is_err());
    assert_ne!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(s.existing_export(&r).unwrap().unwrap().state, "writing");
    let p = s.prepare_export(r.clone()).unwrap();
    std::fs::write(&path, b"external user edit").unwrap();
    assert_eq!(s.write_export(p, &path).unwrap().state, "writing");
    assert_eq!(std::fs::read(&path).unwrap(), b"external user edit");
}

#[test]
fn rejected_confirmed_destination_records_failure_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let draft = s.create_minutes_draft(&m, &id).unwrap();
    let r = request(&s, &m, &id);
    let p = s.prepare_export(r.clone()).unwrap();
    let path = root.path().join("protected.md");
    std::fs::write(&path, b"managed data").unwrap();
    let record = s
        .write_export_using(p, &path, |_, _| {
            panic!("unsafe destination must not publish")
        })
        .unwrap();
    assert_eq!(record.state, "failed");
    assert!(record.error_message.is_some());
    assert!(record.exported_at.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"managed data");
    assert_eq!(s.load_minutes_draft(&m, None).unwrap(), Some(draft));
    assert_eq!(s.existing_export(&r).unwrap().unwrap().id, record.id);
}

#[test]
fn v14_database_migrates_without_changing_minutes_drafts_or_heads() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, id) = fixture(&s);
    let draft = s.create_minutes_draft(&m, &id).unwrap();
    let view = s.load_minutes_content(&m, &id).unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE diarization_manual_assignments; DROP TRIGGER job_analysis_result_insert; DROP TABLE vocabulary_snapshots; DROP TABLE vocabulary_entries; DROP TABLE vocabulary_scopes; DROP INDEX processing_jobs_meeting_identity; DROP TABLE export_records; DELETE FROM schema_migrations WHERE version>=15;")
        .unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.load_minutes_draft(&m, None).unwrap(), Some(draft));
    assert_eq!(s.load_minutes_content(&m, &id).unwrap(), view);
    assert_eq!(s.minutes_versions(&m).unwrap().current_id, Some(id));
    assert!(s.exports(&m).unwrap().is_empty());
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        17
    );
}
