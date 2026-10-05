use super::*;
use rusqlite::{Row, TransactionBehavior};
use serde::Deserialize;
use serde_json::Value;

const MAX_CHECKPOINT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Transcription,
    Diarization,
    TranscriptCleanup,
    MinutesGeneration,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Transcription => "transcription",
            Self::Diarization => "diarization",
            Self::TranscriptCleanup => "transcript_cleanup",
            Self::MinutesGeneration => "minutes_generation",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub meeting_id: String,
    pub kind: JobKind,
    pub operation_id: String,
    pub input_analysis_id: Option<String>,
    pub input_hash: String,
    pub profile_snapshot: Value,
    pub parameters: Value,
    pub total_units: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub meeting_id: String,
    pub kind: String,
    pub state: String,
    pub operation_id: String,
    pub input_hash: String,
    pub spec_hash: String,
    pub completed_units: i64,
    pub total_units: Option<i64>,
    pub error_code: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub output_analysis_id: Option<String>,
    pub output_minutes_id: Option<String>,
    pub pause_reason: Option<String>,
    pub retry_of_job_id: Option<String>,
    pub attempt: i64,
}

impl Storage {
    pub(crate) fn resume_recording_jobs(&self) -> Result<(), String> {
        let ids = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut statement = connection.prepare("SELECT id FROM processing_jobs WHERE state = 'paused' AND pause_reason = 'recording' ORDER BY created_at, rowid").map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        for id in ids {
            self.resume_job(&id)?;
        }
        Ok(())
    }

    pub(crate) fn pause_for_exit(&self) -> Result<(), String> {
        self.connection.lock().map_err(|_| "DBロックを取得できません")?.execute("UPDATE processing_jobs SET state = 'pausing', pause_reason = 'exit', updated_at = ?1 WHERE state = 'running'", params![now_ms()?]).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[allow(dead_code)]
    pub fn enqueue_job(&self, spec: JobSpec) -> Result<JobSnapshot, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let snapshot = enqueue_in_transaction(&transaction, &spec)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(snapshot)
    }

    #[allow(dead_code)]
    pub fn acquire_next_job(&self) -> Result<Option<JobSnapshot>, String> {
        self.acquire_job_kind(None)
    }

    pub(crate) fn acquire_job_kind(
        &self,
        kind: Option<JobKind>,
    ) -> Result<Option<JobSnapshot>, String> {
        self.acquire_filtered_job(kind, false)
    }

    pub(crate) fn acquire_local_inference_job(&self) -> Result<Option<JobSnapshot>, String> {
        self.acquire_filtered_job(None, true)
    }

    fn acquire_filtered_job(
        &self,
        kind: Option<JobKind>,
        local_only: bool,
    ) -> Result<Option<JobSnapshot>, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let recording_active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing', 'recording', 'stopping'))",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let job_active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running', 'pausing'))",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if recording_active || job_active {
            return Ok(None);
        }
        let queued: Option<(String, String, Option<String>, String)> = transaction.query_row(
            "SELECT id, meeting_id, input_analysis_id, input_hash FROM processing_jobs WHERE state = 'queued' AND (?1 IS NULL OR kind = ?1) AND (?2 = 0 OR kind IN ('transcription','diarization','minutes_generation')) ORDER BY created_at, rowid LIMIT 1",
            params![kind.map(JobKind::as_str), local_only], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(|error| error.to_string())?;
        let Some((id, meeting_id, analysis_id, input_hash)) = queued else {
            return Ok(None);
        };
        validate_job_input(
            &transaction,
            &meeting_id,
            analysis_id.as_deref(),
            &input_hash,
        )?;
        verify_latest_checkpoint(&transaction, &id)?;
        let now = now_ms()?;
        transaction.execute(
            "UPDATE processing_jobs SET state = 'running', started_at = COALESCE(started_at, ?1), updated_at = ?1 WHERE id = ?2 AND state = 'queued'",
            params![now, id],
        ).map_err(|error| error.to_string())?;
        let snapshot = load_job(&transaction, &id)?.ok_or("処理を読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(Some(snapshot))
    }

    #[allow(dead_code)]
    pub fn request_pause_for_recording(&self) -> Result<bool, String> {
        let changed = self.connection.lock().map_err(|_| "DBロックを取得できません".to_string())?
            .execute("UPDATE processing_jobs SET state = 'pausing', pause_reason = 'recording', updated_at = ?1 WHERE state = 'running'", params![now_ms()?])
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    #[allow(dead_code)]
    pub fn save_job_checkpoint(
        &self,
        job_id: &str,
        expected_completed_units: i64,
        completed_units: i64,
        checkpoint: Value,
    ) -> Result<JobSnapshot, String> {
        if expected_completed_units < 0
            || completed_units < expected_completed_units
            || !checkpoint.is_object()
        {
            return Err("処理の再開位置が不正です".into());
        }
        let checkpoint_json =
            serde_json::to_string(&checkpoint).map_err(|error| error.to_string())?;
        if checkpoint_json.len() > MAX_CHECKPOINT_BYTES {
            return Err("再開情報が大きすぎます".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current: Option<(String, i64, Option<i64>)> = transaction
            .query_row(
                "SELECT state, completed_units, total_units FROM processing_jobs WHERE id = ?1",
                params![job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (state, current_units, total_units) = current.ok_or("処理が見つかりません")?;
        if !matches!(state.as_str(), "running" | "pausing")
            || current_units != expected_completed_units
        {
            return Err("処理状態または再開位置が変わりました".into());
        }
        if total_units.is_some_and(|total| completed_units > total) {
            return Err("再開位置が総単位数を超えています".into());
        }
        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM job_checkpoints WHERE job_id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let spec_hash: String = transaction
            .query_row(
                "SELECT spec_hash FROM processing_jobs WHERE id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let checkpoint_hash = checkpoint_hash(&spec_hash, completed_units, &checkpoint_json);
        let now = now_ms()?;
        transaction.execute(
            "INSERT INTO job_checkpoints (id, job_id, sequence, completed_units, checkpoint_json, checkpoint_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![Uuid::new_v4().to_string(), job_id, sequence, completed_units, checkpoint_json, checkpoint_hash, now],
        ).map_err(|error| error.to_string())?;
        let next_state = if state == "pausing" {
            "paused"
        } else {
            "running"
        };
        transaction.execute(
            "UPDATE processing_jobs SET state = ?1, completed_units = ?2, updated_at = ?3 WHERE id = ?4 AND state = ?5 AND completed_units = ?6",
            params![next_state, completed_units, now, job_id, state, current_units],
        ).map_err(|error| error.to_string())?;
        let snapshot = load_job(&transaction, job_id)?.ok_or("処理を読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(snapshot)
    }

    #[allow(dead_code)]
    pub fn resume_job(&self, job_id: &str) -> Result<JobSnapshot, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current: Option<(String, String, Option<String>, String)> = transaction.query_row(
            "SELECT state, meeting_id, input_analysis_id, input_hash FROM processing_jobs WHERE id = ?1",
            params![job_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (state, meeting_id, analysis_id, input_hash) = current.ok_or("処理が見つかりません")?;
        if !matches!(state.as_str(), "paused" | "interrupted") {
            return Err("再開可能な処理状態ではありません".into());
        }
        let has_retry: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE retry_of_job_id = ?1)",
                params![job_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if has_retry {
            return Err("再試行先があるため元ジョブは再開できません".into());
        }
        validate_job_input(
            &transaction,
            &meeting_id,
            analysis_id.as_deref(),
            &input_hash,
        )?;
        verify_latest_checkpoint(&transaction, job_id)?;
        transaction.execute(
            "UPDATE processing_jobs SET state = 'queued', pause_reason = NULL, updated_at = ?1 WHERE id = ?2 AND state = ?3",
            params![now_ms()?, job_id, state],
        ).map_err(|error| error.to_string())?;
        let snapshot = load_job(&transaction, job_id)?.ok_or("処理を読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(snapshot)
    }

    pub(crate) fn get_job(&self, job_id: &str) -> Result<JobSnapshot, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        load_job(&connection, job_id)?.ok_or_else(|| "処理が見つかりません".into())
    }

    pub(crate) fn list_jobs(&self, meeting_id: &str) -> Result<Vec<JobSnapshot>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = connection.prepare("SELECT id, meeting_id, kind, state, operation_id, input_hash, spec_hash, completed_units, total_units, error_code, created_at, updated_at, output_analysis_id, pause_reason, retry_of_job_id, attempt, output_minutes_id FROM processing_jobs WHERE meeting_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 100").map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![meeting_id], job_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn fixed_job_input(&self, job_id: &str) -> Result<(JobSpec, Option<Value>), String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        read_fixed_input(&transaction, job_id)
    }

    // Expensive file verification occurs before this transaction. Recheck the
    // exact DB snapshot before copying only its durable checkpoint references.
    pub(crate) fn enqueue_retry_job(
        &self,
        source: &JobSnapshot,
        verified_spec: &JobSpec,
        verified_checkpoint: Option<&Value>,
        operation_id: &str,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_retry_inner(
            source,
            verified_spec,
            verified_checkpoint,
            operation_id,
            None,
            &|| false,
        )
    }

    // The facade verifies native inputs/outputs, current model bytes and trial
    // artifacts off the transaction, then commits this separately reviewed
    // attempt. Original logical parameters and historical assets stay fixed.
    pub(crate) fn enqueue_rebound_retry_job(
        &self,
        source: &JobSnapshot,
        verified_spec: &JobSpec,
        checkpoint: Option<&Value>,
        operation: &str,
        binding: &super::model_profiles::jobs::rebinding::Binding,
        cancelled: &impl Fn() -> bool,
    ) -> Result<JobSnapshot, String> {
        self.enqueue_retry_inner(
            source,
            verified_spec,
            checkpoint,
            operation,
            Some(binding),
            cancelled,
        )
    }

    fn enqueue_retry_inner(
        &self,
        source: &JobSnapshot,
        verified_spec: &JobSpec,
        verified_checkpoint: Option<&Value>,
        operation_id: &str,
        binding: Option<&super::model_profiles::jobs::rebinding::Binding>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<JobSnapshot, String> {
        if operation_id.trim().is_empty() || operation_id.len() > 128 {
            return Err("再試行の操作IDが不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let original = load_job(&transaction, &source.id)?.ok_or("元の処理が見つかりません")?;
        let (mut spec, checkpoint) = read_fixed_input(&transaction, &source.id)?;
        if &original != source
            || &spec != verified_spec
            || checkpoint.as_ref() != verified_checkpoint
        {
            return Err("検証後に元ジョブまたは再開情報が変化しています".into());
        }
        if !retry_source_permitted(&original, binding.is_some()) || original.attempt >= 1000 {
            return Err(
                "失敗・取消済みの処理だけ再試行できます。一時停止・中断には再開を使ってください"
                    .into(),
            );
        }
        let attempt = original
            .attempt
            .checked_add(1)
            .ok_or("再試行番号が範囲外です")?;
        if let Some(binding) = binding {
            spec.profile_snapshot = binding.attach(&spec)?;
        }
        spec.operation_id = operation_id.into();
        let hash = retry_spec_hash(&spec, &original.id, attempt)?;
        if let Some(existing) = transaction
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id = ?1",
                params![operation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            let existing = load_job(&transaction, &existing)?.ok_or("再試行が見つかりません")?;
            read_fixed_input(&transaction, &existing.id)?;
            if existing.spec_hash != hash
                || existing.retry_of_job_id.as_deref() != Some(&original.id)
                || existing.attempt != attempt
            {
                return Err("同じ操作IDで別の再試行が指定されました".into());
            }
            return Ok(existing);
        }
        if let Some(binding) = binding {
            binding.gate(&transaction, &spec)?;
        }
        if cancelled() {
            return Err("再試行の登録を取り消しました".into());
        }
        let unfinished: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE retry_of_job_id = ?1 AND state IN ('queued', 'running', 'pausing', 'paused', 'interrupted'))", params![original.id], |row| row.get(0)).map_err(|e| e.to_string())?;
        if unfinished {
            return Err(
                "このジョブの再試行が既に存在します。再試行先の状態を確認してください".into(),
            );
        }
        validate_job_input(
            &transaction,
            &spec.meeting_id,
            spec.input_analysis_id.as_deref(),
            &spec.input_hash,
        )?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        let profile = serde_json::to_string(&spec.profile_snapshot).map_err(|e| e.to_string())?;
        let parameters = serde_json::to_string(&spec.parameters).map_err(|e| e.to_string())?;
        // A bound profile includes two complete provenance records and native
        // paths; keep it bounded without shrinking the original 64KiB record.
        if profile.len() > 256 * 1024 || parameters.len() > MAX_CHECKPOINT_BYTES {
            return Err("再試行の固定条件が保存サイズ上限を超えています".into());
        }
        transaction.execute("INSERT INTO processing_jobs (id, meeting_id, kind, state, operation_id, retry_of_job_id, attempt, input_analysis_id, input_hash, profile_snapshot_json, parameters_json, spec_hash, total_units, created_at, updated_at) VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)", params![id, spec.meeting_id, spec.kind.as_str(), operation_id, original.id, attempt, spec.input_analysis_id, spec.input_hash, profile, parameters, hash, spec.total_units, now]).map_err(|e| e.to_string())?;
        if let Some(checkpoint) = checkpoint {
            let json = serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?;
            transaction.execute("INSERT INTO job_checkpoints (id, job_id, sequence, completed_units, checkpoint_json, checkpoint_hash, created_at) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)", params![Uuid::new_v4().to_string(), id, original.completed_units, json, checkpoint_hash(&hash, original.completed_units, &json), now]).map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "UPDATE processing_jobs SET completed_units = ?1 WHERE id = ?2",
                    params![original.completed_units, id],
                )
                .map_err(|e| e.to_string())?;
        }
        let next = load_job(&transaction, &id)?.ok_or("再試行を読み込めません")?;
        verify_latest_checkpoint(&transaction, &id)?;
        if cancelled() {
            return Err("再試行の登録を取り消しました".into());
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(next)
    }

    // Lost-response lookup is read-only and never rechecks today's profile or
    // filesystem. It cannot enqueue, change source state or dispatch inference.
    pub(crate) fn rebound_retry_result(
        &self,
        source: &JobSnapshot,
        verified_spec: &JobSpec,
        checkpoint: Option<&Value>,
        operation: &str,
        binding: &super::model_profiles::jobs::rebinding::Binding,
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
        let Some(id) = id else {
            return Ok(None);
        };
        let current = load_job(&db, &source.id)?.ok_or("再試行元がありません")?;
        let (mut spec, cp) = read_fixed_input(&db, &source.id)?;
        if current.spec_hash != source.spec_hash
            || &spec != verified_spec
            || cp.as_ref() != checkpoint
        {
            return Err("確認した再試行元の固定条件・完了結果が一致しません".into());
        }
        spec.profile_snapshot = binding.attach(&spec)?;
        spec.operation_id = operation.into();
        let attempt = source
            .attempt
            .checked_add(1)
            .ok_or("再試行番号が不正です")?;
        let result = load_job(&db, &id)?.ok_or("再試行がありません")?;
        read_fixed_input(&db, &id)?;
        if result.spec_hash != retry_spec_hash(&spec, &source.id, attempt)?
            || result.retry_of_job_id.as_deref() != Some(source.id.as_str())
            || result.attempt != attempt
        {
            return Err("同じ操作IDで別の再試行が指定されました".into());
        }
        Ok(Some(result))
    }

    // Retry parameters remain byte-for-byte fixed, including the original
    // generation request's operation ID. Resolve it through verified lineage,
    // rather than weakening the envelope's operation binding.
    pub(crate) fn fixed_job_initial_operation(&self, job_id: &str) -> Result<String, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (fixed, _) = read_fixed_input(&tx, job_id)?;
        let current = load_job(&tx, job_id)?.ok_or("処理がありません")?;
        initial_operation(current, &fixed, |parent_id| {
            let parent = load_job(&tx, parent_id)?.ok_or("再試行元がありません")?;
            let (spec, _) = read_fixed_input(&tx, parent_id)?;
            Ok((parent, spec))
        })
    }

    pub(crate) fn request_job_pause(
        &self,
        job_id: &str,
        reason: &str,
    ) -> Result<JobSnapshot, String> {
        if !matches!(reason, "user" | "recording" | "exit") {
            return Err("一時停止理由が不正です".into());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let job = load_job(&connection, job_id)?.ok_or("処理が見つかりません")?;
        if matches!(job.state.as_str(), "paused" | "pausing") {
            if reason == "user" {
                connection.execute("UPDATE processing_jobs SET pause_reason = 'user', updated_at = ?1 WHERE id = ?2", params![now_ms()?, job_id]).map_err(|e| e.to_string())?;
            }
            return load_job(&connection, job_id)?.ok_or_else(|| "処理が見つかりません".into());
        }
        if !matches!(job.state.as_str(), "queued" | "running") {
            return Err("一時停止可能な処理ではありません".into());
        }
        connection.execute("UPDATE processing_jobs SET state = ?1, pause_reason = ?2, updated_at = ?3 WHERE id = ?4", params![if job.state == "queued" {"paused"} else {"pausing"}, reason, now_ms()?, job_id]).map_err(|e| e.to_string())?;
        load_job(&connection, job_id)?.ok_or_else(|| "処理が見つかりません".into())
    }

    #[allow(dead_code)]
    pub fn finish_job(
        &self,
        job_id: &str,
        expected_completed_units: i64,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<JobSnapshot, String> {
        if expected_completed_units < 0 {
            return Err("処理の再開位置が不正です".into());
        }
        match outcome {
            "succeeded" | "cancelled" if error_code.is_some() => {
                return Err("成功・取消にエラーコードは付けられません".into());
            }
            "failed"
                if error_code.is_none_or(|code| code.trim().is_empty() || code.len() > 128) =>
            {
                return Err("失敗にはエラーコードが必要です".into());
            }
            "succeeded" | "failed" | "cancelled" => {}
            _ => return Err("処理の終了状態が不正です".into()),
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current: Option<(String, i64, Option<i64>)> = transaction
            .query_row(
                "SELECT state, completed_units, total_units FROM processing_jobs WHERE id = ?1",
                params![job_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (state, completed_units, total_units) = current.ok_or("処理が見つかりません")?;
        if completed_units != expected_completed_units {
            return Err("処理の再開位置が変わりました".into());
        }
        let permitted = match outcome {
            "succeeded" => {
                state == "running" && total_units.is_none_or(|total| completed_units == total)
            }
            "failed" => matches!(
                state.as_str(),
                "running" | "pausing" | "paused" | "interrupted"
            ),
            "cancelled" => matches!(state.as_str(), "queued" | "paused" | "interrupted"),
            _ => false,
        };
        if !permitted {
            return Err("現在の処理状態から終了できません".into());
        }
        verify_latest_checkpoint(&transaction, job_id)?;
        let now = now_ms()?;
        let changed = transaction.execute(
            "UPDATE processing_jobs SET state = ?1, error_code = ?2, finished_at = ?3, updated_at = ?3 WHERE id = ?4 AND state = ?5 AND completed_units = ?6",
            params![outcome, error_code, now, job_id, state, completed_units],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("処理の終了競合が発生しました".into());
        }
        let snapshot = load_job(&transaction, job_id)?.ok_or("処理を読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(snapshot)
    }
}

fn job_from_row(row: &Row<'_>) -> rusqlite::Result<JobSnapshot> {
    Ok(JobSnapshot {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        kind: row.get(2)?,
        state: row.get(3)?,
        operation_id: row.get(4)?,
        input_hash: row.get(5)?,
        spec_hash: row.get(6)?,
        completed_units: row.get(7)?,
        total_units: row.get(8)?,
        error_code: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        output_analysis_id: row.get(12)?,
        output_minutes_id: row.get(16)?,
        pause_reason: row.get(13)?,
        retry_of_job_id: row.get(14)?,
        attempt: row.get(15)?,
    })
}

pub(super) fn enqueue_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    spec: &JobSpec,
) -> Result<JobSnapshot, String> {
    if spec.operation_id.trim().is_empty() || spec.operation_id.len() > 128 {
        return Err("処理の操作IDが不正です".into());
    }
    if !valid_hash(&spec.input_hash) || spec.total_units.is_some_and(|total| total < 0) {
        return Err("処理の入力ハッシュまたは総単位数が不正です".into());
    }
    if !spec.profile_snapshot.is_object() || !spec.parameters.is_object() {
        return Err("処理設定はJSONオブジェクトが必要です".into());
    }
    if matches!(
        spec.kind,
        JobKind::MinutesGeneration | JobKind::TranscriptCleanup | JobKind::Diarization
    ) != spec.input_analysis_id.is_some()
    {
        return Err("処理種別と入力解析版が一致しません".into());
    }
    let profile_json =
        serde_json::to_string(&spec.profile_snapshot).map_err(|error| error.to_string())?;
    let parameters_json =
        serde_json::to_string(&spec.parameters).map_err(|error| error.to_string())?;
    if profile_json.len() > 65_536 || parameters_json.len() > MAX_CHECKPOINT_BYTES {
        return Err("処理設定が大きすぎます".into());
    }
    let spec_hash = fixed_spec_hash(spec, None, 1)?;
    if let Some(existing) = transaction.query_row(
            "SELECT id, meeting_id, kind, state, operation_id, input_hash, spec_hash, completed_units, total_units, error_code, created_at, updated_at, output_analysis_id, pause_reason, retry_of_job_id, attempt, output_minutes_id FROM processing_jobs WHERE operation_id = ?1",
            params![spec.operation_id],
            job_from_row,
        ).optional().map_err(|error| error.to_string())? {
            if existing.spec_hash != spec_hash {
                return Err("同じ操作IDで異なる処理条件が指定されました".into());
            }
            return Ok(existing);
        }
    super::model_profiles::jobs::gate(transaction, spec)?;
    let meeting_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
            params![spec.meeting_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !meeting_exists {
        return Err("会議が見つかりません".into());
    }
    validate_job_input(
        &transaction,
        &spec.meeting_id,
        spec.input_analysis_id.as_deref(),
        &spec.input_hash,
    )?;
    let id = Uuid::new_v4().to_string();
    let now = now_ms()?;
    transaction.execute(
            "INSERT INTO processing_jobs (id, meeting_id, kind, state, operation_id, input_analysis_id, input_hash, profile_snapshot_json, parameters_json, spec_hash, total_units, created_at, updated_at) VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![id, spec.meeting_id, spec.kind.as_str(), spec.operation_id, spec.input_analysis_id, spec.input_hash, profile_json, parameters_json, spec_hash, spec.total_units, now],
        ).map_err(|error| error.to_string())?;
    let snapshot = load_job(&transaction, &id)?.ok_or("処理を読み込めません")?;
    Ok(snapshot)
}

// Shared lineage policy. The caller supplies its validated spec reader: live
// jobs verify native hashes; portable sources retain original identity hashes.
pub(super) fn initial_operation(
    mut current: JobSnapshot,
    fixed: &JobSpec,
    mut read: impl FnMut(&str) -> Result<(JobSnapshot, JobSpec), String>,
) -> Result<String, String> {
    let mut child_spec = fixed.clone();
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current.id.clone()) || seen.len() > 1000 {
            return Err("再試行の由来が循環しているか長すぎます".into());
        }
        let Some(parent_id) = current.retry_of_job_id.as_deref() else {
            if current.attempt != 1 {
                return Err("元の試行番号が不正です".into());
            }
            return Ok(current.operation_id);
        };
        let (parent, spec) = read(parent_id)?;
        if parent.id != parent_id
            || parent.meeting_id != fixed.meeting_id
            || parent.attempt.checked_add(1) != Some(current.attempt)
            || !retry_source_permitted(
                &parent,
                child_spec.profile_snapshot.get("runtimeModel").is_some(),
            )
            || spec.meeting_id != fixed.meeting_id
            || spec.kind != fixed.kind
            || spec.input_analysis_id != fixed.input_analysis_id
            || spec.input_hash != fixed.input_hash
            || !super::model_profiles::jobs::rebinding::same_profile(&spec, fixed)?
            || spec.parameters != fixed.parameters
            || spec.total_units != fixed.total_units
        {
            return Err("再試行の固定条件または由来が一致しません".into());
        }
        current = parent;
        child_spec = spec;
    }
}

pub(crate) fn retry_source_permitted(source: &JobSnapshot, rebinding: bool) -> bool {
    matches!(source.state.as_str(), "failed" | "cancelled")
        || (rebinding
            && source.state == "interrupted"
            && source.error_code.as_deref() == Some("restored_preparation_required"))
}

#[cfg(test)]
pub(in crate::storage) fn remove_rebinding_schema_for_test(db: &Connection) {
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=26)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if !present {
        return;
    }
    assert_eq!(db.query_row("SELECT COUNT(*) FROM processing_jobs WHERE json_type(profile_snapshot_json,'$.runtimeModel') IS NOT NULL", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    db.execute_batch("DROP TRIGGER job_retry_valid; DROP TRIGGER job_parent_retry_no_resume; DELETE FROM schema_migrations WHERE version=26;").unwrap();
    db.execute_batch(
        super::MIGRATION_12
            .split("\nCREATE TRIGGER job_attempt_fixed")
            .next()
            .unwrap(),
    )
    .unwrap();
}

pub(super) fn load_job(connection: &Connection, id: &str) -> Result<Option<JobSnapshot>, String> {
    connection.query_row(
        "SELECT id, meeting_id, kind, state, operation_id, input_hash, spec_hash, completed_units, total_units, error_code, created_at, updated_at, output_analysis_id, pause_reason, retry_of_job_id, attempt, output_minutes_id FROM processing_jobs WHERE id = ?1",
        params![id], job_from_row,
    ).optional().map_err(|error| error.to_string())
}

pub(super) fn read_fixed_input(
    connection: &Connection,
    job_id: &str,
) -> Result<(JobSpec, Option<Value>), String> {
    let spec: JobSpec = connection.query_row("SELECT meeting_id, kind, operation_id, input_analysis_id, input_hash, profile_snapshot_json, parameters_json, total_units FROM processing_jobs WHERE id = ?1", params![job_id], |row| {
        let kind: String = row.get(1)?;
        let profile: String = row.get(5)?;
        let parameters: String = row.get(6)?;
        let decode = |index, error| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error));
        Ok(JobSpec { meeting_id: row.get(0)?, kind: serde_json::from_value(Value::String(kind)).map_err(|e| decode(1, e))?, operation_id: row.get(2)?, input_analysis_id: row.get(3)?, input_hash: row.get(4)?, profile_snapshot: serde_json::from_str(&profile).map_err(|e| decode(5, e))?, parameters: serde_json::from_str(&parameters).map_err(|e| decode(6, e))?, total_units: row.get(7)? })
    }).map_err(|e| e.to_string())?;
    let job = load_job(connection, job_id)?.ok_or("処理が見つかりません")?;
    let hash = fixed_spec_hash(&spec, job.retry_of_job_id.as_deref(), job.attempt)?;
    if hash != job.spec_hash {
        return Err("処理の固定設定または再試行情報が変更されています".into());
    }
    if spec.profile_snapshot.get("runtimeModel").is_some() {
        super::model_profiles::jobs::rebinding::validate_spec(&spec)?;
    }
    verify_latest_checkpoint(connection, job_id)?;
    let checkpoint: Option<String> = connection.query_row("SELECT checkpoint_json FROM job_checkpoints WHERE job_id = ?1 ORDER BY sequence DESC LIMIT 1", params![job_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    Ok((
        spec,
        checkpoint
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|e| e.to_string())?,
    ))
}

fn retry_spec_hash(spec: &JobSpec, source: &str, attempt: i64) -> Result<String, String> {
    fixed_spec_hash(spec, Some(source), attempt)
}

pub(super) fn fixed_spec_hash(
    spec: &JobSpec,
    source: Option<&str>,
    attempt: i64,
) -> Result<String, String> {
    match source {
        None if attempt == 1 => Ok(hash_bytes(
            b"mon-job-spec-v1",
            &serde_json::to_vec(spec).map_err(|e| e.to_string())?,
        )),
        Some(source) if attempt > 1 => Ok(hash_bytes(
            b"mon-job-retry-spec-v1",
            &serde_json::to_vec(&(spec, source, attempt)).map_err(|e| e.to_string())?,
        )),
        _ => Err("処理の再試行情報が不正です".into()),
    }
}

fn validate_job_input(
    connection: &Connection,
    meeting_id: &str,
    analysis_id: Option<&str>,
    input_hash: &str,
) -> Result<(), String> {
    if let Some(analysis_id) = analysis_id {
        let fixed: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state IN ('committed', 'internal_snapshot') AND content_hash = ?3)",
            params![analysis_id, meeting_id, input_hash], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !fixed {
            return Err("処理の入力解析版が変更されています".into());
        }
    }
    Ok(())
}

pub(super) fn verify_latest_checkpoint(
    connection: &Connection,
    job_id: &str,
) -> Result<(), String> {
    let job: (String, i64) = connection
        .query_row(
            "SELECT spec_hash, completed_units FROM processing_jobs WHERE id = ?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let latest: Option<(i64, String, String)> = connection.query_row(
        "SELECT completed_units, checkpoint_json, checkpoint_hash FROM job_checkpoints WHERE job_id = ?1 ORDER BY sequence DESC LIMIT 1",
        params![job_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(|error| error.to_string())?;
    match latest {
        Some((units, json, hash))
            if units == job.1 && checkpoint_hash(&job.0, units, &json) == hash =>
        {
            Ok(())
        }
        None if job.1 == 0 => Ok(()),
        _ => Err("処理の再開情報が変更されています".into()),
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(super) fn checkpoint_hash(spec_hash: &str, units: i64, json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mon-job-checkpoint-v1");
    hasher.update(spec_hash.as_bytes());
    hasher.update(units.to_be_bytes());
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
#[path = "job_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(super) fn audio_job(meeting_id: &str, operation_id: &str) -> JobSpec {
        JobSpec {
            meeting_id: meeting_id.into(),
            kind: JobKind::Transcription,
            operation_id: operation_id.into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: json!({"engine": "test", "model_hash": "b".repeat(64)}),
            parameters: json!({"window_seconds": 30}),
            total_units: Some(10),
        }
    }

    #[test]
    fn job_pauses_at_checkpoint_and_recording_blocks_resume() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("処理").unwrap();
        let spec = audio_job(&meeting.id, "operation-1");
        let queued = storage.enqueue_job(spec.clone()).unwrap();
        assert_eq!(queued.state, "queued");
        assert_eq!(storage.enqueue_job(spec.clone()).unwrap(), queued);
        let mut changed = spec;
        changed.parameters = json!({"window_seconds": 60});
        assert!(storage.enqueue_job(changed).is_err());
        let running = storage.acquire_next_job().unwrap().unwrap();
        assert_eq!(running.id, queued.id);
        assert_eq!(running.state, "running");
        assert!(storage.acquire_next_job().unwrap().is_none());
        assert!(storage.request_pause_for_recording().unwrap());
        assert!(storage.acquire_next_job().unwrap().is_none());
        assert!(storage
            .save_job_checkpoint(&running.id, 1, 2, json!({"part": 2}))
            .is_err());
        let paused = storage
            .save_job_checkpoint(&running.id, 0, 2, json!({"part": 2}))
            .unwrap();
        assert_eq!(paused.state, "paused");
        assert_eq!(paused.completed_units, 2);
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE meetings SET source_type = 'recording' WHERE id = ?1",
                params![meeting.id],
            )
            .unwrap();
        storage.connection.lock().unwrap().execute(
            "INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES ('recording', ?1, 'recording', 'system', '{}', 'start', 0, 0)",
            params![meeting.id],
        ).unwrap();
        assert_eq!(storage.resume_job(&running.id).unwrap().state, "queued");
        assert!(storage.acquire_next_job().unwrap().is_none());
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE recording_sessions SET state = 'saved' WHERE id = 'recording'",
                [],
            )
            .unwrap();
        assert_eq!(
            storage.acquire_next_job().unwrap().unwrap().completed_units,
            2
        );
        assert!(storage
            .save_job_checkpoint(&running.id, 2, 11, json!({}))
            .is_err());
        assert_eq!(
            storage
                .save_job_checkpoint(&running.id, 2, 10, json!({"part": 10}))
                .unwrap()
                .state,
            "running"
        );
        assert!(storage
            .finish_job(&running.id, 2, "succeeded", None)
            .is_err());
        assert_eq!(
            storage
                .finish_job(&running.id, 10, "succeeded", None)
                .unwrap()
                .state,
            "succeeded"
        );
        assert!(storage.resume_job(&running.id).is_err());
    }

    #[test]
    fn running_job_is_interrupted_on_reopen_and_corrupt_checkpoint_blocks_resume() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("処理").unwrap();
        let job = storage
            .enqueue_job(audio_job(&meeting.id, "operation-2"))
            .unwrap();
        storage.acquire_next_job().unwrap().unwrap();
        storage
            .save_job_checkpoint(&job.id, 0, 3, json!({"part": 3}))
            .unwrap();
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(
            load_job(&reopened.connection.lock().unwrap(), &job.id)
                .unwrap()
                .unwrap()
                .state,
            "interrupted"
        );
        assert_eq!(reopened.resume_job(&job.id).unwrap().state, "queued");
        assert_eq!(
            reopened
                .acquire_next_job()
                .unwrap()
                .unwrap()
                .completed_units,
            3
        );
        drop(reopened);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        reopened
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE job_checkpoints SET checkpoint_json = '{\"part\":4}' WHERE job_id = ?1",
                params![job.id],
            )
            .unwrap();
        assert!(reopened.resume_job(&job.id).is_err());
    }

    #[test]
    fn minutes_job_requires_matching_committed_analysis() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("処理").unwrap();
        let other = storage.create_meeting("別会議").unwrap();
        storage.connection.lock().unwrap().execute(
            "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES ('analysis', ?1, 1, 'engine', 'candidate', ?2, '{}', '{}', ?3, 1, 0)",
            params![meeting.id, "a".repeat(64), "0".repeat(64)],
        ).unwrap();
        let committed = storage
            .commit_analysis_candidate(&meeting.id, "analysis", 0, false)
            .unwrap();
        let mut spec = audio_job(&other.id, "minutes-1");
        spec.kind = JobKind::MinutesGeneration;
        spec.input_analysis_id = Some("analysis".into());
        spec.input_hash = committed.content_hash.clone();
        assert!(storage.enqueue_job(spec.clone()).is_err());
        spec.meeting_id = meeting.id;
        spec.input_hash = "f".repeat(64);
        assert!(storage.enqueue_job(spec.clone()).is_err());
        spec.input_hash = committed.content_hash;
        assert_eq!(storage.enqueue_job(spec).unwrap().state, "queued");
        let cancelled = storage
            .enqueue_job(audio_job(&other.id, "cancel-1"))
            .unwrap();
        assert_eq!(
            storage
                .finish_job(&cancelled.id, 0, "cancelled", None)
                .unwrap()
                .state,
            "cancelled"
        );
        assert!(storage
            .finish_job(&cancelled.id, 0, "failed", Some("retry"))
            .is_err());
    }
}
