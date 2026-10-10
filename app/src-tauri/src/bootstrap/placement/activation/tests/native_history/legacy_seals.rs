//! Synthetic v1 shape over real ownership/cutover; not an old-binary export.
use super::*;
use crate::storage::deletion::relocation::{
    receiver::{
        synthetic_all_deletion_evidence as evidence, synthetic_consent_record as consent,
        synthetic_keep_meeting as keep,
    },
    synthetic_version_one_handoff,
};

fn old_control(f: &NativeFixture) -> serde_json::Value {
    let db = Connection::open_with_flags(
        f.target.join(".mon-scratch.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let json: String = db
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            [&f.operation],
            |r| r.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["deletion_history"]["version"], 1);
    assert!(value["deletion_history"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r.get("immutableHash").is_none()));
    value
}
fn erase(storage: &Storage, meeting: &str) {
    let mut prepared = operations::Prepared::fresh(storage, meeting, &|| false).unwrap();
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

#[test]
fn legacy_seal_full_delete_refuses_before_intent_and_files_but_unrelated_meeting_stays_writable() {
    for kind in ["kept_audio", "completed_audio", "kept_meeting"] {
        let mut f = NativeFixture::new(kind == "kept_audio", false);
        if kind == "kept_meeting" {
            keep(f.storage.as_ref().unwrap(), &f.meeting);
        }
        synthetic_version_one_handoff(|| f.select());
        let old = old_control(&f);
        let (bootstrap, storage) = f.open();
        let before = evidence(&storage);
        let receipt = consent(&storage, &f.deletion);
        let audio = storage.root().join(&f.audio);
        let before_audio = std::fs::read(&audio).ok();
        let text = audio.parent().unwrap().join("retained.txt");
        let before_text = std::fs::read(&text).unwrap();
        let failure = match operations::Prepared::fresh(&storage, &f.meeting, &|| false) {
            Err(error) => error,
            Ok(_) => panic!("legacy full-row seal allowed destructive preparation: {kind}"),
        };
        assert!(failure.contains("形式版1"));
        assert_eq!(evidence(&storage), before, "no new deletion intent");
        assert_eq!(std::fs::read(&audio).ok(), before_audio);
        assert_eq!(std::fs::read(&text).unwrap(), before_text);
        assert_eq!(old_control(&f), old);
        assert!(!storage.data_write_blocked());
        let unrelated = storage
            .create_meeting("SYNTHETIC UNRELATED CURRENT MEETING")
            .unwrap();
        erase(&storage, &unrelated.id);
        assert_eq!(consent(&storage, &f.deletion), receipt);
        drop(storage);
        drop(bootstrap);
        let (bootstrap, storage) = f.open();
        assert!(!storage.data_write_blocked());
        assert!(storage.get_meeting(&f.meeting).unwrap().is_some());
        assert!(storage.get_meeting(&unrelated.id).unwrap().is_none());
        assert_eq!(std::fs::read(text).unwrap(), before_text);
        drop(storage);
        drop(bootstrap);
    }
}

#[test]
fn legacy_seal_new_owned_move_revalidates_without_rewriting_old_receipts_then_fresh_delete_reopens()
{
    let mut f = NativeFixture::new(true, false);
    let kept = keep(f.storage.as_ref().unwrap(), &f.meeting);
    let source = f.storage.as_ref().unwrap().root().join(&f.audio);
    synthetic_version_one_handoff(|| f.select());
    let old = old_control(&f);
    let (bootstrap, storage) = f.open();
    let before = evidence(&storage);
    let audio_consent = consent(&storage, &f.deletion);
    let meeting_consent = consent(&storage, &kept);
    assert!(operations::Prepared::fresh(&storage, &f.meeting, &|| false).is_err());
    let target = f
        ._base
        .path()
        .canonicalize()
        .unwrap()
        .join("legacy-revalidated");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&target)
        .unwrap();
    let control = repeated::select_current(&bootstrap, &storage, &target);
    assert_eq!(control["deletion_history"]["version"], 3);
    assert!(control["deletion_history"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["immutableHash"].as_str().is_some_and(|v| v.len() == 64)));
    assert_eq!(
        evidence(&storage),
        before,
        "new move must not rewrite old rows"
    );
    drop(storage);
    drop(bootstrap);
    let (bootstrap, storage) = f.open();
    assert_eq!(storage.root(), target);
    assert_eq!(evidence(&storage), before);
    erase(&storage, &f.meeting);
    assert_eq!(consent(&storage, &f.deletion), audio_consent);
    assert_eq!(consent(&storage, &kept), meeting_consent);
    drop(storage);
    drop(bootstrap);
    let (bootstrap, storage) = f.open();
    assert!(!storage.data_write_blocked());
    assert!(storage.get_meeting(&f.meeting).unwrap().is_none());
    assert!(storage
        .create_meeting("SYNTHETIC AFTER LEGACY REVALIDATION")
        .is_ok());
    assert_eq!(std::fs::read(source).unwrap(), b"SYNTHETIC AUDIO");
    assert_eq!(
        std::fs::read(f.target.join(&f.audio)).unwrap(),
        b"SYNTHETIC AUDIO"
    );
    assert_eq!(old_control(&f), old);
    drop(storage);
    drop(bootstrap);
}
