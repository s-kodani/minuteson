use super::*;
use rusqlite::Transaction;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisFlow {
    pub id: String,
    pub meeting_id: String,
    pub stage: String,
    pub lock_version: i64,
    pub transcription_job: Option<JobSnapshot>,
    pub diarization_job: Option<JobSnapshot>,
    pub error_message: Option<String>,
    #[serde(skip)]
    pub plan: Option<Value>,
}

pub(super) fn ensure_flow(db: &Connection, meeting: &str) -> Result<(), String> {
    let recorded: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1 AND lifecycle='recorded')",
            params![meeting],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !recorded {
        return Err("音声保存の完了後に解析を開始してください".into());
    }
    let now = now_ms()?;
    db.execute("INSERT INTO analysis_flows(id,meeting_id,stage,created_at,updated_at) VALUES(?1,?2,'pending',?3,?3) ON CONFLICT(meeting_id) DO NOTHING",params![Uuid::new_v4().to_string(),meeting,now]).map_err(|e|e.to_string())?;
    Ok(())
}

fn read_flow(db: &Connection, meeting: &str) -> Result<Option<AnalysisFlow>, String> {
    let row:Option<(String,String,i64,Option<String>,Option<String>,Option<String>,Option<String>,Option<String>)>=db.query_row("SELECT id,stage,lock_version,transcription_job_id,diarization_job_id,error_message,plan_json,plan_hash FROM analysis_flows WHERE meeting_id=?1",params![meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, stage, lock_version, trans, diar, error_message, text, hash)) = row else {
        return Ok(None);
    };
    let plan = match (text, hash) {
        (Some(text), Some(hash))
            if text.len() <= 65536 && format!("{:x}", Sha256::digest(text.as_bytes())) == hash =>
        {
            Some(serde_json::from_str(&text).map_err(|e| e.to_string())?)
        }
        (None, None) => None,
        _ => return Err("解析工程の固定条件が変更されています".into()),
    };
    let load = |id: Option<String>| -> Result<Option<JobSnapshot>, String> {
        id.map(|id| super::jobs::load_job(db, &id)?.ok_or("工程ジョブがありません".into()))
            .transpose()
    };
    Ok(Some(AnalysisFlow {
        id,
        meeting_id: meeting.into(),
        stage,
        lock_version,
        transcription_job: load(trans)?,
        diarization_job: load(diar)?,
        error_message,
        plan,
    }))
}

impl Storage {
    pub(crate) fn analysis_flow(&self, meeting: &str) -> Result<Option<AnalysisFlow>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        read_flow(&db, meeting)
    }
    pub(crate) fn ensure_analysis_flow(&self, meeting: &str) -> Result<AnalysisFlow, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        ensure_flow(&tx, meeting)?;
        let flow = read_flow(&tx, meeting)?.ok_or("解析工程がありません")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(flow)
    }
    pub(crate) fn pending_analysis_flows(&self) -> Result<Vec<AnalysisFlow>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query=db.prepare("SELECT meeting_id FROM analysis_flows WHERE stage IN ('pending','transcription','diarization') ORDER BY created_at,rowid").map_err(|e|e.to_string())?;
        let ids = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        ids.iter()
            .map(|id| read_flow(&db, id)?.ok_or("解析工程がありません".into()))
            .collect()
    }
    pub(crate) fn update_analysis_flow(
        &self,
        old: &AnalysisFlow,
        stage: &str,
        plan: Option<Value>,
        trans: Option<&JobSnapshot>,
        diar: Option<&JobSnapshot>,
        error: Option<&str>,
    ) -> Result<AnalysisFlow, String> {
        for (job, kind) in [(trans, "transcription"), (diar, "diarization")] {
            if let Some(job) = job {
                if job.meeting_id != old.meeting_id
                    || job.kind != kind
                    || self.fixed_job_initial_operation(&job.id)?
                        != format!("flow:{}:{kind}", old.id)
                {
                    return Err("解析工程のジョブ由来が一致しません".into());
                }
            }
        }
        let text = plan
            .map(|p| serde_json::to_string(&p))
            .transpose()
            .map_err(|e| e.to_string())?;
        let hash = text
            .as_ref()
            .map(|t| format!("{:x}", Sha256::digest(t.as_bytes())));
        let message = error.map(|e| e.chars().take(2000).collect::<String>());
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let changed=tx.execute("UPDATE analysis_flows SET stage=?1,plan_json=?2,plan_hash=?3,transcription_job_id=?4,diarization_job_id=?5,error_message=?6,lock_version=lock_version+1,updated_at=?7 WHERE id=?8 AND meeting_id=?9 AND lock_version=?10",params![stage,text,hash,trans.map(|j|&j.id),diar.map(|j|&j.id),message,now_ms()?,old.id,old.meeting_id,old.lock_version]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("確認後に解析工程が変化しています。再読込してください".into());
        }
        let flow = read_flow(&tx, &old.meeting_id)?.ok_or("解析工程がありません")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(flow)
    }
    pub(crate) fn flow_retry(&self, job: &JobSnapshot) -> Result<Option<JobSnapshot>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let id:Option<String>=db.query_row("SELECT id FROM processing_jobs WHERE retry_of_job_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1",params![job.id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        id.map(|id| super::jobs::load_job(&db, &id)?.ok_or("再試行がありません".into()))
            .transpose()
    }
    pub(crate) fn flow_job_by_operation(
        &self,
        operation: &str,
    ) -> Result<Option<JobSnapshot>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let id: Option<String> = db
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id=?1",
                params![operation],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        id.map(|id| super::jobs::load_job(&db, &id)?.ok_or("解析ジョブがありません".into()))
            .transpose()
    }
}

// The final candidate's immediate source remains the unadopted ASR output.
// Permit explicit adoption against that output's original public parent only
// for a verified, completed flow. Do not relax unrelated candidate checks.
pub(super) fn adoption_parent_matches(
    tx: &Transaction<'_>,
    meeting: &str,
    revision: &str,
    current: Option<&str>,
) -> Result<bool, String> {
    let Some(flow) = read_flow(tx, meeting)? else {
        return Ok(false);
    };
    let row:Option<(String,String,String)>=tx.query_row("SELECT f.transcription_job_id,f.diarization_job_id,a.parent_revision_id FROM analysis_flows f JOIN processing_jobs d ON d.id=f.diarization_job_id JOIN analysis_revisions a ON a.id=d.output_analysis_id WHERE f.meeting_id=?1 AND f.stage='review' AND d.state='succeeded' AND d.output_analysis_id=?2",params![meeting,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((trans, diar, source)) = row else {
        return Ok(false);
    };
    let trans = super::jobs::load_job(tx, &trans)?.ok_or("文字起こしジョブがありません")?;
    if trans.kind != "transcription"
        || trans.state != "succeeded"
        || trans.meeting_id != meeting
        || trans.output_analysis_id.as_deref() != Some(&source)
    {
        return Err("引継ぎ元の由来が不正です".into());
    }
    super::jobs::read_fixed_input(tx, &trans.id)?;
    let (spec, _) = super::jobs::read_fixed_input(tx, &diar)?;
    if spec.parameters["request"]["operationId"] != format!("flow:{}:diarization", flow.id) {
        return Err("工程の話者分離操作が一致しません".into());
    }
    let fixed = super::diarization::read_input(tx, &spec)?;
    if fixed.parent_id != source {
        return Err("固定入力と引継ぎ元が一致しません".into());
    }
    let (parent,hash):(Option<String>,String)=tx.query_row("SELECT parent_revision_id,content_hash FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",params![source,meeting],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if super::revisions::analysis_content_hash(tx, &source)? != hash {
        return Err("引継ぎ元の内容が変更されています".into());
    }
    Ok(parent.as_deref() == current)
}

#[cfg(all(test, unix))]
#[path = "analysis_flow_tests.rs"]
mod tests;
