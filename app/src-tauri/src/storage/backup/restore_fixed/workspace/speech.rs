//! Construct saved speech results, retaining omitted PCM/logs as missing rows.
use super::assets::WorkspaceAssets;
use super::*;
use std::collections::BTreeMap;

#[cfg(all(test, unix))]
#[path = "speech_tests.rs"]
mod tests;

impl FixedWorkspace {
    pub(super) fn speech_artifacts(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let source = &fixed.ids.source().database;
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
        let entries: BTreeMap<_, _> = fixed
            .ids
            .source()
            .manifest
            .entries
            .iter()
            .filter_map(|e| e.asset_id.as_deref().map(|id| (id, e)))
            .collect();
        for omitted in &fixed.ids.source().manifest.omitted_assets {
            check_cancelled(cancelled)?;
            let id = fixed
                .ids
                .get("file_assets", &omitted.meeting_id, &omitted.asset_id)?;
            let saved: (String, i64, String) = self
                .database
                .query_row(
                    "SELECT state,size_bytes,content_hash FROM file_assets WHERE id=?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|e| e.to_string())?;
            if saved
                != (
                    "missing".into(),
                    i64::try_from(omitted.source_size).map_err(|e| e.to_string())?,
                    omitted.source_sha256.clone(),
                )
            {
                return Err("除外された途中成果物の登録由来が一致しません".into());
            }
            self.missing_assets.insert(id);
        }
        let mut query=source.prepare("SELECT id,meeting_id,kind FROM processing_jobs WHERE kind IN ('transcription','diarization') ORDER BY rowid").map_err(|e|e.to_string())?;
        let mut jobs = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = jobs.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let job: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let kind: String = row.get(2).map_err(|e| e.to_string())?;
            let old_spec = fixed.source_job_spec(&meeting, &job)?;
            let new_spec = fixed.job_spec(&meeting, &job)?;
            let old_initial = fixed.source_initial_operation(&meeting, &job, cancelled)?;
            let new_initial = fixed.ids.get("operation", &meeting, &old_initial)?;
            let original = crate::storage::jobs::load_job(source, &job)?
                .ok_or("復元元の解析ジョブがありません")?;
            let new_job = fixed.ids.get("processing_jobs", &meeting, &job)?;
            let rebuilt = crate::storage::jobs::load_job(&self.database, &new_job)?
                .ok_or("復元先の解析ジョブがありません")?;
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
                return Err("構築済み解析ジョブの固定条件・由来が変更されています".into());
            }
            if kind == "diarization" {
                fixed.source_snapshot_input(&old_spec)?;
                let tx = self.database.transaction().map_err(|e| e.to_string())?;
                crate::storage::diarization::read_input(&tx, &new_spec)?;
            }
            let mut model_names = vec![];
            if kind == "transcription" {
                let proof: crate::local_engine::Fingerprint =
                    serde_json::from_value(old_spec.parameters["engine"]["model"].clone())
                        .map_err(|e| e.to_string())?;
                model_names = fixed
                    .ids
                    .source()
                    .manifest
                    .file_references
                    .iter()
                    .filter(|r| {
                        r.role == "model" && r.sha256 == proof.sha256 && r.size == proof.size
                    })
                    .map(|r| r.name.clone())
                    .collect();
                if model_names.is_empty() {
                    return Err("文字起こしモデルの名前・SHA記録がありません".into());
                }
                if let Some(vocabulary) = old_spec
                    .parameters
                    .get("vocabulary")
                    .filter(|v| !v.is_null())
                {
                    let owner: String = source
                        .query_row(
                            "SELECT job_id FROM vocabulary_snapshots WHERE id=?1 AND meeting_id=?2",
                            params![
                                vocabulary["id"].as_str().ok_or("固定用語IDがありません")?,
                                meeting
                            ],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    let root_job=source.query_row("SELECT id FROM processing_jobs WHERE operation_id=?1 AND meeting_id=?2",params![old_initial,meeting],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
                    if owner != root_job {
                        return Err("固定用語の元ジョブが再試行の祖先と一致しません".into());
                    }
                }
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
            let mut process =
                |snapshot: &crate::storage::JobSnapshot, value: Value| -> Result<(), String> {
                    match kind.as_str() {
                        "transcription" => crate::transcription::restore_saved_results(
                            snapshot,
                            &old_spec,
                            &new_spec,
                            value,
                            &model_names,
                            &mut assets,
                            cancelled,
                        ),
                        "diarization" => crate::diarization::restore_saved_results(
                            snapshot,
                            &old_spec,
                            &new_spec,
                            value,
                            &old_initial,
                            &new_initial,
                            &mut assets,
                            cancelled,
                        ),
                        _ => Err("未対応の音声解析成果物です".into()),
                    }
                };
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
                    .ok_or("解析チェックポイント数が範囲外です")?;
                if sequence != count
                    || units < last_units
                    || units > original.completed_units
                    || text.len() > 1024 * 1024
                    || crate::storage::jobs::checkpoint_hash(&original.spec_hash, units, &text)
                        != hash
                {
                    return Err("復元元解析のチェックポイント進捗・由来が不正です".into());
                }
                let mut snapshot = original.clone();
                snapshot.completed_units = units;
                process(
                    &snapshot,
                    serde_json::from_str(&text).map_err(|e| e.to_string())?,
                )?;
                let reference = fixed.checkpoint(&meeting, &job, rowid, cancelled)?;
                self.missing_assets.extend(reference.missing_assets);
                let checkpoint = fixed.ids.get("job_checkpoints", &meeting, &id)?;
                if self.database.execute("UPDATE job_checkpoints SET checkpoint_json=?1,checkpoint_hash=?2 WHERE id=?3 AND job_id=?4 AND sequence=?5 AND completed_units=?6",params![reference.json,reference.content_hash,checkpoint,new_job,sequence,units]).map_err(|e|e.to_string())?!=1 {
                    return Err("復元先解析のチェックポイントが一致しません".into());
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
                return Err("解析ジョブの最新進捗と保存位置が一致しません".into());
            }
            if count == 0 {
                process(&original, json!({"outputs":[]}))?;
            }
            drop(process);
            drop(assets);
            if crate::storage::jobs::read_fixed_input(&self.database, &new_job)?.0 != new_spec {
                return Err("復元先の解析条件が構築結果と一致しません".into());
            }
        }
        // All ancestor checkpoints must be rebuilt before the normal vocabulary
        // reader traverses their fixed-spec hashes, irrespective of source rowid.
        let mut query=self.database.prepare("SELECT id,meeting_id,parameters_json FROM processing_jobs WHERE kind='transcription'").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let job: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let parameters: Value =
                serde_json::from_str(&row.get::<_, String>(2).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(vocabulary) = parameters.get("vocabulary").filter(|v| !v.is_null()) {
                crate::storage::vocabulary::verify_snapshot(
                    &self.database,
                    &meeting,
                    &job,
                    &serde_json::from_value(vocabulary.clone()).map_err(|e| e.to_string())?,
                )?;
            }
        }
        check_cancelled(cancelled)
    }
}
