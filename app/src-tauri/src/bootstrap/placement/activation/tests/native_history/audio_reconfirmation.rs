//! Actual cold Bootstrap ownership; the public native-history cutover gate stays closed.
use super::*;
use audio::intent::{self, reconfirmation};
use std::os::unix::fs::MetadataExt;

fn policy() -> intent::Policy {
    intent::Policy {
        include_unregistered_audio: true,
        retain_unclassified: true,
    }
}
fn lease(
    storage: &Storage,
    operation: &str,
) -> crate::analysis_worker::data_maintenance::DataMaintenanceLease {
    AnalysisController::default()
        .begin_audio_deletion_maintenance(storage, operation, false, false, &|| false)
        .unwrap()
}
fn returned(f: &NativeFixture, current: &Path) -> PathBuf {
    let file = current.join(&f.audio);
    std::fs::write(&file, b"REAPPEARED NATIVE AUDIO").unwrap();
    file
}
fn consent(storage: &Storage, original: &str) -> String {
    let owner = lease(storage, original);
    assert!(intent::removal::run(storage, original, &owner, &|| false).is_err());
    let review = reconfirmation::Review::capture(storage, original, &owner, &|| false).unwrap();
    assert!(
        !review
            .preview(&Uuid::new_v4().to_string())
            .unwrap()
            .execute_allowed
    );
    let operation = Uuid::new_v4().to_string();
    reconfirmation::accept(storage, &operation, &review, policy(), &owner, &|| false).unwrap();
    intent::removal::start(storage, &operation, &owner, &|| false).unwrap();
    intent::removal::run(storage, &operation, &owner, &|| false).unwrap();
    assert!(
        intent::removal::completion::finish(storage, &operation, &owner, &|| false)
            .unwrap()
            .completed()
    );
    operation
}

#[test]
fn audio_reconfirmation_after_owned_move_records_current_identity_and_keeps_original_receipt() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    let original = f.live_record(&storage);
    drop(storage);
    drop(b);
    let file = returned(&f, &f.target);
    let retained = file.parent().unwrap().join("retained.txt");
    let (b, storage) = f.open();
    assert!(storage.data_write_blocked());
    let operation = consent(&storage, &f.deletion);
    assert_eq!(f.live_record(&storage), original);
    assert!(!file.exists() && retained.exists());
    assert!(storage.deletion_resume_restart_required());
    let (header, snapshot) = reconfirmation::synthetic_saved(&storage, &operation);
    let header: serde_json::Value = serde_json::from_str(&header).unwrap();
    assert_eq!(header["version"], 2);
    assert_eq!(header["origin"]["operationId"], f.deletion);
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    for (field, path) in [
        ("root", storage.root().to_owned()),
        ("database", storage.root().join("meetings.sqlite3")),
    ] {
        let metadata = std::fs::metadata(path).unwrap();
        assert_eq!(
            snapshot["manifest"][field]["device"].as_u64(),
            Some(metadata.dev())
        );
        assert_eq!(
            snapshot["manifest"][field]["inode"].as_u64(),
            Some(metadata.ino())
        );
    }
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(f.live_record(&storage), original);
    assert!(storage.recorded_audio_plan(&f.meeting).is_err());
    drop(storage);
    drop(b);
}

#[test]
fn audio_reconfirmation_after_move_accepts_then_cold_resumes_same_current_plan() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    let (b, storage) = f.open();
    drop(storage);
    drop(b);
    let file = returned(&f, &f.target);
    let (b, storage) = f.open();
    let original = f.live_record(&storage);
    let owner = lease(&storage, &f.deletion);
    let review = reconfirmation::Review::capture(&storage, &f.deletion, &owner, &|| false).unwrap();
    let operation = Uuid::new_v4().to_string();
    reconfirmation::accept(&storage, &operation, &review, policy(), &owner, &|| false).unwrap();
    assert!(file.exists());
    drop(owner);
    drop(storage);
    drop(b);
    let (b, storage) = f.open();
    let owner = lease(&storage, &operation);
    intent::removal::start(&storage, &operation, &owner, &|| false).unwrap();
    intent::removal::run(&storage, &operation, &owner, &|| false).unwrap();
    intent::removal::completion::finish(&storage, &operation, &owner, &|| false).unwrap();
    assert!(!file.exists());
    assert_eq!(f.live_record(&storage), original);
    drop(owner);
    drop(storage);
    drop(b);
    let (_, storage) = f.open();
    assert!(!storage.data_write_blocked());
}

#[test]
fn audio_reconfirmation_survives_further_owned_moves_and_does_not_move_old_file_permission() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    let (b, storage) = f.open();
    let original = f.live_record(&storage);
    drop(storage);
    drop(b);
    let mut current = f.target.clone();
    let mut previous = f.deletion.clone();
    for _ in 0..2 {
        let file = returned(&f, &current);
        let (b, storage) = f.open();
        previous = consent(&storage, &previous);
        assert!(!file.exists());
        assert_eq!(f.live_record(&storage), original);
        drop(storage);
        drop(b);
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        let next = f
            ._base
            .path()
            .join(format!("additional-{}", Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&next)
            .unwrap();
        let next = next.canonicalize().unwrap();
        repeated::select_current(&b, &storage, &next);
        drop(storage);
        drop(b);
        current = next;
        // Accept the exact installed copy before any legitimate new content.
        // Injecting a returned body into an unapproved activation must fail.
        let (b, storage) = f.open();
        assert_eq!(storage.root(), current);
        assert!(!storage.data_write_blocked());
        assert_eq!(f.live_record(&storage), original);
        drop(storage);
        drop(b);
    }
    let (_, storage) = f.open();
    assert_eq!(storage.root(), current);
    assert_eq!(f.live_record(&storage), original);
    assert!(!storage.data_write_blocked());
    let owner = lease(&storage, &previous);
    assert!(intent::removal::run(&storage, &previous, &owner, &|| false).is_err());
    // A completed plan's old identity does not become an executable alias.
    let metadata = std::fs::metadata(current.join("meetings.sqlite3")).unwrap();
    assert!(metadata.ino() > 0);
}

#[test]
fn audio_reconfirmation_refuses_linked_body_without_touching_external_data() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    let (b, storage) = f.open();
    drop(storage);
    drop(b);
    let external = f._base.path().join("external-audio");
    std::fs::write(&external, b"KEEP EXTERNAL").unwrap();
    let file = f.target.join(&f.audio);
    symlink(&external, &file).unwrap();
    let (b, storage) = f.open();
    let owner = lease(&storage, &f.deletion);
    assert!(reconfirmation::Review::capture(&storage, &f.deletion, &owner, &|| false).is_err());
    assert_eq!(std::fs::read(&external).unwrap(), b"KEEP EXTERNAL");
    assert!(file.is_symlink());
    drop(owner);
    drop(storage);
    drop(b);
}
