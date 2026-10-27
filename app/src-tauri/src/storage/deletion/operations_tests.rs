use super::*;
fn lease() -> crate::analysis_worker::data_maintenance::DataMaintenanceLease {
    crate::analysis_worker::AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap()
}
use std::{cell::Cell, path::PathBuf};
struct Fixture {
    _root: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    audio: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = storage.create_meeting("削除対象").unwrap().id;
        let other = storage.create_meeting("保持対象").unwrap().id;
        let audio = storage
            .root()
            .join(format!("meetings/{meeting}/audio/source.flac"));
        std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
        std::fs::write(&audio, b"TARGET").unwrap();
        storage
            .register_ready_meeting_asset(
                &meeting,
                audio.strip_prefix(storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        Self {
            _root: root,
            storage,
            meeting,
            other,
            audio,
        }
    }
    fn prepared(&self) -> Prepared {
        Prepared::fresh(
            &self.storage,
            &self.meeting,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn pending(&self) -> Prepared {
        let p = self.prepared();
        let Target::Fresh(c) = &p.target else {
            unreachable!()
        };
        intent::prepare(
            &self.storage,
            &p.preview.operation_id,
            c,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        Prepared::pending(
            &self.storage,
            &p.preview.operation_id,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn unaffected(&self) {
        assert!(self.storage.get_meeting(&self.other).unwrap().is_some());
    }
}
#[test]
fn deletion_review_is_read_only_and_wrong_confirmation_never_writes_or_removes() {
    let f = Fixture::new();
    let mut p = f.prepared();
    assert!(p.preview.execute_allowed);
    assert_eq!(p.preview.title, "削除対象");
    assert_eq!(p.preview.file_count, 1);
    assert_eq!(p.preview.bytes, 6);
    assert!(listing(&f.storage).unwrap().items.is_empty());
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            "wrong",
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(listing(&f.storage).unwrap().items.is_empty());
    assert!(f.audio.exists());
    f.unaffected();
}

#[test]
fn automatic_cleanup_cancels_unstarted_without_deleting_changed_audio() {
    let f = Fixture::new();
    let p = f.pending();
    std::fs::write(&f.audio, b"KEEP CHANGED AUDIO").unwrap();
    assert!(crate::storage::deletion::automatic::meeting(
        &f.storage,
        &lease(),
        &p.preview.operation_id,
        &crate::test_support::never_cancelled
    )
    .unwrap());
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"KEEP CHANGED AUDIO");
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert!(listing(&f.storage).unwrap().items.is_empty());
    f.unaffected();
}

#[test]
fn automatic_cleanup_finishes_fixed_scope_and_preserves_reappeared_files() {
    for changed in [false, true] {
        let f = Fixture::new();
        let mut p = f.prepared();
        let stop = Cell::new(false);
        let id = p.preview.confirmation_id.clone();
        assert!(p
            .execute(
                &f.storage,
                &lease(),
                &id,
                false,
                &|| stop.get(),
                &|phase| if phase == "deleting_database" {
                    stop.set(true)
                }
            )
            .is_err());
        assert!(!f.audio.exists());
        if changed {
            std::fs::create_dir_all(f.audio.parent().unwrap()).unwrap();
            std::fs::write(&f.audio, b"NEW AUDIO MUST SURVIVE").unwrap();
        }
        let result = crate::storage::deletion::automatic::meeting(
            &f.storage,
            &lease(),
            &p.preview.operation_id,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
        assert_eq!(result, !changed);
        assert_eq!(
            f.storage.get_meeting(&f.meeting).unwrap().is_some(),
            changed
        );
        if changed {
            assert_eq!(std::fs::read(&f.audio).unwrap(), b"NEW AUDIO MUST SURVIVE");
        }
        f.unaffected();
    }
}

#[test]
fn automatic_cleanup_requires_active_admission_and_honors_cancellation() {
    let f = Fixture::new();
    let p = f.pending();
    assert!(crate::storage::deletion::automatic::meeting(
        &f.storage,
        &lease(),
        &p.preview.operation_id,
        &|| true
    )
    .is_err());
    let analysis = crate::analysis_worker::AnalysisController::default();
    let invalid = analysis
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    analysis.begin_exit().unwrap();
    assert!(crate::storage::deletion::automatic::meeting(
        &f.storage,
        &invalid,
        &p.preview.operation_id,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f.audio.exists());
    assert_eq!(listing(&f.storage).unwrap().items.len(), 1);
    f.unaffected();
}
#[test]
fn deletion_confirmed_pipeline_only_acknowledges_after_files_rows_and_wal_complete() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    let r = p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    assert!(r.deleted && !r.retained && !r.restart_required);
    assert!(!f.audio.exists());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(!f.storage.data_write_blocked());
    f.unaffected();
    let mut replay = Prepared::pending(
        &f.storage,
        &r.operation_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(replay.preview.phase, "awaiting_ack");
    let id = replay.preview.confirmation_id.clone();
    assert!(
        replay
            .execute(
                &f.storage,
                &lease(),
                &id,
                false,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
}
#[test]
fn deletion_unregistered_contents_require_separate_explicit_decision_before_any_intent() {
    let f = Fixture::new();
    let unknown = f.audio.parent().unwrap().join("unknown.txt");
    std::fs::write(&unknown, b"UNKNOWN").unwrap();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    assert_eq!(p.preview.unregistered_count, 1);
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(listing(&f.storage).unwrap().items.is_empty());
    assert!(unknown.exists() && f.audio.exists());
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    let id = p.preview.confirmation_id.clone();
    p.execute(
        &f.storage,
        &lease(),
        &id,
        true,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    assert!(!unknown.exists());
    f.unaffected();
}
#[test]
fn deletion_changed_scope_requires_new_review_and_rejects_old_confirmation() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let old = p.preview.confirmation_id.clone();
    let op = p.preview.operation_id.clone();
    let meeting = f.storage.get_meeting(&f.meeting).unwrap().unwrap();
    f.storage
        .rename_meeting(&f.meeting, "変更後", meeting.lock_version)
        .unwrap();
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &old,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert!(listing(&f.storage).unwrap().items.is_empty());
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(p.preview.operation_id, op);
    assert_eq!(p.preview.title, "変更後");
    assert_ne!(p.preview.confirmation_id, old);
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &old,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    let id = p.preview.confirmation_id.clone();
    p.execute(
        &f.storage,
        &lease(),
        &id,
        false,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    f.unaffected();
}
#[test]
fn deletion_path_pages_are_bounded_metadata_and_reject_stale_or_unaligned_offsets() {
    let f = Fixture::new();
    for n in 0..105 {
        std::fs::write(
            f.audio.parent().unwrap().join(format!("unknown-{n:03}")),
            b"PRIVATE BODY",
        )
        .unwrap();
    }
    let p = f.prepared();
    let id = &p.preview.confirmation_id;
    let first = p.page(id, 0).unwrap();
    assert_eq!(first.entries.len(), 50);
    assert_eq!(first.next_offset, Some(50));
    let directories: Vec<_> = first
        .entries
        .iter()
        .filter(|entry| entry.directory)
        .collect();
    assert!(!directories.is_empty());
    assert!(directories.iter().all(|entry| entry.bytes.is_none()
        && !entry.registered
        && !entry.changed
        && !entry.missing));
    let audio = first
        .entries
        .iter()
        .find(|entry| entry.relative_path.ends_with("/source.flac"))
        .unwrap();
    assert_eq!(audio.bytes, Some(6));
    assert!(audio.registered && !audio.missing && !audio.changed);
    assert_eq!(p.page(id, 50).unwrap().entries.len(), 50);
    assert!(p.page(id, 100).unwrap().entries.len() < 50);
    assert!(p.page("wrong", 0).is_err());
    assert!(p.page(id, 1).is_err());
    assert!(p.page(id, 150).is_err());
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("PRIVATE BODY"));
}
#[test]
fn deletion_cancel_before_confirmation_persists_nothing() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    assert!(p
        .execute(&f.storage, &lease(), &id, false, &|| true, &|_| {})
        .is_err());
    assert!(listing(&f.storage).unwrap().items.is_empty());
    assert!(f.audio.exists());
}
#[test]
fn deletion_cancel_after_start_reloads_durable_progress_and_requires_explicit_refresh() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    let stop = Cell::new(false);
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &|| stop.get(),
            &|phase| if phase == "deleting_files" {
                stop.set(true)
            }
        )
        .is_err());
    assert!(p.preview.pending && !p.preview.execute_allowed && !p.preview.keep_allowed);
    assert!(f.audio.exists());
    assert!(f.storage.data_write_blocked());
    assert!(p
        .keep(
            &f.storage,
            &lease(),
            &p.preview.confirmation_id.clone(),
            &crate::test_support::never_cancelled
        )
        .is_err());
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(p.preview.execute_allowed);
    let id = p.preview.confirmation_id.clone();
    p.execute(
        &f.storage,
        &lease(),
        &id,
        false,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    f.unaffected();
}
#[test]
fn deletion_cancel_at_database_handoff_reports_partial_files_and_resumes_original_scope() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    let stop = Cell::new(false);
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &|| stop.get(),
            &|phase| if phase == "deleting_database" {
                stop.set(true)
            }
        )
        .is_err());
    assert!(!f.audio.exists());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(p.preview.phase, "database");
    assert_eq!(p.preview.processed_paths, p.preview.path_count);
    assert!(!p.preview.row_phase_committed);
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    let id = p.preview.confirmation_id.clone();
    p.execute(
        &f.storage,
        &lease(),
        &id,
        false,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    f.unaffected();
}
#[test]
fn deletion_unstarted_keep_preserves_changed_contents_and_replays_without_claiming_restore() {
    let f = Fixture::new();
    let mut p = f.pending();
    std::fs::write(&f.audio, b"KEEP CHANGED").unwrap();
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(!p.preview.execute_allowed && p.preview.keep_allowed);
    let id = p.preview.confirmation_id.clone();
    let r = p
        .keep(
            &f.storage,
            &lease(),
            &id,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(r.retained && !r.deleted);
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"KEEP CHANGED");
    assert!(
        p.keep(
            &f.storage,
            &lease(),
            &id,
            &crate::test_support::never_cancelled
        )
        .unwrap()
        .retained
    );
    assert!(!f.storage.data_write_blocked());
    f.unaffected();
}
#[test]
fn deletion_cold_pending_review_skips_recovery_and_requires_restart_after_resolution() {
    let f = Fixture::new();
    let p = f.pending();
    let op = p.preview.operation_id;
    let root = f.storage.root().to_path_buf();
    let audio = f.audio.clone();
    drop(f.storage);
    let storage = Storage::open(root.clone()).unwrap();
    assert!(storage.data_write_blocked());
    let mut p = Prepared::pending(&storage, &op, &crate::test_support::never_cancelled).unwrap();
    let id = p.preview.confirmation_id.clone();
    let r = p
        .keep(
            &storage,
            &lease(),
            &id,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    assert!(r.restart_required);
    assert!(storage.data_write_blocked());
    assert!(audio.exists());
    drop(storage);
    let storage = Storage::open(root).unwrap();
    assert!(!storage.data_write_blocked());
    assert!(audio.exists());
}
#[test]
fn deletion_partial_changed_contents_remain_visible_but_are_never_automatically_adopted() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    let stop = Cell::new(false);
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &|| stop.get(),
            &|phase| if phase == "deleting_database" {
                stop.set(true)
            }
        )
        .is_err());
    std::fs::create_dir_all(f.audio.parent().unwrap()).unwrap();
    std::fs::write(&f.audio, b"KEEP NEW").unwrap();
    p.refresh(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(!p.preview.execute_allowed && !p.preview.keep_allowed);
    assert!(p.preview.resume_reason.is_some());
    let id = p.preview.confirmation_id.clone();
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &id,
            false,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .is_err());
    assert_eq!(std::fs::read(&f.audio).unwrap(), b"KEEP NEW");
}
#[test]
fn deletion_wrong_kind_corrupt_receipt_and_other_pending_operation_never_grant_admission() {
    let f = Fixture::new();
    let p = f.pending();
    let id = p.preview.operation_id;
    assert!(authorize_review(&f.storage, None).is_err());
    assert!(authorize_review(&f.storage, Some("bad")).is_err());
    assert!(authorize_review(&f.storage, Some(&id)).is_ok());
    let db = f.storage.connection.lock().unwrap();
    let Target::Unstarted(ref saved) = p.target else {
        unreachable!()
    };
    let _writer = super::super::fence::write_prepared(
        &f.storage,
        &db,
        saved,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute(
        "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
        params![id],
    )
    .unwrap();
    drop(_writer);
    drop(db);
    assert!(authorize_review(&f.storage, Some(&id)).is_err());
    assert!(f.audio.exists());
}

#[test]
fn deletion_other_pending_kind_is_not_a_deletion_admission_or_silently_ignored() {
    let f = Fixture::new();
    let p = f.pending();
    let other = Uuid::new_v4().to_string();
    let db = f.storage.connection.lock().unwrap();
    let Target::Unstarted(ref saved) = p.target else {
        unreachable!()
    };
    let writer = super::super::fence::write_prepared(
        &f.storage,
        &db,
        saved,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at) VALUES(?1,?2,'delete_assets','attention','{}',?3,'{}',1,1)",params![other,f.other,"0".repeat(64)]).unwrap();
    drop(writer);
    drop(db);
    assert!(authorize_review(&f.storage, Some(&other)).is_err());
    assert!(authorize_review(&f.storage, Some(&p.preview.operation_id)).is_err());
    assert!(f.audio.exists());
    f.unaffected();
}

#[test]
fn deletion_actual_lease_cancellation_stops_even_when_caller_check_ignores_it() {
    let f = Fixture::new();
    let mut p = f.prepared();
    let id = p.preview.confirmation_id.clone();
    let analysis = crate::analysis_worker::AnalysisController::default();
    let lease = analysis
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(p
        .execute(
            &f.storage,
            &lease,
            &id,
            false,
            &crate::test_support::never_cancelled,
            &|phase| if phase == "delete_scope_fixed" {
                analysis.begin_exit().unwrap()
            }
        )
        .is_err());
    assert!(p.preview.pending && p.preview.keep_allowed);
    assert_eq!(p.preview.phase, "not_started");
    assert!(f.audio.exists());
    assert!(lease.validate_active().is_err());
    assert!(analysis.begin_export().is_err());
    f.unaffected();
}

#[test]
fn deletion_matching_database_copy_cannot_adopt_original_storage_confirmation() {
    let f = Fixture::new();
    let p = f.pending();
    let destination = tempfile::tempdir().unwrap();
    let mut copy = rusqlite::Connection::open(destination.path().join("meetings.sqlite3")).unwrap();
    {
        let db = f.storage.connection.lock().unwrap();
        rusqlite::backup::Backup::new(&db, &mut copy)
            .unwrap()
            .run_to_completion(32, std::time::Duration::from_millis(1), None)
            .unwrap();
    }
    drop(copy);
    let copied = Storage::open(destination.path().into()).unwrap();
    assert!(copied.data_write_blocked());
    assert!(authorize_review(&copied, Some(&p.preview.operation_id)).is_err());
    assert!(Prepared::pending(
        &copied,
        &p.preview.operation_id,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let mut original = p;
    let id = original.preview.confirmation_id.clone();
    assert!(original
        .keep(
            &copied,
            &lease(),
            &id,
            &crate::test_support::never_cancelled
        )
        .is_err());
    assert!(copied.data_write_blocked());
    assert!(f.audio.exists());
    f.unaffected();
}

#[test]
fn directory_request_removes_changed_unknown_files_without_content_reads_or_vacuum() {
    let f = Fixture::new();
    let marker = format!("DIRECTORY-DELETE-PRIVATE-{}", Uuid::new_v4());
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET title=?2 WHERE id=?1",
            params![f.meeting, marker],
        )
        .unwrap();
    let _measurement = crate::operation_observation::Scope::enter("test", |_| {});
    let mut p = Prepared::fresh_directory(&f.storage, &f.meeting, &|| false).unwrap();
    assert_eq!(p.preview().scope_mode, "directory");
    assert_eq!(p.preview().path_count, 1);
    assert_eq!(
        p.preview().target_directory,
        f.audio.parent().unwrap().parent().unwrap()
    );
    std::fs::write(&f.audio, b"changed since confirmation").unwrap();
    std::fs::write(
        f.audio.parent().unwrap().join("unknown.tmp"),
        b"unregistered",
    )
    .unwrap();
    let outside = f.storage.root().join("outside.txt");
    std::fs::write(&outside, b"KEEP OUTSIDE").unwrap();
    std::os::unix::fs::symlink(&outside, f.audio.parent().unwrap().join("external-link")).unwrap();
    let before_pages: i64 = f
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let receipt = p
        .execute(
            &f.storage,
            &lease(),
            &p.preview().confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.deleted);
    assert!(!f.audio.parent().unwrap().parent().unwrap().exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"KEEP OUTSIDE");
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    f.unaffected();
    assert_eq!(crate::operation_observation::measurements(), (0, 0));
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        db.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
            .unwrap()
            >= before_pages
    );
    drop(db);
    for name in ["meetings.sqlite3", "meetings.sqlite3-wal"] {
        let bytes = std::fs::read(f.storage.root().join(name)).unwrap();
        assert!(!bytes
            .windows(marker.len())
            .any(|bytes| bytes == marker.as_bytes()));
    }
}

#[test]
fn directory_request_recovers_at_folder_boundaries_after_cold_reopen() {
    for phase in ["delete_scope_fixed", "deleting_files", "deleting_database"] {
        let f = Fixture::new();
        let mut p = Prepared::fresh_directory(&f.storage, &f.meeting, &|| false).unwrap();
        let stop = Cell::new(false);
        let operation = p.preview().operation_id;
        assert!(p
            .execute(
                &f.storage,
                &lease(),
                &p.preview().confirmation_id,
                false,
                &|| stop.get(),
                &|at| if at == phase {
                    stop.set(true)
                }
            )
            .is_err());
        let root = f.storage.root().to_path_buf();
        let meeting = f.meeting.clone();
        drop(f.storage);
        let storage = Storage::open(root).unwrap();
        let mut p = Prepared::pending(&storage, &operation, &|| false).unwrap();
        assert_eq!(p.preview().scope_mode, "directory");
        assert!(
            p.execute(
                &storage,
                &lease(),
                &p.preview().confirmation_id,
                false,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
        );
        assert!(storage.get_meeting(&meeting).unwrap().is_none());
    }
}

#[test]
fn directory_request_automatically_finishes_an_accepted_unstarted_request() {
    let f = Fixture::new();
    let mut p = Prepared::fresh_directory(&f.storage, &f.meeting, &|| false).unwrap();
    let stop = Cell::new(false);
    let operation = p.preview().operation_id;
    assert!(p
        .execute(
            &f.storage,
            &lease(),
            &p.preview().confirmation_id,
            false,
            &|| stop.get(),
            &|phase| if phase == "delete_scope_fixed" {
                stop.set(true);
            }
        )
        .is_err());
    assert!(super::super::automatic::meeting(&f.storage, &lease(), &operation, &|| false).unwrap());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_none());
    f.unaffected();
}
