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
pub(in crate::storage) mod tests {
    use super::super::analysis::{AnalysisInput, UtteranceInput};
    use super::*;
    use crate::audio::tests::caf_fixture;
    use crate::playback::PlaybackPreparations;
    use crate::playback::PlaybackStore;

    fn corrected(storage: &Storage, drift: bool) {
        let db = storage.connection.lock().unwrap();
        for sequence in 0..2 {
            let start = sequence * if drift { 550000 } else { 500000 };
            let end = start + if drift { 550000 } else { 500000 };
            db.execute(
                "UPDATE audio_chunks SET start_us=?1,end_us=?2 WHERE id=?3",
                params![start, end, format!("chunk-{sequence}")],
            )
            .unwrap();
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,'track',?2,0,24000,?3,?4,?5)",params![format!("map-{sequence}"),format!("chunk-{sequence}"),start,end,r#"{"schema_version":1,"method":"linear"}"#]).unwrap();
        }
    }

    #[test]
    fn corrected_identity_preserves_samples_original_files_and_saved_analysis() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let before = storage.load_current_analysis(&meeting).unwrap();
        let path = root
            .path()
            .join("meetings")
            .join(&meeting)
            .join("audio/0.caf");
        let bytes = std::fs::read(&path).unwrap();
        let mut original = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        let pcm = original.read_pcm(0, 24000).unwrap();
        corrected(&storage, false);
        let mut mapped = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        assert_eq!(mapped.total_frames(), 24000);
        assert_eq!(mapped.read_pcm(0, 24000).unwrap(), pcm);
        assert_eq!(mapped.alignment, "unverified");
        assert_eq!(storage.load_current_analysis(&meeting).unwrap(), before);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn corrected_drift_moves_chunk_boundary_on_meeting_time_and_keeps_slice_and_seek_phase() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        corrected(&storage, true);
        let mut full = storage
            .prepare_track_audio(&meeting, "track", 0, 1100000)
            .unwrap();
        let pcm = full.read_pcm(0, 52800).unwrap();
        let mut sliced = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        assert_eq!(sliced.total_frames(), 24000);
        assert_eq!(
            sliced.read_pcm(0, 24000).unwrap(),
            pcm[12000 * 4..36000 * 4]
        );
        let values = sliced.read_pcm(14000, 800).unwrap();
        assert!((f32::from_le_bytes(values[0..4].try_into().unwrap()) - 0.25).abs() < 1e-6);
        assert!(
            (f32::from_le_bytes(values[799 * 4..800 * 4].try_into().unwrap()) + 0.5).abs() < 1e-6
        );
    }

    #[test]
    fn corrected_metadata_changed_during_hash_is_rejected_before_publication() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        corrected(&storage, true);
        let checks = std::cell::Cell::new(0);
        let error = storage
            .prepare_utterance_audio_cancellable(&meeting, &analysis, &utterance, &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 4 {
                    storage
                        .connection
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE track_clock_segments SET mapping_json='{}' WHERE id='map-0'",
                            [],
                        )
                        .unwrap();
                }
                false
            })
            .err()
            .unwrap();
        assert!(error.contains("時刻補正が変更"), "{error}");
    }

    #[test]
    fn corrected_hashing_is_cancellable_and_does_not_change_mapping_or_audio() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        corrected(&storage, true);
        let checks = std::cell::Cell::new(0);
        let error = storage
            .prepare_utterance_audio_cancellable(&meeting, &analysis, &utterance, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 5
            })
            .err()
            .unwrap();
        assert!(error.contains("中断"), "{error}");
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .is_ok());
    }

    #[test]
    fn corrected_gap_outside_requested_slice_does_not_block_known_audio_or_become_silence() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        corrected(&storage, true);
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE audio_chunks SET start_us=650000,end_us=1200000 WHERE id='chunk-1'",
                [],
            )
            .unwrap();
            db.execute("UPDATE track_clock_segments SET timeline_start_us=650000,timeline_end_us=1200000 WHERE id='map-1'",[]).unwrap();
            db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES ('gap','track',550000,650000,'missing')",[]).unwrap();
        }
        for (start, end, value) in [(0, 550000, 0.25), (650000, 1200000, -0.5)] {
            let mut audio = storage
                .prepare_track_audio(&meeting, "track", start, end)
                .unwrap();
            assert_eq!(audio.total_frames(), 26400);
            assert!(audio
                .read_pcm(0, 26400)
                .unwrap()
                .chunks_exact(4)
                .all(
                    |sample| (f32::from_le_bytes(sample.try_into().unwrap()) - value).abs() < 1e-6
                ));
        }
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("欠損"));
    }

    #[test]
    fn oversized_clock_metadata_is_rejected_by_sql_before_loading_json() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        corrected(&storage, false);
        let metadata =
            serde_json::json!({"schema_version":1,"method":"linear","padding":"x".repeat(8192)})
                .to_string();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE track_clock_segments SET mapping_json=?1 WHERE id='map-0'",
                params![metadata],
            )
            .unwrap();
        let error = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap();
        assert!(error.contains("容量上限"), "{error}");
    }

    #[test]
    fn native_cancel_interrupts_file_hashing_and_keeps_saved_analysis_and_audio() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let before = storage.load_current_analysis(&meeting).unwrap();
        let store = PlaybackStore::default();
        let preparations = PlaybackPreparations::default();
        let id = Uuid::new_v4().to_string();
        preparations.reserve(&id).unwrap();
        let guard = preparations.claim(&id).unwrap();
        let checks = std::cell::Cell::new(0);
        let error = storage
            .prepare_utterance_audio_cancellable(&meeting, &analysis, &utterance, &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 5 {
                    preparations.cancel(&id, &store).unwrap();
                }
                guard.cancelled()
            })
            .err()
            .unwrap();
        assert!(error.contains("照合を中断"), "{error}");
        assert_eq!(checks.get(), 5);
        assert!(guard
            .publish(|| panic!("cancelled source cannot publish"))
            .is_err());
        assert_eq!(storage.load_current_analysis(&meeting).unwrap(), before);
        drop(guard);
        assert!(preparations.claim(&id).is_err());
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .is_ok());
    }
    #[test]
    fn lost_prepare_reply_closes_actual_native_playback_via_preparation_id() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let store = PlaybackStore::default();
        let preparations = PlaybackPreparations::default();
        let id = Uuid::new_v4().to_string();
        preparations.reserve(&id).unwrap();
        let guard = preparations.claim(&id).unwrap();
        let info = guard
            .publish(|| {
                store.insert(storage.prepare_utterance_audio(&meeting, &analysis, &utterance)?)
            })
            .unwrap();
        drop(guard);
        assert!(store.read(&info.token, 0).is_ok());
        preparations.cancel(&id, &store).unwrap();
        assert!(store.read(&info.token, 0).is_err());
        assert!(preparations.acknowledge(&id, &info.token).is_err());
        preparations.cancel(&id, &store).unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .is_ok());
    }
    #[test]
    fn utterance_reference_revoked_during_hashing_is_rechecked_before_returning_audio() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let checks = std::cell::Cell::new(0);
        let error=storage.prepare_utterance_audio_cancellable(&meeting,&analysis,&utterance,&||{
            checks.set(checks.get()+1);if checks.get()==4 { storage.connection.lock().unwrap().execute("UPDATE meetings SET current_analysis_id=NULL,previous_analysis_id=NULL WHERE id=?1",params![meeting]).unwrap(); }false
        }).err().unwrap();
        assert!(error.contains("再生可能な発言がありません"), "{error}");
    }

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
    fn managed_flac_assets_play_across_chunks_without_rewriting_analysis() {
        use crate::audio::flac::write_segment;
        use sha2::{Digest, Sha256};
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path().to_owned()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let before = storage.load_current_analysis(&meeting).unwrap();
        for sequence in 0..2 {
            let relative = PathBuf::from("meetings").join(&meeting).join("audio");
            let caf = storage
                .managed_path(&relative.join(format!("{sequence}.caf")))
                .unwrap();
            let bytes = std::fs::read(&caf).unwrap();
            let mut input = VerifiedAudio::open(
                File::open(&caf).unwrap(),
                bytes.len() as u64,
                &format!("{:x}", Sha256::digest(&bytes)),
            )
            .unwrap();
            let flac_relative = relative.join(format!("{sequence}.flac"));
            let flac = storage.managed_path(&flac_relative).unwrap();
            write_segment(&mut input, 0, 24000, &flac, &|| false, &|_| {}).unwrap();
            let asset = storage
                .register_ready_meeting_asset(&meeting, &flac_relative, "audio/flac")
                .unwrap();
            storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE audio_chunks SET file_asset_id=?1 WHERE id=?2",
                    params![asset.id, format!("chunk-{sequence}")],
                )
                .unwrap();
            assert_eq!(std::fs::read(caf).unwrap(), bytes);
        }
        storage
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE audio_tracks SET format='flac' WHERE id='track'", [])
            .unwrap();
        let store = PlaybackStore::default();
        let audio = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        let info = store.insert(audio).unwrap();
        assert_eq!(info.total_frames, 24000);
        let packet = store.read(&info.token, 0).unwrap();
        for (index, sample) in packet[12..].chunks_exact(4).enumerate() {
            assert_eq!(
                f32::from_le_bytes(sample.try_into().unwrap()),
                if index < 12000 { 0.25 } else { -0.5 }
            );
        }
        store.close(&info.token).unwrap();
        assert_eq!(storage.load_current_analysis(&meeting).unwrap(), before);
        // Stored metadata remains authoritative: a valid FLAC with an
        // incompatible DB frame count must still be rejected.
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE audio_chunks SET frame_count=24001 WHERE id='chunk-0'",
                [],
            )
            .unwrap();
        assert!(storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .err()
            .unwrap()
            .contains("保存記録"));
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE audio_chunks SET frame_count=24000 WHERE id='chunk-0'",
                [],
            )
            .unwrap();
        corrected(&storage, false);
        let mut mapped = storage
            .prepare_utterance_audio(&meeting, &analysis, &utterance)
            .unwrap();
        assert_eq!(mapped.read_pcm(0, 24000).unwrap(), packet[12..]);
        assert_eq!(storage.load_current_analysis(&meeting).unwrap(), before);
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
    #[cfg(test)]
    pub fn prepare_utterance_audio(
        &self,
        meeting_id: &str,
        analysis_id: &str,
        utterance_id: &str,
    ) -> Result<PreparedAudio, String> {
        self.prepare_utterance_audio_cancellable(meeting_id, analysis_id, utterance_id, &|| false)
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
        let resolve = || -> Result<(String, i64, i64), String> {
            let mut connection = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = connection.transaction().map_err(|e| e.to_string())?;
            #[cfg(unix)]
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
            Ok((track, start, end))
        };
        let before = resolve()?;
        let audio = self.prepare_track_audio_cancellable(
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
            #[cfg(unix)]
            super::deletion::audio::require_source(&connection, meeting_id)?;
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
        let layout = if mappings.is_empty() {
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
            let layout = ClockLayout::new(
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
            layout.range(start, end, rate)?;
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
            .map(|layout| ClockAudio::new(layout, start, end, sample_rate))
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
