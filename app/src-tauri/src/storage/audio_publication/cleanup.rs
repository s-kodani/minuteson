//! Retire inputs only after canonical publication. Unknown leftovers are scoped
//! to completed producer workspaces, never the entire meeting or external input.
use super::*;
use crate::native_fs::Metadata;
use crate::storage::publication_files::Identity;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(any(target_os = "macos", windows))]
pub(in crate::storage) mod relocation;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::storage) struct RetiredAudio {
    pub relative: String,
    pub bytes: u64,
    pub hash: String,
    pub(in crate::storage) identity: Identity,
}

fn check(storage: &Storage, cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        return Err("原音声の整理を中断しました".into());
    }
    storage.require_data_writable()?;
    storage.managed_anchor.verify()?;
    Ok(())
}

fn same(before: &Metadata, after: &Metadata) -> Result<bool, String> {
    Ok(Identity::read(before)? == Identity::read(after)?
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
        && after.nlink() == 1)
}

fn scoped_path(storage: &Storage, meeting: &str, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if !relative.starts_with(Path::new("meetings").join(meeting)) {
        return Err("音声整理の対象が会議領域外です".into());
    }
    storage.managed_path(relative)
}

fn snapshot(
    storage: &Storage,
    meeting: &str,
    relative: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(RetiredAudio, Metadata), String> {
    check(storage, cancelled)?;
    let path = scoped_path(storage, meeting, relative)?;
    let before = crate::native_fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.is_file()
        || before.file_type().is_symlink()
        || before.nlink() != 1
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("音声整理の実体・リンクが不正です".into());
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    if !same(
        &before,
        &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
    )? {
        return Err("音声整理の対象が差し替えられています".into());
    }
    let mut sha = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0; 65536];
    loop {
        check(storage, cancelled)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
        bytes += n as u64;
    }
    if bytes != before.len()
        || !same(
            &before,
            &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
        )?
        || !same(
            &before,
            &crate::native_fs::symlink_metadata(&path).map_err(|e| e.to_string())?,
        )?
    {
        return Err("音声整理の対象が読込み中に変更されました".into());
    }
    Ok((
        RetiredAudio {
            relative: relative.into(),
            bytes,
            hash: format!("{:x}", sha.finalize()),
            identity: Identity::read(&before)?,
        },
        before,
    ))
}

fn keep_document(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "json"
            | "lock"
            | "txt"
            | "md"
            | "pdf"
            | "doc"
            | "docx"
            | "rtf"
            | "csv"
            | "tsv"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "html"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "webp"
    ) || name.starts_with(".mon-import-manifest-")
        || name.starts_with(".mon-import-report-")
}

struct Census<'a, C> {
    storage: &'a Storage,
    meeting: &'a str,
    keep: &'a Path,
    protected: BTreeSet<String>,
    seen: BTreeSet<String>,
    files: &'a mut Vec<RetiredAudio>,
    cancelled: &'a C,
}
impl<C: Fn() -> bool> Census<'_, C> {
    fn visit(&mut self, relative: &Path) -> Result<(), String> {
        check(self.storage, self.cancelled)?;
        let directory = self.storage.managed_path(relative)?;
        let metadata = match std::fs::symlink_metadata(&directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            value => value.map_err(|e| e.to_string())?,
        };
        // Never traverse links (including Windows junctions/canonical aliases).
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || directory.canonicalize().map_err(|e| e.to_string())? != directory
        {
            return Ok(());
        }
        for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let relative = relative.join(entry.file_name());
            let text = relative
                .to_str()
                .ok_or("音声整理パスが不正です")?
                .replace('\\', "/");
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if self.protected.contains(&text) || entry.path() == self.keep || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                self.visit(&relative)?;
            } else if kind.is_file()
                && !keep_document(&entry.path())
                && self.seen.insert(text.clone())
            {
                // The caller admitted this *completed* producer's private area.
                // A receipt is not required for empty/crashed audio leftovers.
                self.files
                    .push(snapshot(self.storage, self.meeting, &text, self.cancelled)?.0);
            }
        }
        Ok(())
    }
}

pub(super) fn inventory(
    storage: &Storage,
    meeting: &str,
    source: Option<&RecordedAudioPlan>,
    keep: &Path,
    files: &mut Vec<RetiredAudio>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut workspaces = BTreeSet::new();
    let mut completed_sessions = BTreeSet::new();
    let mut registered = BTreeMap::new();
    let mut protected = BTreeSet::new();
    {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // Archived source_json deliberately preserves old provenance IDs.
        // Use current FK relationships after additive restore, not those IDs.
        let mut q = db.prepare("SELECT i.input_track_id FROM track_inputs i JOIN meeting_audio_commits a ON a.track_id=i.derived_track_id AND a.meeting_id=i.meeting_id WHERE a.meeting_id=?1 AND i.role='meeting_mix'").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([meeting], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut tracks: Vec<String> = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        if tracks.is_empty() {
            if let Some(source) = source {
                tracks = source.tracks.iter().map(|t| t.id.clone()).collect();
            }
        }
        for track in tracks {
            let mut q = db
                .prepare(
                    "SELECT file_asset_id FROM audio_chunks WHERE track_id=?1 AND meeting_id=?2",
                )
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map(params![track, meeting], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            for id in rows {
                ids.insert(id.map_err(|e| e.to_string())?);
            }
            let (session, import, format): (Option<String>, Option<String>, String) = db
                .query_row(
                    "SELECT session_id,import_id,format FROM audio_tracks WHERE id=?1 AND meeting_id=?2",
                    params![track, meeting],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(|e| e.to_string())?;
            if let Some(session) = session {
                let complete: bool = db.query_row("SELECT state='saved' FROM recording_sessions WHERE id=?1 AND meeting_id=?2", params![session, meeting], |r| r.get(0)).map_err(|e|e.to_string())?;
                if complete && crate::input_levels::valid_owner(&session) {
                    completed_sessions.insert(session.clone());
                    workspaces.insert(
                        Path::new("meetings")
                            .join(meeting)
                            .join("audio")
                            .join(session),
                    );
                    if format == "flac" && crate::input_levels::valid_owner(&track) {
                        workspaces.insert(
                            Path::new("meetings")
                                .join(meeting)
                                .join("audio")
                                .join(&track),
                        );
                    }
                }
            }
            if let Some(import) = import {
                let (complete, original): (bool, Option<String>) = db.query_row("SELECT state='ready',original_asset_id FROM audio_imports WHERE id=?1 AND meeting_id=?2", params![import, meeting], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
                if let Some(original) = original {
                    ids.insert(original);
                }
                if complete && crate::input_levels::valid_owner(&import) {
                    workspaces.insert(
                        Path::new("meetings")
                            .join(meeting)
                            .join("imports")
                            .join(import),
                    );
                }
            }
        }
        let mut q = db.prepare("SELECT id,relative_path,size_bytes,content_hash,media_type,state FROM file_assets WHERE meeting_id=?1").map_err(|e|e.to_string())?;
        let mut rows = q.query([meeting]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let (id, relative, size, hash, media): (String, String, i64, String, String) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                row.get(3).map_err(|e| e.to_string())?,
                row.get(4).map_err(|e| e.to_string())?,
            );
            if ids.contains(&id) && media.starts_with("audio/") {
                // Restore remaps asset paths/IDs, but retirement receipts retain
                // the original source provenance. Do not demand omitted bytes.
                let state: String = row.get(5).map_err(|e| e.to_string())?;
                if state == "missing" && super::super::meeting_audio::superseded_asset(&db, &id)? {
                    continue;
                }
                registered.insert(relative, (size, hash));
            } else {
                // Includes analysis caches and documents even with audio suffixes.
                protected.insert(relative);
            }
        }
    }
    let producers = crate::storage::recordings::flac::inspect_workspaces(
        storage,
        meeting,
        &completed_sessions,
        cancelled,
    )?;
    workspaces.extend(producers.directories().map(Path::to_path_buf));
    if !workspaces.is_empty() {
        if let Some(parent) = keep
            .parent()
            .and_then(|p| p.strip_prefix(&storage.root).ok())
        {
            let output_area = Path::new("meetings")
                .join(meeting)
                .join("audio")
                .join("meeting-wave");
            if parent.starts_with(output_area) {
                workspaces.insert(parent.to_path_buf());
            }
        }
    }
    for (relative, (size, hash)) in registered {
        let path = scoped_path(storage, meeting, &relative)?;
        if path == keep || protected.contains(&relative) {
            continue;
        }
        if let Some(old) = files.iter().find(|f| f.relative == relative) {
            if size < 0 || old.bytes != size as u64 || old.hash != hash {
                return Err("保存済み原音声と整理記録が一致しません".into());
            }
            continue;
        }
        let (file, _) = snapshot(storage, meeting, &relative, cancelled)?;
        if size < 0 || file.bytes != size as u64 || file.hash != hash {
            return Err("保存済み原音声が変更されています。原音声は保持します".into());
        }
        files.push(file);
    }
    let seen = files.iter().map(|f| f.relative.clone()).collect();
    let mut census = Census {
        storage,
        meeting,
        keep,
        protected,
        seen,
        files,
        cancelled,
    };
    for workspace in workspaces {
        census.visit(&workspace)?;
    }
    Ok(())
}

pub(super) fn finish(
    storage: &Storage,
    meeting: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check(storage, cancelled)?;
    let canonical = storage.verify_canonical_audio(meeting, cancelled)?;
    let keep = storage.meeting_mix_path(meeting, &canonical.tracks[0])?;
    let (phase, text): (String, String) = storage
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
    if storage
        .meeting_owners
        .lock()
        .map_err(|_| "会議の実行状態を確認できません")?
        .get(meeting)
        .is_some_and(|work| {
            work.values()
                .any(|w| !matches!(w.kind.as_str(), "recording" | "import" | "audio_cleanup"))
        })
    {
        return Err("音声を使用中の処理が完了してから整理を再開してください".into());
    }
    crate::audio::reuse::release_all();
    for expected in &files {
        check(storage, cancelled)?;
        // Additive restore preserves historical producer paths in the ledger.
        // They are provenance, never permission to unlink another meeting.
        if !Path::new(&expected.relative).starts_with(Path::new("meetings").join(meeting)) {
            continue;
        }
        let path = scoped_path(storage, meeting, &expected.relative)?;
        if path == keep {
            return Err("正式音声を削除対象にはできません".into());
        }
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        crate::local_engine::verification::release(&path)?;
        let (actual, observed) = snapshot(storage, meeting, &expected.relative, cancelled)?;
        let mut identity = expected.identity.clone();
        #[cfg(any(target_os = "macos", windows))]
        crate::storage::recordings::flac::relocation::effective_cleanup_identity(
            storage,
            meeting,
            expected,
            &mut identity,
        )?;
        if actual.bytes != expected.bytes
            || actual.hash != expected.hash
            || identity != actual.identity
        {
            return Err("整理予定の原音声が変更されています".into());
        }
        check(storage, cancelled)?;
        crate::native_fs::remove_owned(&path, &observed).map_err(|e| e.to_string())?;
        crate::native_fs::sync_directory(path.parent().ok_or("音声整理先が不正です")?)?;
    }
    let mut db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    for file in files {
        if !Path::new(&file.relative).starts_with(Path::new("meetings").join(meeting)) {
            continue;
        }
        tx.execute("UPDATE file_assets SET state='missing' WHERE meeting_id=?1 AND relative_path=?2 AND media_type LIKE 'audio/%'", params![meeting,file.relative]).map_err(|e|e.to_string())?;
    }
    tx.execute(
        "UPDATE meeting_audio_commits SET phase='cleaned' WHERE meeting_id=?1",
        [meeting],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
