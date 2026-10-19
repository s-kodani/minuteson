use super::*;
use crate::saved_artifact::Role;
use std::collections::BTreeSet;

pub(in crate::storage::backup) struct RebasedCheckpoint {
    pub json: String,
    pub content_hash: String,
    // Logical references survive, but neither exclusion is a ready file. A
    // publication/retry coordinator must regenerate PCM and handle log history.
    pub missing_assets: BTreeSet<String>,
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;

impl FixedData<'_, '_> {
    pub(in crate::storage::backup) fn diarization_result(
        &self,
        meeting: &str,
        job: &str,
        bytes: &[u8],
    ) -> Result<Vec<u8>, String> {
        crate::diarization::restore_result(
            bytes,
            &self.source_job_spec(meeting, job)?,
            &self.job_spec(meeting, job)?,
        )
    }

    pub(in crate::storage::backup) fn checkpoint(
        &self,
        meeting: &str,
        old_job: &str,
        rowid: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<RebasedCheckpoint, String> {
        check_cancelled(cancelled)?;
        let source = &self.ids.source().database;
        self.ids.get("processing_jobs", meeting, old_job)?;
        let job = super::super::super::jobs::load_job(source, old_job)?
            .ok_or("復元元のジョブがありません")?;
        if job.meeting_id != meeting {
            return Err("復元ジョブの会議が一致しません".into());
        }
        let (text,old_hash,units):(String,String,i64)=source.query_row("SELECT c.checkpoint_json,c.checkpoint_hash,c.completed_units FROM job_checkpoints c JOIN processing_jobs j ON j.id=c.job_id WHERE c.rowid=?1 AND c.job_id=?2 AND j.meeting_id=?3",params![rowid,old_job,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if text.len() > 1024 * 1024
            || super::super::super::jobs::checkpoint_hash(&job.spec_hash, units, &text) != old_hash
        {
            return Err("復元元のチェックポイントが変更されています".into());
        }
        let mut checkpoint_job = job.clone();
        // Every historical checkpoint has its own progress, not necessarily the
        // latest job progress. Keep sequence/progress; never drop saved output.
        checkpoint_job.completed_units = units;
        let mut missing = BTreeSet::new();
        let mut map = |role: Role, id: &str, hash: &str| -> Result<(String, String), String> {
            check_cancelled(cancelled)?;
            let (new, hash, omitted) = self.checkpoint_asset(meeting, id, hash, role)?;
            if omitted {
                missing.insert(new.clone());
            }
            Ok((new, hash))
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let rebased = match job.kind.as_str() {
            "speaker_completion" => {
                let mut v = value;
                if v["completionResult"].as_bool().is_none() {
                    return Err("話者補完の再開情報が不正です".into());
                }
                if let Some(job) = v["jobId"].as_str() {
                    v["jobId"] = json!(self.ids.get("processing_jobs", meeting, job)?);
                }
                if let Some(id) = v["outputAnalysisId"].as_str() {
                    v["outputAnalysisId"] =
                        json!(self.ids.get("analysis_revisions", meeting, id)?);
                }
                if let Some(rows) = v["decisions"].as_array_mut() {
                    for d in rows {
                        let lineage = d["lineage"].as_str().ok_or("話者補完の発言がありません")?;
                        d["lineage"] =
                            json!(self.ids.get("utterance_lineage", meeting, lineage)?);
                        for key in ["speakerId", "bodyBest"] {
                            if let Some(id) = d[key].as_str() {
                                d[key] = json!(self.ids.get("speakers", meeting, id)?);
                            }
                        }
                    }
                }
                v
            }
            "voice_registration" | "voice_matching" => {
                if value.get("voiceResult").and_then(Value::as_bool).is_none() {
                    return Err("声の処理の再開情報が不正です".into());
                }
                value
            }
            "transcription" => crate::transcription::restore_checkpoint_references(
                &checkpoint_job,
                value,
                &mut map,
            )?,
            "diarization" => {
                crate::diarization::restore_checkpoint_references(&checkpoint_job, value, &mut map)?
            }
            "minutes_generation" => {
                let text: String = source
                    .query_row(
                        "SELECT parameters_json FROM processing_jobs WHERE id=?1",
                        params![old_job],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let parameters: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                crate::generation::restore_checkpoint_references(
                    &checkpoint_job,
                    parameters["generation"].clone(),
                    value,
                    &mut map,
                )?
            }
            _ => return Err("未対応の復元チェックポイントです".into()),
        };
        let json = serde_json::to_string(&rebased).map_err(|e| e.to_string())?;
        if json.len() > 1024 * 1024 {
            return Err("復元チェックポイントが容量上限を超えています".into());
        }
        let spec = self.resolve("processing_jobs", meeting, old_job, &job.spec_hash)?;
        let content_hash = super::super::super::jobs::checkpoint_hash(&spec, units, &json);
        Ok(RebasedCheckpoint {
            json,
            content_hash,
            missing_assets: missing,
        })
    }

    fn checkpoint_asset(
        &self,
        meeting: &str,
        id: &str,
        hash: &str,
        role: Role,
    ) -> Result<(String, String, bool), String> {
        let new = self.ids.get("file_assets", meeting, id)?;
        let source = self.ids.source();
        let (media,size,old,state):(String,i64,String,String)=source.database.query_row("SELECT media_type,size_bytes,content_hash,state FROM file_assets WHERE id=?1 AND meeting_id=?2",params![id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
        if media != role.media() || old != hash {
            return Err("成果物の種別・会議・元ハッシュが一致しません".into());
        }
        if let Some(omitted) = source
            .manifest
            .omitted_assets
            .iter()
            .find(|a| a.asset_id == id)
        {
            let reason = match role {
                Role::TemporaryPcm if source.deleted_audio.asset(meeting, id).is_some() => {
                    let missing = source.deleted_audio.asset(meeting, id).unwrap();
                    if missing.size != size || missing.hash != hash || missing.media != media {
                        return Err("PCMと検査済み音声削除履歴が一致しません".into());
                    }
                    archive::OmittedReason::SourceAudioDeleted
                }
                Role::TemporaryPcm => archive::OmittedReason::RecreatablePcm,
                Role::DiarizationLog => archive::OmittedReason::NativeLog,
                _ => return Err("永続的な途中成果物が欠損しています".into()),
            };
            if omitted.reason != reason
                || omitted.meeting_id != meeting
                || omitted.source_sha256 != hash
                || omitted.source_size != size as u64
                || state != "missing"
            {
                return Err("除外された途中成果物の由来が不正です".into());
            }
            return Ok((new, hash.into(), true));
        }
        if state != "ready" || role == Role::DiarizationLog {
            return Err("復元にログ本体を持ち込むことはできません".into());
        }
        let (_, hash) = self.asset_hash(meeting, id, hash)?;
        Ok((new, hash, false))
    }
}
