use crate::minutes_document::{from_generated, GeneratedItem, MinutesContent};
use crate::storage::MinutesInput;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) const RESULT_SCHEMA_VERSION: u32 = 1;
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
pub(crate) fn schema(utterance_ids: &[String]) -> Result<Value, String> {
    let unique: HashSet<_> = utterance_ids.iter().collect();
    if utterance_ids.is_empty() || utterance_ids.len() > 1000 || unique.len() != utterance_ids.len()
    {
        return Err("生成単位の発言IDが空・重複・上限超過です".into());
    }
    let optional = json!({"type":["string","null"],"minLength":1,"maxLength":200});
    Ok(json!({"type":"object", "additionalProperties":false,
        "required":["schemaVersion","items"],
        "properties":{
            "schemaVersion":{"type":"integer","enum":[RESULT_SCHEMA_VERSION]},
            "items":{"type":"array","maxItems":1000,"items":{
                "type":"object", "additionalProperties":false,
                "required":FIELDS,
                "properties":{
                    "kind":{"type":"string","enum":["summary","topic","decision","open_issue","todo"]},
                    "body":{"type":"string","minLength":1,"maxLength":20000},
                    "assigneeName":optional,
                    "assigneeSpeakerId":{"type":["string","null"],"minLength":1,"maxLength":128},
                    "dueDate":{"type":["string","null"],"pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},
                    "dueText":optional,
                    "evidenceUtteranceIds":{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,
                        "items":{"type":"string","enum":utterance_ids}}
                }
            }}
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
        let schema = schema(&ids).unwrap();
        assert_eq!(
            schema["properties"]["items"]["items"]["properties"]["evidenceUtteranceIds"]["items"]
                ["enum"],
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
        assert!(schema(&[]).is_err());
        assert!(schema(&["utterance".into(), "utterance".into()]).is_err());
    }
}
