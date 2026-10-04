use super::*;
use crate::minutes_document::tests::input;

#[test]
fn utf8_partition_has_no_loss_or_duplicate_and_a_fixed_merge_tree() {
    let mut input = input();
    input.analysis.utterances[0].effective_text = "点検日です。\"引用\"\\パス\n".repeat(300);
    let mut empty = input.analysis.utterances[0].clone();
    empty.id = "empty".into();
    empty.effective_text = "  ".into();
    input.analysis.utterances.push(empty);
    let plan = build(&input, 2400).unwrap();
    assert_eq!(plan, build(&input, 2400).unwrap());
    let mut text = String::new();
    let mut count = 0;
    let mut nodes = vec![];
    for (index, node) in plan.nodes.iter().enumerate() {
        match node {
            Node::Extract { slices } => {
                count += 1;
                let (prompt, scoped, ids) = extract_prompt(&input, slices).unwrap();
                assert!(prompt.len() + EXTRACT_SYSTEM.len() + 256 <= 2400);
                assert!(!ids.contains(&"empty".to_string()));
                for (slice, u) in slices.iter().zip(scoped.analysis.utterances) {
                    assert_eq!(slice.first_byte, text.len());
                    text.push_str(&u.effective_text);
                    assert_eq!(u.start_us, input.analysis.utterances[0].start_us);
                    assert_eq!(u.speaker_ids, input.analysis.utterances[0].speaker_ids);
                }
                nodes.push(vec![index]);
            }
            Node::Merge { left, right } => {
                assert!(*left < index && *right < index && left != right);
                let mut owned = nodes[*left].clone();
                owned.extend(&nodes[*right]);
                nodes.push(owned);
            }
        }
    }
    assert!(count > 2);
    assert_eq!(plan.nodes.len(), 2 * count - 1);
    assert_eq!(text, input.analysis.utterances[0].effective_text);
    assert_eq!(nodes.last().unwrap().len(), count);
    let mut unique = nodes.last().unwrap().clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), count);
}
#[test]
fn blank_input_is_a_local_empty_unit_and_metadata_cannot_be_truncated() {
    let mut input = input();
    input.analysis.utterances[0].effective_text = "\n ".into();
    assert_eq!(
        build(&input, 2400).unwrap().nodes,
        vec![Node::Extract { slices: vec![] }]
    );
    assert!(build(&input, 1000).is_err());
    input.meeting_title = "x".repeat(60000);
    assert!(build(&input, 2400).is_err());
    assert!(extract_prompt(
        &input,
        &[Slice {
            utterance_index: 0,
            first_byte: 0,
            end_byte: 100
        }]
    )
    .is_err());
}
