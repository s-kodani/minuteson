//! Parent-owned temporary audio files and SQLite only; no native user commands.
use super::super::super::tests::Fixture;
use super::super::{keep_unstarted, prepare, Policy};
use super::*;
use crate::analysis_worker::AnalysisController;
use std::cell::Cell;

fn owner() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
fn prepare_fixture(f: &Fixture, lease: &DataMaintenanceLease) -> String {
    let op = uuid::Uuid::new_v4().to_string();
    prepare(
        &f.storage,
        &op,
        &f.census(),
        Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        lease,
        &|| false,
    )
    .unwrap();
    op
}
fn reopen(f: &mut Fixture) {
    let root = f.storage.root().to_path_buf();
    let temp = tempfile::tempdir().unwrap();
    let previous = std::mem::replace(
        &mut f.storage,
        Storage::open(temp.path().to_owned()).unwrap(),
    );
    drop(previous);
    f.storage = Storage::open(root).unwrap();
}
fn resume(f: &Fixture, op: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(&f.storage, op, false, false, &|| false)
        .unwrap()
}

#[test]
fn flac_workspace_audio_removal_reopens_after_rename_unlink_and_advance_with_fixed_metadata() {
    for boundary in [
        "audioRenamed",
        "audioStaged",
        "audioUnlinked",
        "audioAdvanced",
    ] {
        let fixture = crate::storage::recordings::partial_test_fixture();
        let review = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        fixture.reject_partial_commit(true);
        assert!(fixture
            .storage
            .recover_partial_recording(&review, true)
            .is_err());
        fixture.reject_partial_commit(false);
        let original = std::fs::read(fixture.base.join("partial-flac.json")).unwrap();
        let census = crate::storage::deletion::audio::Census::capture(
            &fixture.storage,
            &fixture.session.meeting_id,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            census
                .preview(&uuid::Uuid::new_v4().to_string())
                .unwrap()
                .audio_files,
            10
        );
        let lease = owner();
        let operation = uuid::Uuid::new_v4().to_string();
        prepare(
            &fixture.storage,
            &operation,
            &census,
            Policy {
                include_unregistered_audio: true,
                retain_unclassified: true,
            },
            &lease,
            &|| false,
        )
        .unwrap();
        start(&fixture.storage, &operation, &lease, &|| false).unwrap();
        let cancel = Cell::new(false);
        assert!(run_inner(
            &fixture.storage,
            &operation,
            &lease,
            &|| cancel.get(),
            &|event| if event == boundary {
                cancel.set(true)
            }
        )
        .is_err());
        assert!(cancel.get());
        assert_eq!(
            std::fs::read(fixture.base.join("partial-flac.json")).unwrap(),
            original
        );
        drop(lease);
        let root = fixture.storage.root().to_owned();
        let base = fixture.base.clone();
        let meeting = fixture.session.meeting_id.clone();
        let temp = fixture.temp;
        drop(fixture.storage);
        let storage = Storage::open(root).unwrap();
        let lease = AnalysisController::default()
            .begin_audio_deletion_maintenance(&storage, &operation, false, false, &|| false)
            .unwrap();
        assert!(run(&storage, &operation, &lease, &|| false)
            .unwrap()
            .files_removed());
        assert!(completion::finish(&storage, &operation, &lease, &|| false)
            .unwrap()
            .completed());
        assert_eq!(
            std::fs::read(base.join("partial-flac.json")).unwrap(),
            original
        );
        assert!(storage.get_meeting(&meeting).unwrap().is_some());
        assert!(storage.restart_required());
        let restarted_root = storage.root().to_owned();
        drop(lease);
        drop(storage);
        let storage = Storage::open(restarted_root).unwrap();
        assert_eq!(
            crate::storage::deletion::audio::Census::capture(&storage, &meeting, &|| false)
                .unwrap()
                .preview(&uuid::Uuid::new_v4().to_string())
                .unwrap()
                .audio_files,
            0
        );
        drop(temp);
    }
}
#[test]
fn audio_removal_preserves_all_text_rows_manifests_directories_external_and_neighbor_files() {
    let f = Fixture::new();
    f.documents();
    let manifest = f.file(
        &format!("audio/{}/manifest.json", f.session),
        Some("application/json"),
    );
    let document = f.file("documents/result.json", Some("application/json"));
    let input_events = f.file(&format!("audio/{}/input-events.json", f.session), None);
    let input_events_before = std::fs::read(&input_events).unwrap();
    let input_notices = f.file(&format!("audio/{}/input-notices.json", f.session), None);
    let input_lock = f.file(&format!("audio/{}/input-notices.lock", f.session), None);
    let input_notices_before = std::fs::read(&input_notices).unwrap();
    let input_lock_before = std::fs::read(&input_lock).unwrap();
    let unknown = f.file("unknown.wav", None);
    let job = f.job("transcription");
    let unit = uuid::Uuid::new_v4().to_string();
    let pcm = f.file(&format!("analysis/{job}/unit-0-{unit}/input.wav"), None);
    let result = f.file(&format!("analysis/{job}/unit-0-{unit}/result.json"), None);
    let orphan = f.file("処理済み /別音声.flac", Some("audio/flac"));
    let leftover = f.file(&format!("audio/{}/system.caf", f.session), None);
    let external = f.base.path().join("external.m4a");
    std::fs::write(&external, b"EXTERNAL").unwrap();
    let neighbor = f.storage.create_meeting("NEIGHBOR").unwrap();
    let neighbor_file = f
        .storage
        .root()
        .join(format!("meetings/{}/untouched.caf", neighbor.id));
    std::fs::create_dir_all(neighbor_file.parent().unwrap()).unwrap();
    std::fs::write(&neighbor_file, b"NEIGHBOR").unwrap();
    let lease = owner();
    let op = prepare_fixture(&f, &lease);
    let saved = super::super::read(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
    let original = saved.snapshot.scope.clone();
    start(&f.storage, &op, &lease, &|| false).unwrap();
    let receipt = run(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(receipt.files_removed());
    assert_eq!(receipt.processed_files(), 4);
    for removed in [&f.path, &orphan, &leftover, &pcm] {
        assert!(!removed.exists());
        assert!(removed.parent().unwrap().is_dir());
    }
    for retained in [
        &manifest,
        &input_events,
        &input_notices,
        &input_lock,
        &document,
        &unknown,
        &result,
        &external,
        &neighbor_file,
    ] {
        assert!(retained.is_file());
    }
    assert_eq!(std::fs::read(&input_events).unwrap(), input_events_before);
    assert_eq!(std::fs::read(&input_notices).unwrap(), input_notices_before);
    assert_eq!(std::fs::read(&input_lock).unwrap(), input_lock_before);
    assert!(f.storage.data_write_blocked());
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(
        crate::storage::deletion::database::Scope::capture(&db, &f.meeting, &op, &|| false)
            .unwrap(),
        original
    );
    let (state,assets):(String,i64)=db.query_row("SELECT state,(SELECT count(*) FROM file_assets WHERE meeting_id=?2 AND state='ready') FROM maintenance_operations WHERE id=?1",params![op,f.meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, "cleanup");
    assert!(assets > 0);
    assert!(db
        .execute("UPDATE meetings SET title='REJECT'", [])
        .is_err());
    drop(db);
    assert!(f
        .storage
        .get_minutes_markdown(&f.meeting)
        .unwrap()
        .is_some());
    assert!(keep_unstarted(&f.storage, &op, &lease, &|| false).is_err());
    assert!(run(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .files_removed());
}
#[test]
fn audio_removal_start_requires_unchanged_consent_real_lease_and_is_never_reconsent() {
    for change in [false, true] {
        let f = Fixture::new();
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        if change {
            std::fs::write(&f.path, b"NEW").unwrap();
            assert!(start(&f.storage, &op, &lease, &|| false).is_err());
        } else {
            assert!(start(&f.storage, &op, &lease, &|| true).is_err());
            start(&f.storage, &op, &lease, &|| false).unwrap();
            assert!(start(&f.storage, &op, &lease, &|| false).is_err());
        }
        assert!(f.path.is_file());
        assert!(f.storage.data_write_blocked());
    }
    let f = Fixture::new();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let op = prepare_fixture(&f, &lease);
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(start(&f.storage, &op, &lease, &|| false).is_err());
}
#[test]
fn audio_removal_missing_registered_audio_is_not_recreated_or_double_counted() {
    let f = Fixture::new();
    std::fs::remove_file(&f.path).unwrap();
    let lease = owner();
    let op = prepare_fixture(&f, &lease);
    start(&f.storage, &op, &lease, &|| false).unwrap();
    let r = run(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(r.files_removed());
    assert_eq!(r.processed_files(), 1);
    assert!(!f.path.exists());
    assert!(f.storage.data_write_blocked());
}
#[test]
fn audio_removal_cancel_at_each_boundary_resumes_same_receipt_without_deleting_retained_files() {
    for boundary in [
        "audioArmed",
        "beforeAudioRename",
        "audioRenamed",
        "audioStaged",
        "beforeAudioUnlink",
        "audioUnlinked",
        "audioAdvanced",
        "beforeAudioFilesRemoved",
        "audioFilesRemoved",
    ] {
        let mut f = Fixture::new();
        let doc = f.file("retained.txt", Some("text/plain"));
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        let cancel = Cell::new(false);
        assert!(
            run_inner(
                &f.storage,
                &op,
                &lease,
                &|| cancel.get(),
                &|event| if event == boundary {
                    cancel.set(true)
                }
            )
            .is_err(),
            "{boundary}"
        );
        assert!(cancel.get());
        assert!(f.storage.data_write_blocked());
        assert!(doc.is_file());
        drop(lease);
        reopen(&mut f);
        let lease = resume(&f, &op);
        assert!(
            run(&f.storage, &op, &lease, &|| false)
                .unwrap()
                .files_removed(),
            "{boundary}"
        );
        assert!(!f.path.exists());
        assert!(doc.is_file());
        assert!(f.storage.data_write_blocked());
    }
}
#[test]
fn audio_removal_rejects_changes_to_retained_files_at_both_irreversible_boundaries() {
    for boundary in ["beforeAudioRename", "beforeAudioUnlink"] {
        let f = Fixture::new();
        let doc = f.file("retained.txt", Some("text/plain"));
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        assert!(run_inner(
            &f.storage,
            &op,
            &lease,
            &|| false,
            &|event| if event == boundary {
                std::fs::write(&doc, b"NEW RETAINED TEXT").unwrap()
            }
        )
        .is_err());
        assert_eq!(std::fs::read(&doc).unwrap(), b"NEW RETAINED TEXT");
        let r = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
        assert!(!r.files_removed());
        assert_eq!(r.processed_files(), 0);
        assert!(f.storage.data_write_blocked());
        let audio = if boundary == "beforeAudioRename" {
            f.path.clone()
        } else {
            f.path.parent().unwrap().join(r.reserved(0))
        };
        assert_eq!(std::fs::read(audio).unwrap(), b"PRIVATE AUDIO BODY");
    }
}
#[test]
fn audio_removal_rejects_new_unknown_file_and_missing_retained_file_after_partial_stop() {
    for add in [false, true] {
        let f = Fixture::new();
        let doc = f.file("retained.txt", Some("text/plain"));
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        let cancel = Cell::new(false);
        assert!(run_inner(
            &f.storage,
            &op,
            &lease,
            &|| cancel.get(),
            &|e| if e == "audioStaged" {
                cancel.set(true)
            }
        )
        .is_err());
        if add {
            f.file("new-unknown.flac", None);
        } else {
            std::fs::remove_file(&doc).unwrap();
        }
        assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        let r = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
        assert!(f.path.parent().unwrap().join(r.reserved(0)).is_file());
    }
}
#[test]
fn audio_removal_staged_change_original_reappearance_and_missing_unstaged_audio_are_refused() {
    for mode in ["staged", "original", "missing"] {
        let f = Fixture::new();
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        let cancel = Cell::new(false);
        let boundary = if mode == "missing" {
            "audioArmed"
        } else {
            "audioStaged"
        };
        assert!(run_inner(
            &f.storage,
            &op,
            &lease,
            &|| cancel.get(),
            &|e| if e == boundary {
                cancel.set(true)
            }
        )
        .is_err());
        let r = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
        let reserved = f.path.parent().unwrap().join(r.reserved(0));
        match mode {
            "staged" => std::fs::write(&reserved, b"REPLACED").unwrap(),
            "original" => std::fs::write(&f.path, b"NEW ORIGINAL").unwrap(),
            _ => std::fs::remove_file(&f.path).unwrap(),
        }
        assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(
            Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap(),
            r
        );
    }
}
#[test]
fn audio_removal_does_not_adopt_reappeared_missing_or_processed_audio_after_files_removed() {
    for initially_missing in [false, true] {
        let f = Fixture::new();
        if initially_missing {
            std::fs::remove_file(&f.path).unwrap();
        }
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        run(&f.storage, &op, &lease, &|| false).unwrap();
        std::fs::write(&f.path, b"REAPPEARED").unwrap();
        assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(&f.path).unwrap(), b"REAPPEARED");
    }
}
#[test]
fn audio_removal_corrupt_progress_cursor_subtype_and_held_record_never_grant_a_writer() {
    for mode in ["cursor", "phase", "unknown", "hash", "kept"] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        if mode == "kept" {
            keep_unstarted(&f.storage, &op, &lease, &|| false).unwrap();
            assert!(start(&f.storage, &op, &lease, &|| false).is_err());
            continue;
        }
        start(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        let root = f.storage.root().to_path_buf();
        let placeholder = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut f.storage,
            Storage::open(placeholder.path().to_owned()).unwrap(),
        );
        drop(old);
        let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
        db.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        let mut p: serde_json::Value = serde_json::from_str(
            &db.query_row(
                "SELECT progress_json FROM maintenance_operations WHERE id=?1",
                params![op],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        )
        .unwrap();
        match mode {
            "cursor" => p["next"] = 900000.into(),
            "phase" => p["phase"] = "audioFilesRemoved".into(),
            "hash" => p["censusHash"] = "b".repeat(64).into(),
            _ => p["deleteDocuments"] = true.into(),
        }
        // Exercise invalid semantic fields in canonical order, not merely a
        // reordered JSON failure. Unknown fields intentionally fail decoding.
        let encoded = if mode == "unknown" {
            p.to_string()
        } else {
            serde_json::to_string(&serde_json::from_value::<Progress>(p).unwrap()).unwrap()
        };
        db.execute(
            "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
            params![op, encoded],
        )
        .unwrap();
        drop(db);
        f.storage = Storage::open(root).unwrap();
        assert!(resume_attempt(&f, &op).is_err());
        assert!(run(&f.storage, &op, &owner(), &|| false).is_err());
        assert!(f.path.is_file() && f.storage.data_write_blocked());
    }
}
fn resume_attempt(f: &Fixture, op: &str) -> Result<DataMaintenanceLease, String> {
    AnalysisController::default().begin_audio_deletion_maintenance(
        &f.storage,
        op,
        false,
        false,
        &|| false,
    )
}

#[test]
fn audio_removal_reservation_collision_is_never_adopted_even_if_included_in_original_review() {
    for before_consent in [true, false] {
        let f = Fixture::new();
        let lease = owner();
        let op = uuid::Uuid::new_v4().to_string();
        let reserved = format!("audio/{}/.mon-delete-audio-{op}-0", f.session);
        let mut alien = None;
        if before_consent {
            alien = Some(f.file(&reserved, None));
        }
        prepare(
            &f.storage,
            &op,
            &f.census(),
            Policy {
                include_unregistered_audio: true,
                retain_unclassified: true,
            },
            &lease,
            &|| false,
        )
        .unwrap();
        if before_consent {
            assert!(start(&f.storage, &op, &lease, &|| false).is_err());
        } else {
            start(&f.storage, &op, &lease, &|| false).unwrap();
            alien = Some(f.file(&reserved, None));
            assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        }
        assert_eq!(
            std::fs::read(alien.unwrap()).unwrap(),
            b"PRIVATE FILE CONTENT"
        );
        assert!(f.path.is_file());
    }
}
#[test]
fn audio_removal_changed_database_text_and_independent_pending_receipt_stop_before_audio_changes() {
    for other_pending in [false, true] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        // Storage holds an exclusive SQLite lock. Mutate only this closed,
        // parent-owned fixture, then test typed cold admission and execution.
        let root = f.storage.root().to_path_buf();
        let placeholder = tempfile::tempdir().unwrap();
        let previous = std::mem::replace(
            &mut f.storage,
            Storage::open(placeholder.path().to_owned()).unwrap(),
        );
        drop(previous);
        let db = Connection::open(root.join("meetings.sqlite3")).unwrap();
        if other_pending {
            db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_meeting','prepared','{}',?3,1,'{}',1,1)",params![uuid::Uuid::new_v4().to_string(),f.meeting,"a".repeat(64)]).unwrap();
        } else {
            db.execute(
                "UPDATE meetings SET title='EXTERNAL EDIT' WHERE id=?1",
                params![f.meeting],
            )
            .unwrap();
        }
        drop(db);
        f.storage = Storage::open(root).unwrap();
        let lease = if other_pending {
            assert!(resume_attempt(&f, &op).is_err());
            owner()
        } else {
            resume(&f, &op)
        };
        assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(&f.path).unwrap(), b"PRIVATE AUDIO BODY");
        assert!(f.storage.data_write_blocked());
        let r = Receipt::load(&f.storage.connection.lock().unwrap(), &op, &|| false).unwrap();
        assert_eq!(r.processed_files(), 0);
        assert!(r.progress.pending.is_none());
    }
}
#[test]
fn audio_removal_new_hardlink_is_not_unlinked_or_followed() {
    let f = Fixture::new();
    let lease = owner();
    let op = prepare_fixture(&f, &lease);
    start(&f.storage, &op, &lease, &|| false).unwrap();
    let outside = f.base.path().join("shared-audio");
    std::fs::hard_link(&f.path, &outside).unwrap();
    assert!(run(&f.storage, &op, &lease, &|| false).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"PRIVATE AUDIO BODY");
    assert!(f.path.is_file());
}
#[test]
fn audio_removal_cancelled_shared_owner_cannot_run_even_if_callback_never_cancels() {
    let f = Fixture::new();
    let c = AnalysisController::default();
    let lease = c.begin_data_maintenance(false, false, &|| false).unwrap();
    let op = prepare_fixture(&f, &lease);
    start(&f.storage, &op, &lease, &|| false).unwrap();
    c.begin_exit().unwrap();
    c.abort_shutdown();
    assert!(run(&f.storage, &op, &lease, &|| false).is_err());
    assert!(f.path.is_file());
}
#[test]
fn audio_removal_leaf_links_parent_substitution_and_root_substitution_keep_external_data() {
    for mode in ["leaf", "parent", "root"] {
        let f = Fixture::new();
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        let external = f.base.path().join("external");
        std::fs::create_dir(&external).unwrap();
        let outside = external.join("microphone.caf");
        std::fs::write(&outside, b"OUTSIDE").unwrap();
        match mode {
            "leaf" => {
                std::fs::rename(&f.path, f.path.with_extension("held")).unwrap();
                std::os::unix::fs::symlink(&outside, &f.path).unwrap();
            }
            "parent" => {
                let parent = f.path.parent().unwrap();
                std::fs::rename(parent, parent.with_extension("held")).unwrap();
                std::os::unix::fs::symlink(&external, parent).unwrap();
            }
            _ => {
                std::fs::rename(f.storage.root(), f.base.path().join("held-root")).unwrap();
                std::fs::create_dir(f.storage.root()).unwrap();
            }
        }
        assert!(run(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"OUTSIDE");
    }
}
#[test]
fn audio_removal_same_parent_many_registered_files_has_exact_progress_and_keeps_unknowns() {
    let f = Fixture::new();
    for index in 0..32 {
        f.file(
            &format!("audio/{}/registered-{index}.flac", f.session),
            Some("audio/flac"),
        );
    }
    let unknown = f.file(&format!("audio/{}/unknown.flac", f.session), None);
    let lease = owner();
    let op = prepare_fixture(&f, &lease);
    start(&f.storage, &op, &lease, &|| false).unwrap();
    let r = run(&f.storage, &op, &lease, &|| false).unwrap();
    assert_eq!(r.processed_files(), 33);
    assert!(r.files_removed());
    assert!(unknown.is_file());
    assert!(f.path.parent().unwrap().is_dir());
}
#[test]
fn audio_removal_sigkill_preserves_exact_reserved_audio_and_resumes_every_persisted_boundary() {
    use std::os::unix::process::ExitStatusExt;
    for boundary in [
        "audioArmed",
        "audioRenamed",
        "audioStaged",
        "audioUnlinked",
        "audioAdvanced",
        "audioFilesRemoved",
    ] {
        let f = Fixture::new();
        let doc = f.file("keep.json", Some("application/json"));
        let lease = owner();
        let op = prepare_fixture(&f, &lease);
        start(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        let root = f.storage.root().to_path_buf();
        let meeting = f.meeting.clone();
        let path = f.path.clone();
        std::fs::write(
            f.base.path().join("audio-removal-owner.json"),
            serde_json::to_vec(&(root.to_str().unwrap(), meeting.as_str(), op.as_str())).unwrap(),
        )
        .unwrap();
        drop(f.storage);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::deletion::audio::intent::removal::tests::audio_removal_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_AUDIO_REMOVAL_ROOT", &root)
            .env("MON_AUDIO_REMOVAL_MEETING", &meeting)
            .env("MON_AUDIO_REMOVAL_OPERATION", &op)
            .env("MON_AUDIO_REMOVAL_BOUNDARY", boundary)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{boundary}");
        let storage = Storage::open(root).unwrap();
        assert!(storage.data_write_blocked());
        let c = AnalysisController::default();
        let lease = c
            .begin_audio_deletion_maintenance(&storage, &op, false, false, &|| false)
            .unwrap();
        let r = run(&storage, &op, &lease, &|| false).unwrap();
        assert!(r.files_removed());
        assert!(!path.exists() && doc.is_file());
        assert!(storage.data_write_blocked());
    }
}
#[test]
#[ignore = "subprocess-only parent-owned synthetic audio removal fixture"]
fn audio_removal_crash_child() {
    let root = std::path::PathBuf::from(std::env::var("MON_AUDIO_REMOVAL_ROOT").unwrap());
    let meeting = std::env::var("MON_AUDIO_REMOVAL_MEETING").unwrap();
    let op = std::env::var("MON_AUDIO_REMOVAL_OPERATION").unwrap();
    let boundary = std::env::var("MON_AUDIO_REMOVAL_BOUNDARY").unwrap();
    assert_eq!(root.file_name().unwrap(), "root");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(root.parent().unwrap().join("audio-removal-owner.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker,
        (root.to_str().unwrap().into(), meeting.clone(), op.clone())
    );
    assert!(super::super::super::uuid(&meeting) && super::super::super::uuid(&op));
    assert!([
        "audioArmed",
        "audioRenamed",
        "audioStaged",
        "audioUnlinked",
        "audioAdvanced",
        "audioFilesRemoved"
    ]
    .contains(&boundary.as_str()));
    let storage = Storage::open(root).unwrap();
    let c = AnalysisController::default();
    let lease = c
        .begin_audio_deletion_maintenance(&storage, &op, false, false, &|| false)
        .unwrap();
    run_inner(&storage, &op, &lease, &|| false, &|event| {
        if event == boundary {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            panic!("SIGKILL failed")
        }
    })
    .unwrap();
    panic!("crash boundary not reached");
}
