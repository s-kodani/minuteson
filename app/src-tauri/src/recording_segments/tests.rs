use super::*;
use crate::audio::flac::PcmSource;
use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
const ORIGIN: i64 = 9_000_000_000_000;

fn write(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
fn caf(frames: u64) -> Vec<u8> {
    let mut bytes = b"caff\0\x01\0\0desc".to_vec();
    bytes.extend(32_u64.to_be_bytes());
    bytes.extend(8000_f64.to_be_bytes());
    bytes.extend(b"lpcm");
    for word in [3_u32, 8, 1, 2, 32] {
        bytes.extend(word.to_be_bytes());
    }
    bytes.extend(b"data");
    bytes.extend((4 + frames * 8).to_be_bytes());
    bytes.extend(0_u32.to_be_bytes());
    for _ in 0..frames {
        bytes.extend(0.25_f32.to_le_bytes());
        bytes.extend((-0.5_f32).to_le_bytes());
    }
    bytes
}

struct Fixture {
    root: tempfile::TempDir,
    lane: Lane,
    entries: Vec<Value>,
    closed: bool,
}
impl Fixture {
    fn new(count: usize, last_frames: u64, closed: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let lane = Lane::Microphone;
        let mut entries = Vec::new();
        let mut source = 0;
        for index in 0..count {
            let frames = if index == count - 1 {
                last_frames
            } else {
                32_000
            };
            let sequence = index as u32 + 1;
            let name = lane.name_at(sequence, "caf");
            let audio = caf(frames);
            write(&root.path().join(&name), &audio);
            let origin = ORIGIN + source as i64 * 125_000;
            entries.push(json!({"schemaVersion":2,"ownerId":OWNER,"lane":"microphone",
                "sequence":sequence,"audioName":name,"sourceFrameStart":source,"frames":frames,
                "sampleRate":8000,"channels":2,"captureOriginNanoseconds":origin,
                "clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":frames,"timelineStartUs":0,
                "timelineEndUs":frames*125,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}],
                "clockNanoseconds":[{"startNanoseconds":origin,"endNanoseconds":origin+frames as i64*125_000}],
                "audioBytes":audio.len(),"audioSHA256":hash(&audio),"previousEntrySHA256":null}));
            source += frames;
        }
        let fixture = Self {
            root,
            lane,
            entries,
            closed,
        };
        fixture.publish();
        fixture
    }
    fn publish(&self) {
        let mut previous = None;
        let mut frames = 0;
        for (index, value) in self.entries.iter().enumerate() {
            let mut value = value.clone();
            value["previousEntrySHA256"] = json!(previous);
            let data = serde_json::to_vec(&value).unwrap();
            write(
                &self
                    .root
                    .path()
                    .join(self.lane.name_at(index as u32 + 1, "json")),
                &data,
            );
            frames += value["frames"].as_u64().unwrap();
            previous = Some(hash(&data));
        }
        write(&self.root.path().join(self.lane.tip_name()), &serde_json::to_vec(&json!({
            "schemaVersion":2,"ownerId":OWNER,"lane":self.lane.name(),"sealedSegments":self.entries.len(),
            "sealedFrames":frames,"lastEntrySHA256":previous,"producerClosed":self.closed
        })).unwrap());
    }
    fn directory(&self) -> Result<File, String> {
        // macOS TempDir may be under the /var -> /private/var alias. Test the
        // actual managed path, without relaxing the production no-link walk.
        let path = std::fs::canonicalize(self.root.path()).map_err(|e| e.to_string())?;
        crate::input_levels::open_capture_directory(&path)
    }
    fn read(&self) -> Result<Prefix, String> {
        Prefix::read_optional(self.directory()?, OWNER, self.lane, &|| false)?
            .ok_or("test prefix absent".into())
    }
    fn status(&self) -> Status {
        let mut tip: Value = serde_json::from_slice(
            &std::fs::read(self.root.path().join(self.lane.tip_name())).unwrap(),
        )
        .unwrap();
        tip["failure"] = Value::Null;
        serde_json::from_value(tip).unwrap()
    }
}

#[test]
fn short_intermediate_segments_require_the_native_clock_density_seal_budget() {
    for count in [1_u64, 125, 126, 127, 128, 129] {
        let mut fixture = Fixture::new(2, 2000, true);
        let frames = count * 8;
        let audio = caf(frames);
        write(
            &fixture.root.path().join(fixture.lane.name_at(1, "caf")),
            &audio,
        );
        let first = &mut fixture.entries[0];
        first["frames"] = json!(frames);
        first["audioBytes"] = json!(audio.len());
        first["audioSHA256"] = json!(hash(&audio));
        first["clockSegments"] = json!((0..count)
            .map(|i| json!({
                "sourceFrameStart":i*8,"sourceFrameEnd":(i+1)*8,
                "timelineStartUs":i*2000,"timelineEndUs":i*2000+1000,
                "mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"
            }))
            .collect::<Vec<_>>());
        first["clockNanoseconds"] = json!((0..count)
            .map(|i| json!({
                "startNanoseconds":ORIGIN+i as i64*2_000_000,
                "endNanoseconds":ORIGIN+i as i64*2_000_000+1_000_000
            }))
            .collect::<Vec<_>>());
        let origin = ORIGIN + count as i64 * 2_000_000;
        let second = &mut fixture.entries[1];
        second["sourceFrameStart"] = json!(frames);
        second["captureOriginNanoseconds"] = json!(origin);
        second["clockNanoseconds"] = json!([{
            "startNanoseconds":origin,"endNanoseconds":origin+250_000_000
        }]);
        fixture.publish();
        assert_eq!(
            fixture.read().is_ok(),
            (126..=128).contains(&count),
            "{count}"
        );
    }
}

#[test]
fn advertised_prefix_is_bound_to_actual_pcm_and_status_hash_not_to_unadvertised_tail() {
    let fixture = Fixture::new(2, 2000, true);
    write(
        &fixture.root.path().join(fixture.lane.name_at(3, "caf")),
        b"unfinished tail retained",
    );
    let prefix = fixture.read().unwrap();
    assert_eq!(prefix.frames(), 34_000);
    assert_eq!(prefix.entries().len(), 2);
    assert!(fixture.status().agrees_with_completed(&prefix, 34_000));
    for altered in ["hash", "frames", "owner", "closed", "failure", "schema"] {
        let mut status = fixture.status();
        match altered {
            "hash" => status.last_entry_sha256 = Some("0".repeat(64)),
            "frames" => status.sealed_frames -= 1,
            "owner" => status.owner_id = uuid::Uuid::new_v4().to_string(),
            "closed" => status.producer_closed = false,
            "failure" => status.failure = Some("sync failed".into()),
            "schema" => status.schema_version = 1,
            _ => unreachable!(),
        }
        assert!(!status.agrees_with_completed(&prefix, 34_000));
    }
    let pending = Fixture::new(1, 32000, false);
    let prefix = pending.read().unwrap();
    assert_eq!(prefix.frames(), 32000);
    assert!(!pending.status().agrees_with_completed(&prefix, 32000));
    let empty = Fixture::new(0, 0, false);
    assert_eq!(empty.read().unwrap().frames(), 0);
}

#[test]
fn joined_source_crosses_files_without_padding_and_encodes_real_verified_flac() {
    let fixture = Fixture::new(2, 2000, true);
    let prefix = fixture.read().unwrap();
    let check = || false;
    let mut source = prefix.source(0, 2, &check).unwrap();
    assert_eq!(source.format().frames, 34_000);
    let values = source.read_frames(31_990, 30).unwrap();
    assert_eq!(values.len(), 30 * 8);
    for frame in values.chunks_exact(8) {
        assert_eq!(&frame[..4], &0.25_f32.to_le_bytes());
        assert_eq!(&frame[4..], &(-0.5_f32).to_le_bytes());
    }
    let output = fixture.root.path().join("derived.flac");
    let saved = crate::audio::flac::write_segment(&mut source, 0, 34_000, &output, &check, &|_| {})
        .unwrap();
    assert_eq!(saved.frames, 34_000);
    assert_eq!(saved.bits_per_sample, 24);
    let mut audio =
        VerifiedAudio::open(File::open(&output).unwrap(), saved.size, &saved.sha256).unwrap();
    assert_eq!(audio.read_frames(31_990, 30).unwrap(), values);
    assert!(fixture
        .root
        .path()
        .join(fixture.lane.name_at(1, "caf"))
        .exists());
    assert!(prefix.source(0, 0, &check).is_err());
    assert!(prefix.source(1, 2, &check).is_err());
    assert!(source.read_frames(34_000, 1).is_err());
    assert!(source
        .read_frames(0, crate::audio::PREVIEW_FRAMES + 1)
        .is_err());
}

#[test]
fn malformed_entry_boundaries_format_and_chain_are_rejected_without_adopting_paths() {
    for change in [
        "owner",
        "lane",
        "sequence",
        "path",
        "frames",
        "format",
        "time",
        "counter",
        "mapping",
        "endpoints",
        "missing-endpoints",
        "overlap",
        "extra",
    ] {
        let mut fixture = Fixture::new(2, 2000, true);
        let e = &mut fixture.entries[1];
        match change {
            "owner" => e["ownerId"] = json!(uuid::Uuid::new_v4().to_string()),
            "lane" => e["lane"] = json!("system"),
            "sequence" => e["sequence"] = json!(3),
            "path" => e["audioName"] = json!("../foreign.caf"),
            "frames" => e["frames"] = json!(0),
            "format" => e["sampleRate"] = json!(16000),
            "time" => e["clockSegments"][0]["timelineStartUs"] = json!(1),
            "counter" => e["sourceFrameStart"] = json!(32001),
            "mapping" => e["clockSegments"][0]["mappingJson"] = json!("{}"),
            "endpoints" => {
                e["clockNanoseconds"][0]["endNanoseconds"] = json!(ORIGIN + 4_250_002_000)
            }
            "missing-endpoints" => e["clockNanoseconds"] = json!([]),
            "overlap" => {
                e["captureOriginNanoseconds"] = json!(ORIGIN + 3_000_000_000);
                e["clockNanoseconds"][0]["startNanoseconds"] = json!(ORIGIN + 3_000_000_000);
                e["clockNanoseconds"][0]["endNanoseconds"] = json!(ORIGIN + 3_250_000_000);
            }
            "extra" => e["acquisitionClosed"] = json!(true),
            _ => unreachable!(),
        }
        fixture.publish();
        assert!(fixture.read().is_err(), "{change}");
    }
    for change in [
        "missing-json",
        "missing-audio",
        "hash",
        "broken-chain",
        "frames",
        "producer-kind",
    ] {
        let fixture = Fixture::new(2, 2000, true);
        let json = fixture.root.path().join(fixture.lane.name_at(2, "json"));
        let audio = fixture.root.path().join(fixture.lane.name_at(2, "caf"));
        match change {
            "missing-json" => std::fs::remove_file(&json).unwrap(),
            "missing-audio" => std::fs::remove_file(&audio).unwrap(),
            "hash" => write(&audio, &caf(1999)),
            "broken-chain" => {
                let mut value: Value =
                    serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
                value["previousEntrySHA256"] = json!("0".repeat(64));
                write(&json, &serde_json::to_vec(&value).unwrap());
            }
            "frames" | "producer-kind" => {
                let path = fixture.root.path().join(fixture.lane.tip_name());
                let mut value: Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                if change == "frames" {
                    value["sealedFrames"] = json!(34001);
                } else {
                    value["producerClosed"] = json!("true");
                }
                write(&path, &serde_json::to_vec(&value).unwrap());
            }
            _ => unreachable!(),
        }
        assert!(fixture.read().is_err(), "{change}");
    }
}

#[test]
fn missing_tip_is_distinct_from_invalid_tip_and_all_capacity_guards_are_bounded() {
    let fixture = Fixture::new(0, 0, false);
    let path = fixture.root.path().join(fixture.lane.tip_name());
    for value in [
        json!({}),
        json!({"schemaVersion":2,"ownerId":OWNER,"lane":"microphone",
        "sealedSegments":32769,"sealedFrames":1,"lastEntrySHA256":"0".repeat(64),"producerClosed":false}),
        json!({"schemaVersion":2,"ownerId":OWNER,"lane":"microphone",
        "sealedSegments":0,"sealedFrames":1,"producerClosed":false}),
    ] {
        write(&path, &serde_json::to_vec(&value).unwrap());
        assert!(fixture.read().is_err());
    }
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_JSON + 1)
        .unwrap();
    assert!(fixture.read().is_err());
    std::fs::remove_file(path).unwrap();
    assert!(
        Prefix::read_optional(fixture.directory().unwrap(), OWNER, fixture.lane, &|| false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn changed_leaf_link_permissions_or_fifo_cannot_be_replayed_or_block_reader() {
    for name in [
        "raw-microphone.json",
        "raw-microphone-00000001.json",
        "raw-microphone-00000001.caf",
    ] {
        for kind in ["modify", "replace", "symlink", "hardlink", "mode", "fifo"] {
            let fixture = Fixture::new(2, 2000, true);
            let prefix = fixture.read().unwrap();
            let path = fixture.root.path().join(name);
            let saved = fixture.root.path().join("original");
            let bytes = std::fs::read(&path).unwrap();
            match kind {
                "modify" => write(&path, &bytes),
                "replace" | "symlink" | "fifo" => {
                    std::fs::rename(&path, &saved).unwrap();
                    if kind == "replace" {
                        write(&path, &bytes);
                    } else if kind == "symlink" {
                        std::os::unix::fs::symlink(&saved, &path).unwrap();
                    } else {
                        let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
                        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                    }
                }
                "hardlink" => std::fs::hard_link(&path, &saved).unwrap(),
                "mode" => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap()
                }
                _ => unreachable!(),
            }
            assert!(prefix.verify_unchanged(&|| false).is_err(), "{name} {kind}");
            if matches!(kind, "symlink" | "hardlink" | "mode" | "fifo") {
                assert!(fixture.read().is_err(), "{name} {kind}");
            } else {
                // A new reader can verify identical bytes against the durable
                // hash chain; the previously pinned reader must still refuse
                // the changed inode/timestamps above. No stored inode proof
                // exists to invent a historical-identity rejection here.
                fixture.read().unwrap();
            }
        }
    }
}

#[test]
fn cancellation_and_raw_mutation_cannot_publish_flac_or_turn_prefix_into_closed_capture() {
    let fixture = Fixture::new(2, 2000, false);
    assert!(
        Prefix::read_optional(fixture.directory().unwrap(), OWNER, fixture.lane, &|| true).is_err()
    );
    let prefix = fixture.read().unwrap();
    let check = || false;
    let mut source = prefix.source(0, 2, &check).unwrap();
    let output = fixture.root.path().join("cancelled.flac");
    let error =
        crate::audio::flac::write_segment(&mut source, 0, 34_000, &output, &|| true, &|_| {})
            .err()
            .unwrap();
    assert!(!error.published);
    assert!(!output.exists());
    write(
        &fixture.root.path().join(fixture.lane.name_at(1, "caf")),
        &caf(32000),
    );
    assert!(source.read_frames(0, 1).is_err());
    assert!(
        crate::audio::flac::write_segment(&mut source, 0, 34_000, &output, &check, &|_| {})
            .is_err()
    );
    assert!(!output.exists());
    // The status still describes the same originally advertised prefix; it
    // cannot by itself prove that those physical files are unchanged now.
    assert!(!fixture.status().agrees_with_completed(&prefix, 34000));
    assert!(prefix.verify_unchanged(&check).is_err());
}

#[test]
fn four_hour_raw_prefix_exceeds_legacy_chunk_count_without_unbounded_descriptors() {
    let fixture = Fixture::new(3600, 32_000, true);
    let prefix = fixture.read().unwrap();
    assert_eq!(prefix.entries().len(), 3600);
    assert_eq!(prefix.frames(), 8000 * 14_400);
    let groups = prefix.flac_groups(ORIGIN).unwrap();
    assert_eq!(groups.len(), 240);
    assert_eq!(groups.iter().map(|g| g.frames).sum::<u64>(), 8000 * 14_400);
    assert_eq!(groups[0].clock_segments[0].timeline_start_us, 0);
    assert_eq!(
        groups
            .last()
            .unwrap()
            .clock_segments
            .last()
            .unwrap()
            .timeline_end_us,
        14_400_000_000
    );
    assert!(groups
        .iter()
        .all(|g| g.entries == 15 && g.frames == 480000 && g.clock_segments.len() == 15));
    let check = || false;
    let mut final_segment = prefix.source(3599, 1, &check).unwrap();
    assert_eq!(final_segment.format().frames, 32000);
    assert_eq!(final_segment.read_frames(31999, 1).unwrap(), caf(1)[68..]);
    assert_eq!(
        prefix
            .entries()
            .last()
            .unwrap()
            .clock_nanoseconds
            .last()
            .unwrap()
            .end_nanoseconds,
        ORIGIN + 14_400_000_000_000
    );
    // Prefix owns one directory descriptor and scalar stamps, not 7,200 open
    // segment descriptors. The lazy source owns only its current audio reader.
    prefix.verify_unchanged(&check).unwrap();
}

#[test]
fn exact_nanosecond_boundaries_avoid_rounding_gaps_and_preserve_real_missing_interval() {
    let mut fixture = Fixture::new(2, 2000, true);
    // Two independently floored local microsecond maps must not make a false
    // one-microsecond hole when rebased to the common capture clock.
    let drifted = ORIGIN + 4_000_200_999;
    fixture.entries[0]["clockSegments"][0]["timelineEndUs"] = json!(4_000_200);
    fixture.entries[0]["clockNanoseconds"][0]["endNanoseconds"] = json!(drifted);
    fixture.entries[1]["captureOriginNanoseconds"] = json!(drifted);
    fixture.entries[1]["clockNanoseconds"][0]["startNanoseconds"] = json!(drifted);
    fixture.entries[1]["clockNanoseconds"][0]["endNanoseconds"] = json!(drifted + 250_000_000);
    fixture.publish();
    let prefix = fixture.read().unwrap();
    let groups = prefix.flac_groups(ORIGIN).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].clock_segments[0].timeline_end_us,
        groups[0].clock_segments[1].timeline_start_us
    );
    assert_eq!(
        groups[0].clock_segments.last().unwrap().timeline_end_us,
        4_250_200
    );
    assert!(prefix.flac_groups(ORIGIN + 1).is_err());
    let missing = drifted + 2_000_000_000;
    fixture.entries[1]["captureOriginNanoseconds"] = json!(missing);
    fixture.entries[1]["clockNanoseconds"][0]["startNanoseconds"] = json!(missing);
    fixture.entries[1]["clockNanoseconds"][0]["endNanoseconds"] = json!(missing + 250_000_000);
    fixture.publish();
    let groups = fixture.read().unwrap().flac_groups(ORIGIN).unwrap();
    assert_eq!(
        groups[0].clock_segments[1].timeline_start_us - groups[0].clock_segments[0].timeline_end_us,
        2_000_000
    );
    assert_eq!(groups[0].frames, 34000);
}

#[test]
fn flac_group_split_respects_map_budget_without_erasing_capture_observations() {
    let mut fixture = Fixture::new(6, 32000, true);
    for entry in &mut fixture.entries {
        let origin = entry["captureOriginNanoseconds"].as_i64().unwrap();
        let mut maps = Vec::new();
        let mut clocks = Vec::new();
        for i in 0..240_i64 {
            let from = i * 32000 / 240;
            let to = (i + 1) * 32000 / 240;
            maps.push(json!({"sourceFrameStart":from,"sourceFrameEnd":to,
                "timelineStartUs":from*125,"timelineEndUs":to*125,
                "mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}));
            clocks.push(
                json!({"startNanoseconds":origin+from*125000,"endNanoseconds":origin+to*125000}),
            );
        }
        entry["clockSegments"] = json!(maps);
        entry["clockNanoseconds"] = json!(clocks);
    }
    fixture.publish();
    let prefix = fixture.read().unwrap();
    let groups = prefix.flac_groups(ORIGIN).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].entries, 4);
    assert_eq!(groups[0].clock_segments.len(), 960);
    assert_eq!(groups[1].clock_segments.len(), 480);
    assert_eq!(
        groups.iter().map(|g| g.clock_segments.len()).sum::<usize>(),
        1440
    );
    assert_eq!(
        groups[0].clock_segments.last().unwrap().timeline_end_us,
        groups[1].clock_segments[0].timeline_start_us
    );
}

#[test]
fn actual_nonfinite_or_wrong_pcm_encoding_is_rejected_and_overrange_pcm_is_retained_not_clipped() {
    for kind in ["nan", "integer", "overrange"] {
        let mut fixture = Fixture::new(1, 2000, false);
        let path = fixture.root.path().join(fixture.lane.name_at(1, "caf"));
        let mut bytes = std::fs::read(&path).unwrap();
        match kind {
            "nan" => bytes[68..72].copy_from_slice(&f32::NAN.to_le_bytes()),
            "integer" => bytes[32..36].copy_from_slice(&2_u32.to_be_bytes()),
            "overrange" => bytes[68..72].copy_from_slice(&2_f32.to_le_bytes()),
            _ => unreachable!(),
        }
        write(&path, &bytes);
        fixture.entries[0]["audioSHA256"] = json!(hash(&bytes));
        fixture.publish();
        if kind != "overrange" {
            assert!(fixture.read().is_err(), "{kind}");
            continue;
        }
        let prefix = fixture.read().unwrap();
        let check = || false;
        let mut source = prefix.source(0, 1, &check).unwrap();
        let output = fixture.root.path().join("reject-overrange.flac");
        let error =
            crate::audio::flac::write_segment(&mut source, 0, 2000, &output, &check, &|_| {})
                .err()
                .unwrap();
        assert!(!error.published);
        assert!(!output.exists());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!prefix.tip.producer_closed);
    }
}

#[test]
#[ignore = "Dedicated Swift/native raw-prefix roundtrip; synthetic PCM only, no capture devices"]
fn swift_generated_recording_segments_bind_actual_storage_and_encode_flac_without_rewriting_raw() {
    let root = tempfile::tempdir().unwrap();
    let storage = crate::storage::Storage::open(root.path().to_path_buf()).unwrap();
    let meeting = storage.create_meeting("合成原音声の確定検査").unwrap();
    let config = crate::storage::RecordingConfig {
        mode: "system".into(),
        backend: "screencapturekit".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    let session = storage
        .prepare_recording(&meeting.id, &config, "synthetic-native-raw-start")
        .unwrap();
    storage.mark_recording_started(&session.id).unwrap();
    let audio_root = root
        .path()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    std::fs::create_dir_all(&audio_root).unwrap();
    let audio_root = std::fs::canonicalize(audio_root).unwrap();
    let output=std::process::Command::new("swift")
        .args(["test","--package-path",concat!(env!("CARGO_MANIFEST_DIR"),"/../../probes/macos-capture"),
            "--disable-sandbox","--filter","RawRecordingSegmentTests/testNativePrefixFixtureExportsBoundManifestForRustWithoutCaptureDevices"])
        .env("SWIFTPM_MODULECACHE_OVERRIDE","/tmp/mon-swift-module-cache")
        .env("CLANG_MODULE_CACHE_PATH","/tmp/mon-clang-module-cache")
        .env("MON_RAW_SEGMENT_FIXTURE_DIR",&audio_root)
        .env("MON_RAW_SEGMENT_FIXTURE_OWNER",&session.id)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = audio_root.join("manifest.json");
    let original = std::fs::read(&manifest).unwrap();
    let value: Value = serde_json::from_slice(&original).unwrap();
    for change in [
        "status-hash",
        "missing-status",
        "status-owner",
        "incomplete-status",
    ] {
        let mut altered = value.clone();
        match change {
            "status-hash" => {
                altered["microphone"]["rawSegments"]["lastEntrySHA256"] = json!("0".repeat(64))
            }
            "missing-status" => {
                altered["microphone"]
                    .as_object_mut()
                    .unwrap()
                    .remove("rawSegments");
            }
            "status-owner" => altered["microphone"]["rawSegments"]["ownerId"] = json!(OWNER),
            "incomplete-status" => {
                altered["microphone"]["rawSegments"]["producerClosed"] = json!(false)
            }
            _ => unreachable!(),
        }
        std::fs::write(&manifest, serde_json::to_vec(&altered).unwrap()).unwrap();
        assert!(storage.finalize_recording(&session.id).is_err(), "{change}");
        assert_eq!(
            storage
                .prepare_recording(&meeting.id, &config, "synthetic-native-raw-start")
                .unwrap()
                .state,
            "recording"
        );
    }
    std::fs::write(&manifest, &original).unwrap();
    let prefix = Prefix::read_optional(
        crate::input_levels::open_capture_directory(&audio_root).unwrap(),
        &session.id,
        Lane::Microphone,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(prefix.frames(), 204_000);
    let first = std::fs::read(audio_root.join("raw-microphone-00000001.caf")).unwrap();
    let pointer = std::fs::read(audio_root.join("raw-microphone.json")).unwrap();
    let check = || false;
    let mut source = prefix.source(0, 2, &check).unwrap();
    let path = audio_root.join("native-derived.flac");
    let saved =
        crate::audio::flac::write_segment(&mut source, 0, 204000, &path, &check, &|_| {}).unwrap();
    let mut verified =
        VerifiedAudio::open(File::open(&path).unwrap(), saved.size, &saved.sha256).unwrap();
    assert_eq!(verified.format.sample_rate, 48000);
    let expected = source.read_frames(191999, 2).unwrap();
    assert_eq!(verified.read_frames(191999, 2).unwrap(), expected);
    let saved_session = storage.finalize_recording(&session.id).unwrap();
    assert_eq!(saved_session.state, "saved");
    assert_eq!(std::fs::read(manifest).unwrap(), original);
    assert_eq!(
        std::fs::read(audio_root.join("raw-microphone-00000001.caf")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read(audio_root.join("raw-microphone.json")).unwrap(),
        pointer
    );
    // The real Storage finalizer now publishes/admits its own FLAC, independently
    // of the codec-only native-derived.flac above. Producer evidence stays raw.
    let (plan, known) = storage.recorded_analysis_input(&meeting.id).unwrap();
    assert_eq!(plan.tracks.len(), 2);
    if let Some(known) = known {
        assert_eq!(known.runs.len(), 2);
    }
    let publication: Value =
        serde_json::from_slice(&std::fs::read(audio_root.join("final-flac.json")).unwrap())
            .unwrap();
    for track in &plan.tracks {
        assert_eq!(track.chunks.len(), 1);
        assert_eq!(track.chunks[0].frames, 204000);
        assert!(storage.verify_asset(&track.chunks[0].asset_id).unwrap());
        let published_track = publication["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|published| published["id"] == track.id)
            .unwrap();
        let chunk = published_track["chunks"][0].as_str().unwrap();
        // Storage owns an exclusive DB connection. Use its admitted API and
        // deterministic published UUID path, not a second SQLite connection.
        let path = storage
            .root()
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&track.id)
            .join(format!("{chunk}.flac"));
        let mut registered = VerifiedAudio::open(
            File::open(path).unwrap(),
            track.chunks[0].size as u64,
            &track.chunks[0].hash,
        )
        .unwrap();
        assert!(registered.is_flac());
        assert_eq!(registered.read_frames(191999, 2).unwrap(), expected);
    }
}
