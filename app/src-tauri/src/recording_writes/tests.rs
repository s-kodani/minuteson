use super::*;
use mon_core::capture::commit::RawPrefix;
use sha2::{Digest, Sha256};
use std::fs;
const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, WriteReader) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let reader = WriteReader::pin(&path).unwrap();
    (root, path, reader)
}
fn append(path: &Path, prefix: RawPrefix, frames: u64) -> RawPrefix {
    let sequence = prefix.segments + 1;
    let mut audio = mon_core::media::caf::voice_header(frames).unwrap().to_vec();
    audio.extend((0..frames).flat_map(|n| ((n % 1000) as i16).to_le_bytes()));
    let start = prefix.last_end_nanoseconds.max(1_000_000_000);
    let duration = frames * 1_000_000_000 / 16000;
    let receipt = serde_json::to_vec(&serde_json::json!({"schemaVersion":3,
        "sampleEncoding":"pcm16","ownerId":OWNER,"lane":"microphone","sequence":sequence,
        "audioName":format!("raw-microphone-{sequence:08}.caf"),"sourceFrameStart":prefix.frames,
        "frames":frames,"sampleRate":16000,"channels":1,"captureOriginNanoseconds":start,
        "clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":frames,"timelineStartUs":0,
            "timelineEndUs":duration / 1000,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}],
        "clockNanoseconds":[{"startNanoseconds":start,"endNanoseconds":start+duration}],
        "audioBytes":audio.len(),"audioSHA256":mon_core::hash::sha256_hex(&audio),
        "previousEntrySHA256":prefix.hash()})).unwrap();
    let next = prefix.prepare(&receipt).unwrap().next;
    write(
        &path.join(format!("raw-microphone-{sequence:08}.caf")),
        &audio,
    );
    write(
        &path.join(format!("raw-microphone-{sequence:08}.json")),
        &receipt,
    );
    write(
        &path.join("raw-microphone.json"),
        &next.tip(OWNER, "microphone", false).unwrap(),
    );
    next
}
#[test]
fn missing_and_empty_prefix_wait_without_claiming_saved_silence() {
    let (_root, path, mut reader) = fixture();
    let first = reader.read(&path, OWNER).unwrap();
    assert_eq!(first.microphone.state, "waiting");
    assert_eq!(first.microphone.frames, None);
    write(
        &path.join("raw-microphone.json"),
        &RawPrefix::default()
            .tip(OWNER, "microphone", false)
            .unwrap(),
    );
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.state,
        "waiting"
    );
    // A leftover continuous file never substitutes for committed evidence.
    write(&path.join("microphone.caf"), b"unconfirmed audio");
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.state,
        "waiting"
    );
}
#[test]
fn committed_segments_grow_without_continuous_caf_and_touching_tip_cannot_refresh_age() {
    let (_root, path, mut reader) = fixture();
    let prefix = append(&path, RawPrefix::default(), 64000);
    let first = reader.read(&path, OWNER).unwrap();
    assert_eq!(first.microphone.frames, Some(64000));
    assert_eq!(first.microphone.state, "written");
    assert_eq!(first.system.state, "waiting");
    assert!(!path.join("microphone.caf").exists());
    reader.microphone.previous.as_mut().unwrap().1 = 10_000;
    write(
        &path.join("raw-microphone.json"),
        &prefix.tip(OWNER, "microphone", true).unwrap(),
    );
    let closed = reader.read(&path, OWNER).unwrap();
    assert!(closed.microphone.age_ms.unwrap() >= 10_000);
    assert_eq!(closed.microphone.frames, Some(64000));
    assert!(closed.revision > first.revision);
}
#[test]
fn only_new_segments_are_hashed_and_latest_evidence_is_rechecked() {
    let (_root, path, mut reader) = fixture();
    let first = append(&path, RawPrefix::default(), 64000);
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.frames,
        Some(64000)
    );
    let second = append(&path, first, 64000);
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.frames,
        Some(128000)
    );
    // Old source damage is deliberately left to full publication validation;
    // live polling is bounded to new segments and the previous last segment.
    write(
        &path.join("raw-microphone-00000001.caf"),
        b"old damaged audio",
    );
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.frames,
        Some(second.frames)
    );
    write(
        &path.join("raw-microphone-00000002.caf"),
        b"changed latest audio",
    );
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.state,
        "unavailable"
    );
}
#[test]
fn missing_or_corrupt_advertised_pcm_never_advances_live_evidence() {
    for corrupt in [false, true] {
        let (_root, path, mut reader) = fixture();
        append(&path, RawPrefix::default(), 64000);
        let name = path.join("raw-microphone-00000001.caf");
        let valid = fs::read(&name).unwrap();
        if corrupt {
            let mut bad = valid.clone();
            bad[68] ^= 1;
            write(&name, &bad);
        } else {
            fs::remove_file(&name).unwrap();
        }
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
        write(&name, &valid);
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.frames,
            Some(64000)
        );
    }
}
#[test]
fn rollback_owner_mismatch_and_same_counter_hash_changes_invalidate_the_reader() {
    for mode in ["rollback", "owner", "hash"] {
        let (_root, path, mut reader) = fixture();
        let prefix = append(&path, RawPrefix::default(), 64000);
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "written"
        );
        let valid = prefix.tip(OWNER, "microphone", false).unwrap();
        let mut bad: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        match mode {
            "rollback" => {
                bad = serde_json::from_slice(
                    &RawPrefix::default()
                        .tip(OWNER, "microphone", false)
                        .unwrap(),
                )
                .unwrap()
            }
            "owner" => bad["ownerId"] = "735027d4-a03e-43d3-830a-21e767dfaf89".into(),
            _ => bad["lastEntrySHA256"] = "a".repeat(64).into(),
        }
        write(
            &path.join("raw-microphone.json"),
            &serde_json::to_vec(&bad).unwrap(),
        );
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
        write(&path.join("raw-microphone.json"), &valid);
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
    }
}
#[test]
fn owner_revision_and_serialized_nulls_remain_compatible() {
    let (_root, path, mut reader) = fixture();
    let value = serde_json::to_value(reader.read(&path, OWNER).unwrap()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert!(value["microphone"]["frames"].is_null());
    assert!(value["microphone"]["ageMs"].is_null());
    assert!(reader
        .read(&path, "735027d4-a03e-43d3-830a-21e767dfaf89")
        .is_err());
    reader.revision = SAFE;
    assert!(reader.read(&path, OWNER).is_err());
}
#[cfg(unix)]
#[test]
fn linked_segment_and_replaced_pinned_directory_are_rejected() {
    let (_root, path, mut reader) = fixture();
    append(&path, RawPrefix::default(), 64000);
    fs::hard_link(
        path.join("raw-microphone-00000001.caf"),
        path.join("shared.caf"),
    )
    .unwrap();
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.state,
        "unavailable"
    );
    fs::rename(&path, path.with_extension("kept")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(reader.read(&path, OWNER).is_err());
    fs::remove_dir_all(path.with_extension("kept")).unwrap();
}
