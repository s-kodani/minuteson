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
