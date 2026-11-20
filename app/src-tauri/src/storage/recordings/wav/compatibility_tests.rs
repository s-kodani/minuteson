use super::recovery_tests::chain;
use super::tests::fixture;
use super::*;

fn partial_fixture() -> (
    tempfile::TempDir,
    Storage,
    RecordingSession,
    PathBuf,
    Vec<u8>,
) {
    let (root, storage, session, base) = fixture(true);
    chain(&base, &session.id, false);
    std::fs::remove_file(base.join("wav-00000002.json")).unwrap();
    let original = std::fs::read(base.join("recording.wav")).unwrap();
    (root, storage, session, base, original)
}

#[test]
fn partially_recovered_wav_roundtrips_through_portable_backup_without_its_broken_source() {
    let (_root, storage, session, base, original) = partial_fixture();
    storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    let (restored, meeting, _) = crate::storage::backup::published_restore_for_rebinding_test(
        &storage,
        target.path(),
        &session.meeting_id,
        &[],
    );
    let plan = restored
        .verify_canonical_audio(&meeting, &|| false)
        .unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].chunks.len(), 1);
    assert_eq!(plan.tracks[0].chunks[0].frames, 16000);
    let restored_path = restored
        .meeting_mix_path(&meeting, &plan.tracks[0])
        .unwrap();
    assert_eq!(
        &std::fs::read(restored_path).unwrap()[80..],
        &original[80..80 + 32000]
    );
    assert_eq!(plan.tracks[0].end_us, 1_000_000);
    let audio_assets: i64 = restored
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%'",
            [&meeting],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audio_assets, 1);
    let exported = tempfile::tempdir().unwrap();
    restored
        .create_backup_archive(&exported.path().join("roundtrip.monbackup"), &|| false)
        .unwrap();
    assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
}

#[test]
fn recovered_audio_deletion_distinguishes_registered_output_from_retained_native_source() {
    use crate::storage::deletion::audio::{operations::Prepared, Census, Disposition};
    for include_original in [false, true] {
        let (_root, storage, session, base, original) = partial_fixture();
        storage
            .finalize_available_recording(&session.id, &|| false)
            .unwrap();
        let plan = storage.canonical_audio_plan(&session.meeting_id).unwrap();
        let output = storage
            .meeting_mix_path(&session.meeting_id, &plan.tracks[0])
            .unwrap();
        assert_ne!(output, base.join("recording.wav"));
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
        let metadata: Vec<_> = [
            "manifest.json",
            "canonical-wav.json",
            "single-wav.json",
            "single-wav-owner.json",
            "wav-00000001.json",
        ]
        .into_iter()
        .map(|name| {
            let path = base.join(name);
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
        let preview = Census::capture(&storage, &session.meeting_id, &|| false)
            .unwrap()
            .preview(&Uuid::new_v4().to_string())
            .unwrap();
        let relative = |path: &Path| {
            path.strip_prefix(storage.root())
                .unwrap()
                .to_str()
                .unwrap()
                .replace(std::path::MAIN_SEPARATOR, "/")
        };
        assert_eq!(
            preview
                .entries
                .iter()
                .find(|entry| entry.relative_path == relative(&output))
                .unwrap()
                .disposition,
            Disposition::RegisteredAudio
        );
        assert_eq!(
            preview
                .entries
                .iter()
                .find(|entry| entry.relative_path == relative(&base.join("recording.wav")))
                .unwrap()
                .disposition,
            Disposition::ProducerAudioLeftover
        );
        let lease = crate::analysis_worker::AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let mut deletion =
            Prepared::fresh(&storage, &session.meeting_id, &lease, &|| false).unwrap();
        let confirmation = deletion.preview().confirmation_id;
        let result = deletion.execute(
            &storage,
            &lease,
            &confirmation,
            include_original,
            &|| false,
            &|_| {},
        );
        if include_original {
            assert!(result.unwrap().audio_deleted);
            assert!(!output.exists());
        } else {
            // Native producer audio requires explicit consent too: omitting
            // it must not remove the registered output and claim completion.
            assert!(result.is_err());
            assert!(output.exists());
        }
        assert_eq!(base.join("recording.wav").exists(), !include_original);
        if !include_original {
            assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
        }
        for (path, bytes) in metadata {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        assert!(storage.get_meeting(&session.meeting_id).unwrap().is_some());
    }
}

#[test]
fn retry_registers_the_exact_raw_bytes_of_a_reformatted_frozen_publication() {
    let (_root, storage, session, base) = fixture(true);
    let path = base.join("canonical-wav.json");
    assert!(storage
        .finalize_available_recording(&session.id, &|| path.exists())
        .is_err());
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let bytes = serde_json::to_vec_pretty(&document).unwrap();
    assert_ne!(bytes, serde_json::to_vec(&document).unwrap());
    std::fs::write(&path, &bytes).unwrap();
    storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    let plan = storage.canonical_audio_plan(&session.meeting_id).unwrap();
    let (size, hash): (i64, String) = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT size_bytes,content_hash FROM file_assets WHERE id=?1",
            [&plan.manifest_asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(size, bytes.len() as i64);
    assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    storage
        .verify_meeting_asset_cancellable(
            &session.meeting_id,
            &plan.manifest_asset_id,
            &plan.manifest_hash,
            &|| false,
        )
        .unwrap();
}

#[test]
fn partial_recovery_resumes_a_hashed_part_before_its_final_rename_without_rewriting_source() {
    let (_root, storage, session, base, original) = partial_fixture();
    assert!(storage
        .finalize_available_recording(&session.id, &|| {
            base.join("canonical-wav.json").exists()
        })
        .is_err());
    let output = std::fs::read_dir(&base)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("wav-recovery-")
        })
        .unwrap();
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("wave-publication.json")).unwrap())
            .unwrap();
    assert!(receipt["hash"].as_str().is_some());
    let name = receipt["name"].as_str().unwrap();
    let final_path = output.join(name);
    let part_path = output.join(format!(".{name}.part"));
    let completed_bytes = std::fs::read(&final_path).unwrap();
    std::fs::rename(&final_path, &part_path).unwrap();
    storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    assert!(!part_path.exists());
    assert_eq!(std::fs::read(&final_path).unwrap(), completed_bytes);
    assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
    let plan = storage
        .verify_canonical_audio(&session.meeting_id, &|| false)
        .unwrap();
    assert_eq!(plan.tracks[0].chunks[0].frames, 16000);
    assert_eq!(
        storage
            .meeting_mix_path(&session.meeting_id, &plan.tracks[0])
            .unwrap(),
        final_path
    );
}
