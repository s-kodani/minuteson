use super::*;
use crate::storage::{backup, current_minutes_fixture, drafts, with_test_database};

fn audio_path(storage: &Storage, meeting: &str) -> PathBuf {
    let plan = storage.canonical_audio_plan(meeting).unwrap();
    storage.meeting_mix_path(meeting, &plan.tracks[0]).unwrap()
}

fn diagnostics(storage: &Storage) -> Vec<serde_json::Value> {
    serde_json::to_value(storage.meeting_list_diagnostics().unwrap()).unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn insert_scope(db: &Connection, meeting: Option<&str>, target: &str, state: &str) -> String {
    let operation = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets',?3,?4,?5,'{}',0,0)", params![operation,meeting,state,target,"a".repeat(64)]).unwrap();
    operation
}

fn scope_database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    // Comparison fixtures deliberately include orphaned/corrupt claims.
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch(include_str!("../../../schema.sql"))
        .unwrap();
    db
}

#[test]
fn list_audio_scope_queries_do_not_multiply_meetings_by_completed_history() {
    use rusqlite::hooks::{AuthAction, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn manifest_reads(storage: &Storage) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        with_test_database(storage, |db| {
            db.authorizer(Some(move |context: rusqlite::hooks::AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "maintenance_operations",
                        column_name: "target_manifest_json"
                    }
                ) {
                    observed.fetch_add(1, Ordering::Relaxed);
                }
                Authorization::Allow
            }))
            .unwrap();
        });
        assert!(!diagnostics(storage).iter().any(|item| matches!(
            item["reason"].as_str(),
            Some("audio_missing" | "audio_unreadable")
        )));
        with_test_database(storage, |db| {
            db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
                .unwrap();
        });
        count.load(Ordering::Relaxed)
    }

    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    backup::archive::tests::meeting(&storage, "scope-query-first");
    let one = manifest_reads(&storage);
    assert!(one > 0, "the observer must see real deletion-scope SQL");
    for index in 0..3 {
        backup::archive::tests::meeting(&storage, &format!("scope-query-{index}"));
    }
    assert_eq!(manifest_reads(&storage), one);
    with_test_database(&storage, |db| {
        // The list must not reinterpret completed model/audio history per
        // current meeting. Startup receipt verification is a separate boundary.
        for _ in 0..128 {
            insert_scope(db, None, r#"{"subtype":"modelDeletion"}"#, "completed");
        }
    });
    assert_eq!(manifest_reads(&storage), one);
    with_test_database(&storage, |db| {
        insert_scope(
            db,
            None,
            r#"{"meetingId":"pending-other-meeting"}"#,
            "attention",
        );
    });
    assert_eq!(
        manifest_reads(&storage),
        one,
        "an unrelated pending deletion must not restore per-meeting history scans"
    );
}

#[test]
fn audio_scope_aggregation_preserves_sql_comparisons_and_damaged_claims() {
    let db = scope_database();
    let a = "scope-a";
    let b = "scope-b";
    insert_scope(
        &db,
        Some(a),
        &format!(r#"{{"meetingId":"{b}"}}"#),
        "attention",
    );
    let residual = insert_scope(&db, Some("residual"), "{}", "completed");
    db.execute(
        "UPDATE maintenance_operations SET progress_json=?1 WHERE id=?2",
        params![r#"{"version":4,"phase":"removing"}"#, residual],
    )
    .unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    let malformed = insert_scope(&db, Some("bad-json"), "invalid", "completed");
    db.execute(
        "UPDATE maintenance_operations SET progress_json='invalid' WHERE id=?1",
        [&malformed],
    )
    .unwrap();
    let damaged = insert_scope(&db, None, r#"{"meeting":"manifest-only"}"#, "attention");
    db.execute(
        "UPDATE maintenance_operations SET meeting_id=CAST(X'80' AS TEXT) WHERE id=?1",
        [&damaged],
    )
    .unwrap();
    let binary = insert_scope(&db, None, r#"{"meetingId":123}"#, "attention");
    db.execute(
        "UPDATE maintenance_operations SET meeting_id=X'73636f70652d61' WHERE id=?1",
        [&binary],
    )
    .unwrap();
    let mut scopes = AudioPendingScopes::default();
    for meeting in [
        a,
        b,
        "residual",
        "bad-json",
        "manifest-only",
        "123",
        "unprotected",
    ] {
        assert_eq!(
            scopes.contains(&db, meeting).unwrap(),
            crate::storage::deletion::fence::audio_pending(&db, meeting).unwrap(),
            "{meeting}"
        );
    }
    for meeting in [a, b, "residual", "bad-json", "manifest-only"] {
        assert!(scopes.contains(&db, meeting).unwrap(), "{meeting}");
    }
}

#[test]
fn audio_scope_snapshot_invalidates_on_local_writes_rollback_and_temp_ddl() {
    let db = scope_database();
    let operation = insert_scope(&db, Some("reserved"), "{}", "attention");
    let mut scopes = AudioPendingScopes::default();
    assert!(scopes.contains(&db, "reserved").unwrap());
    {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "UPDATE maintenance_operations SET state='completed' WHERE id=?1",
            [&operation],
        )
        .unwrap();
        assert!(!scopes.contains(&tx, "reserved").unwrap());
        tx.rollback().unwrap();
    }
    assert!(
        scopes.contains(&db, "reserved").unwrap(),
        "an uncommitted empty set must not survive rollback"
    );
    db.execute(
        "UPDATE maintenance_operations SET state='completed' WHERE id=?1",
        [&operation],
    )
    .unwrap();
    assert!(!scopes.contains(&db, "reserved").unwrap());
    db.execute_batch("CREATE TEMP TABLE mon_deletion_startup_attention(operation_id TEXT PRIMARY KEY NOT NULL,reason TEXT NOT NULL);").unwrap();
    db.execute(
        "INSERT INTO temp.mon_deletion_startup_attention VALUES(?1,'retained')",
        [&operation],
    )
    .unwrap();
    assert!(scopes.contains(&db, "reserved").unwrap());
    let changes = db.total_changes();
    db.execute_batch("DROP TABLE temp.mon_deletion_startup_attention;")
        .unwrap();
    assert_eq!(db.total_changes(), changes, "TEMP DDL is not a row change");
    assert!(!scopes.contains(&db, "reserved").unwrap());
    db.execute_batch("ALTER TABLE maintenance_operations RENAME TO changed_operations;")
        .unwrap();
    assert!(
        scopes.contains(&db, "reserved").is_err(),
        "main DDL must not reuse the previous empty set"
    );
}

#[test]
fn audio_scope_snapshot_is_bound_to_the_connection_and_outside_commits() {
    let first = scope_database();
    let second = scope_database();
    insert_scope(&first, Some("first"), "{}", "attention");
    insert_scope(&second, Some("second"), "{}", "attention");
    assert_eq!(first.total_changes(), second.total_changes());
    let mut scopes = AudioPendingScopes::default();
    assert!(!scopes.contains(&first, "second").unwrap());
    assert!(scopes.contains(&second, "second").unwrap());

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scopes.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../../../schema.sql"))
        .unwrap();
    let outside = Connection::open(&path).unwrap();
    outside.pragma_update(None, "foreign_keys", false).unwrap();
    let mut scopes = AudioPendingScopes::default();
    assert!(!scopes.contains(&db, "outside").unwrap());
    let changes = db.total_changes();
    insert_scope(&outside, Some("outside"), "{}", "attention");
    assert_eq!(db.total_changes(), changes);
    assert!(scopes.contains(&db, "outside").unwrap());
}

#[test]
fn audio_scope_cache_reads_under_the_readonly_fence_without_allowing_writes() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let meeting = backup::archive::tests::meeting(&storage, "readonly-audio-scope-cache");
    with_test_database(&storage, |db| {
        db.authorizer(Some(crate::storage::deletion::fence::readonly))
            .unwrap();
    });
    // Exercise every new PRAGMA through the real list, without an Allow hook.
    assert!(diagnostics(&storage).is_empty());
    with_test_database(&storage, |db| {
        for statement in [
            "PRAGMA main.data_version=1",
            "PRAGMA main.schema_version=1",
            "PRAGMA temp.schema_version=1",
        ] {
            assert!(db.execute_batch(statement).is_err(), "{statement}");
        }
        assert!(db
            .execute(
                "UPDATE meetings SET title='must remain readonly' WHERE id=?1",
                [&meeting]
            )
            .is_err());
        assert!(db
            .execute_batch("CREATE TEMP TABLE not_authorized(id TEXT)")
            .is_err());
    });
}

#[test]
fn missing_completed_audio_is_scoped_and_documents_stay_writable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let storage = Storage::open(root.clone()).unwrap();
    let good = backup::archive::tests::meeting(&storage, "available-neighbor");
    let missing = backup::archive::tests::meeting(&storage, "missing-completed-audio");
    let deleted = backup::deleted_current_meeting_for_test(&storage, "intentionally-deleted-audio");
    current_minutes_fixture::save(&storage, &missing, "# 保持する議事録\n");
    let missing_path = audio_path(&storage, &missing);
    drop(storage);
    std::fs::remove_file(missing_path).unwrap();

    let storage = Storage::open(root).unwrap();
    let items = diagnostics(&storage);
    assert!(items
        .iter()
        .any(|item| item["meetingId"] == missing && item["reason"] == "audio_missing"));
    assert!(!items
        .iter()
        .any(|item| item["meetingId"] == good || item["meetingId"] == deleted));
    assert!(storage.meeting_audio_summary(&good).is_ok());
    assert!(storage
        .meeting_audio_summary(&missing)
        .err()
        .unwrap()
        .contains("見つかりません"));
    assert!(storage
        .meeting_audio_summary(&deleted)
        .err()
        .unwrap()
        .contains("削除済み"));
    assert!(!storage.data_write_blocked());
    assert!(storage.load_current_analysis(&missing).unwrap().is_some());
    let mut draft = storage.load_transcript_draft(&missing).unwrap().unwrap();
    let utterance = &storage
        .load_current_analysis(&missing)
        .unwrap()
        .unwrap()
        .utterances[0];
    draft.payload.utterances.push(drafts::UtteranceEdit {
        utterance_id: utterance.id.clone(),
        effective_text: "音声欠損後も本文を保存します".into(),
        speaker_ids: utterance.speaker_ids.clone(),
    });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    current_minutes_fixture::save(&storage, &missing, "# 音声欠損後の議事録を保存\n");
    current_minutes_fixture::save(&storage, &deleted, "# 音声削除後も本文を保持\n");
    let current = storage.get_meeting(&good).unwrap().unwrap();
    storage
        .rename_meeting(&good, "正常な会議の編集", current.lock_version)
        .unwrap();
    storage.create_meeting("新しい会議").unwrap();
    with_test_database(&storage, |db| {
        let phase: String = db
            .query_row(
                "SELECT phase FROM meeting_audio_commits WHERE meeting_id=?1",
                [&missing],
                |row| row.get(0),
            )
            .unwrap();
        let state: String = db.query_row("SELECT f.state FROM file_assets f JOIN audio_chunks c ON c.file_asset_id=f.id JOIN meeting_audio_commits a ON a.track_id=c.track_id WHERE a.meeting_id=?1", [&missing], |row| row.get(0)).unwrap();
        assert_eq!(phase, "cleaned");
        assert_eq!(
            state, "ready",
            "availability observation must not reclassify or adopt missing audio"
        );
    });
}

#[test]
fn availability_distinguishes_missing_parent_and_shared_or_invalid_files() {
    for mutation in ["missing-parent", "hardlink", "directory", "truncated"] {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("data")).unwrap();
        let meeting = backup::archive::tests::meeting(&storage, mutation);
        let path = audio_path(&storage, &meeting);
        match mutation {
            "missing-parent" => {
                std::fs::rename(path.parent().unwrap(), temp.path().join("retained-source"))
                    .unwrap()
            }
            "hardlink" => std::fs::hard_link(&path, temp.path().join("retained-link.wav")).unwrap(),
            "directory" => {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            "truncated" => std::fs::write(&path, b"short WAV").unwrap(),
            _ => unreachable!(),
        }
        let expected = if mutation == "missing-parent" {
            "audio_missing"
        } else {
            "audio_unreadable"
        };
        assert!(
            diagnostics(&storage)
                .iter()
                .any(|item| item["meetingId"] == meeting && item["reason"] == expected),
            "{mutation}"
        );
        assert!(
            storage.meeting_audio_summary(&meeting).is_err(),
            "{mutation}"
        );
        assert!(!storage.data_write_blocked());
    }
}

#[cfg(windows)]
#[test]
fn an_exclusive_windows_file_lock_is_unreadable_and_clears_when_released() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let meeting = backup::archive::tests::meeting(&storage, "locked-canonical-wav");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(audio_path(&storage, &meeting))
        .unwrap();
    assert!(diagnostics(&storage)
        .iter()
        .any(|item| item["meetingId"] == meeting && item["reason"] == "audio_unreadable"));
    assert!(storage
        .meeting_audio_summary(&meeting)
        .err()
        .unwrap()
        .contains("読み取れません"));
    drop(held);
    assert!(!diagnostics(&storage)
        .iter()
        .any(|item| item["meetingId"] == meeting));
    assert!(storage.meeting_audio_summary(&meeting).is_ok());
}

#[test]
fn availability_leaves_full_payload_verification_to_playback() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let meeting = backup::archive::tests::meeting(&storage, "bounded-availability-check");
    let path = audio_path(&storage, &meeting);
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(path, bytes).unwrap();
    // Availability reads only file identity/size, independent of audio duration.
    assert!(storage.meeting_audio_summary(&meeting).is_ok());
    assert!(!diagnostics(&storage)
        .iter()
        .any(|item| item["meetingId"] == meeting));
    assert!(storage
        .prepare_meeting_audio(&crate::storage::MeetingPlaybackRequest {
            meeting_id: meeting,
            mode: crate::storage::MeetingPlaybackMode::Mixed,
            start_reference: None,
        })
        .is_err());
}

#[test]
fn malformed_audio_metadata_keeps_other_meeting_diagnostics_and_edits() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().join("data")).unwrap();
    let good = backup::archive::tests::meeting(&storage, "valid-metadata-neighbor");
    let malformed = backup::archive::tests::meeting(&storage, "malformed-audio-metadata");
    let missing = backup::archive::tests::meeting(&storage, "separate-missing-audio");
    let malformed_asset = storage.canonical_audio_plan(&malformed).unwrap().tracks[0].chunks[0]
        .asset_id
        .clone();
    with_test_database(&storage, |db| {
        db.execute(
            "UPDATE file_assets SET relative_path=CAST(relative_path AS BLOB) WHERE id=?1",
            [&malformed_asset],
        )
        .unwrap();
    });
    std::fs::remove_file(audio_path(&storage, &missing)).unwrap();
    let items = diagnostics(&storage);
    assert!(items
        .iter()
        .any(|item| item["meetingId"] == malformed && item["reason"] == "audio_unreadable"));
    assert!(items
        .iter()
        .any(|item| item["meetingId"] == missing && item["reason"] == "audio_missing"));
    assert!(!items.iter().any(|item| item["meetingId"] == good));
    assert!(!storage.data_write_blocked());
    let current = storage.get_meeting(&good).unwrap().unwrap();
    storage
        .rename_meeting(&good, "正常会議の編集を継続", current.lock_version)
        .unwrap();
    current_minutes_fixture::save(&storage, &malformed, "# 文字起こしと議事録は保持\n");
    storage.create_meeting("別の新しい会議").unwrap();
}

#[test]
fn an_audio_snapshot_is_not_current_after_a_new_reservation_or_availability_change() {
    for mutation in ["meeting-deletion", "audio-deletion", "track", "chunk"] {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("data")).unwrap();
        let meeting = backup::archive::tests::meeting(&storage, mutation);
        let source = with_test_database(&storage, |db| {
            Source::ready(db)
                .unwrap()
                .into_iter()
                .find(|candidate| candidate.meeting.as_deref() == Some(meeting.as_str()))
                .unwrap()
                .source
                .unwrap()
        });
        let mut pending = AudioPendingScopes::default();
        assert!(with_test_database(&storage, |db| source.current(db, &mut pending)).unwrap());
        if mutation == "meeting-deletion" {
            storage.reserve_meeting_deletion(&meeting).unwrap();
        } else {
            with_test_database(&storage, |db| match mutation {
                "audio-deletion" => {
                    let target =
                        serde_json::json!({"subtype":"audioOnlyDeletion","meetingId":meeting});
                    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES('new-audio-deletion',?1,'delete_assets','attention',?2,?3,'{}',0,0)", params![meeting,target.to_string(),"a".repeat(64)]).unwrap();
                }
                "track" => {
                    db.execute("UPDATE audio_tracks SET state='failed' WHERE id IN (SELECT track_id FROM meeting_audio_commits WHERE meeting_id=?1)", [&meeting]).unwrap();
                }
                "chunk" => {
                    db.execute("UPDATE audio_chunks SET state='missing' WHERE track_id IN (SELECT track_id FROM meeting_audio_commits WHERE meeting_id=?1)", [&meeting]).unwrap();
                }
                _ => unreachable!(),
            });
        }
        assert!(
            !with_test_database(&storage, |db| source.current(db, &mut pending)).unwrap(),
            "{mutation}"
        );
    }
}

#[cfg(windows)]
#[test]
fn an_availability_probe_releases_its_handles_before_a_deletion_can_reserve() {
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::open(temp.path().join("data")).unwrap());
    let meeting = backup::archive::tests::meeting(&storage, "availability-does-not-pin-deletion");
    let source = with_test_database(&storage, |db| {
        Source::ready(db).unwrap().pop().unwrap().source.unwrap()
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let inspected = storage.clone();
    let reader = std::thread::spawn(move || {
        source.inspect_observed(&inspected, &mut AudioPendingScopes::default(), || {
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        })
    });
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    // The probe's leaf handle is live here. Reservation uses this same lock,
    // rather than racing a Windows rename against open descendant handles.
    assert!(storage.connection.try_lock().is_err());
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (reserved_tx, reserved_rx) = mpsc::channel();
    let deleting = storage.clone();
    let target = meeting.clone();
    let deleter = std::thread::spawn(move || {
        attempt_tx.send(()).unwrap();
        reserved_tx
            .send(deleting.reserve_meeting_deletion(&target))
            .unwrap();
    });
    attempt_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        reserved_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    release_tx.send(()).unwrap();
    assert!(matches!(
        reader.join().unwrap().unwrap(),
        Observation::Available
    ));
    reserved_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    deleter.join().unwrap();
    assert!(
        with_test_database(&storage, |db| crate::storage::meeting_access::reserved(
            db, &meeting
        ))
        .unwrap()
    );
}
