use super::*;
use crate::audio::{flac::encode_segment_file, VerifiedAudio};
use serde_json::{json, Value};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(target_os = "macos")]
#[path = "relocation_tests.rs"]
mod relocation;

pub(crate) struct Fixture {
    pub(crate) temp: tempfile::TempDir,
    pub(crate) storage: Storage,
    pub(crate) session: RecordingSession,
    pub(crate) base: PathBuf,
}
fn write(path: &Path, bytes: &[u8]) {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).unwrap();
    #[cfg(windows)]
    crate::windows_files::set_private(path).unwrap();
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

#[cfg(target_os = "macos")]
#[test]
fn unreceived_system_recording_rejects_conflicting_payload_unclosed_owner_and_failures() {
    for problem in [
        "payload",
        "callback",
        "frames",
        "append",
        "capture",
        "detail",
        "clock",
        "owner",
        "missing-end",
        "open-tip",
        "failed-tip",
        "foreign-tip",
        "tip-frames",
        "mic-origin",
    ] {
        let f = Fixture::new(1, 2000, false);
        let path = f.base.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut tip = json!({"schemaVersion":2,"ownerId":f.session.id,"lane":"system",
            "sealedSegments":0,"sealedFrames":0,"lastEntrySHA256":null,"producerClosed":true});
        if problem == "open-tip" {
            tip["producerClosed"] = json!(false);
        }
        if problem == "foreign-tip" {
            tip["ownerId"] = json!(Uuid::new_v4().to_string());
        }
        if problem == "tip-frames" {
            tip["sealedFrames"] = json!(1);
        }
        write(
            &f.base.join("raw-system.json"),
            &serde_json::to_vec(&tip).unwrap(),
        );
        tip["failure"] = if problem == "failed-tip" {
            json!("synthetic sync failure")
        } else {
            Value::Null
        };
        manifest["systemAudioUnreceived"] = json!(true);
        manifest["system"] = json!({"frames":0,"callbackCount":0,"appendFailures":0,
            "clockSegments":[],"rawSegments":tip});
        match problem {
            "callback" => manifest["system"]["callbackCount"] = json!(1),
            "frames" => manifest["system"]["frames"] = json!(1),
            "append" => manifest["system"]["appendFailures"] = json!(1),
            "capture" => manifest["system"]["captureFailure"] = json!("capture_audio_write_failed"),
            "detail" => manifest["system"]["captureFailureDetail"] = json!("denied"),
            "clock" => manifest["system"]["clockSegments"] = Value::Null,
            "owner" => manifest["recordingOwnerId"] = json!(Uuid::new_v4().to_string()),
            "missing-end" => manifest["captureEnd"] = Value::Null,
            "mic-origin" => {
                manifest["microphone"]["clockSegments"][0]["timelineStartUs"] = json!(100);
                manifest["microphone"]["clockSegments"][0]["timelineEndUs"] = json!(250100);
            }
            _ => {}
        }
        if problem != "payload" {
            std::fs::remove_file(f.base.join("system.caf")).unwrap();
        }
        write(&path, &serde_json::to_vec(&manifest).unwrap());
        assert!(
            f.storage.finalize_recording(&f.session.id).is_err(),
            "{problem}"
        );
        assert_eq!(f.registered_tracks(), 0, "{problem}");
        assert!(f.base.join("microphone.caf").exists());
    }
}

#[test]
fn unreceived_microphone_recording_rejects_conflicting_payload_unclosed_owner_and_failures() {
    for problem in [
        "payload",
        "callback",
        "frames",
        "append",
        "capture",
        "detail",
        "clock",
        "owner",
        "missing-end",
        "open-tip",
        "failed-tip",
        "foreign-tip",
        "tip-frames",
        "pc-origin",
    ] {
        let f = Fixture::new(1, 2000, false);
        let path = f.base.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut tip = json!({"schemaVersion":2,"ownerId":f.session.id,"lane":"microphone",
            "sealedSegments":0,"sealedFrames":0,"lastEntrySHA256":null,"producerClosed":true});
        if problem == "open-tip" {
            tip["producerClosed"] = json!(false);
        }
        if problem == "foreign-tip" {
            tip["ownerId"] = json!(Uuid::new_v4().to_string());
        }
        if problem == "tip-frames" {
            tip["sealedFrames"] = json!(1);
        }
        write(
            &f.base.join("raw-microphone.json"),
            &serde_json::to_vec(&tip).unwrap(),
        );
        tip["failure"] = if problem == "failed-tip" {
            json!("synthetic sync failure")
        } else {
            Value::Null
        };
        manifest["microphoneAudioUnreceived"] = json!(true);
        manifest["microphone"] = json!({"frames":0,"callbackCount":0,"appendFailures":0,
            "clockSegments":[],"rawSegments":tip});
        manifest["captureClock"]["originUtcMs"] = json!(1700000000050_i64);
        for segment in manifest["system"]["clockSegments"].as_array_mut().unwrap() {
            for key in ["timelineStartUs", "timelineEndUs"] {
                segment[key] = json!(segment[key].as_i64().unwrap() - 50000);
            }
        }
        match problem {
            "callback" => manifest["microphone"]["callbackCount"] = json!(1),
            "frames" => manifest["microphone"]["frames"] = json!(1),
            "append" => manifest["microphone"]["appendFailures"] = json!(1),
            "capture" => {
                manifest["microphone"]["captureFailure"] = json!("capture_audio_write_failed")
            }
            "detail" => manifest["microphone"]["captureFailureDetail"] = json!("denied"),
            "clock" => manifest["microphone"]["clockSegments"] = Value::Null,
            "owner" => manifest["recordingOwnerId"] = json!(Uuid::new_v4().to_string()),
            "missing-end" => manifest["captureEnd"] = Value::Null,
            "pc-origin" => {
                manifest["system"]["clockSegments"][0]["timelineStartUs"] = json!(100);
                manifest["system"]["clockSegments"][0]["timelineEndUs"] = json!(250100);
            }
            _ => {}
        }
        if problem != "payload" {
            std::fs::remove_file(f.base.join("microphone.caf")).unwrap();
        }
        write(&path, &serde_json::to_vec(&manifest).unwrap());
        assert!(
            f.storage.finalize_recording(&f.session.id).is_err(),
            "{problem}"
        );
        assert_eq!(f.registered_tracks(), 0, "{problem}");
        assert!(f.base.join("system.caf").exists());
    }
}
impl Fixture {
    pub(crate) fn registered_tracks(&self) -> i64 {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM audio_tracks", [], |r| r.get(0))
            .unwrap()
    }
    pub(crate) fn reject_partial_commit(&self, reject: bool) {
        self.storage.connection.lock().unwrap().execute_batch(if reject {"CREATE TRIGGER reject_partial_controller BEFORE UPDATE OF audio_manifest_hash ON recording_sessions BEGIN SELECT RAISE(ABORT,'synthetic controller rollback'); END"} else {"DROP TRIGGER reject_partial_controller"}).unwrap();
    }
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
        let manifest = json!({"schemaVersion":4,"inputDiagnosticsState":"unavailable","captureMode":"system","backend":"screencapturekit",
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
        .begin_data_maintenance(false, false, &crate::test_support::never_cancelled)
        .unwrap();
    let census = Census::capture(storage, meeting, &crate::test_support::never_cancelled).unwrap();
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
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    intent::removal::start(
        storage,
        &operation,
        &lease,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert!(intent::removal::run(
        storage,
        &operation,
        &lease,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .files_removed());
    assert!(intent::removal::completion::finish(
        storage,
        &operation,
        &lease,
        &crate::test_support::never_cancelled
    )
    .unwrap()
    .completed());
    count
}

#[cfg(target_os = "macos")]
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

pub(crate) fn partial_fixture() -> Fixture {
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
        .pending_capture_input_for_test(&result.meeting_id)
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
    assert_eq!(partial_registered_rows(&fixture), 3);
    drop(fixture.storage);
    let reopened = Storage::open(fixture.temp.path().to_owned()).unwrap();
    assert!(reopened.recover_completed_recordings().unwrap().is_empty());
    assert!(reopened
        .pending_capture_input_for_test(&result.meeting_id)
        .is_ok());
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
            .pending_capture_input_for_test(&result.meeting_id)
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
        .pending_capture_audio_for_test(&fixture.session.meeting_id)
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
        .pending_capture_input_for_test(&result.meeting_id)
        .unwrap();
    for track in &audio.tracks {
        assert!(plan.tracks.iter().any(|item| item.id == track.id));
    }
}

#[test]
fn partial_startup_backup_additive_restore_and_audio_deletion_keep_history_not_authority() {
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
        .create_backup_archive(&archive, &crate::test_support::never_cancelled)
        .unwrap();
    assert_eq!(receipt.asset_count, 2);
    let root = tempfile::tempdir().unwrap();
    let target = Storage::open(root.path().to_owned()).unwrap();
    let existing = target.create_meeting("保持する会議").unwrap().id;
    let selection = target
        .prepare_additive_restore(&archive, &crate::test_support::never_cancelled, &|_| {})
        .unwrap();
    let done = selection
        .publish(
            &target,
            &selection.preview().confirmation_id,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
    let meeting = &done.meeting_ids[0];
    let (audio, known) = target.recorded_analysis_input(meeting).unwrap();
    assert_eq!(known.unwrap().runs.len(), 1);
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
    let history = target.partial_recovery_listing(None).unwrap();
    assert_eq!(history.items.len(), 1);
    let saved = &history.items[0].summary;
    assert_eq!(&saved.meeting_id, meeting);
    assert_eq!(saved.session_id, id);
    assert!(!saved.acquisition_end_known);
    assert_eq!(saved.tail_loss_us, None);
    assert_eq!(
        target
            .partial_recovery_result(&id, &saved.review_hash)
            .unwrap(),
        Some(saved.clone())
    );
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
    assert_eq!(remove_reviewed_audio(&target, meeting), 3);
    assert!(target.recorded_analysis_input(meeting).is_err());
    assert_eq!(target.partial_recovery_listing(None).unwrap(), history);
    assert_eq!(
        target
            .partial_recovery_result(&id, &saved.review_hash)
            .unwrap(),
        Some(saved.clone())
    );
    assert!(target.review_partial_recording(&id).is_err());
    assert!(fixture
        .storage
        .pending_capture_input_for_test(&fixture.session.meeting_id)
        .is_ok());
}

#[test]
fn partial_startup_cold_read_lists_only_commits_without_adopting_raw_or_reopening_capture() {
    let fixture = partial_fixture();
    assert!(fixture
        .storage
        .partial_recovery_listing(None)
        .unwrap()
        .items
        .is_empty());
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert!(fixture
        .storage
        .partial_recovery_listing(None)
        .unwrap()
        .items
        .is_empty());
    fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    let raw = std::fs::read(fixture.base.join("raw-system-00000003.caf")).unwrap();
    let history = fixture.storage.partial_recovery_listing(None).unwrap();
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].summary, review.summary);
    drop(fixture.storage);
    let reopened = Storage::open(fixture.temp.path().to_owned()).unwrap();
    assert_eq!(reopened.partial_recovery_listing(None).unwrap(), history);
    assert_eq!(
        std::fs::read(fixture.base.join("raw-system-00000003.caf")).unwrap(),
        raw
    );
    assert!(reopened
        .partial_recording_listing(None)
        .unwrap()
        .items
        .is_empty());
    assert!(reopened
        .review_partial_recording(&fixture.session.id)
        .is_err());
    assert!(reopened.partial_recovery_listing(Some("invalid")).is_err());
}

#[test]
fn partial_startup_contradictory_saved_metadata_fails_closed_instead_of_disappearing() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    let original: String = fixture
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT processing_config_json FROM audio_tracks WHERE session_id=?1 AND kind='system'",
            [&fixture.session.id],
            |row| row.get(0),
        )
        .unwrap();
    for change in [
        "json_set(processing_config_json,'$.partial_recovery.lanes[0].frames',1)",
        "json_set(processing_config_json,'$.partial_recovery',NULL)",
        "json_set(processing_config_json,'$.partial_recovery.reviewHash','invalid')",
        "json_set(processing_config_json,'$.padding',printf('%.*c',4194305,'x'))",
    ] {
        fixture.storage.connection.lock().unwrap().execute(&format!(
            "UPDATE audio_tracks SET processing_config_json={change} WHERE session_id=?1 AND kind='system'"), [&fixture.session.id]).unwrap();
        assert!(
            fixture.storage.partial_recovery_listing(None).is_err(),
            "{change}"
        );
        fixture.storage.connection.lock().unwrap().execute(
            "UPDATE audio_tracks SET processing_config_json=?1 WHERE session_id=?2 AND kind='system'",
            rusqlite::params![original, fixture.session.id]).unwrap();
    }
    fixture
        .storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET channels=1 WHERE session_id=?1 AND kind='system'",
            [&fixture.session.id],
        )
        .unwrap();
    assert!(fixture.storage.partial_recovery_listing(None).is_err());
    assert!(fixture
        .storage
        .partial_recovery_result(&fixture.session.id, &review.summary.review_hash)
        .is_err());
    fixture
        .storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_tracks SET channels=2,kind='mic' WHERE session_id=?1 AND kind='system'",
            [&fixture.session.id],
        )
        .unwrap();
    assert!(fixture.storage.partial_recovery_listing(None).is_err());
}

#[test]
fn partial_startup_history_paginates_201_registered_metadata_rows_without_claiming_bodies() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    fixture
        .storage
        .recover_partial_recording(&review, true)
        .unwrap();
    // Metadata-only pagination fixture, not 200 additional capture/conversion claims.
    let mut db = fixture.storage.connection.lock().unwrap();
    let tx = db.transaction().unwrap();
    for _ in 0..200 {
        let meeting = Uuid::new_v4().to_string();
        let session = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO meetings(id,title,duration_us,lifecycle,source_type,created_at,updated_at) SELECT ?1,title,duration_us,lifecycle,source_type,created_at,updated_at FROM meetings WHERE id=?2", rusqlite::params![meeting, fixture.session.meeting_id]).unwrap();
        tx.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,duration_us,audio_manifest_hash,last_committed_us,created_at,updated_at) SELECT ?1,?2,state,capture_mode,capture_config_json,?3,duration_us,audio_manifest_hash,last_committed_us,created_at,updated_at FROM recording_sessions WHERE id=?4", rusqlite::params![session,meeting,Uuid::new_v4().to_string(),fixture.session.id]).unwrap();
        tx.execute("INSERT INTO file_assets(id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) SELECT ?1,?2,storage_scope,?3,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at FROM file_assets WHERE meeting_id=?4 AND media_type='application/json'", rusqlite::params![Uuid::new_v4().to_string(),meeting,format!("meetings/{meeting}/history.json"),fixture.session.meeting_id]).unwrap();
        for kind in ["mic", "system"] {
            tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) SELECT ?1,?2,?3,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at FROM audio_tracks WHERE session_id=?4 AND kind=?5", rusqlite::params![Uuid::new_v4().to_string(),meeting,session,fixture.session.id,kind]).unwrap();
        }
    }
    tx.commit().unwrap();
    drop(db);
    let first = fixture.storage.partial_recovery_listing(None).unwrap();
    assert_eq!(first.items.len(), 200);
    assert_eq!(
        first.next.as_deref(),
        Some(first.items.last().unwrap().summary.session_id.as_str())
    );
    let last = fixture
        .storage
        .partial_recovery_listing(first.next.as_deref())
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert_eq!(last.next, None);
    assert!(last.items[0].summary.session_id > first.next.unwrap());
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
    let audio = storage
        .prepare_meeting_audio(&MeetingPlaybackRequest {
            meeting_id: meeting.into(),
            mode: MeetingPlaybackMode::Mixed,
            start_reference: None,
        })
        .unwrap();
    let store = crate::playback::PlaybackStore::default();
    let info = store.insert_meeting(audio).unwrap();
    assert_eq!(info.end_us, duration);
    assert_eq!(info.sample_rate, 16000);
    assert!(info.missing_ranges.is_empty());
    assert!(store
        .read(
            &info.token,
            crate::audio_clock::frame_at(4_800_000, 16000).unwrap()
        )
        .is_ok());
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
            .pending_capture_input_for_test(&result.meeting_id)
            .unwrap();
        known.unwrap().validate(&source).unwrap();
        for track in &source.tracks {
            assert!((1..=2).contains(&track.chunks.len()));
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
                .all(|chunk| chunk.clock_segments.len() <= crate::audio_clock::MAX_CLOCK_SEGMENTS));
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
        .pending_capture_input_for_test(&result.meeting_id)
        .is_err());
    use crate::storage::meeting_playback::{MeetingPlaybackMode, MeetingPlaybackRequest};
    assert!(fixture
        .storage
        .prepare_meeting_audio(&MeetingPlaybackRequest {
            start_reference: None,
            meeting_id: result.meeting_id.clone(),
            mode: MeetingPlaybackMode::Mixed
        })
        .is_ok());
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
        .pending_capture_input_for_test(&result.meeting_id)
        .is_err());
}

#[test]
#[cfg(unix)]
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
        .pending_capture_input_for_test(&result.meeting_id)
        .is_ok());
}

#[test]
#[cfg(unix)]
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
#[cfg(unix)]
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
            .pending_capture_input_for_test(&result.meeting_id)
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

// Inspect retained capture provenance in recovery tests; normal readers use WAV.
impl Storage {
    fn pending_capture_input_for_test(
        &self,
        meeting: &str,
    ) -> Result<
        (
            crate::storage::RecordedAudioPlan,
            Option<crate::storage::KnownInput>,
        ),
        String,
    > {
        let db = self.connection.lock().unwrap();
        crate::storage::deletion::audio::require_source(&db, meeting)?;
        let interrupted:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE meeting_id=?1 AND state='interrupted')",[meeting],|r|r.get(0)).map_err(|e|e.to_string())?;
        if interrupted && !crate::storage::transcription::partial_recording_committed(&db, meeting)?
        {
            return Err("部分救出の元の確定記録が変更されています".into());
        }
        let saved: Option<String> = db
            .query_row(
                "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
                [meeting],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let source = if let Some(json) = saved {
            serde_json::from_str(&json).map_err(|e| e.to_string())?
        } else {
            crate::storage::audio_publication::inputs::read(&db, meeting)?
        };
        let known = crate::storage::transcription::known::read(&db, meeting, &source)?;
        Ok((source, Some(known)))
    }
    fn pending_capture_audio_for_test(
        &self,
        meeting: &str,
    ) -> Result<crate::storage::RecordedAudioPlan, String> {
        self.pending_capture_input_for_test(meeting).map(|v| v.0)
    }
}
