use super::*;
use rusqlite::{Row, TransactionBehavior};
use serde::Deserialize;
use std::fs::File;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureManifest {
    schema_version: u32,
    capture_mode: String,
    backend: String,
    #[serde(rename = "targetPIDs")]
    target_pids: Vec<i32>,
    state: String,
    errors: Vec<String>,
    microphone: CaptureTrack,
    system: CaptureTrack,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureTrack {
    frames: i64,
    sample_rate: Option<f64>,
    channels: Option<i64>,
    append_failures: i64,
}

impl CaptureTrack {
    fn validated_format(&self) -> Result<(i64, i64, i64), String> {
        let rate = self.sample_rate.ok_or("録音のサンプルレートがありません")?;
        let channels = self.channels.ok_or("録音のチャンネル数がありません")?;
        if self.frames <= 0
            || self.append_failures != 0
            || !rate.is_finite()
            || !(8_000.0..=384_000.0).contains(&rate)
            || rate.fract() != 0.0
            || !(1..=8).contains(&channels)
        {
            return Err("録音トラックの形式または保存状態が不正です".into());
        }
        let rate = rate as i64;
        let duration = i128::from(self.frames) * 1_000_000 / i128::from(rate);
        if duration <= 0 || duration > 14_401_000_000 {
            return Err("録音時間が不正です".into());
        }
        Ok((rate, channels, duration as i64))
    }
}

struct CapturedFile {
    relative: PathBuf,
    size_bytes: i64,
    hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub mode: String,
    pub backend: String,
    pub target_pids: Vec<i32>,
}

impl RecordingConfig {
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.mode.as_str(), "system" | "application")
            || !matches!(self.backend.as_str(), "screencapturekit" | "coreaudio")
        {
            return Err("録音方式が不正です".into());
        }
        if (self.mode == "system" && !self.target_pids.is_empty())
            || (self.mode == "application" && self.target_pids.is_empty())
            || (self.backend == "coreaudio" && self.mode != "application")
            || (self.backend == "screencapturekit" && self.target_pids.len() > 1)
            || self.target_pids.iter().any(|pid| *pid <= 0)
        {
            return Err("録音対象のPID指定が不正です".into());
        }
        let mut sorted = self.target_pids.clone();
        sorted.sort_unstable();
        if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("録音対象のPIDが重複しています".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSession {
    pub id: String,
    pub meeting_id: String,
    pub state: String,
    pub start_operation_id: String,
    pub stop_operation_id: Option<String>,
    pub started_at_utc_ms: Option<i64>,
    pub ended_at_utc_ms: Option<i64>,
    pub duration_us: i64,
}

impl Storage {
    pub(crate) fn recover_completed_recordings(&self) -> Result<Vec<String>, String> {
        let interrupted = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません".to_string())?;
            let mut statement = connection
                .prepare("SELECT id FROM recording_sessions WHERE state = 'interrupted' ORDER BY created_at, id")
                .map_err(|error| error.to_string())?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            ids
        };
        let mut recovered = Vec::new();
        for id in interrupted {
            // Incomplete or tampered recordings stay interrupted; no files are removed.
            if self.finalize_recording(&id).is_ok() {
                recovered.push(id);
            }
        }
        Ok(recovered)
    }

    #[allow(dead_code)]
    pub fn prepare_recording(
        &self,
        meeting_id: &str,
        config: &RecordingConfig,
        start_operation_id: &str,
    ) -> Result<RecordingSession, String> {
        config.validate()?;
        if start_operation_id.trim().is_empty() || start_operation_id.len() > 128 {
            return Err("録音開始の操作IDが不正です".into());
        }
        let config_json = serde_json::to_string(config).map_err(|error| error.to_string())?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let existing: Option<(RecordingSession, String)> = transaction.query_row(
            "SELECT id, meeting_id, state, start_operation_id, stop_operation_id, started_at_utc_ms, ended_at_utc_ms, duration_us, capture_config_json FROM recording_sessions WHERE start_operation_id = ?1",
            params![start_operation_id],
            |row| Ok((session_from_row(row)?, row.get(8)?)),
        ).optional().map_err(|error| error.to_string())?;
        if let Some((session, saved_config)) = existing {
            if session.meeting_id != meeting_id || saved_config != config_json {
                return Err("同じ操作IDで異なる録音条件が指定されました".into());
            }
            return Ok(session);
        }
        let meeting: Option<(String, Option<String>)> = transaction
            .query_row(
                "SELECT lifecycle, source_type FROM meetings WHERE id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (lifecycle, source_type) = meeting.ok_or("会議が見つかりません")?;
        if lifecycle != "draft" || source_type.is_some() {
            return Err("この会議では新規録音を開始できません".into());
        }
        let busy: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running', 'pausing'))",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if busy {
            return Err("解析処理の停止が完了していません".into());
        }
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        transaction.execute(
            "UPDATE meetings SET source_type = 'recording', lifecycle = 'recording', lock_version = lock_version + 1, updated_at = ?1 WHERE id = ?2 AND lifecycle = 'draft' AND source_type IS NULL",
            params![now, meeting_id],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES (?1, ?2, 'preparing', ?3, ?4, ?5, ?6, ?6)",
            params![id, meeting_id, config.mode, config_json, start_operation_id, now],
        ).map_err(|error| error.to_string())?;
        let session = load_session(&transaction, &id)?.ok_or("録音セッションを読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(session)
    }

    #[allow(dead_code)]
    pub fn mark_recording_started(&self, session_id: &str) -> Result<RecordingSession, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let now = now_ms()?;
        let changed = connection.execute(
            "UPDATE recording_sessions SET state = 'recording', started_at_utc_ms = ?1, updated_at = ?1 WHERE id = ?2 AND state = 'preparing'",
            params![now, session_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("録音開始状態を更新できません".into());
        }
        load_session(&connection, session_id)?.ok_or("録音セッションが見つかりません".into())
    }

    #[allow(dead_code)]
    pub fn request_recording_stop(
        &self,
        session_id: &str,
        stop_operation_id: &str,
    ) -> Result<RecordingSession, String> {
        if stop_operation_id.trim().is_empty() || stop_operation_id.len() > 128 {
            return Err("録音停止の操作IDが不正です".into());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let existing =
            load_session(&connection, session_id)?.ok_or("録音セッションが見つかりません")?;
        if existing.state == "stopping"
            && existing.stop_operation_id.as_deref() == Some(stop_operation_id)
        {
            return Ok(existing);
        }
        let changed = connection.execute(
            "UPDATE recording_sessions SET state = 'stopping', stop_operation_id = ?1, updated_at = ?2 WHERE id = ?3 AND state IN ('preparing', 'recording') AND stop_operation_id IS NULL",
            params![stop_operation_id, now_ms()?, session_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("録音停止状態を更新できません".into());
        }
        load_session(&connection, session_id)?.ok_or("録音セッションが見つかりません".into())
    }

    #[allow(dead_code)]
    pub fn fail_recording(&self, session_id: &str) -> Result<RecordingSession, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let session =
            load_session(&transaction, session_id)?.ok_or("録音セッションが見つかりません")?;
        if !matches!(
            session.state.as_str(),
            "preparing" | "recording" | "stopping"
        ) {
            return Err("録音失敗状態へ変更できません".into());
        }
        let now = now_ms()?;
        transaction.execute(
            "UPDATE recording_sessions SET state = 'failed', ended_at_utc_ms = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "UPDATE meetings SET lifecycle = 'draft', lock_version = lock_version + 1, updated_at = ?1 WHERE id = ?2 AND lifecycle = 'recording'",
            params![now, session.meeting_id],
        ).map_err(|error| error.to_string())?;
        let failed =
            load_session(&transaction, session_id)?.ok_or("録音セッションを読み込めません")?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(failed)
    }

    pub fn interrupt_recording(&self, session_id: &str) -> Result<RecordingSession, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let changed = connection
            .execute(
                "UPDATE recording_sessions SET state = 'interrupted', updated_at = ?1 WHERE id = ?2 AND state IN ('preparing', 'recording', 'stopping')",
                params![now_ms()?, session_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("録音を中断状態へ変更できません".into());
        }
        load_session(&connection, session_id)?.ok_or("録音セッションが見つかりません".into())
    }

    #[allow(dead_code)]
    pub fn finalize_recording(&self, session_id: &str) -> Result<RecordingSession, String> {
        let session = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません".to_string())?;
            load_session(&connection, session_id)?.ok_or("録音セッションが見つかりません")?
        };
        if !matches!(
            session.state.as_str(),
            "recording" | "stopping" | "interrupted"
        ) {
            return Err("録音が確定可能な状態ではありません".into());
        }
        let base = PathBuf::from("meetings")
            .join(&session.meeting_id)
            .join("audio")
            .join(session_id);
        let manifest_file = self.checked_capture_file(&base.join("manifest.json"), false)?;
        if manifest_file.size_bytes > 256 * 1024 {
            return Err("録音manifestが大きすぎます".into());
        }
        let manifest_bytes = std::fs::read(self.managed_path(&manifest_file.relative)?)
            .map_err(|error| error.to_string())?;
        let manifest: CaptureManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
        if manifest.schema_version != 1 || manifest.state != "saved" || !manifest.errors.is_empty()
        {
            return Err("録音manifestは保存完了を示していません".into());
        }
        let (stored_mode, stored_config): (String, String) = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .query_row(
                "SELECT capture_mode, capture_config_json FROM recording_sessions WHERE id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let config: RecordingConfig =
            serde_json::from_str(&stored_config).map_err(|error| error.to_string())?;
        if manifest.capture_mode != stored_mode
            || manifest.capture_mode != config.mode
            || manifest.backend != config.backend
            || manifest.target_pids != config.target_pids
        {
            return Err("録音manifestと開始条件が一致しません".into());
        }
        let mic_format = manifest.microphone.validated_format()?;
        let system_format = manifest.system.validated_format()?;
        let mic_file = self.checked_capture_file(&base.join("microphone.caf"), true)?;
        let system_file = self.checked_capture_file(&base.join("system.caf"), true)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let current =
            load_session(&transaction, session_id)?.ok_or("録音セッションが見つかりません")?;
        if current != session {
            return Err("録音状態が検証中に変わりました".into());
        }
        let now = now_ms()?;
        insert_capture_asset(
            &transaction,
            &session.meeting_id,
            &manifest_file,
            "application/json",
            now,
        )?;
        for (kind, file, format) in [
            ("mic", &mic_file, mic_format),
            ("system", &system_file, system_format),
        ] {
            let asset_id =
                insert_capture_asset(&transaction, &session.meeting_id, file, "audio/x-caf", now)?;
            let track_id = Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, processing_config_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'caf', ?5, ?6, 'ready', '{\"timeline_alignment\":\"unverified\"}', ?7, ?7)",
                params![track_id, session.meeting_id, session_id, kind, format.0, format.1, now],
            ).map_err(|error| error.to_string())?;
            transaction.execute(
                "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?6, ?7, ?8, 'ready', ?9)",
                params![Uuid::new_v4().to_string(), session.meeting_id, track_id, asset_id, format.2, if kind == "mic" { manifest.microphone.frames } else { manifest.system.frames }, format.0, format.1, now],
            ).map_err(|error| error.to_string())?;
        }
        let duration_us = mic_format.2.max(system_format.2);
        transaction.execute(
            "UPDATE recording_sessions SET state = 'saved', duration_us = ?1, audio_manifest_hash = ?2, ended_at_utc_ms = ?3, last_committed_us = ?1, updated_at = ?3 WHERE id = ?4 AND state IN ('recording', 'stopping', 'interrupted')",
            params![duration_us, manifest_file.hash, now, session_id],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "UPDATE meetings SET lifecycle = 'recorded', duration_us = ?1, started_at_utc_ms = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lifecycle = 'recording'",
            params![duration_us, session.started_at_utc_ms, now, session.meeting_id],
        ).map_err(|error| error.to_string())?;
        let saved =
            load_session(&transaction, session_id)?.ok_or("録音セッションを読み込めません")?;
        super::analysis_flows::ensure_flow(&transaction, &session.meeting_id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(saved)
    }

    fn checked_capture_file(&self, relative: &Path, caf: bool) -> Result<CapturedFile, String> {
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if actual != root.join(relative) || !actual.is_file() {
            return Err("録音ファイルが管理領域外または通常ファイルではありません".into());
        }
        if caf {
            let mut header = [0_u8; 4];
            File::open(&actual)
                .map_err(|error| error.to_string())?
                .read_exact(&mut header)
                .map_err(|error| error.to_string())?;
            if &header != b"caff" {
                return Err("録音ファイルはCAF形式ではありません".into());
            }
        }
        let (size_bytes, hash) = hash_file(&actual)?;
        if size_bytes < if caf { 16 } else { 2 } {
            return Err("録音ファイルが短すぎます".into());
        }
        File::open(&actual)
            .map_err(|error| error.to_string())?
            .sync_all()
            .map_err(|error| error.to_string())?;
        Ok(CapturedFile {
            relative: relative.to_owned(),
            size_bytes,
            hash,
        })
    }
}

fn insert_capture_asset(
    connection: &Connection,
    meeting_id: &str,
    file: &CapturedFile,
    media_type: &str,
    now: i64,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let path = file
        .relative
        .to_str()
        .ok_or("録音ファイルのパスが不正です")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    connection.execute(
        "INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES (?1, ?2, 'meeting', ?3, ?4, ?5, 'sha256', ?6, 'ready', ?7, ?7)",
        params![id, meeting_id, path, media_type, file.size_bytes, file.hash, now],
    ).map_err(|error| error.to_string())?;
    Ok(id)
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<RecordingSession> {
    Ok(RecordingSession {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        state: row.get(2)?,
        start_operation_id: row.get(3)?,
        stop_operation_id: row.get(4)?,
        started_at_utc_ms: row.get(5)?,
        ended_at_utc_ms: row.get(6)?,
        duration_us: row.get(7)?,
    })
}

fn load_session(connection: &Connection, id: &str) -> Result<Option<RecordingSession>, String> {
    connection.query_row(
        "SELECT id, meeting_id, state, start_operation_id, stop_operation_id, started_at_utc_ms, ended_at_utc_ms, duration_us FROM recording_sessions WHERE id = ?1",
        params![id], session_from_row,
    ).optional().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RecordingConfig {
        RecordingConfig {
            mode: "system".into(),
            backend: "screencapturekit".into(),
            target_pids: vec![],
        }
    }

    #[test]
    fn start_stop_operations_are_idempotent_and_only_one_recording_is_active() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let first = storage.create_meeting("録音1").unwrap();
        let second = storage.create_meeting("録音2").unwrap();
        let session = storage
            .prepare_recording(&first.id, &config(), "start-1")
            .unwrap();
        assert_eq!(session.state, "preparing");
        assert_eq!(
            storage
                .prepare_recording(&first.id, &config(), "start-1")
                .unwrap(),
            session
        );
        assert!(storage
            .prepare_recording(&second.id, &config(), "start-2")
            .is_err());
        assert_eq!(
            storage.mark_recording_started(&session.id).unwrap().state,
            "recording"
        );
        let stopping = storage
            .request_recording_stop(&session.id, "stop-1")
            .unwrap();
        assert_eq!(stopping.state, "stopping");
        assert_eq!(
            storage
                .request_recording_stop(&session.id, "stop-1")
                .unwrap(),
            stopping
        );
        assert!(storage
            .request_recording_stop(&session.id, "stop-2")
            .is_err());
        assert_eq!(storage.fail_recording(&session.id).unwrap().state, "failed");
        assert_eq!(
            storage
                .prepare_recording(&second.id, &config(), "start-2")
                .unwrap()
                .state,
            "preparing"
        );
        assert!(storage
            .prepare_recording(&first.id, &config(), "start-3")
            .is_err());
    }

    #[test]
    fn stop_during_preparation_is_recorded_without_claiming_audio_started() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("準備中").unwrap();
        let session = storage
            .prepare_recording(&meeting.id, &config(), "start")
            .unwrap();
        let stopped = storage.request_recording_stop(&session.id, "stop").unwrap();
        assert_eq!(stopped.state, "stopping");
        assert!(stopped.started_at_utc_ms.is_none());
        assert!(storage.mark_recording_started(&session.id).is_err());
        assert_eq!(storage.fail_recording(&session.id).unwrap().state, "failed");
    }

    #[test]
    fn active_recording_is_interrupted_after_reopen_and_invalid_config_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("録音").unwrap();
        let mut invalid = config();
        invalid.target_pids.push(123);
        assert!(storage
            .prepare_recording(&meeting.id, &invalid, "invalid")
            .is_err());
        let session = storage
            .prepare_recording(&meeting.id, &config(), "start")
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        let interrupted = load_session(&reopened.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(interrupted.state, "interrupted");
        let other = reopened.create_meeting("別会議").unwrap();
        assert_eq!(
            reopened
                .prepare_recording(&other.id, &config(), "other")
                .unwrap()
                .state,
            "preparing"
        );
    }

    #[test]
    fn restart_commits_only_complete_saved_probe_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("復旧する録音").unwrap();
        let session = storage
            .prepare_recording(&meeting.id, &config(), "recover-start")
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let output = temp
            .path()
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&output).unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1, "captureMode": "system", "backend": "screencapturekit",
            "targetPIDs": [], "state": "saved", "errors": [],
            "microphone": {"frames": 48000, "sampleRate": 48000, "channels": 1, "appendFailures": 0},
            "system": {"frames": 48000, "sampleRate": 48000, "channels": 2, "appendFailures": 0},
        });
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(output.join("microphone.caf"), b"caff000000000000").unwrap();
        std::fs::write(output.join("system.caf"), b"caff000000000000").unwrap();
        drop(storage);

        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(
            reopened.recover_completed_recordings().unwrap(),
            vec![session.id.clone()]
        );
        assert_eq!(
            load_session(&reopened.connection.lock().unwrap(), &session.id)
                .unwrap()
                .unwrap()
                .state,
            "saved"
        );
        assert_eq!(
            reopened
                .get_meeting(&meeting.id)
                .unwrap()
                .unwrap()
                .lifecycle,
            "recorded"
        );
        assert!(reopened.recover_completed_recordings().unwrap().is_empty());

        let incomplete = reopened.create_meeting("未完成の録音").unwrap();
        let pending = reopened
            .prepare_recording(&incomplete.id, &config(), "pending-start")
            .unwrap();
        reopened.mark_recording_started(&pending.id).unwrap();
        let pending_output = temp
            .path()
            .join("meetings")
            .join(&incomplete.id)
            .join("audio")
            .join(&pending.id);
        std::fs::create_dir_all(&pending_output).unwrap();
        std::fs::write(
            pending_output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(pending_output.join("microphone.caf"), b"caff000000000000").unwrap();
        std::fs::write(pending_output.join("system.caf"), b"bad!000000000000").unwrap();
        drop(reopened);

        let final_open = Storage::open(temp.path().to_owned()).unwrap();
        assert!(final_open
            .recover_completed_recordings()
            .unwrap()
            .is_empty());
        assert_eq!(
            load_session(&final_open.connection.lock().unwrap(), &pending.id)
                .unwrap()
                .unwrap()
                .state,
            "interrupted"
        );
        assert_eq!(
            final_open
                .get_meeting(&incomplete.id)
                .unwrap()
                .unwrap()
                .lifecycle,
            "recording"
        );
        assert!(pending_output.join("system.caf").exists());
    }

    #[test]
    fn failed_registration_keeps_files_retryable() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("確定再試行").unwrap();
        let session = storage
            .prepare_recording(&meeting.id, &config(), "retry-start")
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let output = temp
            .path()
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("microphone.caf"), b"caff000000000000").unwrap();
        std::fs::write(output.join("system.caf"), b"bad!000000000000").unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1, "captureMode": "system", "backend": "screencapturekit",
            "targetPIDs": [], "state": "saved", "errors": [],
            "microphone": {"frames": 48000, "sampleRate": 48000, "channels": 1, "appendFailures": 0},
            "system": {"frames": 48000, "sampleRate": 48000, "channels": 2, "appendFailures": 0},
        });
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(storage.finalize_recording(&session.id).is_err());
        assert_eq!(
            storage.interrupt_recording(&session.id).unwrap().state,
            "interrupted"
        );
        assert!(storage.recover_completed_recordings().unwrap().is_empty());
        assert_eq!(
            load_session(&storage.connection.lock().unwrap(), &session.id)
                .unwrap()
                .unwrap()
                .state,
            "interrupted"
        );
        std::fs::write(output.join("system.caf"), b"caff000000000000").unwrap();
        assert_eq!(
            storage.recover_completed_recordings().unwrap(),
            vec![session.id]
        );
    }

    #[test]
    fn running_job_must_yield_before_recording_prepares() {
        use crate::storage::jobs::{JobKind, JobSpec};
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("録音").unwrap();
        storage
            .enqueue_job(JobSpec {
                meeting_id: meeting.id.clone(),
                kind: JobKind::Transcription,
                operation_id: "job".into(),
                input_analysis_id: None,
                input_hash: "a".repeat(64),
                profile_snapshot: serde_json::json!({}),
                parameters: serde_json::json!({}),
                total_units: None,
            })
            .unwrap();
        let job = storage.acquire_next_job().unwrap().unwrap();
        assert!(storage
            .prepare_recording(&meeting.id, &config(), "start")
            .is_err());
        storage.request_pause_for_recording().unwrap();
        assert!(storage
            .prepare_recording(&meeting.id, &config(), "start")
            .is_err());
        storage
            .save_job_checkpoint(&job.id, 0, 0, serde_json::json!({}))
            .unwrap();
        assert_eq!(
            storage
                .prepare_recording(&meeting.id, &config(), "start")
                .unwrap()
                .state,
            "preparing"
        );
    }

    #[test]
    fn saved_probe_outputs_are_registered_together_and_manifest_mismatch_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("録音").unwrap();
        let session = storage
            .prepare_recording(&meeting.id, &config(), "start")
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let output = temp
            .path()
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&output).unwrap();
        // This fixture tests DB atomicity and manifest checks, not CAF decoding.
        std::fs::write(output.join("microphone.caf"), b"caff000000000000").unwrap();
        std::fs::write(output.join("system.caf"), b"bad!000000000000").unwrap();
        let manifest = serde_json::json!({
            "schemaVersion": 1, "captureMode": "application", "backend": "screencapturekit",
            "targetPIDs": [], "state": "saved", "errors": [],
            "microphone": {"frames": 48000, "sampleRate": 48000, "channels": 1, "appendFailures": 0},
            "system": {"frames": 48000, "sampleRate": 48000, "channels": 2, "appendFailures": 0},
        });
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(storage.finalize_recording(&session.id).is_err());
        assert_eq!(
            load_session(&storage.connection.lock().unwrap(), &session.id)
                .unwrap()
                .unwrap()
                .state,
            "recording"
        );
        let mut corrected = manifest;
        corrected["captureMode"] = serde_json::json!("system");
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&corrected).unwrap(),
        )
        .unwrap();
        assert!(storage.finalize_recording(&session.id).is_err());
        let assets_before: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_assets WHERE meeting_id = ?1",
                params![meeting.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(assets_before, 0);
        std::fs::write(output.join("system.caf"), b"caff000000000000").unwrap();
        let saved = storage.finalize_recording(&session.id).unwrap();
        assert_eq!(saved.state, "saved");
        assert_eq!(saved.duration_us, 1_000_000);
        assert_eq!(
            storage.analysis_flow(&meeting.id).unwrap().unwrap().stage,
            "pending"
        );
        let db = storage.connection.lock().unwrap();
        let assets: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM file_assets WHERE meeting_id = ?1",
                params![meeting.id],
                |row| row.get(0),
            )
            .unwrap();
        let tracks: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audio_tracks WHERE meeting_id = ?1",
                params![meeting.id],
                |row| row.get(0),
            )
            .unwrap();
        let chunks: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audio_chunks WHERE meeting_id = ?1",
                params![meeting.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((assets, tracks, chunks), (3, 2, 2));
        drop(db);
        assert_eq!(
            storage.get_meeting(&meeting.id).unwrap().unwrap().lifecycle,
            "recorded"
        );
        assert!(storage.finalize_recording(&session.id).is_err());
    }
}
