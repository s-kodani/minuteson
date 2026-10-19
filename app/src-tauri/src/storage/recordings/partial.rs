//! Explicit recovery of durably advertised PCM only. This capability cannot
//! be deserialized from IPC. Closure, UTC alignment and lost-tail duration are
//! not inferred from a prefix, a failed process or the time of recovery.
use super::*;
use crate::recording_segments::{Lane, Prefix};
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub session_id: String,
    pub meeting_id: String,
    pub title: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub items: Vec<Item>,
    pub next: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResultItem {
    pub title: String,
    pub summary: Summary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResultListing {
    pub items: Vec<ResultItem>,
    pub next: Option<String>,
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

impl Review {
    /// Stable input evidence for a qualified native move, not IPC authority.
    /// Fresh Summary comparison and explicit consent remain mandatory.
    pub(super) fn content_hash(&self) -> Result<String, String> {
        let generations = self
            .prefixes
            .iter()
            .map(Prefix::content_hash)
            .collect::<Result<Vec<_>, _>>()?;
        let proof = serde_json::to_vec(&serde_json::json!({"version":1,
            "session":self.session,"config":self.config_text,
            "meetingVersion":self.meeting_version,"meetingState":self.meeting_state,
            "origin":self.origin,"lanes":self.summary.lanes,"generations":generations}))
        .map_err(|e| e.to_string())?;
        Ok(format!("{:x}", Sha256::digest(&proof)))
    }
    pub(super) fn input_hash(&self) -> Result<String, String> {
        input_hash(&self.summary.review_hash, self.origin, &self.prefixes)
    }
    pub(super) fn document(&self) -> serde_json::Value {
        serde_json::json!({"schemaVersion":crate::current_formats::CAPTURE,"inputDiagnosticsState":"unavailable","state":"interrupted",
            "captureMode":self.config.mode,"backend":self.config.backend,
            "targetPIDs":self.config.target_pids,"recordingOwnerId":self.session.id,
            "errors":[],"partialRecovery":{
                "schemaVersion":1,"kind":"sealed_prefix",
                "reviewHash":self.summary.review_hash,"acquisitionEndKnown":false,
                "tailLossUs":null,"timelineUtcOriginKnown":false,
                "originalState":self.session.state,
                "originalObservedEndUtcMs":self.session.ended_at_utc_ms,
                "originalDurationUs":self.session.duration_us,
                "timelineOriginNanoseconds":self.origin,"lanes":self.summary.lanes}})
    }
}
pub(super) fn input_hash(review: &str, origin: i64, prefixes: &[Prefix]) -> Result<String, String> {
    let proof = serde_json::to_vec(&serde_json::json!({
        "version":1,"mode":"sealed_prefix","review":review,"origin":origin,
        "lanes":prefixes.iter().map(|p|p.lane().name()).collect::<Vec<_>>()
    }))
    .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(&proof)))
}

impl Storage {
    /// DB-owned unresolved native sessions only; never scan the filesystem or
    /// turn restored provenance into authority. Pagination never hides a tail.
    pub(crate) fn partial_recording_listing(&self, after: Option<&str>) -> Result<Listing, String> {
        self.managed_anchor.verify()?;
        if after.is_some_and(|id| !crate::input_levels::valid_owner(id)) {
            return Err("部分救出一覧の続き位置が不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = db.prepare("SELECT r.id,r.meeting_id,m.title,r.state FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.state IN ('interrupted','failed') AND r.audio_manifest_hash IS NULL AND r.input_history_snapshot_json IS NULL AND m.source_type='recording' AND m.lifecycle IN ('recording','draft') AND (?1 IS NULL OR r.id>?1) ORDER BY r.id LIMIT 201").map_err(|e|e.to_string())?;
        let mut items = query
            .query_map([after], |row| {
                Ok(Item {
                    session_id: row.get(0)?,
                    meeting_id: row.get(1)?,
                    title: row.get(2)?,
                    state: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let more = items.len() > 200;
        items.truncate(200);
        let next = more.then(|| items.last().unwrap().session_id.clone());
        Ok(Listing { items, next })
    }

    /// Durable result, not new recovery authority. This remains readable after
    /// restart/restore or intentional audio deletion and says only what commit
    /// recorded, not whether the current bodies are still playable.
    pub(crate) fn partial_recovery_result(
        &self,
        session_id: &str,
        review_hash: &str,
    ) -> Result<Option<Summary>, String> {
        self.managed_anchor.verify()?;
        if !crate::input_levels::valid_owner(session_id) || !valid_result_hash(review_hash) {
            return Err("部分救出の結果照合条件が不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        read_result(&db, session_id, Some(review_hash))
    }

    /// Startup notification/history only, including restored logical records
    /// and intentionally deleted audio. Never mints a native review or reads
    /// original PCM. A listed commit must validate, rather than silently vanish.
    pub(crate) fn partial_recovery_listing(
        &self,
        after: Option<&str>,
    ) -> Result<ResultListing, String> {
        self.managed_anchor.verify()?;
        if after.is_some_and(|id| !crate::input_levels::valid_owner(id)) {
            return Err("保存済み部分救出の続き位置が不正です".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = db.prepare("SELECT r.id,m.title FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE EXISTS(SELECT 1 FROM audio_tracks t WHERE t.session_id=r.id AND t.kind IN ('mic','system') AND json_type(t.processing_config_json,'$.partial_recovery') IS NOT NULL) AND (?1 IS NULL OR r.id>?1) ORDER BY r.id LIMIT 201").map_err(|e|e.to_string())?;
        let rows = query
            .query_map([after], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let more = rows.len() > 200;
        let mut items = Vec::new();
        for (id, title) in rows.into_iter().take(200) {
            if !crate::input_levels::valid_owner(&id) {
                return Err("保存済み部分救出の録音IDが不正です".into());
            }
            let summary = read_result(&db, &id, None)?
                .ok_or("保存済み部分救出の登録・範囲を確認できません")?;
            items.push(ResultItem { title, summary });
        }
        let next = more.then(|| items.last().unwrap().summary.session_id.clone());
        Ok(ResultListing { items, next })
    }

    #[cfg(test)]
    pub(crate) fn review_partial_recording(&self, session_id: &str) -> Result<Review, String> {
        self.review_partial_recording_checked(session_id, &crate::test_support::never_cancelled)
    }

    pub(crate) fn review_partial_recording_checked(
        &self,
        session_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Review, String> {
        self.require_data_writable()?;
        let review = self.inspect_partial_recording(session_id, cancelled)?;
        self.require_data_writable()?;
        Ok(review)
    }

    /// Read-only recapture across source retirement, only behind the held
    /// native snapshot. Public review/recovery still requires writable data.
    #[cfg(target_os = "macos")]
    pub(super) fn inspect_partial_for_relocation(
        &self,
        snapshot: &super::super::backup::relocation::NativeSnapshot<'_>,
        session_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Review, String> {
        if self.root != snapshot.source_root() {
            return Err("部分救出の検査先と保持中の移動元が一致しません".into());
        }
        snapshot.verify_source(cancelled)?;
        let review = self.inspect_partial_recording(session_id, cancelled)?;
        snapshot.verify_source(cancelled)?;
        Ok(review)
    }

    fn inspect_partial_recording(
        &self,
        session_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Review, String> {
        check_cancel(cancelled)?;
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
                cancelled,
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
                Err(_) => {
                    check_cancel(cancelled)?;
                    "unreadable"
                }
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
            check_cancel(cancelled)?;
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
        self.managed_anchor.verify()?;
        for prefix in &prefixes {
            prefix.verify_at(&self.root.join(&base), cancelled)?;
        }
        check_cancel(cancelled)?;
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

    #[cfg(test)]
    pub(crate) fn recover_partial_recording(
        &self,
        reviewed: &Review,
        confirmed: bool,
    ) -> Result<RecordingSession, String> {
        self.recover_partial_recording_checked(
            reviewed,
            confirmed,
            &crate::test_support::never_cancelled,
        )
    }

    pub(crate) fn recover_partial_recording_checked(
        &self,
        reviewed: &Review,
        confirmed: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<RecordingSession, String> {
        check_cancel(cancelled)?;
        if !confirmed {
            return Err("確定済み部分だけを救出する明示的な同意が必要です".into());
        }
        let current = self.review_partial_recording_checked(&reviewed.session.id, cancelled)?;
        if current.summary != reviewed.summary {
            return Err("部分救出の対象が確認後に変更されました。再確認してください".into());
        }
        drop(current);
        if reviewed.prefixes.is_empty() || reviewed.summary.duration_us <= 0 {
            return Err("部分救出できる確定済み音声がありません".into());
        }
        let document = reviewed.document();
        let mut publication = flac::Publication::prepare_recovery(
            self,
            &reviewed.base,
            &reviewed.summary.review_hash,
            &reviewed.content_hash()?,
            &document,
            reviewed.origin,
            &reviewed.prefixes,
            cancelled,
        )?;
        let recovery = &document["partialRecovery"];
        let manifest = publication.recovery_document(self, &document)?;
        let config =
            serde_json::json!({"timeline_alignment":"unverified","partial_recovery":recovery})
                .to_string();
        // Re-inspect both lanes after conversion: newly available or changed
        // evidence must not be included, or omitted, under an old confirmation.
        if self
            .review_partial_recording_checked(&reviewed.session.id, cancelled)?
            .summary
            != reviewed.summary
        {
            return Err("部分救出の対象が変換中に変更されました".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        check_cancel(cancelled)?;
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
            prefix.verify_at(&self.root.join(&reviewed.base), cancelled)?;
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
            prefix.verify_at(&self.root.join(&reviewed.base), cancelled)?;
        }
        publication.verify(self)?;
        self.require_data_writable()?;
        check_cancel(cancelled)?;
        let result =
            load_session(&tx, &reviewed.session.id)?.ok_or("部分救出の結果がありません")?;
        #[cfg(test)]
        flac::crash_point("partial-before-db-commit");
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

fn valid_result_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read_result(
    db: &Connection,
    session_id: &str,
    expected_hash: Option<&str>,
) -> Result<Option<Summary>, String> {
    let Some(session) = load_session(db, session_id)? else {
        return Ok(None);
    };
    if !crate::input_levels::valid_owner(&session.meeting_id) {
        return Err("保存済み部分救出の会議IDが不正です".into());
    }
    let mut query = db.prepare("SELECT kind,sample_rate,channels,CASE WHEN length(CAST(processing_config_json AS BLOB))<=4194304 THEN processing_config_json ELSE NULL END FROM audio_tracks WHERE session_id=?1 AND kind IN ('mic','system') ORDER BY kind LIMIT 3").map_err(|e|e.to_string())?;
    let tracks = query
        .query_map([session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if tracks.is_empty() {
        return Ok(None);
    }
    if tracks.len() > 2 || tracks.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("保存済み部分救出の入力数が不正です".into());
    }
    let configs = tracks
        .iter()
        .map(|t| {
            let text =
                t.3.as_deref()
                    .ok_or("保存済み部分救出の記録上限を超えています")?;
            serde_json::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let recovery = &configs[0]["partial_recovery"];
    let Some(hash) = recovery["reviewHash"]
        .as_str()
        .filter(|v| valid_result_hash(v))
    else {
        return Ok(None);
    };
    if expected_hash.is_some_and(|expected| expected != hash) {
        return Ok(None);
    }
    if !super::super::transcription::partial_recording_committed(db, &session.meeting_id)? {
        return Ok(None);
    }
    if configs
        .iter()
        .any(|value| &value["partial_recovery"] != recovery)
    {
        return Err("保存済み部分救出の入力ごとの記録が一致しません".into());
    }
    let lanes: Vec<LaneReview> =
        serde_json::from_value(recovery["lanes"].clone()).map_err(|e| e.to_string())?;
    let summary = Summary {
        session_id: session.id,
        meeting_id: session.meeting_id,
        review_hash: hash.into(),
        duration_us: session.duration_us,
        lanes,
        acquisition_end_known: false,
        tail_loss_us: None,
    };
    summary.validate()?;
    if summary
        .lanes
        .iter()
        .filter(|lane| lane.state == "available")
        .count()
        != tracks.len()
        || tracks.iter().any(|(kind, rate, channels, _)| {
            !summary.lanes.iter().any(|lane| {
                lane.state == "available"
                    && lane.lane
                        == if kind == "mic" {
                            "microphone"
                        } else {
                            "system"
                        }
                    && lane.sample_rate == Some(*rate)
                    && lane.channels == Some(*channels)
            })
        })
    {
        return Err("保存済み部分救出の入力・形式が登録と一致しません".into());
    }
    Ok(Some(summary))
}

impl Summary {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.lanes.len() != 2
            || self.lanes[0].lane != "microphone"
            || self.lanes[1].lane != "system"
            || self.acquisition_end_known
            || self.tail_loss_us.is_some()
            || self.duration_us < 0
        {
            return Err("部分救出の範囲・取得終了記録が不正です".into());
        }
        let mut earliest = None;
        let mut extent = 0;
        for lane in &self.lanes {
            if lane.state == "available" {
                let start = lane.start_us.ok_or("部分救出の始端がありません")?;
                let end = lane.end_us.ok_or("部分救出の終端がありません")?;
                if start < 0
                    || end <= start
                    || lane.sealed_segments == 0
                    || lane.sealed_segments > 32768
                    || lane.frames == 0
                    || lane.frames > 9_007_199_254_740_991
                    || !lane
                        .sample_rate
                        .is_some_and(|v| (8000..=384000).contains(&v))
                    || !lane.channels.is_some_and(|v| (1..=8).contains(&v))
                {
                    return Err("部分救出の測定範囲が不正です".into());
                }
                earliest = Some(earliest.map_or(start, |old: i64| old.min(start)));
                extent = extent.max(end);
            } else if !matches!(lane.state.as_str(), "empty" | "missing" | "unreadable")
                || lane.sealed_segments != 0
                || lane.frames != 0
                || lane.start_us.is_some()
                || lane.end_us.is_some()
                || lane.sample_rate.is_some()
                || lane.channels.is_some()
            {
                return Err("検証できない入力に測定範囲があります".into());
            }
        }
        if earliest.is_some_and(|start| start != 0)
            || extent != self.duration_us
            || self.duration_us > 9_007_199_254_740_991
        {
            return Err("部分救出の時間軸が不正です".into());
        }
        Ok(())
    }
}

pub(super) fn check_cancel(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("部分救出を安全な区切りで停止しました。原音声と途中出力は保持します".into())
    } else {
        Ok(())
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
