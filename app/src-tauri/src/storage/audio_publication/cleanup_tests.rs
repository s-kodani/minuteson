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
    std::fs::write(area.join("decoded.caf"), b"redundant decoded audio").unwrap();
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
    std::fs::write(area.join("decoded.caf"), b"redundant decoded audio").unwrap();
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
fn startup_upgrade_recensuses_cleaned_meetings_once_without_regenerating_wav() {
    let (temp, storage, meeting, _) = tests::fixture();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let path = wav(&storage, &meeting);
    let bytes = std::fs::read(&path).unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let leftover = workspace(&storage, &meeting).join("old-unregistered.part");
    let wave_leftover = path.parent().unwrap().join(".unclaimed-wave.part");
    std::fs::write(&leftover, b"left by an earlier version").unwrap();
    std::fs::write(&wave_leftover, b"").unwrap();
    let before: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT cleanup_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    storage
        .upgrade_completed_audio_cleanup(&|_, _, _| {})
        .unwrap();
    assert!(!leftover.exists());
    assert!(!wave_leftover.exists());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        metadata.modified().unwrap()
    );
    let after: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT cleanup_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let before: Vec<serde_json::Value> = serde_json::from_str(&before).unwrap();
    let after: Vec<serde_json::Value> = serde_json::from_str(&after).unwrap();
    assert_eq!(&after[..before.len()], &before);
    drop(storage);
    let storage = Storage::open(temp.path().to_path_buf()).unwrap();
    storage
        .upgrade_completed_audio_cleanup(&|_, _, _| panic!("already complete"))
        .unwrap();
    assert!(wav(&storage, &meeting).is_file());
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

#[test]
fn restored_retired_inputs_do_not_block_initial_cleanup() {
    let (_temp, storage, meeting, _) = tests::fixture();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let (restored, id, _) = super::super::backup::published_restore_for_rebinding_test(
        &storage,
        target.path(),
        &meeting,
        &[],
    );
    let before = std::fs::read(wav(&restored, &id)).unwrap();
    let leftover = workspace(&restored, &id).join("restored-leftover.part");
    std::fs::write(&leftover, b"").unwrap();
    restored
        .upgrade_completed_audio_cleanup(&|_, _, _| {})
        .unwrap();
    assert!(!leftover.exists());
    assert_eq!(std::fs::read(wav(&restored, &id)).unwrap(), before);
}

#[test]
fn invalid_wav_prevents_cleanup_and_upgrade_is_retryable() {
    let (_temp, storage, meeting, _) = tests::fixture();
    storage
        .publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let canonical = wav(&storage, &meeting);
    let before = std::fs::read(&canonical).unwrap();
    let leftover = workspace(&storage, &meeting).join("pending.part");
    std::fs::write(&leftover, b"retained until WAV verifies").unwrap();
    std::fs::write(&canonical, b"corrupt WAV").unwrap();
    assert!(storage
        .upgrade_completed_audio_cleanup(&|_, _, _| {})
        .is_err());
    assert!(leftover.exists());
    std::fs::write(&canonical, before).unwrap();
    storage
        .upgrade_completed_audio_cleanup(&|_, _, _| {})
        .unwrap();
    assert!(!leftover.exists());
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
