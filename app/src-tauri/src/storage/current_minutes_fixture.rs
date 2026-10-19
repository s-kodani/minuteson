//! Current human-owned Markdown fixtures, without structured candidates or drafts.
use super::*;

pub(crate) fn input() -> MinutesInput {
    use crate::storage::{AnalysisSpeaker, AnalysisUtterance, AnalysisView};
    MinutesInput {
        analysis: AnalysisView {
            id: "analysis".into(),
            meeting_id: "meeting".into(),
            content_hash: "a".repeat(64),
            speakers: vec![AnalysisSpeaker {
                id: "speaker".into(),
                label: "話者A".into(),
                display_name: Some("田中".into()),
            }],
            utterances: vec![AnalysisUtterance {
                speaker_identification: None,
                id: "utterance".into(),
                track_id: "track".into(),
                start_us: 120_000_000,
                end_us: 126_000_000,
                recognized_text: "点検日です".into(),
                effective_text:
                    "点検日は2026-10-09です。田中が担当します。来週金曜までに確認します。".into(),
                attribution_state: "assigned".into(),
                speaker_ids: vec!["speaker".into()],
                recognition_is_source: false,
            }],
        },
        meeting_title: "開発定例".into(),
        meeting_created_at: 0,
        meeting_started_at_utc_ms: Some(1791028800000),
        working_content_hash: "b".repeat(64),
    }
}

pub(in crate::storage) fn meeting(storage: &Storage) -> String {
    let meeting = super::analysis::tests::saved_meeting(storage);
    let mut input = super::analysis::tests::input();
    input.utterances[0].text = "点検日を確認することに決まりました。".into();
    let id = storage
        .create_analysis_candidate(&meeting.id, input)
        .unwrap();
    storage
        .commit_analysis_candidate(&meeting.id, &id, 0, false)
        .unwrap();
    storage.create_transcript_draft(&meeting.id, &id).unwrap();
    meeting.id
}

pub(in crate::storage) fn save(
    storage: &Storage,
    meeting: &str,
    text: &str,
) -> minutes_markdown::MarkdownDocument {
    let expected = storage
        .get_minutes_markdown(meeting)
        .unwrap()
        .map(|body| body.stamp());
    storage
        .save_minutes_markdown(minutes_markdown::SaveMarkdownRequest {
            meeting_id: meeting.into(),
            operation_id: Uuid::new_v4().to_string(),
            expected,
            markdown: text.into(),
        })
        .unwrap()
}

pub(in crate::storage) fn candidate(storage: &Storage, meeting: &str) -> String {
    save(
        storage,
        meeting,
        "# 点検会議\n- 点検日を確認する。\n[区間](audio:00:00:00.100001-00:00:00.799999)\n",
    )
    .operation_id
}
