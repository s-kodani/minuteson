use super::*;
use crate::minutes_document::{
    Evidence, MinutesContent, MinutesItem, VerificationState, TEMPLATE_VERSION,
};
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
    // A parsed worker result never adopts itself or replaces a user's draft.
    #[allow(dead_code)]
    pub(crate) fn finish_minutes_candidate(
        &self,
        job_id: &str,
        completed_units: i64,
        content: MinutesContent,
    ) -> Result<String, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let job = super::jobs::load_job(&tx, job_id)?.ok_or("議事録ジョブがありません")?;
        let (spec, _) = super::jobs::read_fixed_input(&tx, job_id)?;
        if job.kind != "minutes_generation"
            || job.state != "running"
            || job.completed_units != completed_units
            || job.total_units != Some(completed_units)
            || job.output_minutes_id.is_some()
        {
            return Err("議事録ジョブの状態・完了位置が一致しません".into());
        }
        if spec.parameters["generation"]["templateVersion"] != TEMPLATE_VERSION {
            return Err("未対応の議事録テンプレートです".into());
        }
        let input = super::minutes_input::read_input(&tx, &spec)?;
        content.validate(&input)?;
        if content
            .items
            .iter()
            .any(|i| i.source_type != crate::minutes_document::SourceType::Engine)
            || content
                .evidence
                .iter()
                .any(|e| e.verification_state != VerificationState::Unverified)
        {
            return Err("生成ワーカーは手動追加・利用者確認済みとして保存できません".into());
        }
        let document = serde_json::to_string(&content).map_err(|e| e.to_string())?;
        let profile = serde_json::to_string(&spec.profile_snapshot).map_err(|e| e.to_string())?;
        let (parent,parent_hash):(Option<String>,Option<String>)=tx.query_row("SELECT r.id,r.content_hash FROM meetings m LEFT JOIN minutes_revisions r ON r.id=m.current_minutes_id WHERE m.id=?1",params![spec.meeting_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if let (Some(parent), Some(hash)) = (&parent, &parent_hash) {
            if super::revisions::minutes_content_hash(&tx, parent)? != *hash {
                return Err("現在の議事録版が変更されています".into());
            }
        }
        let number: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(revision_no),0)+1 FROM minutes_revisions WHERE meeting_id=?1",
                params![spec.meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        tx.execute("INSERT INTO minutes_revisions (id,meeting_id,input_analysis_id,parent_revision_id,parent_content_hash,revision_no,publication_state,generated_document_json,effective_document_json,document_schema_version,input_content_hash,content_hash,model_snapshot_json,template_version,created_at) VALUES (?1,?2,?3,?4,?5,?6,'candidate',?7,?7,1,?8,?9,?10,?11,?12)",params![id,spec.meeting_id,input.analysis.id,parent,parent_hash,number,document,spec.input_hash,"0".repeat(64),profile,TEMPLATE_VERSION,now]).map_err(|e|e.to_string())?;
        insert_projections(&tx, &id, &input.analysis.id, &content)?;
        let hash = super::revisions::minutes_content_hash(&tx, &id)?;
        tx.execute("UPDATE minutes_revisions SET content_hash=?1 WHERE id=?2 AND publication_state='candidate'",params![hash,id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE processing_jobs SET state='succeeded',output_minutes_id=?1,finished_at=?2,updated_at=?2 WHERE id=?3 AND state='running'",params![id,now,job_id]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

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
        let row:Option<(String,String,String,String,String,i64,String)>=tx.query_row("SELECT r.input_analysis_id,r.input_content_hash,r.content_hash,r.effective_document_json,r.generated_document_json,r.document_schema_version,r.template_version FROM minutes_revisions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2 AND (r.publication_state='candidate' OR (r.publication_state='committed' AND r.id IN (m.current_minutes_id,m.previous_minutes_id)))",params![revision_id,meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
        let Some((analysis, analysis_hash, hash, document, generated, schema, template)) = row
        else {
            return Ok(None);
        };
        if schema != 1 || template != TEMPLATE_VERSION {
            return Err("未対応の議事録形式・テンプレートです".into());
        }
        if super::revisions::minutes_content_hash(&tx, revision_id)? != hash {
            return Err("議事録の内容が変更されています".into());
        }
        let input =
            super::minutes_input::read_snapshot(&tx, meeting_id, &analysis, &analysis_hash)?;
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
}

fn insert_projections(
    tx: &Transaction<'_>,
    revision: &str,
    analysis: &str,
    content: &MinutesContent,
) -> Result<(), String> {
    for (order, item) in content.items.iter().enumerate() {
        tx.execute("INSERT INTO minutes_items (id,minutes_revision_id,analysis_revision_id,kind,order_no,body,assignee_name,assignee_speaker_id,due_date,due_text,source_type) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![item.id,revision,analysis,item.kind.as_str(),order as i64,item.body,item.assignee_name,item.assignee_speaker_id,item.due_date,item.due_text,item.source_type.as_str()]).map_err(|e|e.to_string())?;
    }
    for e in &content.evidence {
        tx.execute("INSERT INTO evidence_links (id,minutes_revision_id,minutes_item_id,analysis_revision_id,utterance_id,quote_text,start_us,end_us,source_type,verification_state) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![e.id,revision,e.minutes_item_id,analysis,e.utterance_id,e.quote_text,e.start_us,e.end_us,e.source_type.as_str(),e.verification_state.as_str()]).map_err(|e|e.to_string())?;
    }
    Ok(())
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
mod tests;
