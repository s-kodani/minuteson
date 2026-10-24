//! Read-only producer evidence for an audio census. Not an unlink capability:
//! explicit consent, a durable deletion receipt and exact revalidation follow.
use super::*;
use crate::input_levels;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, PartialEq, Eq)]
pub(in crate::storage) struct WorkspaceFiles(BTreeMap<String, (Stamp, bool)>);
impl WorkspaceFiles {
    pub(in crate::storage) fn hint(
        &self,
        path: &str,
        file: &File,
    ) -> Result<Option<String>, String> {
        let Some((stamp, audio)) = self.0.get(path) else {
            return Ok(None);
        };
        stamp.require(file)?;
        Ok(Some(
            if *audio {
                "recording_flac"
            } else {
                "recording"
            }
            .into(),
        ))
    }
    fn add(&mut self, path: String, stamp: Stamp, audio: bool) -> Result<(), String> {
        if self.0.len() >= 8192 || self.0.insert(path, (stamp, audio)).is_some() {
            return Err("FLAC所有記録の件数・出力先の重複が不正です".into());
        }
        Ok(())
    }
}

// Never create a lock, directory, checkpoint or audio file while inspecting.
// Plain copied JSON cannot mint ownership. A selected native placement can
// supply qualified aliases for its own new inodes, never unknown leftovers.
pub(in crate::storage) fn inspect_workspaces(
    storage: &Storage,
    meeting: &str,
    sessions: &BTreeSet<String>,
    cancelled: &impl Fn() -> bool,
) -> Result<WorkspaceFiles, String> {
    storage.managed_anchor.verify()?;
    if !input_levels::valid_owner(meeting) {
        return Err("FLAC検査の会議IDが不正です".into());
    }
    let mut found = WorkspaceFiles::default();
    let mut ids = BTreeSet::new();
    for session in sessions {
        partial::check_cancel(cancelled)?;
        if !input_levels::valid_owner(session) {
            return Err("FLAC検査の録音IDが不正です".into());
        }
        let relative = format!("meetings/{meeting}/audio/{session}");
        let path = storage.root.join(&relative);
        match crate::native_fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        let root = Directory::open(&path)?;
        // A read-only shared lock excludes the producer's exclusive converter.
        // An incomplete capture may have neither a workspace lock nor a plan.
        let lock = match root.optional(LOCK, false) {
            Ok(lock) => lock,
            // An unknown/non-private sidecar cannot prove native ownership.
            // The generic census still verifies the complete tree and rejects
            // links. Do not block review of unrelated raw audio on this hint.
            Err(_) => continue,
        };
        if let Some(lock) = &lock {
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                return Err("FLAC変換中のため音声の確認を開始できません".into());
            }
        }
        let mut documents = Vec::new();
        for name in [PLAN, PARTIAL_PLAN] {
            partial::check_cancel(cancelled)?;
            let Some((bytes, stamp)) = root.read_json(name).unwrap_or(None) else {
                continue;
            };
            let Ok(mut plan) = serde_json::from_slice::<Plan>(&bytes) else {
                // Damaged/unproven metadata stays retained. Neither output
                // names nor this failure grant ownership of intermediate PCM.
                continue;
            };
            #[cfg(any(target_os = "macos", windows))]
            relocation::effective_plan(storage, &root.path, name, &bytes, &mut plan)?;
            // Backup or an unadmitted copy preserves bytes, not ownership.
            // Registered audio is still classified separately by the DB.
            if plan.recording_directory != root.identity {
                continue;
            }
            if lock.is_none()
                || plan.schema_version != 1
                || !valid_hash(&plan.input_hash)
                || plan.tracks.is_empty()
                || plan.tracks.len() > 2
            {
                return Err("FLAC変換計画の形式・排他記録が不正です".into());
            }
            found.add(format!("{relative}/{name}"), stamp.clone(), false)?;
            documents.push((name.to_string(), stamp));
            for track in &plan.tracks {
                partial::check_cancel(cancelled)?;
                if !input_levels::valid_owner(&track.id)
                    || !ids.insert(track.id.clone())
                    || sessions.contains(&track.id)
                    || track.chunks.is_empty()
                    || track.chunks.len() > 1024
                {
                    return Err("FLAC出力トラックのID・区間数が不正です".into());
                }
                let track_relative = format!("meetings/{meeting}/audio/{}", track.id);
                match crate::native_fs::symlink_metadata(storage.root.join(&track_relative)) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.to_string()),
                    Ok(_) => {}
                }
                let directory = Directory::open(&storage.root.join(&track_relative))?;
                if directory.identity != track.directory {
                    // Do not adopt a replacement directory, even with names
                    // and receipts copied from the original producer.
                    continue;
                }
                let mut receipts = Vec::new();
                for chunk in &track.chunks {
                    partial::check_cancel(cancelled)?;
                    if !input_levels::valid_owner(chunk) || !ids.insert(chunk.clone()) {
                        return Err("FLAC出力区間のIDが不正・重複しています".into());
                    }
                    let checkpoint_name = format!(".{chunk}.json");
                    let Some((bytes, stamp)) = directory.read_json(&checkpoint_name)? else {
                        continue;
                    };
                    let mut checkpoint: Checkpoint =
                        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    #[cfg(any(target_os = "macos", windows))]
                    relocation::effective_checkpoint(
                        storage,
                        &directory.path,
                        &checkpoint_name,
                        &bytes,
                        &mut checkpoint,
                    )?;
                    if checkpoint.schema_version != 1
                        || checkpoint.input_hash != plan.input_hash
                        || checkpoint.chunk_id != *chunk
                        || !valid_staging(&checkpoint.staging_name, chunk)
                        || (checkpoint.staging_identity.is_none() && checkpoint.audio.is_some())
                    {
                        return Err("FLACチェックポイントの所有記録が一致しません".into());
                    }
                    found.add(
                        format!("{track_relative}/{checkpoint_name}"),
                        stamp.clone(),
                        false,
                    )?;
                    receipts.push((checkpoint_name, stamp));
                    // Creation can precede durable identity publication.
                    // Such a leftover is deliberately not classified audio.
                    let Some(identity) = checkpoint.staging_identity else {
                        continue;
                    };
                    let final_name = format!("{chunk}.flac");
                    let staging = directory.optional(&checkpoint.staging_name, false)?;
                    let final_file = directory.optional(&final_name, false)?;
                    if staging.is_some() && final_file.is_some() {
                        return Err("FLAC途中音声と公開音声が重複しています".into());
                    }
                    if let Some(file) = staging {
                        if Identity::file(&file)? == identity {
                            found.add(
                                format!("{track_relative}/{}", checkpoint.staging_name),
                                Stamp::read(&file)?,
                                true,
                            )?;
                        }
                    }
                    if let Some(file) = final_file {
                        if checkpoint.audio.is_some() && Identity::file(&file)? == identity {
                            found.add(
                                format!("{track_relative}/{final_name}"),
                                Stamp::read(&file)?,
                                true,
                            )?;
                        }
                    }
                }
                directory.verify()?;
                for (name, stamp) in receipts {
                    stamp.require(&directory.required(&name, false)?)?;
                }
            }
        }
        root.verify()?;
        for (name, stamp) in documents {
            stamp.require(&root.required(&name, false)?)?;
        }
    }
    storage.managed_anchor.verify()?;
    Ok(found)
}
pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn valid_staging(name: &str, chunk: &str) -> bool {
    name.strip_prefix(&format!(".{chunk}."))
        .and_then(|s| s.strip_suffix(".part"))
        .is_some_and(input_levels::valid_owner)
}
