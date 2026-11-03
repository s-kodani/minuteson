//! Real additive-restore entry points with owned storage and synthetic audio.
use super::*;
use crate::storage::backup::{
    ledger::BatchPlan,
    publication::{self, Publication},
    restore_fixed::{workspace::seal::SealedWorkspace, FixedData},
    restore_ids::RestoreIds,
};
use crate::storage::deletion::audio::restored;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn native_record(target: &Storage, meeting: &str) -> String {
    let db = target.connection.lock().unwrap();
    let op: String = db.query_row("SELECT id FROM maintenance_operations WHERE meeting_id=?1 AND kind='delete_assets' AND state='completed'", [meeting], |r|r.get(0)).unwrap();
    assert!(restored::deleted(&db, &op, meeting).is_ok());
    assert_eq!(
        restored::inspect_one(&db, &target.managed_anchor, &op).unwrap(),
        None
    );
    assert_eq!(db.query_row("SELECT count(*) FROM restore_omitted_assets WHERE meeting_id=?1 AND reason='recreatable_pcm'", [meeting], |r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM maintenance_paths WHERE operation_id=?1",
            [&op],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    op
}

#[test]
fn deleted_audio_publication_commit_failure_rolls_back_unavailable_marker_and_preserves_reservation_for_retry(
) {
    let (root, source, meeting) = fixture();
    deleted(&source, &meeting);
    let output = tempfile::tempdir().unwrap();
    let (archive, _) = write(&source, output.path());
    let ids = RestoreIds::prepare(
        &archive,
        output.path(),
        &Uuid::new_v4().to_string(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let target = Storage::open(root.path().join("commit-target")).unwrap();
    let existing = tests::meeting(&target, "keep on commit failure");
    let before = target.recorded_audio_plan(&existing).unwrap();
    {
        let mut db = target.connection.lock().unwrap();
        let mut tx = db.transaction().unwrap();
        BatchPlan::new(&ids, &crate::test_support::never_cancelled)
            .unwrap()
            .reserve(&mut tx, &crate::test_support::never_cancelled)
            .unwrap();
        tx.commit().unwrap();
    }
    let ready = sealed
        .connection()
        .prepare("SELECT relative_path,size_bytes FROM file_assets WHERE state='ready'")
        .unwrap()
        .query_map([], |r| {
            Ok((
                target.root.join(r.get::<_, String>(0)?),
                r.get::<_, i64>(1)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!ready.is_empty());
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    target
        .connection
        .lock()
        .unwrap()
        .commit_hook(Some(move || {
            if ready
                .iter()
                .all(|(p, n)| std::fs::metadata(p).is_ok_and(|m| m.len() == *n as u64))
            {
                seen.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        }))
        .unwrap();
    assert!(target
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    target
        .connection
        .lock()
        .unwrap()
        .commit_hook(None::<fn() -> bool>)
        .unwrap();
    let new = ids.get("meetings", "", &meeting).unwrap();
    assert!(target.get_meeting(&new).unwrap().is_none());
    assert!(!target.root.join("meetings").join(&new).exists());
    assert_eq!(target.recorded_audio_plan(&existing).unwrap(), before);
    assert_eq!(
        target
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM maintenance_operations WHERE kind='delete_assets'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        target
            .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
            .unwrap(),
        Publication::Published { .. }
    ));
    native_record(&target, &new);
}
