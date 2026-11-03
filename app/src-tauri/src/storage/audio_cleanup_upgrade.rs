//! Removable, one-shot census for WAVs published before scoped audio cleanup.
//! Call after recording/import recovery and before admitting normal work.
use super::*;

const KEY: &str = "completed_audio_cleanup_v1";

impl Storage {
    pub(crate) fn upgrade_completed_audio_cleanup(
        &self,
        progress: &impl Fn(usize, usize, &str),
    ) -> Result<(), String> {
        self.require_data_writable()?;
        let meetings: Vec<String> = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let done: Option<(String, String)> = db
                .query_row(
                    "SELECT value_json,content_hash FROM app_settings WHERE key=?1",
                    [KEY],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some((value, hash)) = done {
                if value != "{\"version\":1}"
                    || hash != format!("{:x}", Sha256::digest(value.as_bytes()))
                {
                    return Err("音声整理の更新記録が不正です".into());
                }
                return Ok(());
            }
            let mut q = db.prepare("SELECT a.meeting_id FROM meeting_audio_commits a JOIN meetings m ON m.id=a.meeting_id
                JOIN audio_tracks t ON t.id=a.track_id
                WHERE m.lifecycle IN ('recorded','imported') AND t.state='ready'
                ORDER BY a.meeting_id").map_err(|e|e.to_string())?;
            let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        for (index, meeting) in meetings.iter().enumerate() {
            let _owner = self.begin_meeting_work(meeting, "audio_cleanup", None)?;
            progress(index, meetings.len(), meeting);
            self.refresh_completed_audio_cleanup(meeting)?;
        }
        self.managed_anchor.verify()?;
        self.require_data_writable()?;
        let value = "{\"version\":1}";
        self.connection.lock().map_err(|_| "DBロックを取得できません")?
            .execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,?4)",
                params![KEY,value,format!("{:x}",Sha256::digest(value.as_bytes())),now_ms()?]).map_err(|e|e.to_string())?;
        Ok(())
    }
}
