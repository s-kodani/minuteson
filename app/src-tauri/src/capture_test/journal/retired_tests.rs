use super::*;

// Simulate a remount without replacing any object. Historical fixture proofs
// change together; production never rewrites these immutable records.
fn old_device(root: &Path) {
    let proof_path = root.join("root-proof.json");
    let mut saved = certificate(&File::open(&proof_path).unwrap()).unwrap();
    let device = saved.directory.device + 1;
    for identity in [
        &mut saved.directory,
        &mut saved.database,
        &mut saved.lock,
        &mut saved.certificate,
    ] {
        identity.device = device;
    }
    let db = Connection::open(root.join("ownership.sqlite3")).unwrap();
    let triggers: Vec<String> = db.prepare("SELECT sql FROM sqlite_master WHERE name IN ('metadata_fixed','runs_fixed') ORDER BY name")
        .unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    db.execute_batch("DROP TRIGGER metadata_fixed; DROP TRIGGER runs_fixed;")
        .unwrap();
    let (proof, hash) = encode(&saved).unwrap();
    db.execute(
        "UPDATE metadata SET proof=?1,hash=?2 WHERE id=1",
        params![proof, hash],
    )
    .unwrap();
    let rows: Vec<(String, String, String)> = db
        .prepare("SELECT id,proof,hash FROM runs")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (id, proof, hash) in rows {
        let mut record: Pending = decode(&proof, &hash).unwrap();
        record.directory.device = device;
        record.lease.device = device;
        let (proof, hash) = encode(&record).unwrap();
        db.execute(
            "UPDATE runs SET proof=?1,hash=?2 WHERE id=?3",
            params![proof, hash, id],
        )
        .unwrap();
    }
    for trigger in triggers {
        db.execute_batch(&trigger).unwrap();
    }
    drop(db);
    std::fs::write(
        proof_path,
        serde_json::to_vec(&encode(&saved).unwrap()).unwrap(),
    )
    .unwrap();
}

fn fixture(removed: bool) -> (tempfile::TempDir, PathBuf, String) {
    let parent = tempfile::tempdir().unwrap();
    let registry = Registry::open(parent.path()).unwrap();
    let run = super::super::tests::request();
    let id = run.run_id.clone();
    let area = registry.allocate(run).unwrap();
    area.release_parent_lease();
    if removed {
        let quiet = area.quiet().unwrap();
        std::fs::remove_dir(area.path()).unwrap();
        area.finish(&quiet).unwrap();
    }
    let root = registry.root.clone();
    drop(area);
    drop(registry);
    old_device(&root);
    (parent, root, id)
}

#[test]
fn retired_device_change_preserves_old_bytes_and_opens_a_new_owned_generation() {
    let (parent, root, id) = fixture(true);
    let original_db = std::fs::read(root.join("ownership.sqlite3")).unwrap();
    let original_proof = std::fs::read(root.join("root-proof.json")).unwrap();
    let registry = Registry::open(parent.path()).unwrap();
    assert!(registry.pending().unwrap().is_empty());
    let archived = std::fs::read_dir(parent.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mon-capture-tests-retained-")
        })
        .unwrap();
    assert_eq!(
        std::fs::read(archived.join("ownership.sqlite3")).unwrap(),
        original_db
    );
    assert_eq!(
        std::fs::read(archived.join("root-proof.json")).unwrap(),
        original_proof
    );
    assert!(archived.join(lease_name(&id)).is_file());
    let new = registry.allocate(super::super::tests::request()).unwrap();
    assert!(new.path().is_dir());
    new.release_parent_lease();
    registry
        .collect(registry.pending().unwrap().pop().unwrap())
        .unwrap();
    drop(new);
    drop(registry);
    assert!(Registry::open(parent.path())
        .unwrap()
        .pending()
        .unwrap()
        .is_empty());
}

#[test]
fn retired_device_change_never_rotates_pending_audio_or_unknown_files() {
    for pending in [true, false] {
        let (parent, root, id) = fixture(!pending);
        let keep = if pending {
            root.join(payload_name(&id)).join("microphone.caf")
        } else {
            root.join("unknown")
        };
        std::fs::write(&keep, b"retain").unwrap();
        assert!(Registry::open(parent.path()).is_err());
        assert_eq!(std::fs::read(keep).unwrap(), b"retain");
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
    }
}

#[test]
fn retired_device_change_requires_quiet_leases_and_unchanged_object_identities() {
    let (parent, root, id) = fixture(true);
    let lease = File::open(root.join(lease_name(&id))).unwrap();
    lock(&lease).unwrap();
    assert!(Registry::open(parent.path()).is_err());
    drop(lease);
    let file = root.join("root-proof.json");
    let mut saved = certificate(&File::open(&file).unwrap()).unwrap();
    saved.directory.inode += 1;
    std::fs::write(&file, serde_json::to_vec(&encode(&saved).unwrap()).unwrap()).unwrap();
    assert!(Registry::open(parent.path()).is_err());
    assert!(root.is_dir());
}

#[test]
fn retired_device_change_preserves_sqlite_recovery_state_without_opening_it() {
    for wal in [true, false] {
        let (parent, root, _) = fixture(true);
        let database = root.join("ownership.sqlite3");
        if wal {
            let mut bytes = std::fs::read(&database).unwrap();
            bytes[18] = 2;
            bytes[19] = 2;
            std::fs::write(&database, bytes).unwrap();
        } else {
            std::fs::write(root.join("ownership.sqlite3-journal"), b"retain").unwrap();
        }
        let before = std::fs::read(&database).unwrap();
        assert!(Registry::open(parent.path()).is_err());
        assert_eq!(std::fs::read(&database).unwrap(), before);
        assert!(!root.join("ownership.sqlite3-wal").exists());
        assert!(!root.join("ownership.sqlite3-shm").exists());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
    }
}
