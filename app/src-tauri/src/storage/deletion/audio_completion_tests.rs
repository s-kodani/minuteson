//! Owned synthetic files and SQLite only; no user audio or native IPC.
use super::*;
use crate::storage::deletion::database::Scope;
use crate::{
    analysis_worker::AnalysisController,
    storage::{
        deletion::audio::{
            intent::{prepare, Policy},
            tests::Fixture,
        },
        JobKind, JobSpec,
    },
};
use std::cell::Cell;

fn audio_job(meeting: &str, operation: &str) -> JobSpec {
    JobSpec {
        meeting_id: meeting.into(),
        kind: JobKind::Transcription,
        operation_id: operation.into(),
        input_analysis_id: None,
        input_hash: "a".repeat(64),
        profile_snapshot: serde_json::json!({"engine":"test","model_hash":"b".repeat(64)}),
        parameters: serde_json::json!({"window_seconds":30}),
        total_units: Some(10),
    }
}

fn owner() -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap()
}
fn prepared(f: &Fixture, lease: &DataMaintenanceLease) -> String {
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
fn removed(f: &Fixture, lease: &DataMaintenanceLease) -> String {
    let op = prepared(f, lease);
    super::super::start(&f.storage, &op, lease, &|| false).unwrap();
    assert!(super::super::run(&f.storage, &op, lease, &|| false)
        .unwrap()
        .files_removed());
    op
}
fn fixed(f: &Fixture, op: &str) -> String {
    f.storage.connection.lock().unwrap().query_row("SELECT json_array(target_manifest_json,target_hash,consent_at,created_at,(SELECT json_group_array(json_array(relative_path,identity_json,identity_hash)) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1",[op],|r|r.get(0)).unwrap()
}
fn reopen(f: &mut Fixture) {
    let root = f.storage.root().to_owned();
    let temp = tempfile::tempdir().unwrap();
    let old = std::mem::replace(
        &mut f.storage,
        Storage::open(temp.path().to_owned()).unwrap(),
    );
    drop(old);
    f.storage = Storage::open(root).unwrap();
}
fn resume(f: &Fixture, op: &str) -> DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(&f.storage, op, false, false, &|| false)
        .unwrap()
}
fn hint(f: &Fixture, op: &str) -> Option<String> {
    crate::storage::deletion::startup::reason(&f.storage.connection.lock().unwrap(), op).unwrap()
}
fn phase(f: &Fixture, op: &str) -> String {
    f.storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1",
            [op],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn native_raw_audio_deletion_cold_restart_removes_pcm_but_keeps_documents_and_raw_proofs() {
    let mut f = Fixture::new();
    f.documents();
    let raw = f.file(
        &format!("audio/{}/raw-system-00000001.caf", f.session),
        None,
    );
    let metadata = f.file(
        &format!("audio/{}/raw-system-00000001.json", f.session),
        None,
    );
    let pointer = f.file(&format!("audio/{}/raw-system.json", f.session), None);
    let unknown = f.file(
        &format!("audio/{}/raw-system-00000002.caf.part", f.session),
        None,
    );
    let before_metadata = std::fs::read(&metadata).unwrap();
    let before_pointer = std::fs::read(&pointer).unwrap();
    let lease = owner();
    let op = prepared(&f, &lease);
    let original = fixed(&f, &op);
    super::super::start(&f.storage, &op, &lease, &|| false).unwrap();
    // Stop after durable consent/start, not by recreating a deletion permit.
    drop(lease);
    reopen(&mut f);
    let lease = resume(&f, &op);
    let receipt = super::super::run(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(receipt.files_removed());
    assert_eq!(receipt.processed_files(), 2);
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
    assert_eq!(fixed(&f, &op), original);
    assert!(!raw.exists() && !f.path.exists());
    assert_eq!(std::fs::read(&metadata).unwrap(), before_metadata);
    assert_eq!(std::fs::read(&pointer).unwrap(), before_pointer);
    assert!(unknown.is_file());
    assert!(f
        .storage
        .minutes_versions(&f.meeting)
        .unwrap()
        .current_id
        .is_some());
    drop(lease);
    reopen(&mut f);
    assert!(!f.storage.data_write_blocked());
    assert!(deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
    assert_eq!(fixed(&f, &op), original);
}

#[test]
fn history_v27_completed_audio_upgrades_preserving_unavailable_source_and_original_evidence() {
    let mut f = Fixture::new();
    f.documents();
    let lease = owner();
    let op = removed(&f, &lease);
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
    drop(lease);
    {
        let db = f.storage.connection.lock().unwrap();
        crate::storage::deletion::audio::intent::historical_completed_fixture(&db, &op);
        crate::storage::edit_recovery::remove_schema_for_test(&db);
    }
    let saved = fixed(&f, &op);
    reopen(&mut f);
    assert_eq!(fixed(&f, &op), saved);
    assert_eq!(hint(&f, &op), None);
    assert!(!f.storage.data_write_blocked());
    assert_eq!(phase(&f, &op), "completed");
    assert!(crate::storage::deletion::audio::require_source(
        &f.storage.connection.lock().unwrap(),
        &f.meeting
    )
    .is_err());
    assert_eq!(
        crate::storage::verify_migration_history(&f.storage.connection.lock().unwrap()).unwrap(),
        crate::storage::MIGRATIONS.len()
    );
    f.storage
        .create_meeting("OTHER MEETING AFTER AUDIO HISTORY UPGRADE")
        .unwrap();
    reopen(&mut f);
    assert_eq!(fixed(&f, &op), saved);
    assert!(!f.storage.data_write_blocked());
}
#[test]
fn completion_changes_only_availability_retains_every_history_and_releases_hot_fence() {
    let f = Fixture::new();
    f.documents();
    let manifest = f.file(
        &format!("audio/{}/manifest.json", f.session),
        Some("application/json"),
    );
    let text = f.file("documents/minutes.md", Some("text/markdown"));
    let lease = owner();
    let op = removed(&f, &lease);
    let original = fixed(&f, &op);
    let before = Scope::capture(
        &f.storage.connection.lock().unwrap(),
        &f.meeting,
        &op,
        &|| false,
    )
    .unwrap();
    let done = finish(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(done.completed());
    assert_eq!(fixed(&f, &op), original);
    assert_eq!(phase(&f, &op), "completed");
    assert!(!f.storage.data_write_blocked());
    assert!(manifest.is_file() && text.is_file() && !f.path.exists());
    let db = f.storage.connection.lock().unwrap();
    let after = Scope::capture(&db, &f.meeting, &op, &|| false).unwrap();
    for (a, b) in before.tables.iter().zip(&after.tables) {
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.name, b.name);
        if !matches!(
            a.name.as_str(),
            "audio_chunks" | "audio_tracks" | "file_assets"
        ) {
            assert_eq!(a, b);
        }
    }
    let states:(String,String,String)=db.query_row("SELECT f.state,t.state,c.state FROM file_assets f JOIN audio_chunks c ON c.file_asset_id=f.id JOIN audio_tracks t ON t.id=c.track_id WHERE f.id=?1",[&f.asset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        states,
        ("missing".into(), "failed".into(), "missing".into())
    );
    assert!(deleted(&db, &f.meeting).unwrap());
    verify_available_state(&db, &done, &|| false).unwrap();
    drop(db);
    assert!(f
        .storage
        .minutes_versions(&f.meeting)
        .unwrap()
        .current_id
        .is_some());
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
}
#[test]
fn mime_case_cannot_retain_audio_body_or_bypass_deleted_source_admission() {
    let f = Fixture::new();
    let audio = f.file("registered-upper.flac", Some("Audio/FLAC"));
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(!audio.exists());
    let missing = std::path::PathBuf::from(format!("meetings/{}/unopened.flac", f.meeting));
    assert!(f
        .storage
        .register_ready_meeting_asset(&f.meeting, &missing, "AuDiO/FLAC")
        .err()
        .unwrap()
        .contains("削除済み"));
    assert!(deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
}
#[test]
fn completion_requires_files_finished_and_live_actual_lease() {
    let f = Fixture::new();
    let controller = AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let op = prepared(&f, &lease);
    assert!(finish(&f.storage, &op, &lease, &|| false).is_err());
    assert!(f.path.exists());
    super::super::start(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(finish(&f.storage, &op, &lease, &|| false).is_err());
    super::super::run(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(finish(&f.storage, &op, &lease, &|| true).is_err());
    controller.begin_exit().unwrap();
    controller.abort_shutdown();
    assert!(finish(&f.storage, &op, &lease, &|| false).is_err());
    assert_eq!(phase(&f, &op), "cleanup");
}
#[test]
fn missing_journal_in_current_schema_never_grants_source_admission() {
    for truncated_history in [false, true] {
        let f = Fixture::new();
        let db = f.storage.connection.lock().unwrap();
        db.execute_batch("DROP TABLE maintenance_paths; DROP TABLE maintenance_operations;")
            .unwrap();
        if truncated_history {
            db.execute("DELETE FROM schema_migrations WHERE version>=22", [])
                .unwrap();
        }
        assert!(deleted(&db, &f.meeting).is_err());
        assert!(crate::storage::deletion::audio::require_source(&db, &f.meeting).is_err());
    }
}
#[test]
fn each_precommit_cancel_rolls_back_all_availability_and_progress_not_deleted_files() {
    for boundary in [
        "audioAssetsUnavailable",
        "audioChunksUnavailable",
        "audioTracksUnavailable",
        "beforeAudioCompletionCommit",
    ] {
        let f = Fixture::new();
        let lease = owner();
        let op = removed(&f, &lease);
        let tables =
            database::audio_tables(&f.storage.connection.lock().unwrap(), &f.meeting, &|| false)
                .unwrap();
        let cancel = Cell::new(false);
        assert!(
            finish_inner(
                &f.storage,
                &op,
                &lease,
                &|| cancel.get(),
                &|e| if e == boundary {
                    cancel.set(true)
                }
            )
            .is_err(),
            "{boundary}"
        );
        assert_eq!(
            database::audio_tables(&f.storage.connection.lock().unwrap(), &f.meeting, &|| false)
                .unwrap(),
            tables
        );
        assert_eq!(phase(&f, &op), "cleanup");
        assert!(!f.path.exists());
        assert!(f.storage.data_write_blocked());
        assert!(finish(&f.storage, &op, &lease, &|| false)
            .unwrap()
            .completed());
    }
}
#[test]
fn last_boundary_rechecks_retained_text_and_rolls_back_if_changed() {
    let f = Fixture::new();
    let text = f.file("keep.txt", Some("text/plain"));
    let lease = owner();
    let op = removed(&f, &lease);
    assert!(finish_inner(&f.storage, &op, &lease, &|| false, &|e| if e
        == "beforeAudioCompletionCommit"
    {
        std::fs::write(&text, b"CHANGED").unwrap()
    })
    .is_err());
    assert_eq!(phase(&f, &op), "cleanup");
    assert!(f.storage.data_write_blocked());
    assert!(!deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
}
#[test]
fn late_cancel_cannot_turn_durable_completion_into_failure() {
    let f = Fixture::new();
    let lease = owner();
    let op = removed(&f, &lease);
    let cancel = Cell::new(false);
    assert!(
        finish_inner(&f.storage, &op, &lease, &|| cancel.get(), &|e| if e
            == "audioCompletionCommitted"
        {
            cancel.set(true)
        })
        .unwrap()
        .completed()
    );
    assert!(!f.storage.data_write_blocked());
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
}
#[test]
fn postcommit_panic_keeps_receipt_and_restores_fence_before_callback() {
    let mut f = Fixture::new();
    let lease = owner();
    let op = removed(&f, &lease);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| finish_inner(
            &f.storage,
            &op,
            &lease,
            &|| false,
            &|e| if e == "audioCompletionCommitted" {
                assert!(!f.storage.data_write_blocked());
                panic!("owned test postcommit")
            }
        )))
        .is_err()
    );
    // Panic poisoned this test's DB mutex; cold reopen models process restart.
    drop(lease);
    reopen(&mut f);
    assert_eq!(phase(&f, &op), "completed");
    assert!(!f.storage.data_write_blocked());
    assert_eq!(hint(&f, &op), None);
}
#[test]
fn completion_allows_later_text_edits_generation_and_nonaudio_assets_without_new_audio() {
    let f = Fixture::new();
    f.documents();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    let saved = fixed(&f, &op);
    let meeting = f
        .storage
        .list_meetings()
        .unwrap()
        .into_iter()
        .find(|m| m.id == f.meeting)
        .unwrap();
    f.storage
        .rename_meeting(&f.meeting, "EDITED", meeting.lock_version)
        .unwrap();
    let new = f.file("documents/new.md", Some("text/markdown"));
    let candidate = crate::storage::minutes_ui::tests::candidate(&f.storage, &f.meeting);
    assert!(!candidate.is_empty());
    assert!(new.is_file());
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
    assert_eq!(fixed(&f, &op), saved);
    assert!(deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
    // Missing path makes it explicit that rejection precedes file open/hash.
    let missing = std::path::PathBuf::from(format!("meetings/{}/never-open.flac", f.meeting));
    let error = f
        .storage
        .register_ready_meeting_asset(&f.meeting, &missing, "audio/flac")
        .err()
        .unwrap();
    assert!(error.contains("削除済み"));
}
#[test]
fn playback_and_new_audio_plans_reject_before_using_history_or_reappeared_body() {
    let f = Fixture::new();
    f.documents();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    std::fs::write(&f.path, b"ALIEN AUDIO").unwrap();
    assert!(f
        .storage
        .prepare_track_audio(&f.meeting, &f.track, 0, 1_000_000)
        .err()
        .unwrap()
        .contains("削除済み"));
    let db = f.storage.connection.lock().unwrap();
    assert!(
        crate::storage::transcription::read_audio_plan(&db, &f.meeting)
            .err()
            .unwrap()
            .contains("削除済み")
    );
    drop(db);
    let spec = audio_job(&f.meeting, "new-transcription");
    assert!(f
        .storage
        .enqueue_job(spec)
        .err()
        .unwrap()
        .contains("削除済み"));
    assert_eq!(std::fs::read(&f.path).unwrap(), b"ALIEN AUDIO");
}
#[test]
fn old_queued_audio_job_cannot_acquire_but_fixed_input_stays_readable() {
    let f = Fixture::new();
    let spec = audio_job(&f.meeting, "queued-audio");
    let job = f.storage.enqueue_job(spec.clone()).unwrap();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    assert_eq!(f.storage.fixed_job_input(&job.id).unwrap().0, spec);
    assert!(f.storage.acquire_next_job().unwrap().is_none());
    assert_eq!(f.storage.list_jobs(&f.meeting).unwrap()[0].state, "queued");
}
#[test]
fn deleted_queued_audio_never_starves_other_meeting_and_corrupt_history_is_not_skipped() {
    for corrupt in [false, true] {
        let f = Fixture::new();
        let old = f
            .storage
            .enqueue_job(audio_job(&f.meeting, "old-queued"))
            .unwrap();
        let lease = owner();
        let op = removed(&f, &lease);
        finish(&f.storage, &op, &lease, &|| false).unwrap();
        let other = f.storage.create_meeting("OTHER").unwrap();
        let next = f
            .storage
            .enqueue_job(audio_job(&other.id, "new-queued"))
            .unwrap();
        if corrupt {
            f.storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE file_assets SET state='ready' WHERE id=?1",
                    [&f.asset],
                )
                .unwrap();
            assert!(f.storage.acquire_next_job().is_err());
            assert_eq!(f.storage.get_job(&next.id).unwrap().state, "queued");
        } else {
            assert_eq!(f.storage.acquire_next_job().unwrap().unwrap().id, next.id);
        }
        assert_eq!(f.storage.get_job(&old.id).unwrap().state, "queued");
    }
}
#[test]
fn diarization_enqueue_and_cached_input_reject_before_configuring_or_reading_snapshot() {
    let f = Fixture::new();
    f.documents();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    let request = crate::storage::MinutesJobRequest {
        meeting_id: f.meeting.clone(),
        operation_id: "new-diarization".into(),
        expected_analysis_id: uuid::Uuid::new_v4().to_string(),
        expected_content_hash: "a".repeat(64),
        expected_meeting_lock_version: 0,
        expected_draft: None,
    };
    let configured = Cell::new(false);
    assert!(f
        .storage
        .enqueue_diarization_source(request, serde_json::json!({}), None, |_| {
            configured.set(true);
            Ok((serde_json::json!({}), 1))
        })
        .err()
        .unwrap()
        .contains("削除済み"));
    assert!(!configured.get());
    let mut spec = audio_job(&f.meeting, "cached-diarization");
    spec.kind = JobKind::Diarization;
    spec.input_analysis_id = Some(uuid::Uuid::new_v4().to_string());
    let mut db = f.storage.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    assert!(crate::storage::diarization::read_input(&tx, &spec)
        .err()
        .unwrap()
        .contains("削除済み"));
}
#[test]
fn retained_transcript_draft_can_save_after_completion_without_changing_audio_history() {
    let f = Fixture::new();
    f.documents();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    let draft = f
        .storage
        .load_transcript_draft(&f.meeting)
        .unwrap()
        .unwrap();
    let mut payload = draft.payload.clone();
    let original = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: original.utterances[0].id.clone(),
            effective_text: "AFTER AUDIO DELETION".into(),
            speaker_ids: original.utterances[0].speaker_ids.clone(),
        });
    let changed = f
        .storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    assert_eq!(
        changed.payload.utterances[0].effective_text,
        "AFTER AUDIO DELETION"
    );
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
    assert!(deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
}
#[test]
fn paused_or_interrupted_audio_job_cannot_resume_without_mutating_its_history() {
    for state in ["paused", "interrupted"] {
        let f = Fixture::new();
        let job = f
            .storage
            .enqueue_job(audio_job(&f.meeting, "paused-audio"))
            .unwrap();
        f.storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE processing_jobs SET state=?2 WHERE id=?1",
                params![job.id, state],
            )
            .unwrap();
        let lease = owner();
        let op = removed(&f, &lease);
        finish(&f.storage, &op, &lease, &|| false).unwrap();
        assert!(f
            .storage
            .resume_job(&job.id)
            .err()
            .unwrap()
            .contains("削除済み"));
        assert_eq!(f.storage.list_jobs(&f.meeting).unwrap()[0].state, state);
    }
}
#[test]
fn failed_audio_retry_refuses_even_fixed_cached_input_and_preserves_original() {
    let f = Fixture::new();
    let spec = audio_job(&f.meeting, "failed-audio");
    let job = f.storage.enqueue_job(spec.clone()).unwrap();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='failed' WHERE id=?1",
            [&job.id],
        )
        .unwrap();
    let source = f.storage.list_jobs(&f.meeting).unwrap().remove(0);
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    assert!(f
        .storage
        .enqueue_retry_job(&source, &spec, None, "new-retry")
        .err()
        .unwrap()
        .contains("削除済み"));
    assert_eq!(f.storage.list_jobs(&f.meeting).unwrap(), vec![source]);
}
#[test]
fn cold_pending_completion_requires_restart_then_clean_startup_accepts_typed_audio_receipt() {
    let mut f = Fixture::new();
    let lease = owner();
    let op = removed(&f, &lease);
    drop(lease);
    reopen(&mut f);
    assert!(f.storage.data_write_blocked());
    let lease = resume(&f, &op);
    assert!(finish(&f.storage, &op, &lease, &|| false)
        .unwrap()
        .completed());
    assert!(f.storage.data_write_blocked());
    assert!(f.storage.create_meeting("WAIT RESTART").is_err());
    drop(lease);
    reopen(&mut f);
    assert!(!f.storage.data_write_blocked());
    assert_eq!(hint(&f, &op), None);
    f.storage.create_meeting("AFTER RESTART").unwrap();
}
#[test]
fn completed_startup_reappearance_never_deletes_or_recovers_stale_jobs() {
    for reserved in [false, true] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = removed(&f, &lease);
        let r = finish(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        let path = if reserved {
            f.path.parent().unwrap().join(r.reserved(0))
        } else {
            f.path.clone()
        };
        std::fs::write(&path, b"NEW EXPLICIT REVIEW ONLY").unwrap();
        let stale = f.job("transcription");
        f.storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE processing_jobs SET state='running' WHERE id=?1",
                [&stale],
            )
            .unwrap();
        reopen(&mut f);
        assert!(f.storage.data_write_blocked());
        assert_eq!(
            hint(&f, &op).as_deref(),
            Some("audio_completed_scope_reappeared")
        );
        assert_eq!(f.storage.list_jobs(&f.meeting).unwrap()[0].state, "running");
        let lease = resume(&f, &op);
        assert!(finish(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"NEW EXPLICIT REVIEW ONLY");
        std::fs::remove_file(&path).unwrap();
        finish(&f.storage, &op, &lease, &|| false).unwrap();
        assert_eq!(hint(&f, &op), None);
        assert!(f.storage.data_write_blocked());
        drop(lease);
        reopen(&mut f);
        assert_eq!(
            f.storage.list_jobs(&f.meeting).unwrap()[0].state,
            "interrupted"
        );
    }
}
#[test]
fn completed_metadata_corruption_and_ready_resurrection_never_grant_audio_access() {
    for mutation in ["hash", "count", "phase", "subtype", "ready"] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = removed(&f, &lease);
        finish(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        {
            let db = f.storage.connection.lock().unwrap();
            let mut r = Receipt::load(&db, &op, &|| false).unwrap();
            // Simulate disk corruption in this owned fixture only. Production
            // intent immutability remains enabled and unchanged.
            db.set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                false,
            )
            .unwrap();
            match mutation {
                "ready" => {
                    db.execute(
                        "UPDATE file_assets SET state='ready' WHERE id=?1",
                        [&f.asset],
                    )
                    .unwrap();
                }
                "subtype" => {
                    db.execute("UPDATE maintenance_operations SET target_manifest_json=json_set(target_manifest_json,'$.subtype','modelDeletion') WHERE id=?1",[&op]).unwrap();
                }
                _ => {
                    match mutation {
                        "hash" => {
                            r.progress.final_audio.as_mut().unwrap()[0].sha256 = "c".repeat(64)
                        }
                        "count" => r.progress.final_audio.as_mut().unwrap()[1].rows += 1,
                        _ => r.progress.phase = Phase::AudioFilesRemoved,
                    };
                    db.execute(
                        "UPDATE maintenance_operations SET progress_json=?2 WHERE id=?1",
                        params![op, r.progress.encode().unwrap()],
                    )
                    .unwrap();
                }
            }
            db.set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                true,
            )
            .unwrap();
            assert!(deleted(&db, &f.meeting).is_err());
        }
        reopen(&mut f);
        assert!(f.storage.data_write_blocked());
        assert!(hint(&f, &op).is_some());
        assert!(!f.path.exists());
    }
}
#[test]
fn completed_parent_replacement_or_symlink_is_attention_not_auto_cleanup() {
    for symlink in [false, true] {
        let mut f = Fixture::new();
        let lease = owner();
        let op = removed(&f, &lease);
        finish(&f.storage, &op, &lease, &|| false).unwrap();
        drop(lease);
        let parent = f.path.parent().unwrap();
        let held = parent.with_extension("owned-held");
        std::fs::rename(parent, &held).unwrap();
        let outside = f.base.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let keep = outside.join("microphone.caf");
        std::fs::write(&keep, b"OUTSIDE").unwrap();
        if symlink {
            std::os::unix::fs::symlink(&outside, parent).unwrap();
        } else {
            std::fs::create_dir(parent).unwrap();
        }
        reopen(&mut f);
        assert_eq!(
            hint(&f, &op).as_deref(),
            Some("audio_completed_scope_uninspectable")
        );
        assert!(f.storage.data_write_blocked());
        let lease = resume(&f, &op);
        assert!(finish(&f.storage, &op, &lease, &|| false).is_err());
        assert_eq!(std::fs::read(keep).unwrap(), b"OUTSIDE");
    }
}
#[test]
fn availability_validation_is_independent_of_other_meeting_running_jobs() {
    let f = Fixture::new();
    let other = f.storage.create_meeting("OTHER").unwrap();
    let lease = owner();
    let op = removed(&f, &lease);
    finish(&f.storage, &op, &lease, &|| false).unwrap();
    let job = f
        .storage
        .enqueue_job(audio_job(&other.id, "other-job"))
        .unwrap();
    assert_eq!(f.storage.acquire_next_job().unwrap().unwrap().id, job.id);
    assert!(deleted(&f.storage.connection.lock().unwrap(), &f.meeting).unwrap());
    assert!(f
        .storage
        .prepare_track_audio(&f.meeting, &f.track, 0, 1)
        .err()
        .unwrap()
        .contains("削除済み"));
}
#[test]
fn completion_sigkill_each_atomic_boundary_is_cleanup_or_fully_completed() {
    use std::os::unix::process::ExitStatusExt;
    for boundary in [
        "audioAssetsUnavailable",
        "audioChunksUnavailable",
        "audioTracksUnavailable",
        "beforeAudioCompletionCommit",
        "audioCompletionCommitted",
    ] {
        let f = Fixture::new();
        let text = f.file("keep.txt", Some("text/plain"));
        let lease = owner();
        let op = removed(&f, &lease);
        drop(lease);
        let root = f.storage.root().to_owned();
        let meeting = f.meeting.clone();
        std::fs::write(
            f.base.path().join("audio-completion-owner.json"),
            serde_json::to_vec(&(root.to_str().unwrap(), meeting.as_str(), op.as_str())).unwrap(),
        )
        .unwrap();
        drop(f.storage);
        let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","storage::deletion::audio::intent::removal::completion::tests::audio_completion_crash_child","--ignored","--nocapture"]).env("MON_AUDIO_COMPLETION_ROOT",&root).env("MON_AUDIO_COMPLETION_MEETING",&meeting).env("MON_AUDIO_COMPLETION_OPERATION",&op).env("MON_AUDIO_COMPLETION_BOUNDARY",boundary).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{boundary}");
        let storage = Storage::open(root.clone()).unwrap();
        let phase: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM maintenance_operations WHERE id=?1",
                [&op],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            phase,
            if boundary == "audioCompletionCommitted" {
                "completed"
            } else {
                "cleanup"
            }
        );
        let lease = AnalysisController::default()
            .begin_audio_deletion_maintenance(&storage, &op, false, false, &|| false)
            .unwrap();
        finish(&storage, &op, &lease, &|| false).unwrap();
        assert!(text.is_file());
        assert!(!f.path.exists());
        drop(lease);
        drop(storage);
        let storage = Storage::open(root).unwrap();
        assert!(!storage.data_write_blocked());
        assert!(deleted(&storage.connection.lock().unwrap(), &meeting).unwrap());
    }
}
#[test]
#[ignore = "subprocess-only parent-owned synthetic audio completion fixture"]
fn audio_completion_crash_child() {
    let root = std::path::PathBuf::from(std::env::var("MON_AUDIO_COMPLETION_ROOT").unwrap());
    let meeting = std::env::var("MON_AUDIO_COMPLETION_MEETING").unwrap();
    let op = std::env::var("MON_AUDIO_COMPLETION_OPERATION").unwrap();
    let boundary = std::env::var("MON_AUDIO_COMPLETION_BOUNDARY").unwrap();
    assert_eq!(root.file_name().unwrap(), "root");
    assert_eq!(root.canonicalize().unwrap(), root);
    let marker: (String, String, String) = serde_json::from_slice(
        &std::fs::read(root.parent().unwrap().join("audio-completion-owner.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(marker, (root.to_str().unwrap().into(), meeting, op.clone()));
    let storage = Storage::open(root).unwrap();
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(&storage, &op, false, false, &|| false)
        .unwrap();
    finish_inner(&storage, &op, &lease, &|| false, &|event| {
        if event == boundary {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            panic!("SIGKILL failed")
        }
    })
    .unwrap();
    panic!("boundary was not reached");
}
