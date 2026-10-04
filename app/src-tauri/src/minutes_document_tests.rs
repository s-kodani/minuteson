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
                id: "utterance".into(),
                track_id: "track".into(),
                start_us: 120_000_000,
                end_us: 126_000_000,
                recognized_text: "点検日です".into(),
                effective_text:
                    "点検日は2026-10-09です。田中が担当します。来週金曜までに確認します。".into(),
                attribution_state: "assigned".into(),
                speaker_ids: vec!["speaker".into()],
            }],
        },
        meeting_title: "開発定例".into(),
        meeting_created_at: 0,
        meeting_started_at_utc_ms: Some(1791028800000),
        working_content_hash: "b".repeat(64),
    }
}
pub(crate) fn item(kind: ItemKind) -> GeneratedItem {
    GeneratedItem {
        kind,
        body: "点検日を確認する".into(),
        assignee_name: None,
        assignee_speaker_id: None,
        due_date: None,
        due_text: None,
        evidence_utterance_ids: vec!["utterance".into()],
    }
}

#[test]
fn generated_result_limits_reject_oversized_text_and_repeated_quotes() {
    let mut input = input();
    let mut raw = item(ItemKind::Summary);
    raw.body = "x".repeat(20_001);
    assert!(from_generated(&input, vec![raw]).is_err());
    let mut raw = item(ItemKind::Todo);
    raw.assignee_name = Some("x".repeat(201));
    assert!(from_generated(&input, vec![raw]).is_err());
    assert!(from_generated(&input, vec![item(ItemKind::Summary); 10_001]).is_err());
    let mut raw = item(ItemKind::Summary);
    raw.evidence_utterance_ids = vec!["utterance".into(); 101];
    assert!(from_generated(&input, vec![raw]).is_err());
    input.analysis.utterances[0].effective_text = "x".repeat(20_000);
    assert!(from_generated(&input, vec![item(ItemKind::Summary); 110]).is_err());
    input.analysis.utterances[0].effective_text = "x".repeat(20_001);
    assert!(from_generated(&input, vec![item(ItemKind::Summary)]).is_err());
    input.analysis.utterances[0].effective_text = "   ".into();
    assert!(from_generated(&input, vec![item(ItemKind::Summary)]).is_err());
}

#[test]
fn template_has_typed_blocks_fixed_evidence_and_unknown_todo_fields() {
    let input = input();
    let content =
        from_generated(&input, vec![item(ItemKind::Decision), item(ItemKind::Todo)]).unwrap();
    let json = serde_json::to_vec(&content).unwrap();
    assert_eq!(
        serde_json::from_slice::<MinutesContent>(&json).unwrap(),
        content
    );
    let markdown = content.markdown(&input).unwrap();
    for text in [
        "# 開発定例",
        "## 概要",
        "## 議題・論点",
        "## 決定事項",
        "## 課題・未決事項",
        "## ToDo",
        "| 内容 | 担当者 | 期限 | 根拠 |",
        "| 未確認 | 未確認 |",
        "00:02:00[^根拠",
        "解析版: analysis",
        "発言ID: utterance",
        "内容未確認",
    ] {
        assert!(markdown.contains(text), "{text}: {markdown}");
    }
    assert_eq!(content.items[1].assignee_name, None);
    assert_eq!(content.items[1].due_date, None);
    assert!(content
        .evidence
        .iter()
        .all(|e| e.verification_state == VerificationState::Unverified));
}

#[test]
fn generated_metadata_is_literal_grounded_and_dates_are_real_calendar_dates() {
    let input = input();
    let mut todo = item(ItemKind::Todo);
    todo.assignee_name = Some("田中".into());
    todo.assignee_speaker_id = Some("speaker".into());
    todo.due_date = Some("2026-10-09".into());
    todo.due_text = Some("来週金曜".into());
    let result = from_generated(&input, vec![todo.clone()]).unwrap();
    assert!(result
        .markdown(&input)
        .unwrap()
        .contains("田中 | 2026\\-10\\-09（来週金曜）"));
    for bad in [
        "2026-02-29",
        "2026-04-31",
        "0000-01-01",
        "2026-13-01",
        "2026-01-00",
        "２０２６-01-01",
    ] {
        assert!(!valid_date(bad));
    }
    for good in ["2000-02-29", "2024-02-29", "2026-10-09"] {
        assert!(valid_date(good));
    }
    todo.due_date = Some("2026-10-10".into());
    assert!(from_generated(&input, vec![todo.clone()]).is_err());
    todo.due_date = None;
    todo.assignee_name = Some("佐藤".into());
    assert!(from_generated(&input, vec![todo]).is_err());
    let mut summary = item(ItemKind::Summary);
    summary.due_text = Some("来週金曜".into());
    assert!(from_generated(&input, vec![summary]).is_err());
}

#[test]
fn document_rejects_wrong_ids_scopes_quotes_times_and_invisible_annotations() {
    let input = input();
    let content = from_generated(&input, vec![item(ItemKind::Decision)]).unwrap();
    for change in [
        "item",
        "link",
        "analysis",
        "utterance",
        "quote",
        "start",
        "end",
        "body",
        "hidden_item",
        "hidden_link",
        "schema",
        "heading",
        "table",
    ] {
        let mut changed = content.clone();
        match change {
            "item" => changed.evidence[0].minutes_item_id = "missing".into(),
            "link" => {
                if let Block::List { items, .. } = &mut changed.blocks[6] {
                    items[0].content.push(Inline::Evidence {
                        minutes_item_id: content.items[0].id.clone(),
                        evidence_link_id: "missing".into(),
                    });
                } else {
                    changed.evidence[0].id = "missing".into();
                }
            }
            "analysis" => changed.evidence[0].analysis_revision_id = "other".into(),
            "utterance" => changed.evidence[0].utterance_id = "other".into(),
            "quote" => changed.evidence[0].quote_text = "発言にない引用".into(),
            "start" => changed.evidence[0].start_us -= 1,
            "end" => changed.evidence[0].end_us += 1,
            "body" => changed.items[0].body = "異なる本文".into(),
            "hidden_item" => {
                let mut i = changed.items[0].clone();
                i.id = "hidden".into();
                i.source_type = SourceType::Manual;
                changed.items.push(i);
            }
            "hidden_link" => {
                let mut e = changed.evidence[0].clone();
                e.id = "hidden".into();
                changed.evidence.push(e);
            }
            "schema" => changed.schema_version = 2,
            "heading" => changed.blocks.push(heading(7, "不正")),
            "table" => changed.blocks.push(Block::Table {
                headers: vec![vec![Inline::text("列")]],
                rows: vec![TableRow {
                    item_id: None,
                    cells: vec![],
                }],
            }),
            _ => unreachable!(),
        }
        assert!(changed.validate(&input).is_err(), "{change}");
    }
    let mut raw = item(ItemKind::Decision);
    raw.evidence_utterance_ids.push("utterance".into());
    assert!(from_generated(&input, vec![raw]).is_err());
}

#[test]
fn rich_text_markdown_escapes_payloads_and_keeps_lists_tables_and_footnotes() {
    let input = input();
    let mut content = from_generated(&input, vec![item(ItemKind::Decision)]).unwrap();
    content.blocks.push(Block::Paragraph {
        item_id: None,
        content: vec![Inline::Text {
            text: " 強調<script>*x* ".into(),
            bold: true,
            italic: true,
        }],
    });
    content.blocks.push(Block::List {
        ordered: true,
        items: vec![TextBlock {
            item_id: None,
            content: vec![Inline::text("項目\n継続行")],
        }],
    });
    content.blocks.push(Block::Table {
        headers: vec![vec![Inline::text("自由入力")]],
        rows: vec![TableRow {
            item_id: None,
            cells: vec![vec![Inline::text("A | B\n<script>")]],
        }],
    });
    let text = content.markdown(&input).unwrap();
    assert!(!text.contains("<script>"));
    assert!(text.contains("***強調\\<script\\>\\*x\\****"));
    assert!(text.contains("1. 項目  \n   継続行"));
    assert!(text.contains("A \\| B<br>\\<script\\>"));
    assert_eq!(text.matches("[^根拠1]:").count(), 1);
    let mut json = serde_json::to_value(&content).unwrap();
    json["blocks"][0]["html"] = serde_json::json!("<script>");
    assert!(serde_json::from_value::<MinutesContent>(json).is_err());
}

#[test]
fn heading_formatting_preserves_item_evidence_and_old_canonical_json() {
    let input = input();
    let mut content = from_generated(&input, vec![item(ItemKind::Decision)]).unwrap();
    let old = serde_json::to_value(&content).unwrap();
    assert!(old["blocks"][0].get("itemId").is_none());
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<MinutesContent>(old.clone()).unwrap())
            .unwrap(),
        old
    );
    let index = content
        .blocks
        .iter()
        .position(|b| matches!(b, Block::List { items, .. } if !items.is_empty()))
        .unwrap();
    let Block::List { items, .. } = &content.blocks[index] else {
        panic!("list");
    };
    let row = items[0].clone();
    content.blocks[index] = Block::Heading {
        level: 3,
        item_id: row.item_id,
        content: row.content,
    };
    content.validate(&input).unwrap();
    let markdown = content.markdown(&input).unwrap();
    assert!(markdown.contains("### 点検日を確認する"));
    assert!(markdown.contains("発言ID: utterance"));
    let Block::Heading { item_id, .. } = &mut content.blocks[index] else {
        unreachable!();
    };
    *item_id = None;
    assert!(content.validate(&input).is_err());
}

#[test]
fn utc_dates_unknown_participants_empty_output_and_manual_additions_are_explicit() {
    assert_eq!(utc_label(0).as_deref(), Some("1970-01-01 00:00:00 UTC"));
    assert_eq!(utc_label(-1).as_deref(), Some("1969-12-31 23:59:59 UTC"));
    assert_eq!(
        utc_label(951782400000).as_deref(),
        Some("2000-02-29 00:00:00 UTC")
    );
    assert_eq!(utc_label(i64::MAX), None);
    let mut input = input();
    input.meeting_started_at_utc_ms = None;
    input.analysis.speakers[0].display_name = None;
    let mut content = from_generated(&input, vec![]).unwrap();
    let text = content.markdown(&input).unwrap();
    assert!(text.contains("会議日時\\: 未確認"));
    assert!(text.contains("確認済み参加者\\: 未確認"));
    assert!(!text.contains("話者A"));
    assert!(text.contains("抽出項目なし"));
    assert!(content.evidence.is_empty());
    content.items.push(MinutesItem {
        id: "manual".into(),
        kind: ItemKind::OpenIssue,
        body: "手動で追加した課題".into(),
        assignee_name: None,
        assignee_speaker_id: None,
        due_date: None,
        due_text: None,
        source_type: SourceType::Manual,
    });
    content.blocks.push(Block::Paragraph {
        item_id: Some("manual".into()),
        content: vec![Inline::text("手動で追加した課題")],
    });
    assert!(content.validate(&input).is_ok());
    content.blocks.push(Block::Paragraph {
        item_id: Some("manual".into()),
        content: vec![Inline::text("手動で追加した課題")],
    });
    assert!(content.validate(&input).is_err());
}
