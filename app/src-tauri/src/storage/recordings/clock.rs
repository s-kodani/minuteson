use super::{CaptureManifest, CaptureTrack};
use crate::audio_clock::{ClockChunk, ClockLayout, ClockSegment};
use rusqlite::{params, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(test)]
mod ending_tests;
#[cfg(test)]
mod tests;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CaptureClock {
    domain: String,
    pub(super) origin_utc_ms: i64,
    uncertainty_ns: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CaptureEnd {
    method: String,
    microphone_closed_us: i64,
    system_closed_us: i64,
    pub(super) end_us: i64,
}

impl CaptureManifest {
    pub(super) fn validated_unreceived_system(&self, owner: &str) -> Result<bool, String> {
        if !self.system_audio_unreceived {
            return Ok(false);
        }
        let track = &self.system;
        #[cfg(unix)]
        let empty_raw = track.raw_segments.as_ref().is_some_and(|status| {
            status.capacity_owner(owner, crate::recording_segments::Lane::System)
        });
        #[cfg(not(unix))]
        let empty_raw = false;
        if self.schema_version != crate::current_formats::CAPTURE
            || self.state != "saved"
            || !self.errors.is_empty()
            || self.recording_owner_id.as_deref() != Some(owner)
            || !crate::input_levels::valid_owner(owner)
            || self.capture_end.is_none()
            || self.capture_clock.is_none()
            || track.frames != 0
            || track.callback_count != 0
            || track.append_failures != 0
            || track.sample_rate.is_some()
            || track.channels.is_some()
            || track.capture_failure.is_some()
            || track.capture_failure_detail.is_some()
            || track.clock_segments.as_deref() != Some(&[])
            || !empty_raw
        {
            return Err("未取得のPC音声の形式・終了証拠が不正です".into());
        }
        Ok(true)
    }
    pub(super) fn validated_capture_clock(&self) -> Result<Option<&CaptureClock>, String> {
        match (self.schema_version, &self.capture_clock) {
            (crate::current_formats::CAPTURE, Some(clock))
                if clock.domain == "mach_continuous_time"
                    && (0..=8_640_000_000_000_000).contains(&clock.origin_utc_ms)
                    && (0..=9_007_199_254_740_991).contains(&clock.uncertainty_ns) =>
            {
                Ok(Some(clock))
            }
            _ => Err("録音の共通時計・UTC原点または形式版が不正です".into()),
        }
    }

    pub(super) fn validated_capture_end(
        &self,
        owner: &str,
        mic: &CaptureTimeline<'_>,
        system: &CaptureTimeline<'_>,
    ) -> Result<Option<&CaptureEnd>, String> {
        match (self.schema_version, &self.capture_end) {
            (crate::current_formats::CAPTURE, Some(end))
                if self.recording_owner_id.as_deref() == Some(owner)
                    && crate::input_levels::valid_owner(owner)
                    && end.method == "acquisition_closed"
                    && (1..=9_007_199_254_740_991).contains(&end.end_us)
                    && end.microphone_closed_us >= mic.end_us
                    && end.system_closed_us >= system.end_us
                    && end.end_us == end.microphone_closed_us.max(end.system_closed_us) =>
            {
                Ok(Some(end))
            }
            _ => Err("録音終了の所有・両入力の終了証拠または時刻が不正です".into()),
        }
    }
}

pub(super) struct CaptureTimeline<'a> {
    pub(super) start_us: i64,
    pub(super) end_us: i64,
    pub(super) segments: &'a [ClockSegment],
}

impl CaptureTrack {
    pub(super) fn validated_timeline(
        &self,
        corrected: bool,
    ) -> Result<CaptureTimeline<'_>, String> {
        let (rate, _, nominal_end) = self.validated_format()?;
        if !corrected {
            return Err("非対応の旧録音時計です".into());
        }
        let segments = self
            .clock_segments
            .as_deref()
            .ok_or("録音の時刻補正区間がありません")?;
        #[cfg(unix)]
        let segmented = self.raw_segments.is_some();
        #[cfg(not(unix))]
        let segmented = false;
        let limit = if segmented {
            crate::audio_clock::MAX_CLOCK_SEGMENTS
        } else {
            1024
        };
        if segments.is_empty() || segments.len() > limit {
            return Err("録音の時刻補正区間数が不正です".into());
        }
        let start = segments[0].timeline_start_us;
        let end = segments.last().unwrap().timeline_end_us;
        if end > 9_007_199_254_740_991 {
            return Err("録音の補正時刻が数値表現の範囲外です".into());
        }
        let chunk = ClockChunk {
            frames: self.frames,
            start_us: start,
            end_us: end,
            segments,
        };
        let layout = if segmented {
            ClockLayout::capture_track(chunk, rate as u32)?
        } else {
            ClockLayout::new(&[chunk], rate as u32)?
        };
        // Do not require a contiguous range here: explicit positive gaps are
        // evidence to save, not PCM to invent. Readers gate each requested run.
        Ok(CaptureTimeline {
            start_us: layout.start_us,
            end_us: layout.end_us,
            segments,
        })
    }
}

impl CaptureTimeline<'_> {
    pub(super) fn insert_boundary_gaps(
        &self,
        db: &Transaction<'_>,
        track: &str,
        recording_end: i64,
    ) -> Result<(), String> {
        for (start, end, reason) in [
            (0, self.start_us, "capture_start_gap"),
            (self.end_us, recording_end, "capture_stop_gap"),
        ] {
            if start < end {
                db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES (?1,?2,?3,?4,?5)",
                    params![Uuid::new_v4().to_string(), track, start, end, reason]).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    pub(super) fn insert(
        &self,
        db: &Transaction<'_>,
        track: &str,
        chunk: &str,
    ) -> Result<(), String> {
        let mut previous_end = None;
        for segment in self.segments {
            if let Some(end) = previous_end.filter(|end| *end < segment.timeline_start_us) {
                db.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES (?1,?2,?3,?4,'capture_timestamp_gap')", params![Uuid::new_v4().to_string(), track, end, segment.timeline_start_us]).map_err(|e| e.to_string())?;
            }
            db.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![Uuid::new_v4().to_string(), track, chunk, segment.source_frame_start, segment.source_frame_end, segment.timeline_start_us, segment.timeline_end_us, segment.mapping_json]).map_err(|e| e.to_string())?;
            previous_end = Some(segment.timeline_end_us);
        }
        Ok(())
    }
}
