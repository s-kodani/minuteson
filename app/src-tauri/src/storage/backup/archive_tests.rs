use super::*;
use crate::audio_import::ImportOptions;
use serde_json::{json, Value};
use std::cell::Cell;

fn meeting(storage: &Storage, operation: &str) -> String {
    let options = ImportOptions {
        profile: "high_accuracy".into(),
        meeting_vocabulary: vec![VocabularyEntry {
            id: Uuid::new_v4().to_string(),
            term: "点検日".into(),
            reading: Some("てんけんび".into()),
            note: None,
            enabled: true,
        }],
        expected_global_version: None,
    };
    let (import, _, _) =
        super::super::super::imports::tests::pending_configured(storage, operation, &options);
    let saved = storage.finalize_audio_import(&import.id).unwrap();
    let db = storage.connection.lock().unwrap();
    let track: String = db
        .query_row(
            "SELECT id FROM audio_tracks WHERE meeting_id=?1",
            params![saved.meeting_id],
            |r| r.get(0),
        )
        .unwrap();
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
    let receipt = storage.create_backup_archive(&path, &|| false).unwrap();
    (path, receipt)
}

fn verified(path: &Path, dir: &Path) -> VerifiedArchive {
    read(&mut File::open(path).unwrap(), dir, &|| false).unwrap()
}

fn pack(path: &Path, manifest: &Manifest, payloads: &[Vec<u8>]) {
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
fn archive_preserves_audio_options_text_fixed_evidence_and_model_metadata_without_host_paths() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let id = meeting(&storage, "import");
    let revision = super::super::super::minutes_ui::tests::candidate(&storage, &id);
    storage
        .adopt_minutes(super::super::super::minutes_ui::tests::request(
            &storage, &id, &revision,
        ))
        .unwrap();
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
    assert_eq!(receipt.asset_count, 3);
    assert_eq!(receipt.sha256, digest_file(&path, &|| false).unwrap().1);
    assert_eq!(archive.manifest.encryption, Encryption::None);
    assert_eq!(archive.manifest.hash_basis, HashBasis::SourceIdentity);
    assert_eq!(archive.manifest.file_references.len(), 2);
    assert!(archive
        .manifest
        .file_references
        .iter()
        .any(|r| r.name == "model.gguf" && r.sha256 == "b".repeat(64)));
    assert!(archive
        .manifest
        .file_references
        .iter()
        .any(|r| r.name == "llama.exe" && r.role == "executable"));
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
            .profile,
        "high_accuracy"
    );
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
        1
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
fn registered_recreatable_pcm_is_omitted_but_durable_source_wav_is_kept() {
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
            parameters: json!({}),
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
    assert_eq!(archive.manifest.omitted_assets[0].asset_id, asset.id);
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
    assert!(archive.assets.contains_key(&imported_original));
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
    std::fs::write(&manifest_path,serde_json::to_vec(&json!({"schemaVersion":1,"captureMode":"application","backend":"coreaudio","targetPID":777,"targetPIDs":[777],"targetApplicationName":"Meeting App","state":"saved","microphone":{"frames":48000,"sampleRate":48000,"channels":1,"appendFailures":0},"system":{"frames":48000,"sampleRate":48000,"channels":2,"appendFailures":0},"errors":["file error /Users/source-machine/recording"],"power":{"assertionId":123}})).unwrap()).unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting.id, Path::new(&relative), "application/json")
        .unwrap();
    storage.connection.lock().unwrap().execute("UPDATE recording_sessions SET audio_manifest_hash=?1,capture_config_json=?2,capture_mode='application'",params![asset.content_hash,json!({"mode":"application","backend":"coreaudio","target_pids":[777]}).to_string()]).unwrap();
    let (path, _) = write(&storage, output.path());
    let archive = verified(&path, output.path());
    let portable: Value =
        serde_json::from_slice(&std::fs::read(&archive.assets[&asset.id]).unwrap()).unwrap();
    assert_eq!(portable["targetPIDs"], json!([]));
    assert!(portable["targetPID"].is_null());
    assert!(portable.get("power").is_none());
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
            read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
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
            read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
            "{path_attack}"
        );
    }
    for kind in ["format", "schema", "duplicate", "size", "table", "type"] {
        let mut m = archive.manifest.clone();
        match kind {
            "format" => m.format_version = 2,
            "schema" => m.schema_version = usize::MAX,
            "duplicate" => m.entries[2].path = m.entries[1].path.clone(),
            "size" => m.entries[1].size = u64::MAX,
            "table" => m.tables[0].name = "meetings; DROP TABLE meetings;--".into(),
            "type" => m.tables[0].columns[0].data_type = "BLOB".into(),
            _ => unreachable!(),
        }
        pack(&path, &m, &data);
        assert!(
            read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
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
    assert!(read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err());
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
        assert!(read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err());
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
    let receipt = storage.create_backup_archive(&path, &|| false).unwrap();
    assert_eq!(
        verified(&path, output.path()).manifest.archive_id,
        receipt.archive_id
    );
    assert!(storage
        .create_backup_archive(&storage.root.join("forbidden.monbackup"), &|| false)
        .is_err());
    assert!(storage
        .create_backup_archive(&output.path().join("wrong.md"), &|| false)
        .is_err());
    #[cfg(unix)]
    {
        let alias = output.path().join("alias.monbackup");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(storage.create_backup_archive(&alias, &|| false).is_err());
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
        assert!(
            read(&mut File::open(&path).unwrap(), output.path(), &|| false)
                .err()
                .unwrap()
                .contains("端末固有")
        );
    }
    let mut m = archive.manifest.clone();
    m.file_references.clear();
    pack(&path, &m, &payloads(&archive));
    assert!(
        read(&mut File::open(&path).unwrap(), output.path(), &|| false)
            .err()
            .unwrap()
            .contains("メタデータ")
    );
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
            read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
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
    assert!(
        read(&mut File::open(&path).unwrap(), output.path(), &|| false)
            .err()
            .unwrap()
            .contains("ハッシュ")
    );
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
    let receipt = storage.create_backup_archive(&path, &|| false).unwrap();
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
