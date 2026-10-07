use crate::audio::{VerifiedAudio, PREVIEW_FRAMES};
use serde::Serialize;
use std::sync::Mutex;
use uuid::Uuid;

mod meeting;
pub(crate) use meeting::MeetingAudio;

pub struct AudioSpan {
    pub audio: VerifiedAudio,
    pub first_frame: u64,
    pub frames: u64,
}

pub struct PreparedAudio {
    pub sample_rate: u32,
    pub channels: u32,
    pub start_us: i64,
    pub end_us: i64,
    pub track_kind: String,
    pub alignment: String,
    pub spans: Vec<AudioSpan>,
}

impl PreparedAudio {
    pub fn total_frames(&self) -> u64 {
        self.spans.iter().map(|span| span.frames).sum()
    }

    pub fn read_pcm(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        if frames == 0
            || frames > PREVIEW_FRAMES
            || offset
                .checked_add(frames)
                .is_none_or(|end| end > self.total_frames())
        {
            return Err("音声の読出し範囲が不正です".into());
        }
        let mut result = Vec::with_capacity(frames as usize * self.channels as usize * 4);
        let mut span_start = 0;
        let end = offset + frames;
        for span in &mut self.spans {
            let span_end = span_start + span.frames;
            let from = offset.max(span_start);
            let to = end.min(span_end);
            if to > from {
                result.extend_from_slice(
                    &span
                        .audio
                        .read_frames(span.first_frame + from - span_start, to - from)?,
                );
            }
            span_start = span_end;
            if span_start >= end {
                break;
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackInfo {
    pub token: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub total_frames: u64,
    pub start_us: i64,
    pub end_us: i64,
    pub track_kind: String,
    pub alignment: String,
}

struct Playback {
    info: PlaybackInfo,
    audio: PreparedPlayback,
}

enum PreparedPlayback {
    Track(PreparedAudio),
    Meeting(MeetingAudio),
}

impl PreparedPlayback {
    fn read_pcm(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        match self {
            Self::Track(audio) => audio.read_pcm(offset, frames),
            Self::Meeting(audio) => audio.read_pcm(offset, frames),
        }
    }
}

#[derive(Default)]
pub struct PlaybackStore(Mutex<Vec<Playback>>);

impl PlaybackStore {
    pub fn insert(&self, audio: PreparedAudio) -> Result<PlaybackInfo, String> {
        let info = PlaybackInfo {
            token: Uuid::new_v4().to_string(),
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            total_frames: audio.total_frames(),
            start_us: audio.start_us,
            end_us: audio.end_us,
            track_kind: audio.track_kind.clone(),
            alignment: audio.alignment.clone(),
        };
        self.insert_prepared(info, PreparedPlayback::Track(audio))
    }

    pub(crate) fn insert_meeting(&self, audio: MeetingAudio) -> Result<PlaybackInfo, String> {
        let info = PlaybackInfo {
            token: Uuid::new_v4().to_string(),
            sample_rate: meeting::MEETING_RATE,
            channels: 1,
            total_frames: audio.total_frames,
            start_us: 0,
            end_us: audio.end_us,
            track_kind: audio.track_kind.clone(),
            // Mixing/resampling is not evidence of physically synchronized capture.
            alignment: "unverified".into(),
        };
        self.insert_prepared(info, PreparedPlayback::Meeting(audio))
    }

    fn insert_prepared(
        &self,
        info: PlaybackInfo,
        audio: PreparedPlayback,
    ) -> Result<PlaybackInfo, String> {
        let mut sessions = self.0.lock().map_err(|_| "再生ロックを取得できません")?;
        // Bound retained file handles even if a frontend goes away mid-request.
        if sessions.len() >= 4 {
            sessions.remove(0);
        }
        sessions.push(Playback {
            info: info.clone(),
            audio,
        });
        Ok(info)
    }

    pub fn close(&self, token: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "再生ロックを取得できません")?
            .retain(|session| session.info.token != token);
        Ok(())
    }

    // Binary packet: rate, channels, frames (three u32 LE), then interleaved
    // float32 LE samples. No full-recording transfer, paths or native pointers.
    pub fn read(&self, token: &str, offset_frames: u64) -> Result<Vec<u8>, String> {
        let mut sessions = self.0.lock().map_err(|_| "再生ロックを取得できません")?;
        let session = sessions
            .iter_mut()
            .find(|session| session.info.token == token)
            .ok_or("再生セッションがありません。発言を選び直してください")?;
        if offset_frames >= session.info.total_frames {
            return Err("再生位置が範囲外です".into());
        }
        let frames = PREVIEW_FRAMES.min(session.info.total_frames - offset_frames);
        let mut result =
            Vec::with_capacity(12 + frames as usize * session.info.channels as usize * 4);
        result.extend_from_slice(&session.info.sample_rate.to_le_bytes());
        result.extend_from_slice(&session.info.channels.to_le_bytes());
        result.extend_from_slice(&(frames as u32).to_le_bytes());
        result.extend_from_slice(&session.audio.read_pcm(offset_frames, frames)?);
        Ok(result)
    }
}
