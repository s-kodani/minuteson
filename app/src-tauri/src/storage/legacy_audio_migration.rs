//! Removable one-shot bridge. Normal readers never interpret legacy audio.
use super::*;
#[cfg(test)]
mod tests;

pub(super) fn admits_schema(version: usize) -> bool {
    version == 34
}

// Immutable deletion receipts keep their schema-34 fingerprints. Recognizing
// that repository-owned schema is read-only provenance, never new consent.
pub(in crate::storage) fn deletion_schema(
) -> Result<&'static Vec<(String, String, String, String)>, String> {
    static SCHEMA: std::sync::OnceLock<Result<Vec<(String, String, String, String)>, String>> =
        std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").map_err(|e| e.to_string())?;
        for (_, sql) in super::MIGRATIONS.iter().take(34) {
            db.execute_batch(sql).map_err(|e| e.to_string())?;
        }
        let mut query = db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type,name").map_err(|e| e.to_string())?;
        let rows = query.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }).as_ref().map_err(Clone::clone)
}

pub(super) fn upgrade_schema(db: &mut Connection) -> Result<(), Box<dyn Error>> {
    let version = super::verify_migration_history(db)?;
    // This bridge still ends at 35, even when the application adds later
    // migrations. Repeating the completed one-shot bridge stays idempotent.
    if version == 35 || version == super::MIGRATIONS.len() {
        return Ok(());
    }
    if !admits_schema(version) {
        return Err("非対応の移行元DBです".into());
    }
    // Snapshot the legacy DB before changing even the schema. SQLite backup
    // includes committed WAL pages; copying meetings.sqlite3 alone does not.
    super::schema_upgrade::save_snapshot(db, "before-single-wav.sqlite3")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(super::MIGRATION_35)?;
    tx.execute(
        "INSERT INTO schema_migrations(version,checksum,applied_at) VALUES(35,?1,?2)",
        params![
            format!("{:x}", Sha256::digest(super::MIGRATION_35.as_bytes())),
            now_ms()?
        ],
    )?;
    tx.commit()?;
    Ok(())
}

impl Storage {
    pub(crate) fn needs_single_wav_migration(&self) -> Result<bool, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE audio_policy='legacy') OR EXISTS(SELECT 1 FROM audio_imports WHERE audio_policy='legacy') OR EXISTS(SELECT 1 FROM audio_upgrade_progress WHERE phase!='complete')", [], |r| r.get(0)).map_err(|e| e.to_string())
    }
    pub(crate) fn migrate_single_wav(
        &self,
        progress: &impl Fn(usize, usize, &str),
    ) -> Result<(), String> {
        self.require_data_writable()?;
        let meetings: Vec<String> = {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO audio_upgrade_progress(meeting_id,phase,restart_analysis,updated_at)
                SELECT m.id,'pending',EXISTS(SELECT 1 FROM processing_jobs j WHERE j.meeting_id=m.id AND j.state IN ('queued','running','pausing','paused','interrupted')) OR EXISTS(SELECT 1 FROM analysis_flows f JOIN processing_jobs j ON j.id=f.transcription_job_id OR j.id=f.diarization_job_id WHERE f.meeting_id=m.id AND f.stage='attention' AND j.state='failed'),?1
                FROM meetings m WHERE m.lifecycle IN ('recorded','imported')
                AND (EXISTS(SELECT 1 FROM recording_sessions r WHERE r.meeting_id=m.id AND r.audio_policy='legacy')
                     OR EXISTS(SELECT 1 FROM audio_imports i WHERE i.meeting_id=m.id AND i.audio_policy='legacy'))
                ON CONFLICT(meeting_id) DO NOTHING",[now_ms()?]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            let mut q=db.prepare("SELECT meeting_id FROM audio_upgrade_progress WHERE phase!='complete' ORDER BY meeting_id").map_err(|e|e.to_string())?;
            let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let total = meetings.len();
        for (index, meeting) in meetings.iter().enumerate() {
            progress(index, total, meeting);
            let deleted = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                super::deletion::audio::source_deleted(&db, meeting)?
            };
            if !deleted {
                self.publish_single_wav(meeting, &|| false, &|_, _| {})?;
            }
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let restart: bool = tx
                .query_row(
                    "SELECT restart_analysis FROM audio_upgrade_progress WHERE meeting_id=?1",
                    [meeting],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let target = super::analysis_flows::regeneration::target(&tx, meeting)?;
            tx.execute("UPDATE processing_jobs SET state='cancelled',error_code=NULL,finished_at=?2,updated_at=?2 WHERE meeting_id=?1 AND state IN ('queued','running','pausing','paused','interrupted')",params![meeting,now_ms()?]).map_err(|e|e.to_string())?;
            if restart && !deleted {
                let flow = Uuid::new_v4().to_string();
                // Flow identities and fixed plans are immutable. Retire only
                // the coordinator; published revisions and jobs keep history.
                tx.execute("DELETE FROM analysis_flows WHERE meeting_id=?1", [meeting])
                    .map_err(|e| e.to_string())?;
                tx.execute("INSERT INTO analysis_flows(id,meeting_id,stage,created_at,updated_at,initial_replacement_json) VALUES(?2,?1,'pending',?3,?3,?4)",params![meeting,flow,now_ms()?,target.as_ref().map(serde_json::to_string).transpose().map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
                tx.execute("UPDATE audio_upgrade_progress SET replacement_flow_id=?2,replacement_target_json=?3 WHERE meeting_id=?1",params![meeting,flow,target.as_ref().map(serde_json::to_string).transpose().map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
            }
            // Published revisions and drafts are the durable historical
            // results. Old resumable PCM checkpoints are no longer executable.
            // Cancelled job identities cannot resume. Keep their checkpoints as
            // immutable result provenance; the new flow owns fresh job IDs.
            tx.execute(
                "UPDATE recording_sessions SET audio_policy='single_wav_v1' WHERE meeting_id=?1",
                [meeting],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE audio_imports SET audio_policy='single_wav_v1' WHERE meeting_id=?1",
                [meeting],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("UPDATE audio_upgrade_progress SET phase='complete',updated_at=?2 WHERE meeting_id=?1",params![meeting,now_ms()?]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        progress(total, total, "");
        Ok(())
    }
}
