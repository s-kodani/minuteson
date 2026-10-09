//! Real native placement, selection and cold admission; not generic copying.
use super::*;
use crate::bootstrap::Bootstrap;
use crate::storage::deletion::audio::{Census, Disposition};
use std::collections::BTreeMap;
use std::os::unix::fs::DirBuilderExt;

fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(tree(&path));
        } else {
            let bytes = std::fs::read(&path).unwrap();
            result.insert(path, bytes);
        }
    }
    result
}
fn control_temp() -> tempfile::TempDir {
    // Use an already canonical macOS parent; /var is commonly a symlink.
    tempfile::tempdir_in("/private/tmp").unwrap()
}
fn failed() -> Fixture {
    let f = Fixture::new(2, 2000, false);
    f.fail_db();
    assert!(f.storage.finalize_recording(&f.session.id).is_err());
    f.allow_db();
    f.storage.interrupt_recording(&f.session.id).unwrap();
    f
}
fn move_storage(storage: Storage, config: &Path, legacy: &Path, target: &Path) -> Storage {
    let bootstrap = Bootstrap::open(config, legacy).unwrap();
    assert_eq!(bootstrap.root(), storage.root());
    bootstrap.initialize(&storage).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(target)
        .unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let revision = bootstrap.view().unwrap().revision;
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    bootstrap
        .begin_move(
            &operation,
            &confirmation,
            target,
            snapshot.manifest_hash(),
            revision,
        )
        .unwrap();
    let mut placement = bootstrap.pending_placement(&operation, &snapshot).unwrap();
    placement.copy(&snapshot, &|| false).unwrap();
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    let envelope: Value = serde_json::from_slice(handoff.bytes()).unwrap();
    assert_eq!(envelope["version"], 5);
    assert!(envelope["flac_history"]["workspaces"]
        .as_array()
        .is_some_and(|w| !w.is_empty()));
    placement.install_history(&handoff, &|| false).unwrap();
    placement
        .activate_for_restart(&handoff, &confirmation, revision + 1, &|| false)
        .unwrap();
    assert!(storage.restart_required());
    assert!(storage.create_meeting("旧Storageを変更しない").is_err());
    drop(placement);
    drop(handoff);
    drop(snapshot);
    drop(storage);
    drop(bootstrap);
    let bootstrap = Bootstrap::open(config, legacy).unwrap();
    assert_eq!(bootstrap.root(), target);
    let storage = bootstrap.open_storage().unwrap();
    assert_eq!(
        storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM audio_tracks", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    storage
}
fn first(f: &Fixture) -> (Directory, String, Checkpoint) {
    let plan = f.plan();
    let track = &plan.tracks[0];
    let directory = f.directory(track);
    let name = format!(".{}.json", track.chunks[0]);
    let checkpoint =
        serde_json::from_slice(&directory.read_json(&name).unwrap().unwrap().0).unwrap();
    (directory, name, checkpoint)
}
fn preview(storage: &Storage, meeting: &str) -> crate::storage::deletion::audio::Preview {
    Census::capture(storage, meeting, &|| false)
        .unwrap()
        .preview(&Uuid::new_v4().to_string())
        .unwrap()
}

#[test]
fn flac_relocation_normal_rollback_cold_resume_preserves_ids_pcm_and_original_receipts() {
    let f = failed();
    let control = control_temp();
    let config = control.path().join("config");
    let target = control.path().join("target");
    let source = f.storage.root().to_owned();
    let relative = f.base.strip_prefix(&source).unwrap().to_owned();
    let original = tree(f.base.parent().unwrap());
    let plan = f.plan();
    let storage = move_storage(f.storage, &config, &source, &target);
    let copied_plan = std::fs::read(target.join(&relative).join(PLAN)).unwrap();
    assert_eq!(copied_plan, original[&f.base.join(PLAN)]);
    assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 8);
    assert_eq!(
        storage.finalize_recording(&f.session.id).unwrap().state,
        "saved"
    );
    let db = storage.connection.lock().unwrap();
    for track in &plan.tracks {
        assert_eq!(
            db.query_row(
                "SELECT id FROM audio_tracks WHERE id=?1",
                [&track.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            track.id
        );
        for chunk in &track.chunks {
            assert_eq!(
                db.query_row(
                    "SELECT track_id FROM audio_chunks WHERE id=?1",
                    [chunk],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                track.id
            );
        }
    }
    drop(db);
    assert_eq!(
        std::fs::read(target.join(&relative).join(PLAN)).unwrap(),
        copied_plan
    );
    assert_eq!(tree(f.base.parent().unwrap()), original);
    drop(storage);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    assert_eq!(
        storage
            .get_meeting(&f.session.meeting_id)
            .unwrap()
            .unwrap()
            .lifecycle,
        "recorded"
    );
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
fn flac_relocation_twice_flattens_original_plan_and_checkpoint_identities() {
    let f = failed();
    let control = control_temp();
    let config = control.path().join("config");
    let source = f.storage.root().to_owned();
    let relative = f.base.strip_prefix(&source).unwrap().to_owned();
    let original = tree(f.base.parent().unwrap());
    let second = control.path().join("second");
    let third = control.path().join("third");
    let storage = move_storage(f.storage, &config, &source, &second);
    let second_tree = tree(second.join(&relative).parent().unwrap());
    let storage = move_storage(storage, &config, &source, &third);
    assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 8);
    storage.finalize_recording(&f.session.id).unwrap();
    assert_eq!(
        std::fs::read(third.join(&relative).join(PLAN)).unwrap(),
        original[&f.base.join(PLAN)]
    );
    assert_eq!(tree(second.join(&relative).parent().unwrap()), second_tree);
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
fn flac_relocation_reserved_unknown_stage_is_retained_and_not_adopted_on_resume() {
    let f = failed();
    let (directory, name, mut checkpoint) = first(&f);
    let final_name = format!("{}.flac", checkpoint.chunk_id);
    directory
        .rename(&final_name, &checkpoint.staging_name, true)
        .unwrap();
    checkpoint.staging_identity = None;
    checkpoint.audio = None;
    let stamp = directory.read_json(&name).unwrap().unwrap().1;
    directory.json(&name, &checkpoint, Some(&stamp)).unwrap();
    let unknown = directory.path.join(&checkpoint.staging_name);
    let unknown_bytes = std::fs::read(&unknown).unwrap();
    let source = f.storage.root().to_owned();
    let relative = unknown.strip_prefix(&source).unwrap().to_owned();
    let control = control_temp();
    let target = control.path().join("target");
    let storage = move_storage(f.storage, &control.path().join("config"), &source, &target);
    let p = preview(&storage, &f.session.meeting_id);
    assert_eq!(p.audio_files, 7);
    assert_eq!(
        p.entries
            .iter()
            .find(|e| e.relative_path == relative.to_str().unwrap())
            .unwrap()
            .disposition,
        Disposition::Unclassified
    );
    storage.finalize_recording(&f.session.id).unwrap();
    assert_eq!(
        std::fs::read(target.join(&relative)).unwrap(),
        unknown_bytes
    );
    assert_eq!(std::fs::read(&unknown).unwrap(), unknown_bytes);
    let moved_receipt: Checkpoint = serde_json::from_slice(
        &std::fs::read(
            target
                .join(directory.path.strip_prefix(&source).unwrap())
                .join(&name),
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(moved_receipt.staging_name, checkpoint.staging_name);
}

#[test]
fn flac_relocation_allocated_stage_resumes_with_new_durable_local_identity() {
    let f = failed();
    let (directory, name, mut checkpoint) = first(&f);
    let old_identity = checkpoint.staging_identity.clone().unwrap();
    directory
        .rename(
            &format!("{}.flac", checkpoint.chunk_id),
            &checkpoint.staging_name,
            true,
        )
        .unwrap();
    checkpoint.audio = None;
    let file = directory.required(&checkpoint.staging_name, true).unwrap();
    file.set_len(19).unwrap();
    file.sync_all().unwrap();
    let stamp = directory.read_json(&name).unwrap().unwrap().1;
    directory.json(&name, &checkpoint, Some(&stamp)).unwrap();
    let source = f.storage.root().to_owned();
    let control = control_temp();
    let target = control.path().join("target");
    let storage = move_storage(f.storage, &control.path().join("config"), &source, &target);
    let copied =
        Directory::open(&target.join(directory.path.strip_prefix(&source).unwrap())).unwrap();
    let owned_identity =
        Identity::file(&copied.required(&checkpoint.staging_name, false).unwrap()).unwrap();
    assert_ne!(owned_identity, old_identity);
    storage.finalize_recording(&f.session.id).unwrap();
    let updated: Checkpoint =
        serde_json::from_slice(&copied.read_json(&name).unwrap().unwrap().0).unwrap();
    assert_eq!(updated.staging_identity, Some(owned_identity.clone()));
    assert_eq!(
        Identity::file(
            &copied
                .required(&format!("{}.flac", checkpoint.chunk_id), false)
                .unwrap()
        )
        .unwrap(),
        owned_identity
    );
    assert_eq!(
        directory
            .required(&checkpoint.staging_name, false)
            .unwrap()
            .metadata()
            .unwrap()
            .len(),
        19
    );
    assert_eq!(
        serde_json::from_slice::<Checkpoint>(&directory.read_json(&name).unwrap().unwrap().0)
            .unwrap(),
        checkpoint
    );
}

#[test]
fn flac_relocation_plain_storage_open_does_not_adopt_copied_plan_or_audio() {
    let f = failed();
    let control = control_temp();
    let source = f.storage.root().to_owned();
    let config = control.path().join("config");
    let target = control.path().join("target");
    let storage = move_storage(f.storage, &config, &source, &target);
    let original = tree(&target.join("meetings"));
    drop(storage);
    let storage = Storage::open(target.clone()).unwrap();
    assert!(storage.finalize_recording(&f.session.id).is_err());
    assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 6);
    assert_eq!(tree(&target.join("meetings")), original);
    drop(storage);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 8);
    storage.finalize_recording(&f.session.id).unwrap();
}

#[test]
fn flac_relocation_replaced_body_or_output_directory_is_retained_not_rebound() {
    for replace_directory in [false, true] {
        let f = failed();
        let plan = f.plan();
        let control = control_temp();
        let source = f.storage.root().to_owned();
        let target = control.path().join("target");
        let storage = move_storage(f.storage, &control.path().join("config"), &source, &target);
        let directory = target.join(format!(
            "meetings/{}/audio/{}",
            f.session.meeting_id, plan.tracks[0].id
        ));
        if replace_directory {
            std::fs::rename(&directory, directory.with_extension("retained")).unwrap();
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            for entry in std::fs::read_dir(directory.with_extension("retained")).unwrap() {
                let path = entry.unwrap().path();
                write(
                    &directory.join(path.file_name().unwrap()),
                    &std::fs::read(&path).unwrap(),
                );
            }
        } else {
            let path = directory.join(format!("{}.flac", plan.tracks[0].chunks[0]));
            let bytes = std::fs::read(&path).unwrap();
            std::fs::rename(&path, path.with_extension("retained")).unwrap();
            write(&path, &bytes);
        }
        let before = tree(&target.join("meetings"));
        assert!(storage.finalize_recording(&f.session.id).is_err());
        assert_eq!(tree(&target.join("meetings")), before);
        assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 7);
        assert_eq!(tree(&target.join("meetings")), before);
    }
}

#[test]
fn flac_relocation_partial_outputs_are_deletable_but_old_consent_is_invalid() {
    let f = partial_fixture();
    f.reject_partial_commit(true);
    let review = f.storage.review_partial_recording(&f.session.id).unwrap();
    assert!(f.storage.recover_partial_recording(&review, true).is_err());
    f.reject_partial_commit(false);
    let source = f.storage.root().to_owned();
    let old = tree(f.base.parent().unwrap());
    let control = control_temp();
    let config = control.path().join("config");
    let target = control.path().join("target");
    let storage = move_storage(f.storage, &config, &source, &target);
    let before = tree(&target.join("meetings"));
    assert_eq!(preview(&storage, &f.session.meeting_id).audio_files, 10);
    assert!(storage.recover_partial_recording(&review, true).is_err());
    let fresh = storage.review_partial_recording(&f.session.id).unwrap();
    assert_ne!(fresh.summary.review_hash, review.summary.review_hash);
    assert!(storage.recover_partial_recording(&fresh, false).is_err());
    assert_eq!(tree(&target.join("meetings")), before);
    assert_eq!(tree(f.base.parent().unwrap()), old);
    assert_eq!(remove_reviewed_audio(&storage, &f.session.meeting_id), 10);
    for (path, bytes) in before {
        if path.extension().is_some_and(|ext| ext == "json") || path.file_name().unwrap() == LOCK {
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        } else {
            assert!(!path.exists(), "{}", path.display());
        }
    }
    assert_eq!(tree(f.base.parent().unwrap()), old);
}

fn failed_partial() -> Fixture {
    let f = partial_fixture();
    f.reject_partial_commit(true);
    let review = f.storage.review_partial_recording(&f.session.id).unwrap();
    assert!(f.storage.recover_partial_recording(&review, true).is_err());
    f.reject_partial_commit(false);
    f
}

#[test]
fn flac_relocation_partial_fresh_consent_resumes_same_ids_and_current_result_after_cold_restart() {
    let f = failed_partial();
    let old_review = f.storage.review_partial_recording(&f.session.id).unwrap();
    let control = control_temp();
    let config = control.path().join("config");
    let target = control.path().join("target");
    let source = f.storage.root().to_owned();
    let relative = f.base.strip_prefix(&source).unwrap().to_owned();
    let original = tree(f.base.parent().unwrap());
    let plan_bytes = std::fs::read(f.base.join(PARTIAL_PLAN)).unwrap();
    let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
    drop(move_storage(f.storage, &config, &source, &target));
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    let before = tree(&target.join("meetings"));
    let review = storage.review_partial_recording(&f.session.id).unwrap();
    assert_ne!(review.summary.review_hash, old_review.summary.review_hash);
    assert_eq!(
        review.content_hash().unwrap(),
        old_review.content_hash().unwrap()
    );
    assert!(storage
        .recover_partial_recording(&old_review, true)
        .is_err());
    assert!(storage.recover_partial_recording(&review, false).is_err());
    assert_eq!(tree(&target.join("meetings")), before);
    let result = storage.recover_partial_recording(&review, true).unwrap();
    assert_eq!(result.state, "interrupted");
    assert_eq!(result.ended_at_utc_ms, None);
    assert_eq!(
        storage
            .partial_recovery_result(&f.session.id, &review.summary.review_hash)
            .unwrap(),
        Some(review.summary.clone())
    );
    assert!(storage
        .partial_recovery_result(&f.session.id, &old_review.summary.review_hash)
        .unwrap()
        .is_none());
    // This fixture has a real inner gap: use the saved-run-aware path, never
    // demand continuous playback or turn the gap into silence.
    let (audio, known) = storage
        .recorded_analysis_input(&f.session.meeting_id)
        .unwrap();
    assert_eq!(audio.tracks.len(), 2);
    assert_eq!(known.unwrap().runs.len(), 4);
    assert_partial_whole_meeting_playback(&storage, &f.session.meeting_id, result.duration_us);
    let db = storage.connection.lock().unwrap();
    for track in &plan.tracks {
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM audio_tracks WHERE id=?1",
                [&track.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        for chunk in &track.chunks {
            assert_eq!(
                db.query_row(
                    "SELECT track_id FROM audio_chunks WHERE id=?1",
                    [chunk],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                track.id
            );
        }
    }
    drop(db);
    assert_eq!(
        std::fs::read(target.join(&relative).join(PARTIAL_PLAN)).unwrap(),
        plan_bytes
    );
    assert_eq!(tree(f.base.parent().unwrap()), original);
    drop(storage);
    drop(bootstrap);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    assert_eq!(
        storage
            .partial_recovery_result(&f.session.id, &review.summary.review_hash)
            .unwrap(),
        Some(review.summary)
    );
    assert_eq!(
        storage.partial_recovery_listing(None).unwrap().items.len(),
        1
    );
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
fn flac_relocation_partial_twice_before_resume_flattens_input_and_document_evidence() {
    let f = failed_partial();
    let source = f.storage.root().to_owned();
    let original = tree(f.base.parent().unwrap());
    let control = control_temp();
    let config = control.path().join("config");
    let second = control.path().join("second");
    let third = control.path().join("third");
    let storage = move_storage(f.storage, &config, &source, &second);
    let review_second = storage.review_partial_recording(&f.session.id).unwrap();
    let second_tree = tree(&second.join("meetings"));
    let storage = move_storage(storage, &config, &source, &third);
    let review = storage.review_partial_recording(&f.session.id).unwrap();
    assert_ne!(
        review.summary.review_hash,
        review_second.summary.review_hash
    );
    assert!(storage
        .recover_partial_recording(&review_second, true)
        .is_err());
    storage.recover_partial_recording(&review, true).unwrap();
    assert_eq!(tree(&second.join("meetings")), second_tree);
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
fn flac_relocation_partial_local_progress_then_second_move_uses_new_consent_again() {
    let f = failed_partial();
    let source = f.storage.root().to_owned();
    let original = tree(f.base.parent().unwrap());
    let control = control_temp();
    let config = control.path().join("config");
    let second = control.path().join("second");
    let third = control.path().join("third");
    let storage = move_storage(f.storage, &config, &source, &second);
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_moved_partial BEFORE UPDATE OF audio_manifest_hash ON recording_sessions BEGIN SELECT RAISE(ABORT,'synthetic relocated rollback'); END").unwrap();
    let second_review = storage.review_partial_recording(&f.session.id).unwrap();
    assert!(storage
        .recover_partial_recording(&second_review, true)
        .is_err());
    storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_moved_partial")
        .unwrap();
    let second_tree = tree(&second.join("meetings"));
    let document: Value = serde_json::from_slice(
        &std::fs::read(
            second
                .join(f.base.strip_prefix(&source).unwrap())
                .join("partial-recovery.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        document["partialRecovery"]["reviewHash"],
        second_review.summary.review_hash
    );
    let storage = move_storage(storage, &config, &source, &third);
    assert!(storage
        .recover_partial_recording(&second_review, true)
        .is_err());
    let review = storage.review_partial_recording(&f.session.id).unwrap();
    assert_ne!(
        review.summary.review_hash,
        second_review.summary.review_hash
    );
    storage.recover_partial_recording(&review, true).unwrap();
    assert_eq!(tree(&second.join("meetings")), second_tree);
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
fn flac_relocation_partial_generic_open_does_not_mint_input_reuse_or_new_consent() {
    let f = failed_partial();
    let source = f.storage.root().to_owned();
    let control = control_temp();
    let config = control.path().join("config");
    let target = control.path().join("target");
    drop(move_storage(f.storage, &config, &source, &target));
    let before = tree(&target.join("meetings"));
    let generic = Storage::open(target.clone()).unwrap();
    let review = generic.review_partial_recording(&f.session.id).unwrap();
    assert!(generic.recover_partial_recording(&review, true).is_err());
    assert_eq!(tree(&target.join("meetings")), before);
    drop(generic);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    let review = storage.review_partial_recording(&f.session.id).unwrap();
    storage.recover_partial_recording(&review, true).unwrap();
}

#[test]
fn flac_relocation_partial_changed_tip_or_meeting_invalidates_input_reuse_without_writes() {
    for change in ["closure", "meeting"] {
        let f = failed_partial();
        let source = f.storage.root().to_owned();
        let control = control_temp();
        let target = control.path().join("target");
        let storage = move_storage(f.storage, &control.path().join("config"), &source, &target);
        if change == "closure" {
            let path = target
                .join(f.base.strip_prefix(&source).unwrap())
                .join("raw-microphone.json");
            let mut tip: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            tip["producerClosed"] = true.into();
            write(&path, &serde_json::to_vec(&tip).unwrap());
        } else {
            storage
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE meetings SET lock_version=lock_version+1 WHERE id=?1",
                    [&f.session.meeting_id],
                )
                .unwrap();
        }
        let before = tree(&target.join("meetings"));
        let review = storage.review_partial_recording(&f.session.id).unwrap();
        let error = storage
            .recover_partial_recording(&review, true)
            .unwrap_err();
        assert!(error.contains("移動元から変更"), "{change}: {error}");
        assert_eq!(tree(&target.join("meetings")), before);
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM audio_tracks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn flac_relocation_partial_replaced_or_semantically_changed_document_cannot_be_rebased() {
    for change in ["same-bytes", "meaning", "missing"] {
        let f = failed_partial();
        let source = f.storage.root().to_owned();
        let control = control_temp();
        let target = control.path().join("target");
        let storage = move_storage(f.storage, &control.path().join("config"), &source, &target);
        let path = target
            .join(f.base.strip_prefix(&source).unwrap())
            .join("partial-recovery.json");
        let bytes = std::fs::read(&path).unwrap();
        if change == "same-bytes" {
            std::fs::rename(&path, path.with_extension("retained")).unwrap();
            write(&path, &bytes);
        } else if change == "meaning" {
            let mut document: Value = serde_json::from_slice(&bytes).unwrap();
            document["partialRecovery"]["tailLossUs"] = 0.into();
            write(&path, &serde_json::to_vec(&document).unwrap());
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        let before = tree(&target.join("meetings"));
        let review = storage.review_partial_recording(&f.session.id).unwrap();
        assert!(storage.recover_partial_recording(&review, true).is_err());
        assert_eq!(tree(&target.join("meetings")), before);
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM audio_tracks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn flac_relocation_rehashed_binding_still_requires_the_independently_placed_inode() {
    use rusqlite::config::DbConfig;
    for failure in ["directory", "body", "unknown-field", "document"] {
        let f = if failure == "document" {
            failed_partial()
        } else {
            failed()
        };
        let control = control_temp();
        let config = control.path().join("config");
        let target = control.path().join("target");
        let source = f.storage.root().to_owned();
        drop(move_storage(f.storage, &config, &source, &target));
        let registry_path = target.join(".mon-scratch.sqlite3");
        let registry = Connection::open(&registry_path).unwrap();
        registry
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        let (operation, json): (String, String) = registry
            .query_row(
                "SELECT operation_id,local_json FROM relocation_history",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let mut json: Value = serde_json::from_str(&json).unwrap();
        match failure {
            "directory" => {
                json["flac_binding"]["workspaces"][0]["tracks"][0]["directory"]["inode"] = 0.into()
            }
            "body" => {
                json["flac_binding"]["workspaces"][0]["tracks"][0]["bodies"][0]["inode"] = 0.into()
            }
            "unknown-field" => {
                json["flac_binding"]["workspaces"][0]["tracks"][0]["verified"] = true.into()
            }
            "document" => {
                json["flac_binding"]["workspaces"][0]["recovery_document"]["inode"] = 0.into()
            }
            _ => unreachable!(),
        }
        let json = serde_json::to_string(&json).unwrap();
        registry
            .execute(
                "UPDATE relocation_history SET local_json=?1,local_hash=?2 WHERE operation_id=?3",
                params![json, hash(json.as_bytes()), operation],
            )
            .unwrap();
        drop(registry);
        let native = std::fs::read(target.join("meetings.sqlite3")).unwrap();
        let registry = std::fs::read(&registry_path).unwrap();
        let audio = tree(&target.join("meetings"));
        let bootstrap = Bootstrap::open(&config, &source).unwrap();
        let error = match bootstrap.open_storage() {
            Ok(_) => panic!("forged FLAC ownership admitted: {failure}"),
            Err(error) => error,
        };
        assert!(
            error.contains(match failure {
                "directory" => "配置した所有対象",
                "body" => "配置した実体",
                "document" => "記録は配置した実体",
                _ => "unknown field",
            }),
            "{failure}: {error}"
        );
        assert_eq!(
            std::fs::read(target.join("meetings.sqlite3")).unwrap(),
            native
        );
        assert_eq!(std::fs::read(&registry_path).unwrap(), registry);
        assert_eq!(tree(&target.join("meetings")), audio);
    }
}

#[test]
fn flac_relocation_source_capture_cancel_and_busy_converter_never_mint_ownership() {
    let f = failed();
    let original = tree(f.base.parent().unwrap());
    let snapshot = f.storage.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(super::super::relocation::Source::capture(&f.storage, &snapshot, &|| true).is_err());
    let directory = Directory::open(&f.base).unwrap();
    let lock = directory.required(LOCK, false).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let error = match super::super::relocation::Source::capture(&f.storage, &snapshot, &|| false) {
        Ok(_) => panic!("busy converter granted FLAC handoff"),
        Err(error) => error,
    };
    assert!(error.contains("FLAC変換中"), "{error}");
    drop(lock);
    assert!(
        super::super::relocation::Source::capture(&f.storage, &snapshot, &|| false)
            .unwrap()
            .is_some()
    );
    assert_eq!(tree(f.base.parent().unwrap()), original);
}

#[test]
#[ignore = "Owned native-bootstrap SIGKILL child, launched only by its dedicated parent"]
fn flac_relocation_crash_child() {
    assert_eq!(
        std::env::var("MON_FINAL_FLAC_CRASH_CHILD").unwrap(),
        "synthetic-only"
    );
    let (config, legacy): (PathBuf, PathBuf) =
        serde_json::from_str(&std::env::var("MON_FINAL_FLAC_RELOCATION_BOOTSTRAP").unwrap())
            .unwrap();
    let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    storage
        .finalize_recording(&std::env::var("MON_FINAL_FLAC_CRASH_OWNER").unwrap())
        .unwrap();
    panic!("owned relocation child missed its crash boundary");
}

#[test]
#[ignore = "Owned partial-recovery native-bootstrap SIGKILL child; dedicated parent only"]
fn flac_relocation_partial_crash_child() {
    assert_eq!(
        std::env::var("MON_FINAL_FLAC_CRASH_CHILD").unwrap(),
        "synthetic-only"
    );
    let (config, legacy): (PathBuf, PathBuf) =
        serde_json::from_str(&std::env::var("MON_FINAL_FLAC_RELOCATION_BOOTSTRAP").unwrap())
            .unwrap();
    let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
    let storage = bootstrap.open_storage().unwrap();
    let review = storage
        .review_partial_recording(&std::env::var("MON_FINAL_FLAC_CRASH_OWNER").unwrap())
        .unwrap();
    storage.recover_partial_recording(&review, true).unwrap();
    panic!("owned partial relocation child missed its crash boundary");
}

fn kill_partial_relocation(config: &Path, source: &Path, owner: &str, phase: &str) {
    use std::os::unix::process::ExitStatusExt;
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "storage::recordings::flac::tests::relocation::flac_relocation_partial_crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
        .env("MON_FINAL_FLAC_CRASH_PHASE", phase)
        .env(
            "MON_FINAL_FLAC_RELOCATION_BOOTSTRAP",
            serde_json::to_string(&(config, source)).unwrap(),
        )
        .env("MON_FINAL_FLAC_CRASH_OWNER", owner)
        .output()
        .unwrap();
    assert_eq!(
        child.status.signal(),
        Some(libc::SIGKILL),
        "{phase}: {} {}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

#[test]
#[ignore = "Dedicated owned partial SIGKILL, native copy and fresh-consent cold resume"]
fn flac_relocation_partial_owned_sigkill_resumes_with_fresh_consent_and_current_document() {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "reserved",
        "created-before-identity",
        "allocated",
        "encoded",
        "published",
        "partial-before-db-commit",
    ] {
        let Fixture {
            temp,
            storage,
            session,
            base,
        } = partial_fixture();
        let source = storage.root().to_owned();
        drop(storage);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::recordings::flac::tests::partial_recovery_crash_child",
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
            "{phase}: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let original = tree(base.parent().unwrap());
        let plan_bytes = std::fs::read(base.join(PARTIAL_PLAN)).unwrap();
        let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
        let track = &plan.tracks[0];
        let id = &track.chunks[0];
        let checkpoint_name = format!(".{id}.json");
        let checkpoint: Checkpoint = serde_json::from_slice(
            &std::fs::read(
                base.parent()
                    .unwrap()
                    .join(&track.id)
                    .join(&checkpoint_name),
            )
            .unwrap(),
        )
        .unwrap();
        let control = control_temp();
        let config = control.path().join("config");
        let target = control.path().join("target");
        let storage = move_storage(
            Storage::open(source.clone()).unwrap(),
            &config,
            &source,
            &target,
        );
        let first_review = storage.review_partial_recording(&session.id).unwrap();
        let copied_directory = Directory::open(&target.join(format!(
            "meetings/{}/audio/{}",
            session.meeting_id, track.id
        )))
        .unwrap();
        let body_name = if matches!(phase, "published" | "partial-before-db-commit") {
            format!("{id}.flac")
        } else {
            checkpoint.staging_name.clone()
        };
        let copied_identity = checkpoint.staging_identity.as_ref().map(|_| {
            Identity::file(&copied_directory.required(&body_name, false).unwrap()).unwrap()
        });
        drop(storage);
        if let Some(identity) = &copied_identity {
            kill_partial_relocation(&config, &source, &session.id, "relocation-rebound");
            let local: Checkpoint = serde_json::from_slice(
                &copied_directory
                    .read_json(&checkpoint_name)
                    .unwrap()
                    .unwrap()
                    .0,
            )
            .unwrap();
            assert_eq!(local.staging_identity.as_ref(), Some(identity));
        }
        if phase == "partial-before-db-commit" {
            kill_partial_relocation(&config, &source, &session.id, "partial-relocation-document");
            let document: Value = serde_json::from_slice(
                &std::fs::read(
                    target
                        .join(base.strip_prefix(&source).unwrap())
                        .join("partial-recovery.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                document["partialRecovery"]["reviewHash"],
                first_review.summary.review_hash
            );
        }
        let bootstrap = Bootstrap::open(&config, &source).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        assert!(storage.recover_completed_recordings().unwrap().is_empty());
        let review = storage.review_partial_recording(&session.id).unwrap();
        let before = tree(&target.join("meetings"));
        assert!(storage.recover_partial_recording(&review, false).is_err());
        assert_eq!(tree(&target.join("meetings")), before);
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM audio_tracks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let result = storage.recover_partial_recording(&review, true).unwrap();
        assert_eq!(result.state, "interrupted");
        assert_eq!(result.ended_at_utc_ms, None);
        assert_eq!(
            storage
                .partial_recovery_result(&session.id, &review.summary.review_hash)
                .unwrap(),
            Some(review.summary)
        );
        if let Some(identity) = copied_identity {
            identity
                .require(
                    &copied_directory
                        .required(&format!("{id}.flac"), false)
                        .unwrap(),
                )
                .unwrap();
        } else if phase == "created-before-identity" {
            assert_eq!(
                std::fs::read(copied_directory.path.join(&checkpoint.staging_name)).unwrap(),
                b""
            );
        }
        assert_eq!(
            std::fs::read(
                target
                    .join(base.strip_prefix(&source).unwrap())
                    .join(PARTIAL_PLAN)
            )
            .unwrap(),
            plan_bytes
        );
        assert_eq!(tree(base.parent().unwrap()), original);
        assert_partial_whole_meeting_playback(&storage, &session.meeting_id, result.duration_us);
    }
}

#[test]
#[ignore = "Dedicated owned SIGKILL, placement and cold native resume; synthetic PCM only"]
fn flac_relocation_owned_sigkill_cold_resume_keeps_receipts_and_unknown_files() {
    use std::os::unix::process::ExitStatusExt;
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
        } = Fixture::new(1, 2000, false);
        let source = storage.root().to_owned();
        drop(storage);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::recordings::flac::tests::final_flac_crash_child",
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
            "{phase}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let original = tree(base.parent().unwrap());
        let plan_bytes = std::fs::read(base.join(PLAN)).unwrap();
        let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
        let track = &plan.tracks[0];
        let id = &track.chunks[0];
        let checkpoint_name = format!(".{id}.json");
        let checkpoint: Checkpoint = serde_json::from_slice(
            &std::fs::read(
                base.parent()
                    .unwrap()
                    .join(&track.id)
                    .join(&checkpoint_name),
            )
            .unwrap(),
        )
        .unwrap();
        let control = control_temp();
        let config = control.path().join("config");
        let target = control.path().join("target");
        let storage = move_storage(
            Storage::open(source.clone()).unwrap(),
            &config,
            &source,
            &target,
        );
        let copied_directory = Directory::open(&target.join(format!(
            "meetings/{}/audio/{}",
            session.meeting_id, track.id
        )))
        .unwrap();
        let copied_identity = checkpoint.staging_identity.as_ref().map(|_| {
            let audio_name = if phase == "published" {
                format!("{id}.flac")
            } else {
                checkpoint.staging_name.clone()
            };
            Identity::file(&copied_directory.required(&audio_name, false).unwrap()).unwrap()
        });
        drop(storage);
        if let Some(identity) = &copied_identity {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "storage::recordings::flac::tests::relocation::flac_relocation_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
                .env("MON_FINAL_FLAC_CRASH_PHASE", "relocation-rebound")
                .env(
                    "MON_FINAL_FLAC_RELOCATION_BOOTSTRAP",
                    serde_json::to_string(&(&config, &source)).unwrap(),
                )
                .env("MON_FINAL_FLAC_CRASH_OWNER", &session.id)
                .output()
                .unwrap();
            assert_eq!(
                child.status.signal(),
                Some(libc::SIGKILL),
                "rebound/{phase}: {}",
                String::from_utf8_lossy(&child.stderr)
            );
            let local: Checkpoint = serde_json::from_slice(
                &copied_directory
                    .read_json(&checkpoint_name)
                    .unwrap()
                    .unwrap()
                    .0,
            )
            .unwrap();
            assert_eq!(local.staging_identity.as_ref(), Some(identity));
        }
        let bootstrap = Bootstrap::open(&config, &source).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        assert_eq!(
            storage.recover_completed_recordings().unwrap(),
            vec![session.id.clone()]
        );
        if let Some(identity) = copied_identity {
            identity
                .require(
                    &copied_directory
                        .required(&format!("{id}.flac"), false)
                        .unwrap(),
                )
                .unwrap();
        } else if phase == "created-before-identity" {
            assert_eq!(
                std::fs::read(copied_directory.path.join(checkpoint.staging_name)).unwrap(),
                b""
            );
        }
        assert_eq!(
            std::fs::read(target.join(base.strip_prefix(&source).unwrap()).join(PLAN)).unwrap(),
            plan_bytes
        );
        assert_eq!(tree(base.parent().unwrap()), original);
        assert_eq!(
            storage
                .recorded_audio_plan(&session.meeting_id)
                .unwrap()
                .tracks
                .len(),
            2
        );
    }
}
