//! Archived job provenance only. Never a playable or executable audio source.
use super::*;
use crate::storage::transcription::{read_clock_segments, RecordedAudioPlan};

pub(super) fn verifies(
    db: &Connection,
    meeting: &str,
    plan: &RecordedAudioPlan,
) -> Result<bool, String> {
    if plan.tracks.is_empty() || plan.tracks.len() > 2 {
        return Ok(false);
    }
    let manifest:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE id=?1 AND meeting_id=?2 AND media_type='application/json' AND content_hash=?3 AND state='ready')",params![plan.manifest_asset_id,meeting,plan.manifest_hash],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !manifest {
        return Ok(false);
    }
    for track in &plan.tracks {
        let format: Option<(String, u32, u32)> = db
            .query_row(
                "SELECT kind,sample_rate,channels FROM audio_tracks WHERE id=?1 AND meeting_id=?2",
                params![track.id, meeting],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if format != Some((track.kind.clone(), track.sample_rate, track.channels))
            || track.chunks.is_empty()
        {
            return Ok(false);
        }
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM audio_chunks WHERE track_id=?1 AND meeting_id=?2",
                params![track.id, meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count != track.chunks.len() as i64
            || track.start_us != track.chunks[0].start_us
            || track.end_us != track.chunks.last().unwrap().end_us
        {
            return Ok(false);
        }
        let mut maps = read_clock_segments(db, &track.id)?;
        for (sequence, chunk) in track.chunks.iter().enumerate() {
            let row:Option<(String,String,i64,i64,i64,i64,u32,u32)>=db.query_row("SELECT c.id,f.content_hash,f.size_bytes,c.start_us,c.end_us,c.frame_count,c.sample_rate,c.channels FROM audio_chunks c JOIN file_assets f ON f.id=c.file_asset_id AND f.meeting_id=c.meeting_id WHERE c.track_id=?1 AND c.meeting_id=?2 AND c.sequence=?3 AND f.id=?4",params![track.id,meeting,sequence as i64,chunk.asset_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?;
            let Some((id, hash, size, start, end, frames, rate, channels)) = row else {
                return Ok(false);
            };
            if (hash, size, start, end, frames, rate, channels)
                != (
                    chunk.hash.clone(),
                    chunk.size,
                    chunk.start_us,
                    chunk.end_us,
                    chunk.frames,
                    chunk.sample_rate,
                    chunk.channels,
                )
                || maps.remove(&id).unwrap_or_default() != chunk.clock_segments
                || !super::super::meeting_audio::superseded_asset(db, &chunk.asset_id)?
            {
                return Ok(false);
            }
        }
        if !maps.is_empty() {
            return Ok(false);
        }
        track.total_frames()?;
    }
    Ok(true)
}
