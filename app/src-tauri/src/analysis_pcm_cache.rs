//! Bounded, unnamed, process-local PCM. The cache is never a source of truth:
//! callers validate source coverage and physical audio before consulting it.
use crate::storage::{KnownInput, RecordedAudioPlan, Storage};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
const LIMIT: u64 = 1024 * 1024 * 1024;
struct Entry {
    key: String,
    file: File,
    frames: u64,
    written: u64,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: Vec<Entry>,
}
pub(crate) fn key(
    source: &RecordedAudioPlan,
    known: Option<&KnownInput>,
    track: usize,
    run: Option<usize>,
) -> Result<String, String> {
    Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"version":"analysis-pcm16-core-v1","source":source,"known":known,"track":track,"run":run})).map_err(|e|e.to_string())?)))
}
fn count(frames: u64, rate: u32) -> u64 {
    (frames * 16000 + u64::from(rate) / 2) / u64::from(rate)
}
impl Storage {
    pub(crate) fn clear_analysis_pcm(&self) {
        if let Ok(mut c) = self.analysis_pcm.lock() {
            c.entries.clear();
        }
    }
    pub(crate) fn cache_analysis_core(
        &self,
        key: &str,
        wave: &Path,
        rate: u32,
        total: u64,
        window_first: u64,
        core_first: u64,
        core_end: u64,
        cancelled: &impl Fn() -> bool,
    ) {
        // Windows start on integer seconds. Other phases require a different
        // conversion, and must not silently populate a whole-run cache.
        if rate == 0 || window_first % u64::from(rate) != 0 || cancelled() {
            return;
        }
        let total = count(total, rate);
        let first = count(core_first, rate);
        let end = count(core_end, rate);
        if total == 0 || total > LIMIT / 2 || first >= end || end > total {
            return;
        }
        let result = (|| -> Result<(), String> {
            let mut c = self
                .analysis_pcm
                .lock()
                .map_err(|_| "PCMキャッシュロックを取得できません")?;
            let pos = match c.entries.iter().position(|e| e.key == key) {
                Some(p) => p,
                None => {
                    if first != 0 {
                        return Ok(());
                    }
                    while c.entries.len() >= 4
                        || c.entries.iter().map(|e| e.frames * 2).sum::<u64>() + total * 2 > LIMIT
                    {
                        c.entries.remove(0);
                    }
                    c.entries.push(Entry {
                        key: key.into(),
                        file: tempfile::tempfile().map_err(|e| e.to_string())?,
                        frames: total,
                        written: 0,
                    });
                    c.entries.len() - 1
                }
            };
            let e = &mut c.entries[pos];
            if e.frames != total || first != e.written {
                return Ok(());
            }
            let mut input = File::open(wave).map_err(|e| e.to_string())?;
            let offset = first
                .checked_sub(count(window_first, rate))
                .ok_or("PCM区間が不正です")?;
            input
                .seek(SeekFrom::Start(44 + offset * 2))
                .map_err(|e| e.to_string())?;
            e.file
                .seek(SeekFrom::Start(first * 2))
                .map_err(|e| e.to_string())?;
            let mut remaining = (end - first) * 2;
            let mut buffer = [0; 65536];
            while remaining > 0 {
                if cancelled() {
                    return Ok(());
                }
                let n = remaining.min(buffer.len() as u64) as usize;
                input
                    .read_exact(&mut buffer[..n])
                    .map_err(|e| e.to_string())?;
                e.file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                remaining -= n as u64;
            }
            e.written = end;
            Ok(())
        })();
        if let Err(error) = result {
            if let Ok(mut c) = self.analysis_pcm.lock() {
                c.entries.retain(|e| e.key != key);
            }
            crate::internal_logs::detail(
                crate::internal_logs::current_context("transcription"),
                "running",
                "info",
                std::time::Duration::ZERO,
                &format!("PCMキャッシュを使用せず続行: {error}"),
            );
        }
    }
    pub(crate) fn write_cached_analysis_pcm(
        &self,
        key: &str,
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<u64>, String> {
        if cancelled() {
            return Err("PCM再利用を中断しました".into());
        }
        let mut c = self
            .analysis_pcm
            .lock()
            .map_err(|_| "PCMキャッシュロックを取得できません")?;
        let Some(pos) = c
            .entries
            .iter()
            .position(|e| e.key == key && e.written == e.frames)
        else {
            return Ok(None);
        };
        let mut e = c.entries.remove(pos);
        let result = (|| {
            let mut output = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            output
                .write_all(&crate::transcription::wave::wav_header(e.frames as u32))
                .map_err(|e| e.to_string())?;
            e.file.rewind().map_err(|e| e.to_string())?;
            let mut remaining = e.frames * 2;
            let mut buffer = [0; 65536];
            while remaining > 0 {
                if cancelled() {
                    return Err("PCM再利用を中断しました".into());
                }
                let n = remaining.min(buffer.len() as u64) as usize;
                e.file
                    .read_exact(&mut buffer[..n])
                    .map_err(|e| e.to_string())?;
                output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                remaining -= n as u64;
            }
            output.sync_all().map_err(|e| e.to_string())?;
            Ok(Some(e.frames))
        })();
        c.entries.push(e);
        if result.is_ok() {
            crate::internal_logs::detail(crate::internal_logs::current_context("diarization"),"running","info",std::time::Duration::ZERO,&serde_json::json!({"event":"解析PCM再利用","key":key,"bytes":c.entries.last().unwrap().frames*2}).to_string());
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::{tests::caf_fixture, VerifiedAudio},
        playback::{AudioSpan, PreparedAudio},
    };
    fn audio(rate: u32, channels: u32, frames: u64) -> PreparedAudio {
        let samples: Vec<u8> = (0..frames)
            .flat_map(|i| {
                (0..channels).flat_map(move |c| {
                    (((i * 37 + u64::from(c)) % 1000) as f32 / 2048.).to_le_bytes()
                })
            })
            .collect();
        let bytes = caf_fixture(rate, channels, 3, 32, 4, &samples);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        let audio = VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        PreparedAudio {
            sample_rate: rate,
            channels,
            start_us: 0,
            end_us: (frames * 1_000_000 / u64::from(rate)) as i64,
            track_kind: "imported".into(),
            alignment: "verified".into(),
            spans: vec![AudioSpan {
                audio,
                first_frame: 0,
                frames,
            }],
            clock: None,
        }
    }
    #[test]
    fn analysis_pcm_cache_matches_direct_conversion_for_rates_channels_and_boundaries() {
        let t = tempfile::tempdir().unwrap();
        let storage = Storage::open(t.path().join("data")).unwrap();
        for rate in [8000, 16000, 44100, 48000, 96000, 384000] {
            for channels in [1, 2] {
                let mut audio = audio(
                    rate,
                    channels,
                    u64::from(rate) * if channels == 2 { 4 } else { 3 } + 7,
                );
                if channels == 2 {
                    let segments = vec![crate::audio_clock::ClockSegment {
                        source_frame_start: 0,
                        source_frame_end: audio.total_frames() as i64,
                        timeline_start_us: 0,
                        timeline_end_us: 3_000_000,
                        mapping_json: r#"{"schema_version":1,"method":"linear"}"#.into(),
                    }];
                    let layout = crate::audio_clock::ClockLayout::new(
                        &[crate::audio_clock::ClockChunk {
                            frames: audio.total_frames() as i64,
                            start_us: 0,
                            end_us: 3_000_000,
                            segments: &segments,
                        }],
                        rate,
                    )
                    .unwrap();
                    audio.clock =
                        Some(crate::playback::ClockAudio::new(layout, 0, 3_000_000, rate).unwrap());
                    audio.end_us = 3_000_000;
                }
                let total = audio.total_frames();
                let direct = t.path().join(format!("{rate}-{channels}-direct.wav"));
                crate::transcription::wave::write_track(&mut audio, &direct, &|| false).unwrap();
                let key = format!("{rate}-{channels}");
                for core in 0..3 {
                    let first = if core == 0 {
                        0
                    } else {
                        u64::from(rate) * (core - 1)
                    };
                    let end = if core == 2 {
                        total
                    } else {
                        u64::from(rate) * (core + 2)
                    };
                    let path = t.path().join(format!("{key}-{core}.wav"));
                    crate::transcription::wave::write_window(
                        &mut audio,
                        first,
                        end - first,
                        &path,
                        &|| false,
                    )
                    .unwrap();
                    let core_end = if core == 2 {
                        total
                    } else {
                        u64::from(rate) * (core + 1)
                    };
                    storage.cache_analysis_core(
                        &key,
                        &path,
                        rate,
                        total,
                        first,
                        u64::from(rate) * core,
                        core_end,
                        &|| false,
                    );
                    if core < 2 {
                        assert_eq!(
                            storage
                                .write_cached_analysis_pcm(
                                    &key,
                                    &t.path().join("incomplete"),
                                    &|| false
                                )
                                .unwrap(),
                            None
                        );
                    }
                }
                let cached = t.path().join(format!("{key}-cached.wav"));
                assert_eq!(
                    storage
                        .write_cached_analysis_pcm(&key, &cached, &|| false)
                        .unwrap(),
                    Some(count(total, rate))
                );
                assert_eq!(
                    std::fs::read(&direct).unwrap(),
                    std::fs::read(&cached).unwrap(),
                    "{key}"
                );
            }
        }
        assert!(storage.analysis_pcm.lock().unwrap().entries.len() <= 4);
        storage.clear_analysis_pcm();
        assert!(storage.analysis_pcm.lock().unwrap().entries.is_empty());
    }
    #[test]
    fn analysis_pcm_cache_refuses_non_integral_phase_cancellation_and_different_keys() {
        let t = tempfile::tempdir().unwrap();
        let s = Storage::open(t.path().join("data")).unwrap();
        s.cache_analysis_core(
            "x",
            &t.path().join("absent"),
            44100,
            88200,
            1,
            1,
            44101,
            &|| false,
        );
        assert!(s.analysis_pcm.lock().unwrap().entries.is_empty());
        assert!(s
            .write_cached_analysis_pcm("x", &t.path().join("out"), &|| true)
            .is_err());
        assert_eq!(
            s.write_cached_analysis_pcm("other", &t.path().join("out"), &|| false)
                .unwrap(),
            None
        );
    }
}

#[cfg(test)]
#[test]
fn analysis_pcm_keys_bind_clock_source_and_saved_run() {
    let mut source = RecordedAudioPlan {
        manifest_asset_id: "manifest".into(),
        manifest_hash: "a".repeat(64),
        tracks: vec![],
    };
    let first = key(&source, None, 0, None).unwrap();
    assert_ne!(first, key(&source, None, 0, Some(0)).unwrap());
    assert_ne!(
        key(&source, None, 0, Some(0)).unwrap(),
        key(&source, None, 0, Some(1)).unwrap()
    );
    source.manifest_hash = "b".repeat(64);
    assert_ne!(first, key(&source, None, 0, None).unwrap());
}
