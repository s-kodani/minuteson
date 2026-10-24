use super::*;
use crate::storage::current_minutes_fixture::input;

#[test]
fn current_prompts_preserve_metadata_and_partition_every_utf8_byte_with_rounded_links() {
    let mut input = input();
    input.analysis.utterances[0].effective_text = "点検日。\"引用\"\\パス\n".repeat(500);
    input.analysis.utterances[0].start_us = 100_001;
    input.analysis.utterances[0].end_us = 999_999;
    let whole = extract_prompt(
        &input,
        &[Slice {
            utterance_index: 0,
            first_byte: 0,
            end_byte: input.analysis.utterances[0].effective_text.len(),
        }],
    )
    .unwrap();
    let whole: serde_json::Value = serde_json::from_str(&whole).unwrap();
    assert_eq!(whole["meetingTitle"], input.meeting_title);
    assert_eq!(
        whole["startedAtUtcMs"],
        input.meeting_started_at_utc_ms.unwrap()
    );
    let utterance = &whole["utterances"][0];
    assert_eq!(
        utterance["text"],
        input.analysis.utterances[0].effective_text
    );
    assert_eq!(utterance["speakers"], serde_json::json!(["田中"]));
    for key in ["start", "end", "partial"] {
        assert!(utterance.get(key).is_none(), "{key}");
    }
    assert_eq!(
        audio_links_with_version(&input, PromptVersion::CustomPromptsV5).unwrap()[0],
        utterance["audioLink"].as_str().unwrap()
    );
    let plan = build(&input, 5400).unwrap();
    assert_eq!(plan, build(&input, 5400).unwrap());
    let mut text = String::new();
    let mut extracts = 0;
    for (index, node) in plan.nodes.iter().enumerate() {
        match node {
            Node::Extract { slices } => {
                extracts += 1;
                let prompt = extract_prompt(&input, slices).unwrap();
                assert!(fits(markdown::system(), &prompt, 5400));
                let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
                assert_eq!(
                    value["utterances"][0]["audioLink"],
                    "[00:00:00–00:00:01](audio:00:00:00-00:00:01)"
                );
                assert!(!prompt.contains("utteranceId"));
                assert!(!prompt.contains("speakerId"));
                assert!(!prompt.contains(&format!("\"{}\"", input.analysis.utterances[0].id)));
                assert_eq!(value["utterances"][0]["partial"], true);
                assert_eq!(
                    value["utterances"][0]["speakers"],
                    serde_json::json!(["田中"])
                );
                for slice in slices {
                    assert_eq!(slice.first_byte, text.len());
                    text.push_str(
                        &input.analysis.utterances[slice.utterance_index].effective_text
                            [slice.first_byte..slice.end_byte],
                    );
                    assert_eq!(slice.end_byte, text.len());
                }
            }
            Node::Merge { left, right } => {
                assert!(*left < index && *right < index && left != right);
            }
        }
    }
    assert_eq!(text, input.analysis.utterances[0].effective_text);
    assert!(extracts > 1);
    assert_eq!(plan.nodes.len(), extracts * 2 - 1);
}
#[test]
fn empty_or_impossible_inputs_fail_without_admitting_a_truncated_plan() {
    let mut input = input();
    for value in [0, 2047, 262145, usize::MAX] {
        assert!(build(&input, value).is_err());
    }
    let end = input.analysis.utterances[0].end_us;
    let start = input.analysis.utterances[0].start_us;
    for invalid in [-1, start - 1, start] {
        input.analysis.utterances[0].end_us = invalid;
        assert!(build(&input, 5400).is_err(), "end={invalid}");
    }
    input.analysis.utterances[0].end_us = end;
    input.meeting_title = "巨大な会議名".repeat(1000);
    assert!(build(&input, 5400).is_err());
    input.meeting_title = "会議".into();
    for u in &mut input.analysis.utterances {
        u.effective_text = " \n".into();
    }
    assert!(build(&input, 5400).is_err());
}
#[test]
fn reviewed_large_context_admits_complete_plans_without_changing_legacy_plans() {
    let input = input();
    let legacy = build(&input, 5400).unwrap();
    for limit in [60001, 131072, 262144] {
        let plan = build(&input, limit).unwrap();
        validate_coverage(&input, &plan).unwrap();
        assert_eq!(plan, legacy);
    }
}
#[test]
fn merge_compression_consumes_all_parts_and_replays_deterministically() {
    let parts = vec![
        "左\"\n[区間](audio:00:20:11.100001-00:20:25.799999)\n".repeat(100),
        "右\\\n[区間](audio:01:00:12.000001-01:00:17.999999)\n".repeat(100),
    ];
    let mut cursor = Cursor::new(parts.clone(), 5400).unwrap();
    let mut reconstructed = String::new();
    let mut saved = vec![];
    while cursor.round == 0 && cursor.result().is_none() {
        let prompt = cursor.task().unwrap();
        assert_eq!(
            cursor.system(),
            PromptVersion::CustomPromptsV5.merge_system()
        );
        assert!(fits(MERGE_SYSTEM, &prompt, 5400));
        let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
        for p in value["drafts"].as_array().unwrap() {
            reconstructed.push_str(p.as_str().unwrap());
        }
        let result = format!("要約 {}", saved.len());
        saved.push((prompt, result.clone()));
        cursor.accept(result).unwrap();
    }
    assert_eq!(reconstructed, parts.concat());
    assert!(saved.len() > 1);
    while cursor.result().is_none() {
        let prompt = cursor.task().unwrap();
        saved.push((prompt, "# 統合した本文\n".into()));
        cursor.accept("# 統合した本文\n".into()).unwrap();
    }
    let mut resumed = Cursor::new(parts, 5400).unwrap();
    for (prompt, result) in saved {
        assert_eq!(resumed.task().unwrap(), prompt);
        resumed.accept(result).unwrap();
    }
    assert_eq!(resumed.result(), cursor.result());
    assert!(resumed.task().is_err());
    assert!(resumed.accept("余分な結果".into()).is_err());
}
#[test]
fn nonconverging_large_output_is_bounded_instead_of_cutting_off_drafts() {
    let mut cursor = Cursor::new(vec!["大きい本文".repeat(250)], 5400).unwrap();
    let mut failed = false;
    for _ in 0..MAX_BATCHES * MAX_ROUNDS {
        let result = cursor.accept("収束しない要約".repeat(100));
        if result.is_err() {
            failed = true;
            break;
        }
        assert!(cursor.result().is_none());
    }
    assert!(failed);
    assert!(Cursor::new(vec!["".into()], 5400).is_err());
}

#[test]
fn split_paragraph_keeps_its_trailing_citation_available_before_the_link_enters_the_fragment() {
    for version in [PromptVersion::CustomPromptsV5] {
        let link = "[00:20:11–00:20:26](audio:00:20:11-00:20:26)";
        let source = format!(
            "# 議事録\n\n## 要約\n{}。{link}",
            "設計を検討する".repeat(200)
        );
        let mut cursor = Cursor::with_version(vec![source.clone()], 5400, version).unwrap();
        let mut rebuilt = String::new();
        while cursor.round == 0 && cursor.result().is_none() {
            let prompt = cursor.task().unwrap();
            assert!(fits(cursor.system(), &prompt, 5400));
            let output = format!("# 議事録\n\n## 要約\n設計を検討する。{link}");
            let data: serde_json::Value = serde_json::from_str(&prompt).unwrap();
            for part in data["drafts"].as_array().unwrap() {
                rebuilt.push_str(part.as_str().unwrap());
            }
            cursor.accept(output).unwrap();
        }
        assert_eq!(rebuilt, source);
    }
}

#[test]
fn second_intervals_cover_subsecond_utterances_boundaries_and_large_values() {
    let v = PromptVersion::CustomPromptsV5;
    for (a, b, start, end) in [
        (1, 2, "00:00:00", "00:00:01"),
        (608_800_000, 614_200_000, "00:10:08", "00:10:15"),
        (1_211_100_001, 1_225_799_999, "00:20:11", "00:20:26"),
        (999_999, 1_000_000, "00:00:00", "00:00:01"),
        (3_599_999_999, 3_600_000_001, "00:59:59", "01:00:01"),
    ] {
        assert_eq!(interval(a, b, v).unwrap(), (start.into(), end.into()));
    }
    assert!(interval(i64::MAX - 1, i64::MAX, v).is_ok());
    for (a, b) in [(-1, 1), (1, 1), (2, 1)] {
        assert!(interval(a, b, v).is_err());
    }
}

#[test]
fn evaluated_token_partition_keeps_all_source_and_replays_without_a_model() {
    let mut input = input();
    let utterance = input.analysis.utterances[0].clone();
    input.analysis.utterances = (0..150)
        .map(|i| {
            let mut u = utterance.clone();
            u.id = format!("u{i}");
            u.effective_text = format!("確認事項{i}を報告します。");
            u
        })
        .collect();
    let baseline = build_with_version(&input, 54000, PromptVersion::CustomPromptsV5).unwrap();
    let (scope, engine, policy) = super::super::optimization::Scope::fixture();
    let optimized = scope
        .check(build_with_version(
            &input,
            54000,
            PromptVersion::CustomPromptsV5,
        ))
        .unwrap();
    validate_coverage(&input, &optimized).unwrap();
    assert!(optimized.nodes.len() < baseline.nodes.len());
    let counts = super::super::optimization::counts();
    drop(scope);
    let replay = super::super::optimization::Scope::replay(&engine, Some(&policy), &counts)
        .unwrap()
        .unwrap();
    assert_eq!(
        replay
            .check(build_with_version(
                &input,
                54000,
                PromptVersion::CustomPromptsV5
            ))
            .unwrap(),
        optimized
    );
    let mut broken = optimized.clone();
    if let Node::Extract { slices } = &mut broken.nodes[0] {
        slices.remove(0);
    }
    assert!(validate_coverage(&input, &broken).is_err());
    drop(replay);
    let replay =
        super::super::optimization::Scope::replay(&engine, Some(&policy), &Default::default())
            .unwrap()
            .unwrap();
    assert!(replay
        .check(build_with_version(
            &input,
            54000,
            PromptVersion::CustomPromptsV5
        ))
        .is_err());
}
