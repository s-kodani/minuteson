//! Pending capture/import metadata, used only before canonical publication.
use crate::storage::transcription::{
    partial_recording_committed, read_clock_segments, ChunkPlan, TrackPlan,
};
use crate::storage::*;

pub(in crate::storage) fn read(
    transaction: &Connection,
    meeting_id: &str,
) -> Result<RecordedAudioPlan, String> {
    let (track_state, chunk_state) = ("ready", "ready");
    // Both current source types use verified managed manifests.
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
        {
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
        }
        plans.push(plan);
    }
    if plans.is_empty() || plans.len() > 2 || (source_kind == "audio_import" && plans.len() != 1) {
        return Err("解析対象の保存済み音声トラックが不正です".into());
    }
    {
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
