//! Real archive bytes and owned SQLite/synthetic import files; no user data.
use super::*;
use crate::{analysis_worker::AnalysisController, storage::deletion::audio};
use serde_json::{json, Value};

fn deleted(storage: &Storage, meeting: &str) -> String {
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let op = Uuid::new_v4().to_string();
    let census = audio::Census::capture(storage, meeting, &|| false).unwrap();
    audio::intent::prepare(
        storage,
        &op,
        &census,
        audio::intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &lease,
        &|| false,
    )
    .unwrap();
    audio::intent::removal::start(storage, &op, &lease, &|| false).unwrap();
    audio::intent::removal::run(storage, &op, &lease, &|| false).unwrap();
    audio::intent::removal::completion::finish(storage, &op, &lease, &|| false).unwrap();
    op
}
fn fixture() -> (tempfile::TempDir, Storage, String) {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().join("root")).unwrap();
    let meeting = tests::meeting(&storage, "deleted-audio-import");
    (root, storage, meeting)
}
fn write(storage: &Storage, output: &Path) -> (VerifiedArchive, BackupReceipt) {
    let path = output.join(format!("{}.monbackup", Uuid::new_v4()));
    let receipt = storage.create_backup_archive(&path, &|| false).unwrap();
    let archive = read(&mut File::open(path).unwrap(), output, &|| false).unwrap();
    (archive, receipt)
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
fn payloads(a: &VerifiedArchive) -> Vec<Vec<u8>> {
    a.manifest
        .entries
        .iter()
        .map(|e| std::fs::read(a.stage.path().join(&e.path)).unwrap())
        .collect()
}
fn audio_path(storage: &Storage, meeting: &str) -> PathBuf {
    let db = storage.connection.lock().unwrap();
    let path: String = db.query_row("SELECT relative_path FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' ORDER BY id LIMIT 1", [meeting], |r|r.get(0)).unwrap();
    storage.root.join(path)
}

#[test]
fn deleted_audio_archive_preserves_text_drafts_references_and_import_history_without_source_bodies()
{
    let (_root, storage, meeting) = fixture();
    let candidate = crate::storage::minutes_ui::tests::candidate(&storage, &meeting);
    storage
        .adopt_minutes(crate::storage::minutes_ui::tests::request(
            &storage, &meeting, &candidate,
        ))
        .unwrap();
    storage.create_minutes_draft(&meeting, &candidate).unwrap();
    let neighbor = tests::meeting(&storage, "neighbor-audio-import");
    let output = tempfile::tempdir().unwrap();
    let db = storage.connection.lock().unwrap();
    let text: String = db
        .query_row(
            "SELECT effective_text FROM utterances WHERE meeting_id=?1 ORDER BY id LIMIT 1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let asset_ids: BTreeSet<String> = db
        .prepare("SELECT id FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%'")
        .unwrap()
        .query_map([&meeting], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let history: (i64,i64,i64,i64,i64) = db.query_row("SELECT (SELECT count(*) FROM working_drafts WHERE meeting_id=?1),(SELECT count(*) FROM audio_imports WHERE meeting_id=?1),(SELECT count(*) FROM audio_import_commits WHERE meeting_id=?1),(SELECT count(*) FROM minutes_revisions WHERE meeting_id=?1),(SELECT count(*) FROM evidence_links e JOIN minutes_revisions m ON m.id=e.minutes_revision_id WHERE m.meeting_id=?1)", [&meeting], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert!(history.3 > 0 && history.4 > 0);
    drop(db);
    deleted(&storage, &meeting);
    let (archive, receipt) = write(&storage, output.path());
    assert_eq!(archive.manifest.format_version, 3);
    assert_eq!(receipt.deleted_audio_meeting_count, 1);
    assert_eq!(receipt.deleted_audio_asset_count, asset_ids.len());
    assert_eq!(receipt.omitted_pcm_count, 0);
    assert_eq!(archive.manifest.deleted_audio.len(), 1);
    assert!(archive.deleted_audio.contains(&meeting));
    assert!(!archive.deleted_audio.contains(&neighbor));
    assert!(asset_ids.iter().all(|id| !archive.assets.contains_key(id)));
    assert_eq!(
        archive
            .manifest
            .omitted_assets
            .iter()
            .filter(|a| a.reason == OmittedReason::SourceAudioDeleted)
            .map(|a| a.asset_id.clone())
            .collect::<BTreeSet<_>>(),
        asset_ids
    );
    assert_eq!(
        archive
            .database
            .query_row(
                "SELECT effective_text FROM utterances WHERE meeting_id=?1 ORDER BY id LIMIT 1",
                [&meeting],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        text
    );
    assert_eq!(archive.database.query_row("SELECT (SELECT count(*) FROM working_drafts WHERE meeting_id=?1),(SELECT count(*) FROM audio_imports WHERE meeting_id=?1),(SELECT count(*) FROM audio_import_commits WHERE meeting_id=?1),(SELECT count(*) FROM minutes_revisions WHERE meeting_id=?1),(SELECT count(*) FROM evidence_links e JOIN minutes_revisions m ON m.id=e.minutes_revision_id WHERE m.meeting_id=?1)", [&meeting], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap(), history);
    let available: bool = archive.database.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state!='missing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND state!='failed') OR EXISTS(SELECT 1 FROM audio_chunks WHERE meeting_id=?1 AND state!='missing')", [&meeting], |r|r.get(0)).unwrap();
    assert!(!available);
    assert_eq!(
        archive
            .database
            .query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let evidence = serde_json::to_string(&archive.manifest.deleted_audio).unwrap();
    for secret in [
        storage.root.to_str().unwrap(),
        "/Users/source-machine",
        "relative_path",
        "operationId",
        "target_manifest_json",
        "点検日",
    ] {
        assert!(!evidence.contains(secret), "{secret}");
    }
    assert!(storage.get_meeting(&meeting).unwrap().is_some());
    assert!(audio_path(&storage, &neighbor).is_file());
    assert!(storage.recorded_audio_plan(&meeting).is_err());
}

#[test]
fn deleted_audio_normal_export_keeps_format_two_and_no_deletion_claim() {
    let (_root, storage, _meeting) = fixture();
    let output = tempfile::tempdir().unwrap();
    let (archive, receipt) = write(&storage, output.path());
    assert_eq!(archive.manifest.format_version, 2);
    assert!(archive.deleted_audio.is_empty());
    assert_eq!(
        (
            receipt.deleted_audio_meeting_count,
            receipt.deleted_audio_asset_count
        ),
        (0, 0)
    );
    assert!(serde_json::to_value(&archive.manifest)
        .unwrap()
        .get("deletedAudio")
        .is_none());
}

#[test]
fn deleted_audio_unjournaled_missing_source_is_not_adopted_as_intentional_deletion() {
    for state in ["ready", "missing"] {
        let (_root, storage, meeting) = fixture();
        let path = audio_path(&storage, &meeting);
        std::fs::remove_file(path).unwrap();
        if state == "missing" {
            storage.connection.lock().unwrap().execute("UPDATE file_assets SET state='missing' WHERE meeting_id=?1 AND media_type LIKE 'audio/%'", [&meeting]).unwrap();
        }
        let output = tempfile::tempdir().unwrap();
        let destination = output.path().join("blocked.monbackup");
        assert!(storage
            .create_backup_archive(&destination, &|| false)
            .is_err());
        assert!(!destination.exists());
        assert!(storage.get_meeting(&meeting).unwrap().is_some());
    }
}

#[test]
fn deleted_audio_native_state_revival_stops_export_without_reclassifying_missing_source() {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET state='ready' WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    assert!(storage
        .create_backup_archive(&output.path().join("blocked.monbackup"), &|| false)
        .is_err());
}

#[test]
fn deleted_audio_reappeared_body_link_or_parent_is_never_silently_omitted() {
    use std::os::unix::fs::symlink;
    for attack in ["file", "link", "parent"] {
        let (_root, storage, meeting) = fixture();
        let path = audio_path(&storage, &meeting);
        deleted(&storage, &meeting);
        let output = tempfile::tempdir().unwrap();
        let external = output.path().join("untouched.wav");
        std::fs::write(&external, b"external source").unwrap();
        match attack {
            "file" => std::fs::write(&path, b"reappeared source").unwrap(),
            "link" => symlink(&external, &path).unwrap(),
            "parent" => {
                let parent = path.parent().unwrap();
                std::fs::rename(parent, parent.with_extension("kept")).unwrap();
                std::fs::create_dir(parent).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            storage
                .create_backup_archive(&output.path().join("blocked.monbackup"), &|| false)
                .is_err(),
            "{attack}"
        );
        assert_eq!(std::fs::read(&external).unwrap(), b"external source");
    }
}

#[test]
fn deleted_audio_snapshot_rechecks_absence_before_archive_bytes_are_written() {
    let (_root, storage, meeting) = fixture();
    let path = audio_path(&storage, &meeting);
    deleted(&storage, &meeting);
    let snapshot = storage.prepare_backup_snapshot(&|| false).unwrap();
    std::fs::write(&path, b"returned source").unwrap();
    let output = tempfile::tempdir().unwrap();
    let mut file = File::create(output.path().join("not-published.monbackup")).unwrap();
    assert!(snapshot
        .write_archive(&mut file, output.path(), &|| false)
        .is_err());
    assert_eq!(file.metadata().unwrap().len(), 0);
    assert_eq!(std::fs::read(path).unwrap(), b"returned source");
}

#[test]
fn deleted_audio_all_registered_audio_including_uppercase_orphan_pcm_is_recorded_as_deleted() {
    let (_root, storage, meeting) = fixture();
    let relative = PathBuf::from(format!("meetings/{meeting}/scratch/orphan.wav"));
    let path = storage.root.join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"owned orphan pcm").unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting, &relative, "Audio/WAV")
        .unwrap();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&storage, output.path());
    assert!(!path.exists());
    assert_eq!(
        archive
            .manifest
            .omitted_assets
            .iter()
            .find(|a| a.asset_id == asset.id)
            .unwrap()
            .reason,
        OmittedReason::SourceAudioDeleted
    );
    assert!(!archive
        .manifest
        .omitted_assets
        .iter()
        .any(|a| a.reason == OmittedReason::RecreatablePcm));
}

#[test]
fn deleted_audio_rehashed_manifest_cannot_change_reason_scope_version_or_fingerprints() {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&storage, output.path());
    let data = payloads(&archive);
    let path = output.path().join("tampered.monbackup");
    for attack in [
        "missing",
        "duplicate",
        "meeting",
        "version",
        "reason",
        "hash",
        "count",
        "table",
        "nativePermission",
        "wrongFormat",
        "pcmReason",
        "logReason",
    ] {
        let mut value = serde_json::to_value(&archive.manifest).unwrap();
        match attack {
            "missing" => value["deletedAudio"] = json!([]),
            "duplicate" => {
                let item = value["deletedAudio"][0].clone();
                value["deletedAudio"].as_array_mut().unwrap().push(item);
            }
            "meeting" => value["deletedAudio"][0]["meetingId"] = json!(Uuid::new_v4().to_string()),
            "version" => value["deletedAudio"][0]["version"] = json!(2),
            "reason" => value["deletedAudio"][0]["reason"] = json!("ordinary_missing"),
            "hash" => value["deletedAudio"][0]["tables"][0]["sha256"] = json!("a".repeat(64)),
            "count" => value["deletedAudio"][0]["tables"][0]["rows"] = json!(0),
            "table" => value["deletedAudio"][0]["tables"][0]["name"] = json!("app_settings"),
            "nativePermission" => {
                value["deletedAudio"][0]["operationId"] = json!(Uuid::new_v4().to_string())
            }
            "wrongFormat" => value["formatVersion"] = json!(2),
            "pcmReason" => value["omittedAssets"][0]["reason"] = json!("recreatable_pcm"),
            "logReason" => value["omittedAssets"][0]["reason"] = json!("native_log"),
            _ => unreachable!(),
        }
        match serde_json::from_value::<Manifest>(value) {
            Ok(manifest) => {
                pack(&path, &manifest, &data);
                assert!(
                    read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
                    "{attack}"
                );
            }
            Err(_) => assert!(matches!(attack, "reason" | "nativePermission")),
        }
    }
}

#[test]
fn deleted_audio_rehashed_logical_rows_cannot_revive_or_rewrite_audio_history() {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&storage, output.path());
    let path = output.path().join("logical-tampered.monbackup");
    for (table, column, replacement) in [
        ("audio_tracks", "state", json!("ready")),
        ("audio_chunks", "state", json!("ready")),
        ("file_assets", "content_hash", json!("f".repeat(64))),
        ("audio_chunks", "frame_count", json!(48001)),
    ] {
        let mut data = payloads(&archive);
        let index = archive
            .manifest
            .tables
            .iter()
            .find(|t| t.name == table)
            .unwrap()
            .columns
            .iter()
            .position(|c| c.name == column)
            .unwrap();
        let mut rows: Vec<Value> = data[0]
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_slice(s).unwrap())
            .collect();
        let media = archive
            .manifest
            .tables
            .iter()
            .find(|t| t.name == "file_assets")
            .unwrap()
            .columns
            .iter()
            .position(|c| c.name == "media_type")
            .unwrap();
        let row = rows
            .iter_mut()
            .find(|r| {
                r["table"] == table
                    && (table != "file_assets"
                        || r["cells"][media]["value"]
                            .as_str()
                            .is_some_and(deleted_audio::audio_media))
            })
            .unwrap();
        row["cells"][index]["value"] = replacement;
        data[0] = rows
            .into_iter()
            .flat_map(|r| {
                let mut b = serde_json::to_vec(&r).unwrap();
                b.push(b'\n');
                b
            })
            .collect();
        let mut manifest = archive.manifest.clone();
        manifest.entries[0].size = data[0].len() as u64;
        manifest.entries[0].sha256 = format!("{:x}", Sha256::digest(&data[0]));
        pack(&path, &manifest, &data);
        assert!(
            read(&mut File::open(&path).unwrap(), output.path(), &|| false).is_err(),
            "{table}.{column}"
        );
    }
}

#[test]
fn deleted_audio_restore_stops_before_minting_native_ids_until_unavailable_publication_is_connected(
) {
    let (_root, storage, meeting) = fixture();
    deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (mut archive, _) = write(&storage, output.path());
    let before = std::fs::read_dir(output.path()).unwrap().count();
    // A raw manifest edit cannot erase the reader's private validated view.
    archive.manifest.deleted_audio.clear();
    archive.manifest.format_version = 2;
    let error = match super::super::restore_ids::RestoreIds::prepare(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &|| false,
    ) {
        Ok(_) => panic!("deleted source was treated as available"),
        Err(e) => e,
    };
    assert!(error.contains("追加復元はまだ未接続"));
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
}

#[test]
fn deleted_audio_export_cancellation_does_not_publish_or_mutate_source_history() {
    let (_root, storage, meeting) = fixture();
    let operation = deleted(&storage, &meeting);
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("cancelled.monbackup");
    assert!(storage.create_backup_archive(&path, &|| true).is_err());
    assert!(!path.exists());
    let db = storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            [&operation],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "completed"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM utterances WHERE meeting_id=?1",
            [&meeting],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
