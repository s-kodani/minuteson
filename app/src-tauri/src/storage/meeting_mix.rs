//! All consumers resolve the same canonical WAV. No playback/analysis copies.
use super::transcription::{RecordedAudioPlan, TrackPlan};
use super::*;

impl Storage {
    pub(crate) fn ensure_meeting_mix(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(u64, u64),
    ) -> Result<RecordedAudioPlan, String> {
        let plan = self.verify_canonical_audio(meeting, cancelled)?;
        let frames = plan.tracks[0].chunks[0].frames as u64;
        progress(frames, frames);
        Ok(plan)
    }
    pub(crate) fn verify_meeting_mix_sources(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.verify_canonical_audio(meeting, cancelled).map(|_| ())
    }
    pub(crate) fn meeting_mix_path(
        &self,
        meeting: &str,
        track: &TrackPlan,
    ) -> Result<PathBuf, String> {
        if track.kind != "processed" || track.chunks.len() != 1 {
            return Err("共通WAVの参照が不正です".into());
        }
        let relative:String=self.connection.lock().map_err(|_|"DBロックを取得できません")?.query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2 AND state='ready' AND content_hash=?3",
            params![track.chunks[0].asset_id,meeting,track.chunks[0].hash],|r|r.get(0)).map_err(|e|e.to_string())?;
        self.managed_path(Path::new(&relative))
    }
}
