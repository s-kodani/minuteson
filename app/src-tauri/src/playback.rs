use crate::audio::{VerifiedAudio, PREVIEW_FRAMES};
use serde::Serialize;
use std::sync::Mutex;
use uuid::Uuid;

mod clock;
mod meeting;
mod preparation;
pub(crate) use clock::ClockAudio;
pub(crate) use meeting::{MeetingAudio, MeetingIntervalAudio};
pub(crate) use preparation::PlaybackPreparations;

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
    pub(crate) clock: Option<ClockAudio>,
}

impl PreparedAudio {
    pub fn total_frames(&self) -> u64 {
        self.clock.as_ref().map_or_else(
            || self.spans.iter().map(|span| span.frames).sum(),
            |clock| clock.frames,
        )
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
        match &mut self.clock {
            Some(clock) => clock.read(
                &mut self.spans,
                self.channels,
                self.sample_rate,
                clock.first_output + offset,
                frames,
            ),
            None => clock::read_raw(&mut self.spans, self.channels, offset, frames),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackMissingRange {
    pub track_kind: String,
    pub start_us: i64,
    pub end_us: i64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub reason_code: String,
}

impl PlaybackMissingRange {
    pub(crate) fn new(
        kind: &str,
        start: i64,
        end: i64,
        reason: &str,
        total_us: i64,
    ) -> Result<Self, String> {
        Self::at_rate(kind,start,end,reason,total_us,meeting::MEETING_RATE)
    }
    pub(crate) fn at_rate(kind:&str,start:i64,end:i64,reason:&str,total_us:i64,rate:u32)->Result<Self,String> {
        if !matches!(rate,16000|48000) || !matches!(kind, "mic" | "system" | "imported" | "processed")
            || start < 0
            || end <= start
            || end > total_us
            || reason.is_empty()
            || reason.len() > 256
            || reason.chars().any(char::is_control)
        {
            return Err("会議音声の欠損記録が不正です".into());
        }
        let start_frame =
            u64::try_from(start as u128 * u128::from(rate) / 1000000).map_err(|_| "欠損位置が範囲外です")?;
        let end_frame = crate::audio_clock::frame_at(end, rate)?;
        Ok(Self {
            track_kind: kind.into(),
            start_us: start,
            end_us: end,
            start_frame,
            end_frame,
            reason_code: reason.into(),
        })
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_ranges: Vec<PlaybackMissingRange>,
}

struct Playback {
    info: PlaybackInfo,
    audio: PreparedPlayback,
    _meeting_owner: Option<crate::storage::meeting_access::MeetingWork>,
}

enum PreparedPlayback {
    Track(PreparedAudio),
    Meeting(MeetingAudio),
    Interval(MeetingIntervalAudio),
}

impl PreparedPlayback {
    fn read_pcm(&mut self, offset: u64, frames: u64) -> Result<Vec<u8>, String> {
        match self {
            Self::Track(audio) => audio.read_pcm(offset, frames),
            Self::Meeting(audio) => audio.read_pcm(offset, frames),
            Self::Interval(audio) => audio.read_pcm(offset, frames),
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
            missing_ranges: vec![],
        };
        self.insert_prepared(info, PreparedPlayback::Track(audio))
    }

    pub(crate) fn insert_meeting(&self, audio: MeetingAudio) -> Result<PlaybackInfo, String> {
        let info = PlaybackInfo {
            token: Uuid::new_v4().to_string(),
            sample_rate: audio.sample_rate,
            channels: 1,
            total_frames: audio.total_frames,
            start_us: 0,
            end_us: audio.end_us,
            track_kind: "mixed".into(),
            // Mixing/resampling is not evidence of physically synchronized capture.
            alignment: "unverified".into(),
            missing_ranges: audio.missing_ranges.clone(),
        };
        self.insert_prepared(info, PreparedPlayback::Meeting(audio))
    }

    pub(crate) fn insert_interval(
        &self,
        audio: MeetingIntervalAudio,
    ) -> Result<PlaybackInfo, String> {
        let info = PlaybackInfo {
            token: Uuid::new_v4().to_string(),
            sample_rate: audio.sample_rate,
            channels: 1,
            total_frames: audio.total_frames,
            start_us: audio.start_us,
            end_us: audio.end_us,
            track_kind: "mixed".into(),
            alignment: "unverified".into(),
            missing_ranges: audio.missing_ranges.clone(),
        };
        self.insert_prepared(info, PreparedPlayback::Interval(audio))
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
            _meeting_owner: None,
        });
        Ok(info)
    }

    pub(crate) fn own_meeting(
        &self,
        info: PlaybackInfo,
        owner: crate::storage::meeting_access::MeetingWork,
    ) -> Result<PlaybackInfo, String> {
        let mut sessions = self.0.lock().map_err(|_| "再生ロックを取得できません")?;
        let session = sessions
            .iter_mut()
            .find(|s| s.info.token == info.token)
            .ok_or("音声の所有が変更されました")?;
        session._meeting_owner = Some(owner);
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
        let mut frames = PREVIEW_FRAMES.min(session.info.total_frames - offset_frames);
        if let PreparedPlayback::Meeting(audio) = &session.audio {
            frames = audio.readable_frames(offset_frames, frames)?;
        }
        if let PreparedPlayback::Interval(audio) = &session.audio {
            frames = audio.readable_frames(offset_frames, frames)?;
        }
        let mut result =
            Vec::with_capacity(12 + frames as usize * session.info.channels as usize * 4);
        result.extend_from_slice(&session.info.sample_rate.to_le_bytes());
        result.extend_from_slice(&session.info.channels.to_le_bytes());
        result.extend_from_slice(&(frames as u32).to_le_bytes());
        result.extend_from_slice(&session.audio.read_pcm(offset_frames, frames)?);
        Ok(result)
    }
}
