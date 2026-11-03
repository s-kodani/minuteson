//! Real additive restore, owned copies and cold bootstrap, not fabricated receipts.
//! Production cutover and owned admission never turn a baseline into consent.
use super::*;
use crate::data_controller::{restored_audio_synthetic::Synthetic, Action};
use crate::storage::deletion::relocation::receiver::{
    synthetic_audio_operation_record as record, synthetic_restored_audio_operation,
    synthetic_restored_audio_paths,
};

struct Restored {
    fixture: NativeFixture,
    meeting: String,
    original: String,
    before: String,
}
impl Restored {
    fn new() -> Self {
        let mut fixture = NativeFixture::new(false, false);
        let storage = fixture.storage.as_ref().unwrap();
        let current = crate::storage::backup::deleted_current_meeting_for_test(
            storage,
            "restored-current-audio",
        );
        let title = storage.get_meeting(&current).unwrap().unwrap().title;
        let archive = fixture._base.path().join("restored-input.monbackup");
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
            .meeting_ids
            .into_iter()
            .find(|id| storage.get_meeting(id).unwrap().unwrap().title == title)
            .unwrap();
        drop(selected);
        let original = synthetic_restored_audio_operation(storage, &meeting);
        let before = record(storage, &original);
        fixture.select();
        // Approve the exact owned copy before injecting legitimate later content.
        let (b, storage) = fixture.open();
        assert!(!storage.data_write_blocked());
        assert_eq!(record(&storage, &original), before);
        drop(storage);
        drop(b);
        Self {
            fixture,
            meeting,
            original,
            before,
        }
    }
    fn path(&self, storage: &Storage) -> PathBuf {
        let paths = synthetic_restored_audio_paths(storage, &self.original);
        assert_eq!(paths.len(), 1);
        paths[0].clone()
    }
    fn returned(&self) -> PathBuf {
        let (b, storage) = self.fixture.open();
        let file = self.path(&storage);
        assert!(!file.exists());
        drop(storage);
        drop(b);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"RETURNED RESTORED AUDIO").unwrap();
        file
    }
    fn unchanged(&self, storage: &Storage) {
        assert_eq!(record(storage, &self.original), self.before);
        assert!(storage.get_meeting(&self.meeting).unwrap().is_some());
        assert!(storage.recorded_audio_plan(&self.meeting).is_err());
    }
}

#[test]
fn restored_audio_reconfirmation_after_owned_move_requires_new_controller_consent() {
    let f = Restored::new();
    let file = f.returned();
    let retained = file.parent().unwrap().join("retained.txt");
    std::fs::write(&retained, b"KEEP RESTORED TEXT").unwrap();
    let (b, storage) = f.fixture.open();
    assert!(storage.data_write_blocked());
    let c = Synthetic::new(&storage);
    let baseline = c.pending(&storage, &f.original).unwrap();
    let p = baseline.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "restored_unavailable");
    assert!(!p.execute_allowed && !p.prior_deletion_completed);
    for action in [
        Action::Publish,
        Action::DeleteIncludingUnregistered,
        Action::Keep,
    ] {
        assert!(c.denied(&baseline, action));
    }
    let reviewed = c.run(&storage, &baseline, Action::ReviewRemaining);
    let p = reviewed.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(
        p.reconfirmation_source_kind.as_deref(),
        Some("restored_audio_unavailable")
    );
    assert_eq!(p.source_operation_id.as_deref(), Some(f.original.as_str()));
    assert_ne!(p.operation_id, f.original);
    assert!(!p.prior_deletion_completed);
    let done = c.run(&storage, &reviewed, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(done.restart_required && done.audio_deletion_receipt.as_ref().unwrap().audio_deleted);
    assert!(!file.exists());
    assert_eq!(std::fs::read(&retained).unwrap(), b"KEEP RESTORED TEXT");
    f.unchanged(&storage);
    drop(c);
    drop(storage);
    drop(b);
    let (_, storage) = f.fixture.open();
    assert!(!storage.data_write_blocked());
    f.unchanged(&storage);
}

#[test]
fn restored_audio_reconfirmation_after_owned_move_cold_resumes_exact_saved_new_policy() {
    let f = Restored::new();
    let file = f.returned();
    let (b, storage) = f.fixture.open();
    let c = Synthetic::new(&storage);
    let baseline = c.pending(&storage, &f.original).unwrap();
    let reviewed = c.run(&storage, &baseline, Action::ReviewRemaining);
    let stopped = c.stop_at(
        &storage,
        &reviewed,
        Action::Publish,
        "audio_delete_scope_fixed",
    );
    let p = stopped.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.phase, "not_started");
    let operation = p.operation_id.clone();
    assert_ne!(operation, f.original);
    assert_eq!(p.include_unregistered_recorded, Some(false));
    assert!(file.exists());
    f.unchanged(&storage);
    drop(c);
    drop(storage);
    drop(b);
    let (b, storage) = f.fixture.open();
    let c = Synthetic::new(&storage);
    let saved = c.pending(&storage, &operation).unwrap();
    assert_eq!(
        saved
            .audio_deletion_preview
            .as_ref()
            .unwrap()
            .include_unregistered_recorded,
        Some(false)
    );
    assert!(c.denied(&saved, Action::DeleteIncludingUnregistered));
    let done = c.run(&storage, &saved, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(!file.exists() && done.restart_required);
    f.unchanged(&storage);
    drop(c);
    drop(storage);
    drop(b);
    let (_, storage) = f.fixture.open();
    assert!(!storage.data_write_blocked());
    f.unchanged(&storage);
}

#[test]
fn restored_audio_reconfirmation_mixed_chain_survives_two_further_moves_without_old_root_permission(
) {
    let f = Restored::new();
    let mut previous = f.original.clone();
    let mut current = f.fixture.target.clone();
    let mut old_files = Vec::new();
    for _ in 0..3 {
        let file = f.returned();
        let (b, storage) = f.fixture.open();
        assert_eq!(storage.root(), current);
        let c = Synthetic::new(&storage);
        let baseline = c.pending(&storage, &previous).unwrap();
        let reviewed = c.run(&storage, &baseline, Action::ReviewRemaining);
        let p = reviewed.audio_deletion_preview.as_ref().unwrap();
        assert_eq!(p.source_operation_id.as_deref(), Some(previous.as_str()));
        assert_eq!(p.prior_deletion_completed, previous != f.original);
        previous = p.operation_id.clone();
        let done = c.run(&storage, &reviewed, Action::Publish);
        assert_eq!(done.phase, "completed", "{:?}", done.error_message);
        assert!(!file.exists());
        for old in &old_files {
            assert_eq!(std::fs::read(old).unwrap(), b"KEEP OLD ROOT AUDIO");
        }
        f.unchanged(&storage);
        drop(c);
        drop(storage);
        drop(b);
        if old_files.len() == 2 {
            break;
        }
        let (b, storage) = f.fixture.open();
        assert!(!storage.data_write_blocked());
        let next = f
            .fixture
            ._base
            .path()
            .join(format!("restored-move-{}", Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&next)
            .unwrap();
        let next = next.canonicalize().unwrap();
        repeated::select_current(&b, &storage, &next);
        drop(storage);
        drop(b);
        // Cold admission validates exact installed copy before any reappearance.
        let (b, storage) = f.fixture.open();
        assert_eq!(storage.root(), next);
        assert!(!storage.data_write_blocked());
        f.unchanged(&storage);
        drop(storage);
        drop(b);
        // Returned data in the retired root must never become removal authority.
        std::fs::write(&file, b"KEEP OLD ROOT AUDIO").unwrap();
        old_files.push(file);
        current = next;
    }
    let (_, storage) = f.fixture.open();
    assert_eq!(storage.root(), current);
    assert!(!storage.data_write_blocked());
    f.unchanged(&storage);
    for file in old_files {
        assert_eq!(std::fs::read(file).unwrap(), b"KEEP OLD ROOT AUDIO");
    }
}

#[test]
fn restored_audio_reconfirmation_after_owned_move_refuses_link_without_external_deletion() {
    let f = Restored::new();
    let (b, storage) = f.fixture.open();
    let file = f.path(&storage);
    drop(storage);
    drop(b);
    let external = f.fixture._base.path().join("external-restored-audio");
    std::fs::write(&external, b"KEEP EXTERNAL AUDIO").unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    symlink(&external, &file).unwrap();
    let (_, storage) = f.fixture.open();
    assert!(storage.data_write_blocked());
    let c = Synthetic::new(&storage);
    let result = c.pending(&storage, &f.original).unwrap();
    assert!(result.attention_required && result.audio_deletion_preview.is_none());
    assert_eq!(std::fs::read(external).unwrap(), b"KEEP EXTERNAL AUDIO");
    assert!(file.is_symlink());
    f.unchanged(&storage);
}

#[test]
#[ignore = "child only; real owned restored fixture is supplied by parent"]
fn restored_audio_reconfirmation_owned_move_crash_child() {
    let config = PathBuf::from(std::env::var("MON_RESTORED_AUDIO_CONFIG").unwrap());
    let legacy = PathBuf::from(std::env::var("MON_RESTORED_AUDIO_LEGACY").unwrap());
    let original = std::env::var("MON_RESTORED_AUDIO_ORIGINAL").unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    let c = Synthetic::new(&storage);
    let baseline = c.pending(&storage, &original).unwrap();
    let reviewed = c.run(&storage, &baseline, Action::ReviewRemaining);
    c.run_observed(
        &storage,
        &reviewed,
        Action::Publish,
        &crate::test_support::never_cancelled,
        &|phase| {
            if phase == "audio_delete_scope_fixed" {
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGKILL);
                }
                panic!("synthetic child kill failed");
            }
        },
    );
    panic!("synthetic child did not reach saved-consent boundary");
}

#[test]
fn restored_audio_reconfirmation_owned_move_sigkill_after_consent_resumes_without_new_policy() {
    use std::os::unix::process::ExitStatusExt;
    let f = Restored::new();
    let file = f.returned();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "bootstrap::placement::activation::tests::native_history::restored_audio_reconfirmation::restored_audio_reconfirmation_owned_move_crash_child"])
        .env("MON_RESTORED_AUDIO_CONFIG", &f.fixture.config)
        .env("MON_RESTORED_AUDIO_LEGACY", &f.fixture.legacy)
        .env("MON_RESTORED_AUDIO_ORIGINAL", &f.original)
        .status().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(file.exists());
    let (b, storage) = f.fixture.open();
    assert!(storage.data_write_blocked());
    f.unchanged(&storage);
    let list = audio::operations::listing(&storage).unwrap();
    let children: Vec<_> = list
        .items
        .iter()
        .filter(|i| i.state == "prepared")
        .collect();
    assert_eq!(children.len(), 1);
    assert!(children[0].review_allowed && children[0].error_message.is_none());
    let operation = children[0].operation_id.clone();
    assert_ne!(operation, f.original);
    let c = Synthetic::new(&storage);
    let saved = c.pending(&storage, &operation).unwrap();
    let p = saved.audio_deletion_preview.as_ref().unwrap();
    assert_eq!(p.source_operation_id.as_deref(), Some(f.original.as_str()));
    assert_eq!(p.include_unregistered_recorded, Some(false));
    assert!(c.denied(&saved, Action::DeleteIncludingUnregistered));
    let done = c.run(&storage, &saved, Action::Publish);
    assert_eq!(done.phase, "completed", "{:?}", done.error_message);
    assert!(done.restart_required && !file.exists());
    f.unchanged(&storage);
    drop(c);
    drop(storage);
    drop(b);
    let (_, storage) = f.fixture.open();
    assert!(!storage.data_write_blocked());
    f.unchanged(&storage);
}
