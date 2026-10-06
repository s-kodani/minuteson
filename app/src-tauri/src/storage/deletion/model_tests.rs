//! Synthetic model files only; no real model, recording or engine execution.
use super::*;
use crate::model_inspection::{self, Engine, PackageFile, PackageManifest, Role, Target};
use crate::model_placement;
use std::{cell::Cell, path::PathBuf};

#[path = "model_intent_tests.rs"]
mod durable;

pub(crate) struct Fixture {
    _source: tempfile::TempDir,
    _meetings: tempfile::TempDir,
    _app_data: tempfile::TempDir,
    source: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) package: String,
    pub(crate) review: String,
    pub(crate) storage: Storage,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let source = tempfile::tempdir().unwrap();
        let meetings = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        let source_path = source.path().canonicalize().unwrap();
        std::fs::create_dir(source_path.join("weights")).unwrap();
        std::fs::write(source_path.join("cli"), b"synthetic; do not execute").unwrap();
        std::fs::write(source_path.join("weights/model.bin"), vec![b'm'; 150000]).unwrap();
        let manifest = PackageManifest {
            schema_version: 1,
            engine: Engine::Whisper,
            engine_version: "test".into(),
            model_name: "合成モデル".into(),
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
                    relative_path: "weights/model.bin".into(),
                    role: Role::Asr,
                    expected_size: None,
                    expected_hash: None,
                },
            ],
        };
        std::fs::write(
            source_path.join(MANIFEST),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let report = model_inspection::inspect(&source_path, &|| false).unwrap();
        let storage = Storage::open(meetings.path().into()).unwrap();
        storage.create_meeting("KEEP").unwrap();
        let root = app_data.path().canonicalize().unwrap().join("models");
        let package = uuid::Uuid::new_v4().to_string();
        model_placement::place(
            &storage,
            &root,
            &source_path,
            &package,
            &report.review_hash,
            &|| false,
        )
        .unwrap();
        Self {
            _source: source,
            _meetings: meetings,
            _app_data: app_data,
            source: source_path,
            root,
            package,
            review: report.review_hash,
            storage,
        }
    }
    fn preview(&self) -> Result<Preview, String> {
        preview(
            &self.storage,
            &self.root,
            &uuid::Uuid::new_v4().to_string(),
            &self.package,
            &self.review,
            &|| false,
        )
    }
    pub(crate) fn evidence(&self) -> String {
        let db = self.storage.connection.lock().unwrap();
        db.query_row("SELECT json_array((SELECT json_group_array(json_array(id,manifest_json,manifest_hash,state)) FROM model_packages),(SELECT json_group_array(json_array(id,package_id,relative_path,actual_hash,state)) FROM model_files),(SELECT count(*) FROM maintenance_operations),(SELECT count(*) FROM model_compatibility_checks),(SELECT count(*) FROM meetings),(SELECT json_group_array(json_array(id,state,profile_snapshot_json)) FROM processing_jobs))",[],|r|r.get(0)).unwrap()
    }
    pub(crate) fn operation_count(&self) -> i64 {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM maintenance_operations", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    pub(crate) fn operation_state(&self, id: &str) -> String {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM maintenance_operations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
    pub(crate) fn cold(self) -> Self {
        let Self {
            _source,
            _meetings,
            _app_data,
            source,
            root,
            package,
            review,
            storage,
        } = self;
        drop(storage);
        let storage = Storage::open_with_models(_meetings.path().into(), root.clone()).unwrap();
        Self {
            _source,
            _meetings,
            _app_data,
            source,
            root,
            package,
            review,
            storage,
        }
    }
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        if relative.is_empty() {
            self.root.join(&self.package)
        } else {
            self.root.join(&self.package).join(relative)
        }
    }
}

#[test]
fn model_deletion_census_is_read_only_exact_sorted_and_never_execution_authority() {
    let f = Fixture::new();
    let before = f.evidence();
    let a = f.preview().unwrap();
    let b = f.preview().unwrap();
    assert!(!a.execute_allowed && a.inspection_id != b.inspection_id);
    assert_eq!(a.census_hash, b.census_hash);
    assert_eq!(a.entries.len(), 5);
    assert_eq!(
        (a.missing_files, a.changed_files, a.unregistered_files),
        (0, 0, 0)
    );
    assert!(a.entries.iter().all(|e| e.registered && e.present));
    assert!(a
        .entries
        .windows(2)
        .all(|w| w[0].relative_path < w[1].relative_path));
    assert_eq!(
        a.bytes,
        a.entries.iter().filter_map(|e| e.bytes).sum::<u64>()
    );
    assert_eq!(f.evidence(), before);
    assert_eq!(
        std::fs::read(f.path("weights/model.bin")).unwrap(),
        vec![b'm'; 150000]
    );
    assert_eq!(f.storage.list_meetings().unwrap().len(), 1);
}
#[test]
fn model_deletion_census_classifies_missing_changed_and_unknown_without_adopting_them() {
    let f = Fixture::new();
    let old = f.preview().unwrap().census_hash;
    std::fs::remove_file(f.path("weights/model.bin")).unwrap();
    std::fs::write(f.path("cli"), b"CHANGED").unwrap();
    std::fs::write(f.path("unknown"), b"UNKNOWN").unwrap();
    let before = f.evidence();
    let p = f.preview().unwrap();
    assert_eq!(
        (p.missing_files, p.changed_files, p.unregistered_files),
        (1, 1, 1)
    );
    assert_ne!(old, p.census_hash);
    let missing = p
        .entries
        .iter()
        .find(|e| e.relative_path == "weights/model.bin")
        .unwrap();
    assert!(missing.registered && !missing.present && missing.sha256.is_none());
    assert_eq!(std::fs::read(f.path("unknown")).unwrap(), b"UNKNOWN");
    assert_eq!(f.evidence(), before);
}
#[test]
fn model_deletion_census_does_not_enumerate_or_read_other_packages_or_external_sources() {
    let f = Fixture::new();
    let neighbor = f.root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&neighbor).unwrap();
    std::os::unix::fs::symlink("/not-readable", neighbor.join("keep")).unwrap();
    std::fs::write(f.source.join("cli"), b"SOURCE CHANGED").unwrap();
    let p = f.preview().unwrap();
    assert_eq!(p.changed_files, 0);
    assert!(neighbor
        .join("keep")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
}
#[test]
fn model_deletion_census_rejects_leaf_parent_package_links_and_hardlinks() {
    for kind in ["leaf", "parent", "package", "hardlink"] {
        let f = Fixture::new();
        let before = f.evidence();
        let held = f.root.parent().unwrap().join("held");
        if kind == "package" {
            std::fs::rename(f.path(""), &held).unwrap();
            std::os::unix::fs::symlink(&held, f.path("")).unwrap();
        } else if kind == "parent" {
            std::fs::rename(f.path("weights"), &held).unwrap();
            std::os::unix::fs::symlink(&held, f.path("weights")).unwrap();
        } else if kind == "leaf" {
            std::fs::rename(f.path("cli"), &held).unwrap();
            std::os::unix::fs::symlink(&held, f.path("cli")).unwrap();
        } else {
            std::fs::hard_link(f.path("cli"), &held).unwrap();
        }
        assert!(f.preview().is_err(), "{kind}");
        assert_eq!(f.evidence(), before);
        assert!(held.symlink_metadata().is_ok());
    }
}
#[test]
fn model_deletion_census_rejects_wrong_identity_root_review_or_noncanonical_operation() {
    let f = Fixture::new();
    let before = f.evidence();
    let id = uuid::Uuid::new_v4().to_string();
    for (root, pkg, review, token) in [
        (&f.root, "bad", f.review.as_str(), id.as_str()),
        (&f.root, f.package.as_str(), "wrong", id.as_str()),
        (&f.root, f.package.as_str(), f.review.as_str(), "bad"),
        (
            &f.storage.root().join("models"),
            f.package.as_str(),
            f.review.as_str(),
            id.as_str(),
        ),
        (
            &f.root.parent().unwrap().join("arbitrary"),
            f.package.as_str(),
            f.review.as_str(),
            id.as_str(),
        ),
    ] {
        assert!(preview(&f.storage, root, token, pkg, review, &|| false).is_err());
    }
    assert_eq!(f.evidence(), before);
    assert!(!f.storage.root().join("models").exists());
}
#[test]
fn model_deletion_census_reports_absent_package_without_recreating_or_relabeling_it() {
    let f = Fixture::new();
    let before = f.evidence();
    let held = f.root.join("held");
    std::fs::rename(f.path(""), &held).unwrap();
    let p = f.preview().unwrap();
    assert_eq!(p.missing_files, 3);
    assert_eq!(p.bytes, 0);
    assert!(!f.path("").exists());
    assert_eq!(f.evidence(), before);
    assert!(held.join("cli").exists());
}
#[test]
fn model_deletion_census_cancellation_and_inventory_limits_preserve_every_object() {
    let f = Fixture::new();
    let before = f.evidence();
    let calls = Cell::new(0);
    assert!(preview(
        &f.storage,
        &f.root,
        &uuid::Uuid::new_v4().to_string(),
        &f.package,
        &f.review,
        &|| {
            calls.set(calls.get() + 1);
            calls.get() > 6
        }
    )
    .is_err());
    for n in 0..MAX_ENTRIES {
        std::fs::write(f.path(&format!("extra-{n}")), b"KEEP").unwrap();
    }
    assert!(f.preview().unwrap_err().contains("上限"));
    assert_eq!(std::fs::read(f.path("extra-255")).unwrap(), b"KEEP");
    assert_eq!(f.evidence(), before);
}
#[test]
fn model_deletion_census_rejects_corrupt_readiness_and_file_directory_substitution() {
    let f = Fixture::new();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE model_packages SET state='ready' WHERE id=?1",
            params![f.package],
        )
        .unwrap();
    let before = f.evidence();
    assert!(f.preview().is_err());
    assert_eq!(f.evidence(), before);
    let f = Fixture::new();
    std::fs::remove_file(f.path("cli")).unwrap();
    std::fs::create_dir(f.path("cli")).unwrap();
    let before = f.evidence();
    assert!(f.preview().is_err());
    assert_eq!(f.evidence(), before);
}
#[test]
fn model_deletion_census_job_references_are_information_not_running_process_or_deletion_permission()
{
    let f = Fixture::new();
    let meeting = f.storage.list_meetings().unwrap()[0].id.clone();
    let db = f.storage.connection.lock().unwrap();
    for (id, state, field) in [
        ("active", "running", "managedModel"),
        ("paused", "paused", "managedModel"),
        ("history", "failed", "managedModel"),
    ] {
        let profile = serde_json::json!({field:{"package":{"packageId":f.package}}}).to_string();
        db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES(?1,?2,'transcription',?3,?1,?4,?5,'{}',?4,1,1)",params![id,meeting,state,"a".repeat(64),profile]).unwrap();
    }
    drop(db);
    let before = f.evidence();
    let p = f.preview().unwrap();
    assert_eq!((p.job_references, p.active_jobs), (3, 1));
    assert!(!p.execute_allowed);
    assert_eq!(f.evidence(), before);
}

#[test]
fn model_deletion_census_counts_historical_profile_components_without_changing_selection() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    for (id, version) in [("old", 1), ("current", 2)] {
        db.execute("INSERT INTO model_profiles(id,name,profile_version,state) VALUES(?1,'standard',?2,'preparing')",params![id,version]).unwrap();
        db.execute("INSERT INTO profile_components(profile_id,role,package_id,settings_json) VALUES(?1,'asr',?2,'{}')",params![id,f.package]).unwrap();
        db.execute(
            "UPDATE model_profiles SET state='ready' WHERE id=?1",
            params![id],
        )
        .unwrap();
    }
    let before: String = db.query_row("SELECT json_group_array(json_array(id,name,profile_version,state)) FROM model_profiles",[],|r|r.get(0)).unwrap();
    drop(db);
    let p = f.preview().unwrap();
    assert_eq!(p.profile_references, 2);
    assert!(!p.execute_allowed);
    let after: String = f.storage.connection.lock().unwrap().query_row("SELECT json_group_array(json_array(id,name,profile_version,state)) FROM model_profiles",[],|r|r.get(0)).unwrap();
    assert_eq!(before, after);
}

#[test]
fn model_deletion_census_counts_rebound_runtime_reference_once_per_job() {
    let f = Fixture::new();
    let meeting = f.storage.list_meetings().unwrap()[0].id.clone();
    let db = f.storage.connection.lock().unwrap();
    let original = serde_json::json!({"managedModel":{"package":{"packageId":"another-package"}}});
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES('parent',?1,'transcription','failed','parent',?2,?3,'{}',?2,1,1)",params![meeting,"a".repeat(64),original.to_string()]).unwrap();
    let mut runtime = original;
    runtime["runtimeModel"] =
        serde_json::json!({"schemaVersion":1,"package":{"packageId":f.package}});
    db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at,retry_of_job_id,attempt) VALUES('child',?1,'transcription','paused','child',?2,?3,'{}',?2,1,1,'parent',2)",params![meeting,"a".repeat(64),runtime.to_string()]).unwrap();
    drop(db);
    let before = f.evidence();
    assert_eq!(f.preview().unwrap().job_references, 1);
    assert_eq!(f.evidence(), before);
}

#[test]
fn model_deletion_census_rejects_mid_scan_change_and_root_replacement() {
    for replace_root in [false, true] {
        let f = Fixture::new();
        let before = f.evidence();
        let calls = Cell::new(0);
        let held = f.root.parent().unwrap().join("held");
        let result = preview(
            &f.storage,
            &f.root,
            &uuid::Uuid::new_v4().to_string(),
            &f.package,
            &f.review,
            &|| {
                calls.set(calls.get() + 1);
                if calls.get() == 7 {
                    if replace_root {
                        std::fs::rename(&f.root, &held).unwrap();
                        std::fs::create_dir(&f.root).unwrap();
                    } else {
                        std::fs::write(f.path("cli"), b"CHANGED DURING SCAN").unwrap();
                    }
                }
                false
            },
        );
        assert!(calls.get() >= 7);
        assert!(result.is_err(), "root replacement: {replace_root}");
        assert_eq!(f.evidence(), before);
        if replace_root {
            assert!(held.join(&f.package).join("cli").exists());
        }
    }
}

#[test]
fn model_deletion_census_preserves_unicode_paths_and_rejects_excessive_depth() {
    let f = Fixture::new();
    for path in ["\u{e000}", "\u{10000}", "日本語"] {
        std::fs::write(f.path(path), b"KEEP").unwrap();
    }
    let p = f.preview().unwrap();
    assert_eq!(p.unregistered_files, 3);
    assert!(p
        .entries
        .windows(2)
        .all(|w| w[0].relative_path < w[1].relative_path));
    let mut deep = f.path("");
    for _ in 0..=MAX_DEPTH {
        deep = deep.join("x");
        std::fs::create_dir(&deep).unwrap();
    }
    assert!(f.preview().unwrap_err().contains("上限"));
    assert!(deep.exists());
}

#[test]
fn model_deletion_census_keeps_partial_placement_evidence_instead_of_marking_it_ready() {
    let f = Fixture::new();
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE model_files SET state='unavailable' WHERE package_id=?1",
        params![f.package],
    )
    .unwrap();
    drop(db);
    std::fs::remove_file(f.path("weights/model.bin")).unwrap();
    let before = f.evidence();
    let p = f.preview().unwrap();
    assert_eq!(p.missing_files, 1);
    assert_eq!(f.evidence(), before);
    assert!(!p.execute_allowed);
}

#[test]
fn model_deletion_census_supports_exact_legacy_app_data_child_without_scanning_meetings_or_neighbors(
) {
    let f = Fixture::new();
    let legacy = Storage::open(f.root.parent().unwrap().to_path_buf()).unwrap();
    let meeting = legacy.create_meeting("LEGACY KEEP").unwrap();
    let package = uuid::Uuid::new_v4().to_string();
    model_placement::place(&legacy, &f.root, &f.source, &package, &f.review, &|| false).unwrap();
    let p = preview(
        &legacy,
        &f.root,
        &uuid::Uuid::new_v4().to_string(),
        &package,
        &f.review,
        &|| false,
    )
    .unwrap();
    assert_eq!(p.entries.len(), 5);
    assert_eq!(p.package_id, package);
    assert!(!p.execute_allowed);
    assert!(f.path("cli").exists());
    assert_eq!(legacy.list_meetings().unwrap()[0].id, meeting.id);
    let nested = legacy
        .root()
        .join("meetings")
        .join(&meeting.id)
        .join("models");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(preview(
        &legacy,
        &nested,
        &uuid::Uuid::new_v4().to_string(),
        &package,
        &f.review,
        &|| false
    )
    .is_err());
    assert!(nested.is_dir());
}
