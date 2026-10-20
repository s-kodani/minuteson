//! The one analysis source. Original capture tracks remain immutable.
use super::transcription::{ChunkPlan, TrackPlan};
use super::*;
use crate::playback::{MeetingAudio, PlaybackMissingRange};
use std::io::{BufWriter, Read, Write};

pub(super) fn read(db: &Connection, meeting: &str) -> Result<Option<RecordedAudioPlan>, String> {
    let original = super::transcription::read_original_analysis_audio_plan(db, meeting)?;
    read_existing(db, meeting, original, "ready", "ready")
}
pub(super) fn read_historical(
    db: &Connection,
    meeting: &str,
    history: &super::backup::DeletedAudioHistory,
) -> Result<Option<RecordedAudioPlan>, String> {
    let original =
        super::transcription::read_original_historical_analysis_audio_plan(db, meeting, history)?;
    read_existing(db, meeting, original, "failed", "missing")
}
fn read_existing(
    db: &Connection,
    meeting: &str,
    original: RecordedAudioPlan,
    track_state: &str,
    asset_state: &str,
) -> Result<Option<RecordedAudioPlan>, String> {
    let known = super::transcription::known::read(db, meeting, &original)?;
    let key = source_key(&original, &known)?;
    let row: Option<(String,String,i64,i64,i64)> = db.query_row(
        "SELECT t.id,f.id,f.size_bytes,c.end_us,c.frame_count FROM audio_tracks t JOIN audio_chunks c ON c.track_id=t.id JOIN file_assets f ON f.id=c.file_asset_id WHERE t.meeting_id=?1 AND t.kind='processed' AND t.state=?3 AND t.format='wav' AND t.sample_rate=16000 AND t.channels=1 AND c.state=?4 AND c.start_us=0 AND c.sample_rate=16000 AND c.channels=1 AND f.state=?4 AND json_extract(t.processing_config_json,'$.meetingMix')=?2",
        params![meeting,key,track_state,asset_state], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, asset_id, size, end_us, frames)) = row else {
        return Ok(None);
    };
    let hash = db
        .query_row(
            "SELECT content_hash FROM file_assets WHERE id=?1",
            [&asset_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if frames <= 0
        || frames > i32::MAX as i64
        || end_us != frames * 1_000_000 / 16000
        || size != 44 + frames * 2
    {
        return Err("共通会議音声の保存形式・フレーム数が不正です".into());
    }
    Ok(Some(RecordedAudioPlan {
        manifest_asset_id: original.manifest_asset_id,
        manifest_hash: original.manifest_hash,
        tracks: vec![TrackPlan {
            id,
            kind: "processed".into(),
            start_us: 0,
            end_us,
            sample_rate: 16000,
            channels: 1,
            chunks: vec![ChunkPlan {
                asset_id,
                hash,
                size,
                start_us: 0,
                end_us,
                frames,
                sample_rate: 16000,
                channels: 1,
                clock_segments: vec![],
            }],
        }],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeting_mix_is_a_reusable_verified_single_source() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        let first = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        assert_eq!(first.tracks.len(), 1);
        assert_eq!(first.tracks[0].kind, "processed");
        assert_eq!(first.tracks[0].total_frames().unwrap(), 64000);
        let second = s
            .ensure_meeting_mix(&m, &|| false, &|_, _| panic!("cache must not regenerate"))
            .unwrap();
        assert_eq!(first, second);
        let (current, known) = s.recorded_analysis_input(&m).unwrap();
        assert_eq!(first, current);
        assert_eq!(known.unwrap().runs.len(), 1);
        let mut audio = s
            .prepare_track_audio(&m, &first.tracks[0].id, 0, 4_000_000)
            .unwrap();
        let pcm = audio.read_pcm(16000, 160).unwrap();
        let value = f32::from_le_bytes(pcm[0..4].try_into().unwrap());
        assert!((value - 0.25).abs() < 0.001, "{value}");
    }
    #[test]
    fn meeting_mix_keeps_clock_gaps_and_later_audio_at_the_original_time() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::known_tests::split_fixture(root.path());
        let plan = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        assert_eq!(plan.tracks[0].total_frames().unwrap(), 320000);
        let (_, known) = s.recorded_analysis_input(&m).unwrap();
        let known = known.unwrap();
        assert_eq!(known.runs.len(), 1);
        assert_eq!(known.gaps.len(), 1);
        let mut audio = s
            .prepare_analysis_track(&m, &plan.tracks[0], &|| false)
            .unwrap();
        for (seconds, expected) in [(2, 0.25), (10, 0.0), (17, -0.5)] {
            let pcm = audio.read_pcm(seconds * 16000, 160).unwrap();
            let value = f32::from_le_bytes(pcm[..4].try_into().unwrap());
            assert!((value - expected).abs() < 0.001, "{seconds}s: {value}");
        }
        // Preview reads the padded meeting WAV; enrollment/feature extraction
        // continues to reject any interval crossing a known capture gap.
        let mut preview = s
            .prepare_preview_track_audio_cancellable(
                &m,
                &plan.tracks[0].id,
                9_000_000,
                11_000_000,
                &|| false,
            )
            .unwrap();
        assert!(preview.read_pcm(0, 160).unwrap().iter().all(|v| *v == 0));
        assert!(s
            .prepare_track_audio(&m, &plan.tracks[0].id, 9_000_000, 11_000_000)
            .is_err());
        let cached = s
            .ensure_meeting_mix(&m, &|| false, &|_, _| panic!("cache"))
            .unwrap();
        assert_eq!(plan, cached);
    }
    #[test]
    fn meeting_mix_source_mutation_during_generation_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE meetings SET duration_us=4000000 WHERE id=?1", [&m])
            .unwrap();
        let changed = std::cell::Cell::new(false);
        let result=s.ensure_meeting_mix(&m,&||false,&|done,_| {
            if done>0 && !changed.replace(true) {
                s.connection.lock().unwrap().execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('late-gap','track',1000000,2000000,'capture_timestamp_gap',0)",[]).unwrap();
            }
        });
        assert!(result.is_err());
        assert!(read(&s.connection.lock().unwrap(), &m).unwrap().is_none());
    }
    #[test]
    fn meeting_mix_new_backup_restores_and_reuses_the_waveform() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        let original = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        let archive = tempfile::tempdir().unwrap();
        let path = archive.path().join("mixed.monbackup");
        s.create_backup_archive(&path, &|| false).unwrap();
        let target = tempfile::tempdir().unwrap();
        let dest = Storage::open(target.path().into()).unwrap();
        let prepared = dest
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        let receipt = prepared
            .publish(
                &dest,
                &prepared.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap();
        let meeting = &receipt.meeting_ids[0];
        let restored = dest
            .ensure_meeting_mix(meeting, &|| false, &|_, _| {
                panic!("restored common WAV must be reusable")
            })
            .unwrap();
        assert_eq!(
            restored.tracks[0].chunks[0].hash,
            original.tracks[0].chunks[0].hash
        );
        drop(prepared);
        drop(dest);
        let reopened = Storage::open(target.path().into()).unwrap();
        assert_eq!(
            reopened.recorded_analysis_input(meeting).unwrap().0,
            restored
        );
    }
    #[test]
    fn meeting_mix_aligns_stereo_pc_audio_and_preserves_missing_microphone_time() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        let relative = PathBuf::from("meetings").join(&m).join("audio/system.caf");
        let pcm: Vec<_> = (0..48000 * 2 * 2)
            .flat_map(|_| 0.5_f32.to_le_bytes())
            .collect();
        std::fs::write(
            s.managed_path(&relative).unwrap(),
            crate::audio::tests::caf_fixture(48000, 2, 3, 32, 4, &pcm),
        )
        .unwrap();
        let asset = s
            .register_ready_meeting_asset(&m, &relative, "audio/x-caf")
            .unwrap();
        {
            let db = s.connection.lock().unwrap();
            db.execute("UPDATE meetings SET duration_us=4000000 WHERE id=?1", [&m])
                .unwrap();
            db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES ('system',?1,'session','system','caf',48000,2,'ready',0,0)",[&m]).unwrap();
            db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES ('system-chunk',?1,'system',?2,0,2000000,4000000,96000,48000,2,'ready',0)",params![m,asset.id]).unwrap();
            db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES ('gap','track',1000000,2000000,'capture_timestamp_gap',0)",[]).unwrap();
        }
        let mixed = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        let mut audio = s
            .prepare_analysis_track(&m, &mixed.tracks[0], &|| false)
            .unwrap();
        for (frame, expected) in [(8000, 0.125), (24000, 0.0), (40000, 0.375)] {
            let pcm = audio.read_pcm(frame, 160).unwrap();
            let value = f32::from_le_bytes(pcm[..4].try_into().unwrap());
            assert!((value - expected).abs() < 0.001, "{frame}: {value}");
        }
    }
    #[test]
    fn meeting_mix_keeps_leading_padding_out_of_voice_evidence() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        {
            let db = s.connection.lock().unwrap();
            db.execute("UPDATE meetings SET duration_us=6000000 WHERE id=?1", [&m])
                .unwrap();
            db.execute(
                "UPDATE audio_chunks SET start_us=2000000,end_us=6000000 WHERE meeting_id=?1",
                [&m],
            )
            .unwrap();
        }
        let mixed = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        assert_eq!(mixed.tracks[0].total_frames().unwrap(), 96000);
        let (_, known) = s.recorded_analysis_input(&m).unwrap();
        let gaps = known.unwrap().gaps;
        assert_eq!((gaps[0].start_us, gaps[0].end_us), (0, 2000000));
        assert!(s
            .prepare_track_audio(&m, &mixed.tracks[0].id, 0, 1000000)
            .is_err());
        let mut audio = s
            .prepare_analysis_track(&m, &mixed.tracks[0], &|| false)
            .unwrap();
        assert!(audio.read_pcm(16000, 160).unwrap().iter().all(|v| *v == 0));
        let pcm = audio.read_pcm(40000, 160).unwrap();
        assert!((f32::from_le_bytes(pcm[..4].try_into().unwrap()) - 0.25).abs() < 0.001);
    }
    #[test]
    fn meeting_mix_cancellation_never_admits_a_partial_source() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        assert!(s.ensure_meeting_mix(&m, &|| true, &|_, _| {}).is_err());
        assert!(read(&s.connection.lock().unwrap(), &m).unwrap().is_none());
    }
}

fn source_key(source: &RecordedAudioPlan, known: &KnownInput) -> Result<String, String> {
    // IDs and manifests are remapped by backup restore. Cache identity depends
    // only on audio bytes, timeline geometry, gaps and the mixing policy.
    let tracks:Vec<_>=source.tracks.iter().map(|t|serde_json::json!({
        "kind":t.kind,"startUs":t.start_us,"endUs":t.end_us,"sampleRate":t.sample_rate,"channels":t.channels,
        "chunks":t.chunks.iter().map(|c|serde_json::json!({"hash":c.hash,"size":c.size,"startUs":c.start_us,"endUs":c.end_us,"frames":c.frames,"sampleRate":c.sample_rate,"channels":c.channels,"clockSegments":c.clock_segments})).collect::<Vec<_>>()
    })).collect();
    Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"policy":"meeting_mix_pcm16_16000_v1","tracks":tracks,"coverage":known})).map_err(|e|e.to_string())?)))
}

// Keep capture gaps conservative for voice evidence, and also mark timeline
// padding where no capture source had any saved samples at all.
fn missing_intervals(known: &KnownInput) -> Vec<(i64, i64)> {
    let mut missing: Vec<_> = known
        .gaps
        .iter()
        .filter(|g| !g.recovered)
        .map(|g| (g.start_us, g.end_us))
        .collect();
    let mut coverage: Vec<_> = known.runs.iter().map(|r| (r.start_us, r.end_us)).collect();
    coverage.sort_unstable();
    let mut cursor = 0;
    for (start, end) in coverage {
        if start > cursor {
            missing.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < known.duration_us {
        missing.push((cursor, known.duration_us));
    }
    missing.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in missing {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

impl Storage {
    pub(crate) fn verify_meeting_mix_sources(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let source = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            if read(&db, meeting)?.is_none() {
                return Err("共通会議音声の入力条件が変更されています".into());
            }
            super::transcription::read_original_analysis_audio_plan(&db, meeting)?
        };
        for track in &source.tracks {
            self.prepare_analysis_track(meeting, track, cancelled)?;
        }
        Ok(())
    }
    pub(crate) fn meeting_mix_path(
        &self,
        meeting: &str,
        track: &TrackPlan,
    ) -> Result<PathBuf, String> {
        if track.kind != "processed" || track.chunks.len() != 1 {
            return Err("共通会議音声がありません".into());
        }
        let relative:String=self.connection.lock().map_err(|_|"DBロックを取得できません")?.query_row("SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2 AND state='ready'",params![track.chunks[0].asset_id,meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
        self.managed_path(Path::new(&relative))
    }
    pub(crate) fn copy_meeting_mix(
        &self,
        meeting: &str,
        track: &TrackPlan,
        target: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<u64, String> {
        if track.kind != "processed" || track.chunks.len() != 1 {
            return Err("共通会議音声がありません".into());
        }
        let chunk = &track.chunks[0];
        self.verify_meeting_asset_cancellable(meeting, &chunk.asset_id, &chunk.hash, cancelled)?;
        let source = self.meeting_mix_path(meeting, track)?;
        // A hard link changes ctime and invalidates retained verification.
        // clonefile is exclusive and shares PCM storage without touching input.
        let mut cloned = false;
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::ffi::OsStrExt;
            let from =
                std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            let to =
                std::ffi::CString::new(target.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            cloned = unsafe { libc::clonefile(from.as_ptr(), to.as_ptr(), 0) } == 0;
        }
        if !cloned {
            let mut input = std::fs::File::open(&source).map_err(|e| e.to_string())?;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| e.to_string())?;
            let mut buffer = vec![0_u8; 128 * 1024];
            loop {
                if cancelled() {
                    drop(output);
                    let _ = std::fs::remove_file(target);
                    return Err("会議音声の準備を中断しました".into());
                }
                let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            }
            output.sync_all().map_err(|e| e.to_string())?;
        }
        if cancelled() {
            let _ = std::fs::remove_file(target);
            return Err("会議音声の準備を中断しました".into());
        }
        Ok(chunk.frames as u64)
    }
    pub(crate) fn ensure_meeting_mix(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(u64, u64),
    ) -> Result<RecordedAudioPlan, String> {
        let (source, known) = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            if let Some(plan) = read(&db, meeting)? {
                drop(db);
                self.verify_meeting_mix_sources(meeting, cancelled)?;
                self.verify_meeting_asset_cancellable(
                    meeting,
                    &plan.tracks[0].chunks[0].asset_id,
                    &plan.tracks[0].chunks[0].hash,
                    cancelled,
                )?;
                return Ok(plan);
            }
            let source = super::transcription::read_original_analysis_audio_plan(&db, meeting)?;
            let known = super::transcription::known::read(&db, meeting, &source)?;
            (source, known)
        };
        let key = source_key(&source, &known)?;
        let duration = known.duration_us;
        let mut sources = Vec::new();
        for track in &source.tracks {
            sources.push(self.prepare_analysis_track(meeting, track, cancelled)?);
        }
        let gaps = known
            .gaps
            .iter()
            .filter(|gap| !gap.recovered)
            .map(|gap| {
                PlaybackMissingRange::new(
                    &source.tracks[gap.track_index].kind,
                    gap.start_us,
                    gap.end_us,
                    &gap.reason_code,
                    duration,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut audio = MeetingAudio::with_missing_ranges(sources, duration, gaps)?;
        let frames = (duration as u64)
            .checked_mul(16000)
            .ok_or("会議音声が長すぎます")?
            .div_ceil(1_000_000);
        if frames == 0 || frames > u64::from((u32::MAX - 36) / 2) || frames > i32::MAX as u64 {
            return Err("会議音声がWAV・話者分離のフレーム上限を超えています".into());
        }
        let directory = self.managed_path(
            &Path::new("meetings")
                .join(meeting)
                .join("analysis-input")
                .join(&key),
        )?;
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let temporary = directory.join(format!("{}.wav", Uuid::new_v4()));
        let result = (|| {
            let mut writer = BufWriter::new(
                std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)
                    .map_err(|e| e.to_string())?,
            );
            writer
                .write_all(&crate::transcription::wave::wav_header(frames as u32))
                .map_err(|e| e.to_string())?;
            let filter = crate::resampling::Filter::new(48000, 16000);
            let mut offset = 0;
            progress(0, frames);
            while offset < frames {
                if cancelled() {
                    return Err("会議音声の準備を中断しました".into());
                }
                let count = (frames - offset).min(8192);
                let first = (offset * 3).saturating_sub(filter.half as u64);
                let end = ((offset + count) * 3 + filter.half as u64).min(audio.total_frames);
                let pcm = audio.read_analysis_pcm(first, end - first)?;
                let mono: Vec<_> = pcm
                    .chunks_exact(4)
                    .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                for index in offset..offset + count {
                    let v = filter.sample_from(&mono, index, 48000, first);
                    writer
                        .write_all(&((v.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes())
                        .map_err(|e| e.to_string())?;
                }
                offset += count;
                progress(offset, frames);
            }
            writer.flush().map_err(|e| e.to_string())?;
            writer.get_ref().sync_all().map_err(|e| e.to_string())?;
            if cancelled() {
                return Err("会議音声の準備を中断しました".into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        let asset = self.register_analysis_artifact(meeting, &temporary, "audio/wav")?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if super::transcription::read_original_analysis_audio_plan(&tx, meeting)? != source
            || super::transcription::known::read(&tx, meeting, &source)? != known
            || cancelled()
        {
            return Err("会議音声の準備中に入力が変更されたか中断されました".into());
        }
        if read(&tx, meeting)?.is_none() {
            let (session, import): (Option<String>, Option<String>) = tx
                .query_row(
                    "SELECT session_id,import_id FROM audio_tracks WHERE id=?1",
                    [&source.tracks[0].id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            let id = Uuid::new_v4().to_string();
            let now = now_ms()?;
            let config = serde_json::json!({"meetingMix":key,"policy":"meeting_mix_pcm16_16000_v1","missingRanges":known.gaps});
            tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,import_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'processed','wav',16000,1,'ready',?5,?6,?6)",params![id,meeting,session,import,config.to_string(),now]).map_err(|e|e.to_string())?;
            for original in &source.tracks {
                tx.execute("INSERT INTO track_inputs(meeting_id,derived_track_id,input_track_id,role) VALUES(?1,?2,?3,'meeting_mix')",params![meeting,id,original.id]).map_err(|e|e.to_string())?;
            }
            for (start, end) in missing_intervals(&known) {
                tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,?3,?4,'meeting_input_missing',0)",params![Uuid::new_v4().to_string(),id,start,end]).map_err(|e|e.to_string())?;
            }
            let end_us = (frames * 1_000_000 / 16000) as i64;
            tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,?5,?6,16000,1,'ready',?7)",params![Uuid::new_v4().to_string(),meeting,id,asset.id,end_us,frames as i64,now]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        read(&db, meeting)?.ok_or("会議音声を登録できません".into())
    }
}
