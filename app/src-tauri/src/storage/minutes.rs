use super::*;
use crate::minutes_document::{Evidence, MinutesContent, MinutesItem, TEMPLATE_VERSION};
use rusqlite::{Transaction, TransactionBehavior};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesView {
    pub id: String,
    pub meeting_id: String,
    pub input_analysis_id: String,
    pub input_content_hash: String,
    pub content_hash: String,
    pub content: MinutesContent,
    pub generated_content: MinutesContent,
}

impl Storage {
    #[allow(dead_code)]
    pub(crate) fn load_minutes_content(
        &self,
        meeting_id: &str,
        revision_id: &str,
    ) -> Result<Option<MinutesView>, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        read_content(&tx, meeting_id, revision_id)
    }
}

pub(super) fn read_content(
    tx: &Transaction<'_>,
    meeting_id: &str,
    revision_id: &str,
) -> Result<Option<MinutesView>, String> {
    let row:Option<(String,String,String,String,String,i64,String)>=tx.query_row("SELECT r.input_analysis_id,r.input_content_hash,r.content_hash,r.effective_document_json,r.generated_document_json,r.document_schema_version,r.template_version FROM minutes_revisions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2 AND (r.publication_state='candidate' OR (r.publication_state='committed' AND r.id IN (m.current_minutes_id,m.previous_minutes_id)))",params![revision_id,meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
    let Some((analysis, analysis_hash, hash, document, generated, schema, template)) = row else {
        return Ok(None);
    };
    if schema != 1 || template != TEMPLATE_VERSION {
        return Err("未対応の議事録形式・テンプレートです".into());
    }
    if document.len() > 2 * 1024 * 1024 || generated.len() > 2 * 1024 * 1024 {
        return Err("議事録文書が大きすぎます".into());
    }
    if super::revisions::minutes_content_hash(&tx, revision_id)? != hash {
        return Err("議事録の内容が変更されています".into());
    }
    let input = super::minutes_input::read_snapshot(&tx, meeting_id, &analysis, &analysis_hash)?;
    let content: MinutesContent =
        serde_json::from_str(&document).map_err(|e| format!("議事録JSONが不正です: {e}"))?;
    let generated_content: MinutesContent =
        serde_json::from_str(&generated).map_err(|e| format!("生成原文JSONが不正です: {e}"))?;
    content.validate(&input)?;
    generated_content.validate(&input)?;
    check_projections(&tx, revision_id, &analysis, &content)?;
    Ok(Some(MinutesView {
        id: revision_id.into(),
        meeting_id: meeting_id.into(),
        input_analysis_id: analysis,
        input_content_hash: analysis_hash,
        content_hash: hash,
        content,
        generated_content,
    }))
}

fn check_projections(
    tx: &Transaction<'_>,
    revision: &str,
    analysis: &str,
    content: &MinutesContent,
) -> Result<(), String> {
    let decode =
        |value: String| serde_json::from_value(Value::String(value)).map_err(|e| e.to_string());
    let mut query=tx.prepare("SELECT id,kind,body,assignee_name,assignee_speaker_id,due_date,due_text,source_type,analysis_revision_id FROM minutes_items WHERE minutes_revision_id=?1 ORDER BY order_no").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![revision], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut items = vec![];
    for row in rows {
        let (id, kind, body, assignee_name, assignee_speaker_id, due_date, due_text, source, scope) =
            row.map_err(|e| e.to_string())?;
        if scope != analysis {
            return Err("議事録項目の入力版が一致しません".into());
        }
        items.push(MinutesItem {
            id,
            kind: serde_json::from_value(Value::String(kind)).map_err(|e| e.to_string())?,
            body,
            assignee_name,
            assignee_speaker_id,
            due_date,
            due_text,
            source_type: decode(source)?,
        });
    }
    if items != content.items {
        return Err("文書と構造化項目が一致しません".into());
    }
    let mut query=tx.prepare("SELECT id,minutes_item_id,analysis_revision_id,utterance_id,quote_text,start_us,end_us,source_type,verification_state FROM evidence_links WHERE minutes_revision_id=?1").map_err(|e|e.to_string())?;
    let rows = query
        .query_map(params![revision], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut evidence = vec![];
    for row in rows {
        let (
            id,
            minutes_item_id,
            analysis_revision_id,
            utterance_id,
            quote_text,
            start_us,
            end_us,
            source,
            state,
        ) = row.map_err(|e| e.to_string())?;
        evidence.push(Evidence {
            id,
            minutes_item_id,
            analysis_revision_id,
            utterance_id,
            quote_text,
            start_us,
            end_us,
            source_type: decode(source)?,
            verification_state: serde_json::from_value(Value::String(state))
                .map_err(|e| e.to_string())?,
        });
    }
    let mut expected = content.evidence.clone();
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    evidence.sort_by(|a, b| a.id.cmp(&b.id));
    if evidence != expected {
        return Err("文書と根拠情報が一致しません".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "minutes_tests.rs"]
pub(super) mod tests;
