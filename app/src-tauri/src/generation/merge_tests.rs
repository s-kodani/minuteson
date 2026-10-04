use super::*;
use crate::minutes_document::{
    tests::{input, item},
    ItemKind,
};
#[test]
fn merge_preserves_source_evidence_and_metadata_without_missing_or_double_used_items() {
    let input = input();
    let a = item(ItemKind::Todo);
    let mut b = a.clone();
    b.body = "点検日も確認する".into();
    let parents = vec![vec![a.clone()], vec![b]];
    let src = sources(0, 1, &parents).unwrap();
    let output = json!({"schemaVersion":1,"items":[{"body":"点検日を確認する","sourceItemIds":["n0-i0","n1-i0"]}]});
    let merged = parse(output.clone(), &input, &src).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].evidence_utterance_ids, a.evidence_utterance_ids);
    assert_eq!(merged[0].assignee_name, None);
    assert_eq!(merged[0].due_date, None);
    for bad in [
        json!({"schemaVersion":1,"items":[]}),
        json!({"schemaVersion":1,"items":[{"body":"x","sourceItemIds":["n0-i0"]}]}),
        json!({"schemaVersion":1,"items":[{"body":"x","sourceItemIds":["n0-i0","n0-i0","n1-i0"]}]}),
        json!({"schemaVersion":1,"items":[{"body":"x","sourceItemIds":["missing","n1-i0"]}]}),
        json!({"schemaVersion":1,"items":[{"body":"x","kind":"decision","sourceItemIds":["n0-i0","n1-i0"]}]}),
    ] {
        assert!(parse(bad, &input, &src).is_err());
    }
    let mut changed = src.clone();
    changed[1].1.kind = ItemKind::Decision;
    assert!(parse(output.clone(), &input, &changed).is_err());
    let mut changed = src.clone();
    changed[1].1.due_date = Some("2026-10-09".into());
    assert!(parse(output, &input, &changed).is_err());
    assert!(prompt(&src).unwrap().contains("sourceItemId"));
    assert_eq!(
        schema(&src)["properties"]["items"]["items"]["properties"]["sourceItemIds"]["items"]
            ["enum"],
        json!(["n0-i0", "n1-i0"])
    );
}
