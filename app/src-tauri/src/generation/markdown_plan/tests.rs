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
