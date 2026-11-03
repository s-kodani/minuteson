//! Real archive bytes and owned SQLite/synthetic import files; no user data.
use super::*;
use crate::{analysis_worker::AnalysisController, storage::deletion::audio};
use serde_json::{json, Value};

#[path = "deleted_audio_publication_tests.rs"]
mod publication_tests;

pub(in crate::storage::backup) fn deleted(storage: &Storage, meeting: &str) -> String {
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let op = Uuid::new_v4().to_string();
    let census =
        audio::Census::capture(storage, meeting, &crate::test_support::never_cancelled).unwrap();
    audio::intent::prepare(
        storage,
        &op,
        &census,
        audio::intent::Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    audio::intent::removal::start(storage, &op, &lease, &crate::test_support::never_cancelled)
        .unwrap();
    audio::intent::removal::run(storage, &op, &lease, &crate::test_support::never_cancelled)
        .unwrap();
    audio::intent::removal::completion::finish(
        storage,
        &op,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
    let receipt = storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let archive = read(
        &mut File::open(path).unwrap(),
        output,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
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
    let path: String = db.query_row("SELECT f.relative_path FROM meeting_audio_commits a JOIN audio_chunks c ON c.track_id=a.track_id JOIN file_assets f ON f.id=c.file_asset_id WHERE a.meeting_id=?1", [meeting], |r|r.get(0)).unwrap();
    storage.root.join(path)
}

fn historical_plan(
    a: &VerifiedArchive,
    meeting: &str,
) -> Result<crate::storage::transcription::RecordedAudioPlan, String> {
    crate::storage::transcription::read_historical_audio_plan(
        &a.database,
        meeting,
        &a.deleted_audio,
    )
}

fn change_logical(a: &VerifiedArchive, sql: &str, meeting: &str) {
    // Keep the production connection query-only. This second writer attacks
    // only the owned validation image, never a live/native database.
    let writer = Connection::open(a.stage.path().join("validation.sqlite3")).unwrap();
    writer.execute(sql, [meeting]).unwrap();
}

#[test]
fn deleted_audio_normal_export_keeps_current_format_and_no_deletion_claim() {
    let (_root, storage, _meeting) = fixture();
    let output = tempfile::tempdir().unwrap();
    let (archive, receipt) = write(&storage, output.path());
    assert_eq!(
        archive.manifest.format_version,
        crate::current_formats::BACKUP
    );
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
            .create_backup_archive(&destination, &crate::test_support::never_cancelled)
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
        .create_backup_archive(
            &output.path().join("blocked.monbackup"),
            &crate::test_support::never_cancelled
        )
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
                .create_backup_archive(
                    &output.path().join("blocked.monbackup"),
                    &crate::test_support::never_cancelled
                )
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
    let snapshot = storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    std::fs::write(&path, b"returned source").unwrap();
    let output = tempfile::tempdir().unwrap();
    let mut file = File::create(output.path().join("not-published.monbackup")).unwrap();
    assert!(snapshot
        .write_archive(
            &mut file,
            output.path(),
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert_eq!(file.metadata().unwrap().len(), 0);
    assert_eq!(std::fs::read(path).unwrap(), b"returned source");
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
                    read(
                        &mut File::open(&path).unwrap(),
                        output.path(),
                        &crate::test_support::never_cancelled
                    )
                    .is_err(),
                    "{attack}"
                );
            }
            Err(_) => assert!(matches!(attack, "reason" | "nativePermission")),
        }
    }
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

#[test]
fn deleted_audio_current_documents_roundtrip_three_times_with_available_neighbor_and_replay() {
    let (_root, mut source, meeting) = fixture();
    source
        .rename_meeting(
            &meeting,
            "削除済み",
            source.get_meeting(&meeting).unwrap().unwrap().lock_version,
        )
        .unwrap();
    crate::storage::current_minutes_fixture::save(&source, &meeting, "# 保持する議事録\n");
    deleted(&source, &meeting);
    let available = tests::meeting(&source, "available-neighbor");
    source
        .rename_meeting(
            &available,
            "音声あり",
            source
                .get_meeting(&available)
                .unwrap()
                .unwrap()
                .lock_version,
        )
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut roots = Vec::new();
    for generation in 0..3 {
        let path = out.path().join(format!("current-{generation}.monbackup"));
        let receipt = source.create_backup_archive(&path, &|| false).unwrap();
        assert_eq!(receipt.meeting_count, 2);
        assert_eq!(receipt.deleted_audio_meeting_count, 1);
        assert_eq!(receipt.deleted_audio_asset_count, 1);
        let target_root = tempfile::tempdir().unwrap();
        let target = Storage::open(target_root.path().into()).unwrap();
        let prepared = target
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        let p = prepared.preview();
        assert_eq!(p.deleted_audio_meeting_count, 1);
        let restored = prepared
            .publish(&target, &p.confirmation_id, &|| false, &|_| {})
            .unwrap();
        assert_eq!(restored.meeting_ids.len(), 2);
        assert!(
            prepared
                .publish(&target, &p.confirmation_id, &|| false, &|_| {})
                .unwrap()
                .already_published
        );
        let meetings = target.list_meetings().unwrap();
        assert_eq!(meetings.len(), 2);
        let missing = &meetings.iter().find(|m| m.title == "削除済み").unwrap().id;
        assert!(target.recorded_audio_plan(missing).is_err());
        assert_eq!(
            crate::storage::minutes_markdown::read(&target.connection.lock().unwrap(), missing)
                .unwrap()
                .unwrap()
                .markdown,
            "# 保持する議事録\n"
        );
        let view = target.load_current_analysis(missing).unwrap().unwrap();
        assert_eq!(view.utterances.len(), 1);
        assert!(target.load_transcript_draft(missing).unwrap().is_none());
        let present = &meetings.iter().find(|m| m.title == "音声あり").unwrap().id;
        target.verify_canonical_audio(present, &|| false).unwrap();
        assert_eq!(
            target
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM processing_jobs", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(prepared);
        roots.push(target_root);
        source = target;
    }
}
