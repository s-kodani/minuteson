//! Reconstruct derived flow identities and retain external export evidence.
//! This pass also prevents restored work from being automatically scheduled.
use super::*;
use crate::analysis_flow::Plan;
use crate::local_engine::Fingerprint;
use crate::storage::{JobKind, JobSnapshot, JobSpec};

#[derive(Debug, PartialEq, Eq)]
struct FlowRow {
    id: String,
    meeting: String,
    stage: String,
    plan: Option<String>,
    hash: Option<String>,
    trans: Option<String>,
    diar: Option<String>,
    error: Option<String>,
    lock: i64,
    created: i64,
    updated: i64,
}
fn flow_row(db: &Connection, id: &str) -> Result<FlowRow, String> {
    db.query_row("SELECT id,meeting_id,stage,plan_json,plan_hash,transcription_job_id,diarization_job_id,error_message,lock_version,created_at,updated_at FROM analysis_flows WHERE id=?1", params![id], |r| Ok(FlowRow { id:r.get(0)?,meeting:r.get(1)?,stage:r.get(2)?,plan:r.get(3)?,hash:r.get(4)?,trans:r.get(5)?,diar:r.get(6)?,error:r.get(7)?,lock:r.get(8)?,created:r.get(9)?,updated:r.get(10)? })).map_err(|e|e.to_string())
}

impl FixedWorkspace {
    pub(super) fn processing_history(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let source = &fixed.ids.source().database;
        let mut query = source
            .prepare("SELECT id FROM analysis_flows ORDER BY id")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let old = flow_row(source, &row.get::<_, String>(0).map_err(|e| e.to_string())?)?;
            let meeting = fixed.ids.get("meetings", "", &old.meeting)?;
            let new = fixed.ids.get("analysis_flows", &old.meeting, &old.id)?;
            let mapped = |job: &Option<String>| {
                job.as_deref()
                    .map(|id| fixed.ids.get("processing_jobs", &old.meeting, id))
                    .transpose()
            };
            let expected = FlowRow {
                id: new.clone(),
                meeting: meeting.clone(),
                stage: old.stage.clone(),
                plan: old.plan.clone(),
                hash: old.hash.clone(),
                trans: mapped(&old.trans)?,
                diar: mapped(&old.diar)?,
                error: None,
                lock: old.lock,
                created: old.created,
                updated: old.updated,
            };
            if flow_row(&self.database, &new)? != expected || old.error.is_some() {
                return Err("構築済み解析工程の参照・固定条件・履歴が変更されています".into());
            }
            let plan = match (&old.plan, &old.hash) {
                (None, None) => None,
                (Some(text), Some(hash)) if text.len() <= 65_536 && portable::valid_hash(hash) => {
                    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                    let profile = value.get("profile").and_then(Value::as_str).ok_or("固定された実行条件がありません")?.to_owned();
                    let plan = crate::analysis_flow::saved_plan(value, &profile)?;
                    self.flow_files(fixed, &plan)?;
                    Some(plan)
                }
                _ => return Err("解析工程の固定計画・元SHAが不正です".into()),
            };
            for (kind, linked) in [("transcription", &old.trans), ("diarization", &old.diar)] {
                check_cancelled(cancelled)?;
                // A root job may exist before a failed handoff persisted its ID.
                let operation = format!("flow:{}:{kind}", old.id);
                let root: Option<String> = source
                    .query_row(
                        "SELECT id FROM processing_jobs WHERE operation_id=?1",
                        params![operation],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(root) = root {
                    self.flow_job(
                        fixed,
                        &old,
                        &new,
                        kind,
                        &root,
                        plan.as_ref().ok_or("工程ジョブに固定計画がありません")?,
                        cancelled,
                    )?;
                }
                if let Some(job) = linked {
                    self.flow_job(
                        fixed,
                        &old,
                        &new,
                        kind,
                        job,
                        plan.as_ref().ok_or("工程参照に固定計画がありません")?,
                        cancelled,
                    )?;
                }
            }
            self.flow_chain(fixed, &old)?;
            let stage = if old.stage == "review" {
                "review"
            } else if plan.is_some() {
                "attention"
            } else {
                "configuration"
            };
            let message = (stage != "review").then_some("バックアップから復元した解析工程です。音声とAIモデルの準備を確認してから明示的に再試行してください");
            let mut rebuilt_plan = old.plan.clone();
            if let Some(mut plan) = plan.clone() {
                if let Some(target) = &mut plan.replacement {
                    target.content_hash = fixed.resolve("analysis_revisions", &old.meeting, &target.analysis_id, &target.content_hash)?;
                    target.analysis_id = fixed.ids.get("analysis_revisions", &old.meeting, &target.analysis_id)?;
                    if let Some(draft) = &mut target.draft {
                        let version: i64 = source.query_row("SELECT lock_version FROM working_drafts WHERE id=?1 AND meeting_id=?2 AND target_kind='transcript'", params![draft.id,old.meeting], |r|r.get(0)).map_err(|e|e.to_string())?;
                        // A later edit can legitimately make this admission
                        // stamp stale. Preserve its old version (and rejection)
                        // rather than blessing the newer human draft.
                        if version == draft.lock_version {
                            draft.content_hash = fixed.resolve("working_drafts", &old.meeting, &draft.id, &draft.content_hash)?;
                        }
                        draft.id = fixed.ids.get("working_drafts", &old.meeting, &draft.id)?;
                    }
                    rebuilt_plan = Some(serde_json::to_string(&plan).map_err(|e|e.to_string())?);
                }
            }
            let new_hash = rebuilt_plan.as_ref().map(|text| format!("{:x}", Sha256::digest(text.as_bytes())));
            if self.database.execute("UPDATE analysis_flows SET stage=?1,plan_hash=?2,error_message=?3,plan_json=?6 WHERE id=?4 AND meeting_id=?5",params![stage,new_hash,message,new,meeting,rebuilt_plan]).map_err(|e|e.to_string())? != 1 {
                return Err("解析工程の構築先がありません".into());
            }
            if let (Some(hash), Some(rebuilt)) = (&old.hash, &new_hash) {
                fixed.remember("analysis_flows", &old.meeting, &old.id, hash, rebuilt)?;
            }
            let flow = crate::storage::analysis_flows::read_flow(&self.database, &meeting)?
                .ok_or("復元した解析工程がありません")?;
            if flow.stage != stage
                || flow.plan.is_some() != plan.is_some()
                || flow.lock_version != old.lock
            {
                return Err("復元した解析工程の通常読取りが一致しません".into());
            }
        }
        self.export_history(fixed, cancelled)?;
        self.interrupt_jobs(fixed, cancelled)?;
        let runnable: bool = self.database.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('queued','running','pausing','paused')) OR EXISTS(SELECT 1 FROM analysis_flows WHERE stage IN ('pending','transcription','diarization'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if runnable {
            return Err("復元データに自動開始可能な処理が残っています".into());
        }
        check_cancelled(cancelled)
    }

    fn flow_files(&self, fixed: &FixedData<'_, '_>, plan: &Plan) -> Result<(), String> {
        let mut proofs: Vec<(&Fingerprint, &str)> = vec![
            (&plan.whisper.executable, "executable"),
            (&plan.whisper.model, "model"),
            (&plan.sherpa.executable, "executable"),
            (&plan.sherpa.segmentation_model, "artifact"),
            (&plan.sherpa.embedding_model, "artifact"),
        ];
        if let Some(proof) = &plan.whisper.vad_model {
            proofs.push((proof, "vadModel"));
        }
        proofs.extend(
            plan.sherpa
                .runtime_libraries
                .iter()
                .map(|p| (p, "artifact")),
        );
        for (proof, role) in proofs {
            if !portable::valid_hash(&proof.sha256)
                || proof.size == 0
                || proof.path.to_str() != Some(format!("mon-unavailable:{}", proof.sha256).as_str())
                || !fixed
                    .ids
                    .source()
                    .manifest
                    .file_references
                    .iter()
                    .any(|r| r.role == role && r.sha256 == proof.sha256 && r.size == proof.size)
            {
                return Err(
                    "解析工程のモデル・実行ファイルの未配置参照とSHA記録が一致しません".into(),
                );
            }
        }
        Ok(())
    }

    fn flow_job(
        &self,
        fixed: &FixedData<'_, '_>,
        flow: &FlowRow,
        new_flow: &str,
        kind: &str,
        id: &str,
        plan: &Plan,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let source = fixed.source_job_spec(&flow.meeting, id)?;
        if source.kind
            != serde_json::from_value::<JobKind>(json!(kind)).map_err(|e| e.to_string())?
            || source.profile_snapshot["profile"] != plan.profile
            || fixed.source_initial_operation(&flow.meeting, id, cancelled)?
                != format!("flow:{}:{kind}", flow.id)
        {
            return Err("解析工程のジョブ由来・プロファイルが一致しません".into());
        }
        let mapped = fixed.ids.get("processing_jobs", &flow.meeting, id)?;
        let (rebuilt, _) = crate::storage::jobs::read_fixed_input(&self.database, &mapped)?;
        if rebuilt != fixed.job_spec(&flow.meeting, id)? {
            return Err("解析工程ジョブの固定入力が変更されています".into());
        }
        let job = crate::storage::jobs::load_job(&self.database, &mapped)?
            .ok_or("解析工程ジョブがありません")?;
        let operation = crate::storage::jobs::initial_operation(job, &rebuilt, |parent| {
            check_cancelled(cancelled)?;
            let job = crate::storage::jobs::load_job(&self.database, parent)?
                .ok_or("解析工程の再試行祖先がありません")?;
            Ok((
                job,
                crate::storage::jobs::read_fixed_input(&self.database, parent)?.0,
            ))
        })?;
        if operation != format!("flow:{new_flow}:{kind}") {
            return Err("復元した解析工程の初回操作が一致しません".into());
        }
        match kind {
            "transcription" => {
                if crate::storage::model_profiles::jobs::read(&source.profile_snapshot)?
                    != plan.whisper_managed
                {
                    return Err("文字起こしの管理モデルが固定計画と一致しません".into());
                }
                let mut engine: crate::transcription::whisper::WhisperConfig =
                    serde_json::from_value(source.parameters["engine"].clone())
                        .map_err(|e| e.to_string())?;
                // Enqueue adds the validated fixed vocabulary to the empty base
                // hint; it is the only allowed difference from the flow plan.
                engine.prompt.clear();
                if serde_json::to_value(engine).map_err(|e| e.to_string())?
                    != serde_json::to_value(&plan.whisper).map_err(|e| e.to_string())?
                {
                    return Err("文字起こし条件が解析工程の固定計画と一致しません".into());
                }
            }
            "diarization" => {
                if crate::storage::model_profiles::jobs::read(&source.profile_snapshot)?
                    != plan.sherpa_managed
                {
                    return Err("話者分離の管理モデルが固定計画と一致しません".into());
                }
                let engine: crate::diarization::sherpa::SherpaConfig =
                    serde_json::from_value(source.parameters["diarization"]["engine"].clone())
                        .map_err(|e| e.to_string())?;
                let settings: Vec<crate::diarization::TrackSettings> =
                    serde_json::from_value(source.parameters["diarization"]["tracks"].clone())
                        .map_err(|e| e.to_string())?;
                if engine != plan.sherpa || settings.iter().any(|t| t.num_speakers.is_some()) {
                    return Err("話者分離条件が解析工程の固定計画と一致しません".into());
                }
                let trans = flow
                    .trans
                    .as_deref()
                    .ok_or("話者分離工程の引継ぎ元がありません")?;
                let trans = crate::storage::jobs::load_job(&fixed.ids.source().database, trans)?
                    .ok_or("引継ぎ元ジョブがありません")?;
                let output = trans
                    .output_analysis_id
                    .as_deref()
                    .ok_or("文字起こし出力がありません")?;
                let input = source
                    .input_analysis_id
                    .as_deref()
                    .ok_or("工程の固定解析入力がありません")?;
                let parent: String = fixed.ids.source().database.query_row("SELECT source_analysis_id FROM analysis_input_snapshots WHERE analysis_id=?1 AND meeting_id=?2",params![input,flow.meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
                if trans.state != "succeeded" || parent != output {
                    return Err(
                        "話者分離工程の固定入力が成功した文字起こし出力と一致しません".into(),
                    );
                }
            }
            _ => return Err("未対応の解析工程です".into()),
        }
        Ok(())
    }

    fn flow_chain(&mut self, fixed: &FixedData<'_, '_>, flow: &FlowRow) -> Result<(), String> {
        let source = &fixed.ids.source().database;
        let trans = flow
            .trans
            .as_deref()
            .map(|id| {
                crate::storage::jobs::load_job(source, id)?
                    .ok_or_else(|| "文字起こし工程がありません".to_owned())
            })
            .transpose()?;
        if matches!(flow.stage.as_str(), "diarization" | "review")
            && trans
                .as_ref()
                .is_none_or(|j| j.state != "succeeded" || j.output_analysis_id.is_none())
        {
            return Err("解析工程に成功した文字起こし出力がありません".into());
        }
        if let Some(id) = &flow.diar {
            let trans = trans.as_ref().ok_or("話者分離工程の引継ぎ元がありません")?;
            let output = trans
                .output_analysis_id
                .as_deref()
                .ok_or("文字起こし出力がありません")?;
            if trans.state != "succeeded" {
                return Err("話者分離工程の引継ぎ元が成功していません".into());
            }
            let spec = fixed.source_job_spec(&flow.meeting, id)?;
            let input = spec
                .input_analysis_id
                .as_deref()
                .ok_or("工程の固定解析入力がありません")?;
            let parent: String = source.query_row("SELECT source_analysis_id FROM analysis_input_snapshots WHERE analysis_id=?1 AND meeting_id=?2",params![input,flow.meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
            if parent != output {
                return Err("話者分離工程の固定入力が文字起こし出力と一致しません".into());
            }
            let mapped = fixed.ids.get("processing_jobs", &flow.meeting, id)?;
            let (new_spec, _) = crate::storage::jobs::read_fixed_input(&self.database, &mapped)?;
            let tx = self.database.transaction().map_err(|e| e.to_string())?;
            let input = crate::storage::diarization::read_input(&tx, &new_spec)?;
            if input.parent_id != fixed.ids.get("analysis_revisions", &flow.meeting, output)? {
                return Err("復元した工程の固定入力が引継ぎ元と一致しません".into());
            }
            tx.commit().map_err(|e| e.to_string())?;
            if flow.stage == "review" {
                let diar = crate::storage::jobs::load_job(source, id)?
                    .ok_or("確認工程の話者分離ジョブがありません")?;
                let candidate = diar
                    .output_analysis_id
                    .as_deref()
                    .ok_or("確認工程の出力がありません")?;
                let parent: Option<String> = source.query_row("SELECT parent_revision_id FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",params![candidate,flow.meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
                if diar.state != "succeeded" || parent.as_deref() != Some(output) {
                    return Err("確認工程の成功出力と引継ぎ元が一致しません".into());
                }
            }
        }
        Ok(())
    }

    fn export_history(
        &self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let source = &fixed.ids.source().database;
        let tables = portable::tables(source)?;
        let table = tables
            .iter()
            .find(|t| t.name == "export_records")
            .ok_or("書き出し履歴の定義がありません")?;
        let mut query = source.prepare("SELECT rowid,id,meeting_id,minutes_revision_id,request_hash,exported_content_hash,output_content_hash,destination_path,source_draft_id,state FROM export_records ORDER BY rowid").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let rowid: i64 = row.get(0).map_err(|e| e.to_string())?;
            let id: String = row.get(1).map_err(|e| e.to_string())?;
            let meeting: String = row.get(2).map_err(|e| e.to_string())?;
            for index in 4..=6 {
                if !portable::valid_hash(&row.get::<_, String>(index).map_err(|e| e.to_string())?) {
                    return Err("書き出し履歴の元ハッシュが不正です".into());
                }
            }
            let name: String = row.get(7).map_err(|e| e.to_string())?;
            if portable::basename(&name)? != name {
                return Err("書き出し履歴の保存先はファイル名だけである必要があります".into());
            }
            if !matches!(
                row.get::<_, String>(9).map_err(|e| e.to_string())?.as_str(),
                "succeeded" | "failed" | "interrupted"
            ) {
                return Err("未確定の書き出し履歴は復元できません".into());
            }
            if let Some(revision) = row.get::<_, Option<String>>(3).map_err(|e| e.to_string())? {
                let owned: bool = source.query_row("SELECT EXISTS(SELECT 1 FROM minutes_revisions WHERE id=?1 AND meeting_id=?2 AND publication_state='committed')",params![revision,meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
                if !owned {
                    return Err("書き出し履歴の議事録が別会議・未確定です".into());
                }
            }
            // Historical draft IDs can legitimately outlive their row. If the
            // row still exists, however, it must be this meeting's minutes draft.
            if let Some(draft) = row.get::<_, Option<String>>(8).map_err(|e| e.to_string())? {
                let owner: Option<(String, String)> = source
                    .query_row(
                        "SELECT meeting_id,target_kind FROM working_drafts WHERE id=?1",
                        params![draft],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if owner.is_some_and(|(m, k)| m != meeting || k != "minutes") {
                    return Err("書き出し履歴の編集由来が別会議・別種別です".into());
                }
            }
            let new = fixed.ids.get("export_records", &meeting, &id)?;
            let actual: Vec<rusqlite::types::Value> = self
                .database
                .query_row(
                    "SELECT * FROM export_records WHERE id=?1",
                    params![new],
                    |r| (0..table.columns.len()).map(|i| r.get(i)).collect(),
                )
                .map_err(|e| e.to_string())?;
            if actual != fixed.typed_row(table, rowid)? {
                return Err("構築済み書き出し履歴の外部結果・参照が変更されています".into());
            }
            // request/content/output hashes describe the past external export,
            // not new-ID content. Never read/rewrite a host path or fabricate a
            // new hash from today's draft or claim the external file exists.
        }
        Ok(())
    }

    fn interrupt_jobs(
        &self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let mut query = fixed
            .ids
            .source()
            .database
            .prepare("SELECT id,meeting_id FROM processing_jobs ORDER BY id")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let old: JobSnapshot =
                crate::storage::jobs::load_job(&fixed.ids.source().database, &id)?
                    .ok_or("復元元の処理履歴がありません")?;
            let new = fixed.ids.get("processing_jobs", &meeting, &id)?;
            let rebuilt = crate::storage::jobs::load_job(&self.database, &new)?
                .ok_or("復元先の処理履歴がありません")?;
            let spec: JobSpec = fixed.job_spec(&meeting, &id)?;
            let mut expected = old.clone();
            expected.id = new.clone();
            expected.meeting_id = spec.meeting_id.clone();
            expected.operation_id = spec.operation_id.clone();
            expected.input_hash = spec.input_hash.clone();
            expected.spec_hash = fixed.resolve("processing_jobs", &meeting, &id, &old.spec_hash)?;
            expected.retry_of_job_id = old
                .retry_of_job_id
                .as_deref()
                .map(|i| fixed.ids.get("processing_jobs", &meeting, i))
                .transpose()?;
            expected.output_analysis_id = old
                .output_analysis_id
                .as_deref()
                .map(|i| fixed.ids.get("analysis_revisions", &meeting, i))
                .transpose()?;
            expected.output_minutes_id = old
                .output_minutes_id
                .as_deref()
                .map(|i| fixed.ids.get("minutes_revisions", &meeting, i))
                .transpose()?;
            if crate::storage::jobs::read_fixed_input(&self.database, &new)?.0 != spec
                || rebuilt != expected
            {
                return Err("構築済み処理履歴の固定入力・進捗・出力が変更されています".into());
            }
            if matches!(
                old.state.as_str(),
                "queued" | "running" | "pausing" | "paused" | "interrupted"
            ) {
                self.database.execute("UPDATE processing_jobs SET state='interrupted',pause_reason=NULL,error_code='restored_preparation_required' WHERE id=?1",params![new]).map_err(|e|e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
#[path = "history_tests.rs"]
mod tests;
