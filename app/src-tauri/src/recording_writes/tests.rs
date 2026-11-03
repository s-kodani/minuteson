use super::*;
use std::fs::{self, FileTimes, OpenOptions};
use std::io::Write;
use std::time::Duration;
const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
fn caf(frames: usize, live: bool) -> Vec<u8> {
    let mut value = b"caff\0\x01\0\0desc".to_vec();
    value.extend(32_i64.to_be_bytes());
    value.extend(48000_f64.to_be_bytes());
    value.extend(b"lpcm");
    for word in [3_u32, 8, 1, 2, 32] {
        value.extend(word.to_be_bytes());
    }
    value.extend(b"data");
    value.extend(
        if live {
            -1_i64
        } else {
            (frames * 8 + 4) as i64
        }
        .to_be_bytes(),
    );
    value.extend([0; 4]);
    for _ in 0..frames {
        value.extend(0.25_f32.to_le_bytes());
        value.extend((-0.5_f32).to_le_bytes());
    }
    value
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, WriteReader) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let reader = WriteReader::pin(&path).unwrap();
    (root, path, reader)
}
#[test]
fn live_and_finalized_caf_count_only_complete_pcm_not_free_chunk_or_partial_tail() {
    for live in [false, true] {
        let (_root, path, mut reader) = fixture();
        let bytes = caf(7, live);
        fs::write(path.join("microphone.caf"), &bytes).unwrap();
        let snapshot = reader.read(&path, OWNER).unwrap();
        assert_eq!(snapshot.microphone.frames, Some(7));
        assert_eq!(snapshot.microphone.state, "written");
        assert_eq!(snapshot.system.state, "unavailable");
        assert_eq!(fs::read(path.join("microphone.caf")).unwrap(), bytes);
    }
    let (_root, path, mut reader) = fixture();
    let mut bytes = caf(7, true);
    let free = [
        b"free".as_slice(),
        1024_i64.to_be_bytes().as_slice(),
        &[0; 1024],
    ]
    .concat();
    bytes.splice(52..52, free);
    bytes.extend([0; 7]);
    fs::write(path.join("microphone.caf"), bytes).unwrap();
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.frames,
        Some(7)
    );
}
#[test]
fn empty_waits_missing_and_invalid_are_not_saved_silence() {
    let (_root, path, mut reader) = fixture();
    fs::write(path.join("microphone.caf"), []).unwrap();
    let r = reader.read(&path, OWNER).unwrap();
    assert_eq!(r.microphone.state, "waiting");
    assert_eq!(r.microphone.frames, None);
    assert_eq!(r.system.state, "unavailable");
    fs::write(path.join("microphone.caf"), caf(0, true)).unwrap();
    assert_eq!(
        reader.read(&path, OWNER).unwrap().microphone.state,
        "waiting"
    );
}
#[test]
fn touching_or_repolling_without_more_pcm_cannot_freshen_old_write_but_growth_can() {
    let (_root, path, mut reader) = fixture();
    let name = path.join("microphone.caf");
    fs::write(&name, caf(2, true)).unwrap();
    let file = OpenOptions::new().write(true).open(&name).unwrap();
    file.set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(10)))
        .unwrap();
    drop(file);
    let first = reader.read(&path, OWNER).unwrap();
    assert!(first.microphone.age_ms.unwrap() >= 10000);
    // Leave room for Windows' automatic write timestamp to advance past the
    // explicit touch, even when both operations happen within one clock tick.
    // Touching still must not freshen the unchanged PCM from ten seconds ago.
    OpenOptions::new()
        .write(true)
        .open(&name)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(1)))
        .unwrap();
    let touched = reader.read(&path, OWNER).unwrap();
    assert!(touched.microphone.age_ms.unwrap() >= first.microphone.age_ms.unwrap());
    OpenOptions::new()
        .append(true)
        .open(&name)
        .unwrap()
        .write_all(&[0; 8])
        .unwrap();
    let grown = reader.read(&path, OWNER).unwrap();
    assert_eq!(grown.microphone.frames, Some(3));
    assert!(grown.microphone.age_ms.unwrap() < 10000);
    assert!(grown.revision > touched.revision);
}
#[test]
fn truncation_replacement_and_format_change_invalidate_until_a_new_reader() {
    for mode in ["truncate", "replace", "format", "edit"] {
        let (_root, path, mut reader) = fixture();
        let name = path.join("microphone.caf");
        fs::write(&name, caf(4, true)).unwrap();
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.frames,
            Some(4)
        );
        let mut changed = caf(5, true);
        match mode {
            "truncate" => changed = caf(2, true),
            "replace" => fs::rename(&name, path.join("retained.caf")).unwrap(),
            "edit" => changed[64..68].copy_from_slice(&1_u32.to_be_bytes()),
            _ => changed[20..28].copy_from_slice(&44100_f64.to_be_bytes()),
        }
        fs::write(&name, changed).unwrap();
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
        fs::write(&name, caf(6, true)).unwrap();
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
    }
}
#[test]
fn malformed_headers_formats_and_oversized_prefix_are_rejected_read_only() {
    let base = caf(2, true);
    let mut cases = vec![b"not CAF".to_vec(), base[..30].to_vec()];
    for (offset, data) in [
        (28, b"nope".to_vec()),
        (32, 8_u32.to_be_bytes().to_vec()),
        (36, 0_u32.to_be_bytes().to_vec()),
        (40, 2_u32.to_be_bytes().to_vec()),
        (44, 9_u32.to_be_bytes().to_vec()),
        (48, 7_u32.to_be_bytes().to_vec()),
        (56, (-2_i64).to_be_bytes().to_vec()),
        (56, 999_i64.to_be_bytes().to_vec()),
    ] {
        let mut bad = base.clone();
        bad[offset..offset + data.len()].copy_from_slice(&data);
        cases.push(bad);
    }
    for bytes in cases {
        let (_root, path, mut reader) = fixture();
        let name = path.join("microphone.caf");
        fs::write(&name, &bytes).unwrap();
        assert_eq!(
            reader.read(&path, OWNER).unwrap().microphone.state,
            "unavailable"
        );
        assert_eq!(fs::read(&name).unwrap(), bytes);
    }
}
#[cfg(unix)]
#[test]
fn linked_audio_and_replaced_pinned_directory_never_redirect_reads() {
    use std::os::unix::fs::symlink;
    let (_root, path, mut reader) = fixture();
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("foreign.caf");
    let bytes = caf(3, true);
    fs::write(&foreign, &bytes).unwrap();
    symlink(&foreign, path.join("microphone.caf")).unwrap();
    fs::hard_link(&foreign, path.join("system.caf")).unwrap();
    let snapshot = reader.read(&path, OWNER).unwrap();
    assert_eq!(snapshot.microphone.state, "unavailable");
    assert_eq!(snapshot.system.state, "unavailable");
    fs::rename(&path, path.with_extension("kept")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(reader.read(&path, OWNER).is_err());
    // The renamed fixture is explicit test cleanup, not production collection.
    fs::remove_dir_all(path.with_extension("kept")).unwrap();
    assert_eq!(fs::read(foreign).unwrap(), bytes);
}
#[test]
fn owner_revision_and_serialized_nulls_are_exact() {
    let (_root, path, mut reader) = fixture();
    let snapshot = reader.read(&path, OWNER).unwrap();
    let value = serde_json::to_value(snapshot).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["microphone"]["frames"], serde_json::Value::Null);
    assert_eq!(value["microphone"]["ageMs"], serde_json::Value::Null);
    assert!(reader
        .read(&path, "735027d4-a03e-43d3-830a-21e767dfaf89")
        .is_err());
    reader.revision = SAFE;
    assert!(reader.read(&path, OWNER).is_err());
}
#[test]
#[ignore = "requires MON_RECORDING_WRITES_CAF_FIXTURE_DIR with synthetic AVFoundation source.caf and result.caf"]
fn real_avfoundation_caf_source_and_asset_writer_output_have_exact_pcm_counts() {
    let source =
        std::path::PathBuf::from(std::env::var_os("MON_RECORDING_WRITES_CAF_FIXTURE_DIR").unwrap());
    let (_root, path, mut reader) = fixture();
    fs::copy(source.join("source.caf"), path.join("microphone.caf")).unwrap();
    fs::copy(source.join("result.caf"), path.join("system.caf")).unwrap();
    let r = reader.read(&path, OWNER).unwrap();
    assert_eq!(r.microphone.frames, Some(4800));
    assert_eq!(r.system.frames, Some(4800));
}

#[test]
#[ignore = "requires MON_RECORDING_WRITES_LIVE_CAF_FIXTURE_DIR with the observed 4096-frame open-writer snapshot"]
fn live_avfoundation_snapshot_counts_present_pcm_and_not_the_buffered_tail() {
    let source = std::path::PathBuf::from(
        std::env::var_os("MON_RECORDING_WRITES_LIVE_CAF_FIXTURE_DIR").unwrap(),
    );
    let (_root, path, mut reader) = fixture();
    let live = fs::read(source.join("source.caf")).unwrap();
    // Independently observed snapshot: 4096-byte CAF prefix and 4096 stereo
    // Float32 frames. AVAudioFile accepted 4800; 704 had not reached this file.
    assert_eq!(live.len(), 4096 + 4096 * 8);
    assert_eq!(&live[4080..4084], b"data");
    assert_eq!(&live[4084..4092], &(-1_i64).to_be_bytes());
    fs::write(path.join("microphone.caf"), &live).unwrap();
    fs::copy(source.join("result.caf"), path.join("system.caf")).unwrap();
    let r = reader.read(&path, OWNER).unwrap();
    assert_eq!(r.microphone.frames, Some(4096));
    assert_eq!(r.system.frames, Some(4800));
    assert_eq!(fs::read(path.join("microphone.caf")).unwrap(), live);
}
