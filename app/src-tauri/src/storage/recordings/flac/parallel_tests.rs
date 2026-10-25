//! Portable raw-recording fixtures. No device, real recording or engine.
use super::*;
use crate::audio::VerifiedAudio;
use serde_json::{json, Value};
struct Fixture {
    _temp: tempfile::TempDir,
    storage: Storage,
    session: RecordingSession,
    base: PathBuf,
}
fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(windows)]
    crate::windows_files::set_private(path).unwrap();
    std::fs::File::open(path).unwrap().sync_all().ok();
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
            // Native manifests coalesce continuous clock intervals. Raw
            // per-segment proofs remain separate and fully hash-checked.
            if !gap {
                maps = vec![map(0, frames as i64, offset, frames as i64 * 125 + offset)];
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
            _temp: temp,
            storage,
            session,
            base,
        }
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

#[test]
fn parallel_raw_save_keeps_chunk_order_source_samples_and_atomic_registration() {
    let f = Fixture::new(17, 2000, true);
    let original = std::fs::read(f.base.join("microphone.caf")).unwrap();
    let saved = f.storage.finalize_recording(&f.session.id).unwrap();
    assert_eq!(saved.state, "saved");
    // The current input API preserves gaps; the legacy contiguous plan is
    // intentionally unsuitable for this synthetic discontinuous recording.
    let (_, known) = f
        .storage
        .recorded_analysis_input(&f.session.meeting_id)
        .unwrap();
    assert_eq!(known.unwrap().runs.len(), 4);
    let chunks: Vec<(String, i64, String)> = {
        let db = f.storage.connection.lock().unwrap();
        let mut q = db.prepare("SELECT f.relative_path,f.size_bytes,f.content_hash FROM audio_tracks t JOIN audio_chunks c ON c.track_id=t.id JOIN file_assets f ON f.id=c.file_asset_id WHERE t.meeting_id=?1 ORDER BY t.kind,c.sequence").unwrap();
        q.query_map([&f.session.meeting_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    assert_eq!(chunks.len(), 4);
    for (relative, size, hash) in chunks {
        let mut audio = VerifiedAudio::open(
            File::open(f.storage.managed_path(Path::new(&relative)).unwrap()).unwrap(),
            size as u64,
            &hash,
        )
        .unwrap();
        assert_eq!(
            audio.read_frames(0, 1).unwrap(),
            [0.25_f32.to_le_bytes(), (-0.5_f32).to_le_bytes()].concat()
        );
    }
    assert_eq!(
        std::fs::read(f.base.join("microphone.caf")).unwrap(),
        original
    );
}
#[test]
fn raw_save_db_failure_retains_source_and_retry_verifies_owned_chunks() {
    let f = Fixture::new(2, 2000, false);
    f.storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_save BEFORE UPDATE OF audio_manifest_hash ON recording_sessions BEGIN SELECT RAISE(ABORT,'synthetic rollback'); END").unwrap();
    assert!(f.storage.finalize_recording(&f.session.id).is_err());
    assert!(f.base.join("raw-microphone-00000001.caf").is_file());
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_save")
        .unwrap();
    assert_eq!(
        f.storage.finalize_recording(&f.session.id).unwrap().state,
        "saved"
    );
    assert_eq!(
        f.storage
            .recorded_audio_plan(&f.session.meeting_id)
            .unwrap()
            .tracks
            .len(),
        2
    );
}

#[test]
#[ignore = "explicit synthetic recording benchmark; no device or user recording"]
fn benchmark_raw_recording_save() {
    let minutes: usize = std::env::var("MON_OPERATION_BENCH_MINUTES")
        .unwrap_or("5".into())
        .parse()
        .unwrap();
    assert!([5, 60, 180].contains(&minutes));
    let f = Fixture::new(minutes * 15, 32_000, false);
    let _measure = crate::operation_observation::Scope::enter("recording_benchmark", |_| {});
    let started = std::time::Instant::now();
    assert_eq!(
        f.storage.finalize_recording(&f.session.id).unwrap().state,
        "saved"
    );
    let saved_ms = started.elapsed().as_millis();
    let (_, input) = f
        .storage
        .recorded_analysis_input(&f.session.meeting_id)
        .unwrap();
    assert!(!input.unwrap().runs.is_empty());
    println!(
        "{}",
        serde_json::json!({"minutes":minutes,"rate":8000,"channels":2,"tracks":2,"saveMs":saved_ms,
        "readBytes":crate::operation_observation::measurements().0,"fullFileChecks":crate::operation_observation::measurements().1})
    );
}

#[test]
fn changing_an_already_converted_raw_group_still_blocks_whole_track_publication() {
    use std::{cell::Cell, io::Write, rc::Rc};
    let f = Fixture::new(33, 2000, false);
    let path = f.base.join("raw-microphone-00000001.caf");
    let changed = Rc::new(Cell::new(false));
    let changed_in_callback = changed.clone();
    let _measure = crate::operation_observation::Scope::enter("test", move |p| {
        // Ensure the first completed group is observable even on a fast host;
        // production notifications intentionally throttle to 250 ms.
        if p.stage == "converting_audio" && p.unit == "files" && p.completed == 0 {
            std::thread::sleep(std::time::Duration::from_millis(260));
        }
        if p.stage == "converting_audio"
            && p.unit == "files"
            && p.completed >= 1
            && !changed_in_callback.replace(true)
        {
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.seek(std::io::SeekFrom::Start(68)).unwrap();
            file.write_all(&0.75_f32.to_le_bytes()).unwrap();
            file.sync_all().unwrap();
        }
    });
    assert!(f.storage.finalize_recording(&f.session.id).is_err());
    assert!(
        changed.get(),
        "fixture never reached a completed independent group"
    );
    let db = f.storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM audio_tracks WHERE meeting_id=?1",
            [&f.session.meeting_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(f.base.join("raw-microphone-00000001.caf").is_file());
    assert!(f.base.join("raw-microphone-00000033.caf").is_file());
}
