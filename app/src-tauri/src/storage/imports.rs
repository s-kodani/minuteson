use super::*;
use crate::audio::VerifiedAudio;
use crate::audio_import::{AudioInfo, FileEvidence, ImportManifest, ImportOptions};
use crate::local_engine::Fingerprint;
use rusqlite::TransactionBehavior;
use std::fs::File;
use std::io::Read;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportSnapshot {
    pub id: String,
    pub meeting_id: String,
    pub state: String,
    pub original_file_name: String,
    pub info: AudioInfo,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}
fn read(db: &Connection, id: &str) -> Result<ImportSnapshot, String> {
    let row: (String,String,String,String,String,Option<String>,Option<String>) = db.query_row(
        "SELECT i.id,i.meeting_id,i.state,i.original_file_name,p.info_json,i.error_code,p.error_message FROM audio_imports i JOIN audio_import_commits p ON p.import_id=i.id AND p.meeting_id=i.meeting_id WHERE i.id=?1",
        params![id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).map_err(|e|e.to_string())?;
    let info: AudioInfo = serde_json::from_str(&row.4).map_err(|e| e.to_string())?;
    info.validate()?;
    Ok(ImportSnapshot {
        id: row.0,
        meeting_id: row.1,
        state: row.2,
        original_file_name: row.3,
        info,
        error_code: row.5,
        error_message: row.6,
    })
}
pub(super) fn read_options(db: &Connection, id: &str) -> Result<ImportOptions, String> {
    let (json, hash): (String, String) = db
        .query_row(
            "SELECT options_json,options_hash FROM audio_import_options WHERE import_id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if json.len() > 1048576 || format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
        return Err("取り込み条件のハッシュが一致しません".into());
    }
    let options: ImportOptions = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    options.validate()?;
    Ok(options)
}
impl Storage {
    pub(crate) fn import_scratch(&self) -> Result<&Path, String> {
        if self.root.canonicalize().map_err(|e| e.to_string())? != self.root {
            return Err("音声取り込みの保存先は正規の管理パスである必要があります".into());
        }
        Ok(&self.root)
    }
    #[cfg(test)]
    pub(crate) fn begin_audio_import(
        &self,
        title: &str,
        operation: &str,
        file_name: &str,
        source: &Fingerprint,
        info: &AudioInfo,
    ) -> Result<(ImportSnapshot, bool), String> {
        self.begin_audio_import_configured(
            title,
            operation,
            file_name,
            source,
            info,
            &ImportOptions::default(),
        )
    }
    pub(crate) fn begin_audio_import_configured(
        &self,
        title: &str,
        operation: &str,
        file_name: &str,
        source: &Fingerprint,
        info: &AudioInfo,
        options: &ImportOptions,
    ) -> Result<(ImportSnapshot, bool), String> {
        info.validate()?;
        options.validate()?;
        let title = title.trim();
        if title.is_empty()
            || title.chars().count() > 200
            || operation.is_empty()
            || operation.len() > 200
            || source.size == 0
            || source.size > i64::MAX as u64
            || source.sha256.len() != 64
            || file_name.is_empty()
            || file_name.chars().count() > 255
            || file_name.chars().any(char::is_control)
        {
            return Err("取り込みの会議名・操作ID・元音声が不正です".into());
        }
        let text = serde_json::to_string(info).map_err(|e| e.to_string())?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let old: Option<(String,i64,String,String,String,String)> = tx.query_row(
            "SELECT p.import_id,p.source_size,p.source_hash,p.info_json,i.original_file_name,m.title FROM audio_import_commits p JOIN audio_imports i ON i.id=p.import_id JOIN meetings m ON m.id=p.meeting_id WHERE p.operation_id=?1",
            params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(|e|e.to_string())?;
        if let Some((id, size, hash, stored, name, old_title)) = old {
            if size != source.size as i64
                || hash != source.sha256
                || stored != text
                || name != file_name
                || old_title != title
                || read_options(&tx, &id)? != *options
            {
                return Err("同じ操作IDの取り込み条件が一致しません".into());
            }
            return read(&tx, &id).map(|s| (s, false));
        }
        let meeting = Uuid::new_v4().to_string();
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        tx.execute("INSERT INTO meetings(id,title,source_type,lifecycle,created_at,updated_at) VALUES(?1,?2,'audio_import','draft',?3,?3)",params![meeting,title,now]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,detected_format,duration_us,created_at,updated_at) VALUES(?1,?2,'copying',?3,?4,?5,?6,?6)",params![id,meeting,file_name,info.detected_format,info.duration_us,now]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO audio_import_commits(import_id,meeting_id,operation_id,source_size,source_hash,info_json) VALUES(?1,?2,?3,?4,?5,?6)",params![id,meeting,operation,source.size as i64,source.sha256,text]).map_err(|e|e.to_string())?;
        super::vocabulary::save_initial_import_vocabulary(
            &tx,
            &meeting,
            &options.meeting_vocabulary,
            options.expected_global_version,
        )?;
        let options_json = serde_json::to_string(options).map_err(|e| e.to_string())?;
        let options_hash = format!("{:x}", Sha256::digest(options_json.as_bytes()));
        tx.execute("INSERT INTO audio_import_options(import_id,options_json,options_hash) VALUES(?1,?2,?3)",params![id,options_json,options_hash]).map_err(|e|e.to_string())?;
        let result = read(&tx, &id)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((result, true))
    }
    pub(crate) fn audio_import(&self, id: &str) -> Result<ImportSnapshot, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        read(&db, id)
    }
    pub(crate) fn audio_import_directory(
        &self,
        import: &ImportSnapshot,
    ) -> Result<PathBuf, String> {
        self.import_scratch()?;
        Uuid::parse_str(&import.id).map_err(|e| e.to_string())?;
        Uuid::parse_str(&import.meeting_id).map_err(|e| e.to_string())?;
        let path = self.managed_path(
            &PathBuf::from("meetings")
                .join(&import.meeting_id)
                .join("imports")
                .join(&import.id),
        )?;
        for parent in [
            self.root.join("meetings"),
            self.root.join("meetings").join(&import.meeting_id),
            self.root
                .join("meetings")
                .join(&import.meeting_id)
                .join("imports"),
            path.clone(),
        ] {
            if parent.symlink_metadata().is_ok()
                && (parent.canonicalize().map_err(|e| e.to_string())? != parent || !parent.is_dir())
            {
                return Err("取り込み領域が管理パス外を参照しています".into());
            }
            std::fs::create_dir(&parent)
                .or_else(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(path)
    }
    pub(crate) fn audio_import_converting(&self, id: &str) -> Result<(), String> {
        let changed=self.connection.lock().map_err(|_|"DBロックを取得できません")?.execute("UPDATE audio_imports SET state='converting',updated_at=?2 WHERE id=?1 AND state='copying'",params![id,now_ms()?]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("取り込み状態が変更されました".into());
        }
        Ok(())
    }
    pub(crate) fn fail_audio_import(
        &self,
        id: &str,
        code: &str,
        error: &str,
    ) -> Result<(), String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let changed=tx.execute("UPDATE audio_imports SET state='failed',error_code=?2,updated_at=?3 WHERE id=?1 AND state IN ('copying','converting','failed')",params![id,code,now_ms()?]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("保存済み取り込みは失敗状態へ戻せません".into());
        }
        tx.execute(
            "UPDATE audio_import_commits SET error_message=?2 WHERE import_id=?1",
            params![id, error.chars().take(2048).collect::<String>()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(crate) fn audio_import_pending_finalization(
        &self,
        id: &str,
        error: &str,
    ) -> Result<(), String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute("UPDATE audio_imports SET error_code='finalize_pending',updated_at=?2 WHERE id=?1 AND state='converting'",params![id,now_ms()?]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE audio_import_commits SET error_message=?2 WHERE import_id=?1 AND manifest_asset_id IS NULL",params![id,error.chars().take(2048).collect::<String>()]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn finalize_audio_import(&self, id: &str) -> Result<ImportSnapshot, String> {
        self.finalize_audio_import_cancellable(id, &crate::test_support::never_cancelled)
    }
    pub(crate) fn finalize_audio_import_cancellable(
        &self,
        id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<ImportSnapshot, String> {
        if cancelled() {
            return Err("取り込み保存の検証を停止しました".into());
        }
        let before = self.audio_import(id)?;
        if before.state == "ready" {
            return Ok(before);
        }
        if before.state != "converting"
            && !(before.state == "failed" && before.error_code.as_deref() == Some("interrupted"))
        {
            return Err("この取り込みは保存確定可能な状態ではありません".into());
        }
        let base = self.audio_import_directory(&before)?;
        let manifest_path = base.join("manifest.json");
        let evidence = self.checked_import_file(&manifest_path, None, cancelled)?;
        if evidence.size > 16384 {
            return Err("取り込みmanifestが大きすぎます".into());
        }
        let mut bytes = Vec::new();
        File::open(&manifest_path)
            .map_err(|e| e.to_string())?
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 16384 {
            return Err("取り込みmanifestが大きすぎます".into());
        }
        let manifest: ImportManifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let (size, hash): (i64, String) = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT source_size,source_hash FROM audio_import_commits WHERE import_id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if manifest.schema_version != 1
            || manifest.import_id != before.id
            || manifest.meeting_id != before.meeting_id
            || manifest.info != before.info
            || manifest.original_file_name != before.original_file_name
            || manifest.original.size != size as u64
            || manifest.original.sha256 != hash
        {
            return Err("取り込みmanifestと固定条件が一致しません".into());
        }
        manifest.info.validate()?;
        let original_path = base.join(format!("original.{}", manifest.info.detected_format));
        self.checked_import_file(&original_path, Some(&manifest.original), cancelled)?;
        let flac_path = base.join("audio.flac");
        self.checked_import_file(&flac_path, Some(&manifest.audio), cancelled)?;
        let audio = VerifiedAudio::open_cancellable(
            File::open(&flac_path).map_err(|e| e.to_string())?,
            manifest.audio.size,
            &manifest.audio.sha256,
            cancelled,
        )?;
        if audio.format.frames != manifest.info.frames
            || audio.format.sample_rate != manifest.info.sample_rate
            || audio.format.channels != manifest.info.channels
        {
            return Err("取り込みFLACの実音声と固定条件が一致しません".into());
        }
        drop(audio);
        // Recheck immutable evidence immediately before opening the DB commit.
        self.checked_import_file(&manifest_path, Some(&evidence), cancelled)?;
        if cancelled() {
            return Err("取り込み保存の検証を停止しました".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if read(&tx, id)? != before {
            return Err("保存検証中に取り込み状態が変更されました".into());
        }
        read_options(&tx, id)?;
        let now = now_ms()?;
        let original = insert_asset(
            &tx,
            &before.meeting_id,
            &self.root,
            &original_path,
            &manifest.original,
            match manifest.info.detected_format.as_str() {
                "wav" => "audio/wav",
                "mp3" => "audio/mpeg",
                "m4a" => "audio/mp4",
                _ => "audio/flac",
            },
            now,
        )?;
        let converted = insert_asset(
            &tx,
            &before.meeting_id,
            &self.root,
            &flac_path,
            &manifest.audio,
            "audio/flac",
            now,
        )?;
        let manifest_id = insert_asset(
            &tx,
            &before.meeting_id,
            &self.root,
            &manifest_path,
            &evidence,
            "application/json",
            now,
        )?;
        tx.execute("UPDATE audio_import_commits SET manifest_asset_id=?2,manifest_hash=?3,error_message=NULL WHERE import_id=?1 AND manifest_asset_id IS NULL",params![id,manifest_id,evidence.sha256]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE audio_imports SET state='ready',original_asset_id=?2,converted_asset_id=?3,error_code=NULL,updated_at=?4 WHERE id=?1",params![id,original,converted,now]).map_err(|e|e.to_string())?;
        let track = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO audio_tracks(id,meeting_id,import_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,'imported','flac',?4,?5,'ready','{\"timeline_alignment\":\"source\"}',?6,?6)",params![track,before.meeting_id,id,manifest.info.sample_rate,manifest.info.channels,now]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,?5,?6,?7,?8,'ready',?9)",params![Uuid::new_v4().to_string(),before.meeting_id,track,converted,manifest.info.duration_us,manifest.info.frames as i64,manifest.info.sample_rate,manifest.info.channels,now]).map_err(|e|e.to_string())?;
        if tx.execute("UPDATE meetings SET lifecycle='imported',duration_us=?2,lock_version=lock_version+1,updated_at=?3 WHERE id=?1 AND source_type='audio_import' AND lifecycle='draft'",params![before.meeting_id,manifest.info.duration_us,now]).map_err(|e|e.to_string())?!=1 { return Err("会議の取り込み状態が変更されています".into()); }
        super::analysis_flows::ensure_flow(&tx, &before.meeting_id)?;
        let result = read(&tx, id)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
    fn checked_import_file(
        &self,
        path: &Path,
        expected: Option<&FileEvidence>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FileEvidence, String> {
        let metadata = path.symlink_metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || path.canonicalize().map_err(|e| e.to_string())? != path
            || !path.starts_with(&self.root)
        {
            return Err("取り込み音声が管理領域外または通常ファイルではありません".into());
        }
        let proof = Fingerprint::capture(path, cancelled)?;
        let result = FileEvidence {
            size: proof.size,
            sha256: proof.sha256,
        };
        if expected.is_some_and(|e| e.size != result.size || e.sha256 != result.sha256) {
            return Err("取り込み音声のサイズ・ハッシュが一致しません".into());
        }
        crate::native_fs::sync_path(path)?;
        Ok(result)
    }
    pub(crate) fn cleanup_audio_import(&self, id: &str) -> Result<(), String> {
        let snapshot = self.audio_import(id)?;
        let registered: bool = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1)",
                params![snapshot.meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if snapshot.state != "failed" || registered {
            return Err("確定済み・参照済みの取り込み物は整理できません".into());
        }
        let base = self.audio_import_directory(&snapshot)?;
        let mut owned_files = Vec::new();
        for entry in std::fs::read_dir(&base).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name = name.to_str().ok_or("取り込み物の名前が不正です")?;
            let owned = matches!(name, "decoded.caf" | "audio.flac" | "manifest.json")
                || name == format!("original.{}", snapshot.info.detected_format)
                || ((name.starts_with(".mon-flac-") || name.starts_with(".mon-import-manifest-"))
                    && name.ends_with(".part"))
                || name.starts_with(".mon-import-report-");
            if !owned || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err(
                    "取り込み領域に未確認のファイルがあります。削除せず保持しました".into(),
                );
            }
            let proof =
                crate::native_fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            if proof.nlink() != 1 {
                return Err("取り込み領域の共有ファイルは削除せず保持します".into());
            }
            owned_files.push((entry.path(), proof));
        }
        // Delete only the checked list; files arriving after inspection are
        // retained, and remove_dir refuses a newly non-empty directory.
        for (path, proof) in owned_files {
            crate::native_fs::remove_owned(&path, &proof).map_err(|e| e.to_string())?;
        }
        std::fs::remove_dir(base).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn recover_audio_imports(&self) -> Result<Vec<ImportSnapshot>, String> {
        self.recover_audio_imports_cancellable(&crate::test_support::never_cancelled)
    }
    pub(crate) fn recover_audio_imports_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<ImportSnapshot>, String> {
        if self.data_write_blocked() {
            return Ok(vec![]);
        }
        let ids = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut q=db.prepare("SELECT i.id FROM audio_imports i JOIN audio_import_commits p ON p.import_id=i.id WHERE (i.state='failed' AND i.error_code='interrupted') OR (i.state='converting' AND i.error_code='finalize_pending') ORDER BY i.created_at").map_err(|e|e.to_string())?;
            let ids = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            ids.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut recovered = Vec::new();
        for id in ids {
            if cancelled() {
                return Err("取り込み回復の検証を停止しました".into());
            }
            let snapshot = self.audio_import(&id)?;
            let base = self.audio_import_directory(&snapshot)?;
            if base.join("manifest.json").exists() {
                // Complete but invalid evidence is retained for inspection.
                match self.finalize_audio_import_cancellable(&id, cancelled) {
                    Ok(saved) => recovered.push(saved),
                    Err(error) => {
                        if cancelled() {
                            return Err(error);
                        }
                        self.fail_audio_import(&id, "interrupted", &error)?;
                    }
                }
            } else {
                self.fail_audio_import(&id,"interrupted","前回の取り込みは音声確定前に中断しました。元ファイルから再度取り込んでください")?;
                self.cleanup_audio_import(&id)?;
            }
        }
        Ok(recovered)
    }
}
fn insert_asset(
    db: &Connection,
    meeting: &str,
    root: &Path,
    path: &Path,
    e: &FileEvidence,
    media: &str,
    now: i64,
) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_str()
        .ok_or("取り込みパスが不正です")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let id = Uuid::new_v4().to_string();
    let size = i64::try_from(e.size).map_err(|_| "取り込み音声サイズが範囲外です")?;
    db.execute("INSERT INTO file_assets(id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) VALUES(?1,?2,'meeting',?3,?4,?5,'sha256',?6,'ready',?7,?7)",params![id,meeting,relative,media,size,e.sha256,now]).map_err(|e|e.to_string())?;
    Ok(id)
}

#[cfg(test)]
#[path = "import_tests.rs"]
pub(in crate::storage) mod tests;
