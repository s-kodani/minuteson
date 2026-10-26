use super::*;
use crate::playback::{MeetingAudio, MeetingIntervalAudio};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MeetingPlaybackMode {
    Mixed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MeetingPlaybackRequest {
    pub(crate) meeting_id: String,
    pub(crate) mode: MeetingPlaybackMode,
    pub(crate) start_reference: Option<MeetingPlaybackStartReference>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum MeetingPlaybackStartReference {
    Utterance {
        analysis_id: String,
        utterance_id: String,
        expected_content_hash: String,
        start_us: i64,
        end_us: i64,
    },
    DraftUtterance {
        request: DraftUtteranceAudioRequest,
    },
    Interval {
        start_us: i64,
        end_us: i64,
        expected_duration_us: i64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MeetingIntervalRequest {
    pub(crate) meeting_id: String,
    pub(crate) mode: MeetingPlaybackMode,
    pub(crate) start_us: i64,
    pub(crate) end_us: i64,
    pub(crate) expected_duration_us: i64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeetingAudioSummary {
    pub(crate) schema_version: u32,
    pub(crate) duration_us: i64,
    pub(crate) modes: Vec<MeetingPlaybackMode>,
}

impl Storage {
    fn resolve_meeting_playback_start(
        &self,
        request: &MeetingPlaybackRequest,
        duration: i64,
    ) -> Result<Option<(String, i64, i64, String)>, String> {
        let Some(reference) = &request.start_reference else {
            return Ok(None);
        };
        let stamp = match reference {
            MeetingPlaybackStartReference::Utterance {
                analysis_id,
                utterance_id,
                expected_content_hash,
                start_us,
                end_us,
            } => {
                let stamp = self.resolve_utterance_audio_reference(
                    &request.meeting_id,
                    analysis_id,
                    utterance_id,
                )?;
                if stamp.1 != *start_us || stamp.2 != *end_us || stamp.3 != *expected_content_hash {
                    return Err("確認した発言と再生開始位置が一致しません".into());
                }
                stamp
            }
            MeetingPlaybackStartReference::DraftUtterance { request: draft } => {
                if draft.meeting_id != request.meeting_id {
                    return Err("分割発言と再生する会議が一致しません".into());
                }
                let track = self.resolve_draft_utterance_audio_reference(draft)?;
                (
                    track,
                    draft.expected_start_us,
                    draft.expected_end_us,
                    draft.expected_content_hash.clone(),
                )
            }
            MeetingPlaybackStartReference::Interval {
                start_us,
                end_us,
                expected_duration_us,
            } => {
                if *expected_duration_us != duration {
                    return Err(
                        "確認後に会議音声の時間が変更されました。区間を確認し直してください".into(),
                    );
                }
                (String::new(), *start_us, *end_us, String::new())
            }
        };
        if stamp.1 < 0
            || stamp.2 <= stamp.1
            || stamp.2 > duration
            || duration > 9_007_199_254_740_991
        {
            return Err("再生開始位置と会議音声の時間が一致しません".into());
        }
        Ok(Some(stamp))
    }

    pub(crate) fn meeting_audio_summary(
        &self,
        meeting: &str,
    ) -> Result<MeetingAudioSummary, String> {
        let plan = self.canonical_audio_plan(meeting)?;
        Ok(MeetingAudioSummary {
            schema_version: 1,
            duration_us: plan.tracks[0].end_us,
            modes: vec![MeetingPlaybackMode::Mixed],
        })
    }

    pub(crate) fn prepare_meeting_interval_cancellable(
        &self,
        request: &MeetingIntervalRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MeetingIntervalAudio, String> {
        if request.start_us < 0
            || request.end_us <= request.start_us
            || request.end_us > request.expected_duration_us
            || request.expected_duration_us <= 0
            || request.expected_duration_us > 9_007_199_254_740_991
        {
            return Err("Markdownの再生区間・確認済み音声時間が不正です".into());
        }
        let audio = self.prepare_meeting_audio_cancellable(
            &MeetingPlaybackRequest {
                start_reference: None,
                meeting_id: request.meeting_id.clone(),
                mode: request.mode,
            },
            cancelled,
        )?;
        if audio.end_us != request.expected_duration_us {
            return Err(
                "確認後に会議音声の時間が変更されました。区間を確認し直してください".into(),
            );
        }
        if cancelled() {
            return Err("会議音声の準備を中止しました".into());
        }
        MeetingIntervalAudio::new(audio, request.start_us, request.end_us)
    }
    #[cfg(test)]
    pub(crate) fn prepare_meeting_audio(
        &self,
        request: &MeetingPlaybackRequest,
    ) -> Result<MeetingAudio, String> {
        self.prepare_meeting_audio_cancellable(request, &crate::test_support::never_cancelled)
    }

    pub(crate) fn prepare_meeting_audio_cancellable(
        &self,
        request: &MeetingPlaybackRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MeetingAudio, String> {
        if !matches!(request.mode, MeetingPlaybackMode::Mixed) {
            return Err("会議音声の再生方式が不正です".into());
        }
        let plan = self.verify_canonical_audio(&request.meeting_id, cancelled)?;
        let track = &plan.tracks[0];
        let before = self.resolve_meeting_playback_start(request, track.end_us)?;
        let audio = self.prepare_meeting_track_audio_cancellable(
            &request.meeting_id,
            &track.id,
            0,
            track.end_us,
            cancelled,
        )?;
        if self.canonical_audio_plan(&request.meeting_id)? != plan
            || self.resolve_meeting_playback_start(request, track.end_us)? != before
        {
            return Err("再生準備中に会議音声の参照が変わりました".into());
        }
        MeetingAudio::with_rate(vec![audio], track.end_us, vec![], 16000)
    }
}
