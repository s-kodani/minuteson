//! Synthetic recordings and actual durable journals; never user audio.
use super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::deletion::audio::{intent, tests::Fixture},
};
use std::cell::Cell;
use std::os::unix::process::ExitStatusExt;

fn owner() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
fn policy() -> Policy {
    Policy {
        include_unregistered_audio: true,
        retain_unclassified: true,
    }
}
fn complete(f: &Fixture) -> String {
    let lease = owner();
    let op = uuid::Uuid::new_v4().to_string();
    intent::prepare(&f.storage, &op, &f.census(), policy(), &lease, &|| false).unwrap();
    intent::removal::start(&f.storage, &op, &lease, &|| false).unwrap();
    intent::removal::run(&f.storage, &op, &lease, &|| false).unwrap();
    completion::finish(&f.storage, &op, &lease, &|| false).unwrap();
    op
}
fn fixed(f: &Fixture, op: &str) -> String {
    f.storage.connection.lock().unwrap().query_row("SELECT json_array(rowid,meeting_id,kind,state,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at,(SELECT json_group_array(json_array(relative_path,kind,size_bytes,content_hash,identity_json,identity_hash)) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1",params![op],|r|r.get(0)).unwrap()
}
fn reopen(f: &mut Fixture) {
    let root = f.storage.root().to_owned();
    let temporary = tempfile::tempdir().unwrap();
    let old = std::mem::replace(
        &mut f.storage,
        Storage::open(temporary.path().to_owned()).unwrap(),
    );
    drop(old);
    f.storage = Storage::open(root).unwrap();
}
fn resume(f: &Fixture, op: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(&f.storage, op, false, false, &|| false)
        .unwrap()
}
fn execute(f: &Fixture, op: &str, lease: &DataMaintenanceLease) {
    intent::removal::start(&f.storage, op, lease, &|| false).unwrap();
    intent::removal::run(&f.storage, op, lease, &|| false).unwrap();
    assert!(completion::finish(&f.storage, op, lease, &|| false)
        .unwrap()
        .completed());
}
fn attention(f: &Fixture, op: &str) -> bool {
    crate::storage::deletion::startup::contains(&f.storage.connection.lock().unwrap(), op).unwrap()
}

#[test]
fn audio_reconfirmation_keeps_original_availability_documents_and_postcompletion_edits() {
    let f = Fixture::new();
    f.documents();
    let text = f.file("documents/minutes.md", Some("text/markdown"));
    let original = complete(&f);
    let before = fixed(&f, &original);
    std::fs::write(&f.path, b"REAPPEARED AUDIO").unwrap();
    std::fs::write(&text, b"LATER EDITED DOCUMENT").unwrap();
    let unknown = f.file("unknown.bin", None);
    let census = f.census();
    let lease = resume(&f, &original);
    assert!(intent::prepare(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &census,
        policy(),
        &lease,
        &|| false
    )
    .is_err());
    assert!(intent::removal::run(&f.storage, &original, &lease, &|| false).is_err());
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let preview = review.preview(&uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(!preview.execute_allowed);
    assert_eq!(preview.changed_audio_files, 1);
    assert!(preview.retained.minutes_revisions > 0 && preview.unclassified_files > 0);
    let op = uuid::Uuid::new_v4().to_string();
    let saved = accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
    assert_eq!(saved.header.version, 2);
    assert_eq!(saved.header.origin.as_ref().unwrap().operation_id, original);
    assert_eq!(
        saved.snapshot.manifest.root,
        f.storage.managed_anchor.identity().unwrap().0
    );
    assert!(f.storage.data_write_blocked());
    execute(&f, &op, &lease);
    assert_eq!(fixed(&f, &original), before);
    assert_eq!(std::fs::read(&text).unwrap(), b"LATER EDITED DOCUMENT");
    assert!(unknown.exists() && !f.path.exists());
    assert!(crate::storage::deletion::audio::require_source(
        &f.storage.connection.lock().unwrap(),
        &f.meeting
    )
    .is_err());
    assert!(!f.storage.data_write_blocked());
    assert!(f
        .storage
        .read_stored_minutes_state(&f.meeting)
        .unwrap()
        .current_id
        .is_some());
}

#[test]
fn audio_reconfirmation_cold_resolves_attention_and_preserves_restart_barrier() {
    let mut f = Fixture::new();
    let original = complete(&f);
    let before = fixed(&f, &original);
    std::fs::write(&f.path, b"RETURNED").unwrap();
    reopen(&mut f);
    assert!(attention(&f, &original) && f.storage.data_write_blocked());
    let lease = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
    drop(lease);
    reopen(&mut f);
    let lease = resume(&f, &op);
    execute(&f, &op, &lease);
    assert!(!attention(&f, &original));
    assert!(f.storage.data_write_blocked() && f.storage.deletion_resume_restart_required());
    assert_eq!(fixed(&f, &original), before);
    assert!(f.storage.create_meeting("RESTART FIRST").is_err());
    drop(lease);
    reopen(&mut f);
    assert!(!f.storage.data_write_blocked());
    assert!(f.storage.create_meeting("NORMAL").is_ok());
}

#[test]
fn audio_reconfirmation_exact_historical_reservations_need_explicit_unregistered_choice() {
    let f = Fixture::new();
    let original = complete(&f);
    let source = Source::load(&f.storage.connection.lock().unwrap(), &original, &|| false).unwrap();
    let reserved = source
        .paths
        .iter()
        .find(|p| p.contains(".mon-delete-audio-"))
        .unwrap();
    let path = f.storage.root().join(reserved);
    std::fs::write(&path, b"RETURNED RESERVED BODY").unwrap();
    let neighbor = path.with_file_name(format!(".mon-delete-audio-{original}-999"));
    std::fs::write(&neighbor, b"KEEP NEAR NAME").unwrap();
    let lease = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let p = review.preview(&uuid::Uuid::new_v4().to_string()).unwrap();
    assert_eq!(p.unregistered_audio_files, 1);
    assert!(p
        .entries
        .iter()
        .any(|e| e.relative_path == *reserved && e.disposition == Disposition::HistoricalAudio));
    assert!(accept(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &review,
        Policy {
            include_unregistered_audio: false,
            retain_unclassified: true
        },
        &lease,
        &|| false
    )
    .is_err());
    let op = uuid::Uuid::new_v4().to_string();
    accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
    execute(&f, &op, &lease);
    assert!(!path.exists() && neighbor.exists());
}

#[test]
fn audio_reconfirmation_each_precommit_cancel_rolls_back_new_operation_and_original() {
    for boundary in [
        "audioReconfirmationStored",
        "audioReconfirmationProofStored",
        "audioReconfirmationValidated",
        "beforeAudioReconfirmationCommit",
    ] {
        let mut f = Fixture::new();
        let original = complete(&f);
        std::fs::write(&f.path, b"RETURNED").unwrap();
        reopen(&mut f);
        let before = fixed(&f, &original);
        let lease = resume(&f, &original);
        let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        let stop = Cell::new(false);
        assert!(
            accept_inner(
                &f.storage,
                &op,
                &review,
                policy(),
                &lease,
                &|| stop.get(),
                &|e| if e == boundary {
                    stop.set(true);
                }
            )
            .is_err(),
            "{boundary}"
        );
        let count: i64 = f
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM maintenance_operations WHERE id=?1",
                params![op],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(fixed(&f, &original), before);
        assert!(f.path.exists() && attention(&f, &original) && f.storage.data_write_blocked());
        assert!(f
            .storage
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE meetings SET title='DENIED'", [])
            .is_err());
    }
}

#[test]
fn audio_reconfirmation_saved_policy_replays_and_conflicting_operation_id_is_rejected() {
    let f = Fixture::new();
    let original = complete(&f);
    std::fs::write(&f.path, b"RETURNED").unwrap();
    let lease = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let saved = accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
    assert_eq!(
        accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap(),
        saved
    );
    assert!(accept(
        &f.storage,
        &op,
        &review,
        Policy {
            include_unregistered_audio: false,
            retain_unclassified: true
        },
        &lease,
        &|| false
    )
    .is_err());
    assert!(accept(&f.storage, &original, &review, policy(), &lease, &|| false).is_err());
    execute(&f, &op, &lease);
}

#[test]
fn audio_reconfirmation_current_inventory_changes_cancel_or_dead_lease_never_publish_consent() {
    for variant in 0..3 {
        let f = Fixture::new();
        let original = complete(&f);
        std::fs::write(&f.path, b"RETURNED").unwrap();
        let controller = AnalysisController::default();
        let lease = controller
            .begin_audio_deletion_maintenance(&f.storage, &original, false, false, &|| false)
            .unwrap();
        let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
        if variant == 0 {
            std::fs::write(&f.path, b"AFTER REVIEW CHANGE").unwrap();
        }
        if variant == 2 {
            controller.begin_exit().unwrap();
            controller.abort_shutdown();
        }
        assert!(accept(
            &f.storage,
            &uuid::Uuid::new_v4().to_string(),
            &review,
            policy(),
            &lease,
            &|| variant == 1
        )
        .is_err());
        assert!(f.path.exists());
        let count: i64 = f
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}

#[test]
fn audio_reconfirmation_keep_unstarted_leaves_old_attention_and_can_review_again() {
    let mut f = Fixture::new();
    let original = complete(&f);
    let before = fixed(&f, &original);
    std::fs::write(&f.path, b"RETURNED").unwrap();
    reopen(&mut f);
    let lease = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
    intent::keep_unstarted(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(f.path.exists() && attention(&f, &original));
    assert!(intent::removal::start(&f.storage, &op, &lease, &|| false).is_err());
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    let again = uuid::Uuid::new_v4().to_string();
    accept(&f.storage, &again, &review, policy(), &lease, &|| false).unwrap();
    execute(&f, &again, &lease);
    assert_eq!(fixed(&f, &original), before);
}

#[test]
fn audio_reconfirmation_multiple_generations_derive_prior_reserved_names_without_recursive_loading()
{
    let f = Fixture::new();
    let original = complete(&f);
    let before = fixed(&f, &original);
    let mut previous = original.clone();
    for index in 0..3 {
        let source =
            Source::load(&f.storage.connection.lock().unwrap(), &previous, &|| false).unwrap();
        let path = if index == 0 {
            f.path.clone()
        } else {
            let paths = source.receipts[0].source_paths();
            f.storage.root().join(
                paths
                    .iter()
                    .find(|p| p.contains(&format!(".mon-delete-audio-{previous}-")))
                    .unwrap(),
            )
        };
        std::fs::write(&path, b"ANOTHER GENERATION").unwrap();
        let lease = resume(&f, &previous);
        let review = Review::capture(&f.storage, &previous, &lease, &|| false).unwrap();
        let op = uuid::Uuid::new_v4().to_string();
        accept(&f.storage, &op, &review, policy(), &lease, &|| false).unwrap();
        execute(&f, &op, &lease);
        assert!(!path.exists());
        previous = op;
    }
    assert_eq!(
        Source::load(&f.storage.connection.lock().unwrap(), &previous, &|| false)
            .unwrap()
            .receipts
            .len(),
        4
    );
    assert_eq!(fixed(&f, &original), before);
}

#[test]
fn audio_reconfirmation_changed_original_seal_and_unowned_database_copy_do_not_authorize_review() {
    let f = Fixture::new();
    let original = complete(&f);
    std::fs::write(&f.path, b"RETURNED").unwrap();
    let lease = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &lease, &|| false).unwrap();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
            params![original],
        )
        .unwrap();
    assert!(accept(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &review,
        policy(),
        &lease,
        &|| false
    )
    .is_err());
    assert!(f.path.exists());
    let copy = tempfile::tempdir().unwrap();
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "VACUUM INTO ?1",
        params![copy.path().join("meetings.sqlite3").to_str().unwrap()],
    )
    .unwrap();
    drop(db);
    let copied = Storage::open(copy.path().to_owned()).unwrap();
    assert!(AnalysisController::default()
        .begin_audio_deletion_maintenance(&copied, &original, false, false, &|| false)
        .is_err());
}

#[test]
fn audio_reconfirmation_cold_preserves_stale_jobs_until_normal_recovery_after_reopen() {
    let mut f = Fixture::new();
    let original = complete(&f);
    let job = f.job("transcription");
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='running' WHERE id=?1",
            params![job],
        )
        .unwrap();
    std::fs::write(&f.path, b"RETURNED").unwrap();
    let owner = resume(&f, &original);
    assert!(Review::capture(&f.storage, &original, &owner, &|| false).is_err());
    drop(owner);
    reopen(&mut f);
    let owner = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &owner, &|| false).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    accept(&f.storage, &operation, &review, policy(), &owner, &|| false).unwrap();
    execute(&f, &operation, &owner);
    assert_eq!(f.storage.list_jobs(&f.meeting).unwrap()[0].state, "running");
    assert!(f.storage.data_write_blocked());
    drop(owner);
    reopen(&mut f);
    assert_eq!(
        f.storage.list_jobs(&f.meeting).unwrap()[0].state,
        "interrupted"
    );
    assert!(!f.storage.data_write_blocked());
}

#[test]
fn audio_reconfirmation_rejects_clock_rollback_before_publishing_new_consent() {
    let f = Fixture::new();
    let original = complete(&f);
    let future = crate::storage::now_ms().unwrap() + 86_400_000;
    {
        // Own synthetic journal only: emulate a prior valid future-clock
        // consent, then restore the real triggers before any product call.
        let db = f.storage.connection.lock().unwrap();
        use rusqlite::config::DbConfig;
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute("UPDATE maintenance_operations SET consent_at=?2,created_at=?2,updated_at=?2 WHERE id=?1",params![original,future]).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .unwrap();
    }
    std::fs::write(&f.path, b"RETURNED").unwrap();
    let owner = resume(&f, &original);
    let review = Review::capture(&f.storage, &original, &owner, &|| false).unwrap();
    assert!(accept(
        &f.storage,
        &uuid::Uuid::new_v4().to_string(),
        &review,
        policy(),
        &owner,
        &|| false
    )
    .is_err());
    let count: i64 = f
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(f.path.exists());
}

#[test]
fn audio_reconfirmation_real_sigkill_after_consent_commit_cold_resumes_fixed_plan() {
    let mut f = Fixture::new();
    let original = complete(&f);
    let before = fixed(&f, &original);
    std::fs::write(&f.path, b"RETURNED").unwrap();
    let root = f.storage.root().to_owned();
    let operation = uuid::Uuid::new_v4().to_string();
    let marker = (
        root.to_str().unwrap().to_string(),
        original.clone(),
        operation.clone(),
    );
    std::fs::write(
        f.base.path().join("audio-reconfirmation-owner.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    drop(std::mem::replace(
        &mut f.storage,
        Storage::open(temporary.path().to_owned()).unwrap(),
    ));
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "storage::deletion::audio::intent::reconfirmation::tests::audio_reconfirmation_crash_child", "--ignored", "--nocapture"])
        .env("MON_AUDIO_RECONFIRMATION_ROOT", &root).env("MON_AUDIO_RECONFIRMATION_SOURCE", &original).env("MON_AUDIO_RECONFIRMATION_OPERATION", &operation)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    f.storage = Storage::open(root).unwrap();
    assert!(f.path.exists() && f.storage.data_write_blocked());
    assert_eq!(fixed(&f, &original), before);
    let owner = resume(&f, &operation);
    let saved = read(&f.storage.connection.lock().unwrap(), &operation, &|| false).unwrap();
    assert_eq!(saved.header.policy, policy());
    execute(&f, &operation, &owner);
    assert!(!f.path.exists());
    assert_eq!(fixed(&f, &original), before);
    drop(owner);
    reopen(&mut f);
    assert!(!f.storage.data_write_blocked());
}

#[test]
#[ignore = "subprocess-only parent-owned synthetic audio reconfirmation fixture"]
fn audio_reconfirmation_crash_child() {
    let root = std::path::PathBuf::from(std::env::var("MON_AUDIO_RECONFIRMATION_ROOT").unwrap());
    let source = std::env::var("MON_AUDIO_RECONFIRMATION_SOURCE").unwrap();
    let operation = std::env::var("MON_AUDIO_RECONFIRMATION_OPERATION").unwrap();
    assert_eq!(root.file_name().unwrap(), "root");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(
            root.parent()
                .unwrap()
                .join("audio-reconfirmation-owner.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        marker,
        (
            root.to_str().unwrap().into(),
            source.clone(),
            operation.clone()
        )
    );
    let storage = Storage::open(root).unwrap();
    let owner = AnalysisController::default()
        .begin_audio_deletion_maintenance(&storage, &source, false, false, &|| false)
        .unwrap();
    let review = Review::capture(&storage, &source, &owner, &|| false).unwrap();
    accept_inner(
        &storage,
        &operation,
        &review,
        policy(),
        &owner,
        &|| false,
        &|event| {
            if event == "audioReconfirmationCommitted" {
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGKILL);
                }
                panic!("SIGKILL failed");
            }
        },
    )
    .unwrap();
    panic!("boundary not reached");
}
