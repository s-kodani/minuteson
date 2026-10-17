use super::{
    database::Scope,
    files::{Census, Manifest},
};
use crate::storage::Storage;
use rusqlite::params;
use std::cell::Cell;
use std::path::PathBuf;

struct Fixture {
    _base: tempfile::TempDir,
    storage: Storage,
    meeting: String,
    other: String,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        let storage = Storage::open(root).unwrap();
        let meeting = storage.create_meeting("対象会議").unwrap().id;
        let other = storage.create_meeting("残す会議").unwrap().id;
        let directory = storage.root().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        Self {
            _base: base,
            storage,
            meeting,
            other,
            directory,
        }
    }
    fn asset(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let full = self.directory.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        self.storage
            .register_ready_meeting_asset(
                &self.meeting,
                full.strip_prefix(self.storage.root()).unwrap(),
                "audio/flac",
            )
            .unwrap();
        full
    }
    fn census(&self) -> Census {
        Census::capture(
            &self.storage,
            &self.meeting,
            &crate::test_support::never_cancelled,
        )
        .unwrap()
    }
    fn job(&self, kind: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.storage.connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES(?1,?2,?3,'failed',?1,?4,'{}','{}',?4,1,1)",params![id,self.meeting,kind,"a".repeat(64)]).unwrap();
        id
    }
}

#[test]
fn deletion_file_census_is_fixed_read_only_and_contains_no_payload_or_other_meeting_files() {
    let f = Fixture::new();
    let path = f.asset("audio/chunk.flac", b"PRIVATE AUDIO AND TRANSCRIPT BODY");
    let external = f._base.path().join("external-export.md");
    std::fs::write(&external, b"EXTERNAL EXPORT").unwrap();
    let other_path = f
        .storage
        .root()
        .join("meetings")
        .join(&f.other)
        .join("keep.flac");
    std::fs::create_dir_all(other_path.parent().unwrap()).unwrap();
    std::fs::write(&other_path, b"OTHER MEETING").unwrap();
    let census = f.census();
    let entries = &census.manifest().entries;
    assert_eq!(entries.len(), 3);
    assert_eq!(entries.iter().filter(|e| !e.directory).count(), 1);
    let encoded = census.encode().unwrap();
    let json = std::str::from_utf8(&encoded).unwrap();
    assert!(!json.contains("PRIVATE AUDIO"));
    assert!(!json.contains(&f.other));
    assert!(!json.contains("対象会議"));
    let decoded: Manifest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, *census.manifest());
    assert_eq!(f.census().hash().unwrap(), census.hash().unwrap());
    census
        .verify(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"PRIVATE AUDIO AND TRANSCRIPT BODY"
    );
    assert_eq!(std::fs::read(external).unwrap(), b"EXTERNAL EXPORT");
    assert_eq!(std::fs::read(other_path).unwrap(), b"OTHER MEETING");
    assert_eq!(f.storage.list_meetings().unwrap().len(), 2);
}

#[test]
fn deletion_file_census_records_missing_corrupt_and_unknown_without_adopting_them() {
    let f = Fixture::new();
    let missing = f.asset("audio/missing.flac", b"MISSING");
    std::fs::remove_file(missing).unwrap();
    let changed = f.asset("audio/changed.flac", b"ORIGINAL");
    std::fs::write(&changed, b"DIFFERENT").unwrap();
    let unknown = f.directory.join("unknown.txt");
    std::fs::write(&unknown, b"UNKNOWN").unwrap();
    let census = f.census();
    let summary = census.summary().unwrap();
    assert_eq!(
        (
            summary.files,
            summary.missing,
            summary.changed,
            summary.unknown
        ),
        (3, 1, 1, 1)
    );
    let missing = census
        .manifest()
        .entries
        .iter()
        .find(|e| e.path.ends_with("missing.flac"))
        .unwrap();
    assert!(missing.proof.is_none());
    assert!(missing.sha256.is_none());
    assert!(missing.registered.is_some());
    let changed = census
        .manifest()
        .entries
        .iter()
        .find(|e| e.path.ends_with("changed.flac"))
        .unwrap();
    assert!(changed.proof.is_some());
    assert!(changed.registered.is_some());
    let unknown = census
        .manifest()
        .entries
        .iter()
        .find(|e| e.path.ends_with("unknown.txt"))
        .unwrap();
    assert!(unknown.registered.is_none());
    assert!(unknown.stage_hint.is_none());
    assert!(unknown.proof.is_some());
    assert!(unknown.sha256.is_some());
    census
        .verify(&f.storage, &crate::test_support::never_cancelled)
        .unwrap();
    assert!(changed.path.ends_with("changed.flac"));
}

#[test]
fn deletion_file_census_preserves_unregistered_generation_inputs_logs_and_known_producer_hints() {
    let f = Fixture::new();
    for (kind, names) in [
        (
            "transcription",
            vec!["input.wav", "result.json", "stderr.log"],
        ),
        (
            "diarization",
            vec!["input.wav", "stdout.log", "result.json"],
        ),
        (
            "minutes_generation",
            vec![
                "prompt.txt",
                "system.txt",
                "schema.json",
                "result.json",
                "items.json",
            ],
        ),
    ] {
        let job = f.job(kind);
        let dir = f
            .directory
            .join("analysis")
            .join(&job)
            .join(format!("unit-0-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in &names {
            std::fs::write(dir.join(name), b"unregistered working content").unwrap();
        }
        // An arbitrary added file is never promoted by its nearby known job.
        std::fs::write(dir.join("foreign.txt"), b"not a known producer output").unwrap();
    }
    let census = f.census();
    let files: Vec<_> = census
        .manifest()
        .entries
        .iter()
        .filter(|e| !e.directory)
        .collect();
    assert_eq!(files.len(), 14);
    assert_eq!(files.iter().filter(|e| e.stage_hint.is_some()).count(), 11);
    assert!(files.iter().all(|e| e.registered.is_none()));
    assert!(files
        .iter()
        .filter(|e| e.path.ends_with("foreign.txt"))
        .all(|e| e.stage_hint.is_none()));
}

#[test]
fn deletion_file_census_hints_unconfirmed_recording_and_import_only_for_fixed_db_ids() {
    for source in ["recording", "audio_import"] {
        let f = Fixture::new();
        let id = uuid::Uuid::new_v4().to_string();
        let db = f.storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type=?2 WHERE id=?1",
            params![f.meeting, source],
        )
        .unwrap();
        let (group, file) = if source == "recording" {
            db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES(?1,?2,'interrupted','system','{}',?1,1,1)",params![id,f.meeting]).unwrap();
            ("audio", "microphone.caf")
        } else {
            db.execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,created_at,updated_at) VALUES(?1,?2,'failed','original.m4a',1,1)",params![id,f.meeting]).unwrap();
            ("imports", "original.m4a")
        };
        drop(db);
        for candidate in [&id, &uuid::Uuid::new_v4().to_string()] {
            let dir = f.directory.join(group).join(candidate);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), b"unfinished source audio").unwrap();
        }
        let census = f.census();
        assert_eq!(
            census
                .manifest()
                .entries
                .iter()
                .filter(|e| e.stage_hint.is_some())
                .count(),
            1
        );
        assert!(census
            .manifest()
            .entries
            .iter()
            .filter(|e| !e.directory)
            .all(|e| e.registered.is_none()));
    }
}

#[test]
fn deletion_file_census_reverification_rejects_title_file_inode_contents_additions_and_disappearance(
) {
    for change in [
        "title",
        "content",
        "inode",
        "addition",
        "disappearance",
        "permissions",
    ] {
        let f = Fixture::new();
        let path = f.asset("audio/file.flac", b"PAYLOAD");
        let census = f.census();
        match change {
            "title" => {
                let m = f.storage.get_meeting(&f.meeting).unwrap().unwrap();
                f.storage
                    .rename_meeting(&m.id, "変更済み", m.lock_version)
                    .unwrap();
            }
            "content" => std::fs::write(&path, b"MODIFIED").unwrap(),
            "inode" => {
                std::fs::rename(&path, path.with_extension("old")).unwrap();
                std::fs::write(&path, b"PAYLOAD").unwrap();
            }
            "addition" => std::fs::write(f.directory.join("extra.txt"), b"ADDED").unwrap(),
            "disappearance" => std::fs::remove_file(path).unwrap(),
            "permissions" => {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            census
                .verify(&f.storage, &crate::test_support::never_cancelled)
                .is_err(),
            "{change}"
        );
        assert!(f.storage.get_meeting(&f.other).unwrap().is_some());
    }
}

#[test]
fn deletion_file_census_refuses_leaf_parent_symlinks_hardlinks_and_registered_directories() {
    use std::os::unix::fs::symlink;
    for change in ["leaf", "parent", "hardlink", "directory"] {
        let f = Fixture::new();
        let path = f.asset("audio/file.flac", b"PAYLOAD");
        let external = f._base.path().join("external");
        std::fs::create_dir(&external).unwrap();
        let keep = external.join("file.flac");
        std::fs::write(&keep, b"EXTERNAL").unwrap();
        match change {
            "leaf" => {
                std::fs::remove_file(&path).unwrap();
                symlink(&keep, &path).unwrap();
            }
            "parent" => {
                std::fs::rename(f.directory.join("audio"), f.directory.join("old-audio")).unwrap();
                symlink(&external, f.directory.join("audio")).unwrap();
            }
            "hardlink" => std::fs::hard_link(&path, external.join("shared.flac")).unwrap(),
            "directory" => {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            Census::capture(
                &f.storage,
                &f.meeting,
                &crate::test_support::never_cancelled
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(std::fs::read(keep).unwrap(), b"EXTERNAL");
    }
}

#[test]
fn deletion_file_census_refuses_fifo_without_opening_a_blocking_stream() {
    use std::os::unix::ffi::OsStrExt;
    let f = Fixture::new();
    let path = f.directory.join("pipe");
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let start = std::time::Instant::now();
    assert!(Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled
    )
    .unwrap_err()
    .contains("特殊ファイル"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn deletion_file_census_root_and_cached_database_replacements_are_not_adopted_by_later_maintenance()
{
    for change in ["root", "database"] {
        let f = Fixture::new();
        f.asset("audio/file.flac", b"PAYLOAD");
        let root = f.storage.root().to_owned();
        if change == "root" {
            std::fs::rename(&root, f._base.path().join("old-root")).unwrap();
            std::fs::create_dir(&root).unwrap();
        } else {
            std::fs::rename(root.join("meetings.sqlite3"), root.join("old.sqlite3")).unwrap();
            std::fs::write(
                root.join("meetings.sqlite3"),
                b"different inode, not the cached DB",
            )
            .unwrap();
        }
        assert!(Census::capture(
            &f.storage,
            &f.meeting,
            &crate::test_support::never_cancelled
        )
        .is_err());
        assert!(f
            .storage
            .prepare_backup_snapshot(&crate::test_support::never_cancelled)
            .is_err());
        assert!(f.storage.create_meeting("must stay write blocked").is_err());
        assert!(f
            .storage
            .maintenance
            .load(std::sync::atomic::Ordering::Acquire));
    }
}

#[test]
fn deletion_file_census_cancel_and_mid_read_mutation_remove_no_files_or_db_rows() {
    use std::io::{Seek, SeekFrom, Write};
    let f = Fixture::new();
    let path = f.asset("file.flac", &vec![b'A'; 256 * 1024]);
    let before = f.census();
    assert!(before.verify(&f.storage, &|| true).is_err());
    let scope_checks = Cell::new(0);
    Scope::capture(
        &f.storage.connection.lock().unwrap(),
        &f.meeting,
        "",
        &|| {
            scope_checks.set(scope_checks.get() + 1);
            false
        },
    )
    .unwrap();
    let calls = Cell::new(0);
    let changed = Cell::new(false);
    assert!(Census::capture(&f.storage, &f.meeting, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == scope_checks.get() + 5 {
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.seek(SeekFrom::Start(128 * 1024)).unwrap();
            file.write_all(b"CHANGED").unwrap();
            file.sync_all().unwrap();
            changed.set(true);
        }
        false
    })
    .is_err());
    assert!(changed.get());
    assert!(path.exists());
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
}

#[test]
fn deletion_file_census_rejects_malformed_asset_paths_and_bounded_deep_trees() {
    let f = Fixture::new();
    f.asset("file.flac", b"PAYLOAD");
    {
        let db = f.storage.connection.lock().unwrap();
        db.execute(
            "UPDATE file_assets SET relative_path='meetings/../outside' WHERE meeting_id=?1",
            params![f.meeting],
        )
        .unwrap();
    }
    assert!(Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let f = Fixture::new();
    let mut path = f.directory.clone();
    for _ in 0..34 {
        path = path.join("d");
        std::fs::create_dir(&path).unwrap();
    }
    assert!(Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn deletion_file_census_handles_a_draft_without_directory_and_detects_active_work() {
    let f = Fixture::new();
    std::fs::remove_dir(&f.directory).unwrap();
    assert!(f.census().manifest().entries.is_empty());
    let id = f.job("transcription");
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='running' WHERE id=?1",
            params![id],
        )
        .unwrap();
    assert!(Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled
    )
    .is_err());
}

#[test]
fn storage_anchor_rejects_database_and_control_aliases_before_opening_sqlite() {
    use std::os::unix::fs::symlink;
    for name in [
        "meetings.sqlite3",
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        "meetings.sqlite3-journal",
    ] {
        let base = tempfile::tempdir().unwrap();
        let external = base.path().join("outside");
        std::fs::write(&external, b"EXTERNAL CONTENT MUST NOT CHANGE").unwrap();
        let root = base.path().join("root");
        std::fs::create_dir(&root).unwrap();
        symlink(&external, root.join(name)).unwrap();
        assert!(Storage::open(root.clone()).is_err(), "{name}");
        assert_eq!(
            std::fs::read(&external).unwrap(),
            b"EXTERNAL CONTENT MUST NOT CHANGE"
        );
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
    }
}

#[test]
fn storage_anchor_rejects_late_wal_substitution_and_keeps_maintenance_write_blocked() {
    let f = Fixture::new();
    f.asset("file.flac", b"PAYLOAD");
    let wal = f.storage.root().join("meetings.sqlite3-wal");
    assert!(wal.exists());
    std::fs::rename(&wal, wal.with_extension("old-wal")).unwrap();
    std::fs::write(&wal, b"not the cached WAL inode").unwrap();
    assert!(Census::capture(
        &f.storage,
        &f.meeting,
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(f
        .storage
        .prepare_backup_snapshot(&crate::test_support::never_cancelled)
        .is_err());
    assert!(f
        .storage
        .create_meeting("must not write cached old WAL")
        .is_err());
    assert_eq!(std::fs::read(wal).unwrap(), b"not the cached WAL inode");
}
