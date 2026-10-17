//! Owned source/selected bootstrap, new full consent and further real copies.
use super::*;
use crate::storage::deletion::relocation::receiver::{
    synthetic_audio_operation_record as record, synthetic_consent_record as consent,
    synthetic_keep_meeting as keep, synthetic_restored_audio_operation,
};

fn purge(storage: &Storage, meeting: &str) {
    let mut p =
        operations::Prepared::fresh(storage, meeting, &crate::test_support::never_cancelled)
            .unwrap();
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(
        p.execute(
            storage,
            &lease,
            &p.preview().confirmation_id,
            true,
            &crate::test_support::never_cancelled,
            &|_| {}
        )
        .unwrap()
        .deleted
    );
}
fn next(f: &NativeFixture) -> PathBuf {
    let path = f
        ._base
        .path()
        .join(format!("kept-meeting-next-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    path.canonicalize().unwrap()
}
fn verify_kept(storage: &Storage, kept: &str, expected: &str) {
    assert_eq!(record(storage, kept), expected);
    let row: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert!(row[1].is_null());
    assert_eq!(row[2], "delete_meeting");
    assert_eq!(row[3], "recovered");
    assert_eq!(row[8], r#"{"filePhase":"notStarted","kept":true}"#);
    assert!(
        operations::Prepared::pending(storage, kept, &crate::test_support::never_cancelled)
            .is_err()
    );
    assert!(storage
        .create_meeting("UNRELATED MEETING STILL WRITABLE")
        .is_ok());
}

#[test]
fn kept_meeting_owned_history_after_move_full_purge_and_two_further_moves_never_becomes_consent() {
    let mut f = NativeFixture::new(true, false);
    let storage = f.storage.as_ref().unwrap();
    let kept = keep(storage, &f.meeting);
    let before = consent(storage, &kept);
    let audio_before = consent(storage, &f.deletion);
    let old_file = storage.root().join(&f.audio);
    f.select();
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    purge(&storage, &f.meeting);
    assert_eq!(consent(&storage, &kept), before);
    assert_eq!(consent(&storage, &f.deletion), audio_before);
    let expected = record(&storage, &kept);
    let audio = record(&storage, &f.deletion);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        verify_kept(&storage, &kept, &expected);
        assert_eq!(record(&storage, &f.deletion), audio);
        assert_eq!(
            storage
                .create_backup_archive(
                    &f._base.path().join(format!("{}.monbackup", Uuid::new_v4())),
                    &crate::test_support::never_cancelled
                )
                .unwrap()
                .deleted_audio_meeting_count,
            0
        );
        let target = next(&f);
        repeated::select_current(&b, &storage, &target);
        drop(storage);
        drop(b);
        let (b, storage) = f.open();
        assert_eq!(storage.root(), target);
        assert!(!storage.data_write_blocked());
        verify_kept(&storage, &kept, &expected);
        assert_eq!(std::fs::read(&old_file).unwrap(), b"SYNTHETIC AUDIO");
        drop(storage);
        drop(b);
    }
}

#[test]
fn kept_meeting_owned_new_current_keep_with_old_completed_audio_survives_full_purge_and_relocation()
{
    let mut f = NativeFixture::new(false, false);
    let before = consent(f.storage.as_ref().unwrap(), &f.deletion);
    let old = f.storage.as_ref().unwrap().root().join(&f.audio);
    f.select();
    let (b, storage) = f.open();
    let kept = keep(&storage, &f.meeting);
    let kept_before = consent(&storage, &kept);
    std::fs::write(&old, b"OLD ROOT MUST NOT BE PURGED").unwrap();
    purge(&storage, &f.meeting);
    assert_eq!(consent(&storage, &kept), kept_before);
    assert_eq!(consent(&storage, &f.deletion), before);
    let expected = record(&storage, &kept);
    let audio = record(&storage, &f.deletion);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    let target = next(&f);
    repeated::select_current(&b, &storage, &target);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(storage.root(), target);
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        verify_kept(&storage, &kept, &expected);
        assert_eq!(record(&storage, &f.deletion), audio);
        assert_eq!(std::fs::read(&old).unwrap(), b"OLD ROOT MUST NOT BE PURGED");
        drop(storage);
        drop(b);
    }
}

#[test]
fn kept_meeting_owned_restored_audio_baseline_and_kept_meeting_survive_purge_and_repeat_move() {
    let mut f = NativeFixture::new(false, false);
    let storage = f.storage.as_ref().unwrap();
    let archive = f._base.path().join("kept-meeting-restore.monbackup");
    storage
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let prepared = storage
        .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let meeting = prepared
        .publish(
            storage,
            &prepared.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .meeting_ids[0]
        .clone();
    drop(prepared);
    let baseline = synthetic_restored_audio_operation(storage, &meeting);
    let baseline_before = consent(storage, &baseline);
    let kept = keep(storage, &meeting);
    let before = consent(storage, &kept);
    let old_file = storage
        .root()
        .join("meetings")
        .join(&meeting)
        .join("old-marker.txt");
    std::fs::write(&old_file, b"KEEP OLD RESTORED ROOT").unwrap();
    f.select();
    let (b, storage) = f.open();
    purge(&storage, &meeting);
    assert_eq!(consent(&storage, &kept), before);
    assert_eq!(consent(&storage, &baseline), baseline_before);
    let expected = record(&storage, &kept);
    let restored = record(&storage, &baseline);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    let target = next(&f);
    repeated::select_current(&b, &storage, &target);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(storage.root(), target);
        assert!(storage.get_meeting(&meeting).unwrap().is_none());
        verify_kept(&storage, &kept, &expected);
        assert_eq!(record(&storage, &baseline), restored);
        assert_eq!(std::fs::read(&old_file).unwrap(), b"KEEP OLD RESTORED ROOT");
        assert_eq!(
            storage
                .create_backup_archive(
                    &f._base.path().join(format!("{}.monbackup", Uuid::new_v4())),
                    &crate::test_support::never_cancelled
                )
                .unwrap()
                .deleted_audio_meeting_count,
            1
        );
        drop(storage);
        drop(b);
    }
}
