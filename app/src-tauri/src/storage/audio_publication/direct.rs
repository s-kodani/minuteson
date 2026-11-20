//! Adopt already canonical PCM without decoding or rewriting it. File checks
//! precede the short DB transaction; source retirement uses the existing ledger.
use super::*;
use crate::storage::publication_files::{Directory, Stamp};

impl Storage {
    pub(super) fn adopt_import_wav(
        &self,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(u64, u64),
    ) -> Result<bool, String> {
        let source = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let is_import: bool = db
                .query_row(
                    "SELECT source_type='audio_import' FROM meetings WHERE id=?1",
                    [meeting],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !is_import {
                return Ok(false);
            }
            inputs::read(&db, meeting)?
        };
        if source.tracks.len() != 1 {
            return Ok(false);
        }
        let original = &source.tracks[0];
        if original.kind != "imported" || original.chunks.len() != 1 {
            return Ok(false);
        }
        let chunk = &original.chunks[0];
        let (format, relative): (String, String) = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            db.query_row("SELECT t.format,f.relative_path FROM audio_tracks t JOIN audio_chunks c ON c.track_id=t.id JOIN file_assets f ON f.id=c.file_asset_id WHERE t.id=?1 AND t.meeting_id=?2", params![original.id,meeting], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?
        };
        if format != "wav" {
            return Ok(false);
        }
        progress(0, chunk.frames as u64);
        crate::operation_observation::stage_for("import", "import_verifying_audio", None, "bytes");
        let path = self.managed_path(Path::new(&relative))?;
        let directory = Directory::open(path.parent().ok_or("取り込み音声の保存先がありません")?)?;
        let leaf = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("取り込み音声名が不正です")?;
        let file = directory.required(leaf, false)?;
        let stamp = Stamp::read(&file)?;
        let mut audio = crate::audio::VerifiedAudio::open_cancellable(
            file,
            chunk.size as u64,
            &chunk.hash,
            cancelled,
        )?;
        audio.require_wav()?;
        if audio.format.frames != chunk.frames as u64
            || audio.format.sample_rate != 16000
            || audio.format.channels != 1
        {
            return Err("取り込みWAVの形式と保存記録が一致しません".into());
        }
        let mut retired = Vec::new();
        cleanup::inventory(self, meeting, Some(&source), &path, &mut retired, cancelled)?;
        if cancelled() {
            return Err("音声保存を中断しました。元音声は保持しています".into());
        }
        audio.check_unchanged()?;
        stamp.require(&directory.required(leaf, false)?)?;
        progress(chunk.frames as u64, chunk.frames as u64);
        crate::operation_observation::stage_for("import", "import_committing_audio", None, "files");
        if cancelled() {
            return Err("音声保存を中断しました。元音声は保持しています".into());
        }
        audio.check_unchanged()?;
        stamp.require(&directory.required(leaf, false)?)?;
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if inputs::read(&tx, meeting)? != source {
            return Err("保存中に取り込み音声が変更されました".into());
        }
        if super::super::meeting_audio::read(&tx, meeting)?.is_some() {
            return Ok(true);
        }
        let current_path: String = tx
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![chunk.asset_id, meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if current_path != relative {
            return Err("保存中に取り込み音声の保存先が変更されました".into());
        }
        let track = &original.id;
        let now = now_ms()?;
        let end = chunk
            .frames
            .checked_mul(1_000_000)
            .ok_or("WAV時間が範囲外です")?
            / 16000;
        let config = serde_json::json!({"policy":super::super::meeting_audio::POLICY,"directImportWav":true});
        // The schema assigns each file asset to exactly one chunk. Promote
        // that existing track/chunk instead of inventing a second reference.
        tx.execute(
            "DELETE FROM track_clock_segments WHERE track_id=?1",
            [track],
        )
        .map_err(|e| e.to_string())?;
        if tx.execute("UPDATE audio_tracks SET kind='processed',processing_config_json=?3,updated_at=?4 WHERE id=?1 AND meeting_id=?2 AND kind='imported' AND format='wav' AND state='ready'",params![track,meeting,config.to_string(),now]).map_err(|e|e.to_string())? != 1 {
            return Err("取り込み音声の正式登録条件が変更されました".into());
        }
        if tx.execute("UPDATE audio_chunks SET end_us=?4 WHERE track_id=?1 AND meeting_id=?2 AND file_asset_id=?3 AND sequence=0 AND state='ready'",params![track,meeting,chunk.asset_id,end]).map_err(|e|e.to_string())? != 1 {
            return Err("取り込みWAVのチャンクが変更されました".into());
        }
        tx.execute("INSERT INTO meeting_audio_commits(meeting_id,track_id,manifest_asset_id,manifest_hash,frames,source_json,cleanup_json,phase,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'committed',?8)", params![meeting,track,source.manifest_asset_id,source.manifest_hash,chunk.frames,serde_json::to_string(&source).map_err(|e|e.to_string())?,serde_json::to_string(&retired).map_err(|e|e.to_string())?,now]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
}
