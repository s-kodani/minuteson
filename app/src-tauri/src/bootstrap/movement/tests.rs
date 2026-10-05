use super::*;
use std::cell::Cell;

struct Fixture {
    base: tempfile::TempDir,
    bootstrap: Bootstrap,
    storage: Storage,
}
impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let parent = base.path().canonicalize().unwrap();
        let bootstrap = Bootstrap::open(&parent.join("config"), &parent.join("app-data")).unwrap();
        let storage = Storage::open(bootstrap.root().into()).unwrap();
        bootstrap.initialize(&storage).unwrap();
        storage.create_meeting("same native meeting").unwrap();
        Self {
            base,
            bootstrap,
            storage,
        }
    }
    fn prepared(&self) -> Prepared {
        self.bootstrap
            .prepare_move(&self.storage, self.base.path(), &|| false, &|_| {})
            .unwrap()
    }
}

#[test]
fn move_confirmation_creates_no_target_and_preserves_same_meeting_and_original_on_cutover() {
    let f = Fixture::new();
    let original = f.storage.list_meetings().unwrap()[0].clone();
    let mut prepared = f.prepared();
    let preview = prepared.preview();
    assert_eq!(preview.meeting_count, 1);
    assert_eq!(preview.file_count, Some(1));
    assert!(preview.required_bytes > 0);
    assert!(!preview.destination.exists());
    assert!(f.bootstrap.view().unwrap().pending_operation_id.is_none());
    let receipt = prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &preview.confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.restart_required);
    assert!(f.storage.restart_required());
    assert!(f.storage.create_meeting("must reject old writes").is_err());
    assert!(f.storage.root().join("meetings.sqlite3").exists());
    assert_eq!(
        f.bootstrap.view().unwrap().active_meeting_root,
        f.storage.root()
    );
    let config = f.bootstrap.config.clone();
    let data = f.bootstrap.models_root.parent().unwrap().to_path_buf();
    drop(f.storage);
    drop(f.bootstrap);
    let cold = Bootstrap::open(&config, &data).unwrap();
    assert_eq!(cold.root(), receipt.destination);
    let reopened = Storage::open(cold.root().into()).unwrap();
    cold.initialize(&reopened).unwrap();
    let meetings = reopened.list_meetings().unwrap();
    assert_eq!(meetings.len(), 1);
    assert_eq!(meetings[0].id, original.id);
    reopened
        .create_meeting("new root writable after admission")
        .unwrap();
}

#[test]
fn move_confirmation_requires_refresh_after_edits_and_never_uses_wrong_confirmation() {
    let f = Fixture::new();
    let mut prepared = f.prepared();
    let before = prepared.preview();
    assert!(prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &Uuid::new_v4().to_string(),
            &|| false,
            &|_| {}
        )
        .is_err());
    f.storage.create_meeting("after confirmation").unwrap();
    assert!(prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &before.confirmation_id,
            &|| false,
            &|_| {}
        )
        .is_err());
    assert!(!before.destination.exists());
    assert!(f.bootstrap.view().unwrap().pending_operation_id.is_none());
    assert!(!f.storage.restart_required());
    prepared
        .refresh(&f.bootstrap, &f.storage, &|| false, &|_| {})
        .unwrap();
    let refreshed = prepared.preview();
    assert_ne!(refreshed.confirmation_id, before.confirmation_id);
    assert_eq!(refreshed.operation_id, before.operation_id);
    assert_eq!(refreshed.meeting_count, 2);
    prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &refreshed.confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
}

#[test]
fn move_parent_substitution_collision_and_excluded_roots_are_refused_without_adoption() {
    for failure in ["parent", "collision", "revision"] {
        let f = Fixture::new();
        let chosen = f.base.path().join("chosen");
        std::fs::create_dir(&chosen).unwrap();
        let mut prepared = f
            .bootstrap
            .prepare_move(&f.storage, &chosen, &|| false, &|_| {})
            .unwrap();
        let preview = prepared.preview();
        match failure {
            "parent" => {
                std::fs::rename(&chosen, f.base.path().join("retained-parent")).unwrap();
                std::fs::create_dir(&chosen).unwrap();
                std::fs::write(chosen.join("foreign"), b"retain").unwrap();
            }
            "collision" => std::fs::write(&preview.destination, b"retain").unwrap(),
            "revision" => {
                let mut state = f.bootstrap.state.lock().unwrap();
                let mut record = state.loaded.as_ref().unwrap().record.clone();
                record.revision += 1;
                f.bootstrap.write(&mut state, record).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            prepared
                .publish(
                    &f.bootstrap,
                    &f.storage,
                    &preview.confirmation_id,
                    &|| false,
                    &|_| {}
                )
                .is_err(),
            "{failure}"
        );
        assert!(f.bootstrap.view().unwrap().pending_operation_id.is_none());
        assert!(!f.storage.restart_required());
        if failure == "collision" {
            assert_eq!(std::fs::read(&preview.destination).unwrap(), b"retain");
        }
        if failure == "parent" {
            assert_eq!(std::fs::read(chosen.join("foreign")).unwrap(), b"retain");
        }
    }
    let f = Fixture::new();
    for root in [f.storage.root().to_path_buf(), f.bootstrap.config.clone()] {
        assert!(f
            .bootstrap
            .prepare_move(&f.storage, &root, &|| false, &|_| {})
            .is_err());
    }
    std::fs::create_dir_all(&f.bootstrap.models_root).unwrap();
    assert!(f
        .bootstrap
        .prepare_move(&f.storage, &f.bootstrap.models_root, &|| false, &|_| {})
        .is_err());
}

#[test]
fn move_cancel_before_intent_is_reversible_but_after_copy_intent_keeps_both_roots() {
    let f = Fixture::new();
    let mut prepared = f.prepared();
    let preview = prepared.preview();
    assert!(prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &preview.confirmation_id,
            &|| true,
            &|_| {}
        )
        .is_err());
    assert!(!preview.destination.exists());
    assert!(f.bootstrap.view().unwrap().pending_operation_id.is_none());
    let cancelled = Cell::new(false);
    assert!(prepared
        .publish(
            &f.bootstrap,
            &f.storage,
            &preview.confirmation_id,
            &|| cancelled.get(),
            &|phase| {
                if phase == "copying" {
                    cancelled.set(true);
                }
            }
        )
        .is_err());
    let current = f.bootstrap.view().unwrap();
    assert_eq!(current.meeting_root, f.storage.root());
    assert_eq!(
        current.pending_operation_id.as_deref(),
        Some(preview.operation_id.as_str())
    );
    assert!(preview.destination.exists());
    assert!(!f.storage.restart_required());
    f.storage.create_meeting("source remains selected").unwrap();
    assert!(f
        .bootstrap
        .prepare_move(&f.storage, f.base.path(), &|| false, &|_| {})
        .is_err());
}

#[test]
fn move_panicked_copy_progress_keeps_intent_and_never_selects_partial_target() {
    let f = Fixture::new();
    let mut prepared = f.prepared();
    let preview = prepared.preview();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepared.publish(
            &f.bootstrap,
            &f.storage,
            &preview.confirmation_id,
            &|| false,
            &|phase| {
                if phase == "handoff" {
                    panic!("synthetic interruption after copy");
                }
            },
        )
    }));
    assert!(result.is_err());
    assert_eq!(f.bootstrap.view().unwrap().meeting_root, f.storage.root());
    assert!(preview.destination.join("meetings.sqlite3").exists());
    assert!(!f.storage.restart_required());
    f.storage
        .create_meeting("old selected after interrupted copy")
        .unwrap();
}
