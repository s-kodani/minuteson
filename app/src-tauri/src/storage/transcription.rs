use super::*;
use crate::audio_clock::{ClockChunk, ClockLayout, ClockSegment};
use serde::Deserialize;
use std::collections::BTreeMap;
pub(crate) mod known;

#[cfg(all(test, unix))]
#[path = "transcription_tests.rs"]
pub(super) mod tests;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RecordedAudioPlan {
    pub manifest_asset_id: String,
    pub manifest_hash: String,
    pub tracks: Vec<TrackPlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TrackPlan {
    pub id: String,
    pub kind: String,
    pub start_us: i64,
    pub end_us: i64,
    pub sample_rate: u32,
    pub channels: u32,
    pub chunks: Vec<ChunkPlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChunkPlan {
    pub asset_id: String,
    pub hash: String,
    pub size: i64,
    pub start_us: i64,
    pub end_us: i64,
    pub frames: i64,
    pub sample_rate: u32,
    pub channels: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) clock_segments: Vec<ClockSegment>,
}

pub(super) fn read_clock_segments(
    db: &Connection,
    track: &str,
) -> Result<BTreeMap<String, Vec<ClockSegment>>, String> {
    // Do not allocate an arbitrarily large archived JSON string before checking
    // the bound. SQLite tests UTF-8 byte length and returns only a bounded value.
    let mut query = db.prepare("SELECT chunk_id, source_frame_start, source_frame_end, timeline_start_us, timeline_end_us, CASE WHEN length(CAST(mapping_json AS BLOB)) <= 4096 THEN mapping_json ELSE '' END, length(CAST(mapping_json AS BLOB)) <= 4096 FROM track_clock_segments WHERE track_id=?1 ORDER BY chunk_id, source_frame_start, id LIMIT 65537").map_err(|e| e.to_string())?;
    let mut rows = query.query(params![track]).map_err(|e| e.to_string())?;
    let mut segments = BTreeMap::<String, Vec<ClockSegment>>::new();
    let (mut count, mut bytes) = (0_usize, 0_usize);
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > crate::audio_clock::MAX_CLOCK_SEGMENTS {
            return Err("時刻補正の対応区間が多すぎます".into());
        }
        if !row.get::<_, bool>(6).map_err(|e| e.to_string())? {
            return Err("時刻補正の形式情報が容量上限を超えています".into());
        }
        let chunk: String = row.get(0).map_err(|e| e.to_string())?;
        let segment = ClockSegment {
            source_frame_start: row.get(1).map_err(|e| e.to_string())?,
            source_frame_end: row.get(2).map_err(|e| e.to_string())?,
            timeline_start_us: row.get(3).map_err(|e| e.to_string())?,
            timeline_end_us: row.get(4).map_err(|e| e.to_string())?,
            mapping_json: row.get(5).map_err(|e| e.to_string())?,
        };
        bytes = bytes
            .checked_add(segment.mapping_json.len())
            .filter(|bytes| *bytes <= crate::audio_clock::MAX_CLOCK_MAPPING_BYTES)
            .ok_or("時刻補正の形式情報の合計容量が上限を超えています")?;
        let chunk_maps = segments.entry(chunk).or_default();
        if chunk_maps.len() >= crate::audio_clock::MAX_CLOCK_SEGMENTS {
            return Err("時刻補正の区間内対応数が上限を超えています".into());
        }
        chunk_maps.push(segment);
    }
    Ok(segments)
}

impl TrackPlan {
    pub(crate) fn clock_layout(&self) -> Result<ClockLayout, String> {
        if self
            .chunks
            .iter()
            .any(|chunk| chunk.sample_rate != self.sample_rate || chunk.channels != self.channels)
        {
            return Err("時刻補正のトラックと音声区間の形式が一致しません".into());
        }
        let chunks = self
            .chunks
            .iter()
            .map(|chunk| ClockChunk {
                frames: chunk.frames,
                start_us: chunk.start_us,
                end_us: chunk.end_us,
                segments: &chunk.clock_segments,
            })
            .collect::<Vec<_>>();
        let layout = if chunks.len() == 1 && matches!(self.kind.as_str(), "mic" | "system") {
            let c = &chunks[0];
            ClockLayout::capture_track(
                ClockChunk {
                    frames: c.frames,
                    start_us: c.start_us,
                    end_us: c.end_us,
                    segments: c.segments,
                },
                self.sample_rate,
            )?
        } else {
            ClockLayout::new(&chunks, self.sample_rate)?
        };
        if self.start_us != layout.start_us || self.end_us != layout.end_us {
            return Err("保存トラックと取得時計の端点が一致しません".into());
        }
        Ok(layout)
    }
    pub(crate) fn total_frames(&self) -> Result<u64, String> {
        if self
            .chunks
            .iter()
            .any(|chunk| !chunk.clock_segments.is_empty())
        {
            if self.chunks.iter().any(|chunk| {
                chunk.sample_rate != self.sample_rate || chunk.channels != self.channels
            }) {
                return Err("時刻補正のトラックと音声区間の形式が一致しません".into());
            }
            let layout = self.clock_layout()?;
            let (first, last) = layout.range(self.start_us, self.end_us, self.sample_rate)?;
            Ok(last - first)
        } else {
            self.chunks
                .iter()
                .try_fold(0_u64, |sum, chunk| {
                    sum.checked_add(u64::try_from(chunk.frames).ok()?)
                })
                .ok_or("音声フレーム数が不正です".into())
        }
    }
}

impl Storage {
    pub(crate) fn recorded_audio_plan(
        &self,
        meeting_id: &str,
    ) -> Result<RecordedAudioPlan, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        read_audio_plan(&transaction, meeting_id)
    }
}

/// Read fixed audio metadata from one consistent DB view. File verification is
/// separate, so the same metadata checks can be used in private restore staging.
pub(super) fn read_audio_plan(
    transaction: &Connection,
    meeting_id: &str,
) -> Result<RecordedAudioPlan, String> {
    super::deletion::audio::require_source(transaction, meeting_id)?;
    super::meeting_audio::read(transaction, meeting_id)?
        .ok_or("共通WAVの保存が完了していません".into())
}

/// Restore-only metadata, not playable input or permission to enqueue work.
/// The capability has no public constructor and rechecks all audio row proofs
/// before reading. Missing flags or an untrusted manifest alone do not suffice.
pub(super) fn read_historical_audio_plan(
    transaction: &Connection,
    meeting_id: &str,
    history: &super::backup::DeletedAudioHistory,
) -> Result<RecordedAudioPlan, String> {
    history.verify_meeting(transaction, meeting_id)?;
    super::meeting_audio::read(transaction, meeting_id)?.ok_or("共通WAVの履歴がありません".into())
}

pub(super) fn read_analysis_audio_plan(
    db: &Connection,
    meeting: &str,
    historical: Option<&super::backup::DeletedAudioHistory>,
) -> Result<RecordedAudioPlan, String> {
    let mixed = if let Some(history) = historical {
        super::meeting_mix::read_historical(db, meeting, history)?
    } else {
        super::deletion::audio::require_source(db, meeting)?;
        super::meeting_mix::read(db, meeting)?
    };
    if let Some(plan) = mixed {
        return Ok(plan);
    }
    Err("共通WAVの保存・移行が完了していません".into())
}

/// An interrupted session is playable only after the explicit, atomic prefix
/// recovery commit. Every native track must carry the same recovery receipt;
/// one marked track must not authorize another unconfirmed input. This gate
/// also works for restored/deleted-audio metadata without native paths.
pub(super) fn partial_recording_committed(db: &Connection, meeting: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id
        WHERE r.meeting_id=?1 AND r.state='interrupted' AND r.ended_at_utc_ms IS NULL
        AND r.duration_us>0 AND r.last_committed_us=r.duration_us AND m.duration_us=r.duration_us
        AND m.source_type='recording' AND EXISTS(SELECT 1 FROM file_assets f
            WHERE f.meeting_id=r.meeting_id AND f.content_hash=r.audio_manifest_hash
            AND f.media_type='application/json' AND f.state='ready')
        AND EXISTS(SELECT 1 FROM audio_tracks t WHERE t.meeting_id=r.meeting_id AND t.session_id=r.id AND t.kind IN ('mic','system'))
        AND NOT EXISTS(SELECT 1 FROM audio_tracks t WHERE t.meeting_id=r.meeting_id AND t.kind IN ('mic','system')
            AND (t.session_id IS NOT r.id
                OR json_extract(t.processing_config_json,'$.partial_recovery.schemaVersion') IS NOT 1
                OR json_extract(t.processing_config_json,'$.partial_recovery.kind') IS NOT 'sealed_prefix'
                OR json_extract(t.processing_config_json,'$.partial_recovery.acquisitionEndKnown') IS NOT 0
                OR json_extract(t.processing_config_json,'$.partial_recovery.timelineUtcOriginKnown') IS NOT 0
                OR json_type(t.processing_config_json,'$.partial_recovery.tailLossUs') IS NOT 'null'
                OR json_type(t.processing_config_json,'$.partial_recovery.reviewHash') IS NOT 'text'
                OR length(json_extract(t.processing_config_json,'$.partial_recovery.reviewHash')) != 64
                OR json_extract(t.processing_config_json,'$.partial_recovery.reviewHash') GLOB '*[^0-9a-f]*'))
        AND (SELECT count(DISTINCT json_extract(t.processing_config_json,'$.partial_recovery.reviewHash'))
            FROM audio_tracks t WHERE t.meeting_id=r.meeting_id AND t.kind IN ('mic','system'))=1)",
        [meeting], |row|row.get(0),
    ).map_err(|e|e.to_string())
}

impl Storage {
    pub(crate) fn analysis_unit_directory(
        &self,
        meeting_id: &str,
        job_id: &str,
        index: usize,
    ) -> Result<PathBuf, String> {
        // Validate IDs against the DB and the managed path before creating files.
        if self.get_job(job_id)?.meeting_id != meeting_id {
            return Err("別会議の処理領域は作成できません".into());
        }
        let relative = PathBuf::from("meetings")
            .join(meeting_id)
            .join("analysis")
            .join(job_id)
            .join(format!("unit-{index}-{}", Uuid::new_v4()));
        let path = self.managed_path(&relative)?;
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let meeting = root.join("meetings").join(meeting_id);
        if meeting.canonicalize().map_err(|e| e.to_string())?
            != root.join("meetings").join(meeting_id)
        {
            return Err("解析領域が管理領域外を参照しています".into());
        }
        // Check every existing parent before create_dir_all (which follows links).
        for parent in [
            meeting.join("analysis"),
            meeting.join("analysis").join(job_id),
        ] {
            if parent.symlink_metadata().is_ok()
                && parent.canonicalize().map_err(|e| e.to_string())? != parent
            {
                return Err("解析領域にシンボリックリンクがあります".into());
            }
        }
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if canonical != root.join(relative) {
            return Err("解析領域が管理領域外を参照しています".into());
        }
        Ok(canonical)
    }

    pub(crate) fn register_analysis_artifact(
        &self,
        meeting_id: &str,
        path: &Path,
        media_type: &str,
    ) -> Result<FileAsset, String> {
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "解析結果が管理領域内にありません")?;
        self.register_ready_meeting_asset(meeting_id, relative, media_type)
    }

    pub(crate) fn verify_meeting_asset(
        &self,
        meeting_id: &str,
        asset_id: &str,
        hash: &str,
    ) -> Result<(), String> {
        let valid: bool = self.connection.lock().map_err(|_| "DBロックを取得できません")?.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE id = ?1 AND meeting_id = ?2 AND content_hash = ?3 AND state = 'ready')", params![asset_id, meeting_id, hash], |row| row.get(0)).map_err(|e| e.to_string())?;
        if !valid || !self.verify_asset(asset_id)? {
            return Err("解析入力・結果ファイルの参照またはハッシュが一致しません".into());
        }
        Ok(())
    }

    pub(crate) fn verify_meeting_asset_cancellable(
        &self,
        meeting_id: &str,
        asset_id: &str,
        hash: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let read = || -> Result<(String, i64), String> {
            self.connection.lock().map_err(|_| "DBロックを取得できません")?.query_row(
                "SELECT relative_path,size_bytes FROM file_assets WHERE id=?1 AND meeting_id=?2 AND content_hash=?3 AND state='ready'",
                params![asset_id, meeting_id, hash], |r| Ok((r.get(0)?, r.get(1)?)),
            ).map_err(|e| e.to_string())
        };
        let expected = read()?;
        let path = self.managed_path(Path::new(&expected.0))?;
        let proof = crate::local_engine::Fingerprint::capture(&path, cancelled)?;
        if proof.path
            != self
                .root
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(&expected.0)
            || u64::try_from(expected.1).ok() != Some(proof.size)
            || proof.sha256 != hash
            || read()? != expected
            || cancelled()
        {
            return Err("解析入力・結果ファイルの参照またはハッシュが一致しません".into());
        }
        Ok(())
    }

    pub(crate) fn read_analysis_result(
        &self,
        meeting_id: &str,
        asset_id: &str,
        hash: &str,
    ) -> Result<Vec<u8>, String> {
        self.read_inference_artifact(
            meeting_id,
            asset_id,
            hash,
            "application/json",
            8 * 1024 * 1024,
        )
    }

    pub(crate) fn read_inference_artifact(
        &self,
        meeting_id: &str,
        asset_id: &str,
        hash: &str,
        media_type: &str,
        limit: u64,
    ) -> Result<Vec<u8>, String> {
        let maximum = if media_type == "text/plain" {
            16 * 1024 * 1024
        } else {
            8 * 1024 * 1024
        };
        if limit == 0 || limit > maximum {
            return Err("解析結果の読出し上限が不正です".into());
        }
        let relative: String = self.connection.lock().map_err(|_| "DBロックを取得できません")?.query_row("SELECT relative_path FROM file_assets WHERE id = ?1 AND meeting_id = ?2 AND content_hash = ?3 AND state = 'ready' AND media_type = ?4 AND size_bytes <= ?5", params![asset_id, meeting_id, hash, media_type, limit as i64], |row| row.get(0)).map_err(|e| e.to_string())?;
        self.verify_meeting_asset(meeting_id, asset_id, hash)?;
        let path = self.managed_path(Path::new(&relative))?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit || format!("{:x}", Sha256::digest(&bytes)) != hash {
            return Err("解析結果が読込中に変更されています".into());
        }
        Ok(bytes)
    }
}
