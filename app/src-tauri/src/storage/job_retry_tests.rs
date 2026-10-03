use super::*;
use serde_json::json;

fn failed(storage: &Storage, meeting: &str, operation: &str) -> JobSnapshot {
    let job = storage
        .enqueue_job(super::tests::audio_job(meeting, operation))
        .unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    storage
        .save_job_checkpoint(&job.id, 0, 2, json!({"part":2}))
        .unwrap();
    storage
        .finish_job(&job.id, 2, "failed", Some("engine_failed"))
        .unwrap()
}

#[test]
fn retry_is_idempotent_fixed_and_checkpointed_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let meeting = storage.create_meeting("再試行").unwrap();
    let source = failed(&storage, &meeting.id, "first");
    let (spec, checkpoint) = storage.fixed_job_input(&source.id).unwrap();
    let next = storage
        .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "retry")
        .unwrap();
    assert_ne!(next.id, source.id);
    assert_eq!(next.retry_of_job_id.as_deref(), Some(source.id.as_str()));
    assert_eq!(next.attempt, 2);
    assert_eq!(next.completed_units, 2);
    assert_eq!(next.total_units, source.total_units);
    assert_ne!(next.spec_hash, source.spec_hash);
    assert_eq!(
        storage
            .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "retry")
            .unwrap(),
        next
    );
    assert!(storage
        .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "duplicate-retry")
        .is_err());
    let (fixed, inherited) = storage.fixed_job_input(&next.id).unwrap();
    assert_eq!(fixed.parameters, spec.parameters);
    assert_eq!(fixed.profile_snapshot, spec.profile_snapshot);
    assert_eq!(inherited, checkpoint);
    let hashes: Vec<String> = storage
        .connection
        .lock()
        .unwrap()
        .prepare("SELECT checkpoint_hash FROM job_checkpoints ORDER BY created_at, rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_ne!(hashes[0], hashes[1]);
    assert_eq!(storage.get_job(&source.id).unwrap(), source);
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.fixed_job_input(&next.id).unwrap().1, checkpoint);
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, next.id);
    storage
        .finish_job(&next.id, 2, "failed", Some("engine_failed"))
        .unwrap();
    let next = storage.get_job(&next.id).unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&next.id).unwrap();
    let third = storage
        .enqueue_retry_job(&next, &spec, checkpoint.as_ref(), "third")
        .unwrap();
    assert_eq!(third.attempt, 3);
    assert_eq!(third.retry_of_job_id, Some(next.id));
}

#[test]
fn retry_rejects_nonterminal_stale_and_conflicting_operations() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let meeting = storage.create_meeting("競合").unwrap();
    let queued = storage
        .enqueue_job(super::tests::audio_job(&meeting.id, "queued"))
        .unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&queued.id).unwrap();
    assert!(storage
        .enqueue_retry_job(&queued, &spec, checkpoint.as_ref(), "not-terminal")
        .is_err());
    let source = storage
        .finish_job(&queued.id, 0, "cancelled", None)
        .unwrap();
    assert!(storage
        .enqueue_retry_job(&queued, &spec, checkpoint.as_ref(), "stale-state")
        .is_err());
    let mut wrong = source.clone();
    wrong.spec_hash = "f".repeat(64);
    assert!(storage
        .enqueue_retry_job(&wrong, &spec, checkpoint.as_ref(), "stale-hash")
        .is_err());
    let mut wrong_spec = spec.clone();
    wrong_spec.profile_snapshot = json!({"changed":true});
    assert!(storage
        .enqueue_retry_job(&source, &wrong_spec, checkpoint.as_ref(), "changed-profile")
        .is_err());
    let next = storage
        .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "retry")
        .unwrap();
    assert_eq!(next.completed_units, 0);
    assert!(storage.fixed_job_input(&next.id).unwrap().1.is_none());
    let other = storage
        .enqueue_job(super::tests::audio_job(&meeting.id, "other"))
        .unwrap();
    let other = storage.finish_job(&other.id, 0, "cancelled", None).unwrap();
    let (other_spec, other_checkpoint) = storage.fixed_job_input(&other.id).unwrap();
    assert!(storage
        .enqueue_retry_job(&other, &other_spec, other_checkpoint.as_ref(), "retry")
        .is_err());
    assert_eq!(storage.get_job(&source.id).unwrap(), source);
}

#[test]
fn retry_rollback_and_sql_guards_preserve_provenance() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let meeting = storage.create_meeting("原子性").unwrap();
    let source = failed(&storage, &meeting.id, "original");
    let (spec, checkpoint) = storage.fixed_job_input(&source.id).unwrap();
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_retry_checkpoint BEFORE INSERT ON job_checkpoints BEGIN SELECT RAISE(ABORT, 'injected checkpoint failure'); END;").unwrap();
    assert!(storage
        .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "retry")
        .is_err());
    assert_eq!(
        storage.list_jobs(&meeting.id).unwrap(),
        vec![source.clone()]
    );
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_retry_checkpoint;")
        .unwrap();
    let next = storage
        .enqueue_retry_job(&source, &spec, checkpoint.as_ref(), "retry")
        .unwrap();
    let db = storage.connection.lock().unwrap();
    for sql in [
        "UPDATE processing_jobs SET attempt = 9 WHERE id = ?1",
        "UPDATE processing_jobs SET retry_of_job_id = NULL WHERE id = ?1",
        "UPDATE processing_jobs SET total_units = 2 WHERE id = ?1",
    ] {
        assert!(db.execute(sql, params![next.id]).is_err());
    }
    assert!(db.execute("INSERT INTO processing_jobs (id, meeting_id, kind, state, operation_id, retry_of_job_id, attempt, input_hash, profile_snapshot_json, parameters_json, spec_hash, total_units, created_at, updated_at) SELECT 'invalid', meeting_id, kind, 'queued', 'invalid', id, attempt + 1, input_hash, '{}', parameters_json, spec_hash, total_units, 0, 0 FROM processing_jobs WHERE id = ?1", params![source.id]).is_err());
    // Integrity validation also detects tampering after SQL guards are removed.
    db.execute_batch("DROP TRIGGER job_attempt_fixed;").unwrap();
    db.execute(
        "UPDATE processing_jobs SET attempt = 99 WHERE id = ?1",
        params![next.id],
    )
    .unwrap();
    drop(db);
    assert!(storage.fixed_job_input(&next.id).is_err());
    assert_eq!(storage.get_job(&source.id).unwrap(), source);
}
