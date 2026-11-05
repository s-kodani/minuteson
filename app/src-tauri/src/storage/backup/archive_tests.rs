use super::*;
use crate::audio_import::ImportOptions;
use serde_json::{json, Value};

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
    storage
        .publish_single_wav(&saved.meeting_id, &|| false, &|_, _| {})
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
fn current_contents_roundtrip_has_one_baseline_and_no_jobs_or_edits() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = meeting(&storage, "current-content");
    let out = tempfile::tempdir().unwrap();
    let (path, _) = write(&storage, out.path());
    let archive = verified(&path, out.path());
    assert_eq!(archive.manifest.format_version, 8);
    assert_eq!(archive.manifest.schema_version, SCHEMA_VERSION);
    for table in [
        "processing_jobs",
        "job_checkpoints",
        "working_drafts",
        "model_packages",
        "model_profiles",
        "export_records",
    ] {
        assert_eq!(
            archive
                .database
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        archive
            .database
            .query_row("SELECT count(*) FROM analysis_revisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        archive
            .database
            .query_row("SELECT effective_text FROM utterances", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "点検日を確認する。発言に含まれる /Users/fictional はそのまま保持。"
    );
    assert_eq!(
        archive
            .database
            .query_row(
                "SELECT term FROM vocabulary_entries WHERE scope_id=?1",
                [format!("meeting:{meeting}")],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "点検日"
    );
    let body = std::fs::read(path).unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("source-machine"));
    assert!(!serde_json::to_value(&archive.manifest)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("tables"));
}
#[test]
fn format8_reader_rejects_corruption_truncation_extra_bytes_and_old_versions() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    meeting(&storage, "corruption");
    let out = tempfile::tempdir().unwrap();
    let (path, _) = write(&storage, out.path());
    let body = std::fs::read(&path).unwrap();
    let mut manifest_corrupt = body.clone();
    manifest_corrupt[12] ^= 1;
    let mut payload_corrupt = body.clone();
    let last = payload_corrupt.len() - 1;
    payload_corrupt[last] ^= 1;
    for bytes in [
        body[..body.len() - 1].to_vec(),
        [body.as_slice(), b"extra"].concat(),
        manifest_corrupt,
        payload_corrupt,
    ] {
        let bad = out.path().join("bad.monbackup");
        std::fs::write(&bad, bytes).unwrap();
        assert!(read(&mut File::open(bad).unwrap(), out.path(), &|| false).is_err());
    }
    let archive = verified(&path, out.path());
    for (format, schema) in [
        (5, SCHEMA_VERSION),
        (6, SCHEMA_VERSION),
        (7, SCHEMA_VERSION),
        (8, 38),
    ] {
        let mut manifest = archive.manifest.clone();
        manifest.format_version = format;
        manifest.schema_version = schema;
        let bad = out.path().join("old.monbackup");
        pack(&bad, &manifest, &payloads(&archive));
        assert!(read(&mut File::open(bad).unwrap(), out.path(), &|| false)
            .err()
            .unwrap()
            .contains("対応"));
    }
}

#[test]
fn archive_rejects_wrong_paths_and_audio_even_with_recomputed_hashes() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    meeting(&s, "typed-tamper");
    let out = tempfile::tempdir().unwrap();
    let (path, _) = write(&s, out.path());
    let source = verified(&path, out.path());
    let mut m = source.manifest.clone();
    let mut data = payloads(&source);
    m.entries[1].path = "../outside".into();
    let bad = out.path().join("path.monbackup");
    pack(&bad, &m, &data);
    assert!(read(&mut File::open(bad).unwrap(), out.path(), &|| false).is_err());
    m = source.manifest.clone();
    let mut contents: Value = serde_json::from_slice(&data[0]).unwrap();
    let aid = contents["meetings"][0]["audio"]["asset"]["id"]
        .as_str()
        .unwrap();
    let index = m
        .entries
        .iter()
        .position(|e| e.asset_id.as_deref() == Some(aid))
        .unwrap();
    data[index][..4].copy_from_slice(b"NOPE");
    let hash = format!("{:x}", Sha256::digest(&data[index]));
    m.entries[index].sha256 = hash.clone();
    m.entries[index].source_sha256 = Some(hash.clone());
    contents["meetings"][0]["audio"]["asset"]["hash"] = Value::String(hash);
    data[0] = serde_json::to_vec(&contents).unwrap();
    m.entries[0].size = data[0].len() as u64;
    m.entries[0].sha256 = format!("{:x}", Sha256::digest(&data[0]));
    let bad = out.path().join("audio.monbackup");
    pack(&bad, &m, &data);
    assert!(read(&mut File::open(bad).unwrap(), out.path(), &|| false).is_err());
}
