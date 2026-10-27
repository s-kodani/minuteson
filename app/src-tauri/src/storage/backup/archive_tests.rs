use super::*;
use crate::audio_import::ImportOptions;
use serde_json::{json, Value};
use std::cell::Cell;

// Local database migration fixtures only, never a backup reader or writer.
pub(in crate::storage::backup) fn migration_database(db: Connection, version: usize) -> Connection {
    db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
    for &(version, sql) in &MIGRATIONS[..version] {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,0)",
            params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
        )
        .unwrap();
    }
    db
}

pub(in crate::storage::backup) fn meeting(storage: &Storage, operation: &str) -> String {
    meeting_source(storage, operation, false)
}
pub(in crate::storage::backup) fn meeting_with_mix(storage: &Storage, operation: &str) -> String {
    meeting_source(storage, operation, true)
}
fn meeting_source(storage: &Storage, operation: &str, mixed: bool) -> String {
    let options = ImportOptions {
        meeting_vocabulary: vec![VocabularyEntry {
            id: Uuid::new_v4().to_string(),
            term: "点検日".into(),
            reading: Some("てんけんび".into()),
            note: None,
            enabled: true,
        }],
        expected_global_version: None,
        num_speakers: None,
    };
    let (import, _, _) =
        super::super::super::imports::tests::pending_configured(storage, operation, &options);
    let saved = storage.finalize_audio_import(&import.id).unwrap();
    let mixed_track = mixed.then(|| {
        storage
            .ensure_meeting_mix(&saved.meeting_id, &|| false, &|_, _| {})
            .unwrap()
            .tracks[0]
            .id
            .clone()
    });
    let db = storage.connection.lock().unwrap();
    let track: String = mixed_track.unwrap_or_else(|| {
        db.query_row(
            "SELECT id FROM audio_tracks WHERE meeting_id=?1",
            params![saved.meeting_id],
            |r| r.get(0),
        )
        .unwrap()
    });
    let manifest_hash: String = db
        .query_row(
            "SELECT manifest_hash FROM audio_import_commits WHERE import_id=?1",
            params![saved.id],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let analysis = storage.create_analysis_candidate(&saved.meeting_id,AnalysisInput {
        audio_manifest_hash:manifest_hash,
        profile_snapshot:json!({"model":{"path":"/Users/source-machine/models/model.gguf","size":7,"sha256":"b".repeat(64)}}),
        config:json!({"executable":{"path":"C:\\Users\\source-machine\\llama.exe","size":9,"sha256":"c".repeat(64)}}),
        speakers:vec![],segments:vec![],utterances:vec![UtteranceInput {track_id:track,start_us:0,end_us:1_000_000,text:"点検日を確認する。発言に含まれる /Users/fictional はそのまま保持。".into(),speaker_keys:vec![],confidence:None}],
    }).unwrap();
    let version = storage
        .get_meeting(&saved.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    storage
        .commit_analysis_candidate(&saved.meeting_id, &analysis, version, false)
        .unwrap();
    storage
        .create_transcript_draft(&saved.meeting_id, &analysis)
        .unwrap();
    saved.meeting_id
}

fn write(storage: &Storage, dir: &Path) -> (PathBuf, BackupReceipt) {
    let path = dir.join("meeting.monbackup");
    let receipt = storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    (path, receipt)
}

fn verified(path: &Path, dir: &Path) -> VerifiedArchive {
    read(
        &mut File::open(path).unwrap(),
        dir,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
}

pub(in crate::storage::backup) fn pack(path: &Path, manifest: &Manifest, payloads: &[Vec<u8>]) {
    let bytes = serde_json::to_vec(manifest).unwrap();
    let mut file = File::create(path).unwrap();
    file.write_all(MAGIC).unwrap();
    file.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
    file.write_all(&Sha256::digest(&bytes)).unwrap();
    file.write_all(&bytes).unwrap();
    for payload in payloads {
        file.write_all(payload).unwrap();
    }
}

fn payloads(archive: &VerifiedArchive) -> Vec<Vec<u8>> {
    archive
        .manifest
        .entries
        .iter()
        .map(|e| std::fs::read(archive.stage.path().join(&e.path)).unwrap())
        .collect()
}

#[test]
fn obsolete_backup_versions_are_rejected_before_staging_without_changing_existing_data() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let source_storage = Storage::open(root.path().into()).unwrap();
    meeting(&source_storage, "current-only-source");
    let (path, _) = write(&source_storage, output.path());
    let source = verified(&path, output.path());
    let payloads = payloads(&source);
    let target = Storage::open(target_root.path().into()).unwrap();
    meeting(&target, "preserve-existing");
    let before = super::super::publication::tests::all_rows(&target.connection.lock().unwrap());
    let mut versions = (0..MIGRATIONS.len())
        .chain([MIGRATIONS.len() + 1])
        .map(|schema| (crate::current_formats::BACKUP, schema))
        .collect::<Vec<_>>();
    versions.extend([
        (1, MIGRATIONS.len()),
        (0, MIGRATIONS.len()),
        (3, MIGRATIONS.len()),
        (4, MIGRATIONS.len()),
        (crate::current_formats::BACKUP + 1, MIGRATIONS.len()),
    ]);
    for (format, schema) in versions {
        let mut manifest = source.manifest.clone();
        manifest.format_version = format;
        manifest.schema_version = schema;
        let invalid = output.path().join("unsupported.monbackup");
        pack(&invalid, &manifest, &payloads);
        let count = std::fs::read_dir(output.path()).unwrap().count();
        let error = read(
            &mut File::open(&invalid).unwrap(),
            output.path(),
            &crate::test_support::never_cancelled,
        )
        .err()
        .unwrap();
        assert_eq!(error, "現在の形式ではないバックアップは復元できません");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), count);
        assert!(target
            .prepare_additive_restore(&invalid, &crate::test_support::never_cancelled, &|_| {})
            .is_err());
        assert_eq!(
            super::super::publication::tests::all_rows(&target.connection.lock().unwrap()),
            before
        );
    }
}

#[test]
#[cfg(unix)]
fn archive_preserves_audio_options_text_markdown_and_model_metadata_without_host_paths() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let id = meeting(&storage, "import");
    let source = storage.load_current_analysis(&id).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&id).unwrap().unwrap();
    let engine = crate::storage::markdown_generation_tests::engine(root.path(), "normal");
    let model_hash = engine.model.sha256.clone();
    let generated = crate::generation::markdown_job::enqueue_synthetic(
        &storage,
        crate::storage::MinutesJobRequest {
            meeting_id: id.clone(),
            operation_id: Uuid::new_v4().to_string(),
            expected_analysis_id: source.id,
            expected_content_hash: source.content_hash,
            expected_meeting_lock_version: storage.get_meeting(&id).unwrap().unwrap().lock_version,
            expected_draft: Some(crate::storage::DraftStamp {
                id: draft.id,
                lock_version: draft.lock_version,
            }),
        },
        engine,
        "standard",
        2400,
        None,
    )
    .unwrap();
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        generated.id
    );
    crate::storage::markdown_generation_tests::process(
        &storage,
        &generated.id,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap()
    .unwrap();
    let markdown = storage.get_minutes_markdown(&id).unwrap().unwrap();
    storage
        .save_analysis_preferences(
            0,
            AnalysisPreferences {
                standard: super::super::super::preferences::EnginePreferences {
                    model_path: "/Users/source-machine/private/model.bin".into(),
                    ..Default::default()
                },
                high_accuracy: Default::default(),
            },
        )
        .unwrap();
    let (path, receipt) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    assert_eq!(receipt.meeting_count, 1);
    assert!(receipt.asset_count >= 3);
    assert_eq!(receipt.asset_count, archive.manifest.entries.len() - 1);
    assert_eq!(
        receipt.sha256,
        digest_file(&path, &crate::test_support::never_cancelled)
            .unwrap()
            .1
    );
    assert_eq!(archive.manifest.encryption, Encryption::None);
    assert_eq!(archive.manifest.hash_basis, HashBasis::SourceIdentity);
    // The committed transcript carries two model references and the current
    // Markdown job carries two more; none includes source-machine paths.
    assert_eq!(archive.manifest.file_references.len(), 4);
    for (name, sha256) in [
        ("model.gguf", "b".repeat(64)),
        ("llama.exe", "c".repeat(64)),
    ] {
        assert!(archive
            .manifest
            .file_references
            .iter()
            .any(|r| r.name == name && r.sha256 == sha256));
    }
    assert!(archive
        .manifest
        .file_references
        .iter()
        .any(|r| r.name == "model.gguf" && r.sha256 == model_hash));
    assert!(archive
        .manifest
        .file_references
        .iter()
        .any(|r| r.name == "llama-completion" && r.role == "executable"));
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes
        .windows(b"source-machine".len())
        .any(|s| s == b"source-machine"));
    let recognized: String = archive.database.query_row("SELECT effective_text FROM utterances WHERE analysis_revision_id=(SELECT current_analysis_id FROM meetings WHERE id=?1)",params![id],|r| r.get(0)).unwrap();
    assert!(recognized.contains("/Users/fictional"));
    let config: String = archive
        .database
        .query_row("SELECT options_json FROM audio_import_options", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<ImportOptions>(&config)
            .unwrap()
            .expected_global_version,
        None
    );
    assert!(!config.contains("profile"));
    assert_eq!(
        archive
            .database
            .query_row("SELECT COUNT(*) FROM app_settings", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        archive
            .database
            .query_row("SELECT COUNT(*) FROM evidence_links", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        archive
            .database
            .query_row(
                "SELECT markdown FROM minutes_markdown WHERE meeting_id=?1",
                [&id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        markdown.markdown
    );
    for entry in &archive.manifest.entries[1..] {
        if entry.transform == Transform::Identity {
            assert_eq!(entry.sha256, entry.source_sha256.clone().unwrap());
            assert_eq!(entry.size, entry.source_size.unwrap());
        }
    }
    let staged = archive.stage.path().to_path_buf();
    drop(archive);
    assert!(!staged.exists());
    assert!(storage.create_meeting("書き込み再開").is_ok());
}

#[test]
fn registered_recreatable_pcm_and_superseded_source_are_omitted_but_canonical_wav_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let id = meeting(&storage, "import");
    let job = storage
        .enqueue_job(JobSpec {
            meeting_id: id.clone(),
            kind: JobKind::Transcription,
            operation_id: "job".into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: json!({}),
            parameters: json!({"schemaVersion":4}),
            total_units: Some(1),
        })
        .unwrap();
    let directory = storage.analysis_unit_directory(&id, &job.id, 0).unwrap();
    let pcm = directory.join("input.wav");
    std::fs::write(&pcm, b"recreatable PCM that must not appear").unwrap();
    let asset = storage
        .register_analysis_artifact(&id, &pcm, "audio/wav")
        .unwrap();
    let native = directory.join("result.json");
    std::fs::write(&native,serde_json::to_vec(&json!({"params":{"model":"/Users/source-machine/model.bin","fname":"C:\\Users\\source-machine\\input.wav"},"transcription":[{"text":"保持する自然文 /Users/fictional"}]})).unwrap()).unwrap();
    let result = storage
        .register_analysis_artifact(&id, &native, "application/json")
        .unwrap();
    let (path, receipt) = write(&storage, output.path());
    assert_eq!(receipt.omitted_pcm_count, 1);
    let archive = verified(&path, output.path());
    assert!(!archive.assets.contains_key(&asset.id));
    assert!(archive
        .manifest
        .omitted_assets
        .iter()
        .any(|omitted| omitted.asset_id == asset.id
            && omitted.reason == OmittedReason::RecreatablePcm));
    let state: String = archive
        .database
        .query_row(
            "SELECT state FROM file_assets WHERE id=?1",
            params![asset.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "missing");
    let imported_original: String = archive
        .database
        .query_row("SELECT original_asset_id FROM audio_imports", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!archive.assets.contains_key(&imported_original));
    assert!(
        super::super::meeting_audio::superseded_asset(&archive.database, &imported_original)
            .unwrap()
    );
    let canonical: String = archive.database.query_row("SELECT c.file_asset_id FROM meeting_audio_commits a JOIN audio_chunks c ON c.track_id=a.track_id", [], |r| r.get(0)).unwrap();
    assert!(archive.assets.contains_key(&canonical));
    let result: Value =
        serde_json::from_slice(&std::fs::read(&archive.assets[&result.id]).unwrap()).unwrap();
    assert_eq!(result["params"]["model"], "model.bin");
    assert_eq!(result["params"]["fname"], "input.wav");
    assert!(result["transcription"][0]["text"]
        .as_str()
        .unwrap()
        .contains("/Users/fictional"));
    assert!(storage.verify_asset(&asset.id).unwrap());
}

#[test]
fn native_logs_are_omitted_without_dropping_prompts_or_reusing_the_pcm_reason() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let id = meeting(&storage, "logs");
    let job = storage
        .enqueue_job(JobSpec {
            meeting_id: id.clone(),
            kind: JobKind::Transcription,
            operation_id: "logs-job".into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: json!({}),
            parameters: json!({"schemaVersion":4}),
            total_units: Some(1),
        })
        .unwrap();
    let directory = storage.analysis_unit_directory(&id, &job.id, 0).unwrap();
    let secret = b"private-native-log /Users/source-machine/private-model.bin";
    let mut logs = vec![];
    for name in ["stdout.log", "stderr.log", "logical-stdout.log"] {
        let path = directory.join(name);
        std::fs::write(&path, secret).unwrap();
        logs.push(
            storage
                .register_analysis_artifact(&id, &path, "text/plain")
                .unwrap()
                .id,
        );
    }
    let prompt = directory.join("prompt.txt");
    std::fs::write(&prompt, b"natural content /Users/fictional must remain").unwrap();
    let prompt = storage
        .register_analysis_artifact(&id, &prompt, "text/plain")
        .unwrap();
    let (path, receipt) = write(&storage, output.path());
    assert_eq!(receipt.omitted_log_count, 3);
    assert_eq!(receipt.omitted_pcm_count, 0);
    let mut archive = verified(&path, output.path());
    for id in &logs {
        assert!(!archive.assets.contains_key(id));
        let entry = archive
            .manifest
            .omitted_assets
            .iter()
            .find(|e| &e.asset_id == id)
            .unwrap();
        assert_eq!(entry.reason, OmittedReason::NativeLog);
    }
    assert_eq!(
        std::fs::read(&archive.assets[&prompt.id]).unwrap(),
        b"natural content /Users/fictional must remain"
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(secret.len()).any(|s| s == secret));
    // Rehashed omission metadata cannot disguise a log as regeneratable audio.
    archive.manifest.omitted_assets[0].reason = OmittedReason::RecreatablePcm;
    let forged = output.path().join("wrong-log-kind.monbackup");
    pack(&forged, &archive.manifest, &payloads(&archive));
    assert!(read(
        &mut File::open(forged).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled
    )
    .err()
    .unwrap()
    .contains("除外ファイル"));
    assert!(storage.verify_asset(&prompt.id).is_ok());
}

#[test]
fn native_completion_marker_is_canonicalized_without_repairing_invalid_output_or_text() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let id = meeting(&storage, "completion");
    let job = storage
        .enqueue_job(JobSpec {
            meeting_id: id.clone(),
            kind: JobKind::Transcription,
            operation_id: "completion-job".into(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: json!({}),
            parameters: json!({"schemaVersion":4}),
            total_units: Some(1),
        })
        .unwrap();
    let directory = storage.analysis_unit_directory(&id, &job.id, 0).unwrap();
    let value = json!({"schemaVersion":1,"items":[{"body":"本文中の [end of text] と /Users/fictional は保持"}]});
    let bytes = format!("{}\n[end of text]\n", value).into_bytes();
    let path = directory.join("result.json");
    std::fs::write(&path, &bytes).unwrap();
    let asset = storage
        .register_analysis_artifact(&id, &path, "application/vnd.mon.llama-completion")
        .unwrap();
    let (backup, _) = write(&storage, output.path());
    let archive = verified(&backup, output.path());
    let saved = std::fs::read(&archive.assets[&asset.id]).unwrap();
    assert_eq!(
        crate::generation::llama::parse_output(&saved).unwrap(),
        value
    );
    let entry = archive
        .manifest
        .entries
        .iter()
        .find(|e| e.asset_id.as_ref() == Some(&asset.id))
        .unwrap();
    assert_eq!(entry.source_sha256.as_ref(), Some(&asset.content_hash));
    assert_ne!(entry.sha256, asset.content_hash);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    for bad in [
        b"{\"x\":1,\"x\":2}".as_slice(),
        b"{} trailing log",
        b"{} {}",
        b"{\"items\":[",
    ] {
        assert!(portable::portable_completion(bad, &mut BTreeSet::new()).is_err());
    }
}

#[test]
fn capture_manifest_removes_process_ids_power_handles_and_error_paths_without_changing_audio() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = super::super::super::analysis::tests::saved_meeting(&storage);
    let audio = storage
        .root
        .join(format!("meetings/{}/audio/source.caf", meeting.id));
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(&audio, b"fixture audio").unwrap();
    storage.connection.lock().unwrap().execute("UPDATE file_assets SET relative_path=?1,size_bytes=13,content_hash=?2 WHERE id='asset'",params![format!("meetings/{}/audio/source.caf",meeting.id),format!("{:x}",Sha256::digest(b"fixture audio"))]).unwrap();
    let relative = format!("meetings/{}/audio/manifest.json", meeting.id);
    let manifest_path = storage.root.join(&relative);
    let application_identity = json!({"pid":777,"bundleIdentifier":"com.example.private-app","launchTimeUs":1791100000000001_i64});
    std::fs::write(&manifest_path,serde_json::to_vec(&json!({"schemaVersion":4,"inputDiagnosticsState":"unavailable","recordingOwnerId":Uuid::new_v4().to_string(),"captureMode":"application","backend":"screencapturekit","targetPID":777,"targetPIDs":[777],"targetApplicationName":"Meeting App","targetApplicationIdentity":application_identity,"microphoneDeviceUID":"private-device-123","state":"saved","microphone":{"frames":48000,"sampleRate":48000,"channels":1,"appendFailures":0},"system":{"frames":48000,"sampleRate":48000,"channels":2,"appendFailures":0},"errors":["file error /Users/source-machine/recording"],"power":{"assertionId":123}})).unwrap()).unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting.id, Path::new(&relative), "application/json")
        .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meeting_audio_commits SET manifest_asset_id=?1,manifest_hash=?2",
            params![asset.id, asset.content_hash],
        )
        .unwrap();
        db.execute("DELETE FROM file_assets WHERE id='manifest'", [])
            .unwrap();
    }
    storage.connection.lock().unwrap().execute("UPDATE recording_sessions SET audio_manifest_hash=?1,capture_config_json=?2,capture_mode='application'",params![asset.content_hash,json!({"mode":"application","backend":"screencapturekit","target_pids":[777],"target_application_identity":application_identity,"microphone_device_uid":"private-device-123"}).to_string()]).unwrap();
    let tx_connection = storage.connection.lock().unwrap();
    let tx = tx_connection.unchecked_transaction().unwrap();
    super::super::capture_preferences::save(&tx, "private-device-123").unwrap();
    tx.commit().unwrap();
    drop(tx_connection);
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let portable: Value =
        serde_json::from_slice(&std::fs::read(&archive.assets[&asset.id]).unwrap()).unwrap();
    assert_eq!(portable["targetPIDs"], json!([]));
    assert!(portable["targetPID"].is_null());
    assert!(portable.get("power").is_none());
    assert!(portable.get("microphoneDeviceUID").is_none());
    assert!(portable.get("targetApplicationIdentity").is_none());
    assert_eq!(portable["errors"], json!([]));
    assert_eq!(portable["backupRemovedErrorCount"], 1);
    let condition: String = archive
        .database
        .query_row(
            "SELECT capture_config_json FROM recording_sessions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&condition).unwrap()["target_pids"],
        json!([])
    );
    assert!(serde_json::from_str::<Value>(&condition).unwrap()["microphone_device_uid"].is_null());
    assert!(!condition.contains("private-device-123"));
    assert!(
        serde_json::from_str::<Value>(&condition).unwrap()["target_application_identity"].is_null()
    );
    assert!(!condition.contains("com.example.private-app"));
    assert!(!condition.contains("1791100000000001"));
    assert_eq!(
        storage.selected_microphone_uid().unwrap().as_deref(),
        Some("private-device-123")
    );
    assert!(!archive
        .manifest
        .tables
        .iter()
        .any(|table| table.name == "app_settings"));
    let original: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(original["microphoneDeviceUID"], "private-device-123");
    assert_eq!(original["targetApplicationIdentity"], application_identity);
    let entry = archive
        .manifest
        .entries
        .iter()
        .find(|e| e.asset_id.as_deref() == Some("asset"))
        .unwrap();
    assert_eq!(entry.sha256, entry.source_sha256.clone().unwrap());
    assert!(storage.verify_asset(&asset.id).unwrap());
}

#[test]
fn capture_journal_diagnostics_survive_verified_backup_additive_restore_and_rebackup() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, owner, directory, mut manifest) =
        super::super::super::recordings::tests::saved_probe_fixture(&storage);
    let persistence =
        json!({"requestedRevision":3,"committedRevision":3,"state":"confirmed","closed":true});
    manifest["recordingOwnerId"] = json!(owner);
    manifest["inputEventJournalPersistence"] = persistence.clone();
    manifest["inputDiagnosticsState"] = json!("unconfirmed");
    // A timeout followed by a confirmed snapshot is still unconfirmed at stop.
    manifest["inputEventJournalError"] = json!("sync timed out: /Users/source/private/recording");
    let original = serde_json::to_vec(&manifest).unwrap();
    let manifest_path = directory.join("manifest.json");
    std::fs::write(&manifest_path, &original).unwrap();
    storage.finalize_recording(&owner).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(&original));
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let entry = archive
        .manifest
        .entries
        .iter()
        .find(|e| e.source_sha256.as_deref() == Some(&source_hash))
        .unwrap();
    let source_asset = entry.asset_id.as_ref().unwrap();
    let encoded = std::fs::read(&archive.assets[source_asset]).unwrap();
    let portable: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(portable["inputEventJournalPersistence"], persistence);
    assert!(portable["inputEventJournalError"].is_string());
    assert!(!String::from_utf8(encoded.clone())
        .unwrap()
        .contains("/Users/source/private"));
    assert_eq!(portable["state"], "saved");
    assert_eq!(portable["microphone"], manifest["microphone"]);
    assert_eq!(portable["system"], manifest["system"]);
    assert_eq!(entry.source_sha256.as_deref(), Some(source_hash.as_str()));
    assert_ne!(entry.sha256, source_hash);

    let ids = super::super::restore_ids::RestoreIds::prepare(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let new_owner = ids.get("recording_sessions", &meeting, &owner).unwrap();
    let new_asset = ids.get("file_assets", &meeting, source_asset).unwrap();
    assert_ne!(new_owner, owner);
    let fixed = super::super::restore_fixed::FixedData::new(
        &ids,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let sealed = super::super::restore_fixed::workspace::seal::SealedWorkspace::prepare(
        &fixed,
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let existing = target.create_meeting("既存会議は保持").unwrap();
    target
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(target.get_meeting(&existing.id).unwrap().is_some());
    let restored = target
        .root
        .join(format!("meetings/{new_meeting}/restored/{new_asset}"));
    assert_eq!(std::fs::read(&restored).unwrap(), encoded);
    assert!(target.verify_asset(&new_asset).unwrap());
    // The capture owner remains ORIGINAL producer provenance in the manifest;
    // it is not permission to receive notices for the newly mapped session.
    assert_eq!(portable["recordingOwnerId"], owner);
    let next_path = output.path().join("rebackup.monbackup");
    target
        .create_backup_archive(&next_path, &crate::test_support::never_cancelled)
        .unwrap();
    let next = verified(&next_path, output.path());
    assert_eq!(std::fs::read(&next.assets[&new_asset]).unwrap(), encoded);
    assert_eq!(std::fs::read(&manifest_path).unwrap(), original);
    assert_eq!(storage.get_meeting(&meeting).unwrap().unwrap().id, meeting);
}

#[test]
fn capture_journal_invalid_final_diagnostic_refuses_backup_publication_without_changing_source() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let (meeting, owner, directory, mut manifest) =
        super::super::super::recordings::tests::saved_probe_fixture(&storage);
    manifest["recordingOwnerId"] = json!(owner);
    manifest["inputEventJournalPersistence"] =
        json!({"requestedRevision":3,"committedRevision":2,"state":"failed","closed":false});
    let original = serde_json::to_vec(&manifest).unwrap();
    let manifest_path = directory.join("manifest.json");
    std::fs::write(&manifest_path, &original).unwrap();
    storage.finalize_recording(&owner).unwrap();
    let destination = output.path().join("invalid.monbackup");
    let failure = storage
        .create_backup_archive(&destination, &crate::test_support::never_cancelled)
        .unwrap_err();
    assert!(!failure.published);
    assert!(!destination.exists());
    assert!(failure.message.contains("停止時の履歴診断の結果"));
    assert_eq!(std::fs::read(&manifest_path).unwrap(), original);
    assert_eq!(storage.get_meeting(&meeting).unwrap().unwrap().id, meeting);
    // Ordinary data admission is restored after this unpublished failure.
    storage.create_meeting("失敗後も既存領域を保持").unwrap();
}

#[test]
fn reader_rejects_truncated_appended_tampered_and_oversized_headers_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.create_meeting("空の会議").unwrap();
    let (path, _) = write(&storage, output.path());
    let original = std::fs::read(&path).unwrap();
    let directory_entries = std::fs::read_dir(output.path()).unwrap().count();
    for kind in ["truncated", "appended", "digest", "oversized", "magic"] {
        let mut bytes = original.clone();
        match kind {
            "truncated" => {
                bytes.pop();
            }
            "appended" => bytes.push(0),
            "digest" => bytes[12] ^= 1,
            "oversized" => bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes()),
            "magic" => bytes[0] = 0,
            _ => unreachable!(),
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(
            read(
                &mut File::open(&path).unwrap(),
                output.path(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(
            std::fs::read_dir(output.path()).unwrap().count(),
            directory_entries
        );
    }
}

#[test]
fn reader_rejects_rehashed_manifest_path_version_duplicate_type_size_and_unknown_fields() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    meeting(&storage, "import");
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let data = payloads(&archive);
    for path_attack in [
        "../escape",
        "/absolute",
        "assets/../escape",
        "assets\\escape",
        "assets/not-a-uuid",
    ] {
        let mut m = archive.manifest.clone();
        m.entries[1].path = path_attack.into();
        pack(&path, &m, &data);
        assert!(
            read(
                &mut File::open(&path).unwrap(),
                output.path(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{path_attack}"
        );
    }
    for kind in ["format", "schema", "duplicate", "size", "table", "type"] {
        let mut m = archive.manifest.clone();
        match kind {
            "format" => m.format_version = 3,
            "schema" => m.schema_version = usize::MAX,
            "duplicate" => m.entries[2].path = m.entries[1].path.clone(),
            "size" => m.entries[1].size = u64::MAX,
            "table" => m.tables[0].name = "meetings; DROP TABLE meetings;--".into(),
            "type" => m.tables[0].columns[0].data_type = "BLOB".into(),
            _ => unreachable!(),
        }
        pack(&path, &m, &data);
        assert!(
            read(
                &mut File::open(&path).unwrap(),
                output.path(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{kind}"
        );
    }
    let mut m = serde_json::to_value(&archive.manifest).unwrap();
    m["executeSql"] = json!("DELETE FROM meetings");
    let bytes = serde_json::to_vec(&m).unwrap();
    let mut file = File::create(&path).unwrap();
    file.write_all(MAGIC).unwrap();
    file.write_all(&(bytes.len() as u32).to_le_bytes()).unwrap();
    file.write_all(&Sha256::digest(&bytes)).unwrap();
    file.write_all(&bytes).unwrap();
    for bytes in data {
        file.write_all(&bytes).unwrap();
    }
    drop(file);
    assert!(read(
        &mut File::open(&path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}

fn edit_database(archive: &VerifiedArchive, path: &Path, mut edit: impl FnMut(&mut Value)) {
    let mut m = archive.manifest.clone();
    let mut data = payloads(archive);
    let mut output = Vec::new();
    for line in std::str::from_utf8(&data[0]).unwrap().lines() {
        let mut row: Value = serde_json::from_str(line).unwrap();
        edit(&mut row);
        output.extend_from_slice(&serde_json::to_vec(&row).unwrap());
        output.push(b'\n');
    }
    m.entries[0].size = output.len() as u64;
    m.entries[0].sha256 = format!("{:x}", Sha256::digest(&output));
    data[0] = output;
    pack(path, &m, &data);
}

#[test]
fn reader_rejects_rehashed_cross_meeting_heads_and_dangling_foreign_keys_without_live_writes() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let first = meeting(&storage, "first");
    let second = meeting(&storage, "second");
    let other: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT current_analysis_id FROM meetings WHERE id=?1",
            params![second],
            |r| r.get(0),
        )
        .unwrap();
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let columns = &archive
        .manifest
        .tables
        .iter()
        .find(|t| t.name == "meetings")
        .unwrap()
        .columns;
    let id = columns.iter().position(|c| c.name == "id").unwrap();
    let head = columns
        .iter()
        .position(|c| c.name == "current_analysis_id")
        .unwrap();
    for target in [other.as_str(), "missing-analysis"] {
        edit_database(&archive, &path, |row| {
            if row["table"] == "meetings" && row["cells"][id]["value"] == first {
                row["cells"][head] = json!({"type":"text","value":target});
            }
        });
        assert!(read(
            &mut File::open(&path).unwrap(),
            output.path(),
            &crate::test_support::never_cancelled
        )
        .is_err());
    }
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
    assert!(storage.create_meeting("元データは書き込み可能").is_ok());
}

#[test]
fn cancellation_preserves_existing_backup_and_releases_maintenance_and_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    meeting(&storage, "import");
    let path = output.path().join("meeting.monbackup");
    std::fs::write(&path, b"existing backup bytes").unwrap();
    let calls = Cell::new(0);
    let error = storage
        .create_backup_archive(&path, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 25
        })
        .err()
        .unwrap();
    assert!(!error.published);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing backup bytes");
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 1);
    assert!(storage.create_meeting("書き込み再開").is_ok());
}

#[test]
fn successful_replace_is_verified_and_managed_paths_and_symlinks_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.create_meeting("保存").unwrap();
    let path = output.path().join("meeting.monbackup");
    std::fs::write(&path, b"old file").unwrap();
    let receipt = storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        verified(&path, output.path()).manifest.archive_id,
        receipt.archive_id
    );
    assert!(storage
        .create_backup_archive(
            &storage.root.join("forbidden.monbackup"),
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(storage
        .create_backup_archive(
            &output.path().join("wrong.md"),
            &crate::test_support::never_cancelled
        )
        .is_err());
    #[cfg(unix)]
    {
        let alias = output.path().join("alias.monbackup");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(storage
            .create_backup_archive(&alias, &crate::test_support::never_cancelled)
            .is_err());
    }
}

#[test]
fn reader_rejects_rehashed_machine_paths_availability_and_missing_model_metadata() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    meeting(&storage, "import");
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let profile = archive
        .manifest
        .tables
        .iter()
        .find(|t| t.name == "analysis_revisions")
        .unwrap()
        .columns
        .iter()
        .position(|c| c.name == "profile_snapshot_json")
        .unwrap();
    for kind in ["path", "availability"] {
        edit_database(&archive, &path, |row| {
            if row["table"] == "analysis_revisions" {
                let mut config: Value =
                    serde_json::from_str(row["cells"][profile]["value"].as_str().unwrap()).unwrap();
                if kind == "path" {
                    config["model"]["path"] = json!("/Users/other-machine/model.gguf");
                } else {
                    config["available"] = json!(true);
                }
                row["cells"][profile]["value"] = json!(config.to_string());
            }
        });
        assert!(read(
            &mut File::open(&path).unwrap(),
            output.path(),
            &crate::test_support::never_cancelled
        )
        .err()
        .unwrap()
        .contains("端末固有"));
    }
    let mut m = archive.manifest.clone();
    m.file_references.clear();
    pack(&path, &m, &payloads(&archive));
    assert!(read(
        &mut File::open(&path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled
    )
    .err()
    .unwrap()
    .contains("メタデータ"));
}

#[test]
fn reader_rejects_column_value_types_null_required_cells_and_unknown_row_fields() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.create_meeting("保存").unwrap();
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let title = archive.manifest.tables[0]
        .columns
        .iter()
        .position(|c| c.name == "title")
        .unwrap();
    for kind in ["wrong_type", "null", "unknown", "row_table"] {
        edit_database(&archive, &path, |row| {
            if row["table"] == "meetings" {
                match kind {
                    "wrong_type" => row["cells"][title] = json!({"type":"integer","value":4}),
                    "null" => row["cells"][title] = json!({"type":"null"}),
                    "unknown" => row["sql"] = json!("DROP TABLE meetings"),
                    "row_table" => row["table"] = json!("app_settings"),
                    _ => unreachable!(),
                }
            }
        });
        assert!(
            read(
                &mut File::open(&path).unwrap(),
                output.path(),
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{kind}"
        );
    }
    assert_eq!(storage.list_meetings().unwrap()[0].title, "保存");
}

#[test]
fn payload_corruption_and_read_cancellation_clean_only_owned_staging() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    meeting(&storage, "import");
    let (path, _) = write(&storage, output.path());
    let original = std::fs::read(&path).unwrap();
    let mut changed = original.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert!(read(
        &mut File::open(&path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled
    )
    .err()
    .unwrap()
    .contains("ハッシュ"));
    std::fs::write(&path, &original).unwrap();
    let keep = output.path().join("unrelated");
    std::fs::write(&keep, b"keep").unwrap();
    let count = std::fs::read_dir(output.path()).unwrap().count();
    let calls = Cell::new(0);
    assert!(read(&mut File::open(&path).unwrap(), output.path(), &|| {
        calls.set(calls.get() + 1);
        calls.get() > 4
    })
    .err()
    .unwrap()
    .contains("中断"));
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), count);
    assert_eq!(std::fs::read(&keep).unwrap(), b"keep");
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn replacing_large_backup_is_streamed_without_markdown_size_limit() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.create_meeting("保存").unwrap();
    let path = output.path().join("large.monbackup");
    File::create(&path)
        .unwrap()
        .set_len(65 * 1024 * 1024)
        .unwrap();
    let receipt = storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(receipt.size < 65 * 1024 * 1024);
    assert_eq!(
        verified(&path, output.path()).manifest.archive_id,
        receipt.archive_id
    );
    let markdown = output.path().join("large.md");
    File::create(&markdown)
        .unwrap()
        .set_len(65 * 1024 * 1024)
        .unwrap();
    assert!(crate::exports::Destination::selected(&storage.root, &markdown).is_err());
}
