use super::*;
use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

struct Fixture {
    temp: tempfile::TempDir,
    storage: Storage,
    session: RecordingSession,
    base: PathBuf,
}
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
impl Fixture {
    fn new(count: usize, last_frames: u64, gap: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("通常録音の合成FLAC").unwrap();
        let session = storage
            .prepare_recording(
                &meeting.id,
                &RecordingConfig {
                    mode: "system".into(),
                    backend: "screencapturekit".into(),
                    target_pids: vec![],
                    microphone_device_uid: None,
                    target_application_identity: None,
                },
                "synthetic-flac-start",
            )
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let session = {
            let db = storage.connection.lock().unwrap();
            load_session(&db, &session.id).unwrap().unwrap()
        };
        let base = storage
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&base).unwrap();
        let origin_ns = 9_000_000_000_000_i64;
        let mut tracks = Vec::new();
        for (lane, offset) in [("microphone", 0_i64), ("system", 50_000_i64)] {
            let mut frames = 0_u64;
            let mut previous = None::<String>;
            let mut maps = Vec::new();
            for index in 0..count {
                let count_frames = if index + 1 == count {
                    last_frames
                } else {
                    32_000
                };
                let start =
                    frames as i64 * 125 + offset + if gap && index > 0 { 700_000 } else { 0 };
                let end = start + count_frames as i64 * 125;
                let origin = origin_ns + start * 1000;
                let name = format!("raw-{lane}-{:08}.caf", index + 1);
                let audio = caf(count_frames);
                write(&base.join(&name), &audio);
                let entry = json!({"schemaVersion":2,"ownerId":session.id,"lane":lane,"sequence":index+1,
                    "audioName":name,"sourceFrameStart":frames,"frames":count_frames,"sampleRate":8000,"channels":2,
                    "captureOriginNanoseconds":origin,
                    "clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":count_frames,"timelineStartUs":0,
                    "timelineEndUs":count_frames*125,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}],
                    "clockNanoseconds":[{"startNanoseconds":origin,"endNanoseconds":origin+count_frames as i64*125000}],
                    "audioBytes":audio.len(),"audioSHA256":hash(&audio),"previousEntrySHA256":previous});
                let bytes = serde_json::to_vec(&entry).unwrap();
                write(
                    &base.join(format!("raw-{lane}-{:08}.json", index + 1)),
                    &bytes,
                );
                previous = Some(hash(&bytes));
                maps.push(map(
                    frames as i64,
                    (frames + count_frames) as i64,
                    start,
                    end,
                ));
                frames += count_frames;
            }
            let tip = json!({"schemaVersion":2,"ownerId":session.id,"lane":lane,"sealedSegments":count,
                "sealedFrames":frames,"lastEntrySHA256":previous,"producerClosed":true});
            write(
                &base.join(format!("raw-{lane}.json")),
                &serde_json::to_vec(&tip).unwrap(),
            );
            let mut status = tip;
            status["failure"] = Value::Null;
            write(&base.join(format!("{lane}.caf")), &caf(frames));
            tracks.push(
                json!({"frames":frames,"sampleRate":8000,"channels":2,"appendFailures":0,
                "clockSegments":maps,"rawSegments":status}),
            );
        }
        let mic_end = tracks[0]["clockSegments"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["timelineEndUs"]
            .as_i64()
            .unwrap();
        let pc_end = tracks[1]["clockSegments"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["timelineEndUs"]
            .as_i64()
            .unwrap();
        let manifest = json!({"schemaVersion":3,"captureMode":"system","backend":"screencapturekit",
            "targetPIDs":[],"state":"saved","errors":[],"recordingOwnerId":session.id,
            "microphone":tracks[0],"system":tracks[1],
            "captureClock":{"domain":"mach_continuous_time","originUtcMs":1700000000000_i64,"uncertaintyNs":1000},
            "captureEnd":{"method":"acquisition_closed","microphoneClosedUs":mic_end+500000,
                "systemClosedUs":pc_end+600000,"endUs":pc_end+600000}});
        write(
            &base.join("manifest.json"),
            &serde_json::to_vec(&manifest).unwrap(),
        );
        Self {
            temp,
            storage,
            session,
            base,
        }
    }
    fn plan(&self) -> Plan {
        serde_json::from_slice(&std::fs::read(self.base.join(PLAN)).unwrap()).unwrap()
    }
    fn fail_db(&self) {
        self.storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_flac_commit BEFORE UPDATE OF state ON recording_sessions WHEN NEW.state='saved' BEGIN SELECT RAISE(ABORT,'injected final DB failure'); END").unwrap();
    }
    fn allow_db(&self) {
        self.storage
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_flac_commit")
            .unwrap();
    }
    fn assert_no_registration(&self) {
        let db = self.storage.connection.lock().unwrap();
        for table in ["audio_tracks", "audio_chunks", "file_assets"] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0,
                "{table}"
            );
        }
        assert_eq!(
            load_session(&db, &self.session.id).unwrap().unwrap().state,
            "recording"
        );
    }
    fn directory(&self, track: &TrackPlan) -> Directory {
        Directory::open(&self.base.parent().unwrap().join(&track.id)).unwrap()
    }
}

fn map(from: i64, to: i64, start: i64, end: i64) -> ClockSegment {
    ClockSegment {
        source_frame_start: from,
        source_frame_end: to,
        timeline_start_us: start,
        timeline_end_us: end,
        mapping_json: "{\"schema_version\":1,\"method\":\"linear\"}".into(),
    }
}
fn remove_reviewed_audio(storage: &Storage, meeting: &str) -> usize {
    use crate::storage::deletion::audio::{
        intent::{self, Policy},
        Census,
    };
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let census = Census::capture(storage, meeting, &|| false).unwrap();
    let count = census
        .preview(&Uuid::new_v4().to_string())
        .unwrap()
        .audio_files;
    let operation = Uuid::new_v4().to_string();
    intent::prepare(
        storage,
        &operation,
        &census,
        Policy {
            include_unregistered_audio: true,
            retain_unclassified: true,
        },
        &lease,
        &|| false,
    )
    .unwrap();
    intent::removal::start(storage, &operation, &lease, &|| false).unwrap();
    assert!(intent::removal::run(storage, &operation, &lease, &|| false)
        .unwrap()
        .files_removed());
    assert!(
        intent::removal::completion::finish(storage, &operation, &lease, &|| false)
            .unwrap()
            .completed()
    );
    count
}

#[test]
fn native_raw_audio_final_flac_additive_restore_preserves_pcm_clocks_gaps_and_manual_deletion() {
    let fixture = Fixture::new(17, 2000, true);
    fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .unwrap();
    let (source, source_known) = fixture
        .storage
        .recorded_analysis_input(&fixture.session.meeting_id)
        .unwrap();
    assert_eq!(source_known.unwrap().runs.len(), 4);
    let output = tempfile::tempdir().unwrap();
    let archive_path = output.path().join("normal-final-flac.monbackup");
    let receipt = fixture
        .storage
        .create_backup_archive(&archive_path, &|| false)
        .unwrap();
    assert_eq!(receipt.asset_count, 5); // Four final FLACs and the manifest, not raw PCM.
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().to_owned()).unwrap();
    let existing = target.create_meeting("保持する既存会議").unwrap().id;
    let selection = target
        .prepare_additive_restore(&archive_path, &|| false, &|_| {})
        .unwrap();
    let done = selection
        .publish(
            &target,
            &selection.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(done.meeting_ids.len(), 1);
    let meeting = &done.meeting_ids[0];
    assert_ne!(meeting, &fixture.session.meeting_id);
    let (restored, known) = target.recorded_analysis_input(meeting).unwrap();
    assert_eq!(known.unwrap().runs.len(), 4);
    for track in &source.tracks {
        let other = restored
            .tracks
            .iter()
            .find(|other| other.kind == track.kind)
            .unwrap();
        assert_ne!(other.id, track.id);
        assert_eq!(
            (
                other.start_us,
                other.end_us,
                other.sample_rate,
                other.channels
            ),
            (
                track.start_us,
                track.end_us,
                track.sample_rate,
                track.channels
            )
        );
        assert_eq!(other.chunks.len(), 2);
        for (original, copy) in track.chunks.iter().zip(&other.chunks) {
            assert_ne!(copy.asset_id, original.asset_id);
            assert_eq!(
                (
                    copy.frames,
                    copy.start_us,
                    copy.end_us,
                    &copy.clock_segments,
                    &copy.hash
                ),
                (
                    original.frames,
                    original.start_us,
                    original.end_us,
                    &original.clock_segments,
                    &original.hash
                )
            );
            assert!(target.verify_asset(&copy.asset_id).unwrap());
            let (relative, media): (String, String) = target.connection.lock().unwrap().query_row(
                "SELECT relative_path,media_type FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![copy.asset_id, meeting], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(media, "audio/flac");
            let file = File::open(target.root.join(relative)).unwrap();
            let mut audio = VerifiedAudio::open(file, copy.size as u64, &copy.hash).unwrap();
            assert!(audio.is_flac());
            assert_eq!(
                audio.read_frames(0, 1).unwrap(),
                [0.25_f32.to_le_bytes(), (-0.5_f32).to_le_bytes()].concat()
            );
        }
        assert!(target
            .prepare_track_audio(meeting, &other.id, 3_900_000, 4_800_000)
            .is_err());
        target
            .prepare_track_audio(meeting, &other.id, 60_800_000, 61_000_000)
            .unwrap();
    }
    drop(selection);
    assert_eq!(remove_reviewed_audio(&target, meeting), 4);
    assert!(target.recorded_analysis_input(meeting).is_err());
    assert!(target.get_meeting(&existing).unwrap().is_some());
    assert!(fixture
        .storage
        .recorded_analysis_input(&fixture.session.meeting_id)
        .is_ok());
    assert_eq!(
        remove_reviewed_audio(&fixture.storage, &fixture.session.meeting_id),
        40
    );
    for lane in ["microphone", "system"] {
        assert!(!fixture.base.join(format!("{lane}.caf")).exists());
        for sequence in 1..=17 {
            assert!(!fixture
                .base
                .join(format!("raw-{lane}-{sequence:08}.caf"))
                .exists());
            assert!(fixture
                .base
                .join(format!("raw-{lane}-{sequence:08}.json"))
                .is_file());
        }
    }
    assert!(fixture.base.join(PLAN).is_file());
    assert!(fixture.base.join("manifest.json").is_file());
}

#[cfg(target_os = "macos")]
#[test]
fn native_raw_audio_completed_relocation_image_keeps_raw_final_flac_and_all_sidecars() {
    let fixture = Fixture::new(2, 2000, true);
    fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .unwrap();
    let image = fixture
        .storage
        .prepare_relocation_snapshot(&|| false)
        .unwrap();
    image.verify(&|| false).unwrap();
    image.verify_source(&|| false).unwrap();
    for entry in image
        .manifest()
        .entries
        .iter()
        .filter(|entry| !entry.directory)
    {
        assert_eq!(
            std::fs::read(image.source_root().join(&entry.relative_path)).unwrap(),
            std::fs::read(image.image().join(&entry.relative_path)).unwrap()
        );
    }
    assert_eq!(
        image
            .manifest()
            .entries
            .iter()
            .filter(|entry| entry.relative_path.ends_with(".caf"))
            .count(),
        6
    );
    assert_eq!(
        image
            .manifest()
            .entries
            .iter()
            .filter(|entry| entry.relative_path.ends_with(".flac"))
            .count(),
        2
    );
    assert!(image
        .manifest()
        .entries
        .iter()
        .any(|entry| entry.relative_path.ends_with(PLAN)));
    // This is an image-integrity check, not an activation/rebinding test of an
    // interrupted conversion whose checkpoints contain original inode IDs.
}

#[test]
fn independently_compressed_raw_clocks_keep_frame_alignment_and_exact_gaps() {
    let global = vec![
        map(0, 32000, 0, 4000000),
        map(32000, 64000, 5000000, 9000000),
    ];
    let raw = vec![
        map(0, 16000, 0, 2000000),
        map(16000, 32000, 2000000, 4000000),
        map(32000, 64000, 5000000, 9000000),
    ];
    verify_maps(&raw, &global, 8000).unwrap();
    let mut within_error = raw.clone();
    within_error[0].timeline_end_us += 200;
    within_error[1].timeline_start_us += 200;
    verify_maps(&within_error, &global, 8000).unwrap();
    within_error[0].timeline_end_us += 200;
    within_error[1].timeline_start_us += 200;
    assert!(verify_maps(&within_error, &global, 8000).is_err());
    let mut concealed_gap = raw.clone();
    concealed_gap[2].timeline_start_us = 4000000;
    assert!(verify_maps(&concealed_gap, &global, 8000).is_err());
    let mut changed_gap = raw.clone();
    changed_gap[2].timeline_start_us += 1;
    assert!(verify_maps(&changed_gap, &global, 8000).is_err());
}

#[test]
fn workspace_refuses_links_replaced_directories_and_final_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let actual = temp.path().canonicalize().unwrap();
    let root = Directory::open(&actual).unwrap();
    let child = root.create_child("owned").unwrap();
    let mut staging = child.create("source.part").unwrap();
    staging.write_all(b"original").unwrap();
    let identity = Identity::file(&staging).unwrap();
    child.rename("source.part", "final.flac", true).unwrap();
    identity
        .require(&child.required("final.flac", false).unwrap())
        .unwrap();
    let mut other = child.create("other.part").unwrap();
    other.write_all(b"other").unwrap();
    assert!(child.rename("other.part", "final.flac", true).is_err());
    assert_eq!(
        std::fs::read(child.path.join("final.flac")).unwrap(),
        b"original"
    );
    std::os::unix::fs::symlink("final.flac", child.path.join("alias")).unwrap();
    assert!(child.optional("alias", true).is_err());
    std::fs::rename(&child.path, actual.join("displaced")).unwrap();
    std::fs::create_dir(&child.path).unwrap();
    assert!(child.create("unbound.part").is_err());
    assert!(!child.path.join("unbound.part").exists());
}

#[test]
fn normal_stop_registers_only_verified_flac_and_preserves_source_gaps_and_raw_evidence() {
    let fixture = Fixture::new(2, 2000, true);
    let raw_before = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
    let manifest_before = std::fs::read(fixture.base.join("manifest.json")).unwrap();
    let saved = fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .unwrap();
    assert_eq!(saved.state, "saved");
    assert_eq!(saved.duration_us, 5_600_000);
    let (plan, known) = fixture
        .storage
        .recorded_analysis_input(&fixture.session.meeting_id)
        .unwrap();
    assert_eq!(known.unwrap().runs.len(), 4);
    assert_eq!(plan.tracks.len(), 2);
    for track in &plan.tracks {
        assert_eq!(track.chunks.len(), 1);
        assert_eq!(track.clock_layout().unwrap().raw_frames, 34000);
        assert!(track
            .clock_layout()
            .unwrap()
            .range(3_900_000, 4_800_000, 8000)
            .is_err());
        let db = fixture.storage.connection.lock().unwrap();
        let format: String = db
            .query_row(
                "SELECT format FROM audio_tracks WHERE id=?1",
                [&track.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(format, "flac");
        let (path, media): (String, String) = db
            .query_row(
                "SELECT relative_path,media_type FROM file_assets WHERE id=?1",
                [&track.chunks[0].asset_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(media, "audio/flac");
        assert!(path.contains(&format!("/audio/{}/", track.id)));
        let mut verified = VerifiedAudio::open(
            File::open(fixture.storage.root.join(path)).unwrap(),
            track.chunks[0].size as u64,
            &track.chunks[0].hash,
        )
        .unwrap();
        assert_eq!(verified.format.frames, 34000);
        assert_eq!(verified.read_frames(31999, 2).unwrap(), caf(2)[68..]);
    }
    assert_eq!(
        std::fs::read(fixture.base.join("manifest.json")).unwrap(),
        manifest_before
    );
    assert_eq!(
        std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap(),
        raw_before
    );
}

#[test]
fn final_db_failure_rolls_back_all_rows_and_restart_reuses_same_verified_output_inodes() {
    let fixture = Fixture::new(2, 2000, false);
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    fixture.assert_no_registration();
    let plan = fixture.plan();
    let directory = fixture.directory(&plan.tracks[0]);
    let name = format!("{}.flac", plan.tracks[0].chunks[0]);
    let identity = Identity::file(&directory.required(&name, false).unwrap()).unwrap();
    fixture.allow_db();
    let Fixture {
        temp,
        storage,
        session,
        ..
    } = fixture;
    drop(storage);
    let reopened = Storage::open(temp.path().to_owned()).unwrap();
    assert_eq!(
        reopened.recover_completed_recordings().unwrap(),
        vec![session.id.clone()]
    );
    identity
        .require(&directory.required(&name, false).unwrap())
        .unwrap();
    let restored_plan = reopened.recorded_audio_plan(&session.meeting_id).unwrap();
    assert_eq!(restored_plan.tracks[0].id, plan.tracks[0].id);
    assert_eq!(
        reopened.recover_completed_recordings().unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn published_output_checkpoint_and_directory_substitution_never_fall_back_to_caf() {
    for change in ["identical-copy", "hardlink", "receipt", "directory"] {
        let fixture = Fixture::new(1, 2000, false);
        fixture.fail_db();
        assert!(fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .is_err());
        let plan = fixture.plan();
        let directory = fixture.directory(&plan.tracks[0]);
        let name = format!("{}.flac", plan.tracks[0].chunks[0]);
        let path = directory.path.join(&name);
        match change {
            "identical-copy" => {
                let bytes = std::fs::read(&path).unwrap();
                std::fs::rename(&path, directory.path.join("displaced.flac")).unwrap();
                write(&path, &bytes);
            }
            "hardlink" => std::fs::hard_link(&path, directory.path.join("alias.flac")).unwrap(),
            "receipt" => {
                let receipt_path = directory
                    .path
                    .join(format!(".{}.json", plan.tracks[0].chunks[0]));
                let mut value: Value =
                    serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
                value["input_hash"] = json!("0".repeat(64));
                write(&receipt_path, &serde_json::to_vec(&value).unwrap());
            }
            "directory" => {
                std::fs::rename(&directory.path, directory.path.with_extension("displaced"))
                    .unwrap();
                std::fs::create_dir(&directory.path).unwrap();
            }
            _ => unreachable!(),
        }
        fixture.allow_db();
        assert!(
            fixture
                .storage
                .finalize_recording(&fixture.session.id)
                .is_err(),
            "{change}"
        );
        fixture.assert_no_registration();
        assert!(fixture.base.join("raw-microphone-00000001.caf").is_file());
    }
}

#[test]
fn checkpoint_bound_partial_and_published_before_sync_can_resume_without_new_ids() {
    for phase in ["encoding", "encoded-before-rename"] {
        let fixture = Fixture::new(1, 2000, false);
        fixture.fail_db();
        assert!(fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .is_err());
        let plan = fixture.plan();
        let directory = fixture.directory(&plan.tracks[0]);
        let id = &plan.tracks[0].chunks[0];
        let (bytes, _) = directory
            .read_json(&format!(".{id}.json"))
            .unwrap()
            .unwrap();
        let checkpoint: Checkpoint = serde_json::from_slice(&bytes).unwrap();
        directory
            .rename(&format!("{id}.flac"), &checkpoint.staging_name, true)
            .unwrap();
        let partial = directory.required(&checkpoint.staging_name, true).unwrap();
        let identity = Identity::file(&partial).unwrap();
        if phase == "encoding" {
            let (bytes, stamp) = directory
                .read_json(&format!(".{id}.json"))
                .unwrap()
                .unwrap();
            let mut receipt: Checkpoint = serde_json::from_slice(&bytes).unwrap();
            receipt.audio = None;
            directory
                .json(&format!(".{id}.json"), &receipt, Some(&stamp))
                .unwrap();
            partial.set_len(100).unwrap();
        }
        fixture.allow_db();
        assert_eq!(
            fixture
                .storage
                .finalize_recording(&fixture.session.id)
                .unwrap()
                .state,
            "saved",
            "{phase}"
        );
        identity
            .require(&directory.required(&format!("{id}.flac"), false).unwrap())
            .unwrap();
        assert!(!directory.path.join(checkpoint.staging_name).exists());
    }
}

#[test]
fn raw_clock_mismatch_rejects_conversion_before_outputs_or_saved_rows_exist() {
    let fixture = Fixture::new(2, 2000, false);
    let path = fixture.base.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["microphone"]["clockSegments"][0]["timelineEndUs"] = json!(3_990_000);
    manifest["microphone"]["clockSegments"][1]["timelineStartUs"] = json!(3_990_000);
    write(&path, &serde_json::to_vec(&manifest).unwrap());
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    assert!(!fixture.base.join(PLAN).exists());
    fixture.assert_no_registration();
}

#[test]
fn valid_foreign_flac_with_rewritten_checkpoint_hash_is_not_adopted_as_recorded_pcm() {
    let fixture = Fixture::new(1, 2000, false);
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    let plan = fixture.plan();
    let directory = fixture.directory(&plan.tracks[0]);
    let id = &plan.tracks[0].chunks[0];
    let mut foreign = caf(2000);
    foreign[68..]
        .chunks_exact_mut(4)
        .for_each(|value| value.copy_from_slice(&0.75_f32.to_le_bytes()));
    let foreign_path = fixture.temp.path().join("foreign.caf");
    write(&foreign_path, &foreign);
    let mut source = VerifiedAudio::open(
        File::open(foreign_path).unwrap(),
        foreign.len() as u64,
        &hash(&foreign),
    )
    .unwrap();
    let mut destination = directory.required(&format!("{id}.flac"), true).unwrap();
    destination.set_len(0).unwrap();
    let forged =
        encode_segment_file(&mut source, 0, 2000, &mut destination, &|| false, &|_| {}).unwrap();
    let name = format!(".{id}.json");
    let (bytes, stamp) = directory.read_json(&name).unwrap().unwrap();
    let mut receipt: Checkpoint = serde_json::from_slice(&bytes).unwrap();
    receipt.audio = Some(forged.into());
    directory.json(&name, &receipt, Some(&stamp)).unwrap();
    fixture.allow_db();
    let error = fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .err()
        .unwrap();
    assert!(error.contains("実PCM"), "{error}");
    fixture.assert_no_registration();
}

#[test]
fn active_conversion_lock_refuses_second_finalizer_without_rewriting_evidence() {
    let fixture = Fixture::new(1, 2000, false);
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    let root = Directory::open(&fixture.base).unwrap();
    let lock = root.required(LOCK, true).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    fixture.allow_db();
    let error = fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .err()
        .unwrap();
    assert!(error.contains("別の処理"), "{error}");
    fixture.assert_no_registration();
    drop(lock);
    assert_eq!(
        fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .unwrap()
            .state,
        "saved"
    );
}

#[test]
fn same_pcm_caf_with_flac_extension_and_rewritten_checkpoint_is_not_a_final_flac() {
    let fixture = Fixture::new(1, 2000, false);
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    let plan = fixture.plan();
    let directory = fixture.directory(&plan.tracks[0]);
    let id = &plan.tracks[0].chunks[0];
    let bytes = caf(2000);
    let mut final_file = directory.required(&format!("{id}.flac"), true).unwrap();
    final_file.set_len(0).unwrap();
    final_file.write_all(&bytes).unwrap();
    final_file.sync_all().unwrap();
    let name = format!(".{id}.json");
    let (json_bytes, stamp) = directory.read_json(&name).unwrap().unwrap();
    let mut receipt: Checkpoint = serde_json::from_slice(&json_bytes).unwrap();
    receipt.audio = Some(AudioProof {
        size: bytes.len() as u64,
        sha256: hash(&bytes),
        frames: 2000,
        sample_rate: 8000,
        channels: 2,
    });
    directory.json(&name, &receipt, Some(&stamp)).unwrap();
    fixture.allow_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    fixture.assert_no_registration();
}

#[test]
fn minute_sized_output_chunks_rebase_source_frames_without_concatenating_across_a_gap() {
    let fixture = Fixture::new(17, 2000, true);
    let saved = fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .unwrap();
    assert_eq!(saved.state, "saved");
    let (plan, known) = fixture
        .storage
        .recorded_analysis_input(&fixture.session.meeting_id)
        .unwrap();
    assert_eq!(known.unwrap().runs.len(), 4);
    for track in &plan.tracks {
        assert_eq!(track.chunks.len(), 2);
        assert_eq!(track.chunks[0].frames, 15 * 32000);
        assert_eq!(track.chunks[1].frames, 34000);
        assert_eq!(track.chunks[1].clock_segments[0].source_frame_start, 0);
        assert_eq!(track.clock_layout().unwrap().raw_frames, 16 * 32000 + 2000);
        assert!(fixture
            .storage
            .prepare_track_audio(&fixture.session.meeting_id, &track.id, 3_900_000, 4_800_000)
            .is_err());
        fixture
            .storage
            .prepare_track_audio(
                &fixture.session.meeting_id,
                &track.id,
                60_800_000,
                61_000_000,
            )
            .unwrap();
    }
}

#[test]
#[ignore = "Owned synthetic SIGKILL child; only its dedicated parent may launch it"]
fn final_flac_crash_child() {
    assert_eq!(
        std::env::var("MON_FINAL_FLAC_CRASH_CHILD").unwrap(),
        "synthetic-only"
    );
    let root = PathBuf::from(std::env::var_os("MON_FINAL_FLAC_CRASH_ROOT").unwrap());
    let owner = std::env::var("MON_FINAL_FLAC_CRASH_OWNER").unwrap();
    let storage = Storage::open(root).unwrap();
    storage.finalize_recording(&owner).unwrap();
    panic!("owned synthetic child failed to reach its crash point");
}

#[test]
#[ignore = "Dedicated owned-child crash/restart verification; synthetic PCM only, no capture devices"]
fn final_flac_owned_sigkill_checkpoints_recover_without_registering_partial_audio() {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "reserved",
        "created-before-identity",
        "allocated",
        "encoded",
        "published",
        "before-db-commit",
    ] {
        let Fixture {
            temp,
            storage,
            session,
            base,
        } = Fixture::new(1, 2000, false);
        let original = std::fs::read(base.join("raw-microphone-00000001.caf")).unwrap();
        drop(storage);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::recordings::flac::tests::final_flac_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
            .env("MON_FINAL_FLAC_CRASH_PHASE", phase)
            .env("MON_FINAL_FLAC_CRASH_ROOT", temp.path())
            .env("MON_FINAL_FLAC_CRASH_OWNER", &session.id)
            .output()
            .unwrap();
        assert_eq!(
            child.status.signal(),
            Some(libc::SIGKILL),
            "{phase}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        let plan: Plan = serde_json::from_slice(&std::fs::read(base.join(PLAN)).unwrap()).unwrap();
        let directory = Directory::open(&base.parent().unwrap().join(&plan.tracks[0].id)).unwrap();
        let id = &plan.tracks[0].chunks[0];
        let (bytes, _) = directory
            .read_json(&format!(".{id}.json"))
            .unwrap()
            .unwrap();
        let checkpoint: Checkpoint = serde_json::from_slice(&bytes).unwrap();
        let identity = checkpoint.staging_identity;
        {
            let db = reopened.connection.lock().unwrap();
            assert_eq!(
                load_session(&db, &session.id).unwrap().unwrap().state,
                "interrupted"
            );
            for table in ["audio_tracks", "audio_chunks", "file_assets"] {
                assert_eq!(
                    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "{phase}: {table}"
                );
            }
        }
        assert_eq!(
            reopened.recover_completed_recordings().unwrap(),
            vec![session.id.clone()],
            "{phase}"
        );
        if let Some(identity) = identity {
            identity
                .require(&directory.required(&format!("{id}.flac"), false).unwrap())
                .unwrap();
        } else if phase == "created-before-identity" {
            assert_eq!(
                std::fs::read(directory.path.join(checkpoint.staging_name)).unwrap(),
                b""
            );
        }
        assert_eq!(
            std::fs::read(base.join("raw-microphone-00000001.caf")).unwrap(),
            original
        );
        let audio = reopened.recorded_audio_plan(&session.meeting_id).unwrap();
        assert_eq!(audio.tracks.len(), 2);
        assert_eq!(audio.tracks[0].id, plan.tracks[0].id);
        for track in &audio.tracks {
            assert_eq!(track.total_frames().unwrap(), 2000);
        }
    }
}

#[test]
fn database_clock_loading_streams_whole_track_maps_with_per_chunk_and_json_budgets() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE track_clock_segments(id TEXT,track_id TEXT,chunk_id TEXT,source_frame_start INTEGER,source_frame_end INTEGER,timeline_start_us INTEGER,timeline_end_us INTEGER,mapping_json TEXT)").unwrap();
    let tx = db.transaction().unwrap();
    for index in 0..1600 {
        tx.execute(
            "INSERT INTO track_clock_segments VALUES(?1,'track',?2,?3,?4,?5,?6,?7)",
            params![
                index.to_string(),
                format!("chunk-{}", index / 800),
                index % 800,
                index % 800 + 1,
                index * 125,
                (index + 1) * 125,
                "{\"schema_version\":1,\"method\":\"linear\"}"
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let loaded = crate::storage::transcription::read_clock_segments(&db, "track").unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.values().map(Vec::len).sum::<usize>(), 1600);
    db.execute("UPDATE track_clock_segments SET chunk_id='same-chunk'", [])
        .unwrap();
    assert!(
        crate::storage::transcription::read_clock_segments(&db, "track")
            .err()
            .unwrap()
            .contains("区間内")
    );
    db.execute("UPDATE track_clock_segments SET chunk_id=CASE WHEN CAST(id AS INTEGER)<800 THEN 'first' ELSE 'second' END,mapping_json=?1", [format!("{}{{\"schema_version\":1,\"method\":\"linear\"}}", " ".repeat(3000))]).unwrap();
    assert!(
        crate::storage::transcription::read_clock_segments(&db, "track")
            .err()
            .unwrap()
            .contains("合計容量")
    );
}

fn partial_fixture() -> Fixture {
    interrupt_fixture(Fixture::new(2, 2000, true))
}
fn interrupt_fixture(fixture: Fixture) -> Fixture {
    let state = load_session(
        &fixture.storage.connection.lock().unwrap(),
        &fixture.session.id,
    )
    .unwrap()
    .unwrap()
    .state;
    if matches!(state.as_str(), "recording" | "stopping" | "preparing") {
        fixture
            .storage
            .interrupt_recording(&fixture.session.id)
            .unwrap();
    } else {
        assert!(matches!(state.as_str(), "failed" | "interrupted"));
    }
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(fixture.base.join("manifest.json")).unwrap())
            .unwrap();
    manifest["state"] = json!("recording");
    manifest["captureEnd"] = Value::Null;
    write(
        &fixture.base.join("manifest.json"),
        &serde_json::to_vec(&manifest).unwrap(),
    );
    for lane in ["microphone", "system"] {
        let path = fixture.base.join(format!("raw-{lane}.json"));
        let mut tip: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        tip["producerClosed"] = json!(false);
        write(&path, &serde_json::to_vec(&tip).unwrap());
        // A broken unadvertised tail and continuous working CAF are not proof.
        let next = tip["sealedSegments"].as_u64().unwrap() + 1;
        write(
            &fixture.base.join(format!("raw-{lane}-{next:08}.caf")),
            b"unfinished tail",
        );
        write(
            &fixture.base.join(format!("{lane}.caf")),
            b"broken continuous CAF",
        );
    }
    fixture
}

fn partial_registered_rows(fixture: &Fixture) -> i64 {
    fixture
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM audio_tracks", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn partial_recovery_sealed_prefix_registers_real_flac_and_known_runs_without_fake_closure() {
    let fixture = partial_fixture();
    assert!(fixture
        .storage
        .recover_completed_recordings()
        .unwrap()
        .is_empty());
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert_eq!(review.summary.duration_us, 5_000_000);
    assert!(!review.summary.acquisition_end_known);
    assert_eq!(review.summary.tail_loss_us, None);
    assert!(review
        .summary
        .lanes
        .iter()
        .all(|lane| lane.state == "available" && lane.frames == 34000));
    let raw = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
    let manifest = std::fs::read(fixture.base.join("manifest.json")).unwrap();
    let result = fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    assert_eq!(result.state, "interrupted");
    assert_eq!(result.ended_at_utc_ms, None);
    assert_eq!(result.duration_us, 5_000_000);
    let (audio, known) = fixture
        .storage
        .recorded_analysis_input(&result.meeting_id)
        .unwrap();
    assert_eq!(audio.tracks.len(), 2);
    let known = known.unwrap();
    assert_eq!(known.runs.len(), 4);
    known.validate(&audio).unwrap();
    for track in &audio.tracks {
        assert_eq!(
            track.chunks.iter().map(|chunk| chunk.frames).sum::<i64>(),
            34000
        );
        let start = track.start_us;
        let mut preview = fixture
            .storage
            .prepare_track_audio(&result.meeting_id, &track.id, start, start + 100_000)
            .unwrap();
        assert_eq!(
            preview.read_pcm(0, 1).unwrap(),
            [0.25_f32.to_le_bytes(), (-0.5_f32).to_le_bytes()].concat()
        );
        assert!(fixture
            .storage
            .prepare_track_audio(&result.meeting_id, &track.id, 3_900_000, 4_800_000)
            .is_err());
        assert!(fixture
            .storage
            .verify_asset(&track.chunks[0].asset_id)
            .unwrap());
    }
    assert!(known
        .gaps
        .iter()
        .all(|gap| gap.reason_code != "capture_stop_gap"));
    assert!(known
        .gaps
        .iter()
        .any(|gap| gap.reason_code == "partial_prefix_end_gap"));
    assert_eq!(
        std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap(),
        raw
    );
    assert_eq!(
        std::fs::read(fixture.base.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        std::fs::read(fixture.base.join("raw-system-00000003.caf")).unwrap(),
        b"unfinished tail"
    );
    let document: Value =
        serde_json::from_slice(&std::fs::read(fixture.base.join("partial-recovery.json")).unwrap())
            .unwrap();
    assert_eq!(document["state"], "interrupted");
    assert_eq!(document["partialRecovery"]["acquisitionEndKnown"], false);
    assert!(document["partialRecovery"]["tailLossUs"].is_null());
    assert!(document.get("captureEnd").is_none());
    assert!(fixture.storage.finalize_recording(&result.id).is_err());
    assert!(fixture
        .storage
        .recover_partial_recording(&review, true)
        .is_err());
    assert_eq!(partial_registered_rows(&fixture), 2);
    drop(fixture.storage);
    let reopened = Storage::open(fixture.temp.path().to_owned()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    assert!(reopened.recorded_analysis_input(&result.meeting_id).is_ok());
    assert_eq!(
        load_session(&reopened.connection.lock().unwrap(), &result.id)
            .unwrap()
            .unwrap()
            .state,
        "interrupted"
    );
}

#[test]
fn partial_recovery_requires_consent_and_closed_native_owner_without_writing_on_review() {
    let active = Fixture::new(1, 2000, false);
    assert!(active
        .storage
        .review_partial_recording(&active.session.id)
        .is_err());
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert!(fixture
        .storage
        .recover_partial_recording(&review, false)
        .is_err());
    assert_eq!(partial_registered_rows(&fixture), 0);
    for name in [PARTIAL_PLAN, "partial-recovery.json", LOCK] {
        assert!(!fixture.base.join(name).exists());
    }
    fixture
        .storage
        .retired
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .is_err());
    assert!(fixture
        .storage
        .recover_partial_recording(&review, true)
        .is_err());
    assert_eq!(partial_registered_rows(&fixture), 0);
}

#[test]
fn partial_recovery_identical_audio_replacement_tip_close_and_meeting_edit_invalidate_review() {
    for change in ["audio", "tip", "meeting"] {
        let fixture = partial_fixture();
        let review = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        match change {
            "audio" => {
                let path = fixture.base.join("raw-microphone-00000001.caf");
                let replacement = fixture.base.join("replacement.caf");
                write(&replacement, &std::fs::read(&path).unwrap());
                std::fs::rename(replacement, path).unwrap();
            }
            "tip" => {
                let path = fixture.base.join("raw-system.json");
                let mut tip: Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                tip["producerClosed"] = json!(true);
                write(&path, &serde_json::to_vec(&tip).unwrap());
            }
            _ => {
                let meeting = fixture
                    .storage
                    .get_meeting(&fixture.session.meeting_id)
                    .unwrap()
                    .unwrap();
                fixture
                    .storage
                    .rename_meeting(&meeting.id, "確認後の変更", meeting.lock_version)
                    .unwrap();
            }
        }
        assert!(fixture
            .storage
            .recover_partial_recording(&review, true)
            .unwrap_err()
            .contains("変更"));
        assert_eq!(partial_registered_rows(&fixture), 0);
        assert!(!fixture.base.join(PARTIAL_PLAN).exists());
    }
}

#[test]
fn partial_recovery_one_valid_lane_does_not_guess_missing_or_corrupt_other_input() {
    for missing in [true, false] {
        let fixture = partial_fixture();
        if missing {
            std::fs::remove_file(fixture.base.join("raw-microphone.json")).unwrap();
        } else {
            write(
                &fixture.base.join("raw-microphone-00000001.caf"),
                b"corrupt advertised audio",
            );
        }
        let review = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        assert_eq!(
            review.summary.lanes[0].state,
            if missing { "missing" } else { "unreadable" }
        );
        assert_eq!(review.summary.lanes[1].state, "available");
        let result = fixture
            .storage
            .recover_partial_recording(&review, true)
            .unwrap();
        let (audio, known) = fixture
            .storage
            .recorded_analysis_input(&result.meeting_id)
            .unwrap();
        assert_eq!(audio.tracks.len(), 1);
        assert_eq!(audio.tracks[0].kind, "system");
        assert_eq!(known.unwrap().runs.len(), 2);
        assert!(fixture.base.join("raw-microphone-00000001.caf").is_file());
    }
}

#[test]
fn partial_recovery_failed_commit_keeps_all_audio_and_resumes_same_checkpoint_ids() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    fixture.storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_partial_commit BEFORE UPDATE OF audio_manifest_hash ON recording_sessions BEGIN SELECT RAISE(ABORT,'injected partial commit failure'); END").unwrap();
    assert!(fixture
        .storage
        .recover_partial_recording(&review, true)
        .is_err());
    assert_eq!(partial_registered_rows(&fixture), 0);
    assert!(fixture
        .storage
        .recorded_audio_plan(&fixture.session.meeting_id)
        .is_err());
    let plan = std::fs::read(fixture.base.join(PARTIAL_PLAN)).unwrap();
    let document = std::fs::read(fixture.base.join("partial-recovery.json")).unwrap();
    fixture
        .storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_partial_commit")
        .unwrap();
    drop(fixture.storage);
    let reopened = Storage::open(fixture.temp.path().to_owned()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    let review = reopened
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    let result = reopened.recover_partial_recording(&review, true).unwrap();
    assert_eq!(
        std::fs::read(fixture.base.join(PARTIAL_PLAN)).unwrap(),
        plan
    );
    assert_eq!(
        std::fs::read(fixture.base.join("partial-recovery.json")).unwrap(),
        document
    );
    let plan: Plan = serde_json::from_slice(&plan).unwrap();
    let (audio, _) = reopened
        .recorded_analysis_input(&result.meeting_id)
        .unwrap();
    for track in &audio.tracks {
        assert!(plan.tracks.iter().any(|item| item.id == track.id));
    }
}

#[test]
fn partial_recovery_backup_additive_restore_keeps_interruption_unknown_tail_and_known_audio() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let archive = output.path().join("partial.monbackup");
    let receipt = fixture
        .storage
        .create_backup_archive(&archive, &|| false)
        .unwrap();
    assert_eq!(receipt.asset_count, 3);
    let root = tempfile::tempdir().unwrap();
    let target = Storage::open(root.path().to_owned()).unwrap();
    let existing = target.create_meeting("保持する会議").unwrap().id;
    let selection = target
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let done = selection
        .publish(
            &target,
            &selection.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    let meeting = &done.meeting_ids[0];
    let (audio, known) = target.recorded_analysis_input(meeting).unwrap();
    assert_eq!(known.unwrap().runs.len(), 4);
    let (id, state, ended): (String, String, Option<i64>) = target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT id,state,ended_at_utc_ms FROM recording_sessions WHERE meeting_id=?1",
            [meeting],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_ne!(id, fixture.session.id);
    assert_eq!(state, "interrupted");
    assert_eq!(ended, None);
    assert!(target.review_partial_recording(&id).is_err());
    assert!(target.get_meeting(&existing).unwrap().is_some());
    let path: String = target
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1",
            [&audio.manifest_asset_id],
            |row| row.get(0),
        )
        .unwrap();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(target.root.join(path)).unwrap()).unwrap();
    assert_eq!(manifest["partialRecovery"]["kind"], "sealed_prefix");
    assert_eq!(manifest["partialRecovery"]["acquisitionEndKnown"], false);
    assert!(manifest["partialRecovery"]["tailLossUs"].is_null());
    assert_eq!(remove_reviewed_audio(&target, meeting), 2);
    assert!(target.recorded_analysis_input(meeting).is_err());
    assert!(fixture
        .storage
        .recorded_analysis_input(&fixture.session.meeting_id)
        .is_ok());
}

#[test]
fn partial_recovery_no_owned_or_empty_prefix_cannot_adopt_tail_and_new_lane_needs_reconfirmation() {
    for empty in [true, false] {
        let fixture = partial_fixture();
        for lane in ["microphone", "system"] {
            let path = fixture.base.join(format!("raw-{lane}.json"));
            let mut tip: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if empty {
                tip["sealedSegments"] = json!(0);
                tip["sealedFrames"] = json!(0);
                tip["lastEntrySHA256"] = Value::Null;
            } else {
                tip["ownerId"] = json!(Uuid::new_v4().to_string());
            }
            write(&path, &serde_json::to_vec(&tip).unwrap());
        }
        let review = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        assert_eq!(review.summary.duration_us, 0);
        assert!(review
            .summary
            .lanes
            .iter()
            .all(|lane| lane.state == if empty { "empty" } else { "unreadable" }));
        assert!(fixture
            .storage
            .recover_partial_recording(&review, true)
            .is_err());
        assert_eq!(partial_registered_rows(&fixture), 0);
        assert!(!fixture.base.join(PARTIAL_PLAN).exists());
        assert!(fixture.base.join("raw-system-00000003.caf").is_file());
    }
    let fixture = partial_fixture();
    let path = fixture.base.join("raw-microphone.json");
    let original = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert_eq!(review.summary.lanes[0].state, "missing");
    write(&path, &original);
    assert!(fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap_err()
        .contains("変更"));
    assert_eq!(partial_registered_rows(&fixture), 0);
    assert!(!fixture.base.join(PARTIAL_PLAN).exists());
}

fn dense_raw_maps(fixture: &Fixture) {
    let manifest_path = fixture.base.join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    for lane in ["microphone", "system"] {
        let tip_path = fixture.base.join(format!("raw-{lane}.json"));
        let mut tip: Value = serde_json::from_slice(&std::fs::read(&tip_path).unwrap()).unwrap();
        let mut previous = None::<String>;
        for sequence in 1..=tip["sealedSegments"].as_u64().unwrap() {
            let path = fixture.base.join(format!("raw-{lane}-{sequence:08}.json"));
            let mut entry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let frames = entry["frames"].as_i64().unwrap();
            assert_eq!(frames % 200, 0);
            let step = frames / 200;
            let origin = entry["captureOriginNanoseconds"].as_i64().unwrap();
            entry["clockSegments"] = json!((0..200)
                .map(|i| map(
                    i * step,
                    (i + 1) * step,
                    i * step * 125,
                    (i + 1) * step * 125
                ))
                .collect::<Vec<_>>());
            entry["clockNanoseconds"]=json!((0..200).map(|i|json!({"startNanoseconds":origin+i*step*125000,"endNanoseconds":origin+(i+1)*step*125000})).collect::<Vec<_>>());
            entry["previousEntrySHA256"] = json!(previous);
            let bytes = serde_json::to_vec(&entry).unwrap();
            assert!(bytes.len() <= 65 * 1024);
            previous = Some(hash(&bytes));
            write(&path, &bytes);
        }
        tip["lastEntrySHA256"] = json!(previous);
        write(&tip_path, &serde_json::to_vec(&tip).unwrap());
        manifest[lane]["rawSegments"]["lastEntrySHA256"] = tip["lastEntrySHA256"].clone();
    }
    write(&manifest_path, &serde_json::to_vec(&manifest).unwrap());
}

fn assert_partial_whole_meeting_playback(storage: &Storage, meeting: &str, duration: i64) {
    use crate::storage::meeting_playback::{MeetingPlaybackMode, MeetingPlaybackRequest};
    let store = crate::playback::PlaybackStore::default();
    for mode in [
        MeetingPlaybackMode::Mic,
        MeetingPlaybackMode::System,
        MeetingPlaybackMode::Both,
    ] {
        let audio = storage
            .prepare_meeting_audio(&MeetingPlaybackRequest {
                meeting_id: meeting.into(),
                mode,
            })
            .unwrap();
        let info = store.insert_meeting(audio).unwrap();
        assert_eq!(info.end_us, duration);
        assert_eq!(info.alignment, "unverified");
        assert!(!info.missing_ranges.is_empty());
        for gap in &info.missing_ranges {
            assert!(store.read(&info.token, gap.start_frame).is_err());
        }
        // Beyond the inter-segment gap, raw and final chunk boundaries do not
        // erase actual PCM or add filler frames to known analysis input.
        let first = crate::audio_clock::frame_at(4_800_000, 48000).unwrap();
        assert!(store.read(&info.token, first).is_ok());
        store.close(&info.token).unwrap();
    }
}

#[test]
fn partial_recovery_and_normal_stop_whole_meeting_playback_keep_1400_maps_per_track() {
    for partial in [false, true] {
        let fixture = Fixture::new(7, 2000, true);
        let fixture = if partial {
            interrupt_fixture(fixture)
        } else {
            fixture
        };
        dense_raw_maps(&fixture);
        let result = if partial {
            let review = fixture
                .storage
                .review_partial_recording(&fixture.session.id)
                .unwrap();
            fixture
                .storage
                .recover_partial_recording(&review, true)
                .unwrap()
        } else {
            fixture
                .storage
                .finalize_recording(&fixture.session.id)
                .unwrap()
        };
        let (source, known) = fixture
            .storage
            .recorded_analysis_input(&result.meeting_id)
            .unwrap();
        known.unwrap().validate(&source).unwrap();
        for track in &source.tracks {
            assert_eq!(track.chunks.len(), 2);
            assert_eq!(
                track
                    .chunks
                    .iter()
                    .map(|chunk| chunk.clock_segments.len())
                    .sum::<usize>(),
                1400
            );
            assert!(track
                .chunks
                .iter()
                .all(|chunk| chunk.clock_segments.len() <= 1024));
        }
        assert_partial_whole_meeting_playback(
            &fixture.storage,
            &result.meeting_id,
            result.duration_us,
        );
    }
}

#[test]
fn partial_recovery_commit_marker_cannot_authorize_unmarked_track_or_invent_known_end() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    let result = fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    assert_partial_whole_meeting_playback(&fixture.storage, &result.meeting_id, result.duration_us);
    let db = fixture.storage.connection.lock().unwrap();
    let track: String = db
        .query_row(
            "SELECT id FROM audio_tracks WHERE kind='system'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let original: String = db
        .query_row(
            "SELECT processing_config_json FROM audio_tracks WHERE id=?1",
            [&track],
            |row| row.get(0),
        )
        .unwrap();
    let mut changed: Value = serde_json::from_str(&original).unwrap();
    changed["partial_recovery"]["reviewHash"] = json!("f".repeat(64));
    db.execute(
        "UPDATE audio_tracks SET processing_config_json=?1 WHERE id=?2",
        params![changed.to_string(), track],
    )
    .unwrap();
    assert!(
        !crate::storage::transcription::partial_recording_committed(&db, &result.meeting_id)
            .unwrap()
    );
    db.execute(
        "UPDATE audio_tracks SET processing_config_json='{}' WHERE id=?1",
        [&track],
    )
    .unwrap();
    drop(db);
    assert!(fixture
        .storage
        .recorded_analysis_input(&result.meeting_id)
        .is_err());
    use crate::storage::meeting_playback::{MeetingPlaybackMode, MeetingPlaybackRequest};
    assert!(fixture
        .storage
        .prepare_meeting_audio(&MeetingPlaybackRequest {
            meeting_id: result.meeting_id.clone(),
            mode: MeetingPlaybackMode::Mic
        })
        .is_err());
    let db = fixture.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE audio_tracks SET processing_config_json=?1 WHERE id=?2",
        params![original, track],
    )
    .unwrap();
    db.execute(
        "UPDATE recording_sessions SET ended_at_utc_ms=123 WHERE id=?1",
        [&result.id],
    )
    .unwrap();
    assert!(
        !crate::storage::transcription::partial_recording_committed(&db, &result.meeting_id)
            .unwrap()
    );
    drop(db);
    assert!(fixture
        .storage
        .recorded_analysis_input(&result.meeting_id)
        .is_err());
}

#[test]
fn partial_recovery_symlinked_input_and_replaced_directory_are_retained_not_adopted() {
    let fixture = partial_fixture();
    let external = tempfile::tempdir().unwrap();
    let target = external.path().join("untouched.caf");
    let original = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
    write(&target, &original);
    let path = fixture.base.join("raw-microphone-00000001.caf");
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert_eq!(review.summary.lanes[0].state, "unreadable");
    let previous = fixture.base.with_file_name(Uuid::new_v4().to_string());
    std::fs::rename(&fixture.base, &previous).unwrap();
    std::os::unix::fs::symlink(&previous, &fixture.base).unwrap();
    assert!(fixture
        .storage
        .recover_partial_recording(&review, true)
        .is_err());
    assert_eq!(partial_registered_rows(&fixture), 0);
    assert_eq!(std::fs::read(target).unwrap(), original);
    assert!(previous.join("raw-system-00000001.caf").is_file());
}

#[test]
fn partial_recovery_preserves_original_failure_observation_without_claiming_acquisition_closure() {
    let fixture = Fixture::new(2, 2000, true);
    let failed = fixture.storage.fail_recording(&fixture.session.id).unwrap();
    assert_eq!(failed.state, "failed");
    assert!(failed.ended_at_utc_ms.is_some());
    let fixture = interrupt_fixture(fixture);
    let original = std::fs::read(fixture.base.join("manifest.json")).unwrap();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    let result = fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    assert_eq!(result.state, "interrupted");
    assert_eq!(result.ended_at_utc_ms, None);
    let document: Value =
        serde_json::from_slice(&std::fs::read(fixture.base.join("partial-recovery.json")).unwrap())
            .unwrap();
    assert_eq!(document["partialRecovery"]["originalState"], "failed");
    assert_eq!(
        document["partialRecovery"]["originalObservedEndUtcMs"],
        json!(failed.ended_at_utc_ms)
    );
    assert_eq!(
        document["partialRecovery"]["originalDurationUs"],
        json!(failed.duration_us)
    );
    assert_eq!(document["partialRecovery"]["acquisitionEndKnown"], false);
    assert!(document["partialRecovery"]["tailLossUs"].is_null());
    assert_eq!(
        std::fs::read(fixture.base.join("manifest.json")).unwrap(),
        original
    );
    assert!(fixture
        .storage
        .recorded_analysis_input(&result.meeting_id)
        .is_ok());
}

#[test]
#[ignore = "Owned synthetic SIGKILL child; only its dedicated parent may launch it"]
fn partial_recovery_crash_child() {
    assert_eq!(
        std::env::var("MON_FINAL_FLAC_CRASH_CHILD").unwrap(),
        "synthetic-only"
    );
    let root = PathBuf::from(std::env::var_os("MON_FINAL_FLAC_CRASH_ROOT").unwrap());
    let owner = std::env::var("MON_FINAL_FLAC_CRASH_OWNER").unwrap();
    let storage = Storage::open(root).unwrap();
    let review = storage.review_partial_recording(&owner).unwrap();
    storage.recover_partial_recording(&review, true).unwrap();
    panic!("owned partial-recovery child failed to reach its crash point");
}

#[test]
#[ignore = "Dedicated owned-child crash/restart verification; synthetic PCM only, no capture devices"]
fn partial_recovery_owned_sigkill_checkpoints_keep_interrupted_and_require_fresh_consent() {
    use std::os::unix::process::ExitStatusExt;
    for phase in [
        "reserved",
        "created-before-identity",
        "allocated",
        "encoded",
        "published",
        "partial-before-db-commit",
    ] {
        let Fixture {
            temp,
            storage,
            session,
            base,
        } = partial_fixture();
        let original = std::fs::read(base.join("raw-microphone-00000001.caf")).unwrap();
        drop(storage);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::recordings::flac::tests::partial_recovery_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
            .env("MON_FINAL_FLAC_CRASH_PHASE", phase)
            .env("MON_FINAL_FLAC_CRASH_ROOT", temp.path())
            .env("MON_FINAL_FLAC_CRASH_OWNER", &session.id)
            .output()
            .unwrap();
        assert_eq!(
            child.status.signal(),
            Some(libc::SIGKILL),
            "{phase}: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert!(reopened.recover_completed_recordings().unwrap().is_empty());
        let plan_bytes = std::fs::read(base.join(PARTIAL_PLAN)).unwrap();
        let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
        {
            let db = reopened.connection.lock().unwrap();
            let current = load_session(&db, &session.id).unwrap().unwrap();
            assert_eq!(current.state, "interrupted");
            assert_eq!(current.duration_us, 0);
            for table in [
                "file_assets",
                "audio_tracks",
                "audio_chunks",
                "track_clock_segments",
                "audio_gaps",
            ] {
                assert_eq!(
                    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                        .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "{phase}: {table}"
                );
            }
        }
        let review = reopened.review_partial_recording(&session.id).unwrap();
        assert!(reopened.recover_partial_recording(&review, false).is_err());
        let result = reopened.recover_partial_recording(&review, true).unwrap();
        assert_eq!(result.state, "interrupted");
        assert_eq!(result.ended_at_utc_ms, None);
        assert_eq!(std::fs::read(base.join(PARTIAL_PLAN)).unwrap(), plan_bytes);
        let (source, known) = reopened
            .recorded_analysis_input(&result.meeting_id)
            .unwrap();
        assert_eq!(known.unwrap().runs.len(), 4);
        for track in &source.tracks {
            assert!(plan.tracks.iter().any(|planned| planned.id == track.id));
        }
        assert_eq!(
            std::fs::read(base.join("raw-microphone-00000001.caf")).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read(base.join("raw-system-00000003.caf")).unwrap(),
            b"unfinished tail"
        );
        assert_partial_whole_meeting_playback(&reopened, &result.meeting_id, result.duration_us);
    }
}
