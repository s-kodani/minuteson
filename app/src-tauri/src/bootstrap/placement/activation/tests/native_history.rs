//! Real owned snapshots, journals, bootstrap and Storage; synthetic audio only.
//! Test selection bypasses only the still-required public cutover gate. It
//! retains all actual copy proofs, source retirement and durable config writes.
use super::super::*;
use crate::{
    analysis_worker::AnalysisController,
    storage::deletion::{audio, operations},
};
use rusqlite::config::DbConfig;
use std::{
    os::unix::fs::{symlink, DirBuilderExt},
    path::PathBuf,
};

struct NativeFixture {
    _base: tempfile::TempDir,
    bootstrap: Option<Bootstrap>,
    storage: Option<Storage>,
    config: PathBuf,
    legacy: PathBuf,
    target: PathBuf,
    operation: String,
    meeting: String,
    deletion: String,
    audio: String,
    full_deletion: Option<String>,
}
impl NativeFixture {
    fn new(kept: bool, full: bool) -> Self {
        let base = tempfile::tempdir().unwrap();
        let parent = base.path().canonicalize().unwrap();
        let config = parent.join("config");
        let legacy = parent.join("app-data");
        let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        let meeting = storage
            .create_meeting("SYNTHETIC NATIVE HISTORY")
            .unwrap()
            .id;
        let audio = format!("meetings/{meeting}/audio/source.flac");
        let file = storage.root().join(&audio);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"SYNTHETIC AUDIO").unwrap();
        std::fs::write(
            file.parent().unwrap().join("retained.txt"),
            b"SYNTHETIC TEXT",
        )
        .unwrap();
        storage
            .register_ready_meeting_asset(&meeting, Path::new(&audio), "audio/flac")
            .unwrap();
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        let deletion = Uuid::new_v4().to_string();
        audio::intent::prepare(
            &storage,
            &deletion,
            &audio::Census::capture(&storage, &meeting, &|| false).unwrap(),
            audio::intent::Policy {
                include_unregistered_audio: true,
                retain_unclassified: true,
            },
            &lease,
            &|| false,
        )
        .unwrap();
        if kept {
            audio::intent::keep_unstarted(&storage, &deletion, &lease, &|| false).unwrap();
        } else {
            audio::intent::removal::start(&storage, &deletion, &lease, &|| false).unwrap();
            audio::intent::removal::run(&storage, &deletion, &lease, &|| false).unwrap();
            audio::intent::removal::completion::finish(&storage, &deletion, &lease, &|| false)
                .unwrap();
        }
        drop(lease);
        let mut full_deletion = None;
        if full {
            let mut p = operations::Prepared::fresh(&storage, &meeting, &|| false).unwrap();
            let view = p.preview();
            full_deletion = Some(view.operation_id.clone());
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                p.execute(
                    &storage,
                    &lease,
                    &view.confirmation_id,
                    true,
                    &|| false,
                    &|_| {}
                )
                .unwrap()
                .deleted
            );
        }
        let target = parent.join("target");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .unwrap();
        Self {
            _base: base,
            bootstrap: Some(bootstrap),
            storage: Some(storage),
            config,
            legacy,
            target,
            operation: Uuid::new_v4().to_string(),
            meeting,
            deletion,
            audio,
            full_deletion,
        }
    }
    fn select(&mut self) {
        let storage = self.storage.as_ref().unwrap();
        let bootstrap = self.bootstrap.as_ref().unwrap();
        let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
        bootstrap
            .begin_move(
                &self.operation,
                &Uuid::new_v4().to_string(),
                &self.target,
                snapshot.manifest_hash(),
                0,
            )
            .unwrap();
        let mut p = bootstrap
            .pending_placement(&self.operation, &snapshot)
            .unwrap();
        p.copy(&snapshot, &|| false).unwrap();
        let h = snapshot.prepare_control_handoff(&|| false).unwrap();
        p.install_history(&h, &|| false).unwrap();
        p.inspect_deletion_handoff(&h, &|| false)
            .unwrap()
            .require_clean()
            .unwrap();
        assert!(h.require_supported_cutover().is_err());
        let (_, control_hash, _) = history::installed_control(&p).unwrap();
        let mut record = bootstrap
            .state
            .lock()
            .unwrap()
            .loaded
            .as_ref()
            .unwrap()
            .record
            .clone();
        record.current = Root::inspect(&self.target).unwrap();
        record.origin = Origin::Relocated;
        record.pending_move = None;
        record.activation = Some(Activation {
            pending: p.pending.clone(),
            control_hash,
        });
        record.revision += 1;
        snapshot.retire_source_for_restart(&|| false).unwrap();
        bootstrap
            .write(&mut bootstrap.state.lock().unwrap(), record)
            .unwrap();
        drop(p);
        drop(h);
        drop(snapshot);
        drop(self.storage.take());
        drop(self.bootstrap.take());
    }
    fn open(&self) -> (Bootstrap, Storage) {
        let b = Bootstrap::open(&self.config, &self.legacy).unwrap();
        let storage = b.open_storage().unwrap();
        (b, storage)
    }
    fn record(&self) -> String {
        let db = Connection::open_with_flags(
            self.target.join("meetings.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        self.read_record(&db)
    }
    fn live_record(&self, storage: &Storage) -> String {
        crate::storage::deletion::relocation::receiver::synthetic_consent_record(
            storage,
            &self.deletion,
        )
    }
    fn read_record(&self, db: &Connection) -> String {
        db.query_row("SELECT json_array(target_manifest_json,target_hash,consent_at,created_at) FROM maintenance_operations WHERE id=?1",[&self.deletion],|r|r.get(0)).unwrap()
    }
}

mod audio_reconfirmation;
mod model_reconfirmation;
mod reconfirmation;
mod repeated;

#[test]
fn deletion_startup_admission_real_bootstrap_reads_completed_audio_and_full_supersession_without_rewriting_consent(
) {
    for full in [false, true] {
        let mut f = NativeFixture::new(false, full);
        f.select();
        let before = f.record();
        for _ in 0..2 {
            let (b, storage) = f.open();
            assert_eq!(storage.root(), f.target);
            assert!(!storage.data_write_blocked());
            assert!(storage.recorded_audio_plan(&f.meeting).is_err());
            assert_eq!(f.live_record(&storage), before);
            let other = storage.create_meeting("UNRELATED CURRENT ROOT").unwrap();
            assert!(storage.get_meeting(&other.id).unwrap().is_some());
            let mut delete_other =
                operations::Prepared::fresh(&storage, &other.id, &|| false).unwrap();
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                delete_other
                    .execute(
                        &storage,
                        &lease,
                        &delete_other.preview().confirmation_id,
                        true,
                        &|| false,
                        &|_| {}
                    )
                    .unwrap()
                    .deleted
            );
            drop(lease);
            assert!(storage.get_meeting(&other.id).unwrap().is_none());
            assert_eq!(f.live_record(&storage), before);
            let backup = f
                ._base
                .path()
                .join(format!("{}-{}.monbackup", full, Uuid::new_v4()));
            let exported = storage.create_backup_archive(&backup, &|| false).unwrap();
            assert_eq!(exported.deleted_audio_meeting_count, usize::from(!full));
            assert!(!f.target.join(&f.audio).exists());
            drop(storage);
            drop(b);
        }
    }
}

#[test]
fn deletion_startup_admission_kept_audio_remains_present_and_old_consent_cannot_resume() {
    let mut f = NativeFixture::new(true, false);
    f.select();
    let before = f.record();
    let (_b, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert_eq!(
        std::fs::read(f.target.join(&f.audio)).unwrap(),
        b"SYNTHETIC AUDIO"
    );
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(audio::intent::removal::start(&storage, &f.deletion, &lease, &|| false).is_err());
    assert_eq!(f.live_record(&storage), before);
    assert!(storage
        .create_meeting("KEEP DOES NOT BECOME CONSENT")
        .is_ok());
}

#[test]
fn deletion_startup_admission_kept_whole_meeting_is_readable_without_current_root_consent() {
    use crate::storage::deletion::relocation::receiver::{
        synthetic_consent_record, synthetic_keep_meeting,
    };
    let mut f = NativeFixture::new(true, false);
    let operation = synthetic_keep_meeting(f.storage.as_ref().unwrap(), &f.meeting);
    let before = synthetic_consent_record(f.storage.as_ref().unwrap(), &operation);
    f.select();
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        let mut p = operations::Prepared::pending(&storage, &operation, &|| false).unwrap();
        assert!(!p.preview().execute_allowed);
        let lease = AnalysisController::default()
            .begin_data_maintenance(false, false, &|| false)
            .unwrap();
        assert!(p
            .execute(
                &storage,
                &lease,
                &p.preview().confirmation_id,
                true,
                &|| false,
                &|_| {}
            )
            .is_err());
        assert_eq!(synthetic_consent_record(&storage, &operation), before);
        assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
        assert_eq!(
            std::fs::read(f.target.join(&f.audio)).unwrap(),
            b"SYNTHETIC AUDIO"
        );
        drop(lease);
        drop(storage);
        drop(b);
    }
}

#[test]
fn deletion_startup_admission_reappearance_is_fenced_without_deleting_new_body_or_reusing_old_consent(
) {
    let mut f = NativeFixture::new(false, false);
    f.select();
    drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
    let file = f.target.join(&f.audio);
    std::fs::write(&file, b"NEW BODY NEVER DELETE").unwrap();
    let before = f.record();
    let (_b, storage) = f.open();
    assert!(storage.data_write_blocked());
    assert!(storage.create_meeting("BLOCKED").is_err());
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    assert!(audio::intent::removal::start(&storage, &f.deletion, &lease, &|| false).is_err());
    assert_eq!(std::fs::read(file).unwrap(), b"NEW BODY NEVER DELETE");
    assert_eq!(f.live_record(&storage), before);
}

#[test]
fn deletion_startup_admission_real_storage_crash_reads_wal_only_and_keeps_latest_committed_meeting()
{
    let mut f = NativeFixture::new(false, false);
    f.select();
    let before = f.record();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bootstrap::placement::activation::tests::native_history::deletion_startup_admission_storage_crash_child", "--ignored", "--nocapture"])
        .env("MON_SYNTHETIC_STARTUP_BOOTSTRAP", serde_json::to_string(&(&f.config, &f.legacy)).unwrap()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        f.target
            .join("meetings.sqlite3-wal")
            .metadata()
            .unwrap()
            .len()
            > 32
    );
    assert!(!f.target.join("meetings.sqlite3-shm").exists());
    for _ in 0..2 {
        let (b, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert!(storage
            .list_meetings()
            .unwrap()
            .iter()
            .any(|m| m.title == "LATEST COMMITTED IN WAL"));
        assert_eq!(f.live_record(&storage), before);
        assert!(storage.recorded_audio_plan(&f.meeting).is_err());
        assert!(!f.target.join(&f.audio).exists());
        drop(storage);
        drop(b);
    }
}

#[test]
#[ignore = "child process only; actual owned bootstrap fixture handoff required"]
fn deletion_startup_admission_storage_crash_child() {
    let (config, legacy): (PathBuf, PathBuf) = serde_json::from_str(
        &std::env::var("MON_SYNTHETIC_STARTUP_BOOTSTRAP").expect("synthetic bootstrap handoff"),
    )
    .unwrap();
    let b = Bootstrap::open(&config, &legacy).unwrap();
    let storage = b.open_storage().unwrap();
    storage.create_meeting("LATEST COMMITTED IN WAL").unwrap();
    // Leave the real exclusive-mode Storage WAL, without running SQLite Drop.
    std::process::exit(0);
}

#[test]
fn deletion_startup_admission_rejects_changed_immutable_consent_before_storage_mutation() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap(); // admit actual image first
    drop(b);
    let db = Connection::open(f.target.join("meetings.sqlite3")).unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    db.execute("UPDATE maintenance_operations SET consent_at=consent_at+1,created_at=created_at+1,updated_at=updated_at+1 WHERE id=?1",[&f.deletion]).unwrap();
    drop(db);
    let changed = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
    assert!(b.open_storage().is_err());
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        changed
    );
    assert!(!f.target.join("meetings.sqlite3-wal").exists());
}

#[test]
fn deletion_startup_admission_mutable_progress_is_still_typed_and_never_a_success_hint() {
    for kept in [false, true] {
        let mut f = NativeFixture::new(kept, false);
        f.select();
        drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
        let before = f.record();
        let db = Connection::open(f.target.join("meetings.sqlite3")).unwrap();
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET progress_json='{}' WHERE id=?1",
            [&f.deletion],
        )
        .unwrap();
        drop(db);
        let changed = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
        let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
        if kept {
            assert!(b.open_storage().is_err());
            assert_eq!(
                std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
                changed
            );
            assert_eq!(
                std::fs::read(f.target.join(&f.audio)).unwrap(),
                b"SYNTHETIC AUDIO"
            );
        } else {
            let storage = b.open_storage().unwrap();
            assert!(storage.data_write_blocked());
            assert!(storage.create_meeting("NO SUCCESS HINT").is_err());
            assert_eq!(f.live_record(&storage), before);
        }
    }
}

#[test]
fn deletion_startup_admission_forged_binding_cannot_adopt_an_unowned_directory_identity() {
    let mut f = NativeFixture::new(false, false);
    f.select();
    drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
    let registry = f.target.join(".mon-scratch.sqlite3");
    let db = Connection::open(&registry).unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
        .unwrap();
    let json: String = db
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            [&f.operation],
            |r| r.get(0),
        )
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&json).unwrap();
    json["deletion_binding"]["directories"][0]["copied"]["inode"] = 0.into();
    let json = serde_json::to_string(&json).unwrap();
    db.execute(
        "UPDATE relocation_history SET local_json=?1,local_hash=?2 WHERE operation_id=?3",
        params![json, hash(json.as_bytes()), f.operation],
    )
    .unwrap();
    drop(db);
    let native = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    let changed = std::fs::read(&registry).unwrap();
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
    assert!(b.open_storage().is_err());
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        native
    );
    assert_eq!(std::fs::read(&registry).unwrap(), changed);
}

#[test]
fn deletion_startup_admission_missing_or_linked_registry_is_not_recreated() {
    for linked in [false, true] {
        let mut f = NativeFixture::new(false, false);
        f.select();
        drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
        let registry = f.target.join(".mon-scratch.sqlite3");
        let retained = registry.with_extension("retained");
        std::fs::rename(&registry, &retained).unwrap();
        if linked {
            symlink(&retained, &registry).unwrap();
        }
        let native = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
        assert!(Bootstrap::open(&f.config, &f.legacy).is_err());
        assert_eq!(
            std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
            native
        );
        assert!(retained.exists());
        if !linked {
            assert!(!registry.exists());
        }
    }
}

#[test]
fn deletion_startup_admission_restored_unavailable_audio_and_supersession_survive_restart_and_rebackup(
) {
    use crate::storage::deletion::relocation::receiver::{
        synthetic_consent_record, synthetic_restored_audio_operation,
    };
    for full in [false, true] {
        let mut f = NativeFixture::new(false, false);
        let storage = f.storage.as_ref().unwrap();
        let archive = f._base.path().join("restore-input.monbackup");
        storage.create_backup_archive(&archive, &|| false).unwrap();
        let selected = storage
            .prepare_additive_restore(&archive, &|| false, &|_| {})
            .unwrap();
        let restored = selected
            .publish(
                storage,
                &selected.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap()
            .meeting_ids[0]
            .clone();
        drop(selected);
        let operation = synthetic_restored_audio_operation(storage, &restored);
        let before = synthetic_consent_record(storage, &operation);
        if full {
            let mut prepared = operations::Prepared::fresh(storage, &restored, &|| false).unwrap();
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                prepared
                    .execute(
                        storage,
                        &lease,
                        &prepared.preview().confirmation_id,
                        true,
                        &|| false,
                        &|_| {}
                    )
                    .unwrap()
                    .deleted
            );
        }
        f.select();
        for _ in 0..2 {
            let (b, storage) = f.open();
            assert!(!storage.data_write_blocked());
            assert_eq!(synthetic_consent_record(&storage, &operation), before);
            assert!(storage.recorded_audio_plan(&restored).is_err());
            let archive = f
                ._base
                .path()
                .join(format!("rebackup-{}.monbackup", Uuid::new_v4()));
            assert_eq!(
                storage
                    .create_backup_archive(&archive, &|| false)
                    .unwrap()
                    .deleted_audio_meeting_count,
                if full { 1 } else { 2 }
            );
            drop(storage);
            drop(b);
        }
    }
}

fn synthetic_model(f: &NativeFixture, completed: bool) -> (String, PathBuf) {
    use crate::{
        model_inspection::{self, Engine, PackageFile, PackageManifest, Role, Target},
        model_placement,
        storage::deletion::models,
    };
    let storage = f.storage.as_ref().unwrap();
    let root = &f.bootstrap.as_ref().unwrap().models_root;
    let source = f._base.path().join("synthetic-model-source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("cli"), b"SYNTHETIC DO NOT EXECUTE").unwrap();
    std::fs::write(source.join("weights.bin"), b"SYNTHETIC MODEL").unwrap();
    let manifest = PackageManifest {
        schema_version: 1,
        engine: Engine::Whisper,
        engine_version: "test".into(),
        model_name: "SYNTHETIC".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "test only".into(),
        target: Target::current().unwrap(),
        files: vec![
            PackageFile {
                relative_path: "cli".into(),
                role: Role::Executable,
                expected_size: None,
                expected_hash: None,
            },
            PackageFile {
                relative_path: "weights.bin".into(),
                role: Role::Asr,
                expected_size: None,
                expected_hash: None,
            },
        ],
    };
    std::fs::write(
        source.join(model_inspection::MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let report = model_inspection::inspect(&source, &|| false).unwrap();
    let package = Uuid::new_v4().to_string();
    model_placement::place(
        storage,
        root,
        &source,
        &package,
        &report.review_hash,
        &|| false,
    )
    .unwrap();
    let preview = models::preview(
        storage,
        root,
        &Uuid::new_v4().to_string(),
        &package,
        &report.review_hash,
        &|| false,
    )
    .unwrap();
    let operation = Uuid::new_v4().to_string();
    let lease = AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    models::intent::prepare(
        storage,
        root,
        &operation,
        &package,
        &report.review_hash,
        &preview.census_hash,
        false,
        &lease,
        &|| false,
    )
    .unwrap();
    if completed {
        models::intent::removal::start(storage, &operation, &lease, &|| false).unwrap();
        models::intent::removal::run(storage, &operation, &lease, &|| false).unwrap();
    } else {
        models::intent::keep_unstarted(storage, &operation, &lease, &|| false).unwrap();
    }
    (operation, root.join(package).join("weights.bin"))
}

#[test]
fn deletion_startup_admission_models_use_only_native_root_and_never_moved_consent() {
    use crate::storage::deletion::{models, relocation::receiver::synthetic_consent_record};
    for completed in [false, true] {
        let mut f = NativeFixture::new(false, false);
        let (operation, file) = synthetic_model(&f, completed);
        let before = synthetic_consent_record(f.storage.as_ref().unwrap(), &operation);
        f.select();
        for _ in 0..2 {
            let (b, storage) = f.open();
            assert!(!storage.data_write_blocked());
            assert_eq!(synthetic_consent_record(&storage, &operation), before);
            assert_eq!(file.exists(), !completed);
            let lease = AnalysisController::default()
                .begin_data_maintenance(false, false, &|| false)
                .unwrap();
            assert!(
                models::intent::removal::start(&storage, &operation, &lease, &|| false).is_err()
            );
            assert!(storage.create_meeting("MODEL ROOT NOT RELOCATED").is_ok());
            drop(lease);
            drop(storage);
            drop(b);
        }
    }
}

#[test]
fn deletion_startup_admission_substituted_native_models_root_is_rejected_before_database_mutation()
{
    let mut f = NativeFixture::new(false, false);
    let (_, file) = synthetic_model(&f, false);
    let root = f.bootstrap.as_ref().unwrap().models_root.clone();
    f.select();
    drop(Bootstrap::open(&f.config, &f.legacy).unwrap());
    let retained = root.with_extension("retained");
    std::fs::rename(&root, &retained).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    std::fs::write(root.join("unowned.bin"), b"UNOWNED DO NOT DELETE").unwrap();
    let before = std::fs::read(f.target.join("meetings.sqlite3")).unwrap();
    let b = Bootstrap::open(&f.config, &f.legacy).unwrap();
    assert!(b.open_storage().is_err());
    assert_eq!(
        std::fs::read(f.target.join("meetings.sqlite3")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(retained.join(file.strip_prefix(&root).unwrap())).unwrap(),
        b"SYNTHETIC MODEL"
    );
    assert_eq!(
        std::fs::read(root.join("unowned.bin")).unwrap(),
        b"UNOWNED DO NOT DELETE"
    );
}
