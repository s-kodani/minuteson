use super::*;
use rusqlite::{Row, TransactionBehavior};
use serde::Deserialize;
use std::fs::File;

mod clock;
#[cfg(unix)]
mod flac;
#[cfg(unix)]
pub(crate) mod partial;
use clock::{CaptureClock, CaptureEnd};
#[cfg(all(test, unix))]
pub(crate) use flac::tests::{
    partial_fixture as partial_test_fixture, Fixture as PartialTestFixture,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureManifest {
    schema_version: u32,
    capture_mode: String,
    backend: String,
    #[serde(rename = "targetPIDs")]
    target_pids: Vec<i32>,
    #[serde(default, rename = "microphoneDeviceUID")]
    microphone_device_uid: Option<String>,
    #[serde(default)]
    target_application_identity: Option<crate::capture_scope::ApplicationIdentity>,
    state: String,
    errors: Vec<String>,
    microphone: CaptureTrack,
    system: CaptureTrack,
    #[serde(default)]
    capture_clock: Option<CaptureClock>,
    #[serde(default)]
    capture_end: Option<CaptureEnd>,
    #[serde(default)]
    recording_owner_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureTrack {
    frames: i64,
    sample_rate: Option<f64>,
    channels: Option<i64>,
    append_failures: i64,
    #[serde(default)]
    clock_segments: Option<Vec<crate::audio_clock::ClockSegment>>,
    #[cfg(unix)]
    #[serde(default)]
    raw_segments: Option<crate::recording_segments::Status>,
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
        // The recording requirement is not a four-hour stop policy. Keep the
        // timeline exactly representable across SQLite and JavaScript instead.
        if duration <= 0 || duration > 9_007_199_254_740_991 {
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

const MAX_CAPTURE_MANIFEST_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub mode: String,
    pub backend: String,
    pub target_pids: Vec<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone_device_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_application_identity: Option<crate::capture_scope::ApplicationIdentity>,
}

impl RecordingConfig {
    fn validate(&self) -> Result<(), String> {
        if let Some(identity) = &self.target_application_identity {
            identity.validate()?;
            if self.mode != "application"
                || self.backend != "screencapturekit"
                || self.target_pids != [identity.pid]
            {
                return Err("録音対象アプリの識別情報と取得条件が一致しません".into());
            }
        }
        if self
            .microphone_device_uid
            .as_deref()
            .is_some_and(|uid| !valid_microphone_uid(uid))
        {
            return Err("マイクの識別情報が不正です".into());
        }
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

pub(crate) fn valid_microphone_uid(uid: &str) -> bool {
    !uid.trim().is_empty() && uid.len() <= 1024 && !uid.chars().any(char::is_control)
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

pub(crate) struct ClosedInputRecording {
    pub(crate) session_id: String,
    pub(crate) meeting_id: String,
    pub(crate) title: String,
}

impl Storage {
    pub(crate) fn verify_closed_input_storage(&self) -> Result<(), String> {
        self.require_data_writable()?;
        #[cfg(unix)]
        self.managed_anchor.verify()?;
        if self.root.canonicalize().map_err(|e| e.to_string())? != self.root {
            return Err("保存後通知の管理領域が変更されています".into());
        }
        Ok(())
    }
    /// Bounded discovery from the database, never a directory scan. Random UUID
    /// order needs cursor wraparound by the caller to revisit newly closed rows.
    pub(crate) fn closed_input_recordings(
        &self,
        after: Option<&str>,
    ) -> Result<(Vec<ClosedInputRecording>, Option<String>), String> {
        self.verify_closed_input_storage()?;
        if after.is_some_and(|v| !crate::input_levels::valid_owner(v)) {
            return Err("録音通知一覧の続き位置が不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut statement = db.prepare(
            "SELECT r.id, r.meeting_id, m.title FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.state IN ('saved','interrupted','failed') AND (?1 IS NULL OR r.id > ?1) ORDER BY r.id LIMIT 33"
        ).map_err(|e|e.to_string())?;
        let mut rows = statement
            .query_map(params![after], |row| {
                Ok(ClosedInputRecording {
                    session_id: row.get(0)?,
                    meeting_id: row.get(1)?,
                    title: row.get::<_, String>(2)?.chars().take(120).collect(),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let next = if rows.len() > 32 {
            rows.truncate(32);
            rows.last().map(|r| r.session_id.clone())
        } else {
            None
        };
        drop(statement);
        drop(db);
        self.verify_closed_input_storage()?;
        Ok((rows, next))
    }

    /// Authorize companion-file writes only for a closed DB-owned recording.
    /// Return an anchored descriptor; release SQLite before file I/O/fsync.
    pub(crate) fn closed_input_directory(
        &self,
        meeting: &str,
        owner: &str,
    ) -> Result<File, String> {
        if self.closed_input_snapshot(meeting, owner)?.is_some() {
            // A restored audio asset directory is not a native producer owner
            // or permission to create/read a companion delivery ledger there.
            return Err("復元された録音の通知は元録音の管理先へ書き込めません".into());
        }
        let directory = crate::input_levels::open_capture_directory(
            &self
                .root
                .join("meetings")
                .join(meeting)
                .join("audio")
                .join(owner),
        )?;
        self.verify_closed_input_storage()?;
        Ok(directory)
    }

    /// NULL alone selects the native producer route. A present immutable
    /// snapshot (including diagnostic-only or invalid content) may NEVER fall
    /// back to guessed companion paths. Binding comes from the closed DB row,
    /// not from IDs supplied inside the snapshot itself.
    pub(crate) fn closed_input_snapshot(
        &self,
        meeting: &str,
        owner: &str,
    ) -> Result<Option<crate::input_status::queue::PortableInputSnapshot>, String> {
        self.verify_closed_input_storage()?;
        if !crate::input_levels::valid_owner(meeting) || !crate::input_levels::valid_owner(owner) {
            return Err("保存後通知の会議または録音IDが不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // Bound the original UTF-8 bytes before allocating them, even if a
        // damaged database has bypassed its normal column CHECK constraint.
        let row: Option<(String, String, Option<i64>, Option<String>)> = db
            .query_row(
                "SELECT meeting_id,state,length(CAST(input_history_snapshot_json AS BLOB)),CASE WHEN length(CAST(input_history_snapshot_json AS BLOB)) <= ?2 THEN input_history_snapshot_json ELSE NULL END FROM recording_sessions WHERE id=?1",
                params![owner, backup::input_snapshots::MAX_BYTES as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (actual_meeting, state, size, text) = row.ok_or("録音セッションが見つかりません")?;
        if actual_meeting != meeting
            || !matches!(state.as_str(), "saved" | "interrupted" | "failed")
        {
            return Err("保存後通知の対象が変更されたか録音が終了していません".into());
        }
        let result = match (size, text) {
            (None, None) => None,
            (Some(size), Some(text)) if size > 0 => {
                Some(backup::input_snapshots::Bound::decode(&text, meeting, owner)?.into_snapshot())
            }
            _ => return Err("入力履歴スナップショットの容量が不正です".into()),
        };
        drop(db);
        self.verify_closed_input_storage()?;
        Ok(result)
    }

    /// Read one DB-owned recording's retained producer suffix. No discovery,
    /// mutation, receipt adoption, telemetry freshening or audio replay.
    pub(crate) fn recording_input_history(
        &self,
        meeting_id: &str,
        session_id: &str,
    ) -> Result<crate::input_status::journal::Retained, String> {
        let result = match self.closed_input_snapshot(meeting_id, session_id)? {
            Some(snapshot) => snapshot
                .retained_history()?
                .ok_or("この復元録音には保存済みの取得履歴がありません")?,
            None => {
                let directory = self.closed_input_directory(meeting_id, session_id)?;
                crate::input_status::journal::Reader::pin_at(&directory)?.read(session_id)?
            }
        };
        self.verify_closed_input_storage()?;
        Ok(result)
    }

    pub(crate) fn recover_completed_recordings(&self) -> Result<Vec<String>, String> {
        if self.data_write_blocked() {
            return Ok(vec![]);
        }
        let interrupted = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません".to_string())?;
            let mut statement = connection
                .prepare("SELECT id FROM recording_sessions WHERE state = 'interrupted' AND audio_manifest_hash IS NULL ORDER BY created_at, id")
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
        if let Some(uid) = config.microphone_device_uid.as_deref() {
            super::capture_preferences::save(&transaction, uid)?;
        }
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

    #[cfg(unix)]
    fn verify_completed_raw_prefixes(
        &self,
        base: &Path,
        manifest: &CaptureManifest,
        owner: &str,
    ) -> Result<Vec<crate::recording_segments::Prefix>, String> {
        use crate::recording_segments::{Lane, Prefix};
        self.require_data_writable()?;
        self.managed_anchor.verify()?;
        let directory = crate::input_levels::open_capture_directory(&self.root.join(base))?;
        let mut prefixes = Vec::new();
        for (lane, track) in [
            (Lane::Microphone, &manifest.microphone),
            (Lane::System, &manifest.system),
        ] {
            let prefix = Prefix::read_optional(
                directory.try_clone().map_err(|e| e.to_string())?,
                owner,
                lane,
                &|| false,
            )?;
            match (prefix, &track.raw_segments) {
                (None, None) => {} // Native recordings made before raw segment support.
                (Some(prefix), Some(status))
                    if manifest.schema_version == 3
                        && manifest.recording_owner_id.as_deref() == Some(owner)
                        && status.agrees_with_completed(&prefix, track.frames as u64) =>
                {
                    prefixes.push(prefix);
                }
                _ => return Err("録音manifestと原音声の確定範囲・保存完了が一致しません".into()),
            }
        }
        if !prefixes.is_empty() && prefixes.len() != 2 {
            return Err("原音声の確定証拠が両入力で揃っていません".into());
        }
        self.require_data_writable()?;
        self.managed_anchor.verify()?;
        Ok(prefixes)
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
        let already_registered: bool = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT audio_manifest_hash IS NOT NULL FROM recording_sessions WHERE id=?1",
                [session_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if already_registered {
            return Err("登録済みの部分救出を正常な録音終了へ置き換えることはできません".into());
        }
        let base = PathBuf::from("meetings")
            .join(&session.meeting_id)
            .join("audio")
            .join(session_id);
        let manifest_file = self.checked_capture_file(&base.join("manifest.json"), None)?;
        let manifest = self.read_capture_manifest(&manifest_file)?;
        if !matches!(manifest.schema_version, 1 | 2 | 3)
            || manifest.state != "saved"
            || !manifest.errors.is_empty()
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
        config.validate()?;
        if manifest.capture_mode != stored_mode
            || manifest.capture_mode != config.mode
            || manifest.backend != config.backend
            || manifest.target_pids != config.target_pids
            || config
                .microphone_device_uid
                .as_ref()
                .is_some_and(|uid| manifest.microphone_device_uid.as_ref() != Some(uid))
            || config
                .target_application_identity
                .as_ref()
                .is_some_and(|identity| {
                    manifest.target_application_identity.as_ref() != Some(identity)
                })
        {
            return Err("録音manifestと開始条件が一致しません".into());
        }
        let mic_format = manifest.microphone.validated_format()?;
        let system_format = manifest.system.validated_format()?;
        let capture_clock = manifest.validated_capture_clock()?;
        let mic_timeline = manifest
            .microphone
            .validated_timeline(capture_clock.is_some())?;
        let system_timeline = manifest
            .system
            .validated_timeline(capture_clock.is_some())?;
        if capture_clock.is_some() && mic_timeline.start_us.min(system_timeline.start_us) != 0 {
            return Err("録音の共通原点が最初の保存音声と一致しません".into());
        }
        let ending = manifest.validated_capture_end(session_id, &mic_timeline, &system_timeline)?;
        let duration_us = ending.map_or_else(
            || mic_timeline.end_us.max(system_timeline.end_us),
            |end| end.end_us,
        );
        let ended_at = if let (Some(clock), Some(end)) = (capture_clock, ending) {
            Some(
                clock
                    .origin_utc_ms
                    .checked_add(end.end_us / 1000)
                    .filter(|v| (0..=8_640_000_000_000_000).contains(v))
                    .ok_or("録音終了UTCが範囲外です")?,
            )
        } else {
            None
        };
        let started_at = capture_clock
            .map(|clock| clock.origin_utc_ms)
            .or(session.started_at_utc_ms);
        let processing_config = if let Some(clock) = capture_clock {
            serde_json::json!({"timeline_alignment":"unverified","capture_clock":clock}).to_string()
        } else {
            "{\"timeline_alignment\":\"unverified\"}".into()
        };
        let mic_file =
            self.checked_capture_file(&base.join("microphone.caf"), Some(&manifest.microphone))?;
        let system_file =
            self.checked_capture_file(&base.join("system.caf"), Some(&manifest.system))?;
        #[cfg(unix)]
        let raw_prefixes = self.verify_completed_raw_prefixes(&base, &manifest, session_id)?;
        #[cfg(unix)]
        let publication = if raw_prefixes.is_empty() {
            None // Keep the admission rules for pre-segment native recordings.
        } else {
            Some(flac::Publication::prepare(
                self,
                &base,
                &manifest_file.hash,
                &manifest,
                &raw_prefixes,
            )?)
        };
        // Conversion can take time. Revalidate the manifest before the DB
        // transition, and keep the conversion lease alive through commit.
        let current_manifest = self.checked_capture_file(&manifest_file.relative, None)?;
        if current_manifest.size_bytes != manifest_file.size_bytes
            || current_manifest.hash != manifest_file.hash
        {
            return Err("録音manifestがFLAC変換中に変更されました".into());
        }
        self.require_data_writable()?;
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
        #[cfg(unix)]
        for prefix in &raw_prefixes {
            prefix.verify_at(&self.root.join(&base), &|| false)?;
        }
        #[cfg(unix)]
        if let Some(publication) = &publication {
            publication.verify(self)?;
        }
        insert_capture_asset(
            &transaction,
            &session.meeting_id,
            &manifest_file,
            "application/json",
            now,
        )?;
        #[cfg(unix)]
        let converted = publication.is_some();
        #[cfg(not(unix))]
        let converted = false;
        #[cfg(unix)]
        if let Some(publication) = &publication {
            publication.insert(
                &transaction,
                &session.meeting_id,
                session_id,
                &processing_config,
                duration_us,
                ending.is_some(),
                now,
            )?;
        }
        if !converted {
            for (kind, file, format, timeline, frames) in [
                (
                    "mic",
                    &mic_file,
                    mic_format,
                    &mic_timeline,
                    manifest.microphone.frames,
                ),
                (
                    "system",
                    &system_file,
                    system_format,
                    &system_timeline,
                    manifest.system.frames,
                ),
            ] {
                let asset_id = insert_capture_asset(
                    &transaction,
                    &session.meeting_id,
                    file,
                    "audio/x-caf",
                    now,
                )?;
                let track_id = Uuid::new_v4().to_string();
                transaction.execute(
                "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, processing_config_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'caf', ?5, ?6, 'ready', ?7, ?8, ?8)",
                params![track_id, session.meeting_id, session_id, kind, format.0, format.1, processing_config, now],
            ).map_err(|error| error.to_string())?;
                let chunk_id = Uuid::new_v4().to_string();
                transaction.execute(
                "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, 'ready', ?10)",
                params![chunk_id, session.meeting_id, track_id, asset_id, timeline.start_us, timeline.end_us, frames, format.0, format.1, now],
            ).map_err(|error| error.to_string())?;
                timeline.insert(&transaction, &track_id, &chunk_id)?;
                if ending.is_some() {
                    timeline.insert_boundary_gaps(&transaction, &track_id, duration_us)?;
                }
            }
        }
        transaction.execute(
            "UPDATE recording_sessions SET state = 'saved', duration_us = ?1, audio_manifest_hash = ?2, ended_at_utc_ms = ?3, last_committed_us = ?1, updated_at = ?6, started_at_utc_ms = ?5 WHERE id = ?4 AND state IN ('recording', 'stopping', 'interrupted')",
            params![duration_us, manifest_file.hash, ended_at.unwrap_or(now), session_id, started_at, now],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "UPDATE meetings SET lifecycle = 'recorded', duration_us = ?1, started_at_utc_ms = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lifecycle = 'recording'",
            params![duration_us, started_at, now, session.meeting_id],
        ).map_err(|error| error.to_string())?;
        let saved =
            load_session(&transaction, session_id)?.ok_or("録音セッションを読み込めません")?;
        super::analysis_flows::ensure_flow(&transaction, &session.meeting_id)?;
        #[cfg(unix)]
        for prefix in &raw_prefixes {
            prefix.verify_at(&self.root.join(&base), &|| false)?;
        }
        #[cfg(unix)]
        if let Some(publication) = &publication {
            publication.verify(self)?;
        }
        self.require_data_writable()?;
        #[cfg(all(test, unix))]
        if publication.is_some() {
            flac::crash_point("before-db-commit");
        }
        self.read_capture_manifest(&manifest_file)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(saved)
    }

    fn read_capture_manifest(&self, proof: &CapturedFile) -> Result<CaptureManifest, String> {
        let mut bytes = Vec::new();
        File::open(self.managed_path(&proof.relative)?)
            .map_err(|error| error.to_string())?
            .take(MAX_CAPTURE_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_CAPTURE_MANIFEST_BYTES {
            return Err("録音manifestが大きすぎます".into());
        }
        if bytes.len() as i64 != proof.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != proof.hash
        {
            return Err("録音manifestが検証中に変更されました".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        // A test is not a meeting recording, even if both private CAF files
        // finalized successfully. Never adopt its short capture as a session.
        if value.get("captureTest").is_some() {
            return Err("録音前テストの音声を会議録音として確定することはできません".into());
        }
        // Preserve serde's duplicate-known-field rejection on the original
        // bytes; a Value alone would collapse conflicting fields.
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    fn checked_capture_file(
        &self,
        relative: &Path,
        caf: Option<&CaptureTrack>,
    ) -> Result<CapturedFile, String> {
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if actual != root.join(relative) || !actual.is_file() {
            return Err("録音ファイルが管理領域外または通常ファイルではありません".into());
        }
        let handle = File::open(&actual).map_err(|error| error.to_string())?;
        let (size_bytes, hash) = if let Some(track) = caf {
            let (rate, channels, _) = track.validated_format()?;
            let (format, size, hash) = crate::audio::inspect_recorded_caf(
                handle.try_clone().map_err(|error| error.to_string())?,
                &|| false,
            )?;
            if format.frames != track.frames as u64
                || i64::from(format.sample_rate) != rate
                || i64::from(format.channels) != channels
            {
                return Err("録音CAFの実フレーム数・形式とmanifestが一致しません".into());
            }
            (
                i64::try_from(size).map_err(|_| "録音ファイルが大きすぎます")?,
                hash,
            )
        } else {
            let before = handle.metadata().map_err(|error| error.to_string())?;
            if before.len() > MAX_CAPTURE_MANIFEST_BYTES {
                return Err("録音manifestが大きすぎます".into());
            }
            let mut bytes = Vec::new();
            (&handle)
                .take(MAX_CAPTURE_MANIFEST_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            let after = handle.metadata().map_err(|error| error.to_string())?;
            if bytes.len() as u64 > MAX_CAPTURE_MANIFEST_BYTES {
                return Err("録音manifestが大きすぎます".into());
            }
            if bytes.len() as u64 != before.len()
                || after.len() != before.len()
                || after.modified().map_err(|error| error.to_string())?
                    != before.modified().map_err(|error| error.to_string())?
            {
                return Err("録音manifestが検証中に変更されました".into());
            }
            (bytes.len() as i64, format!("{:x}", Sha256::digest(&bytes)))
        };
        if size_bytes < if caf.is_some() { 16 } else { 2 } {
            return Err("録音ファイルが短すぎます".into());
        }
        handle.sync_all().map_err(|error| error.to_string())?;
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
pub(in crate::storage) mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn retained_input_history_is_bound_to_closed_database_session_and_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, _) = saved_probe_fixture(&storage);
        let current =
            serde_json::json!({"state":"stopped","sourceEpoch":2,"interruptions":1,"reason":null});
        let history = serde_json::json!({"lastSequence":2,"droppedThrough":0,"events":[
            {"sequence":1,"kind":"interrupted","status":{"state":"disconnected","sourceEpoch":1,"interruptions":1,"reason":"device_disconnected"}},
            {"sequence":2,"kind":"recovered","status":{"state":"connected","sourceEpoch":2,"interruptions":1,"reason":null}}
        ]});
        let value = serde_json::json!({"schemaVersion":1,"sealed":true,"report":{
            "schemaVersion":3,"ownerId":session,"revision":7,"microphone":current,"system":null,
            "microphoneHistory":history,"systemHistory":null}});
        let leaf = output.join("input-events.json");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&leaf, &bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        // Preparing/recording/stopping are not an archived session. No history
        // read may convert a still-active owner into an offline delivery claim.
        assert!(storage.recording_input_history(&meeting, &session).is_err());
        storage.finalize_recording(&session).unwrap();
        let dto =
            serde_json::to_value(storage.recording_input_history(&meeting, &session).unwrap())
                .unwrap();
        assert_eq!(dto["ownerId"], session);
        assert_eq!(dto["microphoneHistory"]["lastSequence"], 2);
        assert!(storage
            .recording_input_history(&Uuid::new_v4().to_string(), &session)
            .is_err());
        assert!(storage
            .recording_input_history(&meeting, "../foreign")
            .is_err());
        drop(storage);
        let reopened = Storage::open(temp.path().into()).unwrap();
        assert_eq!(
            serde_json::to_value(
                reopened
                    .recording_input_history(&meeting, &session)
                    .unwrap()
            )
            .unwrap(),
            dto
        );
        assert_eq!(std::fs::read(&leaf).unwrap(), bytes);
        let mut foreign = value.clone();
        foreign["report"]["ownerId"] = serde_json::json!(Uuid::new_v4().to_string());
        std::fs::write(&leaf, serde_json::to_vec(&foreign).unwrap()).unwrap();
        assert!(reopened
            .recording_input_history(&meeting, &session)
            .is_err());
    }

    #[test]
    fn recording_duration_has_no_four_hour_cap_but_requires_an_exact_safe_timeline() {
        let mut track = CaptureTrack {
            frames: 8_000 * 14_460,
            sample_rate: Some(8_000.0),
            channels: Some(1),
            append_failures: 0,
            clock_segments: None,
            #[cfg(unix)]
            raw_segments: None,
        };
        assert_eq!(track.validated_format().unwrap(), (8000, 1, 14_460_000_000));
        track.frames = 8_000 * 86_400;
        assert_eq!(track.validated_format().unwrap().2, 86_400_000_000);
        // Largest complete frame count that fits the exact JS microsecond range.
        track.frames = (9_007_199_254_740_991_i128 * 8000 / 1_000_000) as i64;
        assert!(track.validated_format().is_ok());
        track.frames += 1;
        assert!(track.validated_format().is_err());
        for frames in [0, -1, i64::MAX] {
            track.frames = frames;
            assert!(track.validated_format().is_err());
        }
        track.frames = 8000;
        for rate in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(7999.0),
            Some(8000.5),
            Some(384001.0),
        ] {
            track.sample_rate = rate;
            assert!(track.validated_format().is_err());
        }
        track.sample_rate = Some(8000.0);
        for channels in [None, Some(0), Some(9)] {
            track.channels = channels;
            assert!(track.validated_format().is_err());
        }
        track.channels = Some(1);
        track.append_failures = 1;
        assert!(track.validated_format().is_err());
    }

    #[test]
    fn saved_recording_beyond_four_hours_verifies_physical_caf_and_commits_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, mut manifest) = saved_probe_fixture(&storage);
        let frames = 8_000_i64 * 14_460;
        for (name, channels) in [("microphone", 1_u32), ("system", 2_u32)] {
            // Valid sparse signed-int16 CAF: no waiting, audio capture, or large Vec.
            let mut header = crate::audio::tests::caf_fixture(8000, channels, 2, 16, 2, &[]);
            let data_chunk = header.len() - 16;
            assert_eq!(&header[data_chunk..data_chunk + 4], b"data");
            let data_bytes = frames * i64::from(channels) * 2;
            header[data_chunk + 4..data_chunk + 12]
                .copy_from_slice(&(data_bytes + 4).to_be_bytes());
            let mut file = File::create(output.join(format!("{name}.caf"))).unwrap();
            file.write_all(&header).unwrap();
            file.set_len(header.len() as u64 + data_bytes as u64)
                .unwrap();
            file.sync_all().unwrap();
            manifest[name] = serde_json::json!({
                "frames": frames, "sampleRate": 8000, "channels": channels, "appendFailures": 0,
            });
        }
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        drop(storage);
        let reopened = Storage::open(temp.path().into()).unwrap();
        assert_eq!(
            reopened.recover_completed_recordings().unwrap(),
            vec![session.clone()]
        );
        let saved = load_session(&reopened.connection.lock().unwrap(), &session)
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, "saved");
        assert_eq!(saved.duration_us, 14_460_000_000);
        assert_eq!(
            reopened.get_meeting(&meeting).unwrap().unwrap().lifecycle,
            "recorded"
        );
        let connection = reopened.connection.lock().unwrap();
        let duration: i64 = connection
            .query_row(
                "SELECT duration_us FROM meetings WHERE id = ?1",
                params![meeting],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(duration, 14_460_000_000);
        let chunks: Vec<(i64, i64, i64, i64, i64, String)> = connection
            .prepare("SELECT c.frame_count, c.sample_rate, c.channels, c.start_us, c.end_us, t.processing_config_json FROM audio_chunks c JOIN audio_tracks t ON t.id = c.track_id WHERE c.meeting_id = ?1 ORDER BY c.channels")
            .unwrap().query_map(params![meeting], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
            .unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(chunks.len(), 2);
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                (chunk.0, chunk.1, chunk.2, chunk.3, chunk.4),
                (frames, 8000, index as i64 + 1, 0, 14_460_000_000)
            );
            assert_eq!(chunk.5, "{\"timeline_alignment\":\"unverified\"}");
        }
    }

    fn caf(channels: u32) -> Vec<u8> {
        crate::audio::tests::caf_fixture(
            48000,
            channels,
            3,
            32,
            4,
            &vec![0; 48000 * channels as usize * 4],
        )
    }

    fn config() -> RecordingConfig {
        RecordingConfig {
            mode: "system".into(),
            backend: "screencapturekit".into(),
            target_pids: vec![],
            microphone_device_uid: None,
            target_application_identity: None,
        }
    }

    pub(in crate::storage) fn saved_probe_fixture(
        storage: &Storage,
    ) -> (String, String, PathBuf, serde_json::Value) {
        let meeting = storage.create_meeting("物理音声の確定").unwrap();
        let session = storage
            .prepare_recording(&meeting.id, &config(), &Uuid::new_v4().to_string())
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let output = storage
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("microphone.caf"), caf(1)).unwrap();
        std::fs::write(output.join("system.caf"), caf(2)).unwrap();
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
        (meeting.id, session.id, output, manifest)
    }

    #[test]
    fn application_identity_must_match_before_atomic_audio_publication() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, mut manifest) = saved_probe_fixture(&storage);
        let identity = crate::capture_scope::ApplicationIdentity {
            pid: 123,
            bundle_identifier: "com.example.browser".into(),
            launch_time_us: 1791100000000001,
        };
        let mut selected = config();
        selected.mode = "application".into();
        selected.target_pids = vec![identity.pid];
        selected.target_application_identity = Some(identity.clone());
        assert!(selected.validate().is_ok());
        storage.connection.lock().unwrap().execute("UPDATE recording_sessions SET capture_mode='application',capture_config_json=?1 WHERE id=?2", params![serde_json::to_string(&selected).unwrap(), session]).unwrap();
        manifest["captureMode"] = serde_json::json!("application");
        manifest["targetPIDs"] = serde_json::json!([identity.pid]);
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!({"pid":123,"bundleIdentifier":"com.example.browser","launchTimeUs":1791100000000002_i64}),
            serde_json::json!({"pid":123,"bundleIdentifier":"com.example.other","launchTimeUs":identity.launch_time_us}),
            serde_json::json!({"pid":456,"bundleIdentifier":identity.bundle_identifier,"launchTimeUs":identity.launch_time_us}),
        ] {
            manifest["targetApplicationIdentity"] = invalid;
            std::fs::write(
                output.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            assert!(storage
                .finalize_recording(&session)
                .unwrap_err()
                .contains("開始条件"));
            assert!(output.join("microphone.caf").exists());
            assert!(output.join("system.caf").exists());
            assert_eq!(
                storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
                "recording"
            );
            assert_eq!(
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM audio_tracks", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        manifest["targetApplicationIdentity"] = serde_json::to_value(&identity).unwrap();
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        storage.finalize_recording(&session).unwrap();
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
            "recorded"
        );
    }

    #[test]
    fn application_identity_rejects_conflicting_configuration_before_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let meeting = storage.create_meeting("アプリの条件確認").unwrap();
        let identity = crate::capture_scope::ApplicationIdentity {
            pid: 123,
            bundle_identifier: "com.example.browser".into(),
            launch_time_us: 1791100000000001,
        };
        for (mode, backend, pids) in [
            ("system", "screencapturekit", vec![]),
            ("application", "coreaudio", vec![123]),
            ("application", "screencapturekit", vec![456]),
            ("application", "screencapturekit", vec![]),
        ] {
            let mut cfg = config();
            cfg.mode = mode.into();
            cfg.backend = backend.into();
            cfg.target_pids = pids;
            cfg.target_application_identity = Some(identity.clone());
            assert!(storage
                .prepare_recording(&meeting.id, &cfg, &Uuid::new_v4().to_string())
                .is_err());
            assert_eq!(
                storage.get_meeting(&meeting.id).unwrap().unwrap().lifecycle,
                "draft"
            );
        }
        let legacy = config();
        let json = serde_json::to_string(&legacy).unwrap();
        assert!(!json.contains("target_application_identity"));
        assert_eq!(
            serde_json::from_str::<RecordingConfig>(&json).unwrap(),
            legacy
        );
        let mut cfg = config();
        cfg.mode = "application".into();
        cfg.target_pids = vec![123];
        cfg.target_application_identity = Some(identity);
        assert!(storage
            .prepare_recording(&meeting.id, &cfg, &Uuid::new_v4().to_string())
            .is_ok());
    }

    #[test]
    fn explicit_microphone_identity_must_match_before_audio_publication() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, mut manifest) = saved_probe_fixture(&storage);
        let mut selected = config();
        selected.microphone_device_uid = Some("usb-123".into());
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE recording_sessions SET capture_config_json=?1 WHERE id=?2",
                params![serde_json::to_string(&selected).unwrap(), session],
            )
            .unwrap();
        for uid in [None, Some("another-microphone")] {
            manifest
                .as_object_mut()
                .unwrap()
                .remove("microphoneDeviceUID");
            if let Some(uid) = uid {
                manifest["microphoneDeviceUID"] = serde_json::json!(uid);
            }
            std::fs::write(
                output.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            assert!(storage
                .finalize_recording(&session)
                .unwrap_err()
                .contains("開始条件"));
            assert!(output.join("microphone.caf").exists());
            assert_eq!(
                storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
                "recording"
            );
            assert_eq!(
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM audio_tracks", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        manifest["microphoneDeviceUID"] = serde_json::json!("usb-123");
        std::fs::write(
            output.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        storage.finalize_recording(&session).unwrap();
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
            "recorded"
        );
    }

    #[test]
    fn physical_caf_mismatch_refuses_atomic_publication_and_preserves_retryable_files() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, _) = saved_probe_fixture(&storage);
        let manifest = std::fs::read(output.join("manifest.json")).unwrap();
        for (name, channels) in [("microphone.caf", 1), ("system.caf", 2)] {
            let path = output.join(name);
            let original = std::fs::read(&path).unwrap();
            for (rate, count, frames) in [
                (44100, channels, 48000),
                (48000, 3 - channels, 48000),
                (48000, channels, 47999),
                (48000, channels, 48001),
            ] {
                let wrong = crate::audio::tests::caf_fixture(
                    rate,
                    count,
                    3,
                    32,
                    4,
                    &vec![0; frames * count as usize * 4],
                );
                std::fs::write(&path, &wrong).unwrap();
                let error = storage.finalize_recording(&session).err().unwrap();
                assert!(
                    error.contains("実フレーム数・形式とmanifestが一致"),
                    "{error}"
                );
                assert_eq!(std::fs::read(&path).unwrap(), wrong);
                assert_eq!(
                    std::fs::read(output.join("manifest.json")).unwrap(),
                    manifest
                );
                assert_eq!(
                    storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
                    "recording"
                );
                let db = storage.connection.lock().unwrap();
                assert_eq!(
                    load_session(&db, &session).unwrap().unwrap().state,
                    "recording"
                );
                for table in [
                    "file_assets",
                    "audio_tracks",
                    "audio_chunks",
                    "analysis_flows",
                ] {
                    let count: i64 = db
                        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    assert_eq!(count, 0, "{table}");
                }
                drop(db);
                std::fs::write(&path, &original).unwrap();
            }
        }
        assert_eq!(storage.finalize_recording(&session).unwrap().state, "saved");
        let db = storage.connection.lock().unwrap();
        let mut query = db.prepare("SELECT relative_path, size_bytes, content_hash FROM file_assets WHERE meeting_id=?1").unwrap();
        let proofs = query
            .query_map(params![meeting], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(proofs.len(), 3);
        for (relative, size, hash) in proofs {
            let bytes = std::fs::read(storage.root.join(relative)).unwrap();
            assert_eq!(size, bytes.len() as i64);
            assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
        }
    }

    #[test]
    fn malformed_and_nonfinite_saved_audio_stays_interrupted_without_recovery_publication() {
        for name in ["microphone.caf", "system.caf"] {
            for invalid in 0..3 {
                let temp = tempfile::tempdir().unwrap();
                let storage = Storage::open(temp.path().into()).unwrap();
                let (meeting, session, output, _) = saved_probe_fixture(&storage);
                let path = output.join(name);
                let channels = if name == "microphone.caf" { 1 } else { 2 };
                let mut wrong = caf(channels);
                match invalid {
                    0 => wrong = b"caff000000000000".to_vec(),
                    1 => {
                        wrong.pop();
                    }
                    _ => {
                        let end = wrong.len();
                        wrong[end - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
                    }
                }
                std::fs::write(&path, &wrong).unwrap();
                drop(storage);
                let reopened = Storage::open(temp.path().into()).unwrap();
                assert!(reopened.recover_completed_recordings().unwrap().is_empty());
                let db = reopened.connection.lock().unwrap();
                assert_eq!(
                    load_session(&db, &session).unwrap().unwrap().state,
                    "interrupted"
                );
                let count: i64 = db
                    .query_row(
                        "SELECT COUNT(*) FROM file_assets WHERE meeting_id=?1",
                        params![meeting],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 0);
                assert_eq!(std::fs::read(&path).unwrap(), wrong);
            }
        }
    }

    #[test]
    fn parsed_capture_manifest_must_match_the_exact_registered_hash() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (_, _, output, mut manifest) = saved_probe_fixture(&storage);
        let path = output.join("manifest.json");
        let relative = path.strip_prefix(&storage.root).unwrap();
        let proof = storage.checked_capture_file(relative, None).unwrap();
        assert_eq!(
            storage.read_capture_manifest(&proof).unwrap().state,
            "saved"
        );
        manifest["state"] = serde_json::json!("ready");
        let changed = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(changed.len() as i64, proof.size_bytes);
        std::fs::write(&path, &changed).unwrap();
        let error = storage.read_capture_manifest(&proof).err().unwrap();
        assert!(error.contains("検証中に変更"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), changed);
    }

    #[test]
    fn five_second_test_manifest_is_never_adopted_as_a_meeting_or_analysis_job() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, mut manifest) = saved_probe_fixture(&storage);
        let mic = std::fs::read(output.join("microphone.caf")).unwrap();
        let system = std::fs::read(output.join("system.caf")).unwrap();
        for test in [
            serde_json::Value::Null,
            serde_json::json!({"ownerId":Uuid::new_v4().to_string(),"durationSeconds":5,"completed":true}),
        ] {
            manifest["captureTest"] = test;
            let bytes = serde_json::to_vec(&manifest).unwrap();
            std::fs::write(output.join("manifest.json"), &bytes).unwrap();
            assert!(storage
                .finalize_recording(&session)
                .unwrap_err()
                .contains("録音前テスト"));
            assert_eq!(
                storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
                "recording"
            );
            let db = storage.connection.lock().unwrap();
            for table in ["audio_tracks", "file_assets", "processing_jobs"] {
                assert_eq!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            }
            drop(db);
            assert_eq!(std::fs::read(output.join("manifest.json")).unwrap(), bytes);
            assert_eq!(std::fs::read(output.join("microphone.caf")).unwrap(), mic);
            assert_eq!(std::fs::read(output.join("system.caf")).unwrap(), system);
        }
    }

    #[test]
    fn capture_test_marker_check_does_not_weaken_duplicate_field_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (_, session, output, manifest) = saved_probe_fixture(&storage);
        let original = serde_json::to_string(&manifest).unwrap();
        let duplicated = format!("{{\"state\":\"saved\",{}", &original[1..]);
        std::fs::write(output.join("manifest.json"), &duplicated).unwrap();
        assert!(storage
            .finalize_recording(&session)
            .unwrap_err()
            .contains("duplicate field"));
        assert_eq!(
            std::fs::read_to_string(output.join("manifest.json")).unwrap(),
            duplicated
        );
    }

    #[test]
    #[ignore = "requires actual Swift-generated five-second test CAF fixtures; make check-capture-test"]
    fn swift_generated_five_second_test_caf_and_manifest_stay_private() {
        let fixture = PathBuf::from(std::env::var_os("MON_CAPTURE_TEST_FIXTURE_DIR").unwrap());
        let manifest_bytes = std::fs::read(fixture.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest["schemaVersion"], 2);
        assert_eq!(manifest["state"], "saved");
        assert_eq!(manifest["requestedDurationSeconds"], 5);
        let test = &manifest["captureTest"];
        assert!(crate::input_levels::valid_owner(
            test["ownerId"].as_str().unwrap()
        ));
        assert_eq!(test["durationSeconds"], 5);
        assert_eq!(test["completed"], true);
        assert_eq!(
            test["windowEndNs"].as_i64().unwrap() - test["windowStartNs"].as_i64().unwrap(),
            5_000_000_000
        );
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, session, output, _) = saved_probe_fixture(&storage);
        for (name, frames, value) in [
            ("microphone.caf", 40000_u64, 0.125_f32),
            ("system.caf", 31200_u64, 0.0_f32),
        ] {
            let source = fixture.join(name);
            let (format, size, hash) =
                crate::audio::inspect_recorded_caf(File::open(&source).unwrap(), &|| false)
                    .unwrap();
            assert_eq!(format.sample_rate, 8000);
            assert_eq!(format.channels, 2);
            assert_eq!(format.frames, frames);
            let mut audio =
                crate::audio::VerifiedAudio::open(File::open(&source).unwrap(), size, &hash)
                    .unwrap();
            let bytes = audio.read_frames(0, frames).unwrap();
            assert_eq!(bytes.len() as u64, frames * 8);
            for frame in bytes.chunks_exact(8) {
                assert_eq!(f32::from_le_bytes(frame[..4].try_into().unwrap()), value);
                assert_eq!(f32::from_le_bytes(frame[4..].try_into().unwrap()), -value);
            }
            std::fs::copy(&source, output.join(name)).unwrap();
        }
        assert_eq!(
            manifest["system"]["clockSegments"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            manifest["system"]["clockSegments"][1]["timelineStartUs"],
            2_900_000
        );
        std::fs::write(output.join("manifest.json"), &manifest_bytes).unwrap();
        assert!(storage
            .finalize_recording(&session)
            .unwrap_err()
            .contains("録音前テスト"));
        assert_eq!(
            storage.get_meeting(&meeting).unwrap().unwrap().lifecycle,
            "recording"
        );
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM audio_tracks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            std::fs::read(output.join("manifest.json")).unwrap(),
            manifest_bytes
        );
    }

    #[test]
    fn oversized_capture_manifest_is_rejected_before_hashing_and_after_a_prior_proof() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (_, _, output, _) = saved_probe_fixture(&storage);
        let path = output.join("manifest.json");
        let relative = path.strip_prefix(&storage.root).unwrap();
        let proof = storage.checked_capture_file(relative, None).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        let error = storage.checked_capture_file(relative, None).err().unwrap();
        assert!(error.contains("大きすぎ"), "{error}");
        let error = storage.read_capture_manifest(&proof).err().unwrap();
        assert!(error.contains("大きすぎ"), "{error}");
        assert_eq!(file.metadata().unwrap().len(), 1024 * 1024 * 1024);
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
        std::fs::write(output.join("microphone.caf"), &caf(1)).unwrap();
        std::fs::write(output.join("system.caf"), &caf(2)).unwrap();
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
        std::fs::write(pending_output.join("microphone.caf"), &caf(1)).unwrap();
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
        std::fs::write(output.join("microphone.caf"), &caf(1)).unwrap();
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
        std::fs::write(output.join("system.caf"), &caf(2)).unwrap();
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
        // Physical CAF frames must also match before this transaction can publish.
        std::fs::write(output.join("microphone.caf"), &caf(1)).unwrap();
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
        std::fs::write(output.join("system.caf"), &caf(2)).unwrap();
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
