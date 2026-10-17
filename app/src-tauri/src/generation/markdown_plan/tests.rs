use super::*;
use crate::storage::current_minutes_fixture::input;

#[test]
fn plain_prompts_partition_every_utf8_byte_without_ids_and_keep_fractional_times() {
    let mut input = input();
    input.analysis.utterances[0].effective_text = "点検日。\"引用\"\\パス\n".repeat(500);
    input.analysis.utterances[0].start_us = 100_001;
    input.analysis.utterances[0].end_us = 999_999;
    let plan = build(&input, 2400).unwrap();
    assert_eq!(plan, build(&input, 2400).unwrap());
    let mut text = String::new();
    let mut extracts = 0;
    for (index, node) in plan.nodes.iter().enumerate() {
        match node {
            Node::Extract { slices } => {
                extracts += 1;
                let prompt = extract_prompt(&input, slices).unwrap();
                assert!(fits(markdown::system(), &prompt, 2400));
                let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
                assert_eq!(value["utterances"][0]["start"], "00:00:00.100001");
                assert_eq!(value["utterances"][0]["end"], "00:00:00.999999");
                assert!(!prompt.contains("utteranceId"));
                assert!(!prompt.contains("speakerId"));
                assert!(!prompt.contains(&format!("\"{}\"", input.analysis.utterances[0].id)));
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
    for value in [0, 2047, 60001] {
        assert!(build(&input, value).is_err());
    }
    input.meeting_title = "巨大な会議名".repeat(1000);
    assert!(build(&input, 2400).is_err());
    input.meeting_title = "会議".into();
    for u in &mut input.analysis.utterances {
        u.effective_text = " \n".into();
    }
    assert!(build(&input, 2400).is_err());
}
#[test]
fn merge_compression_consumes_all_parts_and_replays_deterministically() {
    let parts = vec!["左\"\n".repeat(1000), "右\\\n".repeat(1000)];
    let mut cursor = Cursor::new(parts.clone(), 2400).unwrap();
    let mut reconstructed = String::new();
    let mut saved = vec![];
    while cursor.round == 0 && cursor.result().is_none() {
        let prompt = cursor.task().unwrap();
        assert!(fits(MERGE_SYSTEM, &prompt, 2400));
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
    let mut resumed = Cursor::new(parts, 2400).unwrap();
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
    let mut cursor = Cursor::new(vec!["大きい本文".repeat(250)], 2400).unwrap();
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
    assert!(Cursor::new(vec!["".into()], 2400).is_err());
}

#[test]
fn inline_evidence_plan_carries_exact_absolute_links_and_all_source_bytes_within_budget() {
    for version in [
        PromptVersion::InlineEvidenceV1,
        PromptVersion::InlineEvidenceV2,
    ] {
        let mut input = input();
        input.analysis.utterances.truncate(1);
        let u = &mut input.analysis.utterances[0];
        u.start_us = 1_211_100_001;
        u.end_us = 1_225_799_999;
        u.effective_text = "設計資料。\"引用\"\\資料\n".repeat(200);
        let planned = build_with_version(&input, 2400, version).unwrap();
        let mut rebuilt = String::new();
        let mut count = 0;
        for node in planned.nodes {
            if let Node::Extract { slices } = node {
                let prompt = extract_prompt_with_version(&input, &slices, version).unwrap();
                assert!(fits(version.extract_system(), &prompt, 2400));
                let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
                for item in value["utterances"].as_array().unwrap() {
                    assert_eq!(
                        item["audioLink"],
                        "[00:20:11.100001–00:20:25.799999](audio:00:20:11.100001-00:20:25.799999)"
                    );
                    rebuilt.push_str(item["text"].as_str().unwrap());
                }
                count += 1;
            }
        }
        assert!(count > 1);
        assert_eq!(rebuilt, input.analysis.utterances[0].effective_text);
        for end in [-1, 1_211_100_000, 1_211_100_001] {
            input.analysis.utterances[0].end_us = end;
            assert!(build_with_version(&input, 2400, version).is_err());
        }
    }
}

#[test]
fn inline_merge_partition_and_replay_keep_every_byte_and_link() {
    for version in [
        PromptVersion::InlineEvidenceV1,
        PromptVersion::InlineEvidenceV2,
    ] {
        let parts = vec![
            "## 要約\n設計資料を共有する。[区間](audio:00:20:11.100001-00:20:25.799999)\n"
                .repeat(40),
            "## 要約\nアラートを検討する。[区間](audio:01:00:12.000001-01:00:17.999999)\n"
                .repeat(40),
        ];
        let mut cursor = Cursor::with_version(parts.clone(), 2400, version).unwrap();
        let mut replay = Cursor::with_version(parts.clone(), 2400, version).unwrap();
        let mut consumed = String::new();
        while cursor.result().is_none() {
            let prompt = cursor.task().unwrap();
            assert_eq!(prompt, replay.task().unwrap());
            assert_eq!(cursor.system(), version.merge_system());
            assert!(fits(cursor.system(), &prompt, 2400));
            if cursor.round == 0 {
                let value: serde_json::Value = serde_json::from_str(&prompt).unwrap();
                for part in value["drafts"].as_array().unwrap() {
                    consumed.push_str(part.as_str().unwrap());
                }
            }
            let output =
            "## 要約\n設計資料とアラートを検討する。[区間](audio:00:20:11.100001-00:20:25.799999)";
            cursor.accept(output.into()).unwrap();
            replay.accept(output.into()).unwrap();
        }
        assert_eq!(consumed, parts.concat());
        assert_eq!(cursor.result(), replay.result());
    }
}

#[test]
fn split_paragraph_keeps_its_trailing_citation_available_before_the_link_enters_the_fragment() {
    for version in [
        PromptVersion::InlineEvidenceV1,
        PromptVersion::InlineEvidenceV2,
    ] {
        let link = "[00:20:11.100001–00:20:25.799999](audio:00:20:11.100001-00:20:25.799999)";
        let source = format!(
            "# 議事録\n\n## 要約\n{}。{link}",
            "設計を検討する".repeat(200)
        );
        let mut cursor = Cursor::with_version(vec![source.clone()], 2400, version).unwrap();
        let mut rebuilt = String::new();
        while cursor.round == 0 && cursor.result().is_none() {
            let prompt = cursor.task().unwrap();
            assert!(fits(cursor.system(), &prompt, 2400));
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
fn compact_revision_keeps_all_text_and_speakers_and_rounds_only_references_outwards() {
    let mut input = input();
    let u = &mut input.analysis.utterances[0];
    u.start_us = 608_800_000;
    u.end_us = 614_200_000;
    let whole = Slice {
        utterance_index: 0,
        first_byte: 0,
        end_byte: u.effective_text.len(),
    };
    let old: serde_json::Value = serde_json::from_str(
        &extract_prompt_with_version(&input, &[whole.clone()], PromptVersion::InlineEvidenceV2)
            .unwrap(),
    )
    .unwrap();
    let compact =
        extract_prompt_with_version(&input, &[whole], PromptVersion::CompactEvidenceV3).unwrap();
    let v: serde_json::Value = serde_json::from_str(&compact).unwrap();
    let u = &v["utterances"][0];
    assert_eq!(
        u["audioLink"],
        "[00:10:08–00:10:15](audio:00:10:08-00:10:15)"
    );
    assert_eq!(u["text"], old["utterances"][0]["text"]);
    assert_eq!(u["speakers"], old["utterances"][0]["speakers"]);
    assert!(u.get("start").is_none() && u.get("end").is_none() && u.get("partial").is_none());
    assert_eq!(v["meetingTitle"], old["meetingTitle"]);
    assert_eq!(v["startedAtUtcMs"], old["startedAtUtcMs"]);
    assert_eq!(input.analysis.utterances[0].start_us, 608_800_000);
    assert_eq!(input.analysis.utterances[0].end_us, 614_200_000);
    assert_eq!(
        audio_links_with_version(&input, PromptVersion::CompactEvidenceV3).unwrap()[0],
        u["audioLink"].as_str().unwrap()
    );
    assert!(!PromptVersion::CompactEvidenceV3
        .extract_system()
        .contains("小数秒を省略"));
    assert!(!PromptVersion::CompactEvidenceV3
        .merge_system()
        .contains("小数秒を含め"));
    let original = input.analysis.utterances[0].effective_text.repeat(300);
    input.analysis.utterances[0].effective_text = original.clone();
    let p = build_with_version(&input, 4000, PromptVersion::CompactEvidenceV3).unwrap();
    let mut rebuilt = String::new();
    for node in p.nodes {
        if let Node::Extract { slices } = node {
            let text =
                extract_prompt_with_version(&input, &slices, PromptVersion::CompactEvidenceV3)
                    .unwrap();
            assert!(fits(
                PromptVersion::CompactEvidenceV3.extract_system(),
                &text,
                4000
            ));
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            for u in v["utterances"].as_array().unwrap() {
                assert_eq!(u["partial"], true);
                rebuilt.push_str(u["text"].as_str().unwrap());
            }
        }
    }
    assert_eq!(rebuilt, original);
}
#[test]
fn second_intervals_cover_subsecond_utterances_boundaries_and_large_values() {
    let v = PromptVersion::CompactEvidenceV3;
    for (a, b, start, end) in [
        (1, 2, "00:00:00", "00:00:01"),
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
