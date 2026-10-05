use super::*;
use crate::storage::backup::recovery::{Outcome, Source};
use std::os::unix::fs::symlink;

fn abandoned(storage: &Storage) -> (String, PathBuf) {
    let stage = storage.backup_stage().unwrap();
    let path = stage.path().to_owned();
    private_file(&path.join("owned"))
        .unwrap()
        .write_all(b"owned scratch body")
        .unwrap();
    std::fs::write(path.join("unknown"), b"unregistered body").unwrap();
    let id = path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix(".mon-stage-")
        .unwrap()
        .to_owned();
    drop(stage);
    (id, path)
}
fn keep(storage: &Storage, id: &str) {
    let mut p = storage
        .prepare_recovery(Source::Scratch, id, &|| false)
        .unwrap();
    assert_eq!(
        p.resolve(storage, &p.preview().confirmation_id, true, &|| false)
            .unwrap()
            .outcome,
        Outcome::Retained
    );
}
fn history(h: &ControlHandoff<'_, '_>) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(&h.bytes).unwrap()["history"].clone()
}

#[test]
fn control_handoff_preserves_exact_keep_receipts_history_and_native_rows_without_adoption() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    s.create_meeting("old meeting").unwrap();
    let (id, path) = abandoned(&s);
    keep(&s, &id);
    let recovered = s.backup_stage().unwrap();
    let recovered_id = recovered
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix(".mon-stage-")
        .unwrap()
        .to_owned();
    drop(recovered);
    let before = std::fs::read(path.join("unknown")).unwrap();
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    let native_rows = census(
        &read_only(&saved.image.join("meetings.sqlite3")).unwrap(),
        &|| false,
    )
    .unwrap();
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    h.verify(&|| false).unwrap();
    assert_eq!(h.hash(), digest(&h.bytes));
    let history = history(&h);
    let areas = history["areas"].as_array().unwrap();
    assert_eq!(areas.len(), 2);
    let kept = areas.iter().find(|a| a["id"] == id).unwrap();
    assert_eq!(kept["state"], "attention");
    assert_eq!(kept["record_valid"], true);
    assert_eq!(kept["entries"][0]["removed"], false);
    assert_eq!(
        areas.iter().find(|a| a["id"] == recovered_id).unwrap()["state"],
        "recovered"
    );
    assert!(!areas
        .iter()
        .any(|a| a["path"] == saved.stage.path().file_name().unwrap().to_str().unwrap()));
    let d = &history["decisions"][0];
    let db = Connection::open(root.path().join(".mon-scratch.sqlite3")).unwrap();
    let original: String = db
        .query_row(
            "SELECT receipt_json FROM recovery_decisions WHERE operation_id=?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(d["receipt_json"], original);
    assert_eq!(history["source_root"], s.root.to_str().unwrap());
    assert_eq!(std::fs::read(path.join("unknown")).unwrap(), before);
    assert_eq!(
        census(
            &read_only(&saved.image.join("meetings.sqlite3")).unwrap(),
            &|| false
        )
        .unwrap(),
        native_rows
    );
    assert!(!String::from_utf8(h.bytes.clone())
        .unwrap()
        .contains("unregistered body"));
}

#[test]
fn control_handoff_refuses_other_live_construction_then_includes_its_recovered_history() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let other = s.backup_stage().unwrap();
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(saved.prepare_control_handoff(&|| false).is_err());
    assert!(!saved.stage.path().join("relocation-control.json").exists());
    drop(other);
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    assert_eq!(history(&h)["areas"].as_array().unwrap().len(), 1);
    h.verify(&|| false).unwrap();
}

#[test]
fn control_handoff_cancel_is_non_destructive_and_existing_seal_is_never_replaced() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (id, path) = abandoned(&s);
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    assert!(saved.prepare_control_handoff(&|| true).is_err());
    assert!(!saved.stage.path().join("relocation-control.json").exists());
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    assert!(h.verify(&|| true).is_err());
    assert!(saved.prepare_control_handoff(&|| false).is_err());
    assert_eq!(std::fs::read(&h.path).unwrap(), h.bytes);
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"unregistered body"
    );
    assert!(history(&h)["areas"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == id));
    h.verify(&|| false).unwrap();
}

#[test]
fn control_handoff_preserves_kept_invalid_proof_as_invalid_history_not_cleanup_authority() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (id, path) = abandoned(&s);
    let db = Connection::open(root.path().join(".mon-scratch.sqlite3")).unwrap();
    let sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='entries_fixed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER entries_fixed").unwrap();
    db.execute(
        "UPDATE entries SET proof='{}',hash=?1 WHERE area=?2",
        params![digest(b"{}"), id],
    )
    .unwrap();
    db.execute_batch(&sql).unwrap();
    drop(db);
    keep(&s, &id);
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    let area = history(&h)["areas"][0].clone();
    assert_eq!(area["record_valid"], false);
    assert_eq!(area["entries"][0]["proof"], "{}");
    assert_eq!(history(&h)["decisions"][0]["operation_id"], id);
    h.verify(&|| false).unwrap();
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"unregistered body"
    );
}

#[test]
fn control_handoff_rechecks_source_state_and_new_history_before_any_future_transfer() {
    for new_area in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (id, path) = abandoned(&s);
        let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
        let h = saved.prepare_control_handoff(&|| false).unwrap();
        if new_area {
            drop(s.backup_stage().unwrap());
        } else {
            Connection::open(root.path().join(".mon-scratch.sqlite3"))
                .unwrap()
                .execute(
                    "UPDATE areas SET error='changed attention' WHERE id=?1",
                    params![id],
                )
                .unwrap();
        }
        assert!(h.verify(&|| false).is_err());
        assert_eq!(std::fs::read(&h.path).unwrap(), h.bytes);
        assert_eq!(
            std::fs::read(path.join("unknown")).unwrap(),
            b"unregistered body"
        );
    }
}

#[test]
fn control_handoff_refuses_changed_seal_and_links_without_following_or_overwriting_foreign_data() {
    for mode in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
        let h = saved.prepare_control_handoff(&|| false).unwrap();
        let foreign = root.path().join("foreign");
        std::fs::write(&foreign, b"foreign bytes").unwrap();
        match mode {
            0 => std::fs::write(&h.path, b"changed").unwrap(),
            1 => {
                std::fs::remove_file(&h.path).unwrap();
                symlink(&foreign, &h.path).unwrap();
            }
            _ => {
                std::fs::remove_file(&h.path).unwrap();
                std::fs::hard_link(&foreign, &h.path).unwrap();
            }
        }
        assert!(h.verify(&|| false).is_err());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign bytes");
        assert!(saved.prepare_control_handoff(&|| false).is_err());
    }
}

#[test]
fn control_handoff_refuses_orphan_keep_receipts_and_tampered_receipt_without_source_mutation() {
    for orphan in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (id, path) = abandoned(&s);
        keep(&s, &id);
        let db = Connection::open(root.path().join(".mon-scratch.sqlite3")).unwrap();
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name='recovery_decision_fixed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.execute_batch("DROP TRIGGER recovery_decision_fixed")
            .unwrap();
        if orphan {
            db.execute(
                "UPDATE recovery_decisions SET operation_id=?1",
                params![Uuid::new_v4().to_string()],
            )
            .unwrap();
        } else {
            db.execute("UPDATE recovery_decisions SET receipt_json='{}'", [])
                .unwrap();
        }
        db.execute_batch(&sql).unwrap();
        let before: String = db
            .query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
                r.get(0)
            })
            .unwrap();
        let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
        assert!(saved.prepare_control_handoff(&|| false).is_err());
        assert!(!saved.stage.path().join("relocation-control.json").exists());
        assert_eq!(
            db.query_row("SELECT receipt_json FROM recovery_decisions", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(path.join("unknown")).unwrap(),
            b"unregistered body"
        );
    }
}

#[test]
fn control_handoff_lists_left_behind_models_unknown_names_and_links_without_payload_access() {
    let root = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("outside"), b"foreign body").unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    std::fs::create_dir(s.root.join("models")).unwrap();
    std::fs::write(s.root.join("models/model.gguf"), b"large model body").unwrap();
    std::fs::write(s.root.join("unknown-root-file"), b"unknown root body").unwrap();
    symlink(foreign.path().join("outside"), s.root.join("outside-link")).unwrap();
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    let data = history(&h);
    let left = data["remaining_root_objects"].as_array().unwrap();
    assert!(left
        .iter()
        .any(|o| o["path"] == "models" && o["kind"] == "directory"));
    assert!(left
        .iter()
        .any(|o| o["path"] == "outside-link" && o["kind"] == "link"));
    assert!(left
        .iter()
        .any(|o| o["path"] == "unknown-root-file" && o["kind"] == "file"));
    assert!(!left.iter().any(|o| o["path"] == "meetings.sqlite3"));
    assert!(!String::from_utf8(h.bytes.clone())
        .unwrap()
        .contains("foreign body"));
    assert!(!saved.image.join("models").exists());
    h.verify(&|| false).unwrap();
    std::fs::write(s.root.join("unknown-root-file"), b"changed unknown body").unwrap();
    assert!(h.verify(&|| false).is_err());
    assert_eq!(
        std::fs::read(foreign.path().join("outside")).unwrap(),
        b"foreign body"
    );
    assert_eq!(
        std::fs::read(s.root.join("models/model.gguf")).unwrap(),
        b"large model body"
    );
}

#[test]
fn control_handoff_fresh_restart_keeps_original_consent_and_does_not_change_payload_manifest() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (id, path) = abandoned(&s);
    keep(&s, &id);
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    let original = history(&h)["decisions"][0].clone();
    let native_hash = saved.manifest_hash().to_owned();
    drop(h);
    drop(saved);
    drop(s);
    let reopened = Storage::open(root.path().into()).unwrap();
    let next = reopened.prepare_relocation_snapshot(&|| false).unwrap();
    assert_eq!(next.manifest_hash(), native_hash);
    let h = next.prepare_control_handoff(&|| false).unwrap();
    assert_eq!(history(&h)["decisions"][0], original);
    assert!(history(&h)["areas"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == id && a["state"] == "attention"));
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"unregistered body"
    );
    h.verify(&|| false).unwrap();
}

#[test]
fn control_handoff_refuses_changed_root_metadata_or_oversized_history_without_repair() {
    for oversized in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (id, path) = abandoned(&s);
        let db = Connection::open(root.path().join(".mon-scratch.sqlite3")).unwrap();
        if oversized {
            db.execute(
                "UPDATE areas SET error=?1 WHERE id=?2",
                params!["x".repeat(8193), id],
            )
            .unwrap();
        } else {
            let sql: String = db
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name='metadata_fixed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute_batch("DROP TRIGGER metadata_fixed").unwrap();
            db.execute(
                "UPDATE metadata SET proof='{}',hash=?1",
                params![digest(b"{}")],
            )
            .unwrap();
            db.execute_batch(&sql).unwrap();
        }
        let before: String = db
            .query_row("SELECT proof FROM metadata", [], |r| r.get(0))
            .unwrap();
        let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
        assert!(saved.prepare_control_handoff(&|| false).is_err());
        assert!(!saved.stage.path().join("relocation-control.json").exists());
        assert_eq!(
            db.query_row("SELECT proof FROM metadata", [], |r| r.get::<_, String>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(path.join("unknown")).unwrap(),
            b"unregistered body"
        );
    }
}

#[test]
fn control_handoff_crash_child() {
    let Ok(root) = std::env::var("MON_CONTROL_HANDOFF_CRASH_ROOT") else {
        return;
    };
    let marker = std::env::var("MON_CONTROL_HANDOFF_CRASH_MARKER").unwrap();
    let s = Storage::open(PathBuf::from(root)).unwrap();
    let saved = s.prepare_relocation_snapshot(&|| false).unwrap();
    let h = saved.prepare_control_handoff(&|| false).unwrap();
    h.verify(&|| false).unwrap();
    let mut ready = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .unwrap();
    ready.write_all(b"sealed").unwrap();
    ready.sync_all().unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn control_handoff_sigkill_after_seal_preserves_source_history_consent_and_unknown_payload() {
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("meetings-root");
    let s = Storage::open(root.clone()).unwrap();
    let (id, path) = abandoned(&s);
    keep(&s, &id);
    let receipt: String = Connection::open(root.join(".mon-scratch.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT receipt_json FROM recovery_decisions WHERE operation_id=?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    let native = s.prepare_relocation_snapshot(&|| false).unwrap();
    let native_hash = native.manifest_hash().to_owned();
    drop(native);
    drop(s);
    let marker = base.path().join("sealed-marker");
    let mut child = Reap(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::backup::relocation::control::tests::control_handoff_crash_child",
                "--nocapture",
            ])
            .env("MON_CONTROL_HANDOFF_CRASH_ROOT", &root)
            .env("MON_CONTROL_HANDOFF_CRASH_MARKER", &marker)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    while !marker.exists() {
        if let Some(exit) = child.0.try_wait().unwrap() {
            panic!("control handoff child ended before seal: {exit}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "control handoff child did not reach seal"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let reopened = Storage::open(root.clone()).unwrap();
    assert_eq!(
        std::fs::read(path.join("unknown")).unwrap(),
        b"unregistered body"
    );
    assert!(reopened
        .recovery_listing()
        .unwrap()
        .items
        .iter()
        .any(|i| i.operation_id == id && i.retained));
    let next = reopened.prepare_relocation_snapshot(&|| false).unwrap();
    assert_eq!(next.manifest_hash(), native_hash);
    let h = next.prepare_control_handoff(&|| false).unwrap();
    assert_eq!(history(&h)["decisions"][0]["receipt_json"], receipt);
    h.verify(&|| false).unwrap();
}
