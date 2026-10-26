use super::*;
use crate::storage::deletion::audio::{intent, Census, Disposition};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn failed_fixture() -> Fixture {
    let fixture = Fixture::new(2, 2000, false);
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    fixture.allow_db();
    fixture
        .storage
        .interrupt_recording(&fixture.session.id)
        .unwrap();
    fixture
}
fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}
fn preview(f: &Fixture) -> crate::storage::deletion::audio::Preview {
    Census::capture(
        &f.storage,
        &f.session.meeting_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .preview(&Uuid::new_v4().to_string())
    .unwrap()
}
fn first_checkpoint(f: &Fixture) -> (Directory, String, Checkpoint) {
    let plan = f.plan();
    let track = &plan.tracks[0];
    let directory = f.directory(track);
    let name = format!(".{}.json", track.chunks[0]);
    let (bytes, _) = directory.read_json(&name).unwrap().unwrap();
    let checkpoint: Checkpoint = serde_json::from_slice(&bytes).unwrap();
    (directory, name, checkpoint)
}

#[test]
fn flac_workspace_failed_commit_is_read_only_then_explicit_audio_only_deletion_retains_metadata() {
    let f = failed_fixture();
    let unknown = f
        .base
        .parent()
        .unwrap()
        .join(f.plan().tracks[0].id.clone())
        .join(format!(".{}.{}.part", Uuid::new_v4(), Uuid::new_v4()));
    write(&unknown, b"unknown producer file");
    let original = tree(f.base.parent().unwrap());
    let p = preview(&f);
    assert_eq!(p.audio_files, 8); // Six CAFs plus two unregistered final outputs.
    assert_eq!(p.unregistered_audio_files, 8);
    assert!(p
        .entries
        .iter()
        .filter(|e| e.relative_path.ends_with(".json"))
        .all(|e| e.disposition == Disposition::Retain));
    assert_eq!(p.unclassified_files, 1);
    assert_eq!(tree(f.base.parent().unwrap()), original);
    assert!(Census::capture(&f.storage, &f.session.meeting_id, &|| true).is_err());
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let census = Census::capture(
        &f.storage,
        &f.session.meeting_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(intent::prepare(
        &f.storage,
        &Uuid::new_v4().to_string(),
        &census,
        intent::Policy {
            include_unregistered_audio: false,
            retain_unclassified: true
        },
        &lease,
        &crate::test_support::never_cancelled
    )
    .is_err());
    drop(lease);
    assert_eq!(tree(f.base.parent().unwrap()), original);
    assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 8);
    for (path, bytes) in original {
        if path.extension().is_some_and(|ext| ext == "json")
            || path == unknown
            || path.file_name().unwrap() == LOCK
        {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        } else {
            assert!(!path.exists(), "{}", path.display());
        }
    }
    assert_eq!(f.registered_tracks(), 0);
    let Fixture {
        temp,
        storage,
        session,
        ..
    } = f;
    drop(storage);
    let reopened = Storage::open(temp.path().to_owned()).unwrap();
    let p = Census::capture(
        &reopened,
        &session.meeting_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .preview(&Uuid::new_v4().to_string())
    .unwrap();
    assert_eq!(p.audio_files, 0);
    assert_eq!(p.unclassified_files, 1);
}

#[test]
fn flac_workspace_allocated_inode_is_audio_but_pre_identity_and_replaced_files_are_not_adopted() {
    for owned in [true, false] {
        let f = failed_fixture();
        let (directory, name, mut checkpoint) = first_checkpoint(&f);
        let final_path = directory.path.join(format!("{}.flac", checkpoint.chunk_id));
        let staging_path = directory.path.join(&checkpoint.staging_name);
        std::fs::rename(&final_path, &staging_path).unwrap();
        checkpoint.audio = None;
        if !owned {
            checkpoint.staging_identity = None;
        }
        write(
            &directory.path.join(name),
            &serde_json::to_vec(&checkpoint).unwrap(),
        );
        let original = std::fs::read(&staging_path).unwrap();
        let p = preview(&f);
        let entry = p
            .entries
            .iter()
            .find(|e| f.storage.root.join(&e.relative_path) == staging_path)
            .unwrap();
        assert_eq!(
            entry.disposition,
            if owned {
                Disposition::ProducerAudioLeftover
            } else {
                Disposition::Unclassified
            }
        );
        assert_eq!(
            remove_reviewed_audio(&f.storage, &f.session.meeting_id),
            if owned { 8 } else { 7 }
        );
        if owned {
            assert!(!staging_path.exists());
        } else {
            assert_eq!(std::fs::read(staging_path).unwrap(), original);
        }
    }
    let f = failed_fixture();
    let (directory, _, checkpoint) = first_checkpoint(&f);
    let final_path = directory.path.join(format!("{}.flac", checkpoint.chunk_id));
    let preserved = directory.path.join("unrelated.bin");
    std::fs::rename(&final_path, &preserved).unwrap();
    write(&final_path, b"different inode at a producer filename");
    let p = preview(&f);
    assert_eq!(p.audio_files, 7);
    assert_eq!(p.unclassified_files, 2);
    assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 7);
    assert_eq!(
        std::fs::read(final_path).unwrap(),
        b"different inode at a producer filename"
    );
    assert!(preserved.is_file());
}

#[test]
fn flac_workspace_modified_receipt_invalidates_review_and_busy_converter_does_not_create_files() {
    let f = failed_fixture();
    let lock = Directory::open(&f.base)
        .unwrap()
        .required(LOCK, false)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let original = tree(f.base.parent().unwrap());
    assert!(Census::capture(
        &f.storage,
        &f.session.meeting_id,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(tree(f.base.parent().unwrap()), original);
    drop(lock);
    let census = Census::capture(
        &f.storage,
        &f.session.meeting_id,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let (directory, name, mut checkpoint) = first_checkpoint(&f);
    checkpoint.staging_identity = None;
    checkpoint.audio = None;
    write(
        &directory.path.join(name),
        &serde_json::to_vec(&checkpoint).unwrap(),
    );
    assert!(census
        .verify(&f.storage, &crate::test_support::never_cancelled)
        .is_err());
    assert_eq!(preview(&f).audio_files, 7);
}

#[test]
fn flac_workspace_copied_identity_and_non_audio_registration_do_not_grant_audio_deletion() {
    let f = failed_fixture();
    let foreign = tempfile::tempdir().unwrap();
    let foreign_storage = Storage::open(foreign.path().to_owned()).unwrap();
    let mut plan = f.plan();
    plan.recording_directory = Directory::open(&foreign_storage.root).unwrap().identity;
    write(&f.base.join(PLAN), &serde_json::to_vec(&plan).unwrap());
    assert_eq!(preview(&f).audio_files, 6);
    assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 6);
    for track in &plan.tracks {
        assert!(f
            .directory(track)
            .path
            .join(format!("{}.flac", track.chunks[0]))
            .is_file());
    }
    let f = failed_fixture();
    let (directory, _, checkpoint) = first_checkpoint(&f);
    let relative = format!(
        "meetings/{}/audio/{}/{}.flac",
        f.session.meeting_id,
        directory.path.file_name().unwrap().to_str().unwrap(),
        checkpoint.chunk_id
    );
    let bytes = std::fs::read(f.storage.root.join(&relative)).unwrap();
    f.storage.connection.lock().unwrap().execute(
        "INSERT INTO file_assets(id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) VALUES(?1,?2,'meeting',?3,'application/octet-stream',?4,'sha256',?5,'ready',1,1)",
        params![Uuid::new_v4().to_string(),f.session.meeting_id,relative,bytes.len() as i64,hash(&bytes)]).unwrap();
    assert_eq!(preview(&f).audio_files, 7);
    assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 7);
    assert_eq!(std::fs::read(f.storage.root.join(relative)).unwrap(), bytes);
}

#[test]
fn flac_workspace_partial_rollback_and_saved_partial_retain_history_after_audio_deletion() {
    for rollback in [true, false] {
        let f = partial_fixture();
        let review = f.storage.review_partial_recording(&f.session.id).unwrap();
        if rollback {
            f.reject_partial_commit(true);
        }
        let result = f.storage.recover_partial_recording(&review, true);
        if rollback {
            assert!(result.is_err());
            f.reject_partial_commit(false);
        } else {
            result.unwrap();
        }
        let original = tree(f.base.parent().unwrap());
        assert_eq!(preview(&f).audio_files, 10);
        assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 10);
        for (path, bytes) in original
            .into_iter()
            .filter(|(p, _)| p.extension().is_some_and(|e| e == "json"))
        {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        if !rollback {
            assert_eq!(
                f.storage
                    .partial_recovery_listing(None)
                    .unwrap()
                    .items
                    .len(),
                1
            );
        }
    }
}

#[test]
fn flac_workspace_invalid_receipt_duplicate_outputs_and_links_fail_closed() {
    for kind in ["hash", "duplicate", "link"]
        .into_iter()
        .filter(|kind| *kind != "link" || cfg!(unix))
    {
        let f = failed_fixture();
        let (directory, name, mut checkpoint) = first_checkpoint(&f);
        let final_path = directory.path.join(format!("{}.flac", checkpoint.chunk_id));
        if kind == "hash" {
            checkpoint.input_hash = "0".repeat(64);
            write(
                &directory.path.join(name),
                &serde_json::to_vec(&checkpoint).unwrap(),
            );
        } else if kind == "duplicate" {
            write(
                &directory.path.join(checkpoint.staging_name),
                b"unknown duplicate",
            );
        } else {
            #[cfg(unix)]
            {
                let external = directory.path.join("external.bin");
                std::fs::rename(&final_path, &external).unwrap();
                std::os::unix::fs::symlink(&external, &final_path).unwrap();
            }
        }
        let original = tree_without_links(f.base.parent().unwrap());
        assert!(
            Census::capture(
                &f.storage,
                &f.session.meeting_id,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(tree_without_links(f.base.parent().unwrap()), original);
    }
}

#[test]
fn flac_workspace_unproven_sidecars_allow_raw_audio_review_without_adopting_final_outputs() {
    for invalid_lock in [false, true]
        .into_iter()
        .filter(|invalid| !invalid || cfg!(unix))
    {
        let f = failed_fixture();
        let plan = f.plan();
        if invalid_lock {
            #[cfg(unix)]
            std::fs::set_permissions(f.base.join(LOCK), std::fs::Permissions::from_mode(0o644))
                .unwrap();
        } else {
            write(&f.base.join(PLAN), b"unreadable producer plan");
        }
        let original = tree(f.base.parent().unwrap());
        assert_eq!(preview(&f).audio_files, 6);
        assert_eq!(tree(f.base.parent().unwrap()), original);
        assert_eq!(remove_reviewed_audio(&f.storage, &f.session.meeting_id), 6);
        for track in plan.tracks {
            for chunk in track.chunks {
                let path = f
                    .base
                    .parent()
                    .unwrap()
                    .join(&track.id)
                    .join(format!("{chunk}.flac"));
                assert_eq!(std::fs::read(&path).unwrap(), original[&path]);
            }
        }
    }
}
fn tree_without_links(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            files.extend(tree_without_links(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

#[test]
#[cfg(unix)]
#[ignore = "Dedicated owned synthetic SIGKILL children; no capture devices"]
fn flac_workspace_owned_sigkill_audio_deletion_keeps_pre_identity_files_and_all_receipts() {
    use std::os::unix::process::ExitStatusExt;
    for partial_mode in [false, true] {
        for phase in [
            "reserved",
            "created-before-identity",
            "allocated",
            "encoded",
            "published",
        ] {
            let Fixture {
                temp,
                storage,
                session,
                base,
            } = if partial_mode {
                partial_fixture()
            } else {
                Fixture::new(1, 2000, false)
            };
            drop(storage);
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    if partial_mode {
                        "storage::recordings::flac::tests::partial_recovery_crash_child"
                    } else {
                        "storage::recordings::flac::tests::final_flac_crash_child"
                    },
                    "--ignored",
                    "--nocapture",
                ])
                .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
                .env("MON_FINAL_FLAC_CRASH_PHASE", phase)
                .env("MON_FINAL_FLAC_CRASH_ROOT", temp.path())
                .env("MON_FINAL_FLAC_CRASH_OWNER", &session.id)
                .output()
                .unwrap();
            assert_eq!(
                child.status.signal(),
                Some(libc::SIGKILL),
                "{partial_mode}/{phase}: {}",
                String::from_utf8_lossy(&child.stderr)
            );
            let storage = Storage::open(temp.path().to_owned()).unwrap();
            let original = tree(base.parent().unwrap());
            let raw = if partial_mode { 8 } else { 4 };
            let owned = usize::from(matches!(phase, "allocated" | "encoded" | "published"));
            assert_eq!(
                remove_reviewed_audio(&storage, &session.meeting_id),
                raw + owned,
                "{partial_mode}/{phase}"
            );
            for (path, bytes) in original {
                if path.extension().is_some_and(|e| e == "json")
                    || path.file_name().unwrap() == LOCK
                    || (phase == "created-before-identity"
                        && path.extension().is_some_and(|e| e == "part"))
                {
                    assert_eq!(std::fs::read(path).unwrap(), bytes);
                } else {
                    assert!(!path.exists(), "{}", path.display());
                }
            }
            assert!(storage.get_meeting(&session.meeting_id).unwrap().is_some());
        }
    }
}
