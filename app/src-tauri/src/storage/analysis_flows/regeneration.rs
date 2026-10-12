//! Explicit regeneration intent. The fixed replacement target is never a
//! meeting model preference, and failure never changes public text or drafts.
use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DraftStamp {
    pub id: String,
    pub content_hash: String,
    pub lock_version: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Target {
    pub analysis_id: String,
    pub content_hash: String,
    pub draft: Option<DraftStamp>,
}
impl Target {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let id = |v: &str| !v.is_empty() && v.len() <= 256 && !v.contains('\0');
        let hash = |v: &str| v.len() == 64 && v.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !id(&self.analysis_id) || !hash(&self.content_hash)
            || self.draft.as_ref().is_some_and(|d| !id(&d.id) || !hash(&d.content_hash) || d.lock_version < 0) {
            return Err("再生成の置き換え対象が不正です".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Admission {
    pub target: Target,
    pub flow: Option<FlowStamp>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct FlowStamp {
    pub id: String,
    pub lock_version: i64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub meeting_id: String,
    pub operation_id: String,
    pub expected: Admission,
}

fn target(db: &Transaction<'_>, meeting: &str) -> Result<Target, String> {
    let (analysis_id, content_hash): (String, String) = db.query_row(
        "SELECT a.id,a.content_hash FROM meetings m JOIN analysis_revisions a ON a.id=m.current_analysis_id AND a.meeting_id=m.id WHERE m.id=?1 AND m.lifecycle IN ('recorded','imported') AND a.publication_state='committed'",
        params![meeting], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|e|e.to_string())?.ok_or("先に文字起こしを完了してください")?;
    if super::super::revisions::analysis_content_hash(db, &analysis_id)? != content_hash {
        return Err("保存済みの文字起こしが変更されています".into());
    }
    let draft = super::super::drafts::load_draft(db, meeting)?;
    if draft.as_ref().is_some_and(|d| d.base_analysis_id != analysis_id) {
        return Err("保存済みの編集と文字起こしが一致しません".into());
    }
    Ok(Target { analysis_id, content_hash, draft: draft.map(|d| DraftStamp {
        id: d.id, content_hash: d.content_hash, lock_version: d.lock_version,
    }) })
}
pub(super) fn require_target(db: &Transaction<'_>, meeting: &str, expected: &Target) -> Result<(), String> {
    expected.validate()?;
    if target(db, meeting)? != *expected {
        return Err("再生成の開始後に文字起こしが編集されました。現在の本文と話者の修正を保持し、置き換えを停止しました。".into());
    }
    Ok(())
}
fn admission(db: &Transaction<'_>, meeting: &str) -> Result<Admission, String> {
    Ok(Admission { target: target(db, meeting)?, flow: read_flow(db, meeting)?.map(|f| FlowStamp { id:f.id, lock_version:f.lock_version }) })
}
fn replay(db: &Connection, request: &Request) -> Result<Option<AnalysisFlow>, String> {
    let Some(flow) = read_flow(db, &request.meeting_id)? else { return Ok(None); };
    if flow.id != request.operation_id { return Ok(None); }
    let plan = crate::analysis_flow::saved_plan(flow.plan.clone().ok_or("再生成の固定条件がありません")?, &flow.profile)?;
    if plan.replacement.as_ref() != Some(&request.expected.target) {
        return Err("同じ再生成操作の置き換え条件が一致しません".into());
    }
    Ok(Some(flow))
}

impl Storage {
    pub(crate) fn regeneration_replay(&self, request: &Request) -> Result<Option<AnalysisFlow>, String> {
        let db = self.connection.lock().map_err(|_| "DBロックを取得できません")?;
        replay(&db, request)
    }
    pub(crate) fn regeneration_admission(&self, meeting: &str) -> Result<Admission, String> {
        let mut db = self.connection.lock().map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e|e.to_string())?;
        admission(&tx, meeting)
    }
    pub(crate) fn regenerate_analysis(&self, request: &Request, cancelled: &impl Fn() -> bool) -> Result<AnalysisFlow, String> {
        self.require_data_writable()?;
        if Uuid::parse_str(&request.operation_id).map_err(|_| "再生成の操作IDが不正です")?.to_string() != request.operation_id {
            return Err("再生成の操作IDが不正です".into());
        }
        request.expected.target.validate()?;
        {
            let db = self.connection.lock().map_err(|_| "DBロックを取得できません")?;
            if let Some(flow) = replay(&db, request)? { return Ok(flow); }
        }
        let versions = (self.analysis_preferences()?.lock_version, self.diarization_preferences()?.lock_version);
        let mut plan = crate::analysis_flow::prepare(self, "standard", cancelled)?;
        plan.replacement = Some(request.expected.target.clone());
        let value = serde_json::to_value(&plan).map_err(|e|e.to_string())?;
        let json = serde_json::to_string(&value).map_err(|e|e.to_string())?;
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        let mut db = self.connection.lock().map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e|e.to_string())?;
        if let Some(flow) = replay(&tx, request)? { return Ok(flow); }
        let reused: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE operation_id IN (?1,?2))", params![format!("flow:{}:transcription",request.operation_id),format!("flow:{}:diarization",request.operation_id)], |r|r.get(0)).map_err(|e|e.to_string())?;
        if reused { return Err("過去の再生成操作IDは再利用できません".into()); }
        if cancelled() { return Err("文字起こしの再生成を開始していません".into()); }
        self.require_data_writable()?;
        if admission(&tx, &request.meeting_id)? != request.expected {
            return Err("確認後に文字起こしまたは解析状態が変わりました。現在の内容を保持しています。".into());
        }
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE meeting_id=?1 AND kind IN ('transcription','diarization') AND state NOT IN ('succeeded','failed','cancelled'))", params![request.meeting_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if active { return Err("進行中の文字起こしが終了してから再生成してください".into()); }
        if (super::super::preferences::load(&tx)?.lock_version, super::super::diarization_preferences::load(&tx)?.lock_version) != versions {
            return Err("開始準備中に全体設定が変更されました。設定を確認して再生成してください。".into());
        }
        super::super::model_profiles::jobs::gate_plan(&tx, "standard", &value)?;
        // A new immutable flow identity retains all old jobs/revisions. It does
        // not mutate their frozen plans, public heads or human edit history.
        tx.execute("DELETE FROM analysis_flows WHERE meeting_id=?1", params![request.meeting_id]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO analysis_flows(id,meeting_id,stage,plan_json,plan_hash,created_at,updated_at) VALUES(?1,?2,'transcription',?3,?4,?5,?5)", params![request.operation_id,request.meeting_id,json,hash,now_ms()?]).map_err(|e|e.to_string())?;
        let flow = read_flow(&tx, &request.meeting_id)?.ok_or("再生成の工程がありません")?;
        if cancelled() { return Err("文字起こしの再生成を開始していません".into()); }
        tx.commit().map_err(|e|e.to_string())?;
        Ok(flow)
    }
    pub(crate) fn ready_analysis_flows(&self) -> Result<Vec<AnalysisFlow>, String> {
        Ok(self.pending_analysis_flows()?.into_iter().filter(|f| f.ready_to_publish).collect())
    }
    pub(crate) fn publish_regenerated_flow(&self, meeting: &str, id: &str, version: i64) -> Result<AnalysisFlow, String> {
        self.require_data_writable()?;
        let flow = self.analysis_flow(meeting)?.ok_or("再生成の工程がありません")?;
        if flow.id != id { return Err("再生成の対象工程が変更されています".into()); }
        if flow.stage == "review" { return Ok(flow); } // Lost publication response: read-only replay.
        if flow.lock_version != version || !flow.ready_to_publish { return Err("再生成の完了状態が変更されています".into()); }
        match self.complete_analysis_flow(&flow) {
            Ok(next) => Ok(next),
            Err(cause) => {
                self.update_analysis_flow(&flow, "attention", flow.plan.clone(), flow.transcription_job.as_ref(), flow.diarization_job.as_ref(), Some(&cause))?;
                Err(cause)
            }
        }
    }
}
