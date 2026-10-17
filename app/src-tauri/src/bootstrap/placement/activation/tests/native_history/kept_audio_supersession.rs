//! Real selected owners, full purge and further owned copies. No public cutover.
use super::*;
use crate::storage::deletion::relocation::receiver::{
    synthetic_audio_operation_record as record, synthetic_consent_record as consent,
    synthetic_restored_audio_operation, synthetic_restored_audio_paths,
};
use audio::intent::{self, reconfirmation};

fn policy() -> intent::Policy {
    intent::Policy {
        include_unregistered_audio: true,
        retain_unclassified: true,
    }
}
fn keep_new(storage: &Storage, original: &str) -> String {
    let lease = AnalysisController::default()
        .begin_audio_deletion_maintenance(
            storage,
            original,
            false,
            false,
            &crate::test_support::never_cancelled,
        )
        .unwrap();
    let review = reconfirmation::Review::capture(
        storage,
        original,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let id = Uuid::new_v4().to_string();
    reconfirmation::accept(
        storage,
        &id,
        &review,
        policy(),
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::keep_unstarted(storage, &id, &lease, &crate::test_support::never_cancelled).unwrap();
    id
}
fn purge(storage: &Storage, meeting: &str) -> String {
    let mut prepared =
        operations::Prepared::fresh(storage, meeting, &crate::test_support::never_cancelled)
            .unwrap();
    let id = prepared.preview().operation_id.clone();
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(
        prepared
            .execute(
                storage,
                &lease,
                &prepared.preview().confirmation_id,
                true,
                &crate::test_support::never_cancelled,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    id
}
fn next(f: &NativeFixture) -> PathBuf {
    let path = f._base.path().join(format!("kept-next-{}", Uuid::new_v4()));
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
    assert_eq!(row[3], "recovered");
    assert_eq!(row[8], r#"{"version":1,"phase":"audioKeptUnstarted"}"#);
    assert!(AnalysisController::default()
        .begin_audio_deletion_maintenance(
            storage,
            kept,
            false,
            false,
            &crate::test_support::never_cancelled
        )
        .is_err());
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(
        intent::removal::start(storage, kept, &lease, &crate::test_support::never_cancelled)
            .is_err()
    );
    assert!(reconfirmation::Review::capture(
        storage,
        kept,
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn kept_audio_owned_purge_after_move_preserves_old_root_and_survives_two_further_moves() {
    let mut f = NativeFixture::new(true, false);
    let original = consent(f.storage.as_ref().unwrap(), &f.deletion);
    let old_file = f.storage.as_ref().unwrap().root().join(&f.audio);
    f.select();
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(consent(&storage, &f.deletion), original);
    let full = purge(&storage, &f.meeting);
    assert_eq!(consent(&storage, &f.deletion), original);
    assert_eq!(std::fs::read(&old_file).unwrap(), b"SYNTHETIC AUDIO");
    let expected = record(&storage, &f.deletion);
    let full_record = record(&storage, &full);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        verify_kept(&storage, &f.deletion, &expected);
        assert_eq!(record(&storage, &full), full_record);
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
        verify_kept(&storage, &f.deletion, &expected);
        assert_eq!(std::fs::read(&old_file).unwrap(), b"SYNTHETIC AUDIO");
        drop(storage);
        drop(b);
    }
}

#[test]
fn kept_audio_owned_v2_child_after_move_purges_without_adopting_old_ancestor_as_current_consent() {
    let mut f = NativeFixture::new(false, false);
    let original = consent(f.storage.as_ref().unwrap(), &f.deletion);
    let former = f.storage.as_ref().unwrap().root().to_owned();
    f.select();
    let (b, storage) = f.open();
    let current_file = storage.root().join(&f.audio);
    std::fs::write(&current_file, b"RETURNED CURRENT ROOT AUDIO").unwrap();
    let kept = keep_new(&storage, &f.deletion);
    let kept_consent = consent(&storage, &kept);
    let old_file = former.join(&f.audio);
    std::fs::write(&old_file, b"OLD ROOT MUST NOT BE PURGED").unwrap();
    purge(&storage, &f.meeting);
    assert!(!current_file.exists());
    assert_eq!(consent(&storage, &kept), kept_consent);
    assert_eq!(consent(&storage, &f.deletion), original);
    let expected = record(&storage, &kept);
    let parent = record(&storage, &f.deletion);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    verify_kept(&storage, &kept, &expected);
    let target = next(&f);
    repeated::select_current(&b, &storage, &target);
    drop(storage);
    drop(b);
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert_eq!(storage.root(), target);
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
        verify_kept(&storage, &kept, &expected);
        assert_eq!(record(&storage, &f.deletion), parent);
        assert_eq!(
            std::fs::read(&old_file).unwrap(),
            b"OLD ROOT MUST NOT BE PURGED"
        );
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
        drop(storage);
        drop(b);
    }
}

#[test]
fn kept_audio_owned_restored_baseline_and_kept_child_survive_purge_cold_start_and_relocation() {
    let mut f = NativeFixture::new(false, false);
    let storage = f.storage.as_ref().unwrap();
    let archive = f._base.path().join("kept-restored-source.monbackup");
    storage
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    let selected = storage
        .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let meeting = selected
        .publish(
            storage,
            &selected.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap()
        .meeting_ids[0]
        .clone();
    drop(selected);
    let baseline = synthetic_restored_audio_operation(storage, &meeting);
    let baseline_consent = consent(storage, &baseline);
    let relative = synthetic_restored_audio_paths(storage, &baseline)[0]
        .strip_prefix(storage.root())
        .unwrap()
        .to_path_buf();
    let former = storage.root().to_owned();
    f.select();
    let (b, storage) = f.open();
    let file = storage.root().join(&relative);
    std::fs::write(&file, b"RETURNED RESTORED AUDIO KEPT WITHOUT STARTING").unwrap();
    let kept = keep_new(&storage, &baseline);
    let kept_consent = consent(&storage, &kept);
    let old_file = former.join(&relative);
    std::fs::write(&old_file, b"KEEP OLD RESTORED ROOT").unwrap();
    purge(&storage, &meeting);
    assert!(!file.exists());
    assert_eq!(consent(&storage, &kept), kept_consent);
    assert_eq!(consent(&storage, &baseline), baseline_consent);
    let expected = record(&storage, &kept);
    let parent = record(&storage, &baseline);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
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
        assert_eq!(record(&storage, &baseline), parent);
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
