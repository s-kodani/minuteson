//! Verified publication and source retirement for newly captured/imported audio.
use super::*;
pub(in crate::storage) mod inputs;
mod mix;
#[cfg(test)]
pub(in crate::storage) mod tests;
mod writer;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RetiredAudio {
    relative: String,
    bytes: u64,
    hash: String,
}

fn inventory(
    root: &Path,
    directory: &Path,
    keep: &Path,
    files: &mut Vec<RetiredAudio>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err("音声移行対象にリンクがあります".into());
        }
        if kind.is_dir() {
            inventory(root, &path, keep, files)?;
            continue;
        }
        if !kind.is_file() || path == keep {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            ext.as_str(),
            "caf" | "flac" | "wav" | "mp3" | "m4a" | "aac" | "ogg" | "opus" | "aiff" | "aif"
        ) {
            continue;
        }
        let (bytes, hash) = super::hash_file(&path)?;
        files.push(RetiredAudio {
            relative: path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
            bytes: bytes as u64,
            hash,
        });
    }
    Ok(())
}

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
            let root = self.root.canonicalize().map_err(|e| e.to_string())?;
            let directory = self.managed_path(&Path::new("meetings").join(meeting))?;
            let mut cleanup = Vec::new();
            inventory(&root, &directory, &path, &mut cleanup)?;
            let partial = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                super::transcription::partial_recording_committed(&db, meeting)?
            };
            if partial {
                // A recovered sealed prefix proves no acquisition end. Its
                // unregistered raw tail is recovery evidence, never disposable
                // merely because the selected prefix became a canonical WAV.
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                cleanup.retain(|f|db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND relative_path=?2 AND media_type LIKE 'audio/%')",params![meeting,f.relative],|r|r.get::<_,bool>(0)).unwrap_or(false));
            }
            let registered: Vec<(String, i64, String)> = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                let mut q=db.prepare("SELECT relative_path,size_bytes,content_hash FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND id!=?2 AND state='ready'").map_err(|e|e.to_string())?;
                let rows = q
                    .query_map(params![meeting, c.asset_id], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
            };
            for (relative, stored_size, stored_hash) in registered {
                if let Some(file) = cleanup.iter().find(|f| f.relative == relative) {
                    if stored_size < 0
                        || file.bytes != stored_size as u64
                        || file.hash != stored_hash
                    {
                        return Err(
                            "保存済み原音声が変更されています。原音声は整理せず保持します".into(),
                        );
                    }
                    continue;
                }
                let p = self.managed_path(Path::new(&relative))?;
                if p == path {
                    continue;
                }
                let (bytes, hash) = super::hash_file(&p)?;
                if bytes != stored_size || hash != stored_hash {
                    return Err(
                        "保存済み原音声が変更されています。原音声は整理せず保持します".into(),
                    );
                }
                cleanup.push(RetiredAudio {
                    relative,
                    bytes: bytes as u64,
                    hash,
                });
            }
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
        self.verify_canonical_audio(meeting, cancelled)?;
        let (phase, text): (String, String) = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT phase,cleanup_json FROM meeting_audio_commits WHERE meeting_id=?1",
                [meeting],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if phase == "cleaned" {
            return Ok(());
        }
        let files: Vec<RetiredAudio> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let keep =
            self.meeting_mix_path(meeting, &self.canonical_audio_plan(meeting)?.tracks[0])?;
        crate::audio::reuse::release_all();
        for file in &files {
            if cancelled() {
                return Err("原音声の整理を中断しました".into());
            }
            let relative = Path::new(&file.relative);
            if !relative.starts_with(Path::new("meetings").join(meeting)) {
                return Err("音声整理の対象が会議領域外です".into());
            }
            let path = self.managed_path(relative)?;
            if path == keep {
                return Err("正式音声を削除対象にはできません".into());
            }
            match std::fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
                Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                    return Err("音声整理の実体が変更されています".into())
                }
                Ok(_) => {}
            }
            let (size, hash) = super::hash_file(&path)?;
            if size as u64 != file.bytes || hash != file.hash {
                return Err("整理予定の原音声が変更されています".into());
            }
            let expected = self
                .root
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(relative);
            if path.canonicalize().map_err(|e| e.to_string())? != expected {
                return Err("音声整理の対象にリンクがあります".into());
            }
            crate::local_engine::verification::release(&path)?;
            self.require_data_writable()?;
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for file in files {
            tx.execute(
                "UPDATE file_assets SET state='missing' WHERE meeting_id=?1 AND relative_path=?2",
                params![meeting, file.relative],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE meeting_audio_commits SET phase='cleaned' WHERE meeting_id=?1",
            [meeting],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}
