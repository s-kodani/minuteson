//! Windows normal-stop publication. Keep the original CAFs; publish bounded
//! FLAC chunks only after full input hashing and PCM validation. Lost-response
//! retries compare a fresh deterministic encoding, never overwrite an output.
use super::*;
use crate::audio::{flac::encode_segment_file, VerifiedAudio};
use crate::audio_clock::{ClockChunk, ClockLayout, ClockSegment};
pub(super) enum Publication {
    Legacy(LegacyPublication),
    Segmented(super::flac::Publication),
}
impl Publication {
    pub(super) fn prepare(
        storage: &Storage,
        base: &Path,
        manifest_hash: &str,
        manifest: &CaptureManifest,
        mic: Option<&CapturedFile>,
        system: Option<&CapturedFile>,
        prefixes: Option<&[crate::recording_segments::Prefix]>,
    ) -> Result<Self, String> {
        if let Some(prefixes) = prefixes {
            Ok(Self::Segmented(super::flac::Publication::prepare(
                storage,
                base,
                manifest_hash,
                manifest,
                prefixes,
            )?))
        } else {
            Ok(Self::Legacy(LegacyPublication::prepare(
                storage, base, manifest, mic, system,
            )?))
        }
    }
    pub(super) fn verify(&self, storage: &Storage) -> Result<(), String> {
        match self {
            Self::Legacy(p) => p.verify(storage),
            Self::Segmented(p) => p.verify(storage),
        }
    }
    pub(super) fn insert(
        &self,
        db: &rusqlite::Transaction<'_>,
        meeting: &str,
        session: &str,
        config: &str,
        duration: i64,
        closed: bool,
        now: i64,
    ) -> Result<(), String> {
        match self {
            Self::Legacy(p) => p.insert(db, meeting, session, config, duration, closed, now),
            Self::Segmented(p) => p.insert(db, meeting, session, config, duration, closed, now),
        }
    }
}
struct Chunk {
    file: CapturedFile,
    frames: i64,
    rate: i64,
    channels: i64,
    map: ClockSegment,
}
struct Track {
    kind: &'static str,
    timeline: Vec<ClockSegment>,
    chunks: Vec<Chunk>,
}
pub(super) struct LegacyPublication {
    tracks: Vec<Track>,
    proofs: Vec<(PathBuf, File, String)>,
    _pins: Vec<File>,
}
impl LegacyPublication {
    pub(super) fn prepare(
        storage: &Storage,
        base: &Path,
        manifest: &CaptureManifest,
        mic: Option<&CapturedFile>,
        system: Option<&CapturedFile>,
    ) -> Result<Self, String> {
        let _permit =
            crate::audio::flac::ConversionPermit::acquire(&|| storage.data_write_blocked())?;
        let mut result = Self {
            tracks: Vec::new(),
            proofs: Vec::new(),
            _pins: crate::windows_files::parents(&storage.root.join(base))?,
        };
        let mut inputs = Vec::new();
        if let Some(mic) = mic {
            inputs.push(("mic", "microphone", mic, &manifest.microphone));
        }
        if let Some(file) = system {
            inputs.push(("system", "system", file, &manifest.system));
        }
        for (kind, name, proof, track) in inputs {
            let path = storage.managed_path(&proof.relative)?;
            let file = crate::windows_files::read(&path, false)?;
            result.pin(proof.relative.clone(), &file)?;
            let mut audio = VerifiedAudio::open_cancellable(
                file,
                proof.size_bytes as u64,
                &proof.hash,
                &|| false,
            )?;
            let (rate, channels, _) = track.validated_format()?;
            let maps = track
                .clock_segments
                .as_ref()
                .ok_or("録音の時計情報がありません")?;
            let mut output = Track {
                kind,
                timeline: maps.clone(),
                chunks: Vec::new(),
            };
            for map in maps {
                let mut first = map.source_frame_start;
                while first < map.source_frame_end {
                    storage.require_data_writable()?;
                    let frames = (map.source_frame_end - first).min(rate * 60);
                    let time = |frame: i64| -> Result<i64, String> {
                        let delta = i128::from(frame - map.source_frame_start)
                            * i128::from(map.timeline_end_us - map.timeline_start_us)
                            / i128::from(map.source_frame_end - map.source_frame_start);
                        i64::try_from(i128::from(map.timeline_start_us) + delta)
                            .map_err(|_| "録音時刻の計算が範囲外です".into())
                    };
                    let mapping = ClockSegment {
                        source_frame_start: 0,
                        source_frame_end: frames,
                        timeline_start_us: time(first)?,
                        timeline_end_us: time(first + frames)?,
                        mapping_json: map.mapping_json.clone(),
                    };
                    ClockLayout::new(
                        &[ClockChunk {
                            frames,
                            start_us: mapping.timeline_start_us,
                            end_us: mapping.timeline_end_us,
                            segments: std::slice::from_ref(&mapping),
                        }],
                        rate as u32,
                    )?;
                    let relative =
                        base.join(format!("windows-{name}-{:06}.flac", output.chunks.len()));
                    let path = storage.managed_path(&relative)?;
                    let mut temporary = tempfile::Builder::new()
                        .prefix(".mon-windows-flac-")
                        .tempfile_in(path.parent().ok_or("保存先がありません")?)
                        .map_err(|e| e.to_string())?;
                    let saved = encode_segment_file(
                        &mut audio,
                        first as u64,
                        frames as u64,
                        temporary.as_file_mut(),
                        &|| false,
                        &|_| {},
                    )?;
                    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
                    match temporary.persist_noclobber(&path) {
                        Ok(file) => {
                            file.sync_all().map_err(|e| e.to_string())?;
                        }
                        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.error.to_string()),
                    }
                    // A collision is accepted only if it is byte-for-byte the
                    // freshly encoded source and passes full FLAC/PCM checks.
                    let pinned = crate::windows_files::read(&path, false)?;
                    result.pin(relative.clone(), &pinned)?;
                    let checked = VerifiedAudio::open_cancellable(
                        pinned,
                        saved.size,
                        &saved.sha256,
                        &|| false,
                    )?;
                    if checked.format.frames != frames as u64 {
                        return Err("保存FLACのフレーム数が一致しません".into());
                    }
                    output.chunks.push(Chunk {
                        file: CapturedFile {
                            relative,
                            size_bytes: saved.size as i64,
                            hash: saved.sha256,
                        },
                        frames,
                        rate,
                        channels,
                        map: mapping,
                    });
                    first += frames;
                }
            }
            audio.check_unchanged()?;
            result.tracks.push(output);
        }
        result.verify(storage)?;
        Ok(result)
    }
    fn pin(&mut self, relative: PathBuf, file: &File) -> Result<(), String> {
        let stamp = crate::audio::reuse::stamp(
            &crate::native_fs::metadata(file).map_err(|e| e.to_string())?,
        );
        self.proofs.push((
            relative,
            file.try_clone().map_err(|e| e.to_string())?,
            stamp,
        ));
        Ok(())
    }
    pub(super) fn verify(&self, storage: &Storage) -> Result<(), String> {
        storage.require_data_writable()?;
        // Full SHA/PCM checks happened while these deny-write/deny-delete
        // handles were pinned. Commit checks are proportional to file count,
        // never audio bytes, and do not decode under the shared DB lock.
        for (relative, file, expected) in &self.proofs {
            let current = crate::windows_files::read(&storage.managed_path(relative)?, false)?;
            for candidate in [file, &current] {
                if crate::audio::reuse::stamp(
                    &crate::native_fs::metadata(candidate).map_err(|e| e.to_string())?,
                ) != *expected
                {
                    return Err("録音音声が検証後に変更されています".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn insert(
        &self,
        db: &rusqlite::Transaction<'_>,
        meeting: &str,
        session: &str,
        config: &str,
        duration: i64,
        closed: bool,
        now: i64,
    ) -> Result<(), String> {
        for track in &self.tracks {
            let first = track.chunks.first().ok_or("録音トラックが空です")?;
            let id = Uuid::new_v4().to_string();
            db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES (?1,?2,?3,?4,'flac',?5,?6,'ready',?7,?8,?8)",params![id,meeting,session,track.kind,first.rate,first.channels,config,now]).map_err(|e|e.to_string())?;
            for (sequence, chunk) in track.chunks.iter().enumerate() {
                let asset = insert_capture_asset(db, meeting, &chunk.file, "audio/flac", now)?;
                let chunk_id = Uuid::new_v4().to_string();
                db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'ready',?11)",params![chunk_id,meeting,id,asset,sequence as i64,chunk.map.timeline_start_us,chunk.map.timeline_end_us,chunk.frames,chunk.rate,chunk.channels,now]).map_err(|e|e.to_string())?;
                let timeline = clock::CaptureTimeline {
                    start_us: chunk.map.timeline_start_us,
                    end_us: chunk.map.timeline_end_us,
                    segments: std::slice::from_ref(&chunk.map),
                };
                timeline.insert(db, &id, &chunk_id)?;
            }
            // Each physical FLAC contains one affine map. Clock gaps between
            // those files still need evidence on the original track so known
            // analysis runs preserve their positions instead of closing them.
            for pair in track.timeline.windows(2) {
                if pair[0].timeline_end_us < pair[1].timeline_start_us {
                    db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES (?1,?2,?3,?4,'capture_timestamp_gap')", params![Uuid::new_v4().to_string(), id, pair[0].timeline_end_us, pair[1].timeline_start_us]).map_err(|e| e.to_string())?;
                }
            }
            if closed {
                clock::CaptureTimeline {
                    start_us: track.timeline[0].timeline_start_us,
                    end_us: track.timeline.last().unwrap().timeline_end_us,
                    segments: &track.timeline,
                }
                .insert_boundary_gaps(db, &id, duration)?;
            }
        }
        Ok(())
    }
}
