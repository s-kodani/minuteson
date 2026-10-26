//! Stream verified pending capture/import inputs into the sole meeting WAV.
use super::transcription::{ChunkPlan, TrackPlan};
use super::*;
use crate::playback::{MeetingAudio, PlaybackMissingRange};
use std::io::{BufWriter, Write};

pub(super) fn read(db: &Connection, meeting: &str) -> Result<Option<RecordedAudioPlan>, String> {
    let original = super::inputs::read(db, meeting)?;
    read_existing(db, meeting, original, "ready", "ready")
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
        || end_us != frames * 1_000_000 / 16000
        || size != crate::audio::wave::header(frames as u64)?.len() as i64 + frames * 2
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

fn source_key(source: &RecordedAudioPlan, known: &KnownInput) -> Result<String, String> {
    // IDs and manifests are remapped by backup restore. Cache identity depends
    // only on audio bytes, timeline geometry, gaps and the mixing policy.
    let tracks:Vec<_>=source.tracks.iter().map(|t|serde_json::json!({
        "kind":t.kind,"startUs":t.start_us,"endUs":t.end_us,"sampleRate":t.sample_rate,"channels":t.channels,
        "chunks":t.chunks.iter().map(|c|serde_json::json!({"hash":c.hash,"size":c.size,"startUs":c.start_us,"endUs":c.end_us,"frames":c.frames,"sampleRate":c.sample_rate,"channels":c.channels,"clockSegments":c.clock_segments})).collect::<Vec<_>>()
    })).collect();
    Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"policy":"meeting_wav_pcm16_16000_v1","resampling":"direct_per_input_16000_v1","tracks":tracks,"coverage":known})).map_err(|e|e.to_string())?)))
}

// Keep capture gaps conservative for voice evidence, and also mark timeline
// padding where no capture source had any saved samples at all.
fn missing_intervals(known: &KnownInput) -> Vec<(i64, i64)> {
    let mut missing = Vec::new();
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
    pub(crate) fn verify_pending_meeting_inputs(
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
            super::inputs::read(&db, meeting)?
        };
        for track in &source.tracks {
            self.prepare_analysis_track(meeting, track, cancelled)?;
        }
        Ok(())
    }
    pub(crate) fn build_pending_meeting_wav(
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
                self.verify_pending_meeting_inputs(meeting, cancelled)?;
                self.verify_meeting_asset_cancellable(
                    meeting,
                    &plan.tracks[0].chunks[0].asset_id,
                    &plan.tracks[0].chunks[0].hash,
                    cancelled,
                )?;
                return Ok(plan);
            }
            let source = super::inputs::read(&db, meeting)?;
            let known = super::transcription::known::read(&db, meeting, &source)?;
            (source, known)
        };
        self.verify_meeting_asset_cancellable(
            meeting,
            &source.manifest_asset_id,
            &source.manifest_hash,
            cancelled,
        )?;
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
        let mut audio = MeetingAudio::with_rate(sources, duration, gaps, 16000)?;
        let frames = (duration as u64)
            .checked_mul(16000)
            .ok_or("会議音声が長すぎます")?
            .div_ceil(1_000_000);
        if frames == 0 {
            return Err("会議音声がWAV・話者分離のフレーム上限を超えています".into());
        }
        let required = frames
            .checked_mul(2)
            .and_then(|n| n.checked_add(16 * 1024 * 1024 + 80))
            .ok_or("WAV容量が範囲外です")?;
        if crate::audio_import::available_bytes(&self.root)? < required {
            return Err(
                "会議音声の保存に必要な空き容量がありません。原音声は保持しています".into(),
            );
        }
        let mut directory = super::super::publication_files::Directory::open(
            &self.managed_path(&Path::new("meetings").join(meeting))?,
        )?;
        for leaf in ["audio", "meeting-wave", key.as_str()] {
            let child_path = directory.path.join(leaf);
            let next = if child_path.symlink_metadata().is_ok() {
                super::super::publication_files::Directory::open(&child_path)?
            } else {
                directory.create_child(leaf)?
            };
            directory.verify()?;
            directory = next;
        }
        let mut pending = super::writer::Pending::open(&directory.path, &key, frames)?;
        let result: Result<String, String> = if let Some(hash) = pending.completed_hash() {
            Ok(hash.to_owned())
        } else {
            (|| {
                let mut writer =
                    BufWriter::new(pending.begin()?.ok_or("WAV書込みの所有がありません")?);
                let header = crate::audio::wave::header(frames)?;
                let mut expected = Sha256::new();
                expected.update(&header);
                writer.write_all(&header).map_err(|e| e.to_string())?;
                let mut offset = 0;
                progress(0, frames);
                while offset < frames {
                    if cancelled() {
                        return Err("会議音声の準備を中断しました".into());
                    }
                    let count = (frames - offset).min(8192);
                    let pcm = audio.read_analysis_pcm(offset, count)?;
                    for value in pcm.chunks_exact(4) {
                        let v = f32::from_le_bytes(value.try_into().unwrap());
                        let bytes = ((v.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes();
                        expected.update(bytes);
                        writer.write_all(&bytes).map_err(|e| e.to_string())?;
                    }
                    offset += count;
                    progress(offset, frames);
                }
                writer.flush().map_err(|e| e.to_string())?;
                writer.get_ref().sync_all().map_err(|e| e.to_string())?;
                if cancelled() {
                    return Err("会議音声の準備を中断しました".into());
                }
                Ok(format!("{:x}", expected.finalize()))
            })()
        };
        let expected = result?;
        pending.complete(expected.clone())?;
        let temporary = pending.path();
        let asset = self.register_analysis_artifact(meeting, &temporary, "audio/wav")?;
        if asset.content_hash != expected {
            return Err("保存WAVのPCMチェックサムが生成時と一致しません".into());
        }
        audio.verify_sources()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if super::inputs::read(&tx, meeting)? != source
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
            let config = serde_json::json!({"meetingMix":key,"policy":"meeting_wav_pcm16_16000_v1","missingRanges":known.gaps});
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
