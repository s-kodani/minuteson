use super::*;
use rusqlite::types::Value;
use std::cell::Cell;

fn asset(storage: &Storage, meeting: &str, bytes: &[u8]) -> FileAsset {
    let relative = PathBuf::from(format!("meetings/{meeting}/audio/source.flac"));
    let path = storage.managed_path(&relative).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    storage
        .register_ready_meeting_asset(meeting, &relative, "audio/flac")
        .unwrap()
}

fn rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut statement = db
        .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn wal_snapshot_preserves_all_rows_markdown_transcript_edits_and_vocabulary() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = super::super::current_minutes_fixture::meeting(&storage);
    // Existing analysis fixture uses a fake path/hash. Materialize real managed
    // bytes without changing revision provenance, then include a second asset.
    let relative = format!("meetings/{meeting}/audio/original.caf");
    let path = storage.managed_path(Path::new(&relative)).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"fixture audio").unwrap();
    storage.connection.lock().unwrap().execute(
        "UPDATE file_assets SET relative_path=?1,size_bytes=?2,content_hash=?3 WHERE id='asset'",
        params![relative, 13, format!("{:x}", Sha256::digest(b"fixture audio"))],
    ).unwrap();
    asset(&storage, &meeting, b"second asset");
    let body = super::super::current_minutes_fixture::save(&storage, &meeting, "# 保持する追記\n");
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    draft
        .payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: storage
                .load_current_analysis(&meeting)
                .unwrap()
                .unwrap()
                .utterances[0]
                .id
                .clone(),
            effective_text: "保存済みの文字起こし編集".into(),
            speaker_ids: vec![],
        });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let entry = VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: "点検日".into(),
        reading: None,
        note: None,
        enabled: true,
    };
    storage
        .save_vocabulary_book(None, 0, vec![entry.clone()])
        .unwrap();
    storage
        .save_vocabulary_book(
            Some(&meeting),
            0,
            vec![VocabularyEntry {
                id: Uuid::new_v4().to_string(),
                ..entry
            }],
        )
        .unwrap();
    let wal = storage.root.join("meetings.sqlite3-wal");
    assert!(wal.metadata().unwrap().len() > 0);
    let snapshot = storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    let stage = snapshot.stage.path().to_path_buf();
    assert_eq!(snapshot.assets.len(), 2);
    assert_eq!(snapshot.meetings[0].id, meeting);
    let saved = read_only(&snapshot.database).unwrap();
    let original = storage.connection.lock().unwrap();
    let tables: Vec<String> = original.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    for table in tables {
        assert_eq!(rows(&original, &table), rows(&saved, &table), "{table}");
    }
    let markdown: String = saved
        .query_row(
            "SELECT markdown FROM minutes_markdown WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(markdown, body.markdown);
    assert_eq!(
        saved
            .query_row("SELECT count(*) FROM minutes_revisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        saved
            .query_row("SELECT count(*) FROM evidence_links", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(original);
    drop(saved);
    snapshot
        .verify(&crate::test_support::never_cancelled)
        .unwrap();
    drop(snapshot);
    assert!(!stage.exists());
    assert!(storage
        .rename_meeting(
            &meeting,
            "再開後",
            storage.get_meeting(&meeting).unwrap().unwrap().lock_version
        )
        .is_ok());
}

#[test]
fn maintenance_blocks_all_mutations_and_cached_writes_but_keeps_reads_available() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("元データ").unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.prepare_cached("UPDATE meetings SET title='cached' WHERE id=?1")
            .unwrap();
    }
    let guard = storage.backup_maintenance().unwrap();
    assert!(storage.backup_maintenance().is_err());
    assert_eq!(storage.list_meetings().unwrap(), vec![meeting.clone()]);
    assert!(storage.create_meeting("拒否").is_err());
    assert!(storage.rename_meeting(&meeting.id, "拒否", 0).is_err());
    {
        let db = storage.connection.lock().unwrap();
        for sql in [
            "DELETE FROM meetings",
            "CREATE TABLE forbidden(id)",
            "PRAGMA foreign_keys=OFF",
            "PRAGMA wal_checkpoint(TRUNCATE)",
            "PRAGMA optimize",
            "ATTACH DATABASE ':memory:' AS forbidden",
        ] {
            assert!(db.execute_batch(sql).is_err(), "{sql}");
        }
        assert!(db
            .prepare_cached("UPDATE meetings SET title='cached' WHERE id=?1")
            .is_err());
    }
    drop(guard);
    assert!(storage.rename_meeting(&meeting.id, "再開", 0).is_ok());
}

#[test]
fn snapshot_only_stages_registered_files_and_does_not_publish_raw_database() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("対象").unwrap();
    let managed = asset(&storage, &meeting.id, b"audio");
    for name in ["model.gguf", "temporary.pcm", "engine.log"] {
        std::fs::write(root.path().join(name), b"excluded binary").unwrap();
    }
    let snapshot = storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(snapshot.assets.len(), 1);
    assert_eq!(snapshot.assets[0].asset.id, managed.id);
    assert_eq!(std::fs::read(&snapshot.assets[0].path).unwrap(), b"audio");
    assert_eq!(std::fs::read_dir(snapshot.stage.path()).unwrap().count(), 2);
    assert!(!snapshot.database.with_extension("sqlite3-wal").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            snapshot
                .stage
                .path()
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            snapshot.database.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            snapshot.assets[0]
                .path
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn cancellation_during_copy_releases_gate_and_leaves_original_untouched() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("キャンセル").unwrap();
    let managed = asset(&storage, &meeting.id, &vec![7; 8 * BUFFER_BYTES]);
    let staging = tempfile::tempdir().unwrap();
    let copied = Cell::new(0_u64);
    assert!(storage
        .prepare_backup_snapshot_at(Some(staging.path()), &|| {
            // Cancel at a physical copy boundary, not at a brittle count of
            // validation callbacks. Inspect only this test's private parent.
            for stage in std::fs::read_dir(staging.path()).unwrap() {
                let assets = stage.unwrap().path().join("assets");
                if let Ok(files) = std::fs::read_dir(assets) {
                    for file in files {
                        let bytes = file.unwrap().metadata().unwrap().len();
                        if bytes > 0 {
                            copied.set(bytes);
                            return true;
                        }
                    }
                }
            }
            false
        })
        .err()
        .unwrap()
        .contains("中断"));
    assert!(copied.get() > 0 && copied.get() <= managed.size_bytes as u64);
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
    assert!(storage.verify_asset(&managed.id).unwrap());
    assert_eq!(storage.list_meetings().unwrap(), vec![meeting]);
    assert!(storage.create_meeting("再開").is_ok());
}

#[test]
fn panic_releases_maintenance_and_a_later_snapshot_can_succeed() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    storage.create_meeting("保持").unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = storage.backup_maintenance().unwrap();
        panic!("fixture panic outside DB mutex");
    }));
    assert!(result.is_err());
    assert!(storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_ok());
}

#[test]
fn active_recording_import_export_track_building_or_ai_job_cannot_start_backup() {
    for kind in ["recording", "track", "import", "export", "building", "job"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = storage.create_meeting(kind).unwrap();
        match kind {
            "recording" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE meetings SET source_type='recording' WHERE id=?1",
                        params![meeting.id],
                    )
                    .unwrap();
                storage.connection.lock().unwrap().execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES('s',?1,'preparing','system','{}','op',0,0)",params![meeting.id]).unwrap();
            }
            "import" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE meetings SET source_type='audio_import' WHERE id=?1",
                        params![meeting.id],
                    )
                    .unwrap();
                storage.connection.lock().unwrap().execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,created_at,updated_at) VALUES('i',?1,'copying','source.wav',0,0)",params![meeting.id]).unwrap();
            }
            "track" => {
                let db = storage.connection.lock().unwrap();
                db.execute(
                    "UPDATE meetings SET source_type='recording' WHERE id=?1",
                    params![meeting.id],
                )
                .unwrap();
                db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES('s',?1,'saved','system','{}','op',0,0)",params![meeting.id]).unwrap();
                db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES('t',?1,'s','mic','caf',48000,1,'writing',0,0)",params![meeting.id]).unwrap();
            }
            "export" => {
                storage.connection.lock().unwrap().execute("INSERT INTO export_records(id,meeting_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,format,destination_path,state,created_at,updated_at) VALUES('e',?1,?2,?2,?2,0,'markdown','/tmp/output.md','writing',0,0)",params![meeting.id,"a".repeat(64)]).unwrap();
            }
            "building" => {
                storage.connection.lock().unwrap().execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES('a',?1,1,'engine','building',?2,'{}','{}',?2,1,0)",params![meeting.id,"a".repeat(64)]).unwrap();
            }
            "job" => {
                storage
                    .enqueue_job(JobSpec {
                        meeting_id: meeting.id,
                        kind: JobKind::Transcription,
                        operation_id: "op".into(),
                        input_analysis_id: None,
                        input_hash: "a".repeat(64),
                        profile_snapshot: serde_json::json!({}),
                        parameters: serde_json::json!({"schemaVersion":4}),
                        total_units: Some(1),
                    })
                    .unwrap();
                storage.acquire_next_job().unwrap().unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            storage
                .prepare_backup_snapshot(&crate::test_support::never_cancelled)
                .err()
                .unwrap()
                .contains("停止"),
            "{kind}"
        );
        assert!(!storage.maintenance.load(Ordering::Acquire));
    }
}

#[test]
fn changed_missing_unsafe_and_nonready_assets_are_rejected_without_mutating_source() {
    for kind in ["changed", "missing", "escape", "wrong_meeting", "state"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = storage.create_meeting(kind).unwrap();
        let managed = asset(&storage, &meeting.id, b"audio");
        let path = storage
            .managed_path(Path::new(&managed.relative_path))
            .unwrap();
        match kind {
            "changed" => std::fs::write(path, b"other").unwrap(),
            "missing" => std::fs::remove_file(path).unwrap(),
            "escape" | "wrong_meeting" => {
                let relative = if kind == "escape" {
                    "../outside"
                } else {
                    "meetings/other/audio.flac"
                };
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE file_assets SET relative_path=?1 WHERE id=?2",
                        params![relative, managed.id],
                    )
                    .unwrap();
            }
            "state" => {
                storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE file_assets SET state='missing' WHERE id=?1",
                        params![managed.id],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            storage
                .prepare_backup_snapshot(&crate::test_support::never_cancelled)
                .is_err(),
            "{kind}"
        );
        assert_eq!(storage.list_meetings().unwrap(), vec![meeting]);
        assert!(storage.create_meeting("再開").is_ok());
    }
}

#[cfg(unix)]
#[test]
fn symlinked_parent_and_file_are_rejected_even_if_bytes_and_hash_match() {
    use std::os::unix::fs::symlink;
    for parent in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = storage.create_meeting("alias").unwrap();
        let managed = asset(&storage, &meeting.id, b"audio");
        let path = storage
            .managed_path(Path::new(&managed.relative_path))
            .unwrap();
        std::fs::write(outside.path().join("source.flac"), b"audio").unwrap();
        std::fs::remove_file(&path).unwrap();
        if parent {
            std::fs::remove_dir(path.parent().unwrap()).unwrap();
            symlink(outside.path(), path.parent().unwrap()).unwrap();
        } else {
            symlink(outside.path().join("source.flac"), &path).unwrap();
        }
        assert!(storage
            .prepare_backup_snapshot(&crate::test_support::never_cancelled)
            .is_err());
        assert!(storage.create_meeting("再開").is_ok());
    }
}

#[test]
fn migration_vocabulary_and_staged_bytes_corruption_are_detected() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("破損検知").unwrap();
    asset(&storage, &meeting.id, b"audio");
    let snapshot = storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    std::fs::write(&snapshot.assets[0].path, b"other").unwrap();
    assert!(snapshot
        .verify(&crate::test_support::never_cancelled)
        .is_err());
    drop(snapshot);
    storage
        .save_vocabulary_book(
            None,
            0,
            vec![VocabularyEntry {
                id: Uuid::new_v4().to_string(),
                term: "点検日".into(),
                reading: None,
                note: None,
                enabled: true,
            }],
        )
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE vocabulary_entries SET term='変更'", [])
        .unwrap();
    assert!(storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("用語"));
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE vocabulary_entries SET term='点検日'", [])
        .unwrap();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE schema_migrations SET checksum=?1 WHERE version=20",
            params!["0".repeat(64)],
        )
        .unwrap();
    assert!(storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("migration"));
    assert!(storage.create_meeting("再開").is_ok());
}

fn queued(storage: &Storage, meeting: &str) -> JobSnapshot {
    storage
        .enqueue_job(JobSpec {
            meeting_id: meeting.into(),
            kind: JobKind::Transcription,
            operation_id: Uuid::new_v4().to_string(),
            input_analysis_id: None,
            input_hash: "a".repeat(64),
            profile_snapshot: serde_json::json!({"modelName":"fixture","modelHash":"b".repeat(64)}),
            parameters: serde_json::json!({"schemaVersion":4}),
            total_units: Some(4),
        })
        .unwrap()
}

#[test]
fn queued_job_cannot_start_while_snapshot_exists_and_fixed_conditions_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("待機ジョブ").unwrap();
    let job = queued(&storage, &meeting.id);
    let original = storage.fixed_job_input(&job.id).unwrap();
    let snapshot = storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .unwrap();
    let db = read_only(&snapshot.database).unwrap();
    assert_eq!(
        super::super::jobs::read_fixed_input(&db, &job.id).unwrap(),
        original
    );
    assert!(storage.acquire_next_job().is_err());
    assert_eq!(storage.get_job(&job.id).unwrap().state, "queued");
    drop(db);
    drop(snapshot);
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, job.id);
}

#[test]
fn every_checkpoint_is_validated_not_only_the_most_recent_one() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = storage.create_meeting("過去のチェックポイント").unwrap();
    let job = queued(&storage, &meeting.id);
    storage.acquire_next_job().unwrap().unwrap();
    storage
        .save_job_checkpoint(&job.id, 0, 1, serde_json::json!({"unit":1}))
        .unwrap();
    storage.request_job_pause(&job.id, "user").unwrap();
    storage
        .save_job_checkpoint(&job.id, 1, 2, serde_json::json!({"unit":2}))
        .unwrap();
    assert_eq!(storage.get_job(&job.id).unwrap().state, "paused");
    assert!(storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_ok());
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE job_checkpoints SET checkpoint_json='{}' WHERE job_id=?1 AND sequence=1",
            params![job.id],
        )
        .unwrap();
    // Normal job resume checks the latest checkpoint; backup must inspect all.
    assert!(storage.fixed_job_input(&job.id).is_ok());
    assert!(storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .err()
        .unwrap()
        .contains("チェックポイント"));
    assert!(storage.create_meeting("再開").is_ok());
}

#[test]
fn capacity_overflow_and_grown_file_are_bounded_rejections() {
    let root = tempfile::tempdir().unwrap();
    assert!(check_capacity(root.path(), u64::MAX)
        .unwrap_err()
        .contains("範囲外"));
    let path = root.path().join("growing");
    std::fs::write(&path, vec![0; 2 * BUFFER_BYTES]).unwrap();
    let mut file = File::open(path).unwrap();
    assert!(stream(
        &mut file,
        None,
        BUFFER_BYTES as u64,
        &crate::test_support::never_cancelled
    )
    .unwrap_err()
    .contains("サイズ"));
}
