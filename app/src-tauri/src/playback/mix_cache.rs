//! Disposable, process-local mixed PCM. Unnamed files disappear when their
//! owners close; neither originals nor the portable meeting format changes.
use super::MeetingAudio;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use tauri::{AppHandle, Manager};

const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 4;

pub(crate) struct CachedMix {
    file: Mutex<File>,
    bytes: u64,
}
impl CachedMix {
    pub(super) fn read(&self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| "再生用音声ロックを取得できません")?;
        file.seek(SeekFrom::Start(offset * 4))
            .map_err(|e| e.to_string())?;
        let mut pcm = vec![0; frames as usize * 4];
        file.read_exact(&mut pcm).map_err(|e| e.to_string())?;
        Ok(pcm)
    }
}

#[derive(Default)]
struct State {
    cached: Vec<(String, Arc<CachedMix>)>,
    jobs: HashMap<String, Arc<AtomicBool>>,
    suspended: usize,
    suspended_meetings: HashMap<String, usize>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    idle: Condvar,
}
#[derive(Clone, Default)]
pub(crate) struct MixCache(Arc<Shared>);
struct Job {
    cache: MixCache,
    meeting: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        let mut state = self.cache.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.jobs.remove(&self.meeting);
        self.cache.0.idle.notify_all();
    }
}
impl MixCache {
    pub(crate) fn get(&self, key: &str) -> Option<Arc<CachedMix>> {
        self.0
            .state
            .lock()
            .ok()?
            .cached
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, audio)| audio.clone())
    }
    fn reserve(&self, meeting: &str) -> Option<Job> {
        let mut state = self.0.state.lock().ok()?;
        // Only one expensive background render at a time.
        if state.suspended != 0
            || state.suspended_meetings.contains_key(meeting)
            || !state.jobs.is_empty()
        {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.jobs.insert(meeting.into(), cancel.clone());
        Some(Job {
            cache: self.clone(),
            meeting: meeting.into(),
            cancel,
        })
    }
    fn render(&self, mut audio: MeetingAudio, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let bytes = audio
            .total_frames
            .checked_mul(4)
            .ok_or("再生用音声が大きすぎます")?;
        if audio.cache_key.is_empty() || bytes > MAX_BYTES || self.get(&audio.cache_key).is_some() {
            return Ok(());
        }
        let mut file = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut offset = 0;
        while offset < audio.total_frames {
            if cancelled() {
                return Err("再生用音声の準備を中断しました".into());
            }
            // Gaps retain their meeting-time positions and remain unplayable.
            // Sparse holes in this temporary file are never emitted as PCM.
            if let Some(end) = audio
                .missing_ranges
                .iter()
                .filter(|gap| gap.start_frame <= offset && offset < gap.end_frame)
                .map(|gap| gap.end_frame)
                .max()
            {
                offset = end;
                continue;
            }
            let mut frames = crate::audio::PREVIEW_FRAMES.min(audio.total_frames - offset);
            for gap in &audio.missing_ranges {
                if gap.start_frame > offset {
                    frames = frames.min(gap.start_frame - offset);
                }
            }
            let pcm = audio.read_pcm(offset, frames)?;
            file.seek(SeekFrom::Start(offset * 4))
                .map_err(|e| e.to_string())?;
            file.write_all(&pcm).map_err(|e| e.to_string())?;
            offset += frames;
        }
        file.set_len(bytes).map_err(|e| e.to_string())?;
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| "再生用音声ロックを取得できません")?;
        if state.suspended != 0 || cancelled() {
            return Err("再生用音声の準備を中断しました".into());
        }
        // Bound cached disk space, including recordings of up to about 93 min.
        while state.cached.len() >= MAX_ENTRIES
            || state.cached.iter().map(|(_, item)| item.bytes).sum::<u64>() + bytes > MAX_BYTES
        {
            state.cached.remove(0);
        }
        state.cached.push((
            audio.cache_key,
            Arc::new(CachedMix {
                file: Mutex::new(file),
                bytes,
            }),
        ));
        Ok(())
    }

    // Maintenance admission prevents new renderers and drains existing ones
    // before deletion/backup/relocation starts. Drop all disposable audio too.
    pub(crate) fn suspend(&self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.suspended += 1;
        for cancel in state.jobs.values() {
            cancel.store(true, Ordering::Release);
        }
        state.cached.clear();
        while !state.jobs.is_empty() {
            state = self.0.idle.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub(crate) fn resume(&self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.suspended = state.suspended.saturating_sub(1);
    }

    pub(crate) fn suspend_meeting(&self, meeting: &str) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        *state.suspended_meetings.entry(meeting.into()).or_default() += 1;
        if let Some(cancel) = state.jobs.get(meeting) {
            cancel.store(true, Ordering::Release);
        }
        while state.jobs.contains_key(meeting) {
            state = self.0.idle.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub(crate) fn resume_meeting(&self, meeting: &str) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = state.suspended_meetings.get_mut(meeting) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.suspended_meetings.remove(meeting);
            }
        }
    }
}

pub(crate) fn warm(app: &AppHandle, meeting: &str) {
    let owner = match app
        .state::<crate::storage::Storage>()
        .begin_meeting_work(meeting, "playback", None)
    {
        Ok(owner) => owner,
        Err(_) => return,
    };
    let cache = app.state::<MixCache>().inner().clone();
    let Some(job) = cache.reserve(meeting) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancelled = || {
            job.cancel.load(Ordering::Acquire)
                || app.state::<crate::storage::Storage>().data_write_blocked()
        };
        let request = crate::storage::MeetingPlaybackRequest {
            start_reference: None,
            meeting_id: job.meeting.clone(),
            mode: crate::storage::MeetingPlaybackMode::Both,
        };
        let result = app
            .state::<crate::storage::Storage>()
            .prepare_meeting_audio_cancellable(&request, &cancelled)
            .and_then(|audio| cache.render(audio, &cancelled));
        if let Err(error) = result {
            if !cancelled() {
                eprintln!("再生用ミックスの準備を省略しました: {error}");
            }
        }
        drop(owner);
        drop(job);
    });
}

#[cfg(test)]
mod tests;
