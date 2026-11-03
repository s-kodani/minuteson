//! Verified publication and source retirement for newly captured/imported audio.
use super::*;
pub(in crate::storage) mod cleanup;
use cleanup::RetiredAudio;
#[cfg(test)]
mod cleanup_tests;
pub(in crate::storage) mod inputs;
mod mix;
#[cfg(test)]
pub(in crate::storage) mod tests;
mod writer;

impl Storage {
    pub(crate) fn finish_pending_meeting_audio(&self) -> Result<(), String> {
        let meetings: Vec<String> = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut q=db.prepare("SELECT m.id FROM meetings m LEFT JOIN meeting_audio_commits a ON a.meeting_id=m.id
                WHERE m.lifecycle IN ('recorded','imported') AND (a.phase IS NULL OR a.phase!='cleaned')
                AND (EXISTS(SELECT 1 FROM recording_sessions r WHERE r.meeting_id=m.id AND r.audio_policy='single_wav_v1' AND r.audio_manifest_hash IS NOT NULL)
                     OR EXISTS(SELECT 1 FROM audio_imports i WHERE i.meeting_id=m.id AND i.audio_policy='single_wav_v1' AND i.state='ready')) ORDER BY m.id").map_err(|e|e.to_string())?;
            let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        for meeting in meetings {
            let deleted = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                super::deletion::audio::source_deleted(&db, &meeting)?
            };
            if !deleted {
                self.publish_single_wav(&meeting, &|| false, &|_, _| {})?;
            }
        }
        Ok(())
    }
    /// Explicit finalization/migration entry, never a read-time fallback.
    pub(crate) fn publish_single_wav(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(u64, u64),
    ) -> Result<(), String> {
        self.require_data_writable()?;
        let existing = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            super::meeting_audio::read(&db, meeting)?
        };
        if existing.is_none() {
            let plan = self.build_pending_meeting_wav(meeting, cancelled, progress)?;
            let source = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                inputs::read(&db, meeting)?
            };
            let track = &plan.tracks[0];
            let c = &track.chunks[0];
            self.verify_meeting_asset_cancellable(meeting, &c.asset_id, &c.hash, cancelled)?;
            let path = self.meeting_mix_path(meeting, track)?;
            let mut cleanup = Vec::new();
            cleanup::inventory(self, meeting, Some(&source), &path, &mut cleanup, cancelled)?;
            if cancelled() {
                return Err("音声保存を中断しました。原音声は保持しています".into());
            }
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            if inputs::read(&tx, meeting)? != source {
                return Err("移行中に入力音声が変更されました".into());
            }
            tx.execute("INSERT INTO meeting_audio_commits(meeting_id,track_id,manifest_asset_id,manifest_hash,frames,source_json,cleanup_json,phase,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'committed',?8)",
                params![meeting,track.id,plan.manifest_asset_id,plan.manifest_hash,c.frames,serde_json::to_string(&source).map_err(|e|e.to_string())?,serde_json::to_string(&cleanup).map_err(|e|e.to_string())?,now_ms()?]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE audio_tracks SET processing_config_json=json_set(processing_config_json,'$.policy',?2) WHERE id=?1",params![track.id,super::meeting_audio::POLICY]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        self.finish_single_wav_cleanup(meeting, cancelled)
    }

    fn finish_single_wav_cleanup(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        cleanup::finish(self, meeting, cancelled)
    }

    /// Explicit one-shot upgrade entry. Preserve prior retirement evidence and
    /// never rebuild the WAV or adopt changed files from an existing receipt.
    pub(super) fn refresh_completed_audio_cleanup(&self, meeting: &str) -> Result<(), String> {
        let canonical = self.verify_canonical_audio(meeting, &|| false)?;
        let keep = self.meeting_mix_path(meeting, &canonical.tracks[0])?;
        let previous: String = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT cleanup_json FROM meeting_audio_commits WHERE meeting_id=?1",
                [meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut files: Vec<RetiredAudio> =
            serde_json::from_str(&previous).map_err(|e| e.to_string())?;
        let count = files.len();
        cleanup::inventory(self, meeting, None, &keep, &mut files, &|| false)?;
        if files.len() != count {
            self.require_data_writable()?;
            self.managed_anchor.verify()?;
            let updated = self.connection.lock().map_err(|_| "DBロックを取得できません")?
                .execute("UPDATE meeting_audio_commits SET cleanup_json=?2,phase='committed' WHERE meeting_id=?1 AND cleanup_json=?3",
                    params![meeting,serde_json::to_string(&files).map_err(|e|e.to_string())?,previous]).map_err(|e|e.to_string())?;
            if updated != 1 {
                return Err("音声整理記録が変更されています".into());
            }
        }
        self.finish_single_wav_cleanup(meeting, &|| false)
    }
}
