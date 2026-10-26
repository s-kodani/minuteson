use super::*;
use crate::playback::{MeetingAudio, MeetingIntervalAudio, PlaybackMissingRange};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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
    pub(crate) start_reference: Option<MeetingPlaybackStartReference>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum MeetingPlaybackStartReference {
    Utterance {
        analysis_id: String,
        utterance_id: String,
        expected_content_hash: String,
        start_us: i64,
        end_us: i64,
    },
    DraftUtterance {
        request: DraftUtteranceAudioRequest,
    },
    Interval {
        start_us: i64,
        end_us: i64,
        expected_duration_us: i64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MeetingIntervalRequest {
    pub(crate) meeting_id: String,
    pub(crate) mode: MeetingPlaybackMode,
    pub(crate) start_us: i64,
    pub(crate) end_us: i64,
    pub(crate) expected_duration_us: i64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeetingAudioSummary {
    pub(crate) schema_version: u32,
    pub(crate) duration_us: i64,
    pub(crate) modes: Vec<MeetingPlaybackMode>,
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
    start: i64,
    end: i64,
    reason: String,
    stamp: String,
}

fn covers_missing(gaps: &[Gap], start: i64, end: i64) -> bool {
    let mut missing: Vec<_> = gaps.iter().filter(|gap| !gap.recovered).collect();
    missing.sort_by_key(|gap| (gap.start, gap.end));
    let mut position = start;
    for gap in missing {
        if gap.end <= position {
            continue;
        }
        if gap.start > position {
            break;
        }
        position = position.max(gap.end);
        if position >= end {
            return true;
        }
    }
    position >= end
}

#[derive(Debug, PartialEq, Eq)]
struct Census {
    end: i64,
    stamp: String,
    tracks: Vec<Track>,
}

impl Storage {
    fn resolve_meeting_playback_start(
        &self,
        request: &MeetingPlaybackRequest,
        duration: i64,
    ) -> Result<Option<(String, i64, i64, String)>, String> {
        let Some(reference) = &request.start_reference else {
            return Ok(None);
        };
        let stamp = match reference {
            MeetingPlaybackStartReference::Utterance {
                analysis_id,
                utterance_id,
                expected_content_hash,
                start_us,
                end_us,
            } => {
                let stamp = self.resolve_utterance_audio_reference(
                    &request.meeting_id,
                    analysis_id,
                    utterance_id,
                )?;
                if stamp.1 != *start_us || stamp.2 != *end_us || stamp.3 != *expected_content_hash {
                    return Err("確認した発言と再生開始位置が一致しません".into());
                }
                stamp
            }
            MeetingPlaybackStartReference::DraftUtterance { request: draft } => {
                if draft.meeting_id != request.meeting_id {
                    return Err("分割発言と再生する会議が一致しません".into());
                }
                let track = self.resolve_draft_utterance_audio_reference(draft)?;
                (
                    track,
                    draft.expected_start_us,
                    draft.expected_end_us,
                    draft.expected_content_hash.clone(),
                )
            }
            MeetingPlaybackStartReference::Interval {
                start_us,
                end_us,
                expected_duration_us,
            } => {
                if *expected_duration_us != duration {
                    return Err(
                        "確認後に会議音声の時間が変更されました。区間を確認し直してください".into(),
                    );
                }
                (String::new(), *start_us, *end_us, String::new())
            }
        };
        if stamp.1 < 0
            || stamp.2 <= stamp.1
            || stamp.2 > duration
            || duration > 9_007_199_254_740_991
        {
            return Err("再生開始位置と会議音声の時間が一致しません".into());
        }
        Ok(Some(stamp))
    }

    // Metadata only, not a token, file hash check or permission request. Native
    // preparation still verifies every managed source and rechecks the census.
    pub(crate) fn meeting_audio_summary(
        &self,
        meeting: &str,
    ) -> Result<MeetingAudioSummary, String> {
        let census = self.meeting_audio_census(meeting)?;
        if census.end > 9_007_199_254_740_991 {
            return Err("会議音声の時間が範囲外です".into());
        }
        let available = |kind: &str| {
            let matching: Vec<_> = census.tracks.iter().filter(|t| t.kind == kind).collect();
            matching.len() == 1
                && matching[0].state == "ready"
                && !matching[0].chunks.is_empty()
                && matching[0].chunks.iter().all(|c| c.ready)
        };
        let mut modes = Vec::new();
        if available("mic") {
            modes.push(MeetingPlaybackMode::Mic);
        }
        if available("system") {
            modes.push(MeetingPlaybackMode::System);
        }
        if available("mic") && available("system") {
            modes.push(MeetingPlaybackMode::Both);
        }
        if available("imported") {
            modes.push(MeetingPlaybackMode::Imported);
        }
        Ok(MeetingAudioSummary {
            schema_version: 1,
            duration_us: census.end,
            modes,
        })
    }

    pub(crate) fn prepare_meeting_interval_cancellable(
        &self,
        request: &MeetingIntervalRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MeetingIntervalAudio, String> {
        if request.start_us < 0
            || request.end_us <= request.start_us
            || request.end_us > request.expected_duration_us
            || request.expected_duration_us <= 0
            || request.expected_duration_us > 9_007_199_254_740_991
        {
            return Err("Markdownの再生区間・確認済み音声時間が不正です".into());
        }
        let audio = self.prepare_meeting_audio_cancellable(
            &MeetingPlaybackRequest {
                start_reference: None,
                meeting_id: request.meeting_id.clone(),
                mode: request.mode,
            },
            cancelled,
        )?;
        if audio.end_us != request.expected_duration_us {
            return Err(
                "確認後に会議音声の時間が変更されました。区間を確認し直してください".into(),
            );
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        MeetingIntervalAudio::new(audio, request.start_us, request.end_us)
    }
    #[cfg(test)]
    pub(crate) fn prepare_meeting_audio(
        &self,
        request: &MeetingPlaybackRequest,
    ) -> Result<MeetingAudio, String> {
        self.prepare_meeting_audio_cancellable(request, &crate::test_support::never_cancelled)
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
        let start_reference = self.resolve_meeting_playback_start(request, before.end)?;
        let kinds: &[&str] = match request.mode {
            MeetingPlaybackMode::Mic => &["mic"],
            MeetingPlaybackMode::System => &["system"],
            MeetingPlaybackMode::Both => &["mic", "system"],
            MeetingPlaybackMode::Imported => &["imported"],
        };
        let end = before.end;
        let mut sources = Vec::with_capacity(kinds.len());
        let mut missing_ranges = Vec::new();
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
            if track.chunks.iter().any(|chunk| !chunk.ready) {
                return Err("指定した会議音声の保存ファイルに欠損または破損があります".into());
            }
            let start = track
                .chunks
                .iter()
                .map(|chunk| chunk.start)
                .min()
                .ok_or("トラックの保存音声がありません")?;
            let last = track.chunks.iter().map(|chunk| chunk.end).max().unwrap();
            // Verify managed paths, formats, coverage, deletion and clock maps.
            // Ready FLAC playback reads packets on demand; full content hashes
            // and PCM/MD5 are checked at ingestion/publication and for AI input.
            let audio = self.prepare_meeting_track_audio_cancellable(
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
            let clock = audio
                .clock
                .as_ref()
                .ok_or("会議音声の取得区間を確認できません")?;
            for (first, last) in clock.missing_intervals() {
                if !covers_missing(&track.gaps, first, last) {
                    return Err("保存音声の空白に対応する欠損記録がありません".into());
                }
            }
            for gap in &track.gaps {
                if !gap.recovered {
                    missing_ranges.push(PlaybackMissingRange::new(
                        &track.kind,
                        gap.start,
                        gap.end,
                        &gap.reason,
                        end,
                    )?);
                }
            }
            sources.push(audio);
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        if self.meeting_audio_census(&request.meeting_id)? != before {
            return Err("準備中に会議音声の保存記録が変わりました。選び直してください".into());
        }
        if self.resolve_meeting_playback_start(request, before.end)? != start_reference {
            return Err("準備中に再生開始位置の参照が変わりました。選び直してください".into());
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        let mut audio = MeetingAudio::with_missing_ranges(sources, end, missing_ranges)?;
        if matches!(request.mode, MeetingPlaybackMode::Both) {
            audio.set_cache_key(&format!("{:?}:{before:?}", self.root))?;
        }
        Ok(audio)
    }

    fn meeting_audio_census(&self, meeting: &str) -> Result<Census, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        super::deletion::audio::require_source(&db, meeting)?;
        let source: (Option<String>, String, Option<String>, Option<String>) = db.query_row(
            "SELECT source_type, lifecycle, (SELECT state FROM recording_sessions WHERE meeting_id = meetings.id), (SELECT state FROM audio_imports WHERE meeting_id = meetings.id) FROM meetings WHERE id = ?1",
            params![meeting], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(|e| e.to_string())?.ok_or("会議がありません")?;
        let allowed: &[&str] = match source.0.as_deref() {
            Some("recording")
                if (source.2.as_deref() == Some("saved")
                    || (source.2.as_deref() == Some("interrupted")
                        && super::transcription::partial_recording_committed(&db, meeting)?))
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
            _ => {
                return Err(
                    "録音・取り込みの保存完了または確定済み部分の救出後に再生できます".into(),
                )
            }
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
            let mut gaps = db.prepare("SELECT recovered, start_us, end_us, reason_code, json_object('id',id,'start',start_us,'end',end_us,'reason',reason_code,'recovered',recovered) FROM audio_gaps WHERE track_id = ?1 ORDER BY id LIMIT ?2").map_err(|e| e.to_string())?;
            track.gaps = gaps
                .query_map(
                    params![track.id, (crate::audio_clock::MAX_CLOCK_GAPS + 1) as i64],
                    |row| {
                        Ok(Gap {
                            recovered: row.get(0)?,
                            start: row.get(1)?,
                            end: row.get(2)?,
                            reason: row.get(3)?,
                            stamp: row.get(4)?,
                        })
                    },
                )
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            // Enforce per-chunk and byte budgets before constructing census
            // JSON. Whole-track maps have a separate bound from one chunk.
            super::transcription::read_clock_segments(&db, &track.id)?;
            let mut maps = db.prepare("SELECT json_object('id',id,'chunk',chunk_id,'from',source_frame_start,'to',source_frame_end,'start',timeline_start_us,'end',timeline_end_us,'mapping',mapping_json) FROM track_clock_segments WHERE track_id = ?1 ORDER BY id LIMIT 65537").map_err(|e| e.to_string())?;
            track.mappings = maps
                .query_map(params![track.id], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if track.gaps.len() > crate::audio_clock::MAX_CLOCK_GAPS
                || track.mappings.len() > crate::audio_clock::MAX_CLOCK_SEGMENTS
            {
                return Err("会議音声の時刻・欠損記録が多すぎます".into());
            }
            for gap in &track.gaps {
                PlaybackMissingRange::new(&track.kind, gap.start, gap.end, &gap.reason, duration)?;
            }
        }
        let last = tracks
            .iter()
            .flat_map(|track| &track.chunks)
            .map(|chunk| chunk.end)
            .max()
            .ok_or("会議の保存音声がありません")?;
        if last > duration
            || (last < duration
                && tracks.iter().any(|track| {
                    track
                        .chunks
                        .iter()
                        .map(|chunk| chunk.end)
                        .max()
                        .is_none_or(|end| !covers_missing(&track.gaps, end, duration))
                }))
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
