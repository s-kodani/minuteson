use super::*;
use crate::audio::VerifiedAudio;
use crate::audio_clock::{ClockChunk, ClockLayout};
use crate::playback::ClockAudio;
use crate::playback::{AudioSpan, PreparedAudio};
use std::fs::File;

#[derive(PartialEq, Eq)]
struct Chunk {
    relative_path: String,
    size: i64,
    hash: String,
    start_us: i64,
    end_us: i64,
    frame_count: i64,
    rate: i64,
    channels: i64,
    id: String,
}

#[cfg(test)]
#[path = "playback_tests.rs"]
pub(in crate::storage) mod tests;

impl Storage {
    #[cfg(test)]
    pub fn prepare_utterance_audio(
        &self,
        meeting_id: &str,
        analysis_id: &str,
        utterance_id: &str,
    ) -> Result<PreparedAudio, String> {
        self.prepare_utterance_audio_cancellable(
            meeting_id,
            analysis_id,
            utterance_id,
            &crate::test_support::never_cancelled,
        )
    }

    pub(in crate::storage) fn resolve_utterance_audio_reference(
        &self,
        meeting_id: &str,
        analysis_id: &str,
        utterance_id: &str,
    ) -> Result<(String, i64, i64, String), String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        super::deletion::audio::require_source(&tx, meeting_id)?;
        let utterance: Option<(String, i64, i64,String)> = tx.query_row(
            "SELECT u.track_id, u.start_us, u.end_us,a.content_hash FROM utterances u JOIN analysis_revisions a ON a.id = u.analysis_revision_id AND a.meeting_id = u.meeting_id JOIN meetings m ON m.id=u.meeting_id JOIN audio_tracks t ON t.id = u.track_id AND t.meeting_id = u.meeting_id WHERE u.id = ?1 AND u.meeting_id = ?2 AND a.id = ?3 AND (a.publication_state='candidate' OR (a.publication_state='committed' AND a.id IN (m.current_analysis_id,m.previous_analysis_id))) AND t.state = 'ready'",
            params![utterance_id, meeting_id, analysis_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,row.get(3)?)),
        ).optional().map_err(|e| e.to_string())?;
        let (track, start, end, hash) =
            utterance.ok_or("この会議・解析版の再生可能な発言がありません")?;
        if super::revisions::analysis_content_hash(&tx, analysis_id)? != hash {
            return Err("解析版の内容が変更されています".into());
        }
        Ok((track, start, end, hash))
    }

    pub(crate) fn prepare_utterance_audio_cancellable(
        &self,
        meeting_id: &str,
        analysis_id: &str,
        utterance_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        let resolve =
            || self.resolve_utterance_audio_reference(meeting_id, analysis_id, utterance_id);
        let before = resolve()?;
        let audio = self.prepare_preview_track_audio_cancellable(
            meeting_id, &before.0, before.1, before.2, cancelled,
        )?;
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        if resolve()? != before {
            return Err("音声準備中に発言の参照先が変わりました".into());
        }
        Ok(audio)
    }

    // The analysis worker can read a verified source before a transcript exists.
    // This is deliberately not exposed as an arbitrary-path IPC command.
    #[cfg(test)]
    pub(crate) fn prepare_track_audio(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
    ) -> Result<PreparedAudio, String> {
        self.prepare_track_audio_cancellable(
            meeting_id,
            track,
            start,
            end,
            &crate::test_support::never_cancelled,
        )
    }

    pub(crate) fn prepare_track_audio_cancellable(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        self.prepare_track_audio_mode(meeting_id, track, start, end, cancelled, false, false)
    }

    pub(in crate::storage) fn prepare_preview_track_audio_cancellable(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        let plan = self.canonical_audio_plan(meeting_id)?;
        let belongs: bool = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audio_tracks WHERE id=?1 AND meeting_id=?2)",
                params![track, meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !belongs {
            return Err("登録元の音声トラックが会議にありません".into());
        }
        self.prepare_track_audio_mode(
            meeting_id,
            &plan.tracks[0].id,
            start,
            end,
            cancelled,
            false,
            true,
        )
    }

    pub(in crate::storage) fn prepare_meeting_track_audio_cancellable(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        self.prepare_track_audio_mode(meeting_id, track, start, end, cancelled, true, true)
    }

    fn prepare_track_audio_mode(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
        whole_meeting: bool,
        direct_playback: bool,
    ) -> Result<PreparedAudio, String> {
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        if start < 0 || end <= start {
            return Err("音声区間が不正です".into());
        }
        let read_source = || -> Result<_, String> {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let connection = db.transaction().map_err(|e| e.to_string())?;
            super::deletion::audio::require_source(&connection, meeting_id)?;
            let (kind, config): (String, Option<String>) = connection.query_row(
                "SELECT kind, processing_config_json FROM audio_tracks WHERE id = ?1 AND meeting_id = ?2 AND state = 'ready'",
                params![track, meeting_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(|e| e.to_string())?.ok_or("この会議の確定済み音声トラックがありません")?;
            let gaps: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM audio_gaps WHERE track_id = ?1 AND start_us < ?3 AND end_us > ?2 AND recovered = 0)",
                params![track, start, end], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if gaps && !whole_meeting && kind != "processed" {
                return Err("この発言の音声に欠損区間があります".into());
            }
            let mappings = super::transcription::read_clock_segments(&connection, track)?;
            let mapped = !mappings.is_empty();
            let mut query = connection.prepare(
                "SELECT f.relative_path, f.size_bytes, f.content_hash, c.start_us, c.end_us, c.frame_count, c.sample_rate, c.channels, c.id FROM audio_chunks c JOIN file_assets f ON f.id = c.file_asset_id AND f.meeting_id = c.meeting_id WHERE c.track_id = ?1 AND c.meeting_id = ?2 AND (?5 OR (c.start_us < ?4 AND c.end_us > ?3)) AND c.state = 'ready' AND f.state = 'ready' ORDER BY c.start_us, c.sequence LIMIT 1025",
            ).map_err(|e| e.to_string())?;
            let chunks = query
                .query_map(params![track, meeting_id, start, end, mapped], |row| {
                    Ok(Chunk {
                        relative_path: row.get(0)?,
                        size: row.get(1)?,
                        hash: row.get(2)?,
                        start_us: row.get(3)?,
                        end_us: row.get(4)?,
                        frame_count: row.get(5)?,
                        rate: row.get(6)?,
                        channels: row.get(7)?,
                        id: row.get(8)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            Ok((kind, config, chunks, mappings))
        };
        let source = read_source()?;
        let (kind, config, chunks, mappings) = &source;
        if chunks.len() > 1024 {
            return Err("発言の音声セグメントが多すぎます".into());
        }
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let mut spans = Vec::new();
        let mut position = start;
        let mut audio_format = None;
        let layout = if mappings.is_empty() && !whole_meeting {
            None
        } else {
            if mappings
                .keys()
                .any(|id| !chunks.iter().any(|chunk| &chunk.id == id))
            {
                return Err("時刻補正の対応先に利用可能な音声がありません".into());
            }
            let rate = u32::try_from(chunks.first().ok_or("時刻補正の音声がありません")?.rate)
                .map_err(|_| "時刻補正の音声形式が不正です")?;
            let layout = source_layout(
                &chunks
                    .iter()
                    .map(|chunk| ClockChunk {
                        frames: chunk.frame_count,
                        start_us: chunk.start_us,
                        end_us: chunk.end_us,
                        segments: mappings.get(&chunk.id).map_or(&[], Vec::as_slice),
                    })
                    .collect::<Vec<_>>(),
                rate,
            )?;
            if whole_meeting {
                if start != layout.start_us || end != layout.end_us {
                    return Err("会議全体の保存音声範囲が一致しません".into());
                }
            } else {
                layout.range(start, end, rate)?;
            }
            Some(layout)
        };
        for chunk in chunks {
            if cancelled() {
                return Err("音声の再生準備を取り消しました".into());
            }
            if layout.is_none()
                && (chunk.start_us > position || (!spans.is_empty() && chunk.start_us != position))
            {
                return Err("音声セグメント間に欠損または重複があります".into());
            }
            let relative = Path::new(&chunk.relative_path);
            let mut parts = relative.components();
            if !matches!(parts.next(), Some(Component::Normal(name)) if name == "meetings")
                || !matches!(parts.next(), Some(Component::Normal(name)) if name == meeting_id)
            {
                return Err("この音声は会議の管理領域内にありません".into());
            }
            let file = self.managed_path(relative)?;
            if file.canonicalize().map_err(|e| e.to_string())? != root.join(relative) {
                return Err("音声ファイルが管理領域外を参照しています".into());
            }
            let file = File::open(file).map_err(|e| e.to_string())?;
            let audio = if direct_playback {
                VerifiedAudio::open_saved_cancellable(
                    file,
                    chunk.size as u64,
                    &chunk.hash,
                    cancelled,
                )
            } else {
                VerifiedAudio::open_cancellable(file, chunk.size as u64, &chunk.hash, cancelled)
            }?;
            let format = &audio.format;
            if format.frames != chunk.frame_count as u64
                || i64::from(format.sample_rate) != chunk.rate
                || i64::from(format.channels) != chunk.channels
                || (layout.is_none()
                    && (i128::from(chunk.end_us - chunk.start_us)
                        - i128::from(chunk.frame_count) * 1_000_000 / i128::from(chunk.rate))
                    .abs()
                        > 1)
            {
                return Err("音声形式・時間と保存記録が一致しません".into());
            }
            let current_format = (format.sample_rate, format.channels);
            if audio_format.is_some_and(|previous| previous != current_format) {
                return Err("音声セグメント間で形式が異なります".into());
            }
            audio_format = Some(current_format);
            let from = position.max(chunk.start_us);
            let to = end.min(chunk.end_us);
            let first = if layout.is_some() {
                0
            } else {
                (i128::from(from - chunk.start_us) * i128::from(chunk.rate) / 1_000_000) as u64
            };
            let last = if layout.is_some() {
                format.frames
            } else {
                ((i128::from(to - chunk.start_us) * i128::from(chunk.rate) + 999_999) / 1_000_000)
                    as u64
            };
            let last = last.min(format.frames);
            if last <= first {
                return Err("発言の音声フレームがありません".into());
            }
            spans.push(AudioSpan {
                audio,
                first_frame: first,
                frames: last - first,
            });
            position = to;
        }
        if (layout.is_none() && position != end) || spans.is_empty() {
            return Err("発言に対応する保存音声が不足しています".into());
        }
        let (sample_rate, channels) = audio_format.unwrap();
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        if read_source()? != source {
            return Err("音声準備中に保存記録・時刻補正が変更されました".into());
        }
        let clock = layout
            .map(|layout| {
                if whole_meeting {
                    ClockAudio::for_meeting(layout, sample_rate)
                } else {
                    ClockAudio::new(layout, start, end, sample_rate)
                }
            })
            .transpose()?;
        let alignment = config
            .clone()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .and_then(|json| {
                json.get("timeline_alignment")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "unverified".into());
        Ok(PreparedAudio {
            sample_rate,
            channels,
            start_us: start,
            end_us: end,
            track_kind: kind.clone(),
            alignment: if clock.is_some() {
                "unverified".into()
            } else {
                alignment
            },
            spans,
            clock,
        })
    }
}

fn source_layout(
    chunks: &[crate::audio_clock::ClockChunk<'_>],
    rate: u32,
) -> Result<crate::audio_clock::ClockLayout, String> {
    if chunks.len() == 1 && chunks[0].segments.len() > 1024 {
        let c = &chunks[0];
        crate::audio_clock::ClockLayout::capture_track(
            crate::audio_clock::ClockChunk {
                frames: c.frames,
                start_us: c.start_us,
                end_us: c.end_us,
                segments: c.segments,
            },
            rate,
        )
    } else {
        crate::audio_clock::ClockLayout::new(chunks, rate)
    }
}
