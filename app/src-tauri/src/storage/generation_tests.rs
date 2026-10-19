//! Retired formats are rejected without executing or publishing old workers.
use super::*;
use serde_json::json;

#[test]
fn retired_worker_never_changes_fixed_input_or_human_markdown() {
    for version in 1..=6 {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = analysis::tests::saved_meeting(&storage);
        let id = storage
            .create_analysis_candidate(&meeting.id, analysis::tests::input())
            .unwrap();
        let committed = storage
            .commit_analysis_candidate(&meeting.id, &id, 0, false)
            .unwrap();
        let req = MinutesJobRequest {
            meeting_id: meeting.id.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected_analysis_id: id,
            expected_content_hash: committed.content_hash,
            expected_meeting_lock_version: committed.meeting_lock_version,
            expected_draft: None,
        };
        let human = storage
            .save_minutes_markdown(minutes_markdown::SaveMarkdownRequest {
                meeting_id: meeting.id.clone(),
                operation_id: Uuid::new_v4().to_string(),
                expected: None,
                markdown: "保持する人間本文".into(),
            })
            .unwrap();
        assert!(storage
            .enqueue_markdown_minutes_job_configured(
                req,
                json!({"profile":"standard","engine":"removed"}),
                minutes_markdown::GenerationTarget {
                    schema_version: 1,
                    expected: Some(human.stamp())
                },
                |_| Ok((json!({"schemaVersion":version}), Some(1)))
            )
            .is_err());
        assert!(storage.list_jobs(&meeting.id).unwrap().is_empty());
        assert_eq!(
            storage.get_minutes_markdown(&meeting.id).unwrap(),
            Some(human)
        );
    }
}
