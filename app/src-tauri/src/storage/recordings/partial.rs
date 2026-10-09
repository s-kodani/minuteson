//! Explicit recovery of durably advertised PCM only. This capability cannot
//! be deserialized from IPC. Closure, UTC alignment and lost-tail duration are
//! not inferred from a prefix, a failed process or the time of recovery.
use super::*;
use crate::recording_segments::{Lane, Prefix};
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneReview {
    pub lane: String,
    pub state: String,
    pub sealed_segments: usize,
    pub frames: u64,
    pub start_us: Option<i64>,
    pub end_us: Option<i64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    pub session_id: String,
    pub meeting_id: String,
    pub review_hash: String,
    pub duration_us: i64,
    pub lanes: Vec<LaneReview>,
    pub acquisition_end_known: bool,
    pub tail_loss_us: Option<i64>,
}

/// Only Storage inspection can mint this exact-generation capability. The UI
/// controller must retain it until explicit consent; a client summary is not
/// authority to recover a guessed path or an updated prefix.
pub(crate) struct Review {
    pub summary: Summary,
    session: RecordingSession,
    config: RecordingConfig,
    config_text: String,
    meeting_version: i64,
    meeting_state: String,
    base: PathBuf,
    origin: i64,
    prefixes: Vec<Prefix>,
}

impl Storage {
    pub(crate) fn review_partial_recording(&self, session_id: &str) -> Result<Review, String> {
        self.require_data_writable()?;
        self.managed_anchor.verify()?;
        if !crate::input_levels::valid_owner(session_id) {
            return Err("部分救出の録音IDが不正です".into());
        }
        let (session, config_text, meeting_version, meeting_state) = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            partial_admission(&db, session_id)?
        };
        let config: RecordingConfig =
            serde_json::from_str(&config_text).map_err(|e| e.to_string())?;
        config.validate()?;
        let base = PathBuf::from("meetings")
            .join(&session.meeting_id)
            .join("audio")
            .join(session_id);
        let directory = crate::input_levels::open_capture_directory(&self.root.join(&base))?;
        let identity = directory.metadata().map_err(|e| e.to_string())?;
        let mut prefixes = Vec::new();
        let mut lanes = Vec::new();
        for lane in [Lane::Microphone, Lane::System] {
            let result = Prefix::read_optional(
                directory.try_clone().map_err(|e| e.to_string())?,
                session_id,
                lane,
                &|| false,
            );
            let state = match result {
                Ok(Some(prefix)) if prefix.frames() > 0 => {
                    prefixes.push(prefix);
                    "available"
                }
                Ok(Some(_)) => "empty",
                Ok(None) => "missing",
                // The other lane's valid sealed prefix is still recoverable.
                // Do not guess CAF names or consume unadvertised trailing files.
                Err(_) => "unreadable",
            };
            lanes.push(LaneReview {
                lane: lane.name().into(),
                state: state.into(),
                sealed_segments: 0,
                frames: 0,
                start_us: None,
                end_us: None,
                sample_rate: None,
                channels: None,
            });
        }
        let origin = prefixes
            .iter()
            .map(Prefix::first_capture_nanoseconds)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .unwrap_or(0);
        let mut duration_us = 0;
        for prefix in &prefixes {
            let groups = prefix.flac_groups(origin)?;
            let first = prefix
                .entries()
                .next()
                .ok_or("部分救出の確定範囲がありません")?;
            let summary = lanes
                .iter_mut()
                .find(|lane| lane.lane == prefix.lane().name())
                .unwrap();
            summary.sealed_segments = prefix.entries().len();
            summary.frames = prefix.frames();
            summary.start_us = Some(groups[0].clock_segments[0].timeline_start_us);
            summary.end_us = Some(
                groups
                    .last()
                    .unwrap()
                    .clock_segments
                    .last()
                    .unwrap()
                    .timeline_end_us,
            );
            summary.sample_rate = Some(first.sample_rate);
            summary.channels = Some(first.channels);
            duration_us = duration_us.max(summary.end_us.unwrap());
        }
        let generations = prefixes
            .iter()
            .map(Prefix::review_hash)
            .collect::<Result<Vec<_>, _>>()?;
        let proof = serde_json::to_vec(&serde_json::json!({"version":1,
            "session":session,"config":config_text,"meetingVersion":meeting_version,
            "meetingState":meeting_state,"directory":[identity.dev(),identity.ino(),identity.uid()],
            "origin":origin,"lanes":lanes,"generations":generations}))
        .map_err(|e| e.to_string())?;
        let review_hash = format!("{:x}", Sha256::digest(&proof));
        self.require_data_writable()?;
        self.managed_anchor.verify()?;
        for prefix in &prefixes {
            prefix.verify_at(&self.root.join(&base), &|| false)?;
        }
        Ok(Review {
            summary: Summary {
                session_id: session.id.clone(),
                meeting_id: session.meeting_id.clone(),
                review_hash,
                duration_us,
                lanes,
                acquisition_end_known: false,
                tail_loss_us: None,
            },
            session,
            config,
            config_text,
            meeting_version,
            meeting_state,
            base,
            origin,
            prefixes,
        })
    }

    pub(crate) fn recover_partial_recording(
        &self,
        reviewed: &Review,
        confirmed: bool,
    ) -> Result<RecordingSession, String> {
        if !confirmed {
            return Err("確定済み部分だけを救出する明示的な同意が必要です".into());
        }
        let current = self.review_partial_recording(&reviewed.session.id)?;
        if current.summary != reviewed.summary {
            return Err("部分救出の対象が確認後に変更されました。再確認してください".into());
        }
        drop(current);
        if reviewed.prefixes.is_empty() || reviewed.summary.duration_us <= 0 {
            return Err("部分救出できる確定済み音声がありません".into());
        }
        let mut publication = flac::Publication::prepare_recovery(
            self,
            &reviewed.base,
            &reviewed.summary.review_hash,
            reviewed.origin,
            &reviewed.prefixes,
        )?;
        let recovery = serde_json::json!({"schemaVersion":1,"kind":"sealed_prefix",
            "reviewHash":reviewed.summary.review_hash,"acquisitionEndKnown":false,
            "tailLossUs":null,"timelineUtcOriginKnown":false,
            "originalState":reviewed.session.state,
            "originalObservedEndUtcMs":reviewed.session.ended_at_utc_ms,
            "originalDurationUs":reviewed.session.duration_us,
            "timelineOriginNanoseconds":reviewed.origin,"lanes":reviewed.summary.lanes});
        let document = serde_json::json!({"schemaVersion":1,"state":"interrupted",
            "captureMode":reviewed.config.mode,"backend":reviewed.config.backend,
            "targetPIDs":reviewed.config.target_pids,"recordingOwnerId":reviewed.session.id,
            "errors":[],"partialRecovery":recovery});
        let manifest = publication.recovery_document(self, &document)?;
        let config =
            serde_json::json!({"timeline_alignment":"unverified","partial_recovery":recovery})
                .to_string();
        // Re-inspect both lanes after conversion: newly available or changed
        // evidence must not be included, or omitted, under an old confirmation.
        if self.review_partial_recording(&reviewed.session.id)?.summary != reviewed.summary {
            return Err("部分救出の対象が変換中に変更されました".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let admitted = partial_admission(&tx, &reviewed.session.id)?;
        if admitted
            != (
                reviewed.session.clone(),
                reviewed.config_text.clone(),
                reviewed.meeting_version,
                reviewed.meeting_state.clone(),
            )
        {
            return Err("部分救出の録音・会議状態が確認後に変更されました".into());
        }
        self.require_data_writable()?;
        self.managed_anchor.verify()?;
        for prefix in &reviewed.prefixes {
            prefix.verify_at(&self.root.join(&reviewed.base), &|| false)?;
        }
        publication.verify(self)?;
        let now = now_ms()?;
        insert_capture_asset(
            &tx,
            &reviewed.session.meeting_id,
            &manifest,
            "application/json",
            now,
        )?;
        publication.insert(
            &tx,
            &reviewed.session.meeting_id,
            &reviewed.session.id,
            &config,
            reviewed.summary.duration_us,
            true,
            now,
        )?;
        // Preserve interruption, not a fictitious successful stop. Duration is
        // the recovered timeline extent; actual acquisition end remains NULL.
        tx.execute("UPDATE recording_sessions SET state='interrupted',duration_us=?1,last_committed_us=?1,audio_manifest_hash=?2,ended_at_utc_ms=NULL,updated_at=?3 WHERE id=?4",
            params![reviewed.summary.duration_us,manifest.hash,now,reviewed.session.id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE meetings SET lifecycle='recorded',duration_us=?1,lock_version=lock_version+1,updated_at=?2 WHERE id=?3 AND lock_version=?4",
            params![reviewed.summary.duration_us,now,reviewed.session.meeting_id,reviewed.meeting_version]).map_err(|e|e.to_string())?;
        super::super::analysis_flows::ensure_flow(&tx, &reviewed.session.meeting_id)?;
        let source = super::super::transcription::read_analysis_audio_plan(
            &tx,
            &reviewed.session.meeting_id,
            None,
        )?;
        super::super::transcription::known::read(&tx, &reviewed.session.meeting_id, &source)?;
        for prefix in &reviewed.prefixes {
            prefix.verify_at(&self.root.join(&reviewed.base), &|| false)?;
        }
        publication.verify(self)?;
        self.require_data_writable()?;
        let result =
            load_session(&tx, &reviewed.session.id)?.ok_or("部分救出の結果がありません")?;
        #[cfg(test)]
        flac::crash_point("partial-before-db-commit");
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

fn partial_admission(
    db: &Connection,
    id: &str,
) -> Result<(RecordingSession, String, i64, String), String> {
    let session = load_session(db, id)?.ok_or("部分救出する録音がありません")?;
    if !matches!(session.state.as_str(), "interrupted" | "failed") {
        return Err("未終了または失敗した録音だけを部分救出できます".into());
    }
    let (config, mode, manifest, restored): (String, String, Option<String>, bool) = db.query_row(
        "SELECT capture_config_json,capture_mode,audio_manifest_hash,input_history_snapshot_json IS NOT NULL FROM recording_sessions WHERE id=?1",
        [id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).map_err(|e|e.to_string())?;
    let parsed: RecordingConfig = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    parsed.validate()?;
    if mode != parsed.mode || manifest.is_some() || restored {
        return Err("部分救出の開始条件・元録音の所属または未確定状態が一致しません".into());
    }
    let occupied: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping')) OR EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1) OR EXISTS(SELECT 1 FROM processing_jobs WHERE meeting_id=?1 AND state IN ('queued','running','pausing','paused','interrupted'))",
        [&session.meeting_id],|row|row.get(0)).map_err(|e|e.to_string())?;
    if occupied {
        return Err("録音中・解析中または登録済みの音声は部分救出できません".into());
    }
    let (version, lifecycle, source): (i64, String, Option<String>) = db
        .query_row(
            "SELECT lock_version,lifecycle,source_type FROM meetings WHERE id=?1",
            [&session.meeting_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    if source.as_deref() != Some("recording")
        || !matches!(lifecycle.as_str(), "recording" | "draft")
    {
        return Err("部分救出の会議状態が不正です".into());
    }
    Ok((session, config, version, lifecycle))
}
