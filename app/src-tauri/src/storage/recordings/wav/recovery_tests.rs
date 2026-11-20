use super::tests::fixture;
use super::*;

fn json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(windows)]
    crate::windows_files::set_private(path).unwrap();
}
fn value(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
pub(super) fn chain(base: &Path, owner: &str, auxiliary_invalid: bool) {
    let path = base.join("recording.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.resize(80 + 3 * 32000, 0);
    bytes[..80].copy_from_slice(&crate::audio::wave::streaming_header(48000).unwrap());
    std::fs::write(path, &bytes).unwrap();
    let mut previous = serde_json::Value::Null;
    for index in 0..3 {
        let pcm = &bytes[80 + index * 32000..80 + (index + 1) * 32000];
        let entry = serde_json::json!({"schemaVersion":1,"ownerId":owner,"sequence":index+1,
            "frameStart":index*16000,"frames":16000,"captureOriginNanoseconds":if auxiliary_invalid {serde_json::json!("unknown")}else{serde_json::json!(1_000_000_000)},
            "pcmSHA256":format!("{:x}",Sha256::digest(pcm)),"previousEntrySHA256":previous,
            "missingRanges":if auxiliary_invalid {serde_json::json!([[90000,1]])}else{serde_json::json!([])},
            "coverageFrames":if auxiliary_invalid {serde_json::json!([16001,0])}else{serde_json::json!([16000,0])}});
        let encoded = serde_json::to_vec(&entry).unwrap();
        json(&base.join(format!("wav-{:08}.json", index + 1)), &entry);
        previous = format!("{:x}", Sha256::digest(encoded)).into();
    }
    let mut tip = value(&base.join("single-wav.json"));
    tip["checkpoints"] = 3.into();
    tip["frames"] = 48000.into();
    tip["lastEntrySHA256"] = previous;
    json(&base.join("single-wav.json"), &tip);
}

#[test]
fn descriptive_manifest_failures_save_all_verified_pcm() {
    for condition in [
        "missing",
        "json",
        "closure",
        "utc",
        "input",
        "format",
        "errors",
        "duplicate",
    ] {
        let (_root, storage, session, base) = fixture(true);
        let path = base.join("manifest.json");
        let mut manifest = value(&path);
        match condition {
            "missing" => std::fs::remove_file(&path).unwrap(),
            "json" => std::fs::write(&path, b"{broken").unwrap(),
            "duplicate" => {
                std::fs::write(&path, b"{\"schemaVersion\":5,\"schemaVersion\":4}").unwrap()
            }
            _ => {
                match condition {
                    "closure" => manifest["captureEnd"]["systemClosedUs"] = 3_000_000.into(),
                    "utc" => {
                        manifest["captureClock"]["originUtcMs"] = 8_640_000_000_000_000_i64.into()
                    }
                    "input" => manifest["targetPIDs"] = serde_json::json!([999]),
                    "format" => manifest["audioFormat"] = "caf".into(),
                    "errors" => {
                        manifest["state"] = "failed".into();
                        manifest["errors"] = serde_json::json!(["power release failed"]);
                    }
                    _ => unreachable!(),
                }
                json(&path, &manifest);
            }
        }
        let before = std::fs::read(base.join("recording.wav")).unwrap();
        let saved = storage
            .finalize_available_recording(&session.id, &|| false)
            .unwrap_or_else(|error| panic!("{condition}: {error}"));
        assert_eq!(saved.duration_us, 2_000_000);
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), before);
        assert!(storage
            .recording_save_warning(&session.id)
            .unwrap()
            .is_none());
        storage
            .verify_canonical_audio(&session.meeting_id, &|| false)
            .unwrap();
        if ["missing", "json", "duplicate", "closure", "utc"].contains(&condition) {
            assert_eq!(saved.state, "interrupted");
            assert!(saved.ended_at_utc_ms.is_none());
        }
    }
}

#[test]
fn receipt_diagnostics_and_redundant_tip_totals_do_not_discard_pcm() {
    let (_root, storage, session, base) = fixture(true);
    chain(&base, &session.id, true);
    let mut tip = value(&base.join("single-wav.json"));
    tip["frames"] = "invalid summary".into();
    json(&base.join("single-wav.json"), &tip);
    let saved = storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    assert_eq!(saved.duration_us, 3_000_000);
    assert_eq!(saved.state, "interrupted");
    let manifest = value(&base.join("canonical-wav.json"));
    assert_eq!(manifest["wavRecovery"]["diagnosticsKnown"], false);
    assert_eq!(manifest["wavRecovery"]["partial"], false);
    assert!(manifest["captureClock"].is_null());
    assert!(storage
        .recording_save_warning(&session.id)
        .unwrap()
        .is_none());
}

#[test]
fn broken_later_evidence_saves_only_the_leading_pcm_and_keeps_every_source_byte() {
    for condition in ["missing", "json", "chain", "pcm", "range"] {
        let (_root, storage, session, base) = fixture(true);
        chain(&base, &session.id, false);
        let second = base.join("wav-00000002.json");
        match condition {
            "missing" => std::fs::remove_file(&second).unwrap(),
            "json" => std::fs::write(&second, b"{broken").unwrap(),
            "pcm" => {
                let mut file = File::options()
                    .write(true)
                    .open(base.join("recording.wav"))
                    .unwrap();
                file.seek(SeekFrom::Start(80 + 32000)).unwrap();
                file.write_all(&[7]).unwrap();
            }
            _ => {
                let mut receipt = value(&second);
                receipt[if condition == "chain" {
                    "previousEntrySHA256"
                } else {
                    "frameStart"
                }] = serde_json::json!(9);
                json(&second, &receipt);
            }
        }
        let original = std::fs::read(base.join("recording.wav")).unwrap();
        let saved = storage
            .finalize_available_recording(&session.id, &|| false)
            .unwrap_or_else(|error| panic!("{condition}: {error}"));
        assert_eq!(saved.duration_us, 1_000_000);
        assert_eq!(saved.state, "interrupted");
        assert!(storage
            .recording_save_warning(&session.id)
            .unwrap()
            .is_some());
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
        let plan = storage
            .verify_canonical_audio(&session.meeting_id, &|| false)
            .unwrap();
        assert_eq!(plan.tracks[0].chunks[0].frames, 16000);
        let path = storage
            .meeting_mix_path(&session.meeting_id, &plan.tracks[0])
            .unwrap();
        assert_ne!(path, base.join("recording.wav"));
        assert_eq!(
            &std::fs::read(&path).unwrap()[80..],
            &original[80..80 + 32000]
        );
        storage
            .finalize_available_recording(&session.id, &|| false)
            .unwrap();
        assert_eq!(
            storage.canonical_audio_plan(&session.meeting_id).unwrap(),
            plan
        );
    }
}

#[test]
fn closed_header_damage_is_repaired_without_changing_pcm() {
    let (_root, storage, session, base) = fixture(true);
    let path = base.join("recording.wav");
    let before = std::fs::read(&path).unwrap();
    let mut file = File::options().write(true).open(&path).unwrap();
    file.write_all(b"BAD!").unwrap();
    drop(file);
    storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn renaming_during_verification_preserves_the_new_name_and_saves_audio() {
    let (_root, storage, session, base) = fixture(true);
    let changed = std::cell::Cell::new(false);
    storage
        .finalize_available_recording(&session.id, &|| {
            if base.join("canonical-wav.json").exists() && !changed.replace(true) {
                let meeting = storage.get_meeting(&session.meeting_id).unwrap().unwrap();
                storage
                    .rename_meeting(&session.meeting_id, "保存中に変更", meeting.lock_version)
                    .unwrap();
            }
            false
        })
        .unwrap();
    assert!(changed.get());
    assert_eq!(
        storage
            .get_meeting(&session.meeting_id)
            .unwrap()
            .unwrap()
            .title,
        "保存中に変更"
    );
}

#[test]
fn unregistered_documents_do_not_block_recovery_and_retries_freeze_the_same_audio() {
    let (_root, storage, session, base) = fixture(true);
    json(
        &base.join("canonical-wav.json"),
        &serde_json::json!({"old":"unregistered"}),
    );
    let once = std::cell::Cell::new(false);
    assert!(storage
        .finalize_available_recording(&session.id, &|| {
            let has_recovery = std::fs::read_dir(&base).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("recovered-wav-")
            });
            has_recovery && !once.replace(true)
        })
        .is_err());
    std::fs::remove_file(base.join("manifest.json")).unwrap();
    storage
        .finalize_available_recording(&session.id, &|| false)
        .unwrap();
    assert_eq!(
        value(&base.join("canonical-wav.json")),
        serde_json::json!({"old":"unregistered"})
    );
    storage
        .verify_canonical_audio(&session.meeting_id, &|| false)
        .unwrap();
}

#[test]
fn same_invocation_completion_reuses_evidence_but_detects_file_changes() {
    let (_root, storage, session, base) = fixture(true);
    let _observations = crate::operation_observation::Scope::enter("recording_save", |_| {});
    let (saved, proof) = storage
        .finalize_single_wav_recording(&session, &|| false)
        .unwrap();
    let checked = crate::operation_observation::measurements();
    assert_eq!(checked, (64000, 1));
    assert!(proof.complete(&storage, &saved).unwrap());
    assert_eq!(crate::operation_observation::measurements(), checked);
    let mut file = File::options()
        .write(true)
        .open(base.join("recording.wav"))
        .unwrap();
    file.seek(SeekFrom::Start(80)).unwrap();
    file.write_all(&[99]).unwrap();
    assert!(proof.complete(&storage, &saved).is_err());
}
