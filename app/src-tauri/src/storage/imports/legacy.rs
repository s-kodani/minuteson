//! One-time upgrade of pre-compact, uncommitted import workspaces. The old
//! decoded CAF remains intact until canonical WAV publication retires inputs.
use super::*;
use crate::audio_import::{SavedAudio, VoiceCafWriter};
use std::io::Write;

pub(super) fn upgrade(
    storage: &Storage,
    before: &ImportSnapshot,
    base: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let path = base.join("manifest.json");
    let evidence = storage.checked_import_file(&path, None, cancelled)?;
    if evidence.size > 16384 {
        return Err("旧取り込みmanifestが大きすぎます".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut manifest: ImportManifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.schema_version != 2 {
        return Ok(());
    }
    let (size, hash): (i64, String) = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?
        .query_row(
            "SELECT source_size,source_hash FROM audio_import_commits WHERE import_id=?1",
            [&before.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if manifest.audio_format.as_deref() != Some("caf")
        || manifest.saved_audio.is_some()
        || manifest.import_id != before.id
        || manifest.meeting_id != before.meeting_id
        || manifest.info != before.info
        || manifest.original_file_name != before.original_file_name
        || manifest.original.size != size as u64
        || manifest.original.sha256 != hash
    {
        return Err("旧取り込みmanifestと固定条件が一致しません".into());
    }
    manifest.info.validate()?;
    let original = base.join(format!("original.{}", manifest.info.detected_format));
    storage.checked_import_file(&original, Some(&manifest.original), cancelled)?;
    let old_path = base.join("decoded.caf");
    storage.checked_import_file(&old_path, Some(&manifest.audio), cancelled)?;
    let mut audio = VerifiedAudio::open_cancellable(
        File::open(&old_path).map_err(|e| e.to_string())?,
        manifest.audio.size,
        &manifest.audio.sha256,
        cancelled,
    )?;
    audio.require_caf()?;
    if audio.format.frames != manifest.info.frames
        || audio.format.sample_rate != manifest.info.sample_rate
        || audio.format.channels != manifest.info.channels
    {
        return Err("旧取り込みの音声形式・長さが一致しません".into());
    }
    let candidate = tempfile::Builder::new()
        .prefix(".mon-import-voice-")
        .suffix(".part")
        .tempfile_in(base)
        .map_err(|e| e.to_string())?;
    let mut writer = VoiceCafWriter::from_file(
        candidate.reopen().map_err(|e| e.to_string())?,
        manifest.info.sample_rate,
        manifest.info.channels,
    )?;
    let mut first = 0;
    while first < manifest.info.frames {
        if cancelled() {
            return Err("取り込み形式の移行を停止しました".into());
        }
        let count = (manifest.info.frames - first).min(4096);
        let pcm = audio.read_frames(first, count)?;
        let samples: Vec<f32> = pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        writer.push(&samples)?;
        first += count;
    }
    writer.finish()?;
    let proof = storage.checked_import_file(candidate.path(), None, cancelled)?;
    let saved = SavedAudio::for_source(&manifest.info)?;
    let checked = VerifiedAudio::open_cancellable(
        File::open(candidate.path()).map_err(|e| e.to_string())?,
        proof.size,
        &proof.sha256,
        cancelled,
    )?;
    if !saved.matches(&checked.format) {
        return Err("移行後の取り込み音声が不正です".into());
    }
    drop(checked);
    storage.checked_import_file(&path, Some(&evidence), cancelled)?;
    storage.checked_import_file(&old_path, Some(&manifest.audio), cancelled)?;
    storage.checked_import_file(&original, Some(&manifest.original), cancelled)?;
    let target = base.join("decoded-voice.caf");
    if target.symlink_metadata().is_ok() {
        storage.checked_import_file(&target, Some(&proof), cancelled)?;
    } else {
        candidate
            .persist_noclobber(&target)
            .map_err(|e| e.to_string())?;
    }
    crate::native_fs::sync_directory(base)?;
    manifest.schema_version = 3;
    manifest.saved_audio = Some(saved);
    manifest.audio = proof;
    let mut pending = tempfile::Builder::new()
        .prefix(".mon-import-manifest-")
        .suffix(".part")
        .tempfile_in(base)
        .map_err(|e| e.to_string())?;
    pending
        .write_all(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    pending.as_file().sync_all().map_err(|e| e.to_string())?;
    storage.checked_import_file(&path, Some(&evidence), cancelled)?;
    pending.persist(&path).map_err(|e| e.to_string())?;
    crate::native_fs::sync_directory(base)
}
