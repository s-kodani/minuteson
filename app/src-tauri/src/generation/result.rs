use crate::minutes_document::{from_generated, GeneratedItem, MinutesContent};
use crate::storage::MinutesInput;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) const RESULT_SCHEMA_VERSION: u32 = 1;

// Apply only after strict parsing/grounding validation. Keep the first item's
// text, metadata and evidence order; equality requires all seven fields, with
// evidence treated as a set. Different sources or assignments remain distinct.
// Saved v1-v3 jobs must replay their original (possibly repeated) items.
pub(super) fn normalize_versioned(
    items: Vec<GeneratedItem>,
    version: u32,
) -> Result<Vec<GeneratedItem>, String> {
    match version {
        1..=3 => return Ok(items),
        4 => (),
        _ => return Err("未対応の生成条件です".into()),
    }
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let mut key = item.clone();
        key.evidence_utterance_ids.sort();
        if seen.insert(serde_json::to_vec(&key).map_err(|e| e.to_string())?) {
            normalized.push(item);
        }
    }
    Ok(normalized)
}
const FIELDS: [&str; 7] = [
    "kind",
    "body",
    "assigneeName",
    "assigneeSpeakerId",
    "dueDate",
    "dueText",
    "evidenceUtteranceIds",
];

// The model returns semantic items and source utterance IDs only. Application
// UUIDs, fixed analysis IDs, times, quotations and the rich document are not
// part of this protocol and cannot be invented by the model.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Output {
    schema_version: u32,
    items: Vec<GeneratedItem>,
}

#[allow(dead_code)]
pub(crate) fn schema(input: &MinutesInput, utterance_ids: &[String]) -> Result<Value, String> {
    let unique: HashSet<_> = utterance_ids.iter().collect();
    if utterance_ids.is_empty()
        || utterance_ids.len() > 1000
        || unique.len() != utterance_ids.len()
        || unique
            .iter()
            .any(|id| !input.analysis.utterances.iter().any(|u| &u.id == *id))
    {
        return Err("生成単位の発言IDが空・重複・上限超過です".into());
    }
    let optional = json!({"type":["string","null"],"minLength":1,"maxLength":200});
    // Do not let the grammar invent a speaker UUID. Only a confirmed display
    // name literally present in this unit can possibly ground an assignment.
    // The application still checks the name/ID pair and the item's own evidence.
    let mut assignees = vec![Value::Null];
    for speaker in &input.analysis.speakers {
        if speaker.display_name.as_ref().is_some_and(|name| {
            !name.trim().is_empty()
                && input
                    .analysis
                    .utterances
                    .iter()
                    .any(|u| unique.contains(&u.id) && u.effective_text.contains(name))
        }) {
            assignees.push(json!(speaker.id));
        }
    }
    let mut item = json!({"type":"object", "additionalProperties":false,
        "required":FIELDS,
        "properties":{
            "kind":{"type":"string","enum":["summary","topic","decision","open_issue"]},
            "body":{"type":"string","minLength":1,"maxLength":20000},
            "assigneeName":{"type":"null"},
            "assigneeSpeakerId":{"type":"null"},
            "dueDate":{"type":"null"},
            "dueText":{"type":"null"},
            "evidenceUtteranceIds":{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,
                "items":{"type":"string","enum":utterance_ids}}
        }
    });
    let non_todo = item.clone();
    item["properties"]["kind"] = json!({"type":"string","enum":["todo"]});
    item["properties"]["assigneeName"] = optional.clone();
    item["properties"]["assigneeSpeakerId"] = json!({"enum":assignees});
    item["properties"]["dueDate"] =
        json!({"type":["string","null"],"pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"});
    item["properties"]["dueText"] = optional;
    Ok(json!({"type":"object", "additionalProperties":false,
        "required":["schemaVersion","items"],
        "properties":{
            "schemaVersion":{"type":"integer","enum":[RESULT_SCHEMA_VERSION]},
            "items":{"type":"array","maxItems":1000,"items":{"oneOf":[non_todo,item]}}
        }
    }))
}

#[allow(dead_code)]
pub(crate) fn parse_items(
    value: Value,
    input: &MinutesInput,
    unit_utterance_ids: &[String],
) -> Result<Vec<GeneratedItem>, String> {
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
        return Err("生成項目の出力が大きすぎます".into());
    }
    let valid_ids: HashSet<_> = input.analysis.utterances.iter().map(|u| &u.id).collect();
    let scoped: HashSet<_> = unit_utterance_ids.iter().collect();
    if unit_utterance_ids.len() > 1000
        || scoped.len() != unit_utterance_ids.len()
        || scoped.iter().any(|id| !valid_ids.contains(*id))
    {
        return Err("生成単位の発言IDが固定入力と一致しません".into());
    }
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or("生成項目が配列ではありません")?;
    if items.len() > 1000
        || items
            .iter()
            .any(|item| FIELDS.iter().any(|field| item.get(*field).is_none()))
    {
        return Err("生成項目の件数・必須フィールドが不正です".into());
    }
    let output: Output =
        serde_json::from_value(value).map_err(|e| format!("生成項目JSONが不正です: {e}"))?;
    if output.schema_version != RESULT_SCHEMA_VERSION
        || output.items.iter().any(|item| {
            item.evidence_utterance_ids
                .iter()
                .any(|id| !scoped.contains(id))
        })
    {
        return Err("生成項目のschemaまたは根拠IDが処理単位と一致しません".into());
    }
    // The grammar is only a generation aid. Always perform the full application
    // validation again, including grounding, even for constrained JSON.
    from_generated(input, output.items.clone())?;
    Ok(output.items)
}

#[allow(dead_code)]
pub(crate) fn document(
    value: Value,
    input: &MinutesInput,
    unit_utterance_ids: &[String],
) -> Result<MinutesContent, String> {
    from_generated(input, parse_items(value, input, unit_utterance_ids)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minutes_document::{
        tests::{input, item},
        ItemKind,
    };
    fn output() -> Value {
        json!({"schemaVersion":1,"items":[item(ItemKind::Decision)]})
    }
    #[test]
    fn exact_repeat_normalization_is_versioned_stable_and_metadata_sensitive() {
        let a = item(ItemKind::Decision);
        let mut b = a.clone();
        b.body = "異なる決定事項".into();
        let raw = vec![a.clone(), b.clone(), a.clone(), b.clone()];
        for version in 1..=3 {
            assert_eq!(normalize_versioned(raw.clone(), version).unwrap(), raw);
        }
        assert_eq!(normalize_versioned(raw, 4).unwrap(), vec![a.clone(), b]);
        let mut variants = vec![a.clone()];
        for field in 0..6 {
            let mut changed = a.clone();
            match field {
                0 => changed.kind = ItemKind::Summary,
                1 => changed.assignee_name = Some("田中".into()),
                2 => changed.assignee_speaker_id = Some("speaker".into()),
                3 => changed.due_date = Some("2026-10-12".into()),
                4 => changed.due_text = Some("明日".into()),
                _ => changed.evidence_utterance_ids = vec!["another-source".into()],
            }
            variants.push(changed);
        }
        assert_eq!(normalize_versioned(variants.clone(), 4).unwrap(), variants);
        let mut ordered = a;
        ordered.evidence_utterance_ids = vec!["second".into(), "first".into()];
        let mut reversed = ordered.clone();
        reversed.evidence_utterance_ids.reverse();
        assert_eq!(
            normalize_versioned(vec![ordered.clone(), reversed], 4).unwrap(),
            vec![ordered]
        );
        for version in [0, 5, u32::MAX] {
            assert!(normalize_versioned(vec![], version).is_err());
        }
    }
    #[test]
    fn normalization_cannot_bypass_validation_of_repeated_invalid_items() {
        let input = input();
        let ids = vec!["utterance".into()];
        let mut invalid = item(ItemKind::Decision);
        invalid.evidence_utterance_ids.push("utterance".into());
        let raw = json!({"schemaVersion":1,"items":[invalid.clone(),invalid]});
        assert!(parse_items(raw, &input, &ids).is_err());
    }
    #[test]
    fn result_protocol_builds_fixed_document_without_trusting_model_ids_or_times() {
        let input = input();
        let ids = vec!["utterance".into()];
        let content = document(output(), &input, &ids).unwrap();
        assert_eq!(content.evidence[0].analysis_revision_id, input.analysis.id);
        assert_eq!(
            content.evidence[0].quote_text,
            input.analysis.utterances[0].effective_text
        );
        assert!(content
            .markdown(&input)
            .unwrap()
            .contains("点検日を確認する"));
        let schema = schema(&input, &ids).unwrap();
        assert_eq!(
            schema["properties"]["items"]["items"]["oneOf"][0]["properties"]
                ["evidenceUtteranceIds"]["items"]["enum"],
            json!(ids)
        );
        assert!(
            parse_items(json!({"schemaVersion":1,"items":[]}), &input, &[])
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn result_rejects_missing_fields_extra_fields_unseen_ids_and_wrong_schema() {
        let input = input();
        let ids = vec!["utterance".into()];
        for field in &FIELDS {
            let mut bad = output();
            bad["items"][0].as_object_mut().unwrap().remove(*field);
            assert!(parse_items(bad, &input, &ids).is_err(), "{field}");
        }
        for field in [
            "id",
            "analysisRevisionId",
            "startUs",
            "quoteText",
            "sourceItemIds",
        ] {
            let mut bad = output();
            bad["items"][0][field] = json!("invented");
            assert!(parse_items(bad, &input, &ids).is_err(), "{field}");
        }
        let mut bad = output();
        bad["schemaVersion"] = json!(2);
        assert!(parse_items(bad, &input, &ids).is_err());
        let mut bad = output();
        bad["items"][0]["assigneeName"] = json!("佐藤");
        assert!(parse_items(bad, &input, &ids).is_err());
        assert!(parse_items(output(), &input, &[]).is_err());
        assert!(parse_items(output(), &input, &["missing".into()]).is_err());
        assert!(schema(&input, &[]).is_err());
        assert!(schema(&input, &["utterance".into(), "utterance".into()]).is_err());
        assert!(schema(&input, &["missing".into()]).is_err());
    }
    #[test]
    fn grammar_restricts_assignment_to_todo_and_groundable_confirmed_speakers() {
        let mut input = input();
        let ids = vec![input.analysis.utterances[0].id.clone()];
        let speaker_id = input.analysis.speakers[0].id.clone();
        input.analysis.speakers[0].display_name = Some("田中".into());
        input.analysis.utterances[0].effective_text = "田中が点検日を確認する".into();
        let branches =
            schema(&input, &ids).unwrap()["properties"]["items"]["items"]["oneOf"].clone();
        assert_eq!(
            branches[0]["properties"]["kind"]["enum"],
            json!(["summary", "topic", "decision", "open_issue"])
        );
        for field in ["assigneeName", "assigneeSpeakerId", "dueDate", "dueText"] {
            assert_eq!(branches[0]["properties"][field], json!({"type":"null"}));
        }
        assert_eq!(branches[1]["properties"]["kind"]["enum"], json!(["todo"]));
        assert_eq!(
            branches[1]["properties"]["assigneeSpeakerId"]["enum"],
            json!([null, speaker_id])
        );
        // A partial extraction must not use a name outside its visible text.
        input.analysis.utterances[0].effective_text = "点検日を確認する".into();
        assert_eq!(
            schema(&input, &ids).unwrap()["properties"]["items"]["items"]["oneOf"][1]["properties"]
                ["assigneeSpeakerId"]["enum"],
            json!([null])
        );
    }
}
