use super::*;
use serde::Deserialize;

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
        let (manifest_asset_id, manifest_hash): (String, String) = transaction.query_row(
            "SELECT f.id, r.audio_manifest_hash FROM recording_sessions r JOIN meetings m ON m.id = r.meeting_id JOIN file_assets f ON f.meeting_id = r.meeting_id AND f.content_hash = r.audio_manifest_hash AND f.media_type = 'application/json' AND f.state = 'ready' WHERE r.meeting_id = ?1 AND r.state = 'saved' AND m.lifecycle = 'recorded' ORDER BY f.id LIMIT 1",
            params![meeting_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|e| e.to_string())?.ok_or("解析可能な保存済み録音manifestがありません")?;
        let mut query = transaction.prepare("SELECT id, kind, sample_rate, channels FROM audio_tracks WHERE meeting_id = ?1 AND state = 'ready' AND kind IN ('mic', 'system') ORDER BY kind, id").map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![meeting_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let tracks = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut plans = Vec::new();
        for (id, kind, sample_rate, channels) in tracks {
            let mut query = transaction.prepare("SELECT f.id, f.content_hash, f.size_bytes, c.start_us, c.end_us, c.frame_count, c.sample_rate, c.channels FROM audio_chunks c JOIN file_assets f ON f.id = c.file_asset_id AND f.meeting_id = c.meeting_id WHERE c.track_id = ?1 AND c.meeting_id = ?2 AND c.state = 'ready' AND f.state = 'ready' ORDER BY c.start_us, c.sequence").map_err(|e| e.to_string())?;
            let rows = query
                .query_map(params![id, meeting_id], |row| {
                    Ok(ChunkPlan {
                        asset_id: row.get(0)?,
                        hash: row.get(1)?,
                        size: row.get(2)?,
                        start_us: row.get(3)?,
                        end_us: row.get(4)?,
                        frames: row.get(5)?,
                        sample_rate: row.get(6)?,
                        channels: row.get(7)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            let chunks = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if chunks.is_empty() || chunks.len() > 1024 || sample_rate == 0 || channels == 0 {
                return Err("解析入力トラックの区間または形式が不正です".into());
            }
            let start_us = chunks[0].start_us;
            let end_us = chunks.last().unwrap().end_us;
            plans.push(TrackPlan {
                id,
                kind,
                start_us,
                end_us,
                sample_rate,
                channels,
                chunks,
            });
        }
        if plans.is_empty() || plans.len() > 2 {
            return Err("解析対象の録音トラックがありません".into());
        }
        Ok(RecordedAudioPlan {
            manifest_asset_id,
            manifest_hash,
            tracks: plans,
        })
    }

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
