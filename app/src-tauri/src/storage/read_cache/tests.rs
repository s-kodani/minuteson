use super::*;
use std::io::Write;

fn config() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn root(config: &tempfile::TempDir) -> std::path::PathBuf {
    config.path().canonicalize().unwrap()
}

#[test]
fn read_cache_empty_startup_creates_nothing_and_live_copy_is_not_reclaimed() {
    let config = config();
    let parent = root(&config);
    recover(&parent).unwrap();
    assert!(!parent.join(NAME).exists());
    let snapshot = Snapshot::create(&parent).unwrap();
    let path = snapshot.path().to_owned();
    snapshot
        .create_file("meetings.sqlite3")
        .unwrap()
        .write_all(b"owned partial bytes")
        .unwrap();
    recover(&parent).unwrap();
    assert_eq!(
        std::fs::read(path.join("meetings.sqlite3")).unwrap(),
        b"owned partial bytes"
    );
    assert!(snapshot.create_file("../outside").is_err());
    assert!(snapshot.create_file("meetings.sqlite3-journal").is_err());
    assert!(snapshot.create_file("meetings.sqlite3").is_err());
    drop(snapshot);
    assert!(!path.exists());
    recover(&parent).unwrap();
}

#[test]
fn read_cache_initialization_gap_unknown_root_and_linked_registry_are_never_adopted() {
    for kind in [
        "empty",
        "unknown",
        "linked_root",
        "linked_registry",
        "public",
    ] {
        let config = config();
        let parent = root(&config);
        let path = parent.join(NAME);
        let outside = tempfile::tempdir().unwrap();
        let foreign = outside.path().join("foreign");
        std::fs::write(&foreign, b"DO NOT ERASE").unwrap();
        if kind == "linked_registry" {
            drop(Snapshot::create(&parent).unwrap());
            let ledger = path.join(".mon-scratch.sqlite3");
            std::fs::rename(&ledger, path.join("retained-ledger")).unwrap();
            std::os::unix::fs::symlink(&foreign, ledger).unwrap();
        } else if kind == "linked_root" {
            std::os::unix::fs::symlink(outside.path(), &path).unwrap();
        } else {
            std::fs::DirBuilder::new()
                .mode(if kind == "public" { 0o755 } else { 0o700 })
                .create(&path)
                .unwrap();
            if kind == "unknown" {
                std::fs::write(path.join("unknown"), b"KEEP UNKNOWN").unwrap();
            }
        }
        assert!(recover(&parent).is_err(), "{kind}");
        assert!(Snapshot::create(&parent).is_err(), "{kind}");
        assert_eq!(std::fs::read(&foreign).unwrap(), b"DO NOT ERASE");
        if kind == "unknown" {
            assert_eq!(
                std::fs::read(path.join("unknown")).unwrap(),
                b"KEEP UNKNOWN"
            );
        }
        assert!(std::fs::symlink_metadata(path).is_ok());
    }
}

#[test]
fn read_cache_changed_or_unknown_payload_is_preserved_and_reported_not_silently_pruned() {
    for kind in ["unknown", "replaced", "hardlink"] {
        let config = config();
        let parent = root(&config);
        let snapshot = Snapshot::create(&parent).unwrap();
        let path = snapshot.path().to_owned();
        let file = path.join("meetings.sqlite3");
        snapshot
            .create_file("meetings.sqlite3")
            .unwrap()
            .write_all(b"OWNED")
            .unwrap();
        match kind {
            "unknown" => std::fs::write(path.join("unknown"), b"KEEP UNKNOWN").unwrap(),
            "replaced" => {
                std::fs::rename(&file, path.join("retained-original")).unwrap();
                std::fs::write(&file, b"KEEP REPLACEMENT").unwrap();
            }
            "hardlink" => std::fs::hard_link(&file, parent.join("retained-hardlink")).unwrap(),
            _ => unreachable!(),
        }
        drop(snapshot);
        assert!(recover(&parent).is_err(), "{kind}");
        assert!(Snapshot::create(&parent).is_err(), "{kind}");
        match kind {
            "unknown" => assert_eq!(
                std::fs::read(path.join("unknown")).unwrap(),
                b"KEEP UNKNOWN"
            ),
            "replaced" => assert_eq!(std::fs::read(file).unwrap(), b"KEEP REPLACEMENT"),
            "hardlink" => assert_eq!(std::fs::read(file).unwrap(), b"OWNED"),
            _ => unreachable!(),
        }
    }
}
