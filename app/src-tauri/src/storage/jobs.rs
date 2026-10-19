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
    MinutesGeneration,
    VoiceRegistration,
    VoiceMatching,
    SpeakerCompletion,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Transcription => "transcription",
            Self::Diarization => "diarization",
            Self::MinutesGeneration => "minutes_generation",
            Self::VoiceRegistration => "voice_registration",
            Self::VoiceMatching => "voice_matching",
            Self::SpeakerCompletion => "speaker_completion",
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
pub struct ExecutionProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub phase_completed: usize,
    pub phase_total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_progress: Option<ExecutionProgress>,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationJob {
    #[serde(flatten)]
    pub job: JobSnapshot,
    pub meeting_title: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationOutput {
    pub job: JobSnapshot,
    pub content_hash: String,
    pub publication_state: String,
    pub warnings: Vec<String>,
}

impl Storage {
    // Read the exact output, not the UI's bounded candidate history. One
    // transaction binds the immutable job, meeting ownership and stored hash.
    pub(crate) fn notification_output(&self, job_id: &str) -> Result<NotificationOutput, String> {
        uuid::Uuid::parse_str(job_id).map_err(|_| "解析通知のジョブIDが不正です")?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let job = load_job(&tx, job_id)?.ok_or("通知対象のジョブがありません")?;
        if job.state != "succeeded" {
            return Err("通知対象の処理は成功していません".into());
        }
        let (spec, checkpoint) = read_fixed_input(&tx, job_id)?;
        if spec.kind == JobKind::SpeakerCompletion {
            let c = checkpoint
                .as_ref()
                .ok_or("話者補完の完了記録がありません")?;
            if c["completionResult"] != true
                || job.completed_units != 1
                || job.total_units != Some(1)
                || c["jobId"] != job.id
                || job.output_analysis_id.is_some()
                || job.output_minutes_id.is_some()
            {
                return Err("話者補完の完了記録が一致しません".into());
            }
            let count = c["updatedCount"]
                .as_u64()
                .ok_or("話者補完の件数が不正です")?;
            if let Some(id) = c["outputAnalysisId"].as_str() {
                let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM analysis_revisions WHERE id=?1 AND meeting_id=?2 AND json_extract(config_json,'$.speakerCompletion.jobId')=?3)",params![id,job.meeting_id,job.id],|r|r.get(0)).map_err(|e|e.to_string())?;
                if !valid || count == 0 {
                    return Err("話者補完の保存版が一致しません".into());
                }
                let stored: String = tx
                    .query_row(
                        "SELECT content_hash FROM analysis_revisions WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if stored != super::revisions::analysis_content_hash(&tx, id)? {
                    return Err("話者補完の保存版の内容が変わっています".into());
                }
            } else if count != 0 {
                return Err("話者補完の保存版がありません".into());
            }
            let skipped = c["skipped"]
                .as_object()
                .ok_or("話者補完の見送り件数がありません")?
                .values()
                .map(|v| v.as_u64().unwrap_or(0))
                .sum::<u64>();
            let mut details = vec![format!("{}件を補完、{}件を見送りました", count, skipped)];
            if let Some(n) = c["needsReviewCount"].as_u64().filter(|n| *n > 0) {
                details.push(format!("うち{}件は低信頼のため要確認です", n));
            }
            for (reason, n) in c["skipped"].as_object().unwrap() {
                let label = match reason.as_str() {
                    "manual" => "手動指定を保護",
                    "detected_interval" => "検出済み区間あり",
                    "too_short" => "発言が1秒未満",
                    "too_long" => "切り出しが30秒超",
                    "missing_audio" => "音声欠損・不連続",
                    "feature_unavailable" => "特徴量を取得できず",
                    "no_reference" => "参照音声なし",
                    "tie" => "候補が同点",
                    "different_winners" => "本体と前後付きの候補が不一致",
                    "other_speaker_in_context" => "前後に別話者あり",
                    "below_threshold" => "閾値未達",
                    _ => "条件外",
                };
                details.push(format!("{}：{}件", label, n));
            }
            return Ok(NotificationOutput {
                content_hash: job.spec_hash.clone(),
                job,
                publication_state: "completion".into(),
                warnings: details,
            });
        }
        if matches!(
            spec.kind,
            JobKind::VoiceRegistration | JobKind::VoiceMatching
        ) {
            if checkpoint.as_ref().and_then(|c| c["voiceResult"].as_bool()) != Some(true)
                || job.completed_units != 1
                || job.total_units != Some(1)
                || job.output_analysis_id.is_some()
                || job.output_minutes_id.is_some()
            {
                return Err("声の処理の完了記録が一致しません".into());
            }
            return Ok(NotificationOutput {
                content_hash: job.spec_hash.clone(),
                job,
                publication_state: "voice".into(),
                warnings: vec![],
            });
        }
        if job.kind == "minutes_generation" && crate::generation::markdown_job::is_markdown(&spec) {
            verify_latest_checkpoint(&tx, job_id)?;
            if job.output_minutes_id.is_some()
                || job.output_analysis_id.is_some()
                || job.total_units != Some(job.completed_units)
                || job.completed_units < 1
            {
                return Err("議事録Markdownの完了状態が不正です".into());
            }
            let hash = checkpoint
                .as_ref()
                .and_then(|c| c["markdownHash"].as_str())
                .filter(|h| valid_hash(h))
                .ok_or("議事録の生成結果ハッシュがありません")?
                .to_string();
            let target: super::minutes_markdown::GenerationTarget =
                serde_json::from_value(spec.parameters["generation"]["markdownTarget"].clone())
                    .map_err(|e| e.to_string())?;
            let base = target.expected.as_ref().map_or(0, |s| s.lock_version);
            if target.schema_version != 1
                || base < 0
                || target
                    .expected
                    .as_ref()
                    .is_some_and(|s| s.lock_version < 1 || !valid_hash(&s.content_hash))
            {
                return Err("議事録の固定保存条件が不正です".into());
            }
            let saved_lock = base.checked_add(1).ok_or("議事録の保存状態が不正です")?;
            let current = super::minutes_markdown::read(&tx, &job.meeting_id)?
                .ok_or("保存した議事録がありません")?;
            if current.lock_version < saved_lock
                || current.operation_id != job.operation_id && current.lock_version == saved_lock
                || current.operation_id == job.operation_id
                    && (current.lock_version != saved_lock || current.content_hash != hash)
            {
                return Err("議事録の保存結果が一致しません。成功通知を保留します".into());
            }
            let publication_state = if current.operation_id == job.operation_id {
                "markdown"
            } else {
                "superseded"
            }
            .to_string();
            return Ok(NotificationOutput {
                job,
                content_hash: hash,
                publication_state,
                warnings: vec![],
            });
        }
        let (id, table, computed) = match job.kind.as_str() {
            "transcription" | "diarization" => {
                let id = job
                    .output_analysis_id
                    .as_deref()
                    .ok_or("保存結果の解析版がありません")?;
                (
                    id,
                    "analysis_revisions",
                    super::revisions::analysis_content_hash(&tx, id)?,
                )
            }
            "minutes_generation" => return Err("旧形式の議事録生成結果には対応していません".into()),
            _ => return Err("通知対象の処理種別が不正です".into()),
        };
        // Table name comes only from the fixed match above, never caller input.
        let query = format!("SELECT content_hash, publication_state FROM {table} WHERE id=?1 AND meeting_id=?2 AND publication_state IN ('candidate','committed')");
        let (content_hash, publication_state): (String, String) = tx
            .query_row(&query, params![id, job.meeting_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("ジョブと保存結果の会議または公開状態が一致しません")?;
        if !valid_hash(&content_hash) || content_hash != computed {
            return Err("保存結果の内容が変更されています。成功通知を保留します".into());
        }
        Ok(NotificationOutput {
            job,
            content_hash,
            publication_state,
            warnings: vec![],
        })
    }

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
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if self.retired.load(std::sync::atomic::Ordering::Acquire) {
            let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running','pausing'))", [], |r| r.get(0)).map_err(|e| e.to_string())?;
            if active {
                return Err("旧保存先に停止未完了の処理があります。書込停止を維持します".into());
            }
            return Ok(());
        }
        db.execute("UPDATE processing_jobs SET state = 'pausing', pause_reason = 'exit', updated_at = ?1 WHERE state = 'running'", params![now_ms()?]).map_err(|e| e.to_string())?;
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
        crate::internal_logs::job(&snapshot);
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
        self.acquire_filtered_job(kind)
    }

    pub(crate) fn acquire_local_inference_job(&self) -> Result<Option<JobSnapshot>, String> {
        self.acquire_filtered_job(None)
    }

    fn acquire_filtered_job(&self, kind: Option<JobKind>) -> Result<Option<JobSnapshot>, String> {
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
        // Preserve fixed historical jobs after audio deletion, but do not let
        // those queued entries starve another meeting or text-only work.
        let queued = {
            let mut query=transaction.prepare("SELECT id, meeting_id, input_analysis_id, input_hash,kind FROM processing_jobs WHERE state='queued' AND kind IN ('transcription','diarization','minutes_generation','voice_registration','voice_matching','speaker_completion') AND (?1 IS NULL OR kind=?1) ORDER BY created_at,rowid LIMIT 4097").map_err(|e|e.to_string())?;
            let mut rows = query
                .query(params![kind.map(JobKind::as_str)])
                .map_err(|e| e.to_string())?;
            let mut selected = None;
            let mut count = 0;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                count += 1;
                if count > 4096 {
                    return Err("音声利用可否を確認する待機ジョブが安全上限を超えています".into());
                }
                let id: String = row.get(0).map_err(|e| e.to_string())?;
                let meeting: String = row.get(1).map_err(|e| e.to_string())?;
                let job_kind: String = row.get(4).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                if matches!(
                    job_kind.as_str(),
                    "transcription"
                        | "diarization"
                        | "voice_registration"
                        | "voice_matching"
                        | "speaker_completion"
                ) && super::deletion::audio::intent::removal::completion::deleted(
                    &transaction,
                    &meeting,
                )? {
                    continue;
                }
                #[cfg(not(unix))]
                let _ = job_kind;
                selected = Some((
                    id,
                    meeting,
                    row.get::<_, Option<String>>(2).map_err(|e| e.to_string())?,
                    row.get::<_, String>(3).map_err(|e| e.to_string())?,
                ));
                break;
            }
            selected
        };
        let Some((id, meeting_id, analysis_id, input_hash)) = queued else {
            return Ok(None);
        };
        require_audio_source(
            &transaction,
            &load_job(&transaction, &id)?
                .ok_or("処理を読み込めません")?
                .kind,
            &meeting_id,
        )?;
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
        crate::internal_logs::job(&snapshot);
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
        crate::internal_logs::job(&snapshot);
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
        require_audio_source(
            &transaction,
            &load_job(&transaction, job_id)?
                .ok_or("処理を読み込めません")?
                .kind,
            &meeting_id,
        )?;
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
        crate::internal_logs::job(&snapshot);
        Ok(snapshot)
    }

    fn decorate_execution_progress(&self, job: &mut JobSnapshot) {
        if job.kind == "speaker_completion" && job.state == "running" {
            job.execution_progress = self
                .execution_progress
                .lock()
                .ok()
                .and_then(|map| map.get(&job.id).cloned());
        }
    }
    pub(crate) fn set_completion_progress(
        &self,
        id: &str,
        value: ExecutionProgress,
    ) -> Result<JobSnapshot, String> {
        if value.total == 0
            || value.completed >= value.total
            || value.phase_completed > value.phase_total
        {
            return Err("話者補完の表示進捗が不正です".into());
        }
        let mut job = self.get_job(id)?;
        if job.kind != "speaker_completion" || job.state != "running" {
            return Ok(job);
        }
        let mut map = self
            .execution_progress
            .lock()
            .map_err(|_| "進捗ロックを取得できません")?;
        if map
            .get(id)
            .is_some_and(|old| old.total != value.total || old.completed > value.completed)
        {
            return Err("話者補完の表示進捗が逆戻りしました".into());
        }
        map.insert(id.into(), value.clone());
        job.execution_progress = Some(value);
        Ok(job)
    }
    pub(crate) fn clear_completion_progress(&self, id: &str) {
        if let Ok(mut map) = self.execution_progress.lock() {
            map.remove(id);
        }
    }

    pub(crate) fn get_job(&self, job_id: &str) -> Result<JobSnapshot, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut job = load_job(&connection, job_id)?.ok_or("処理が見つかりません")?;
        self.decorate_execution_progress(&mut job);
        Ok(job)
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
        let mut jobs = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for job in &mut jobs {
            self.decorate_execution_progress(job);
        }
        Ok(jobs)
    }

    /// Read-only notification census, independent of the selected meeting and
    /// the UI's 100-row history. Keep watched work after it becomes terminal,
    /// even when newer work in the same meeting has already been queued.
    pub(crate) fn notification_jobs(
        &self,
        watched: &[String],
    ) -> Result<Vec<NotificationJob>, String> {
        if watched.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err("解析通知の追跡対象が不正です".into());
        }
        let watched = serde_json::to_string(watched).map_err(|error| error.to_string())?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = connection.prepare(
            "WITH ranked AS (
                SELECT id, ROW_NUMBER() OVER (PARTITION BY meeting_id, kind ORDER BY created_at DESC, rowid DESC) AS position
                FROM processing_jobs WHERE kind IN ('transcription', 'diarization', 'minutes_generation', 'voice_registration', 'voice_matching', 'speaker_completion')
            )
            SELECT j.id, j.meeting_id, j.kind, j.state, j.operation_id, j.input_hash, j.spec_hash,
                j.completed_units, j.total_units, j.error_code, j.created_at, j.updated_at,
                j.output_analysis_id, j.pause_reason, j.retry_of_job_id, j.attempt, j.output_minutes_id, m.title
            FROM processing_jobs j JOIN meetings m ON m.id = j.meeting_id
            WHERE j.kind IN ('transcription', 'diarization', 'minutes_generation', 'voice_registration', 'voice_matching', 'speaker_completion') AND (
                j.state IN ('queued', 'running', 'pausing', 'paused', 'interrupted')
                OR j.id IN (SELECT id FROM ranked WHERE position = 1)
                OR j.id IN (SELECT value FROM json_each(?1))
            ) ORDER BY j.created_at DESC, j.rowid DESC"
        ).map_err(|error| error.to_string())?;
        let rows = query
            .query_map(params![watched], |row| {
                Ok(NotificationJob {
                    job: job_from_row(row)?,
                    meeting_title: row.get(17)?,
                })
            })
            .map_err(|error| error.to_string())?;
        let mut jobs = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for job in &mut jobs {
            let parameters: String = connection
                .query_row(
                    "SELECT parameters_json FROM processing_jobs WHERE id=?1",
                    [&job.job.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            crate::current_formats::job(
                &job.job.kind,
                &serde_json::from_str(&parameters).map_err(|e| e.to_string())?,
            )?;
            self.decorate_execution_progress(&mut job.job);
        }
        Ok(jobs)
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
        if crate::generation::markdown_job::is_markdown(verified_spec)
            && !super::minutes_markdown::uuid(operation_id)
        {
            return Err("議事録Markdown再試行の操作IDが不正です".into());
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
        require_audio_source(&transaction, spec.kind.as_str(), &spec.meeting_id)?;
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
        crate::internal_logs::job(&next);
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
        crate::internal_logs::job(&snapshot);
        Ok(snapshot)
    }
}

fn job_from_row(row: &Row<'_>) -> rusqlite::Result<JobSnapshot> {
    Ok(JobSnapshot {
        execution_progress: None,
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

fn require_audio_source(db: &Connection, kind: &str, meeting: &str) -> Result<(), String> {
    #[cfg(unix)]
    if matches!(
        kind,
        "transcription"
            | "diarization"
            | "voice_registration"
            | "voice_matching"
            | "speaker_completion"
    ) {
        super::deletion::audio::require_source(db, meeting)?;
    }
    #[cfg(not(unix))]
    let _ = (db, kind, meeting);
    Ok(())
}

pub(super) fn enqueue_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    spec: &JobSpec,
) -> Result<JobSnapshot, String> {
    crate::current_formats::job(spec.kind.as_str(), &spec.parameters)?;
    require_audio_source(transaction, spec.kind.as_str(), &spec.meeting_id)?;
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
        JobKind::MinutesGeneration
            | JobKind::Diarization
            | JobKind::VoiceRegistration
            | JobKind::VoiceMatching
            | JobKind::SpeakerCompletion
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
    crate::current_formats::job(spec.kind.as_str(), &spec.parameters)?;
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

pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
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

    #[test]
    fn retired_transcript_cleanup_kind_is_not_a_current_job() {
        assert!(serde_json::from_str::<JobKind>("\"transcript_cleanup\"").is_err());
        for (wire, kind) in [
            ("transcription", JobKind::Transcription),
            ("diarization", JobKind::Diarization),
            ("minutes_generation", JobKind::MinutesGeneration),
        ] {
            assert_eq!(
                serde_json::from_value::<JobKind>(json!(wire)).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn queued_retired_job_does_not_preempt_current_work() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("現行ジョブ").unwrap();
        let current = storage
            .enqueue_job(audio_job(&meeting.id, "current-job"))
            .unwrap();
        let retired = uuid::Uuid::new_v4().to_string();
        storage.connection.lock().unwrap().execute(
            "INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,completed_units,total_units,attempt,created_at,updated_at) SELECT ?1,meeting_id,'transcript_cleanup','queued','retired-job',input_hash,profile_snapshot_json,parameters_json,spec_hash,0,total_units,1,created_at-1,updated_at FROM processing_jobs WHERE id=?2",
            params![retired, current.id],
        ).unwrap();
        assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, current.id);
        assert_eq!(storage.get_job(&retired).unwrap().state, "queued");
    }

    pub(super) fn audio_job(meeting_id: &str, operation_id: &str) -> JobSpec {
        JobSpec {
            meeting_id: meeting_id.into(),
            kind: JobKind::Transcription,
            operation_id: operation_id.into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: json!({"engine": "test", "model_hash": "b".repeat(64)}),
            parameters: json!({"schemaVersion":4,"window_seconds": 30}),
            total_units: Some(10),
        }
    }

    #[test]
    fn notification_jobs_cross_meetings_and_keep_watched_terminal_work_beyond_history() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("通知対象").unwrap();
        let other = storage.create_meeting("別会議").unwrap();
        let active = storage
            .enqueue_job(audio_job(&meeting.id, "active"))
            .unwrap();
        let watched = storage
            .enqueue_job(audio_job(&meeting.id, "old-watched"))
            .unwrap();
        storage
            .finish_job(&watched.id, 0, "cancelled", None)
            .unwrap();
        for index in 0..110 {
            let job = storage
                .enqueue_job(audio_job(&meeting.id, &format!("history-{index}")))
                .unwrap();
            storage.finish_job(&job.id, 0, "cancelled", None).unwrap();
        }
        let newest = storage.list_jobs(&meeting.id).unwrap()[0].clone();
        let unrelated = storage
            .enqueue_job(audio_job(&other.id, "other-active"))
            .unwrap();
        assert_eq!(storage.list_jobs(&meeting.id).unwrap().len(), 100);
        let before = storage.get_job(&active.id).unwrap();
        let rows = storage
            .notification_jobs(&[watched.id.clone(), watched.id.clone()])
            .unwrap();
        assert_eq!(rows.len(), 4);
        for id in [&active.id, &watched.id, &newest.id, &unrelated.id] {
            assert_eq!(rows.iter().filter(|row| &row.job.id == id).count(), 1);
        }
        assert!(rows.iter().all(|row| row.meeting_title
            == if row.job.meeting_id == meeting.id {
                "通知対象"
            } else {
                "別会議"
            }));
        assert_eq!(
            storage.get_job(&active.id).unwrap(),
            before,
            "notification reads must not advance work"
        );
        storage
            .finish_job(&active.id, 0, "cancelled", None)
            .unwrap();
        assert!(storage
            .notification_jobs(&[active.id.clone()])
            .unwrap()
            .iter()
            .any(|row| row.job.id == active.id && row.job.state == "cancelled"));
        assert!(!storage
            .notification_jobs(&[])
            .unwrap()
            .iter()
            .any(|row| row.job.id == active.id));
    }

    #[test]
    fn notification_jobs_reject_obsolete_formats() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("現行形式").unwrap();
        for version in [0, 1, 2, 3, 5, 99] {
            let mut spec = audio_job(&meeting.id, &format!("unsupported-{version}"));
            spec.parameters["schemaVersion"] = json!(version);
            assert!(storage.enqueue_job(spec).unwrap_err().contains("非対応"));
        }
        let job = storage
            .enqueue_job(audio_job(&meeting.id, "current"))
            .unwrap();
        assert_eq!(storage.notification_jobs(&[]).unwrap()[0].job.id, job.id);
    }

    #[test]
    fn notification_jobs_keep_latest_per_kind_without_invalid_or_missing_watch_targets() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("三工程").unwrap();
        let transcription = storage
            .enqueue_job(audio_job(&meeting.id, "transcription"))
            .unwrap();
        storage.connection.lock().unwrap().execute(
            "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES ('notification-input', ?1, 1, 'engine', 'candidate', ?2, '{}', '{}', ?3, 1, 0)",
            params![meeting.id, "a".repeat(64), "0".repeat(64)],
        ).unwrap();
        let committed = storage
            .commit_analysis_candidate(&meeting.id, "notification-input", 0, false)
            .unwrap();
        for kind in [JobKind::Diarization, JobKind::MinutesGeneration] {
            let mut spec = audio_job(&meeting.id, kind.as_str());
            spec.kind = kind;
            if spec.kind == JobKind::MinutesGeneration {
                spec.parameters = json!({"generation":{"schemaVersion":9}});
            } else {
                spec.parameters["schemaVersion"] = json!(3);
            }
            spec.input_analysis_id = Some("notification-input".into());
            spec.input_hash = committed.content_hash.clone();
            let job = storage.enqueue_job(spec).unwrap();
            storage.finish_job(&job.id, 0, "cancelled", None).unwrap();
        }
        storage
            .finish_job(&transcription.id, 0, "cancelled", None)
            .unwrap();
        let rows = storage
            .notification_jobs(&[uuid::Uuid::new_v4().to_string()])
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(storage.notification_jobs(&["not-a-job-id".into()]).is_err());
        assert_eq!(
            rows.iter()
                .map(|row| &row.job.kind)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn notification_output_checks_exact_hash_and_ownership_beyond_visible_candidate_history() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = super::super::analysis::tests::saved_meeting(&storage);
        let mut spec = audio_job(&meeting.id, "notification-output");
        spec.total_units = Some(1);
        let mut input = super::super::analysis::tests::input();
        input.profile_snapshot = spec.profile_snapshot.clone();
        storage.connection.lock().unwrap().execute("INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES ('manifest',?1,'meeting','meetings/test/manifest.json','application/json',16,'sha256',?2,'ready',0,0)",params![meeting.id,"a".repeat(64)]).unwrap();
        let (source, known) = storage.recorded_analysis_input(&meeting.id).unwrap();
        let known = known.unwrap();
        spec.parameters["source"] = serde_json::to_value(&source).unwrap();
        spec.parameters["knownInput"] = serde_json::to_value(&known).unwrap();
        input.config["audioCoverage"] = json!({"mode":"known_runs","runCount":known.runs.len(),"missingRangeCount":known.gaps.iter().filter(|g|!g.recovered).count(),"timeBase":"meeting"});

        let job = storage.enqueue_job(spec).unwrap();
        assert!(storage.notification_output(&job.id).is_err());
        storage.acquire_next_job().unwrap().unwrap();
        storage
            .save_job_checkpoint(&job.id, 0, 1, json!({"outputs":[]}))
            .unwrap();
        let id = storage
            .finish_transcription_candidate(&meeting.id, &job.id, 1, input.clone())
            .unwrap();
        for _ in 0..25 {
            storage
                .create_analysis_candidate(&meeting.id, input.clone())
                .unwrap();
        }
        assert!(!storage
            .analysis_versions(&meeting.id)
            .unwrap()
            .versions
            .iter()
            .any(|v| v.id == id));
        let proof = storage.notification_output(&job.id).unwrap();
        assert_eq!(proof.job.id, job.id);
        assert_eq!(proof.job.output_analysis_id.as_deref(), Some(id.as_str()));
        assert_eq!(proof.publication_state, "candidate");
        assert!(valid_hash(&proof.content_hash));
        let before = storage.get_meeting(&meeting.id).unwrap();
        assert_eq!(
            storage.notification_output(&job.id).unwrap().content_hash,
            proof.content_hash
        );
        assert_eq!(
            storage.get_meeting(&meeting.id).unwrap(),
            before,
            "proof is read-only"
        );
        assert!(storage.notification_output("not-a-job-id").is_err());
        assert!(storage
            .notification_output(&uuid::Uuid::new_v4().to_string())
            .is_err());
        // Candidate corruption must withhold success without relaxing the
        // separate immutable-content guard on committed revisions.
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE utterances SET effective_text='改変' WHERE analysis_revision_id=?1",
                params![id],
            )
            .unwrap();
        }
        assert!(storage.notification_output(&job.id).is_err());
        storage.connection.lock().unwrap().execute("UPDATE utterances SET effective_text=recognized_text WHERE analysis_revision_id=?1", params![id]).unwrap();
        assert_eq!(
            storage.notification_output(&job.id).unwrap().content_hash,
            proof.content_hash
        );
        let adopted = storage
            .commit_analysis_candidate(&meeting.id, &id, 0, false)
            .unwrap();
        assert_eq!(
            storage
                .notification_output(&job.id)
                .unwrap()
                .publication_state,
            "committed"
        );
        assert_eq!(proof.content_hash, adopted.content_hash);
        assert!(storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE utterances SET effective_text='改変' WHERE analysis_revision_id=?1",
                params![id],
            )
            .is_err());
        assert_eq!(
            storage.notification_output(&job.id).unwrap().content_hash,
            proof.content_hash
        );
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
        spec.parameters = json!({"generation":{"schemaVersion":9}});
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

impl Storage {
    pub(crate) fn optimization_version_for_operation(
        &self,
        meeting: &str,
        operation: &str,
    ) -> Result<Option<u32>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let value: Option<(String, String)> = db
            .query_row(
                "SELECT meeting_id,parameters_json FROM processing_jobs WHERE operation_id=?1",
                [operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match value {
            None => Ok(Some(1)),
            Some((owner, json)) => {
                if owner != meeting {
                    return Err("操作IDが別の会議に属しています".into());
                }
                let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                let version =
                    value.get("diarization").unwrap_or(&value)["optimizationVersion"].as_u64();
                if version != Some(u64::from(crate::current_formats::OPTIMIZATION)) {
                    return Err("非対応の処理最適化情報です".into());
                }
                Ok(Some(crate::current_formats::OPTIMIZATION))
            }
        }
    }
}
