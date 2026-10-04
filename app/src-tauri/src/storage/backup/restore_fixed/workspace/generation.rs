//! Connect native semantic replay to the owned, still-unsealed restore image.
use super::assets::WorkspaceAssets;
use super::*;
use crate::generation::restore::{Assets, Context};
use crate::storage::{JobSnapshot, MinutesInput};
use std::collections::BTreeMap;

impl FixedData<'_, '_> {
    pub(super) fn source_snapshot_input(&self, spec: &JobSpec) -> Result<MinutesInput, String> {
        let id = spec
            .input_analysis_id
            .as_deref()
            .ok_or("生成の固定入力がありません")?;
        self.ids.get("analysis_revisions", &spec.meeting_id, id)?;
        let db = &self.ids.source().database;
        let (hash,source,source_hash,draft,working,title,created,started,config):(String,String,String,Option<String>,String,String,i64,Option<i64>,String)=db.query_row(
            "SELECT a.content_hash,s.source_analysis_id,s.source_content_hash,s.source_draft_json,s.working_content_hash,s.meeting_title,s.meeting_created_at,s.meeting_started_at_utc_ms,a.config_json FROM analysis_input_snapshots s JOIN analysis_revisions a ON a.id=s.analysis_id AND a.meeting_id=s.meeting_id WHERE a.id=?1 AND a.meeting_id=?2 AND a.publication_state='internal_snapshot'",
            params![id,spec.meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
        ).map_err(|e|e.to_string())?;
        let config: Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
        let draft: Option<Value> = draft
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| e.to_string())?;
        if hash != spec.input_hash
            || config["inputSnapshot"]
                != json!({"schemaVersion":1,"sourceAnalysisId":source,"sourceContentHash":source_hash,"sourceDraft":draft,"workingContentHash":working,"meetingTitle":title,"meetingCreatedAt":created,"meetingStartedAtUtcMs":started})
        {
            return Err("復元元の生成固定入力・由来が一致しません".into());
        }
        // The portable source retains original identity hashes but has redacted
        // engine paths. Recomputing its full revision hash would be invalid.
        // Verify the consumed semantic input hash instead; the target uses the
        // normal full snapshot reader after dependency reconstruction.
        let view =
            crate::storage::analysis::read_analysis_content(db, &spec.meeting_id, id, &hash)?;
        if crate::storage::minutes_input::working_hash(db, &view, &title, created, started, None)?
            != working
        {
            return Err("復元元の生成作業内容が変更されています".into());
        }
        Ok(MinutesInput {
            analysis: view,
            meeting_title: title,
            meeting_created_at: created,
            meeting_started_at_utc_ms: started,
            working_content_hash: working,
        })
    }

    pub(super) fn source_initial_operation(
        &self,
        meeting: &str,
        job: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<String, String> {
        let fixed = self.source_job_spec(meeting, job)?;
        check_cancelled(cancelled)?;
        let current = crate::storage::jobs::load_job(&self.ids.source().database, job)?
            .ok_or("復元元ジョブがありません")?;
        crate::storage::jobs::initial_operation(current, &fixed, |parent_id| {
            check_cancelled(cancelled)?;
            let parent = crate::storage::jobs::load_job(&self.ids.source().database, parent_id)?
                .ok_or("復元元の再試行祖先がありません")?;
            let spec = self.source_job_spec(meeting, parent_id)?;
            Ok((parent, spec))
        })
    }
}

impl FixedWorkspace {
    pub(super) fn generation_artifacts(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let source = &fixed.ids.source().database;
        let entries: BTreeMap<_, _> = fixed
            .ids
            .source()
            .manifest
            .entries
            .iter()
            .filter_map(|e| e.asset_id.as_deref().map(|id| (id, e)))
            .collect();
        if !std::fs::symlink_metadata(self.stage.path())
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("復元専用領域が変更されています".into());
        }
        let root = self
            .stage
            .path()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let mut query=source.prepare("SELECT id,meeting_id FROM processing_jobs WHERE kind='minutes_generation' ORDER BY rowid").map_err(|e|e.to_string())?;
        let mut jobs = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = jobs.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let job: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let old_spec = fixed.source_job_spec(&meeting, &job)?;
            let new_spec = fixed.job_spec(&meeting, &job)?;
            let old_input = fixed.source_snapshot_input(&old_spec)?;
            let new_input = {
                let tx = self.database.transaction().map_err(|e| e.to_string())?;
                crate::storage::minutes_input::read_input(&tx, &new_spec)?
            };
            let old_operation = fixed.source_initial_operation(&meeting, &job, cancelled)?;
            let new_operation = fixed.ids.get("operation", &meeting, &old_operation)?;
            let original = crate::storage::jobs::load_job(source, &job)?
                .ok_or("復元元の生成ジョブがありません")?;
            let new_job = fixed.ids.get("processing_jobs", &meeting, &job)?;
            let rebuilt = crate::storage::jobs::load_job(&self.database, &new_job)?
                .ok_or("復元先の生成ジョブがありません")?;
            let retry = original
                .retry_of_job_id
                .as_deref()
                .map(|id| fixed.ids.get("processing_jobs", &meeting, id))
                .transpose()?;
            if rebuilt.retry_of_job_id != retry
                || rebuilt.attempt != original.attempt
                || fixed_spec_hash(&new_spec, retry.as_deref(), rebuilt.attempt)?
                    != rebuilt.spec_hash
                || fixed.resolve("processing_jobs", &meeting, &job, &original.spec_hash)?
                    != rebuilt.spec_hash
            {
                return Err("構築済み生成ジョブの固定条件・由来が変更されています".into());
            }
            let mut assets = WorkspaceAssets {
                fixed,
                database: &self.database,
                root: &root,
                meeting: &meeting,
                input: old_spec.input_analysis_id.as_deref(),
                entries: &entries,
                cancelled,
            };
            let mut checkpoints=source.prepare("SELECT rowid,id,sequence,checkpoint_json,checkpoint_hash,completed_units FROM job_checkpoints WHERE job_id=?1 ORDER BY sequence").map_err(|e|e.to_string())?;
            let mut rows = checkpoints.query(params![job]).map_err(|e| e.to_string())?;
            let mut count = 0_i64;
            let mut last_units = 0_i64;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                let rowid: i64 = row.get(0).map_err(|e| e.to_string())?;
                let id: String = row.get(1).map_err(|e| e.to_string())?;
                let sequence: i64 = row.get(2).map_err(|e| e.to_string())?;
                let text: String = row.get(3).map_err(|e| e.to_string())?;
                let hash: String = row.get(4).map_err(|e| e.to_string())?;
                let units: i64 = row.get(5).map_err(|e| e.to_string())?;
                count = count
                    .checked_add(1)
                    .ok_or("生成チェックポイント数が範囲外です")?;
                if sequence != count
                    || units < last_units
                    || units > original.completed_units
                    || text.len() > 1024 * 1024
                    || crate::storage::jobs::checkpoint_hash(&original.spec_hash, units, &text)
                        != hash
                {
                    return Err("復元元の生成チェックポイントの進捗・由来が不正です".into());
                }
                let (mut old, mut new) = (original.clone(), rebuilt.clone());
                old.completed_units = units;
                new.completed_units = units;
                let value = replay(
                    &old,
                    &new,
                    &old_spec,
                    &new_spec,
                    &old_input,
                    &new_input,
                    &old_operation,
                    &new_operation,
                    serde_json::from_str(&text).map_err(|e| e.to_string())?,
                    &mut assets,
                    cancelled,
                )?;
                let reference = fixed.checkpoint(&meeting, &job, rowid, cancelled)?;
                if !reference.missing_assets.is_empty()
                    || value
                        != serde_json::from_str::<Value>(&reference.json)
                            .map_err(|e| e.to_string())?
                {
                    return Err("生成成果物の意味照合と参照再構成が一致しません".into());
                }
                let checkpoint = fixed.ids.get("job_checkpoints", &meeting, &id)?;
                if self.database.execute("UPDATE job_checkpoints SET checkpoint_json=?1,checkpoint_hash=?2 WHERE id=?3 AND job_id=?4 AND sequence=?5 AND completed_units=?6",params![reference.json,reference.content_hash,checkpoint,new_job,sequence,units]).map_err(|e|e.to_string())? != 1 {
                    return Err("復元先の生成チェックポイントが一致しません".into());
                }
                fixed.remember(
                    "job_checkpoints",
                    &meeting,
                    &id,
                    &hash,
                    &reference.content_hash,
                )?;
                last_units = units;
            }
            if last_units != original.completed_units {
                return Err("生成ジョブの最新進捗と保存位置が一致しません".into());
            }
            if count == 0 {
                replay(
                    &original,
                    &rebuilt,
                    &old_spec,
                    &new_spec,
                    &old_input,
                    &new_input,
                    &old_operation,
                    &new_operation,
                    json!({"outputs":[]}),
                    &mut assets,
                    cancelled,
                )?;
            }
            drop(assets);
            // Verify the actual stored parameters and current checkpoint, not
            // just the computed spec used by the semantic replay.
            let tx = self.database.transaction().map_err(|e| e.to_string())?;
            if crate::storage::jobs::read_fixed_input(&tx, &new_job)?.0 != new_spec {
                return Err("復元先の生成条件が構築結果と一致しません".into());
            }
        }
        check_cancelled(cancelled)
    }
}

#[allow(clippy::too_many_arguments)]
fn replay(
    old: &JobSnapshot,
    new: &JobSnapshot,
    old_spec: &JobSpec,
    new_spec: &JobSpec,
    old_input: &MinutesInput,
    new_input: &MinutesInput,
    old_operation: &str,
    new_operation: &str,
    value: Value,
    assets: &mut impl Assets,
    cancelled: &impl Fn() -> bool,
) -> Result<Value, String> {
    crate::generation::restore::checkpoint(
        Context {
            job: old,
            spec: old_spec,
            input: old_input,
            initial_operation: old_operation,
        },
        Context {
            job: new,
            spec: new_spec,
            input: new_input,
            initial_operation: new_operation,
        },
        value,
        assets,
        cancelled,
    )
}

#[cfg(all(test, unix))]
#[path = "generation_tests.rs"]
mod tests;
