//! Synthetic Storage, files and real scheduler leases only; no user audio/IPC.
use super::super::tests::Fixture;
use super::*;
use crate::analysis_worker::AnalysisController;
use std::cell::Cell;

fn lease() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
fn policy() -> Policy {
    Policy {
        include_unregistered_audio: false,
        retain_unclassified: false,
    }
}
fn count(f: &Fixture) -> i64 {
    f.storage
        .connection
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
            r.get(0)
        })
        .unwrap()
}
fn save(f: &Fixture, op: &str, census: &Census, owner: &DataMaintenanceLease) -> Prepared {
    prepare(&f.storage, op, census, policy(), owner, &|| false).unwrap()
}
fn reopen(f: &mut Fixture, mutate: impl FnOnce(&Connection)) {
    let root = f.storage.root().to_path_buf();
    let placeholder = tempfile::tempdir().unwrap();
    let old = std::mem::replace(
        &mut f.storage,
        Storage::open(placeholder.path().to_owned()).unwrap(),
    );
    drop(old);
    let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
    mutate(&db);
    drop(db);
    f.storage = Storage::open(root).unwrap();
}

#[test]
fn native_raw_audio_requires_distinct_unregistered_consent_and_blocks_changed_review() {
    let f = Fixture::new();
    let raw = f.file(
        &format!("audio/{}/raw-system-00000001.caf", f.session),
        None,
    );
    let census = f.census();
    let owner = lease();
    let op = uuid::Uuid::new_v4().to_string();
    assert!(prepare(&f.storage, &op, &census, policy(), &owner, &|| false).is_err());
    assert_eq!(count(&f), 0);
    assert!(raw.is_file());
    assert!(!f.storage.data_write_blocked());
    std::fs::write(&raw, b"changed unfinished raw PCM").unwrap();
    assert!(prepare(
        &f.storage,
        &op,
        &census,
        Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &owner,
        &|| false
    )
    .is_err());
    assert_eq!(count(&f), 0);
    assert!(raw.is_file() && f.path.is_file());
}

#[test]
fn audio_consent_is_atomic_typed_and_preserves_all_documents_audio_and_original_review_hash() {
    let f = Fixture::new();
    f.documents();
    let census = f.census();
    let snapshot = census.snapshot();
    let original_hash = census.hash().unwrap();
    assert_eq!(snapshot.hash().unwrap(), original_hash);
    let owner = lease();
    let op = uuid::Uuid::new_v4().to_string();
    let saved = save(&f, &op, &census, &owner);
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(read(&db, &op, &|| false).unwrap(), saved);
    let current = Census::capture_locked(&f.storage, &db, &f.meeting, &op, &|| false).unwrap();
    assert_eq!(current.snapshot(), snapshot);
    let (json,max_json,paths):(String,i64,i64)=db.query_row("SELECT o.target_manifest_json,max(length(CAST(p.identity_json AS BLOB))),count(*) FROM maintenance_operations o JOIN maintenance_paths p ON p.operation_id=o.id WHERE o.id=?1",params![op],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(json.contains("audioOnlyDeletion"));
    assert!(paths > 1 && max_json <= 1024);
    assert!(!String::from_utf8(snapshot.encode().unwrap())
        .unwrap()
        .contains("PRIVATE"));
    assert!(db
        .execute(
            "UPDATE maintenance_operations SET consent_at=0 WHERE id=?1",
            params![op]
        )
        .is_err());
    assert!(db
        .execute(
            "DELETE FROM maintenance_paths WHERE operation_id=?1",
            params![op]
        )
        .is_err());
    assert!(db
        .execute(
            "UPDATE meetings SET title='REJECT' WHERE id=?1",
            params![f.meeting]
        )
        .is_err());
    assert!(super::super::super::models::intent::read(&db, &op).is_err());
    drop(db);
    assert!(super::super::super::intent::read(&f.storage, &op, &|| false).is_err());
    saved.verify_current(&f.storage, &owner, &|| false).unwrap();
    assert!(f.storage.data_write_blocked());
    assert_eq!(std::fs::read(&f.path).unwrap(), b"PRIVATE AUDIO BODY");
    assert_eq!(
        f.storage
            .read_stored_minutes_state(&f.meeting)
            .unwrap()
            .current_id
            .is_some(),
        true
    );
}

#[test]
fn audio_consent_replay_preserves_consent_and_rejects_changed_policy_scope_or_other_operation() {
    let f = Fixture::new();
    let census = f.census();
    let owner = lease();
    let op = uuid::Uuid::new_v4().to_string();
    let saved = save(&f, &op, &census, &owner);
    assert_eq!(
        prepare(&f.storage, &op, &census, policy(), &owner, &|| false).unwrap(),
        saved
    );
    assert!(prepare(
        &f.storage,
        &op,
        &census,
        Policy {
            include_unregistered_audio: true,
            retain_unclassified: false
        },
        &owner,
        &|| false
    )
    .is_err());
    assert!(prepare(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &census,
        policy(),
        &owner,
        &|| false
    )
    .is_err());
    std::fs::write(&f.path, b"CHANGED AUDIO").unwrap();
    assert!(prepare(&f.storage, &op, &census, policy(), &owner, &|| false).is_err());
    assert_eq!(count(&f), 1);
    assert_eq!(
        read(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap(),
        saved
    );
}

#[test]
fn audio_consent_requires_distinct_unregistered_audio_and_unclassified_retention_choices() {
    let f = Fixture::new();
    let job = f.job("transcription");
    f.file(
        &format!("analysis/{job}/unit-0-{}/input.wav", uuid::Uuid::new_v4()),
        None,
    );
    let unknown = f.file("unknown-document.txt", None);
    let census = f.census();
    let owner = lease();
    let op = uuid::Uuid::new_v4().to_string();
    assert!(prepare(&f.storage, &op, &census, policy(), &owner, &|| false).is_err());
    assert!(prepare(
        &f.storage,
        &op,
        &census,
        Policy {
            include_unregistered_audio: true,
            retain_unclassified: false
        },
        &owner,
        &|| false
    )
    .is_err());
    assert!(prepare(
        &f.storage,
        &op,
        &census,
        Policy {
            include_unregistered_audio: false,
            retain_unclassified: true
        },
        &owner,
        &|| false
    )
    .is_err());
    assert_eq!(count(&f), 0);
    let selected = Policy {
        include_unregistered_audio: true,
        retain_unclassified: true,
    };
    let saved = prepare(&f.storage, &op, &census, selected.clone(), &owner, &|| {
        false
    })
    .unwrap();
    assert_eq!(saved.header.policy, selected);
    assert!(unknown.is_file());
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(unknown.is_file());
}

#[test]
fn audio_consent_rejects_stale_audio_and_retained_document_before_any_durable_write() {
    for audio_change in [true, false] {
        let f = Fixture::new();
        let retained = f.file("document.json", Some("application/json"));
        let census = f.census();
        std::fs::write(if audio_change { &f.path } else { &retained }, b"CHANGED").unwrap();
        assert!(prepare(
            &f.storage,
            &uuid::Uuid::new_v4().to_string(),
            &census,
            policy(),
            &lease(),
            &|| false
        )
        .is_err());
        assert_eq!(count(&f), 0);
        assert!(!f.storage.data_write_blocked());
    }
}

#[test]
fn audio_consent_cancellation_before_header_mid_proofs_and_before_commit_rolls_back_all_rows() {
    for boundary in [
        "audioIntentStored",
        "audioProofStored",
        "audioConsentValidated",
        "beforeAudioConsentCommit",
    ] {
        let f = Fixture::new();
        let census = f.census();
        let cancelled = Cell::new(false);
        assert!(
            prepare_inner(
                &f.storage,
                &uuid::Uuid::new_v4().to_string(),
                &census,
                policy(),
                &lease(),
                &|| cancelled.get(),
                &|event| if event == boundary {
                    cancelled.set(true)
                }
            )
            .is_err(),
            "{boundary}"
        );
        assert_eq!(count(&f), 0);
        assert_eq!(
            f.storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM maintenance_paths", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(!f.storage.data_write_blocked());
        assert!(f.path.is_file());
    }
}

#[test]
fn audio_consent_commit_wins_late_cancel_and_panic_keeps_durable_fence() {
    for panic in [false, true] {
        let f = Fixture::new();
        let census = f.census();
        let op = uuid::Uuid::new_v4().to_string();
        let cancelled = Cell::new(false);
        let owner = lease();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_inner(
                &f.storage,
                &op,
                &census,
                policy(),
                &owner,
                &|| cancelled.get(),
                &|event| {
                    if event == "audioConsentCommitted" {
                        cancelled.set(true);
                        if panic {
                            panic!("synthetic post-commit interruption");
                        }
                    }
                },
            )
        }));
        assert_eq!(result.is_err(), panic);
        if let Ok(saved) = result {
            assert!(saved.is_ok());
        }
        assert!(cancelled.get());
        assert_eq!(count(&f), 1);
        assert!(f.storage.data_write_blocked());
        read(
            &f.storage
                .connection
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            &op,
            &|| false,
        )
        .unwrap();
        assert!(f.path.is_file());
    }
}

#[test]
fn audio_consent_detects_scope_mutation_at_final_commit_boundary_without_adopting_it() {
    let f = Fixture::new();
    let census = f.census();
    let result = prepare_inner(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &census,
        policy(),
        &lease(),
        &|| false,
        &|event| {
            if event == "beforeAudioConsentCommit" {
                std::fs::write(&f.path, b"NEW UNCONFIRMED AUDIO").unwrap();
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(count(&f), 0);
    assert!(!f.storage.data_write_blocked());
    assert_eq!(std::fs::read(&f.path).unwrap(), b"NEW UNCONFIRMED AUDIO");
}

#[test]
fn audio_consent_keep_is_idempotent_preserves_changed_missing_and_new_files_without_reviving() {
    let f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    let saved = save(&f, &op, &census, &owner);
    std::fs::remove_file(&f.path).unwrap();
    let new = f.file("new-unregistered.wav", None);
    assert!(saved.verify_current(&f.storage, &owner, &|| false).is_err());
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
    let kept = read(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
    assert!(kept.kept());
    assert_eq!(kept.header, saved.header);
    assert_eq!(kept.consent, saved.consent);
    assert!(prepare(&f.storage, &op, &census, policy(), &owner, &|| false).is_err());
    assert!(!f.path.exists() && new.is_file());
    assert!(!f.storage.data_write_blocked());
    f.storage
        .create_meeting("HOT KEEP ALLOWS NORMAL WRITES")
        .unwrap();
}

#[test]
fn audio_consent_keep_cancel_and_panic_roll_back_progress_and_restore_writer_protection() {
    for panic in [false, true] {
        let f = Fixture::new();
        let census = f.census();
        let op = uuid::Uuid::new_v4().to_string();
        let owner = lease();
        let saved = save(&f, &op, &census, &owner);
        let cancelled = Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            keep_inner(&f.storage, &op, &owner, &|| cancelled.get(), &|event| {
                if event == "audioKeepStored" {
                    cancelled.set(true);
                    if panic {
                        panic!("synthetic keep interruption");
                    }
                }
            })
        }));
        assert_eq!(result.is_err(), panic);
        if let Ok(result) = result {
            assert!(result.is_err());
        }
        let db = f
            .storage
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(read(&db, &op, &|| false).unwrap(), saved);
        assert!(db.execute("DELETE FROM meetings", []).is_err());
        drop(db);
        assert!(f.storage.data_write_blocked() && f.path.is_file());
    }
}

#[test]
fn audio_consent_cold_keep_preserves_stale_jobs_and_requires_restart_before_ordinary_recovery() {
    let mut f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    save(&f, &op, &census, &lease());
    let meeting = f.meeting.clone();
    reopen(&mut f, |db| {
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('stale-audio',?1,'transcription','running','stale-audio',?2,'{}','{}',?2,1,1)",params![meeting,"a".repeat(64)]).unwrap();
    });
    let state = |f: &Fixture| {
        f.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM processing_jobs WHERE id='stale-audio'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
    };
    assert_eq!(state(&f), "running");
    assert!(f.storage.data_write_blocked());
    let controller = AnalysisController::default();
    controller.require_storage_restart();
    assert!(controller
        .begin_data_maintenance(false, false, &|| false)
        .is_err());
    let owner = controller
        .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .unwrap();
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(f
        .storage
        .require_data_writable()
        .unwrap_err()
        .contains("再起動"));
    assert_eq!(state(&f), "running");
    drop(owner);
    reopen(&mut f, |_| {});
    assert_eq!(state(&f), "interrupted");
    assert!(!f.storage.data_write_blocked());
}

#[test]
fn audio_consent_requires_active_real_lease_for_save_verify_keep_and_cold_admission() {
    let f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    let controller = AnalysisController::default();
    let owner = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let saved = save(&f, &op, &census, &owner);
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(saved.verify_current(&f.storage, &owner, &|| false).is_err());
    assert!(keep_unstarted(&f.storage, &op, &owner, &|| false).is_err());
    assert!(prepare(&f.storage, &op, &census, policy(), &owner, &|| false).is_err());
    drop(owner);
    for (recording, importing, cancel) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert!(controller
            .begin_audio_deletion_maintenance(&f.storage, &op, recording, importing, &|| cancel)
            .is_err());
    }
    let export = controller.begin_export().unwrap();
    assert!(controller
        .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .is_err());
    drop(export);
    let owner = controller
        .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .unwrap();
    assert!(controller
        .begin_audio_deletion_maintenance(&f.storage, &op, false, false, &|| false)
        .is_err());
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
}

#[test]
fn audio_consent_corrupt_subtype_proofs_progress_and_null_meeting_never_grant_keep() {
    for mode in [
        "chunk",
        "slot",
        "progress",
        "subtype",
        "meeting",
        "installing",
    ] {
        let mut f = Fixture::new();
        let census = f.census();
        let op = uuid::Uuid::new_v4().to_string();
        save(&f, &op, &census, &lease());
        reopen(&mut f, |db| {
            db.set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                false,
            )
            .unwrap();
            match mode {
                "chunk" => {
                    db.execute("UPDATE maintenance_paths SET identity_json='{}' WHERE operation_id=?1 AND relative_path='audio-deletion-proof/000000'",params![op]).unwrap();
                }
                "slot" => {
                    db.execute("UPDATE maintenance_paths SET relative_path='model-deletion-proof/000000' WHERE operation_id=?1 AND relative_path='audio-deletion-proof/000000'",params![op]).unwrap();
                }
                "progress" => {
                    db.execute(
                        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
                        params![op],
                    )
                    .unwrap();
                }
                "meeting" => {
                    db.execute(
                        "UPDATE maintenance_operations SET meeting_id=NULL WHERE id=?1",
                        params![op],
                    )
                    .unwrap();
                }
                "installing" => {
                    db.execute("UPDATE maintenance_operations SET state='installing',progress_json='{}' WHERE id=?1",params![op]).unwrap();
                }
                _ => {
                    db.execute(
                        "UPDATE maintenance_operations SET target_manifest_json='{}' WHERE id=?1",
                        params![op],
                    )
                    .unwrap();
                }
            }
        });
        assert!(f.storage.data_write_blocked(), "{mode}");
        assert!(authorize_review(&f.storage, &op).is_err(), "{mode}");
        assert!(
            keep_unstarted(&f.storage, &op, &lease(), &|| false).is_err(),
            "{mode}"
        );
        assert!(f.path.is_file());
    }
}

#[test]
fn audio_consent_does_not_clear_other_pending_intents_or_use_replaced_storage() {
    let mut f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    save(&f, &op, &census, &lease());
    let meeting = f.meeting.clone();
    reopen(&mut f, |db| {
        db.execute("INSERT INTO maintenance_operations VALUES(?1,?2,'delete_meeting','prepared',NULL,'{}',?3,1,'{}',NULL,1,1)",params![uuid::Uuid::new_v4().to_string(),meeting,"a".repeat(64)]).unwrap();
    });
    assert!(authorize_review(&f.storage, &op).is_err());
    assert!(keep_unstarted(&f.storage, &op, &lease(), &|| false).is_err());
    let f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    save(&f, &op, &census, &lease());
    std::fs::rename(f.storage.root(), f.base.path().join("held-root")).unwrap();
    std::fs::create_dir(f.storage.root()).unwrap();
    assert!(authorize_review(&f.storage, &op).is_err());
    assert!(keep_unstarted(&f.storage, &op, &lease(), &|| false).is_err());
    assert!(f
        .base
        .path()
        .join("held-root")
        .join("meetings")
        .join(&f.meeting)
        .is_dir());
}

#[test]
fn audio_consent_typed_snapshot_rejects_forged_paths_parent_proofs_track_kind_and_unknown_fields() {
    let f = Fixture::new();
    let snapshot = f.census().snapshot();
    let original = serde_json::to_value(snapshot).unwrap();
    let db = f.storage.connection.lock().unwrap();
    for case in ["path", "parent", "kind", "unknown", "order"] {
        let mut value = original.clone();
        match case {
            "path" => value["manifest"]["entries"][0]["path"] = "../external".into(),
            "parent" => {
                value["manifest"]["entries"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            }
            "kind" => value["records"]["tracks"][0]["kind"] = "text".into(),
            "order" => value["manifest"]["entries"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1),
            _ => value["deleteAllFiles"] = true.into(),
        }
        let decoded = serde_json::from_value::<Snapshot>(value);
        assert!(
            decoded.is_err() || decoded.unwrap().validate(&db).is_err(),
            "{case}"
        );
    }
}

#[test]
fn audio_consent_preserves_unicode_paths_missing_audio_and_non_audio_registration() {
    let f = Fixture::new();
    let relative = format!("audio/{}/{} /区間.flac", f.session, "音声".repeat(12));
    let path = f.file(&relative, Some("audio/flac"));
    std::fs::remove_file(&path).unwrap();
    let manifest = f.file(
        &format!("audio/{}/manifest.json", f.session),
        Some("application/json"),
    );
    let census = f.census();
    let owner = lease();
    let op = uuid::Uuid::new_v4().to_string();
    let saved = save(&f, &op, &census, &owner);
    assert_eq!(saved.snapshot, census.snapshot());
    saved.verify_current(&f.storage, &owner, &|| false).unwrap();
    keep_unstarted(&f.storage, &op, &owner, &|| false).unwrap();
    assert!(!path.exists() && manifest.is_file() && f.path.is_file());
}

#[test]
fn audio_consent_keep_commit_wins_late_cancel_without_claiming_audio_was_restored() {
    let f = Fixture::new();
    let census = f.census();
    let op = uuid::Uuid::new_v4().to_string();
    let owner = lease();
    save(&f, &op, &census, &owner);
    let cancelled = Cell::new(false);
    keep_inner(&f.storage, &op, &owner, &|| cancelled.get(), &|event| {
        if event == "audioKeepCommitted" {
            cancelled.set(true)
        }
    })
    .unwrap();
    assert!(cancelled.get());
    assert!(!f.storage.data_write_blocked());
    assert!(read(&f.storage.connection.lock().unwrap(), &op, &|| false)
        .unwrap()
        .kept());
    assert!(f.path.is_file());
}

#[test]
fn audio_consent_rejects_disabled_reference_guards_and_unknown_schema_without_changes() {
    for mode in ["foreign_keys", "triggers", "schema"] {
        let f = Fixture::new();
        let census = f.census();
        let db = f.storage.connection.lock().unwrap();
        match mode {
            "foreign_keys" => db.execute_batch("PRAGMA foreign_keys=OFF").unwrap(),
            "triggers" => {
                db.set_db_config(
                    rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                    false,
                )
                .unwrap();
            }
            _ => db
                .execute_batch("CREATE TABLE untrusted_extra(value TEXT)")
                .unwrap(),
        }
        drop(db);
        assert!(
            prepare(
                &f.storage,
                &uuid::Uuid::new_v4().to_string(),
                &census,
                policy(),
                &lease(),
                &|| false
            )
            .is_err(),
            "{mode}"
        );
        assert_eq!(count(&f), 0);
        assert!(f.path.is_file());
    }
}

#[test]
fn audio_consent_and_keep_sigkill_boundaries_preserve_atomic_receipts_and_original_audio() {
    use std::os::unix::process::ExitStatusExt;
    for (mode, boundary) in [
        ("consent", "audioIntentStored"),
        ("consent", "audioProofStored"),
        ("consent", "audioConsentValidated"),
        ("consent", "beforeAudioConsentCommit"),
        ("consent", "audioConsentCommitted"),
        ("keep", "audioKeepStored"),
        ("keep", "audioKeepCommitted"),
    ] {
        let f = Fixture::new();
        let census = f.census();
        let op = uuid::Uuid::new_v4().to_string();
        if mode == "keep" {
            save(&f, &op, &census, &lease());
        }
        let root = f.storage.root().to_path_buf();
        let audio_path = f.path.clone();
        let meeting = f.meeting.clone();
        // The exact temporary root and IDs are parent-owned fixture handoff.
        std::fs::write(
            f.base.path().join("audio-consent-owner.json"),
            serde_json::to_vec(&(root.to_str().unwrap(), meeting.as_str(), op.as_str())).unwrap(),
        )
        .unwrap();
        drop(f.storage);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::audio::intent::tests::audio_consent_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_AUDIO_CONSENT_TEST_ROOT", &root)
            .env("MON_AUDIO_CONSENT_TEST_MEETING", &meeting)
            .env("MON_AUDIO_CONSENT_TEST_OPERATION", &op)
            .env("MON_AUDIO_CONSENT_TEST_MODE", mode)
            .env("MON_AUDIO_CONSENT_TEST_BOUNDARY", boundary)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{mode}/{boundary}");
        let storage = Storage::open(root).unwrap();
        let db = storage.connection.lock().unwrap();
        let committed = mode == "keep" || boundary == "audioConsentCommitted";
        assert_eq!(
            db.query_row("SELECT count(*) FROM maintenance_operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            i64::from(committed),
            "{mode}/{boundary}"
        );
        if committed {
            let saved = read(&db, &op, &|| false).unwrap();
            assert_eq!(saved.kept(), boundary == "audioKeepCommitted");
            assert_eq!(storage.data_write_blocked(), !saved.kept());
        } else {
            assert_eq!(
                db.query_row("SELECT count(*) FROM maintenance_paths", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert!(!storage.data_write_blocked());
        }
        drop(db);
        assert_eq!(std::fs::read(audio_path).unwrap(), b"PRIVATE AUDIO BODY");
    }
}

#[test]
#[ignore = "subprocess-only synthetic audio consent fixture"]
fn audio_consent_crash_child() {
    let root = std::path::PathBuf::from(
        std::env::var("MON_AUDIO_CONSENT_TEST_ROOT").expect("parent fixture root"),
    );
    let meeting = std::env::var("MON_AUDIO_CONSENT_TEST_MEETING").unwrap();
    let operation = std::env::var("MON_AUDIO_CONSENT_TEST_OPERATION").unwrap();
    let mode = std::env::var("MON_AUDIO_CONSENT_TEST_MODE").unwrap();
    let boundary = std::env::var("MON_AUDIO_CONSENT_TEST_BOUNDARY").unwrap();
    assert_eq!(root.file_name().unwrap(), "root");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(root.parent().unwrap().join("audio-consent-owner.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker,
        (
            root.to_str().unwrap().into(),
            meeting.clone(),
            operation.clone()
        )
    );
    assert!(super::super::uuid(&meeting) && super::super::uuid(&operation));
    let storage = Storage::open(root).unwrap();
    let controller = AnalysisController::default();
    let event = |phase: &str| {
        if phase == boundary {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            panic!("SIGKILL failed");
        }
    };
    if mode == "consent" {
        assert!([
            "audioIntentStored",
            "audioProofStored",
            "audioConsentValidated",
            "beforeAudioConsentCommit",
            "audioConsentCommitted"
        ]
        .contains(&boundary.as_str()));
        let census = Census::capture(&storage, &meeting, &|| false).unwrap();
        let owner = controller
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        prepare_inner(
            &storage,
            &operation,
            &census,
            policy(),
            &owner,
            &|| false,
            &event,
        )
        .unwrap();
    } else {
        assert_eq!(mode, "keep");
        assert!(["audioKeepStored", "audioKeepCommitted"].contains(&boundary.as_str()));
        let owner = controller
            .begin_audio_deletion_maintenance(&storage, &operation, false, false, &|| false)
            .unwrap();
        keep_inner(&storage, &operation, &owner, &|| false, &event).unwrap();
    }
    panic!("parent-requested crash boundary was not reached");
}
