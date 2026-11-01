use super::*;
use crate::audio::tests::caf_fixture;
use crate::audio_import;
use std::cell::Cell;

fn info() -> AudioInfo {
    AudioInfo {
        schema_version: 1,
        detected_format: "wav".into(),
        codec: "pcm".into(),
        sample_rate: 16000,
        channels: 2,
        frames: 64000,
        duration_us: 4_000_000,
    }
}
fn wav() -> Vec<u8> {
    let data: Vec<u8> = (0..64000)
        .flat_map(|_| [8192_i16, -16384_i16])
        .flat_map(|n| n.to_le_bytes())
        .collect();
    let mut h = Vec::new();
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16_u32.to_le_bytes());
    h.extend_from_slice(&1_u16.to_le_bytes());
    h.extend_from_slice(&2_u16.to_le_bytes());
    h.extend_from_slice(&16000_u32.to_le_bytes());
    h.extend_from_slice(&64000_u32.to_le_bytes());
    h.extend_from_slice(&4_u16.to_le_bytes());
    h.extend_from_slice(&16_u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data.len() as u32).to_le_bytes());
    h.extend_from_slice(&data);
    h
}
pub(in crate::storage) fn pending(
    s: &Storage,
    operation: &str,
) -> (ImportSnapshot, PathBuf, ImportManifest) {
    pending_configured(s, operation, &audio_import::ImportOptions::default())
}
pub(in crate::storage) fn pending_configured(
    s: &Storage,
    operation: &str,
    options: &audio_import::ImportOptions,
) -> (ImportSnapshot, PathBuf, ImportManifest) {
    let bytes = wav();
    let source = Fingerprint {
        path: s.root.join("not-permanent.wav"),
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let (import, created) = s
        .begin_audio_import_configured(
            "取り込み会議",
            operation,
            "会議.wav",
            &source,
            &info(),
            options,
        )
        .unwrap();
    assert!(created);
    let base = s.audio_import_directory(&import).unwrap();
    std::fs::write(base.join("original.wav"), &bytes).unwrap();
    s.audio_import_converting(&import.id).unwrap();
    let caf = caf_fixture(
        16000,
        2,
        3,
        32,
        4,
        &(0..64000)
            .flat_map(|_| [0.25_f32, -0.5_f32])
            .flat_map(|n| n.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    std::fs::write(base.join("decoded.caf"), &caf).unwrap();
    let mut verified = VerifiedAudio::open(
        File::open(base.join("decoded.caf")).unwrap(),
        caf.len() as u64,
        &format!("{:x}", Sha256::digest(&caf)),
    )
    .unwrap();
    let audio = crate::audio::flac::write_segment(
        &mut verified,
        0,
        64000,
        &base.join("audio.flac"),
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    drop(verified);
    std::fs::remove_file(base.join("decoded.caf")).unwrap();
    let manifest = ImportManifest {
        schema_version: 1,
        audio_format: None,
        meeting_id: import.meeting_id.clone(),
        import_id: import.id.clone(),
        original_file_name: import.original_file_name.clone(),
        info: info(),
        original: FileEvidence {
            size: source.size,
            sha256: source.sha256,
        },
        audio: FileEvidence {
            size: audio.size,
            sha256: audio.sha256,
        },
    };
    std::fs::write(
        base.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    (s.audio_import(&import.id).unwrap(), base, manifest)
}
fn count(s: &Storage, table: &str, meeting: &str) -> i64 {
    s.connection
        .lock()
        .unwrap()
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE meeting_id=?1"),
            params![meeting],
            |r| r.get(0),
        )
        .unwrap()
}

fn options() -> audio_import::ImportOptions {
    audio_import::ImportOptions {
        meeting_vocabulary: vec![VocabularyEntry {
            id: Uuid::new_v4().to_string(),
            term: "点検日".into(),
            reading: Some("てんけんび".into()),
            note: None,
            enabled: true,
        }],
        expected_global_version: Some(0),
        num_speakers: None,
    }
}

#[test]
fn import_options_and_initial_vocabulary_are_atomic_checked_and_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let source = Fingerprint {
        path: root.path().join("source.wav"),
        size: 44,
        sha256: "a".repeat(64),
    };
    let chosen = options();
    let mut stale = chosen.clone();
    stale.expected_global_version = Some(1);
    assert!(s
        .begin_audio_import_configured("会議", "stale", "source.wav", &source, &info(), &stale)
        .is_err());
    assert!(s.list_meetings().unwrap().is_empty());
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER injected_options_failure BEFORE INSERT ON audio_import_options BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s
        .begin_audio_import_configured("会議", "rollback", "source.wav", &source, &info(), &chosen)
        .is_err());
    assert!(s.list_meetings().unwrap().is_empty());
    let db = s.connection.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM vocabulary_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM audio_import_commits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER injected_options_failure;")
        .unwrap();
    drop(db);
    let (first, new) = s
        .begin_audio_import_configured("会議", "op", "source.wav", &source, &info(), &chosen)
        .unwrap();
    assert!(new);
    let (same, new) = s
        .begin_audio_import_configured("会議", "op", "source.wav", &source, &info(), &chosen)
        .unwrap();
    assert!(!new);
    assert_eq!(first, same);
    assert_eq!(
        s.get_meeting(&first.meeting_id)
            .unwrap()
            .unwrap()
            .source_type,
        Some("audio_import".into())
    );
    let book = s.vocabulary_book(Some(&first.meeting_id)).unwrap();
    assert_eq!(book.payload, chosen.meeting_vocabulary);
    assert_eq!(book.lock_version, 1);
    let mut changed = chosen.clone();
    changed.meeting_vocabulary[0].term = "変更した用語".into();
    assert!(s
        .begin_audio_import_configured("会議", "op", "source.wav", &source, &info(), &changed)
        .is_err());
    assert!(s
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_import_options SET options_hash=?1 WHERE import_id=?2",
            params!["b".repeat(64), first.id]
        )
        .is_err());
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        s.vocabulary_book(Some(&first.meeting_id)).unwrap().payload,
        chosen.meeting_vocabulary
    );
}

#[test]
fn import_finalization_and_recovery_cancel_keep_complete_evidence_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (before, base, _) = pending(&s, "cancel-finalize");
    let calls = Cell::new(0);
    assert!(s
        .finalize_audio_import_cancellable(&before.id, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 5
        })
        .is_err());
    assert_eq!(s.audio_import(&before.id).unwrap().state, "converting");
    assert_eq!(count(&s, "file_assets", &before.meeting_id), 0);
    assert!(base.join("audio.flac").is_file());
    assert!(base.join("manifest.json").is_file());
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    let interrupted = s.audio_import(&before.id).unwrap();
    assert!(s.recover_audio_imports_cancellable(&|| true).is_err());
    assert_eq!(s.audio_import(&before.id).unwrap(), interrupted);
    assert_eq!(s.recover_audio_imports().unwrap().len(), 1);
    assert_eq!(s.audio_import(&before.id).unwrap().state, "ready");
}

#[test]
fn invalid_import_options_are_rejected_before_audio_commit_or_automatic_handoff() {
    for valid_hash in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (import, base, _) = pending(&s, "bad-options");
        let json =
            "{\"profile\":\"unsupported\",\"meetingVocabulary\":[],\"expectedGlobalVersion\":null}";
        let digest = if valid_hash {
            format!("{:x}", Sha256::digest(json.as_bytes()))
        } else {
            "0".repeat(64)
        };
        let db = s.connection.lock().unwrap();
        db.execute_batch("DROP TRIGGER audio_import_options_fixed;")
            .unwrap();
        db.execute(
            "UPDATE audio_import_options SET options_json=?1,options_hash=?2 WHERE import_id=?3",
            params![json, digest, import.id],
        )
        .unwrap();
        drop(db);
        assert!(s.finalize_audio_import(&import.id).is_err());
        assert_eq!(count(&s, "file_assets", &import.meeting_id), 0);
        assert_eq!(count(&s, "analysis_flows", &import.meeting_id), 0);
        assert_eq!(
            s.get_meeting(&import.meeting_id)
                .unwrap()
                .unwrap()
                .lifecycle,
            "draft"
        );
        assert!(base.join("audio.flac").is_file());
        assert!(base.join("original.wav").is_file());
    }
}

#[test]
fn import_commit_is_atomic_scoped_and_never_fabricates_capture_or_speakers() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (snapshot, base, _manifest) = pending(&s, "import");
    assert!(s.recorded_audio_plan(&snapshot.meeting_id).is_err());
    let saved = s.finalize_audio_import(&snapshot.id).unwrap();
    assert_eq!(saved.state, "ready");
    assert_eq!(
        s.get_meeting(&saved.meeting_id).unwrap().unwrap().lifecycle,
        "imported"
    );
    assert_eq!(count(&s, "file_assets", &saved.meeting_id), 4);
    assert_eq!(count(&s, "recording_sessions", &saved.meeting_id), 0);
    let plan = s.recorded_audio_plan(&saved.meeting_id).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].kind, "processed");
    assert_eq!(plan.tracks[0].channels, 1);
    assert_eq!(
        s.analysis_flow(&saved.meeting_id).unwrap().unwrap().stage,
        "pending"
    );
    let mut audio = s
        .prepare_track_audio(&saved.meeting_id, &plan.tracks[0].id, 0, 4_000_000)
        .unwrap();
    let pcm = audio.read_pcm(63999, 1).unwrap();
    assert!((f32::from_le_bytes(pcm[..4].try_into().unwrap()) + 0.125).abs() < 0.001);
    assert_eq!(s.finalize_audio_import(&snapshot.id).unwrap(), saved);
    assert_eq!(count(&s, "file_assets", &saved.meeting_id), 4);
    assert!(s.cleanup_audio_import(&snapshot.id).is_err());
    assert!(s.fail_audio_import(&snapshot.id, "failed", "bad").is_err());
    assert!(!base.join("original.wav").exists());
    assert!(!base.join("audio.flac").exists());
    s.verify_canonical_audio(&saved.meeting_id, &|| false)
        .unwrap();
    assert!(s
        .load_current_analysis(&saved.meeting_id)
        .unwrap()
        .is_none());
    assert_eq!(s.connection.lock().unwrap().query_row("SELECT COUNT(*) FROM speakers s JOIN analysis_revisions a ON a.id=s.analysis_revision_id WHERE a.meeting_id=?1",params![saved.meeting_id],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn duplicate_import_operations_and_recording_exclusion_preserve_existing_work() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let source = Fingerprint {
        path: root.path().join("source.wav"),
        size: 44,
        sha256: "a".repeat(64),
    };
    let (first, new) = s
        .begin_audio_import("会議", "op", "source.wav", &source, &info())
        .unwrap();
    assert!(new);
    let (same, new) = s
        .begin_audio_import("会議", "op", "source.wav", &source, &info())
        .unwrap();
    assert!(!new);
    assert_eq!(first, same);
    assert!(s
        .begin_audio_import("別名", "op", "source.wav", &source, &info())
        .is_err());
    assert!(s
        .begin_audio_import("他の会議", "other", "source.wav", &source, &info())
        .is_err());
    assert_eq!(s.list_meetings().unwrap().len(), 1);
    let other = s.create_meeting("録音").unwrap();
    let config = RecordingConfig {
        mode: "system".into(),
        backend: "screencapturekit".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    assert!(s.prepare_recording(&other.id, &config, "capture").is_err());
    s.fail_audio_import(&first.id, "cancelled", "取消").unwrap();
    s.prepare_recording(&other.id, &config, "capture").unwrap();
    assert!(s
        .begin_audio_import("会議", "new", "source.wav", &source, &info())
        .is_err());
    assert_eq!(s.list_meetings().unwrap().len(), 2);
}

#[test]
fn import_db_failure_rolls_back_assets_track_lifecycle_and_handoff_then_recovers() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (snapshot, base, _) = pending(&s, "recover");
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER injected_import_failure BEFORE INSERT ON analysis_flows BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s.finalize_audio_import(&snapshot.id).is_err());
    assert_eq!(s.audio_import(&snapshot.id).unwrap().state, "converting");
    assert_eq!(
        s.get_meeting(&snapshot.meeting_id)
            .unwrap()
            .unwrap()
            .lifecycle,
        "draft"
    );
    assert_eq!(count(&s, "file_assets", &snapshot.meeting_id), 0);
    assert_eq!(count(&s, "audio_tracks", &snapshot.meeting_id), 0);
    assert_eq!(count(&s, "analysis_flows", &snapshot.meeting_id), 0);
    assert!(base.join("manifest.json").exists());
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER injected_import_failure;")
        .unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(
        s.audio_import(&snapshot.id).unwrap().error_code.as_deref(),
        Some("interrupted")
    );
    assert_eq!(s.recover_audio_imports().unwrap().len(), 1);
    assert!(s.recover_audio_imports().unwrap().is_empty());
    assert_eq!(s.audio_import(&snapshot.id).unwrap().state, "ready");
    assert_eq!(count(&s, "audio_tracks", &snapshot.meeting_id), 2);
    assert_eq!(count(&s, "file_assets", &snapshot.meeting_id), 4);
}

#[test]
fn recovery_rejects_corrupt_complete_evidence_and_preserves_audio() {
    for name in ["manifest.json", "original.wav", "audio.flac"] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (snapshot, base, _) = pending(&s, name);
        let file = base.join(name);
        let mut bytes = std::fs::read(&file).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&file, &bytes).unwrap();
        drop(s);
        let s = Storage::open(root.path().into()).unwrap();
        assert!(s.recover_audio_imports().unwrap().is_empty());
        assert_eq!(s.audio_import(&snapshot.id).unwrap().state, "failed");
        assert_eq!(count(&s, "file_assets", &snapshot.meeting_id), 0);
        assert_eq!(count(&s, "audio_tracks", &snapshot.meeting_id), 0);
        assert_eq!(std::fs::read(file).unwrap(), bytes);
        assert!(base.join("original.wav").exists());
    }
}

#[test]
fn incomplete_import_cleanup_is_scoped_and_unknown_files_are_retained() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (_snapshot, base, _) = pending(&s, "incomplete");
    std::fs::remove_file(base.join("manifest.json")).unwrap();
    let unrelated = s.create_meeting("既存会議").unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert!(s.recover_audio_imports().unwrap().is_empty());
    assert!(!base.exists());
    assert_eq!(s.get_meeting(&unrelated.id).unwrap().unwrap(), unrelated);
    let (snapshot, base, _) = pending(&s, "unknown");
    s.fail_audio_import(&snapshot.id, "failed", "error")
        .unwrap();
    std::fs::write(base.join("user-file"), b"retain").unwrap();
    assert!(s.cleanup_audio_import(&snapshot.id).is_err());
    assert_eq!(std::fs::read(base.join("user-file")).unwrap(), b"retain");
    assert!(base.join("original.wav").exists());
}

#[test]
fn ready_import_metadata_and_track_roles_are_immutable_and_scoped() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (snapshot, _, _) = pending(&s, "immutable");
    s.finalize_audio_import(&snapshot.id).unwrap();
    let db = s.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE audio_import_commits SET source_hash=?2 WHERE import_id=?1",
            params![snapshot.id, "b".repeat(64)]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE audio_imports SET original_file_name='different.wav' WHERE id=?1",
            params![snapshot.id]
        )
        .is_err());
    assert!(db.execute("INSERT INTO audio_tracks(id,meeting_id,import_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES('false-mic',?1,?2,'mic','flac',16000,2,'ready',0,0)",params![snapshot.meeting_id,snapshot.id]).is_err());
    assert!(db
        .execute(
            "UPDATE audio_tracks SET kind='mic' WHERE import_id=?1",
            params![snapshot.id]
        )
        .is_err());
    let row: String = db
        .query_row(
            "SELECT info_json FROM audio_import_commits WHERE import_id=?1",
            params![snapshot.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!row.contains("path"));
}

#[cfg(unix)]
#[test]
fn import_symlink_paths_and_cleanup_links_never_touch_external_audio() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (snapshot, base, _) = pending(&s, "link");
    let external = outside.path().join("original.wav");
    std::fs::rename(base.join("original.wav"), &external).unwrap();
    std::os::unix::fs::symlink(&external, base.join("original.wav")).unwrap();
    assert!(s.finalize_audio_import(&snapshot.id).is_err());
    s.fail_audio_import(&snapshot.id, "failed", "symlink")
        .unwrap();
    assert!(s.cleanup_audio_import(&snapshot.id).is_err());
    assert_eq!(std::fs::read(external).unwrap(), wav());
}

#[test]
fn import_info_bounds_and_space_estimate_are_checked_before_saved_audio() {
    let mut bad = info();
    bad.frames = 16000 * 14400 + 1;
    assert!(bad.validate().is_err());
    bad = info();
    bad.duration_us += 1;
    assert!(bad.validate().is_err());
    bad = info();
    bad.codec = "video".into();
    assert!(bad.validate().is_err());
    assert!(info().required_bytes(u64::MAX).is_err());
    assert_eq!(
        info().required_bytes(44).unwrap(),
        64000 * 2 * 4 + 64000 * 2 + 44 + 128 * 1024 * 1024
    );
}

#[cfg(unix)]
fn fake_helper(root: &Path) -> Fingerprint {
    use std::os::unix::fs::PermissionsExt;
    let caf = root.join("fixture.caf");
    let pcm: Vec<_> = (0..64000)
        .flat_map(|_| [0.25_f32, -0.5_f32])
        .flat_map(|n| n.to_le_bytes())
        .collect();
    std::fs::write(&caf, caf_fixture(16000, 2, 3, 32, 4, &pcm)).unwrap();
    let helper = root.join("fake-import-helper");
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\nif test \"$3\" != '-'; then cp '{}' \"$3\"; fi\nprintf '%s' '{}'\n",
            caf.display(),
            serde_json::to_string(&info()).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    Fingerprint::capture(&helper, &crate::test_support::never_cancelled).unwrap()
}
#[cfg(unix)]
#[test]
fn import_cancellation_cleans_only_uncommitted_copy_and_conversion() {
    for phase in ["copying", "converting"] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().join("data")).unwrap();
        let helper = fake_helper(root.path());
        let original = root.path().join("source.wav");
        std::fs::write(&original, wav()).unwrap();
        let selected = audio_import::inspect(
            &original,
            &helper,
            s.import_scratch().unwrap(),
            &crate::test_support::never_cancelled,
        )
        .unwrap_or_else(|error| panic!("{phase}: {error}"));
        let stop = Cell::new(false);
        assert!(audio_import::import(
            &s,
            selected,
            &helper,
            "取消",
            phase,
            &|| stop.get(),
            &|stage, _| if stage == phase {
                stop.set(true)
            }
        )
        .is_err());
        let meeting = s.list_meetings().unwrap().remove(0);
        assert_eq!(count(&s, "file_assets", &meeting.id), 0);
        assert_eq!(std::fs::read(&original).unwrap(), wav());
        let (state, code): (String, String) = s
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state,error_code FROM audio_imports WHERE meeting_id=?1",
                params![meeting.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), code.as_str()), ("failed", "cancelled"));
    }
}
#[cfg(unix)]
#[test]
fn inspected_source_changes_and_duplicate_starts_never_delete_an_active_import() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().join("data")).unwrap();
    let helper = fake_helper(root.path());
    let original = root.path().join("source.wav");
    std::fs::write(&original, wav()).unwrap();
    let selected = audio_import::inspect(
        &original,
        &helper,
        s.import_scratch().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let (active, created) = s
        .begin_audio_import(
            "会議",
            "same",
            &selected.file_name,
            &selected.proof,
            &selected.info,
        )
        .unwrap();
    assert!(created);
    let directory = s.audio_import_directory(&active).unwrap();
    std::fs::write(directory.join("original.wav"), b"active copy").unwrap();
    assert!(audio_import::import(
        &s,
        selected,
        &helper,
        "会議",
        "same",
        &crate::test_support::never_cancelled,
        &|_, _| {}
    )
    .is_err());
    assert_eq!(s.audio_import(&active.id).unwrap().state, "copying");
    assert_eq!(
        std::fs::read(directory.join("original.wav")).unwrap(),
        b"active copy"
    );
    s.fail_audio_import(&active.id, "cancelled", "cancelled")
        .unwrap();
    s.cleanup_audio_import(&active.id).unwrap();
    let selected = audio_import::inspect(
        &original,
        &helper,
        s.import_scratch().unwrap(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    std::fs::write(&original, b"changed").unwrap();
    assert!(audio_import::import(
        &s,
        selected,
        &helper,
        "変更",
        "changed",
        &crate::test_support::never_cancelled,
        &|_, _| {}
    )
    .is_err());
    assert_eq!(std::fs::read(original).unwrap(), b"changed");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires MON_IMPORT_HELPER and MON_IMPORT_FFMPEG; real file decoding, no recording"]
fn native_import_all_formats_are_content_detected_and_survive_external_source_move() {
    use std::process::Command;
    let helper = Fingerprint::capture(
        Path::new(&std::env::var("MON_IMPORT_HELPER").unwrap()),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ffmpeg = std::env::var("MON_IMPORT_FFMPEG").unwrap();
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source.wav");
    std::fs::write(&input, wav()).unwrap();
    for (name, codec, format) in [
        ("wav", "pcm_s16le", "wav"),
        ("mp3", "libmp3lame", "mp3"),
        ("aac", "aac", "ipod"),
        ("alac", "alac", "ipod"),
        ("flac", "flac", "flac"),
    ] {
        let source = root.path().join(format!("{name}.wrong-extension"));
        let result = Command::new(&ffmpeg)
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&input)
            .args(["-c:a", codec, "-f", format])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let s = Storage::open(root.path().join(format!("store-{name}"))).unwrap();
        let selected = audio_import::inspect(
            &source,
            &helper,
            s.import_scratch().unwrap(),
            &crate::test_support::never_cancelled,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            selected.info.detected_format,
            if name == "aac" || name == "alac" {
                "m4a"
            } else {
                name
            }
        );
        assert_eq!(selected.info.channels, 2);
        let original = std::fs::read(&source).unwrap();
        let proof = selected.proof.clone();
        let saved = audio_import::import(
            &s,
            selected,
            &helper,
            "実復号試験",
            name,
            &crate::test_support::never_cancelled,
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(saved.state, "ready");
        assert_eq!(std::fs::read(&source).unwrap(), original);
        std::fs::rename(&source, root.path().join(format!("moved-{name}"))).unwrap();
        drop(s);
        let s = Storage::open(root.path().join(format!("store-{name}"))).unwrap();
        let plan = s.recorded_audio_plan(&saved.meeting_id).unwrap();
        assert_eq!(plan.tracks[0].kind, "imported");
        assert_eq!(count(&s, "recording_sessions", &saved.meeting_id), 0);
        let mut audio = s
            .prepare_track_audio(
                &saved.meeting_id,
                &plan.tracks[0].id,
                0,
                saved.info.duration_us,
            )
            .unwrap();
        assert_eq!(audio.channels, 2);
        assert!(!audio.read_pcm(0, 1).unwrap().is_empty());
        let original = s
            .audio_import_directory(&saved)
            .unwrap()
            .join(format!("original.{}", saved.info.detected_format));
        assert_eq!(
            Fingerprint::capture(&original, &crate::test_support::never_cancelled)
                .unwrap()
                .sha256,
            proof.sha256
        );
    }
    let video = root.path().join("video.m4a");
    let result = Command::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=1:duration=1",
            "-i",
        ])
        .arg(&input)
        .args(["-t", "1", "-c:v", "mpeg4", "-c:a", "aac", "-f", "mp4"])
        .arg(&video)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let s = Storage::open(root.path().join("rejected")).unwrap();
    assert!(audio_import::inspect(
        &video,
        &helper,
        s.import_scratch().unwrap(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    std::fs::write(root.path().join("broken.wav"), b"not audio").unwrap();
    assert!(audio_import::inspect(
        &root.path().join("broken.wav"),
        &helper,
        s.import_scratch().unwrap(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(s.list_meetings().unwrap().is_empty());
}

#[test]
fn initial_import_rejects_out_of_range_speaker_counts_before_saving() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    for count in [0, 101, u32::MAX] {
        let configured = audio_import::ImportOptions {
            num_speakers: Some(count),
            ..Default::default()
        };
        assert!(configured.validate().is_err());
    }
    let configured = audio_import::ImportOptions {
        num_speakers: Some(5),
        ..Default::default()
    };
    let (import, _, _) = pending_configured(&s, "count-replay", &configured);
    assert_eq!(
        super::read_options(&s.connection.lock().unwrap(), &import.id)
            .unwrap()
            .num_speakers,
        Some(5)
    );
}

fn pending_pcm(s: &Storage, operation: &str) -> (ImportSnapshot, PathBuf, ImportManifest) {
    let (snapshot, base, mut manifest) = pending(s, operation);
    let pcm = (0..64000)
        .flat_map(|_| [0.25_f32, -0.5_f32])
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let caf = caf_fixture(16000, 2, 3, 32, 4, &pcm);
    std::fs::write(base.join("decoded.caf"), &caf).unwrap();
    std::fs::remove_file(base.join("audio.flac")).unwrap();
    manifest.schema_version = 2;
    manifest.audio_format = Some("caf".into());
    manifest.audio = FileEvidence {
        size: caf.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&caf)),
    };
    std::fs::write(
        base.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    (snapshot, base, manifest)
}

#[test]
fn pcm_import_recovers_without_flac_and_roundtrips_canonical_wav_backup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let s = Storage::open(root.join("source")).unwrap();
    let (snapshot, base, _) = pending_pcm(&s, "pcm-recovery");
    assert!(s
        .finalize_audio_import_cancellable(&snapshot.id, &|| true)
        .is_err());
    assert!(base.join("decoded.caf").is_file());
    assert!(!base.join("audio.flac").exists());
    drop(s);
    let s = Storage::open(root.join("source")).unwrap();
    let recovered = s.recover_audio_imports().unwrap();
    assert_eq!(
        recovered.len(),
        1,
        "{:?}",
        s.audio_import(&snapshot.id).unwrap()
    );
    let plan = s
        .verify_canonical_audio(&snapshot.meeting_id, &|| false)
        .unwrap();
    assert_eq!(plan.tracks[0].sample_rate, 16000);
    assert_eq!(plan.tracks[0].channels, 1);
    assert!(!base.join("decoded.caf").exists());
    assert!(!base.join("audio.flac").exists());
    let mut audio = s
        .prepare_track_audio(&snapshot.meeting_id, &plan.tracks[0].id, 0, 4_000_000)
        .unwrap();
    let value = audio.read_pcm(63999, 1).unwrap();
    assert!((f32::from_le_bytes(value[..4].try_into().unwrap()) + 0.125).abs() < 0.001);
    drop(audio);
    let archive = root.join("pcm.monbackup");
    s.create_backup_archive(&archive, &|| false).unwrap();
    let target = Storage::open(root.join("restored")).unwrap();
    let prepared = target
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let result = prepared.publish(
        &target,
        &prepared.preview().confirmation_id,
        &|| false,
        &|_| {},
    );
    // Restore APIs own collision and ID mapping; verify the published meeting.
    let result = result.unwrap();
    target
        .verify_canonical_audio(&result.meeting_ids[0], &|| false)
        .unwrap();
}

#[test]
fn pcm_import_rejects_ambiguous_or_changed_staging_before_commit() {
    for changed_bytes in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let s = Storage::open(temp.path().join("data")).unwrap();
        let (snapshot, base, mut manifest) = pending_pcm(&s, "bad-pcm");
        if changed_bytes {
            std::fs::write(base.join("decoded.caf"), b"broken").unwrap();
        } else {
            manifest.audio_format = Some("flac".into());
            std::fs::write(
                base.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        assert!(s.finalize_audio_import(&snapshot.id).is_err());
        assert_eq!(count(&s, "file_assets", &snapshot.meeting_id), 0);
        assert!(base.join("original.wav").is_file());
    }
}
