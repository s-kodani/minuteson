//! Registry-owned clips. File intents survive both SQLite rollback and process exit.
use super::*;
use crate::playback::{AudioSpan, PreparedAudio};
use crate::storage::publication_files::{Directory, Identity};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(crate) fn relative(person: &str, sample: &str) -> Result<String, String> {
    for id in [person, sample] {
        if !Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) {
            return Err("登録音声の識別子が不正です".into());
        }
    }
    Ok(format!("voices/{person}/{sample}.wav"))
}
fn directory(root: &Path, person: &str, create: bool) -> Result<Directory, String> {
    let mut dir = Directory::open(root)?;
    for part in ["voices", person] {
        let path = dir.path.join(part);
        dir = match crate::native_fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
                dir.create_child(part)?
            }
            Err(e) => return Err(e.to_string()),
            Ok(m) if m.is_dir() && path.canonicalize().map_err(|e| e.to_string())? == path => {
                Directory::open(&path)?
            }
            _ => return Err("登録音声の保存先が変更されています".into()),
        };
    }
    Ok(dir)
}
pub(in crate::storage) fn file(
    root: &Path,
    path: &str,
    create: bool,
) -> Result<(Directory, String), String> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 3 || parts[0] != "voices" {
        return Err("登録音声のパスが不正です".into());
    }
    let sample = parts[2]
        .strip_suffix(".wav")
        .ok_or("登録音声の拡張子が不正です")?;
    if relative(parts[1], sample)? != path {
        return Err("登録音声のパスが不正です".into());
    }
    Ok((directory(root, parts[1], create)?, parts[2].into()))
}
fn hash(file: &mut std::fs::File) -> Result<(i64, String), String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
        size += n as i64;
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok((size, format!("{:x}", h.finalize())))
}
#[derive(Debug)]
pub(super) struct Clip {
    pub id: String,
    pub path: String,
    pub size: i64,
    pub hash: String,
    pub frames: i64,
}
impl Clip {
    pub(super) fn insert(&self, db: &Connection) -> Result<(), String> {
        db.execute("INSERT INTO voice_sample_audio(id,relative_path,size_bytes,content_hash,frame_count,state) VALUES(?1,?2,?3,?4,?5,'ready') ON CONFLICT(id) DO UPDATE SET relative_path=excluded.relative_path,size_bytes=excluded.size_bytes,content_hash=excluded.content_hash,frame_count=excluded.frame_count,state='ready',error_message=NULL", params![self.id,self.path,self.size,self.hash,self.frames]).map_err(|e|e.to_string())?;
        Ok(())
    }
}
pub(super) struct Cleanup<'a>(pub &'a Storage);
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.0.recover_voice_audio() {
            crate::internal_logs::report_error("voice_audio", &error);
        }
    }
}
impl Storage {
    pub(in crate::storage) fn lock_voice_audio(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        // Refuse overlapping/reentrant maintenance rather than waiting on a
        // snapshot held by this same caller. Durable receipts reconcile panics.
        match self.voice_audio_lock.try_lock() {
            Ok(guard) => Ok(guard),
            Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => Err(
                "登録音声の保存・削除または保存領域の保守が進行中です。完了後に再試行してください"
                    .into(),
            ),
        }
    }
    pub(super) fn stage_voice_clip(
        &self,
        person: &str,
        id: &str,
        meeting: &str,
        track: &str,
        start: i64,
        end: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Clip, String> {
        let mut audio =
            self.prepare_track_audio_cancellable(meeting, track, start, end, cancelled)?;
        if audio.sample_rate != 16000 || audio.channels != 1 || audio.total_frames() == 0 {
            return Err("登録音声の形式が不正です".into());
        }
        let frames = audio.total_frames();
        let data_size = u32::try_from(frames.checked_mul(2).ok_or("登録音声が長すぎます")?)
            .map_err(|_| "登録音声が長すぎます")?;
        let riff = data_size.checked_add(36).ok_or("登録音声が長すぎます")?;
        let path = relative(person, id)?;
        let (dir, leaf) = file(&self.root, &path, true)?;
        let operation = Uuid::new_v4().to_string();
        let tmp = format!(".{operation}.part");
        self.connection.lock().map_err(|_|"DBロックを取得できません")?.execute(
            "INSERT INTO voice_audio_operations(id,sample_key,relative_path,temporary_path,kind) VALUES(?1,?2,?3,?4,'install')",params![operation,id,path,tmp]).map_err(|e|e.to_string())?;
        let mut output = dir.create(&tmp)?;
        let identity =
            serde_json::to_string(&Identity::file(&output)?).map_err(|e| e.to_string())?;
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .execute(
                "UPDATE voice_audio_operations SET identity_json=?2 WHERE id=?1",
                params![operation, identity],
            )
            .map_err(|e| e.to_string())?;
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&riff.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&16000u32.to_le_bytes());
        header.extend_from_slice(&32000u32.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data_size.to_le_bytes());
        output.write_all(&header).map_err(|e| e.to_string())?;
        let mut offset = 0;
        while offset < frames {
            if cancelled() {
                return Err("登録音声の保存を中断しました".into());
            }
            let count = (frames - offset).min(crate::audio::PREVIEW_FRAMES);
            let pcm = audio.read_pcm(offset, count)?;
            let mut bytes = Vec::with_capacity(count as usize * 2);
            for s in pcm.chunks_exact(4) {
                let v = f32::from_le_bytes(s.try_into().unwrap());
                bytes
                    .extend_from_slice(&((v.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
            }
            output.write_all(&bytes).map_err(|e| e.to_string())?;
            offset += count;
        }
        for span in &audio.spans {
            span.audio.check_unchanged()?;
        }
        output.sync_all().map_err(|e| e.to_string())?;
        let (size, content_hash) = hash(&mut output)?;
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .execute(
                "UPDATE voice_audio_operations SET size_bytes=?2,content_hash=?3 WHERE id=?1",
                params![operation, size, content_hash],
            )
            .map_err(|e| e.to_string())?;
        Identity::file(&output)?.require(&dir.required(&tmp, false)?)?;
        if cancelled() {
            return Err("登録音声の保存を中断しました".into());
        }
        dir.rename(&tmp, &leaf, true)?;
        Ok(Clip {
            id: id.into(),
            path,
            size,
            hash: content_hash,
            frames: frames as i64,
        })
    }
    // Caller holds voice_audio_lock. A committed registry row is the keep receipt.
    pub(in crate::storage) fn recover_voice_audio(&self) -> Result<(), String> {
        self.require_data_writable()?;
        if let Err(error) = self.managed_anchor.verify() {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let _ = db.authorizer(Some(|_: rusqlite::hooks::AuthContext<'_>| {
                rusqlite::hooks::Authorization::Deny
            }));
            self.maintenance
                .store(true, std::sync::atomic::Ordering::Release);
            return Err(format!(
                "起動時の保存領域を検証できません。書込みを停止しました: {error}"
            ));
        }
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        )> = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut q=db.prepare("SELECT id,sample_key,relative_path,temporary_path,identity_json,kind FROM voice_audio_operations ORDER BY id").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        for (op, sample, path, tmp, identity, kind) in rows {
            let keep:bool=self.connection.lock().map_err(|_|"DBロックを取得できません")?.query_row("SELECT EXISTS(SELECT 1 FROM voice_sample_audio WHERE id=?1 AND relative_path=?2 AND state='ready')",params![sample,path],|r|r.get(0)).map_err(|e|e.to_string())?;
            if path.rsplit('/').next() != Some(format!("{sample}.wav").as_str()) {
                return Err("登録音声の回収対象が一致しません".into());
            }
            let (dir, leaf) = file(&self.root, &path, true)?;
            let proof: Option<Identity> = identity
                .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
                .transpose()?;
            if keep && kind == "install" {
                self.saved_voice_playback(&sample, &|| false)?
                    .ok_or("確定済みの登録音声がありません")?;
            }
            if kind == "delete" || !keep {
                if let Some(f) = dir.optional(&leaf, false)? {
                    proof
                        .as_ref()
                        .ok_or("登録音声の回収証拠がありません")?
                        .require(&f)?;
                    dir.remove_owned(&leaf, proof.as_ref().unwrap())?;
                }
            }
            if let Some(tmp) = tmp {
                if tmp != format!(".{op}.part") {
                    return Err("登録音声の一時パスが不正です".into());
                }
                if let Some(f) = dir.optional(&tmp, false)? {
                    let identity = match &proof {
                        Some(identity) => identity.clone(),
                        None if f.metadata().map_err(|e| e.to_string())?.len() == 0 => {
                            Identity::file(&f)?
                        }
                        None => return Err("登録音声の一時ファイルの証拠がありません".into()),
                    };
                    identity.require(&f)?;
                    dir.remove_owned(&tmp, &identity)?;
                }
            }
            if kind == "delete" || !keep {
                let parent = Directory::open(dir.path.parent().ok_or("登録音声の親がありません")?)?;
                let person = dir
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("登録音声の人物が不正です")?
                    .to_owned();
                let identity = dir.identity.clone();
                drop(dir);
                parent.remove_empty_child(&person, &identity)?;
            }
            self.connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?
                .execute("DELETE FROM voice_audio_operations WHERE id=?1", [op])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub(crate) fn preserve_voice_audio(
        &self,
        meeting: Option<&str>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let _guard = self.lock_voice_audio()?;
        self.require_data_writable()?;
        self.recover_voice_audio()?;
        let audio_scope = crate::audio::reuse::Scope::join_or_enter()?;
        let samples: Vec<(String, String, Option<String>, Option<String>, i64, i64)> = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            if self.maintenance.load(std::sync::atomic::Ordering::Acquire) {
                return Err("保存領域の保守が進行中です".into());
            }
            let mut q=db.prepare("SELECT s.id,s.person_id,s.source_meeting_id,s.source_track_id,s.start_us,s.end_us FROM voice_samples s LEFT JOIN voice_sample_audio a ON a.id=s.id WHERE coalesce(a.state,'pending')!='ready' AND (?1 IS NULL OR s.source_meeting_id=?1) ORDER BY s.source_meeting_id,s.id").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([meeting], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        let mut errors = Vec::new();
        for (id, person, source, track, start, end) in samples {
            if cancelled() {
                return Err("登録音声の保存を中断しました".into());
            }
            let source_missing = {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                let exists: bool = db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                        [&source],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let known_missing:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM audio_chunks c JOIN file_assets f ON f.id=c.file_asset_id WHERE c.track_id=?1 AND c.meeting_id=?2 AND f.state='missing') AND NOT EXISTS(SELECT 1 FROM audio_chunks c JOIN file_assets f ON f.id=c.file_asset_id WHERE c.track_id=?1 AND c.meeting_id=?2 AND f.state!='missing')",params![track,source],|r|r.get(0)).map_err(|e|e.to_string())?;
                !exists
                    || known_missing
                    || source.is_none()
                    || track.is_none()
                    || super::super::deletion::audio::source_deleted(
                        &db,
                        source.as_deref().unwrap_or(""),
                    )?
            };
            if source_missing {
                self.connection.lock().map_err(|_|"DBロックを取得できません")?.execute("INSERT INTO voice_sample_audio(id,state) VALUES(?1,'unavailable') ON CONFLICT(id) DO UPDATE SET state='unavailable',error_message=NULL",[&id]).map_err(|e|e.to_string())?;
                continue;
            }
            let result = (|| {
                let _work = self.begin_meeting_work(
                    source.as_deref().ok_or("元会議がありません")?,
                    "voice_audio",
                    None,
                )?;
                let clip = self.stage_voice_clip(
                    &person,
                    &id,
                    source.as_deref().unwrap(),
                    track.as_deref().ok_or("元トラックがありません")?,
                    start,
                    end,
                    cancelled,
                )?;
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                clip.insert(&db)
            })();
            if let Err(error) = result {
                self.connection.lock().map_err(|_|"DBロックを取得できません")?.execute("INSERT INTO voice_sample_audio(id,state,error_message) VALUES(?1,'failed',?2) ON CONFLICT(id) DO UPDATE SET state='failed',error_message=excluded.error_message",params![id,error]).map_err(|e|e.to_string())?;
                errors.push(error);
            }
            self.recover_voice_audio()?;
        }
        audio_scope.verify()?;
        if errors.is_empty() {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            self.verify_preserved_voice_audio(&db, meeting)
        } else {
            Err(format!(
                "登録音声を保存できません。再試行してください: {}",
                errors.join("; ")
            ))
        }
    }
    pub(in crate::storage) fn require_preserved_voice_audio(
        &self,
        db: &Connection,
        meeting: &str,
    ) -> Result<(), String> {
        self.verify_preserved_voice_audio(db, Some(meeting))
    }
    fn verify_preserved_voice_audio(
        &self,
        db: &Connection,
        meeting: Option<&str>,
    ) -> Result<(), String> {
        let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM voice_samples s LEFT JOIN voice_sample_audio a ON a.id=s.id WHERE (?1 IS NULL OR s.source_meeting_id=?1) AND coalesce(a.state,'pending') NOT IN ('ready','unavailable'))",[meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
        if pending {
            return Err("登録音声の保存が完了していないため会議・音声を削除できません。ボイス一覧から保存を再試行してください".into());
        }
        let mut q=db.prepare("SELECT a.id,a.relative_path,a.size_bytes,a.content_hash,a.frame_count,s.start_us,s.end_us,s.person_id FROM voice_sample_audio a JOIN voice_samples s ON s.id=a.id WHERE (?1 IS NULL OR s.source_meeting_id=?1) AND a.state='ready'").map_err(|e|e.to_string())?;
        let mut rows = q.query([meeting]).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let id: String = r.get(0).map_err(|e| e.to_string())?;
            let path: String = r.get(1).map_err(|e| e.to_string())?;
            let person: String = r.get(7).map_err(|e| e.to_string())?;
            if path != relative(&person, &id)? {
                return Err("登録音声の所属が一致しません".into());
            }
            self.open_voice_clip(
                r.get(1).map_err(|e| e.to_string())?,
                r.get(2).map_err(|e| e.to_string())?,
                r.get(3).map_err(|e| e.to_string())?,
                r.get(4).map_err(|e| e.to_string())?,
                &|| false,
            )?;
        }
        Ok(())
    }
    fn open_voice_clip(
        &self,
        path: String,
        size: i64,
        hash: String,
        frames: i64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::audio::VerifiedAudio, String> {
        self.managed_anchor.verify()?;
        let (dir, leaf) = file(&self.root, &path, false)?;
        let audio = crate::audio::VerifiedAudio::open_cancellable(
            dir.required(&leaf, false)?,
            size as u64,
            &hash,
            cancelled,
        )?;
        if !audio.format.is_voice_clip() || audio.format.frames != frames as u64 {
            return Err("登録音声の形式・長さが一致しません".into());
        }
        dir.verify()?;
        Ok(audio)
    }
    pub(super) fn saved_voice_playback(
        &self,
        id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<PreparedAudio>, String> {
        let resolve = || -> Result<Option<(String, i64, String, i64, i64, i64, String)>, String> {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            db.query_row("SELECT a.relative_path,a.size_bytes,a.content_hash,a.frame_count,s.start_us,s.end_us,s.person_id FROM voice_sample_audio a JOIN voice_samples s ON s.id=a.id WHERE a.id=?1 AND a.state='ready'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())
        };
        let Some(before) = resolve()? else {
            return Ok(None);
        };
        let (path, size, hash, frames, start, end, person) = before.clone();
        if path != relative(&person, id)? {
            return Err("登録音声の所属が一致しません".into());
        }
        let audio = self.open_voice_clip(path, size, hash, frames, cancelled)?;
        if cancelled() {
            return Err("登録音声の再生準備を取り消しました".into());
        }
        if resolve()?.as_ref() != Some(&before) {
            return Err("音声準備中に登録した声の参照先が変更されました".into());
        }
        Ok(Some(PreparedAudio {
            sample_rate: 16000,
            channels: 1,
            start_us: start,
            end_us: end,
            track_kind: "processed".into(),
            alignment: "unverified".into(),
            spans: vec![AudioSpan {
                audio,
                first_frame: 0,
                frames: frames as u64,
            }],
            clock: None,
        }))
    }
    pub(super) fn journal_voice_deletion(
        &self,
        db: &Connection,
        person: &str,
        sample: Option<&str>,
    ) -> Result<(), String> {
        let mut q=db.prepare("SELECT a.id,a.relative_path FROM voice_sample_audio a JOIN voice_samples s ON s.id=a.id WHERE s.person_id=?1 AND (?2 IS NULL OR a.id=?2) AND a.state='ready'").map_err(|e|e.to_string())?;
        let mut rows = q
            .query(params![person, sample])
            .map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let id: String = r.get(0).map_err(|e| e.to_string())?;
            let path: String = r.get(1).map_err(|e| e.to_string())?;
            if path != relative(person, &id)? {
                return Err("削除する登録音声の所属が一致しません".into());
            }
            let (dir, leaf) = file(&self.root, &path, true)?;
            let identity = dir
                .optional(&leaf, false)?
                .map(|f| {
                    Identity::file(&f)
                        .and_then(|v| serde_json::to_string(&v).map_err(|e| e.to_string()))
                })
                .transpose()?;
            db.execute("INSERT INTO voice_audio_operations(id,sample_key,relative_path,identity_json,kind) VALUES(?1,?2,?3,?4,'delete')",params![Uuid::new_v4().to_string(),id,path,identity]).map_err(|e|e.to_string())?;
        }
        Ok(())
    }
}

pub(in crate::storage) fn assets(db: &Connection) -> Result<Vec<FileAsset>, String> {
    let mut q=db.prepare("SELECT a.id,a.relative_path,a.size_bytes,a.content_hash FROM voice_sample_audio a JOIN voice_samples s ON s.id=a.id WHERE a.state='ready' ORDER BY a.id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok(FileAsset {
                id: r.get(0)?,
                meeting_id: String::new(),
                relative_path: r.get(1)?,
                size_bytes: r.get(2)?,
                content_hash: r.get(3)?,
                media_type: "audio/wav".into(),
                state: "ready".into(),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::voice::tests::{fixture, register, seed_person};
    const NEVER: fn() -> bool = || false;
    #[test]
    fn registered_audio_survives_source_loss_reopen_and_person_rename() {
        let (root, s, m, speaker) = fixture();
        let person = register(&s, &m, &speaker);
        let p = s.voice_people().unwrap().remove(0);
        let sample = p.samples[0].id.clone();
        assert_eq!(p.samples[0].audio_storage_state, "ready");
        let path = relative(&person, &sample).unwrap();
        assert!(s.root().join(&path).is_file());
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE file_assets SET state='missing'", [])
            .unwrap();
        s.manage_voice_person(&person, p.version, Some("新しい名前".into()), None)
            .unwrap();
        drop(s);
        let s = Storage::open(root.path().into()).unwrap();
        assert!(!s.voice_people().unwrap()[0].samples[0].source_available);
        let mut audio = s
            .prepare_voice_sample_audio_cancellable(&sample, &NEVER)
            .unwrap();
        assert_eq!(audio.total_frames(), 16000);
        assert!(!audio.read_pcm(0, 16000).unwrap().is_empty());
        assert!(s.root().join(path).exists());
    }
    #[test]
    fn existing_audio_backfill_crops_exact_interval_and_is_idempotent() {
        let (_root, s, m, _) = fixture();
        let person = seed_person(&s, &m);
        let id = s.voice_people().unwrap()[0].samples[0].id.clone();
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE voice_samples SET start_us=250000,end_us=750000 WHERE id=?1",
                [&id],
            )
            .unwrap();
        s.preserve_voice_audio(None, &NEVER).unwrap();
        let bytes = std::fs::read(s.root().join(relative(&person, &id).unwrap())).unwrap();
        assert_eq!(bytes.len(), 44 + 8000 * 2);
        let samples: Vec<i16> = bytes[44..]
            .chunks_exact(2)
            .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert!(samples[..4000].iter().all(|v| *v == 8192));
        assert!(samples[4000..].iter().all(|v| *v == -16384));
        s.preserve_voice_audio(None, &NEVER).unwrap();
        assert_eq!(
            std::fs::read(s.root().join(relative(&person, &id).unwrap())).unwrap(),
            bytes
        );
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM voice_audio_operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn incomplete_install_is_recovered_and_deleting_sample_removes_its_file() {
        let (_root, s, m, _) = fixture();
        let person = seed_person(&s, &m);
        let orphan = Uuid::new_v4().to_string();
        let clip = s
            .stage_voice_clip(&person, &orphan, &m, "track", 0, 500000, &NEVER)
            .unwrap();
        assert!(s.root().join(&clip.path).exists());
        s.recover_voice_audio().unwrap();
        assert!(!s.root().join(&clip.path).exists());
        s.preserve_voice_audio(None, &NEVER).unwrap();
        let p = s.voice_people().unwrap().remove(0);
        let sample = &p.samples[0].id;
        let path = s.root().join(relative(&person, sample).unwrap());
        assert!(
            !s.manage_voice_person(&person, p.version, None, Some(sample.clone()))
                .unwrap()
                .audio_cleanup_pending
        );
        assert!(!path.exists());
        assert!(s.voice_people().unwrap()[0].samples.is_empty());
    }
    #[test]
    fn preservation_failure_blocks_deletion_and_retry_saves_without_models() {
        let (_root, s, m, _) = fixture();
        seed_person(&s, &m);
        std::fs::write(s.root().join("voices"), b"block creation").unwrap();
        assert!(s.preserve_voice_audio(Some(&m), &NEVER).is_err());
        assert_eq!(
            s.voice_people().unwrap()[0].samples[0].audio_storage_state,
            "failed"
        );
        assert!(s.reserve_meeting_deletion(&m).is_err());
        assert!(s.get_meeting(&m).unwrap().is_some());
        std::fs::remove_file(s.root().join("voices")).unwrap();
        s.preserve_voice_audio(None, &NEVER).unwrap();
        s.reserve_meeting_deletion(&m).unwrap();
        s.release_meeting_deletion(&m).unwrap();
    }
    #[test]
    fn corrupt_owned_audio_is_rejected_and_source_missing_preserves_features() {
        let (_root, s, m, _) = fixture();
        let person = seed_person(&s, &m);
        s.preserve_voice_audio(None, &NEVER).unwrap();
        let id = s.voice_people().unwrap()[0].samples[0].id.clone();
        let path = s.root().join(relative(&person, &id).unwrap());
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[44] ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(s
            .prepare_voice_sample_audio_cancellable(&id, &NEVER)
            .is_err());
        assert!(s.reserve_meeting_deletion(&m).is_err());
        let (_root, s, _, _) = fixture();
        seed_person(&s, "deleted-meeting");
        s.preserve_voice_audio(None, &NEVER).unwrap();
        let sample = s.voice_people().unwrap().remove(0).samples.remove(0);
        assert_eq!(sample.audio_storage_state, "unavailable");
        assert!(!sample.audio_available);
    }
}
