//! Hashes depend across domains: an input snapshot precedes its job, and the
//! job precedes its output analysis. Separate per-table parent ordering is not
//! sufficient. Keep the frontier/edges on disk instead of recursing over JSON.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::storage::backup) struct HashStep {
    pub domain: String,
    pub meeting: String,
    pub old_id: String,
}

impl FixedData<'_, '_> {
    pub(super) fn prepare_order(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.registry.execute_batch(
            "CREATE TABLE hash_nodes(domain TEXT NOT NULL,meeting TEXT NOT NULL,old_id TEXT NOT NULL,degree INTEGER NOT NULL DEFAULT 0,done INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(domain,meeting,old_id));
             CREATE INDEX hash_frontier ON hash_nodes(done,degree,domain,meeting,old_id);
             CREATE TABLE hash_edges(child_domain TEXT NOT NULL,meeting TEXT NOT NULL,child_id TEXT NOT NULL,parent_domain TEXT NOT NULL,parent_id TEXT NOT NULL,PRIMARY KEY(child_domain,meeting,child_id,parent_domain,parent_id));
             CREATE INDEX hash_children ON hash_edges(parent_domain,meeting,parent_id);
             CREATE TABLE hash_order(position INTEGER PRIMARY KEY,domain TEXT NOT NULL,meeting TEXT NOT NULL,old_id TEXT NOT NULL,UNIQUE(domain,meeting,old_id));"
        ).map_err(|e|e.to_string())?;
        let source = &self.ids.source().database;
        for domain in ["analysis_revisions", "processing_jobs", "minutes_revisions"] {
            let mut query = source
                .prepare(&format!("SELECT meeting_id,id FROM {domain}"))
                .map_err(|e| e.to_string())?;
            let mut rows = query.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                self.registry
                    .execute(
                        "INSERT INTO hash_nodes(domain,meeting,old_id) VALUES(?1,?2,?3)",
                        params![
                            domain,
                            row.get::<_, String>(0).map_err(|e| e.to_string())?,
                            row.get::<_, String>(1).map_err(|e| e.to_string())?
                        ],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        for (child,parent,sql) in [
            ("analysis_revisions","analysis_revisions","SELECT meeting_id,id,parent_revision_id FROM analysis_revisions WHERE parent_revision_id IS NOT NULL"),
            ("analysis_revisions","analysis_revisions","SELECT meeting_id,analysis_id,source_analysis_id FROM analysis_input_snapshots"),
            ("minutes_revisions","minutes_revisions","SELECT meeting_id,id,parent_revision_id FROM minutes_revisions WHERE parent_revision_id IS NOT NULL"),
            ("minutes_revisions","analysis_revisions","SELECT meeting_id,id,input_analysis_id FROM minutes_revisions"),
            ("processing_jobs","processing_jobs","SELECT meeting_id,id,retry_of_job_id FROM processing_jobs WHERE retry_of_job_id IS NOT NULL"),
            ("processing_jobs","analysis_revisions","SELECT meeting_id,id,input_analysis_id FROM processing_jobs WHERE input_analysis_id IS NOT NULL"),
        ] {
            let mut query=source.prepare(sql).map_err(|e|e.to_string())?;
            let mut rows=query.query([]).map_err(|e|e.to_string())?;
            while let Some(row)=rows.next().map_err(|e|e.to_string())? {
                check_cancelled(cancelled)?;
                self.edge(child,&row.get::<_,String>(0).map_err(|e|e.to_string())?,&row.get::<_,String>(1).map_err(|e|e.to_string())?,parent,&row.get::<_,String>(2).map_err(|e|e.to_string())?)?;
            }
        }
        let mut query = source
            .prepare("SELECT meeting_id,id,config_json FROM analysis_revisions")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let meeting: String = row.get(0).map_err(|e| e.to_string())?;
            let id: String = row.get(1).map_err(|e| e.to_string())?;
            let text: String = row.get(2).map_err(|e| e.to_string())?;
            let config: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            for (key, parent) in [
                ("jobId", "processing_jobs"),
                ("retryOfJobId", "processing_jobs"),
                ("inputAnalysisId", "analysis_revisions"),
            ] {
                if let Some(value) = config.get(key).filter(|v| !v.is_null()) {
                    self.edge(
                        "analysis_revisions",
                        &meeting,
                        &id,
                        parent,
                        value.as_str().ok_or("固定条件の由来IDが不正です")?,
                    )?;
                }
            }
            if let Some(snapshot) = config.get("inputSnapshot") {
                let snapshot: InputSnapshot =
                    serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?;
                self.edge(
                    "analysis_revisions",
                    &meeting,
                    &id,
                    "analysis_revisions",
                    &snapshot.source_analysis_id,
                )?;
            }
            if let Some(operations) = config.get("manualUtteranceSplits") {
                let operations: Vec<SplitOperation> =
                    serde_json::from_value(operations.clone()).map_err(|e| e.to_string())?;
                for operation in operations {
                    self.edge(
                        "analysis_revisions",
                        &meeting,
                        &id,
                        "analysis_revisions",
                        &operation.source_analysis_id,
                    )?;
                }
            }
        }
        let mut query = source
            .prepare("SELECT meeting_id,id,parameters_json FROM processing_jobs")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let meeting: String = row.get(0).map_err(|e| e.to_string())?;
            let id: String = row.get(1).map_err(|e| e.to_string())?;
            let text: String = row.get(2).map_err(|e| e.to_string())?;
            let parameters: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if let Some(request) = parameters.get("request") {
                let request: MinutesJobRequest =
                    serde_json::from_value(request.clone()).map_err(|e| e.to_string())?;
                if request.meeting_id != meeting {
                    return Err("固定要求の会議が一致しません".into());
                }
                self.edge(
                    "processing_jobs",
                    &meeting,
                    &id,
                    "analysis_revisions",
                    &request.expected_analysis_id,
                )?;
            }
        }
        let count: i64 = self
            .registry
            .query_row("SELECT COUNT(*) FROM hash_nodes", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut position = 0_i64;
        loop {
            check_cancelled(cancelled)?;
            let next:Option<(String,String,String)>=self.registry.query_row("SELECT domain,meeting,old_id FROM hash_nodes WHERE done=0 AND degree=0 ORDER BY domain,meeting,old_id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
            let Some((domain, meeting, id)) = next else {
                break;
            };
            self.registry
                .execute(
                    "INSERT INTO hash_order VALUES(?1,?2,?3,?4)",
                    params![position, domain, meeting, id],
                )
                .map_err(|e| e.to_string())?;
            self.registry
                .execute(
                    "UPDATE hash_nodes SET done=1 WHERE domain=?1 AND meeting=?2 AND old_id=?3",
                    params![domain, meeting, id],
                )
                .map_err(|e| e.to_string())?;
            self.registry.execute("UPDATE hash_nodes SET degree=degree-1 WHERE (domain,meeting,old_id) IN (SELECT child_domain,meeting,child_id FROM hash_edges WHERE parent_domain=?1 AND meeting=?2 AND parent_id=?3)",params![domain,meeting,id]).map_err(|e|e.to_string())?;
            position = position
                .checked_add(1)
                .ok_or("固定ハッシュ再構成順が範囲外です")?;
        }
        if position != count {
            return Err("解析版とジョブをまたぐ固定ハッシュ参照が循環しています".into());
        }
        Ok(())
    }

    fn edge(
        &self,
        child_domain: &str,
        meeting: &str,
        child: &str,
        parent_domain: &str,
        parent: &str,
    ) -> Result<(), String> {
        // IDs are scoped, even if a forged source config names a real ID from
        // another meeting. A declared historical ID is not necessarily a row.
        for (domain, id) in [(child_domain, child), (parent_domain, parent)] {
            self.ids.get(domain, meeting, id)?;
            let exists:bool=self.registry.query_row("SELECT EXISTS(SELECT 1 FROM hash_nodes WHERE domain=?1 AND meeting=?2 AND old_id=?3)",params![domain,meeting,id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !exists {
                return Err("固定ハッシュの依存先が欠損しています".into());
            }
        }
        let changed = self
            .registry
            .execute(
                "INSERT INTO hash_edges VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
                params![child_domain, meeting, child, parent_domain, parent],
            )
            .map_err(|e| e.to_string())?;
        if changed == 1 {
            self.registry.execute("UPDATE hash_nodes SET degree=degree+1 WHERE domain=?1 AND meeting=?2 AND old_id=?3",params![child_domain,meeting,child]).map_err(|e|e.to_string())?;
        }
        Ok(())
    }

    pub(in crate::storage::backup) fn ordered_step(
        &self,
        position: i64,
    ) -> Result<Option<HashStep>, String> {
        self.registry
            .query_row(
                "SELECT domain,meeting,old_id FROM hash_order WHERE position=?1",
                params![position],
                |r| {
                    Ok(HashStep {
                        domain: r.get(0)?,
                        meeting: r.get(1)?,
                        old_id: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }
}
