use super::*;

fn workspace(storage: &Storage, meeting: &str) -> PathBuf {
    let session: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM recording_sessions WHERE meeting_id=?1",
            [meeting],
            |r| r.get(0),
        )
        .unwrap();
    let path = storage
        .root
        .join("meetings")
        .join(meeting)
        .join("audio")
        .join(session);
    crate::native_fs::private_directories(&path, true).unwrap();
    path
}

fn wav(storage: &Storage, meeting: &str) -> PathBuf {
    let plan = storage.verify_canonical_audio(meeting, &|| false).unwrap();
    storage.meeting_mix_path(meeting, &plan.tracks[0]).unwrap()
}

#[test]
fn completed_workspace_retires_unknown_audio_and_parts_but_preserves_documents_and_caches() {
    let (_temp, storage, meeting, originals) = tests::fixture();
    let area = workspace(&storage, &meeting);
    for name in [
        "raw-microphone-00000001.caf",
        ".orphan.part",
        "unfinished.bin",
    ] {
        std::fs::write(area.join(name), b"unfinished audio").unwrap();
    }
    std::fs::write(area.join("empty.part"), b"").unwrap();
    for name in [
        "raw-microphone-00000001.json",
        ".final-flac.lock",
        "notes.pdf",
        ".mon-import-manifest-crashed.part",
    ] {
        std::fs::write(area.join(name), b"retained metadata").unwrap();
    }
    let registered_doc = area.join("attachment.caf");
    std::fs::write(&registered_doc, b"document with an audio suffix").unwrap();
    storage
        .register_ready_meeting_asset(
            &meeting,
            registered_doc.strip_prefix(&storage.root).unwrap(),
            "application/octet-stream",
        )
        .unwrap();
    let cache = area.join("input.wav");
    std::fs::write(&cache, b"independently managed analysis cache").unwrap();
    storage
        .register_ready_meeting_asset(
            &meeting,
            cache.strip_prefix(&storage.root).unwrap(),
            "audio/wav",
        )
        .unwrap();
    let outside = storage
        .root
        .join("meetings")
        .join(&meeting)
        .join("user-audio.wav");
    std::fs::write(&outside, b"outside producer workspace").unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    for name in [
        "raw-microphone-00000001.caf",
        ".orphan.part",
        "unfinished.bin",
        "empty.part",
    ] {
        assert!(!area.join(name).exists(), "{name}");
    }
    for name in [
        "raw-microphone-00000001.json",
        ".final-flac.lock",
        "notes.pdf",
        ".mon-import-manifest-crashed.part",
        "attachment.caf",
        "input.wav",
    ] {
        assert!(area.join(name).exists(), "{name}");
    }
    assert!(outside.exists());
    assert!(originals.iter().all(|p| !p.exists()));
    assert!(wav(&storage, &meeting).is_file());
}

#[test]
fn completed_import_removes_managed_copy_and_decoded_caf_but_never_external_original() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("managed")).unwrap();
    let external = temp.path().join("original.wav");
    let (import, area, _) = super::super::imports::tests::pending(&storage, "cleanup-import");
    let original = std::fs::read(area.join("original.wav")).unwrap();
    std::fs::write(&external, &original).unwrap();
    std::fs::write(area.join("empty.part"), b"").unwrap();
    storage.finalize_audio_import(&import.id).unwrap();
    assert!(!area.join("original.wav").exists());
    assert!(!area.join("decoded.caf").exists());
    assert!(!area.join("audio.flac").exists());
    assert!(!area.join("empty.part").exists());
    assert!(area.join("manifest.json").exists());
    assert_eq!(std::fs::read(external).unwrap(), original);
    assert!(wav(&storage, &import.meeting_id).is_file());
}

#[test]
fn active_work_defers_retirement_and_same_content_replacement_is_not_deleted() {
    let (_temp, storage, meeting, originals) = tests::fixture();
    let owner = storage.begin_meeting_work(&meeting, "job", None).unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    assert!(originals.iter().all(|p| p.exists()));
    let canonical = wav(&storage, &meeting);
    drop(owner);
    let bytes = std::fs::read(&originals[0]).unwrap();
    std::fs::rename(&originals[0], originals[0].with_extension("held")).unwrap();
    std::fs::write(&originals[0], bytes).unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| panic!("must reuse WAV"))
        .is_err());
    assert!(originals[0].exists());
    assert!(canonical.is_file());
}

#[test]
fn deletion_interruption_resumes_from_receipt_after_cold_open() {
    let (temp, storage, meeting, originals) = tests::fixture();
    assert!(storage
        .publish_single_wav(
            &meeting,
            &|| originals.iter().any(|p| !p.exists()),
            &|_, _| {}
        )
        .is_err());
    assert!(originals.iter().any(|p| !p.exists()));
    assert!(originals.iter().any(|p| p.exists()));
    let expected = std::fs::read(wav(&storage, &meeting)).unwrap();
    drop(storage);
    let storage = Storage::open(temp.path().to_path_buf()).unwrap();
    storage.finish_pending_meeting_audio().unwrap();
    assert!(originals.iter().all(|p| !p.exists()));
    assert_eq!(std::fs::read(wav(&storage, &meeting)).unwrap(), expected);
}

#[test]
fn failed_database_completion_retries_without_requiring_deleted_inputs() {
    let (_temp, storage, meeting, originals) = tests::fixture();
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_audio_cleanup BEFORE UPDATE OF phase ON meeting_audio_commits WHEN NEW.phase='cleaned' BEGIN SELECT RAISE(ABORT,'synthetic cleanup failure'); END;").unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    assert!(originals.iter().all(|p| !p.exists()));
    let canonical = wav(&storage, &meeting);
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_audio_cleanup")
        .unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| panic!("do not regenerate"))
        .unwrap();
    assert_eq!(wav(&storage, &meeting), canonical);
}

#[cfg(windows)]
#[test]
fn windows_busy_input_retains_receipt_until_handle_is_closed() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_temp, storage, meeting, originals) = tests::fixture();
    let owner = storage.begin_meeting_work(&meeting, "job", None).unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    drop(owner);
    let busy = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&originals[0])
        .unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    assert!(originals[0].exists());
    drop(busy);
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert!(originals.iter().all(|p| !p.exists()));
    assert!(wav(&storage, &meeting).is_file());
}

#[test]
fn interrupted_recording_does_not_authorize_unknown_workspace_cleanup() {
    let (_temp, storage, meeting, _) = tests::fixture();
    let area = workspace(&storage, &meeting);
    let source = {
        let db = storage.connection.lock().unwrap();
        inputs::read(&db, &meeting).unwrap()
    };
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_sessions SET state='interrupted' WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    let tail = area.join("raw-microphone-00000099.caf");
    std::fs::write(&tail, b"unrescued tail").unwrap();
    let mut files = Vec::new();
    cleanup::inventory(
        &storage,
        &meeting,
        Some(&source),
        &area.join("keep.wav"),
        &mut files,
        &|| false,
    )
    .unwrap();
    assert!(!files.iter().any(|f| f.relative.ends_with("00000099.caf")));
    assert!(tail.exists());
}

#[cfg(unix)]
#[test]
fn workspace_links_are_not_followed_or_removed() {
    let (_temp, storage, meeting, _) = tests::fixture();
    let area = workspace(&storage, &meeting);
    let external = tempfile::tempdir().unwrap();
    std::fs::write(external.path().join("keep.caf"), b"external").unwrap();
    std::os::unix::fs::symlink(external.path(), area.join("linked")).unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert!(external.path().join("keep.caf").exists());
    assert!(area.join("linked").is_symlink());
}

#[test]
fn legacy_flac_input_directory_retires_unregistered_parts() {
    let (_temp, storage, meeting, originals) = tests::fixture();
    let (track, asset, frames, size, hash): (String, String, i64, i64, String) = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT t.id,a.id,c.frame_count,a.size_bytes,a.content_hash FROM audio_tracks t JOIN audio_chunks c ON c.track_id=t.id JOIN file_assets a ON a.id=c.file_asset_id WHERE t.meeting_id=?1 AND t.kind='mic'",
            [&meeting],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    let relative = Path::new("meetings")
        .join(&meeting)
        .join("audio")
        .join(&track);
    let area = storage.root.join(&relative);
    crate::native_fs::private_directories(&area, true).unwrap();
    let path = area.join("legacy.flac");
    let mut source = crate::audio::VerifiedAudio::open(
        std::fs::File::open(&originals[0]).unwrap(),
        size as u64,
        &hash,
    )
    .unwrap();
    let saved =
        crate::audio::flac::write_segment(&mut source, 0, frames as u64, &path, &|| false, &|_| {})
            .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE audio_tracks SET format='flac' WHERE id=?1",
            [&track],
        )
        .unwrap();
        db.execute("UPDATE file_assets SET relative_path=?2,media_type='audio/flac',size_bytes=?3,content_hash=?4 WHERE id=?1", params![asset,relative.join("legacy.flac").to_string_lossy().replace('\\',"/"),saved.size as i64,saved.sha256]).unwrap();
    }
    let part = area.join(".unregistered-old.part");
    std::fs::write(&part, b"crashed encoder").unwrap();
    std::fs::write(area.join("retained.json"), b"metadata").unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert!(!path.exists());
    assert!(!part.exists());
    assert!(area.join("retained.json").exists());
    assert!(wav(&storage, &meeting).is_file());
}

#[test]
fn native_partial_plan_admits_track_with_only_unclaimed_crash_parts() {
    use crate::storage::publication_files::Directory;
    let (_temp, storage, meeting, _) = tests::fixture();
    let root = Directory::open(&workspace(&storage, &meeting)).unwrap();
    drop(root.create(".final-flac.lock").unwrap());
    let track = Uuid::new_v4().to_string();
    let chunk = Uuid::new_v4().to_string();
    let area = root.path.parent().unwrap().join(&track);
    crate::native_fs::private_directories(&area, true).unwrap();
    let output = Directory::open(&area).unwrap();
    let uncheckpointed_track = Uuid::new_v4().to_string();
    let uncheckpointed = root.path.parent().unwrap().join(&uncheckpointed_track);
    crate::native_fs::private_directories(&uncheckpointed, true).unwrap();
    let uncheckpointed_directory = Directory::open(&uncheckpointed).unwrap();
    let input_hash = "a".repeat(64);
    root.json(
        "partial-flac.json",
        &serde_json::json!({
            "schema_version":1,"input_hash":input_hash,
            "recording_directory":root.identity,
            "tracks":[
                {"id":track,"directory":output.identity,"chunks":[chunk]},
                {"id":uncheckpointed_track,"directory":uncheckpointed_directory.identity,"chunks":[Uuid::new_v4().to_string()]}
            ]
        }),
        None,
    )
    .unwrap();
    output
        .json(
            &format!(".{chunk}.json"),
            &serde_json::json!({
                "schema_version":1,"input_hash":input_hash,"chunk_id":chunk,
                "staging_name":format!(".{chunk}.{}.part", Uuid::new_v4()),
                "staging_identity":null,"audio":null
            }),
            None,
        )
        .unwrap();
    let part = area.join(".unclaimed-previous-attempt.part");
    std::fs::write(&part, b"created before an identity receipt was durable").unwrap();
    let no_checkpoint_part = uncheckpointed.join(".unclaimed.part");
    std::fs::write(&no_checkpoint_part, b"unregistered encoder leftover").unwrap();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    assert!(!part.exists());
    assert!(!no_checkpoint_part.exists());
    assert!(area.join(format!(".{chunk}.json")).exists());
    assert!(root.path.join("partial-flac.json").exists());
    assert!(root.path.join(".final-flac.lock").exists());
    assert!(wav(&storage, &meeting).is_file());
}

#[cfg(any(target_os = "macos", windows))]
fn move_pending_cleanup(storage: Storage, config: &Path, legacy: &Path, target: &Path) -> Storage {
    use crate::bootstrap::Bootstrap;
    if !config.join("bootstrap.json").exists() {
        crate::bootstrap::set_test_root(config, &config.join("fixture-app-data"), storage.root());
    }
    let bootstrap = Bootstrap::open(config, legacy).unwrap();
    bootstrap.initialize(&storage).unwrap();
    crate::native_fs::private_directories(target, true).unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let revision = bootstrap.view().unwrap().revision;
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    bootstrap
        .begin_move(
            &operation,
            &confirmation,
            target,
            snapshot.manifest_hash(),
            revision,
        )
        .unwrap();
    let mut placement = bootstrap.pending_placement(&operation, &snapshot).unwrap();
    placement.copy(&snapshot, &|| false).unwrap();
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    placement.install_history(&handoff, &|| false).unwrap();
    placement
        .activate_for_restart(&handoff, &confirmation, revision + 1, &|| false)
        .unwrap();
    drop(placement);
    drop(handoff);
    drop(snapshot);
    drop(storage);
    drop(bootstrap);
    Bootstrap::open(config, legacy)
        .unwrap()
        .open_storage()
        .unwrap()
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn pending_cleanup_resumes_after_repeated_native_moves_and_cold_reopens() {
    use crate::bootstrap::Bootstrap;
    let (_temp, storage, meeting, originals) = tests::fixture();
    let owner = storage.begin_meeting_work(&meeting, "job", None).unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    drop(owner);
    let before = std::fs::read(wav(&storage, &meeting)).unwrap();
    let control = tempfile::tempdir().unwrap();
    let root = control.path().canonicalize().unwrap();
    let config = root.join("config");
    let source = storage.root.clone();
    let relative: Vec<_> = originals
        .iter()
        .map(|p| p.strip_prefix(&source).unwrap().to_owned())
        .collect();
    let storage = move_pending_cleanup(storage, &config, &source, &root.join("first"));
    let storage = move_pending_cleanup(storage, &config, &source, &root.join("second"));
    assert!(storage
        .publish_single_wav(
            &meeting,
            &|| relative.iter().any(|p| !storage.root.join(p).exists()),
            &|_, _| panic!("must reuse WAV after relocation")
        )
        .is_err());
    assert!(relative.iter().any(|p| !storage.root.join(p).exists()));
    assert!(relative.iter().any(|p| storage.root.join(p).exists()));
    drop(storage);
    let storage = Bootstrap::open(&config, &source)
        .unwrap()
        .open_storage()
        .unwrap();
    storage.finish_pending_meeting_audio().unwrap();
    assert!(relative.iter().all(|p| !storage.root.join(p).exists()));
    assert_eq!(std::fs::read(wav(&storage, &meeting)).unwrap(), before);
    drop(storage);
    let storage = Bootstrap::open(&config, &source)
        .unwrap()
        .open_storage()
        .unwrap();
    storage.finish_pending_meeting_audio().unwrap();
    assert_eq!(std::fs::read(wav(&storage, &meeting)).unwrap(), before);
    assert!(originals.iter().all(|p| p.is_file()));
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn native_move_never_authorizes_same_content_replacement() {
    let (_temp, storage, meeting, originals) = tests::fixture();
    let owner = storage.begin_meeting_work(&meeting, "job", None).unwrap();
    assert!(storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .is_err());
    drop(owner);
    let relative = originals[0].strip_prefix(&storage.root).unwrap().to_owned();
    let source = storage.root.clone();
    let control = tempfile::tempdir().unwrap();
    let root = control.path().canonicalize().unwrap();
    let storage =
        move_pending_cleanup(storage, &root.join("config"), &source, &root.join("target"));
    let path = storage.root.join(relative);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, path.with_extension("held")).unwrap();
    std::fs::write(&path, bytes).unwrap();
    assert!(storage.finish_pending_meeting_audio().is_err());
    assert!(path.is_file());
    assert!(wav(&storage, &meeting).is_file());
}
