//! The single canonical source. No dependency on retired input files.
use super::transcription::{ChunkPlan, RecordedAudioPlan, TrackPlan};
use super::*;

pub(crate) const POLICY: &str = "meeting_wav_pcm16_16000_v1";

/// Historical input bytes intentionally retired after verified publication.
/// A missing file alone never authorizes this omission.
pub(in crate::storage) fn superseded_asset(db: &Connection, id: &str) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets f JOIN meeting_audio_commits a ON a.meeting_id=f.meeting_id
        JOIN audio_chunks c ON c.track_id=a.track_id JOIN json_each(a.cleanup_json) j
        WHERE f.id=?1 AND f.state='missing' AND f.media_type LIKE 'audio/%' AND a.phase='cleaned'
        AND c.file_asset_id!=f.id AND json_extract(j.value,'$.hash')=f.content_hash AND json_extract(j.value,'$.bytes')=f.size_bytes)",
        [id],|r|r.get(0)).map_err(|e|e.to_string())
}

pub(in crate::storage) fn read(
    db: &Connection,
    meeting: &str,
) -> Result<Option<RecordedAudioPlan>, String> {
    type Row = (String, String, String, i64, String, String, i64, i64, i64);
    let row: Option<Row> = db.query_row(
        "SELECT a.track_id,a.manifest_asset_id,a.manifest_hash,a.frames,f.id,f.content_hash,f.size_bytes,c.end_us,c.frame_count
         FROM meeting_audio_commits a JOIN audio_tracks t ON t.id=a.track_id AND t.meeting_id=a.meeting_id
         JOIN audio_chunks c ON c.track_id=t.id AND c.meeting_id=t.meeting_id
         JOIN file_assets f ON f.id=c.file_asset_id AND f.meeting_id=c.meeting_id
         WHERE a.meeting_id=?1 AND t.kind='processed' AND t.format='wav' AND t.sample_rate=16000 AND t.channels=1
         AND c.sequence=0 AND c.start_us=0 AND c.sample_rate=16000 AND c.channels=1",
        [meeting], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))
    ).optional().map_err(|e|e.to_string())?;
    let Some((
        id,
        manifest_asset_id,
        manifest_hash,
        frames,
        asset_id,
        hash,
        size,
        end,
        chunk_frames,
    )) = row
    else {
        return Ok(None);
    };
    let chunks: i64 = db
        .query_row(
            "SELECT count(*) FROM audio_chunks WHERE track_id=?1",
            [&id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let header = crate::audio::wave::header(
        u64::try_from(frames).map_err(|_| "会議音声のフレーム数が不正です")?,
    )?;
    if chunks != 1
        || frames <= 0
        || frames != chunk_frames
        || frames
            .checked_mul(2)
            .and_then(|v| v.checked_add(header.len() as i64))
            != Some(size)
        || end != frames * 1_000_000 / 16000
    {
        return Err("共通会議音声の保存記録が不正です".into());
    }
    Ok(Some(RecordedAudioPlan {
        manifest_asset_id,
        manifest_hash,
        tracks: vec![TrackPlan {
            id,
            kind: "processed".into(),
            start_us: 0,
            end_us: end,
            sample_rate: 16000,
            channels: 1,
            chunks: vec![ChunkPlan {
                asset_id,
                hash,
                size,
                start_us: 0,
                end_us: end,
                frames,
                sample_rate: 16000,
                channels: 1,
                clock_segments: vec![],
            }],
        }],
    }))
}

impl Storage {
    pub(crate) fn canonical_audio_plan(&self, meeting: &str) -> Result<RecordedAudioPlan, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        super::deletion::audio::require_source(&db, meeting)?;
        read(&db, meeting)?.ok_or("会議音声の移行・保存が完了していません".into())
    }
    pub(crate) fn verify_canonical_audio(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<RecordedAudioPlan, String> {
        let plan = self.canonical_audio_plan(meeting)?;
        let c = &plan.tracks[0].chunks[0];
        self.verify_meeting_asset_cancellable(meeting, &c.asset_id, &c.hash, cancelled)?;
        let path = self.meeting_mix_path(meeting, &plan.tracks[0])?;
        let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let layout = crate::audio::wave::inspect(&mut f, c.size as u64)?;
        if layout.frames != c.frames as u64 {
            return Err("共通WAVのフレーム数が一致しません".into());
        }
        if self.canonical_audio_plan(meeting)? != plan {
            return Err("会議音声の保存記録が変更されました".into());
        }
        Ok(plan)
    }
}

impl Storage {
    pub(crate) fn pending_analysis_target(
        &self,
        flow: &str,
    ) -> Result<Option<super::analysis_flows::regeneration::Target>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let text: Option<String> = db
            .query_row(
                "SELECT initial_replacement_json FROM analysis_flows WHERE id=?1",
                [flow],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        text.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
            .transpose()
    }
}
