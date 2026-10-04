use super::*;
use crate::audio::VerifiedAudio;
use crate::playback::{AudioSpan, PreparedAudio};
use std::fs::File;

struct Chunk {
    relative_path: String,
    size: i64,
    hash: String,
    start_us: i64,
    end_us: i64,
    frame_count: i64,
    rate: i64,
    channels: i64,
}

#[cfg(test)]
pub(in crate::storage) mod tests {
    use super::super::analysis::{AnalysisInput, UtteranceInput};
    use super::*;
    use crate::audio::tests::caf_fixture;
    use crate::playback::PlaybackStore;

    pub(in crate::storage) fn fixture(storage: &Storage) -> (String, String, String) {
        let meeting = storage.create_meeting("再生テスト").unwrap();
        let relative = PathBuf::from("meetings").join(&meeting.id).join("audio");
        std::fs::create_dir_all(storage.managed_path(&relative).unwrap()).unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type = 'recording', lifecycle = 'recorded' WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, audio_manifest_hash, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', ?2, 0, 0)", params![meeting.id, "a".repeat(64)]).unwrap();
        db.execute("INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'caf', 48000, 1, 'ready', 0, 0)", params![meeting.id]).unwrap();
        drop(db);
        for sequence in 0..2 {
            let samples = [if sequence == 0 { 0.25_f32 } else { -0.5_f32 }; 24_000];
            let bytes: Vec<u8> = samples
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect();
            let path = relative.join(format!("{sequence}.caf"));
            std::fs::write(
                storage.managed_path(&path).unwrap(),
                caf_fixture(48_000, 1, 3, 32, 4, &bytes),
            )
            .unwrap();
            let asset = storage
                .register_ready_meeting_asset(&meeting.id, &path, "audio/x-caf")
                .unwrap();
            storage.connection.lock().unwrap().execute("INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES (?1, ?2, 'track', ?3, ?4, ?5, ?6, 24000, 48000, 1, 'ready', 0)", params![format!("chunk-{sequence}"), meeting.id, asset.id, sequence, sequence * 500_000, (sequence + 1) * 500_000]).unwrap();
        }
        let candidate = storage
            .create_analysis_candidate(
                &meeting.id,
                AnalysisInput {
                    audio_manifest_hash: "a".repeat(64),
                    profile_snapshot: serde_json::json!({}),
                    config: serde_json::json!({}),
                    speakers: vec![],
                    segments: vec![],
                    utterances: vec![UtteranceInput {
                        track_id: "track".into(),
                        start_us: 250_000,
                        end_us: 750_000,
                        text: "発言".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    }],
                },
            )
            .unwrap();
        storage
            .commit_analysis_candidate(&meeting.id, &candidate, meeting.lock_version, false)
            .unwrap();
        let utterance = storage
            .load_current_analysis(&meeting.id)
            .unwrap()
            .unwrap()
            .utterances[0]
            .id
            .clone();
        (meeting.id, candidate, utterance)
    }

    #[test]
    fn playback_is_version_scoped_and_crosses_segment_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        assert!(storage
            .prepare_utterance_audio("other", &analysis, &utterance)
            .is_err());
        assert!(storage
            .prepare_utterance_audio(&meeting, "other", &utterance)
            .is_err());
        let audio = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        let store = PlaybackStore::default();
        let info = store.insert(audio).unwrap();
        assert_eq!(info.total_frames, 24_000);
        assert_eq!(info.track_kind, "mic");
        assert_eq!(info.alignment, "unverified");
        let packet = store.read(&info.token, 0).unwrap();
        assert_eq!(packet.len(), 12 + 24_000 * 4);
        for sample in packet[12..12 + 12_000 * 4].chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), 0.25);
        }
        for sample in packet[12 + 12_000 * 4..].chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), -0.5);
        }
        assert!(store.read(&info.token, info.total_frames).is_err());
        let tail = store.read(&info.token, 23_999).unwrap();
        assert_eq!(tail.len(), 16);
        store.close(&info.token).unwrap();
        assert!(store.read(&info.token, 0).is_err());
    }

    #[test]
    fn missing_corrupt_and_mapped_audio_are_not_played_as_normal() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        storage.connection.lock().unwrap().execute("INSERT INTO audio_gaps (id, track_id, start_us, end_us, reason_code) VALUES ('gap', 'track', 300000, 350000, 'missing')", []).unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("欠損"));
        storage
            .connection
            .lock()
            .unwrap()
            .execute("DELETE FROM audio_gaps", [])
            .unwrap();
        let path = directory
            .path()
            .join("meetings")
            .join(&meeting)
            .join("audio/1.caf");
        let mut changed = std::fs::read(&path).unwrap();
        *changed.last_mut().unwrap() ^= 1;
        std::fs::write(&path, changed).unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("ハッシュ"));
        storage.connection.lock().unwrap().execute("INSERT INTO track_clock_segments (id, track_id, chunk_id, source_frame_start, source_frame_end, timeline_start_us, timeline_end_us, mapping_json) VALUES ('clock', 'track', 'chunk-0', 0, 24000, 0, 500000, '{}')", []).unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("時刻補正"));
    }

    #[cfg(unix)]
    #[test]
    fn managed_audio_cannot_escape_through_a_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let path = directory
            .path()
            .join("meetings")
            .join(&meeting)
            .join("audio/0.caf");
        let moved = outside.path().join("audio.caf");
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(moved, &path).unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("管理領域外"));
    }
}

impl Storage {
    pub fn prepare_utterance_audio(
        &self,
        meeting_id: &str,
        analysis_id: &str,
        utterance_id: &str,
    ) -> Result<PreparedAudio, String> {
        let (track, start, end) = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let utterance: Option<(String, i64, i64)> = connection.query_row(
                "SELECT u.track_id, u.start_us, u.end_us FROM utterances u JOIN analysis_revisions a ON a.id = u.analysis_revision_id AND a.meeting_id = u.meeting_id JOIN audio_tracks t ON t.id = u.track_id AND t.meeting_id = u.meeting_id WHERE u.id = ?1 AND u.meeting_id = ?2 AND a.id = ?3 AND a.publication_state = 'committed' AND t.state = 'ready'",
                params![utterance_id, meeting_id, analysis_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional().map_err(|e| e.to_string())?;
            utterance.ok_or("この会議・解析版の再生可能な発言がありません")?
        };
        if !self.verify_analysis_content_hash(analysis_id)? {
            return Err("解析版の内容が変更されています".into());
        }
        self.prepare_track_audio(meeting_id, &track, start, end)
    }

    // The analysis worker can read a verified source before a transcript exists.
    // This is deliberately not exposed as an arbitrary-path IPC command.
    pub(crate) fn prepare_track_audio(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
    ) -> Result<PreparedAudio, String> {
        self.prepare_track_audio_cancellable(meeting_id, track, start, end, &|| false)
    }

    pub(crate) fn prepare_track_audio_cancellable(
        &self,
        meeting_id: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        if start < 0 || end <= start {
            return Err("音声区間が不正です".into());
        }
        let (kind, config, chunks) = {
            let connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let (kind, config): (String, Option<String>) = connection.query_row(
                "SELECT kind, processing_config_json FROM audio_tracks WHERE id = ?1 AND meeting_id = ?2 AND state = 'ready'",
                params![track, meeting_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional().map_err(|e| e.to_string())?.ok_or("この会議の確定済み音声トラックがありません")?;
            let gaps: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM audio_gaps WHERE track_id = ?1 AND start_us < ?3 AND end_us > ?2 AND recovered = 0)",
                params![track, start, end], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if gaps {
                return Err("この発言の音声に欠損区間があります".into());
            }
            let mapped: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM track_clock_segments WHERE track_id = ?1)",
                    params![track],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if mapped {
                return Err("時刻補正済みトラックの再生はまだ接続されていません".into());
            }
            let mut query = connection.prepare(
                "SELECT f.relative_path, f.size_bytes, f.content_hash, c.start_us, c.end_us, c.frame_count, c.sample_rate, c.channels FROM audio_chunks c JOIN file_assets f ON f.id = c.file_asset_id AND f.meeting_id = c.meeting_id WHERE c.track_id = ?1 AND c.meeting_id = ?2 AND c.start_us < ?4 AND c.end_us > ?3 AND c.state = 'ready' AND f.state = 'ready' ORDER BY c.start_us, c.sequence",
            ).map_err(|e| e.to_string())?;
            let chunks = query
                .query_map(params![track, meeting_id, start, end], |row| {
                    Ok(Chunk {
                        relative_path: row.get(0)?,
                        size: row.get(1)?,
                        hash: row.get(2)?,
                        start_us: row.get(3)?,
                        end_us: row.get(4)?,
                        frame_count: row.get(5)?,
                        rate: row.get(6)?,
                        channels: row.get(7)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            (kind, config, chunks)
        };
        if chunks.len() > 1024 {
            return Err("発言の音声セグメントが多すぎます".into());
        }
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let mut spans = Vec::new();
        let mut position = start;
        let mut audio_format = None;
        for chunk in chunks {
            if chunk.start_us > position || (!spans.is_empty() && chunk.start_us != position) {
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
            let audio = VerifiedAudio::open_cancellable(
                File::open(file).map_err(|e| e.to_string())?,
                chunk.size as u64,
                &chunk.hash,
                cancelled,
            )?;
            let format = &audio.format;
            if format.frames != chunk.frame_count as u64
                || i64::from(format.sample_rate) != chunk.rate
                || i64::from(format.channels) != chunk.channels
                || (i128::from(chunk.end_us - chunk.start_us)
                    - i128::from(chunk.frame_count) * 1_000_000 / i128::from(chunk.rate))
                .abs()
                    > 1
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
            let first =
                (i128::from(from - chunk.start_us) * i128::from(chunk.rate) / 1_000_000) as u64;
            let last = ((i128::from(to - chunk.start_us) * i128::from(chunk.rate) + 999_999)
                / 1_000_000) as u64;
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
        if position != end || spans.is_empty() {
            return Err("発言に対応する保存音声が不足しています".into());
        }
        let (sample_rate, channels) = audio_format.unwrap();
        let alignment = config
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
            track_kind: kind,
            alignment,
            spans,
        })
    }
}
