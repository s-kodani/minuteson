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
        if chunk_maps.len() >= 1024 {
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
        let layout = ClockLayout::new(
            &self
                .chunks
                .iter()
                .map(|chunk| ClockChunk {
                    frames: chunk.frames,
                    start_us: chunk.start_us,
                    end_us: chunk.end_us,
                    segments: &chunk.clock_segments,
                })
                .collect::<Vec<_>>(),
            self.sample_rate,
        )?;
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
            let layout = ClockLayout::new(
                &self
                    .chunks
                    .iter()
                    .map(|chunk| ClockChunk {
                        frames: chunk.frames,
                        start_us: chunk.start_us,
                        end_us: chunk.end_us,
                        segments: &chunk.clock_segments,
                    })
                    .collect::<Vec<_>>(),
                self.sample_rate,
            )?;
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
    #[cfg(unix)]
    super::deletion::audio::require_source(transaction, meeting_id)?;
    read_audio_metadata(
        transaction,
        meeting_id,
        AudioMetadataState::Available,
        false,
    )
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
    read_audio_metadata(transaction, meeting_id, AudioMetadataState::Deleted, false)
}

pub(super) fn read_analysis_audio_plan(
    db: &Connection,
    meeting: &str,
    historical: Option<&super::backup::DeletedAudioHistory>,
) -> Result<RecordedAudioPlan, String> {
    let state = if let Some(history) = historical {
        history.verify_meeting(db, meeting)?;
        AudioMetadataState::Deleted
    } else {
        #[cfg(unix)]
        super::deletion::audio::require_source(db, meeting)?;
        AudioMetadataState::Available
    };
    read_audio_metadata(db, meeting, state, true)
}

enum AudioMetadataState {
    Available,
    Deleted,
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

fn read_audio_metadata(
    transaction: &Connection,
    meeting_id: &str,
    state: AudioMetadataState,
    known_runs: bool,
) -> Result<RecordedAudioPlan, String> {
    let (track_state, chunk_state) = match state {
        AudioMetadataState::Available => ("ready", "ready"),
        AudioMetadataState::Deleted => ("failed", "missing"),
    };
    // Keep the serialized plan shape for old jobs; both source types use
    // verified managed manifests, never an external original path.
    let partial = partial_recording_committed(transaction, meeting_id)?;
    let (manifest_asset_id, manifest_hash, source_kind): (String, String, String) = transaction.query_row(
            "SELECT f.id, r.audio_manifest_hash, 'recording' FROM recording_sessions r JOIN meetings m ON m.id = r.meeting_id JOIN file_assets f ON f.meeting_id = r.meeting_id AND f.content_hash = r.audio_manifest_hash AND f.media_type = 'application/json' AND f.state = 'ready' WHERE r.meeting_id = ?1 AND (r.state = 'saved' OR (r.state='interrupted' AND ?2)) AND m.lifecycle = 'recorded' AND m.source_type='recording' UNION ALL SELECT f.id,p.manifest_hash,'audio_import' FROM audio_import_commits p JOIN audio_imports i ON i.id=p.import_id AND i.meeting_id=p.meeting_id JOIN meetings m ON m.id=p.meeting_id JOIN file_assets f ON f.id=p.manifest_asset_id AND f.meeting_id=p.meeting_id AND f.content_hash=p.manifest_hash AND f.media_type='application/json' AND f.state='ready' WHERE p.meeting_id=?1 AND i.state='ready' AND m.lifecycle='imported' AND m.source_type='audio_import' ORDER BY 1 LIMIT 1",
            params![meeting_id,partial], |row| Ok((row.get(0)?, row.get(1)?,row.get(2)?)),
        ).optional().map_err(|e| e.to_string())?.ok_or("解析可能な保存済み音声manifestがありません")?;
    let mut query = transaction.prepare("SELECT id, kind, sample_rate, channels FROM audio_tracks WHERE meeting_id = ?1 AND state = ?3 AND ((?2='recording' AND kind IN ('mic','system') AND session_id IN (SELECT id FROM recording_sessions WHERE meeting_id=?1 AND (state='saved' OR (state='interrupted' AND ?4)))) OR (?2='audio_import' AND kind='imported' AND session_id IS NULL AND import_id IN (SELECT id FROM audio_imports WHERE meeting_id=?1 AND state='ready'))) ORDER BY kind, id").map_err(|e| e.to_string())?;
    let rows = query
        .query_map(
            params![meeting_id, source_kind, track_state, partial],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let tracks = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut plans = Vec::new();
    for (id, kind, sample_rate, channels) in tracks {
        let mut mappings = read_clock_segments(transaction, &id)?;
        let mut query = transaction.prepare("SELECT f.id, f.content_hash, f.size_bytes, c.start_us, c.end_us, c.frame_count, c.sample_rate, c.channels, c.id FROM audio_chunks c JOIN file_assets f ON f.id = c.file_asset_id AND f.meeting_id = c.meeting_id WHERE c.track_id = ?1 AND c.meeting_id = ?2 AND c.state = ?3 AND f.state = ?3 ORDER BY c.start_us, c.sequence LIMIT 1025").map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![id, meeting_id, chunk_state], |row| {
                Ok(ChunkPlan {
                    asset_id: row.get(0)?,
                    hash: row.get(1)?,
                    size: row.get(2)?,
                    start_us: row.get(3)?,
                    end_us: row.get(4)?,
                    frames: row.get(5)?,
                    sample_rate: row.get(6)?,
                    channels: row.get(7)?,
                    clock_segments: mappings
                        .remove(&row.get::<_, String>(8)?)
                        .unwrap_or_default(),
                })
            })
            .map_err(|e| e.to_string())?;
        let chunks = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if chunks.is_empty()
            || chunks.len() > 1024
            || sample_rate == 0
            || channels == 0
            || !mappings.is_empty()
        {
            return Err("解析入力トラックの区間または形式が不正です".into());
        }
        let start_us = chunks[0].start_us;
        let end_us = chunks.last().unwrap().end_us;
        let plan = TrackPlan {
            id,
            kind,
            start_us,
            end_us,
            sample_rate,
            channels,
            chunks,
        };
        if known_runs {
            plan.clock_layout()?;
            // Missing/corrupt rows cannot be hidden by filtering available rows.
            let mut query = transaction.prepare("SELECT sequence FROM audio_chunks WHERE track_id=?1 AND meeting_id=?2 ORDER BY start_us,sequence LIMIT 1025").map_err(|e|e.to_string())?;
            let sequences = query
                .query_map(params![plan.id, meeting_id], |r| r.get::<_, i64>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if sequences.len() != plan.chunks.len()
                || sequences
                    .iter()
                    .enumerate()
                    .any(|(index, sequence)| *sequence != index as i64)
            {
                return Err("解析入力の保存ファイルに欠損または不正な連番があります".into());
            }
        } else {
            plan.total_frames()?;
        }
        plans.push(plan);
    }
    if plans.is_empty() || plans.len() > 2 || (source_kind == "audio_import" && plans.len() != 1) {
        return Err("解析対象の保存済み音声トラックが不正です".into());
    }
    if known_runs {
        let count:i64=transaction.query_row("SELECT count(*) FROM audio_tracks WHERE meeting_id=?1 AND ((?2='recording' AND kind IN ('mic','system')) OR (?2='audio_import' AND kind='imported'))",params![meeting_id,source_kind],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count != plans.len() as i64 {
            return Err("解析対象の入力が未確定・破損・欠落しています".into());
        }
    }
    Ok(RecordedAudioPlan {
        manifest_asset_id,
        manifest_hash,
        tracks: plans,
    })
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
