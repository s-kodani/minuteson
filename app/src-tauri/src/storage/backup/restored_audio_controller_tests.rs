//! Actual native restored history through shared selection, requests and receipts.
use super::*;
use crate::data_controller::{restored_audio_synthetic::Synthetic, Action};
use crate::storage::deletion::audio::operations;

fn fixed(storage: &Storage, op: &str) -> String {
    storage.connection.lock().unwrap().query_row("SELECT json_array(rowid,meeting_id,kind,state,restore_batch_id,target_manifest_json,target_hash,consent_at,progress_json,error_code,created_at,updated_at,(SELECT count(*) FROM maintenance_paths WHERE operation_id=?1)) FROM maintenance_operations WHERE id=?1", [op], |r|r.get(0)).unwrap()
}
fn count(storage: &Storage) -> i64 {
    storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn restored_audio_controller_readonly_baseline_has_no_consent_and_requires_new_operation() {
    let (_root, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let before = fixed(&storage, &original);
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    let p = s.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "restored_unavailable");
    assert!(
        !p.execute_allowed
            && p.reconfirmation_allowed
            && !p.prior_deletion_completed
            && p.include_unregistered_recorded.is_none()
    );
    assert_eq!(c.page(&s, 0).total, p.path_count);
    for action in [
        Action::Publish,
        Action::DeleteIncludingUnregistered,
        Action::Keep,
    ] {
        assert!(c.denied(&s, action));
    }
    assert_eq!(count(&storage), 1);
    let request = Synthetic::request(&s, Action::ReviewRemaining);
    let reviewed = c.run(&storage, &s, Action::ReviewRemaining);
    let p = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "reconfirmation");
    assert_eq!(
        p.reconfirmation_source_kind.as_deref(),
        Some("restored_audio_unavailable")
    );
    assert!(!p.prior_deletion_completed && p.execute_allowed);
    assert_ne!(p.operation_id, original);
    assert_eq!(p.source_operation_id.as_deref(), Some(original.as_str()));
    assert_eq!(c.admission(), original);
    assert!(c.replay(request));
    assert_eq!(count(&storage), 1);
    let request = Synthetic::request(&reviewed, Action::Publish);
    let done = c.run(&storage, &reviewed, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(done.audio_deletion_receipt.as_ref().unwrap().audio_deleted);
    assert!(c.replay(request));
    assert_eq!(count(&storage), 2);
    assert_eq!(fixed(&storage, &original), before);
    assert!(!path.exists());
}

#[test]
fn restored_audio_controller_fresh_meeting_never_silently_mints_a_v1_consent() {
    let (_root, storage, meeting, original) = native_fixture();
    let c = Synthetic::new(&storage);
    let s = c.fresh(&storage, &meeting);
    let p = s.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(
        (p.operation_id.as_str(), p.phase.as_str()),
        (original.as_str(), "restored_unavailable")
    );
    assert!(c.denied(&s, Action::Publish) && count(&storage) == 1);
    let again = c.run(&storage, &s, Action::Refresh);
    assert!(
        !again
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .execute_allowed
    );
    assert_eq!(count(&storage), 1);
}

#[test]
fn restored_audio_controller_saved_new_consent_resumes_after_cold_restart_with_same_policy() {
    let (_root, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let before = fixed(&storage, &original);
    let root = storage.root.clone();
    drop(storage);
    let storage = Storage::open(root.clone()).unwrap();
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    let reviewed = c.run(&storage, &s, Action::ReviewRemaining);
    let stopped = c.stop_at(
        &storage,
        &reviewed,
        Action::Publish,
        "audio_delete_scope_fixed",
    );
    let p = stopped.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "not_started");
    assert!(p.keep_allowed && !p.execute_allowed && stopped.attention_required);
    assert_eq!(c.admission(), p.operation_id);
    let operation = p.operation_id.clone();
    drop(c);
    drop(storage);
    let storage = Storage::open(root.clone()).unwrap();
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &operation).unwrap();
    assert_eq!(
        s.audio_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(false)
    );
    assert!(c.denied(&s, Action::DeleteIncludingUnregistered));
    let done = c.run(&storage, &s, Action::Publish);
    assert!(done.restart_required && done.audio_deletion_receipt.as_ref().unwrap().audio_deleted);
    assert_eq!(fixed(&storage, &original), before);
    assert!(storage.data_write_blocked() && !path.exists());
    drop(c);
    drop(storage);
    assert!(!Storage::open(root).unwrap().data_write_blocked());
}

#[test]
fn restored_audio_controller_keep_new_unstarted_scope_preserves_original_attention_and_body() {
    let (_root, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    std::fs::write(&path, b"KEEP").unwrap();
    let root = storage.root.clone();
    drop(storage);
    let storage = Storage::open(root).unwrap();
    let before = fixed(&storage, &original);
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    let reviewed = c.run(&storage, &s, Action::ReviewRemaining);
    let stopped = c.stop_at(
        &storage,
        &reviewed,
        Action::Publish,
        "audio_delete_scope_fixed",
    );
    let done = c.run(&storage, &stopped, Action::Keep);
    assert!(done.audio_deletion_receipt.unwrap().retained && !done.restart_required);
    // Unresolved baseline attention is still pending; the restart-only phase
    // follows resolution of all hints, not merely keeping a new child.
    assert!(storage.deletion_resume_restart_required());
    assert!(storage.data_write_blocked() && path.exists());
    let listing = operations::listing(&storage).unwrap();
    assert!(listing.items.iter().any(|i| i.operation_id == original
        && i.review_allowed
        && i.startup_reason.as_deref() == Some("restored_audio_scope_reappeared")));
    assert_eq!(fixed(&storage, &original), before);
}

#[test]
fn restored_audio_controller_changed_new_scope_stays_fenced_and_requires_explicit_refresh() {
    let (_root, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    std::fs::write(&path, b"RETURNED").unwrap();
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    let reviewed = c.run(&storage, &s, Action::ReviewRemaining);
    std::fs::write(&path, b"CHANGED AFTER REVIEW").unwrap();
    let failed = c.run(&storage, &reviewed, Action::Publish);
    assert!(failed.attention_required && failed.audio_deletion_receipt.is_none());
    assert_eq!(count(&storage), 1);
    assert!(c.denied(&failed, Action::Publish));
    let refreshed = c.run(&storage, &failed, Action::Refresh);
    assert!(
        refreshed
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .execute_allowed
    );
    let done = c.run(&storage, &refreshed, Action::Publish);
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted && !path.exists());
}

#[test]
fn restored_audio_controller_uninspectable_cold_scope_keeps_attention_and_deferred_recovery_without_preview(
) {
    let (base, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    let outside = base.path().join("unrelated");
    std::fs::write(&outside, b"KEEP OUTSIDE").unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let root = storage.root.clone();
    drop(storage);
    let storage = Storage::open(root).unwrap();
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    assert!(s.attention_required && !s.restart_required && s.audio_deletion_preview.is_none());
    assert!(storage.deletion_resume_restart_required());
    assert_eq!(s.phase, "attention");
    assert!(storage.data_write_blocked() && path.is_symlink());
    assert_eq!(std::fs::read(outside).unwrap(), b"KEEP OUTSIDE");
}

#[test]
fn restored_audio_controller_invalid_baseline_cannot_authorize_review_or_new_consent() {
    let (_root, storage, _meeting, original) = native_fixture();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE maintenance_operations SET updated_at=updated_at+1 WHERE id=?1",
            [&original],
        )
        .unwrap();
    let root = storage.root.clone();
    drop(storage);
    let storage = Storage::open(root).unwrap();
    let listing = operations::listing(&storage).unwrap();
    assert!(
        listing.write_blocked
            && listing.items.iter().any(|i| i.operation_id == original
                && !i.review_allowed
                && i.meeting_id.is_none()
                && i.error_message.is_some())
    );
    assert!(Synthetic::new(&storage)
        .pending(&storage, &original)
        .is_err());
    assert_eq!(count(&storage), 1);
}

#[test]
fn restored_audio_controller_native_descendant_reports_native_source_and_requires_unregistered_choice(
) {
    let (_root, storage, meeting, original) = native_fixture();
    let path = audio_path(&storage, &meeting);
    std::fs::write(&path, b"FIRST").unwrap();
    let before = fixed(&storage, &original);
    let c = Synthetic::new(&storage);
    let s = c.pending(&storage, &original).unwrap();
    let s = c.run(&storage, &s, Action::ReviewRemaining);
    let done = c.run(&storage, &s, Action::Publish);
    let operation = done.audio_deletion_receipt.unwrap().operation_id;
    let reserved = path.with_file_name(format!(".mon-delete-audio-{operation}-0"));
    std::fs::write(&reserved, b"SECOND").unwrap();
    let s = c.pending(&storage, &operation).unwrap();
    let s = c.run(&storage, &s, Action::ReviewRemaining);
    let p = s.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(
        p.reconfirmation_source_kind.as_deref(),
        Some("native_audio_deletion")
    );
    assert!(p.prior_deletion_completed && p.unregistered_audio_files > 0);
    assert!(c.denied(&s, Action::Publish));
    let done = c.run(&storage, &s, Action::DeleteIncludingUnregistered);
    assert!(done.audio_deletion_receipt.unwrap().audio_deleted && !reserved.exists());
    assert_eq!(fixed(&storage, &original), before);
}
