use super::*;
use crate::playback::MeetingAudio;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MeetingPlaybackMode {
    Mic,
    System,
    Both,
    Imported,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MeetingPlaybackRequest {
    pub(crate) meeting_id: String,
    pub(crate) mode: MeetingPlaybackMode,
}

#[derive(Debug, PartialEq, Eq)]
struct Track {
    id: String,
    kind: String,
    state: String,
    rate: i64,
    channels: i64,
    config: Option<String>,
    binding: String,
    chunks: Vec<Chunk>,
    gaps: Vec<Gap>,
    mappings: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Chunk {
    start: i64,
    end: i64,
    sequence: i64,
    ready: bool,
    // Full native-only census. A change of asset, hash, path, frames or state
    // while hashing audio invalidates preparation, not just a changed extent.
    stamp: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Gap {
    recovered: bool,
    stamp: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Census {
    end: i64,
    stamp: String,
    tracks: Vec<Track>,
}

impl Storage {
    #[cfg(test)]
    pub(crate) fn prepare_meeting_audio(
        &self,
        request: &MeetingPlaybackRequest,
    ) -> Result<MeetingAudio, String> {
        self.prepare_meeting_audio_cancellable(request, &|| false)
    }

    pub(crate) fn prepare_meeting_audio_cancellable(
        &self,
        request: &MeetingPlaybackRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MeetingAudio, String> {
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        let before = self.meeting_audio_census(&request.meeting_id)?;
        let kinds: &[&str] = match request.mode {
            MeetingPlaybackMode::Mic => &["mic"],
            MeetingPlaybackMode::System => &["system"],
            MeetingPlaybackMode::Both => &["mic", "system"],
            MeetingPlaybackMode::Imported => &["imported"],
        };
        let end = before.end;
        let mut sources = Vec::with_capacity(kinds.len());
        for kind in kinds {
            if cancelled() {
                return Err("会議音声の準備を中止しました".into());
            }
            let matches: Vec<_> = before
                .tracks
                .iter()
                .filter(|track| track.kind == *kind)
                .collect();
            if matches.len() != 1 || matches[0].state != "ready" {
                return Err("指定した種類の確定済み音声が一つに定まりません".into());
            }
            let track = matches[0];
            if track.chunks.iter().any(|chunk| !chunk.ready)
                || track.gaps.iter().any(|gap| !gap.recovered)
            {
                return Err("指定した会議音声に欠損または破損があります".into());
            }
            let start = track
                .chunks
                .iter()
                .map(|chunk| chunk.start)
                .min()
                .ok_or("トラックの保存音声がありません")?;
            let last = track.chunks.iter().map(|chunk| chunk.end).max().unwrap();
            // Uses the existing managed-path, content-hash, format, contiguous
            // coverage, deletion and clock-map admission checks over the WHOLE
            // source, including missing/corrupt chunks outside an utterance.
            let audio = self.prepare_track_audio_cancellable(
                &request.meeting_id,
                &track.id,
                start,
                last,
                cancelled,
            )?;
            if i64::from(audio.sample_rate) != track.rate
                || i64::from(audio.channels) != track.channels
            {
                return Err("会議音声の形式とトラック記録が一致しません".into());
            }
            sources.push(audio);
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        if self.meeting_audio_census(&request.meeting_id)? != before {
            return Err("準備中に会議音声の保存記録が変わりました。選び直してください".into());
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        MeetingAudio::new(sources, end)
    }

    fn meeting_audio_census(&self, meeting: &str) -> Result<Census, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        #[cfg(unix)]
        super::deletion::audio::require_source(&db, meeting)?;
        let source: (Option<String>, String, Option<String>, Option<String>) = db.query_row(
            "SELECT source_type, lifecycle, (SELECT state FROM recording_sessions WHERE meeting_id = meetings.id), (SELECT state FROM audio_imports WHERE meeting_id = meetings.id) FROM meetings WHERE id = ?1",
            params![meeting], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(|e| e.to_string())?.ok_or("会議がありません")?;
        let allowed: &[&str] = match source.0.as_deref() {
            Some("recording")
                if source.2.as_deref() == Some("saved")
                    && matches!(source.1.as_str(), "recorded" | "archived") =>
            {
                &["mic", "system"]
            }
            Some("audio_import")
                if source.3.as_deref() == Some("ready")
                    && matches!(source.1.as_str(), "imported" | "archived") =>
            {
                &["imported"]
            }
            _ => return Err("録音・取り込みの保存完了後に再生できます".into()),
        };
        let (duration, parent_duration, stamp): (i64, Option<i64>, String) = db.query_row(
            "SELECT m.duration_us, COALESCE(r.duration_us,i.duration_us), json_object('source',m.source_type,'lifecycle',m.lifecycle,'duration',m.duration_us,'recording',r.id,'recordingState',r.state,'recordingDuration',r.duration_us,'manifest',r.audio_manifest_hash,'import',i.id,'importState',i.state,'importDuration',i.duration_us,'original',i.original_asset_id,'converted',i.converted_asset_id) FROM meetings m LEFT JOIN recording_sessions r ON r.meeting_id = m.id LEFT JOIN audio_imports i ON i.meeting_id = m.id WHERE m.id = ?1",
            params![meeting], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).map_err(|e| e.to_string())?;
        if duration <= 0 || parent_duration != Some(duration) {
            return Err("会議と保存元の音声時間が一致しません".into());
        }
        let mut query = db.prepare("SELECT id, kind, state, sample_rate, channels, processing_config_json, json_object('format',format,'session',session_id,'import',import_id) FROM audio_tracks WHERE meeting_id = ?1 AND kind IN ('mic', 'system', 'imported') ORDER BY id LIMIT 4").map_err(|e| e.to_string())?;
        let mut tracks = query
            .query_map(params![meeting], |row| {
                Ok(Track {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    state: row.get(2)?,
                    rate: row.get(3)?,
                    channels: row.get(4)?,
                    config: row.get(5)?,
                    binding: row.get(6)?,
                    chunks: vec![],
                    gaps: vec![],
                    mappings: vec![],
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if tracks.is_empty()
            || tracks.len() > allowed.len()
            || tracks
                .iter()
                .any(|track| !allowed.contains(&track.kind.as_str()))
        {
            return Err("会議の原音声トラック構成が不正です".into());
        }
        for track in &mut tracks {
            let mut chunks = db.prepare("SELECT c.start_us, c.end_us, c.sequence, c.state = 'ready' AND f.state = 'ready', json_object('id',c.id,'asset',c.file_asset_id,'sequence',c.sequence,'start',c.start_us,'end',c.end_us,'frames',c.frame_count,'rate',c.sample_rate,'channels',c.channels,'state',c.state,'path',f.relative_path,'size',f.size_bytes,'hash',f.content_hash,'assetState',f.state) FROM audio_chunks c LEFT JOIN file_assets f ON f.id = c.file_asset_id AND f.meeting_id = c.meeting_id WHERE c.track_id = ?1 AND c.meeting_id = ?2 ORDER BY c.sequence, c.id LIMIT 1025").map_err(|e| e.to_string())?;
            track.chunks = chunks
                .query_map(params![track.id, meeting], |row| {
                    Ok(Chunk {
                        start: row.get(0)?,
                        end: row.get(1)?,
                        sequence: row.get(2)?,
                        ready: row.get::<_, Option<bool>>(3)?.unwrap_or(false),
                        stamp: row.get(4)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if track.chunks.len() > 1024 {
                return Err("会議音声のセグメントが多すぎます".into());
            }
            if track
                .chunks
                .iter()
                .enumerate()
                .any(|(index, chunk)| chunk.sequence != index as i64)
            {
                return Err("会議音声のセグメント番号に欠損があります".into());
            }
            let mut gaps = db.prepare("SELECT recovered, json_object('id',id,'start',start_us,'end',end_us,'reason',reason_code,'recovered',recovered) FROM audio_gaps WHERE track_id = ?1 ORDER BY id LIMIT 1025").map_err(|e| e.to_string())?;
            track.gaps = gaps
                .query_map(params![track.id], |row| {
                    Ok(Gap {
                        recovered: row.get(0)?,
                        stamp: row.get(1)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let mut maps = db.prepare("SELECT json_object('id',id,'chunk',chunk_id,'from',source_frame_start,'to',source_frame_end,'start',timeline_start_us,'end',timeline_end_us,'mapping',mapping_json) FROM track_clock_segments WHERE track_id = ?1 ORDER BY id LIMIT 1025").map_err(|e| e.to_string())?;
            track.mappings = maps
                .query_map(params![track.id], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if track.gaps.len() > 1024 || track.mappings.len() > 1024 {
                return Err("会議音声の時刻・欠損記録が多すぎます".into());
            }
        }
        if tracks
            .iter()
            .flat_map(|track| &track.chunks)
            .map(|chunk| chunk.end)
            .max()
            != Some(duration)
        {
            return Err("会議全体の音声時間と保存区間が一致しません".into());
        }
        Ok(Census {
            end: duration,
            stamp,
            tracks,
        })
    }
}
