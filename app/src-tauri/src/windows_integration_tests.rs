//! Native Windows integration tests use owned temporary recordings only.
//! The helper's synthetic mode supplies deterministic PCM, never the microphone.
use crate::model_download_recipe::RecipeExt;
use crate::{model_download_recipe::Recipe, model_placement::fs::Directory, Storage};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use uuid::Uuid;
fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe")
        .canonicalize()
        .unwrap()
}

struct DriftFixture {
    _temp: tempfile::TempDir,
    storage: Storage,
    session: String,
    meeting: String,
    directory: PathBuf,
}
impl DriftFixture {
    fn new() -> Self {
        use std::os::windows::process::CommandExt;
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("data")).unwrap();
        let meeting = storage.create_meeting("Synthetic timestamp drift").unwrap();
        let session = storage
            .prepare_recording(
                &meeting.id,
                &crate::storage::RecordingConfig {
                    mode: "system".into(),
                    backend: "wasapi".into(),
                    target_pids: vec![],
                    microphone_device_uid: None,
                    target_application_identity: None,
                },
                &Uuid::new_v4().to_string(),
            )
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let directory = storage
            .root()
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        fs::create_dir_all(&directory).unwrap();
        let output = Command::new(helper())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .args([
                "--backend",
                "wasapi",
                "--mode",
                "system",
                "--input-level-owner",
                &session.id,
                "--parent-pid",
                &std::process::id().to_string(),
                "--synthetic",
                "--synthetic-clock-drift",
            ])
            .arg("--output")
            .arg(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            _temp: temp,
            storage,
            session: session.id,
            meeting: meeting.id,
            directory,
        }
    }
    fn legacy_manifest(&self) {
        let path = self.directory.join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("clockMappingSource");
        for lane in ["microphone", "system"] {
            let maps = value[lane]["clockSegments"].as_array().unwrap();
            assert!(maps.len() >= 2);
            let mut whole = maps[0].clone();
            whole["sourceFrameEnd"] = value[lane]["frames"].clone();
            whole["timelineEndUs"] = maps.last().unwrap()["timelineEndUs"].clone();
            value[lane]["clockSegments"] = serde_json::json!([whole]);
        }
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}

#[test]
fn windows_clock_drift_uses_the_same_committed_raw_maps_and_finalizes() {
    let f = DriftFixture::new();
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(f.directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["clockMappingSource"], "sealed_raw_v1");
    f.storage.finalize_recording(&f.session).unwrap();
    let plan = f.storage.recorded_audio_plan(&f.meeting).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].kind, "processed");
    assert_eq!(
        plan.tracks[0].end_us,
        plan.tracks[0].chunks[0].frames * 1000000 / 16000
    );
    f.storage
        .verify_canonical_audio(&f.meeting, &|| false)
        .unwrap();
}

#[test]
fn windows_legacy_clock_drift_recovers_to_canonical_audio_and_retains_manifest() {
    let f = DriftFixture::new();
    f.legacy_manifest();
    let manifest = fs::read(f.directory.join("manifest.json")).unwrap();
    f.storage.interrupt_recording(&f.session).unwrap();
    assert_eq!(
        f.storage.recover_completed_recordings().unwrap(),
        vec![f.session.clone()]
    );
    assert_eq!(
        fs::read(f.directory.join("manifest.json")).unwrap(),
        manifest
    );
    assert!(!f.directory.join("microphone.caf").exists());
    f.storage
        .verify_canonical_audio(&f.meeting, &|| false)
        .unwrap();
    let source: String = crate::storage::with_test_database(&f.storage, |db| {
        db.query_row(
            "SELECT source_json FROM meeting_audio_commits WHERE meeting_id=?1",
            [&f.meeting],
            |r| r.get(0),
        )
        .unwrap()
    });
    let source: crate::storage::RecordedAudioPlan = serde_json::from_str(&source).unwrap();
    assert!(source.tracks.iter().all(|t| t
        .chunks
        .iter()
        .map(|c| c.clock_segments.len())
        .sum::<usize>()
        >= 2));
    assert!(f.storage.recover_completed_recordings().unwrap().is_empty());
}

#[test]
fn windows_clock_reconciliation_rejects_different_pcm_and_new_producer_mismatch() {
    for problem in ["pcm", "canonical", "endpoint"] {
        let f = DriftFixture::new();
        f.legacy_manifest();
        let path = f.directory.join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if problem == "pcm" {
            let audio = f.directory.join("microphone.caf");
            let mut bytes = fs::read(&audio).unwrap();
            bytes[68..72].copy_from_slice(&0.25_f32.to_le_bytes());
            fs::write(audio, bytes).unwrap();
        } else if problem == "canonical" {
            value["clockMappingSource"] = serde_json::json!("sealed_raw_v1");
        } else {
            value["microphone"]["clockSegments"][0]["timelineEndUs"] = serde_json::json!(5000400);
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(
            f.storage.finalize_recording(&f.session).is_err(),
            "{problem}"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!f.directory.join("final-flac.json").exists());
    }
}

#[test]
#[ignore = "finalizes only an explicitly supplied isolated recording-error-copy; never the source app data"]
fn windows_clock_repair_on_isolated_recording_copy() {
    let root = PathBuf::from(
        std::env::var_os("MON_CLOCK_REPAIR_COPY_ROOT").expect("isolated copy required"),
    )
    .canonicalize()
    .unwrap();
    assert!(root.ends_with("recording-error-copy"));
    let proof: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("clock-repair-copy.json")).unwrap()).unwrap();
    let session = proof["sessionId"].as_str().unwrap();
    let meeting = proof["meetingId"].as_str().unwrap();
    let directory = root
        .join("meetings")
        .join(meeting)
        .join("audio")
        .join(session);
    let before = fs::read(directory.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(manifest["recordingOwnerId"], session);
    let storage = Storage::open(root).unwrap();
    let was_saved: bool = crate::storage::with_test_database(&storage, |db| {
        db.query_row(
            "SELECT state='saved' FROM recording_sessions WHERE id=?1",
            [session],
            |row| row.get(0),
        )
        .unwrap()
    });
    let recovered = storage.recover_completed_recordings().unwrap();
    assert!(was_saved || recovered.contains(&session.to_string()));
    // A recording with genuine capture gaps may be saved, but it cannot be
    // admitted as one continuous playback/analysis range. Check publication
    // without asking that separate consumer to erase the original gaps.
    let tracks: Vec<(String, i64)> = crate::storage::with_test_database(&storage, |db| {
        let mut query = db.prepare("SELECT t.kind,sum(c.frame_count) FROM audio_tracks t JOIN audio_chunks c ON c.track_id=t.id WHERE t.session_id=?1 GROUP BY t.id").unwrap();
        query
            .query_map([session], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    });
    assert_eq!(tracks.len(), 2);
    for (kind, frames) in &tracks {
        let lane = if kind == "mic" {
            "microphone"
        } else {
            "system"
        };
        assert_eq!(*frames, manifest[lane]["frames"].as_i64().unwrap());
    }
    let reconciled: i64 = crate::storage::with_test_database(&storage, |db| {
        db.query_row(
        "SELECT count(*) FROM audio_tracks WHERE session_id=?1 AND json_extract(processing_config_json,'$.legacy_clock_reconciled')=1",
        [session], |row| row.get(0)).unwrap()
    });
    assert_eq!(reconciled, 2);
    assert_eq!(fs::read(directory.join("manifest.json")).unwrap(), before);
    assert!(storage.recover_completed_recordings().unwrap().is_empty());
    eprintln!("Isolated recording copy recovered: {} tracks, complete source frames, unchanged manifest, retry idempotent", tracks.len());
}

#[test]
fn windows_resampled_packets_cross_raw_seal_without_reversing_or_losing_pcm() {
    use std::os::windows::process::CommandExt;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().canonicalize().unwrap();
    crate::windows_files::set_private(&directory).unwrap();
    let owner = Uuid::new_v4().to_string();
    let output = Command::new(helper())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .args([
            "--backend",
            "wasapi",
            "--mode",
            "system",
            "--input-level-owner",
            &owner,
            "--parent-pid",
            &std::process::id().to_string(),
            "--synthetic",
            "--synthetic-clock-resampling",
        ])
        .arg("--output")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["state"], "saved");
    assert_eq!(manifest["timingEstimated"], true);
    for lane in [
        crate::recording_segments::Lane::Microphone,
        crate::recording_segments::Lane::System,
    ] {
        let prefix = crate::recording_segments::Prefix::read_optional(
            crate::input_levels::open_capture_directory(&directory).unwrap(),
            &owner,
            lane,
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(prefix.frames(), 500 * 479);
        prefix.verify_unchanged(&|| false).unwrap();
        let groups = prefix
            .flac_groups(prefix.first_capture_nanoseconds().unwrap())
            .unwrap();
        assert_eq!(
            groups.iter().map(|group| group.frames).sum::<u64>(),
            500 * 479
        );
    }
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("capture-clock.json")).unwrap()).unwrap();
    assert_eq!(diagnostics["microphone"]["estimatedPackets"], 499);
    assert_eq!(
        diagnostics["microphone"]["firstAnomalies"][0]["reason"],
        "overlapping_timestamp"
    );
}
#[test]
#[ignore = "requires the isolated, completed all-model QA root"]
fn windows_real_model_body_deletion_keeps_history_and_other_packages() {
    let root = PathBuf::from(std::env::var_os("MON_WINDOWS_INTEGRATION_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(root.ends_with("windows-parity/model-validation"));
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("model-state.json")).unwrap()).unwrap();
    let id = state["windows-whisper-194-tiny"]["package"]
        .as_str()
        .unwrap();
    let other = state["windows-whisper-194-base"]["package"]
        .as_str()
        .unwrap();
    let storage = Storage::open_with_models(root.join("data"), root.join("models")).unwrap();
    let catalog = storage.list_model_placements().unwrap();
    let entry = catalog.iter().find(|entry| entry.package_id == id).unwrap();
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_model_deletion(false, false, &|| false)
        .unwrap();
    let mut deletion = crate::storage::deletion::models::operations::Prepared::fresh(
        &storage,
        &root.join("models"),
        id,
        &entry.review_hash,
        &lease,
        &|| false,
    )
    .unwrap();
    let preview = deletion.preview();
    assert!(preview.execute_allowed, "{:?}", preview.resume_reason);
    assert!(deletion
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| true,
            &|_| {}
        )
        .is_err());
    assert!(root.join("models").join(id).is_dir());
    let receipt = deletion
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(receipt.body_deleted);
    assert!(receipt.history_retained);
    assert!(root.join("models").join(other).is_dir());
    assert!(storage
        .list_model_placements()
        .unwrap()
        .iter()
        .any(|entry| entry.package_id == id));
}
#[test]
#[ignore = "requires SHA-verified real model assets in MON_WINDOWS_PARITY_ASSETS"]
fn windows_real_speaker_embedding_cpu_and_resident_reuse() {
    use crate::local_engine::Fingerprint;
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var_os("MON_WINDOWS_PARITY_ASSETS").unwrap()).unwrap(),
    )
    .unwrap();
    let asset = |role: &str| {
        Fingerprint::capture(
            Path::new(
                index
                    .as_object()
                    .unwrap()
                    .values()
                    .find(|v| v["role"] == role)
                    .unwrap()["path"]
                    .as_str()
                    .unwrap(),
            ),
            &|| false,
        )
        .unwrap()
    };
    let package = crate::windows_runtime::resources().unwrap().join("sherpa");
    let mut libraries = fs::read_dir(&package)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|v| v == "dll"))
        .collect::<Vec<_>>();
    libraries.sort();
    let engine = crate::diarization::sherpa::SherpaConfig {
        executable: Fingerprint::capture(
            &package.join("sherpa-onnx-offline-speaker-diarization.exe"),
            &|| false,
        )
        .unwrap(),
        segmentation_model: asset("segmentation"),
        embedding_model: asset("speaker_embedding"),
        runtime_libraries: libraries
            .iter()
            .map(|p| Fingerprint::capture(p, &|| false).unwrap())
            .collect(),
        threads: 2,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
        speech_sensitivity_milli: 0,
    };
    let extractor = crate::diarization::embedding::ExtractorConfig::capture(
        &crate::windows_runtime::embedding().unwrap(),
        &engine,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("声の特徴量 日本語");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut bytes = crate::transcription::wave::wav_header(64000).to_vec();
    for n in 0..64000 {
        bytes.extend_from_slice(
            &(((n as f64 * 330.0 * std::f64::consts::TAU / 16000.0).sin() * 4096.0) as i16)
                .to_le_bytes(),
        );
    }
    fs::write(root.join("input.wav"), bytes).unwrap();
    let wave = Fingerprint::capture(&root.join("input.wav"), &|| false).unwrap();
    let mut vectors = Vec::new();
    for resident in [false, true] {
        let snapshot = resident.then(|| {
            crate::local_engine::resident::prepare_installed(&root, "sherpa")
                .unwrap()
                .unwrap()
        });
        let _scope = crate::local_engine::resident::Scope::enter(&root, snapshot.as_ref()).unwrap();
        for unit in 0..2 {
            let path = root.join(format!("{resident}-{unit}"));
            fs::create_dir(&path).unwrap();
            let output = crate::diarization::embedding::run(
                &extractor,
                &engine,
                &wave,
                0,
                64000,
                &path,
                Duration::from_secs(60),
                &|| false,
            )
            .unwrap()
            .unwrap();
            let vector = output.vector(0, 64000).unwrap().unwrap();
            vectors.push(crate::voice::normalize(&vector).unwrap());
        }
    }
    assert!(vectors.iter().all(|vector| vector.len() == vectors[0].len()
        && vector
            .iter()
            .zip(&vectors[0])
            .all(|(a, b)| (a - b).abs() < 1e-5)));
    eprintln!("CPU native helper and resident extractor produced identical {}-dimensional normalized vectors",vectors[0].len());
}
#[test]
#[ignore = "requires --raw-four-hour-fixture output in MON_WINDOWS_RAW_FIXTURE"]
fn windows_four_hour_raw_prefix_sha_clock_and_bounded_groups() {
    let directory = PathBuf::from(std::env::var_os("MON_WINDOWS_RAW_FIXTURE").unwrap())
        .canonicalize()
        .unwrap();
    crate::windows_files::set_private(&directory).unwrap();
    // The independent producer fixture inherits the test user's ACL. Apply the
    // production private-file contract before opening its exact declared files.
    for entry in fs::read_dir(&directory).unwrap() {
        crate::windows_files::set_private(&entry.unwrap().path()).unwrap();
    }
    let prefix = crate::recording_segments::Prefix::read_optional(
        crate::input_levels::open_capture_directory(&directory).unwrap(),
        "6ed98cb6-1658-4e35-ab1c-6c98d82abc58",
        crate::recording_segments::Lane::Microphone,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(prefix.frames(), 691200000);
    let groups = prefix.flac_groups(9_000_000_000_000).unwrap();
    assert_eq!(groups.len(), 240);
    assert!(groups
        .iter()
        .all(|group| group.frames <= 48000 * 60 && group.clock_segments.len() <= 1024));
    prefix.verify_unchanged(&|| false).unwrap();
    eprintln!(
        "Verified 4-hour raw PCM SHA chain, exact clocks and {} bounded FLAC groups",
        groups.len()
    );
}
#[test]
fn windows_chapter_m4a_fixtures_decode_without_external_fixture_generation() {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio-import");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let helper = crate::local_engine::Fingerprint::capture(&helper(), &|| false).unwrap();
    for codec in ["aac", "alac"] {
        let original = fixtures
            .join(format!("chapters-{codec}.m4a"))
            .canonicalize()
            .unwrap();
        let before = fs::read(&original).unwrap();
        let selected = crate::audio_import::inspect(&original, &helper, &root, &|| false).unwrap();
        assert_eq!(selected.info.codec, codec);
        assert_eq!(selected.info.sample_rate, 48000);
        assert_eq!(selected.info.channels, 1);
        assert!((48000..=49152).contains(&selected.info.frames));
        let storage = Storage::open(root.join(codec)).unwrap();
        let imported = crate::audio_import::import(
            &storage,
            selected,
            &helper,
            codec,
            &Uuid::new_v4().to_string(),
            &|| false,
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(imported.state, "ready");
        assert_eq!(
            storage
                .recorded_audio_plan(&imported.meeting_id)
                .unwrap()
                .tracks
                .len(),
            1
        );
        assert_eq!(fs::read(original).unwrap(), before);
    }
}

#[test]
#[ignore = "requires independent fixture files in MON_WINDOWS_IMPORT_FIXTURES"]
fn windows_real_import_four_formats_aac_alac_and_invalid_containers() {
    let fixtures = PathBuf::from(std::env::var_os("MON_WINDOWS_IMPORT_FIXTURES").unwrap())
        .canonicalize()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let storage = Storage::open(root.join("取り込み先 日本語")).unwrap();
    let helper = crate::local_engine::Fingerprint::capture(&helper(), &|| false).unwrap();
    for name in [
        "pcm.wav",
        "float.wav",
        "audio.mp3",
        "audio.flac",
        "aac.m4a",
        "alac.m4a",
    ] {
        let original = fixtures.join(name);
        let before = crate::local_engine::Fingerprint::capture(&original, &|| false).unwrap();
        let selected = crate::audio_import::inspect(&original, &helper, &root, &|| false).unwrap();
        assert!((3_900_000..=4_100_000).contains(&selected.info.duration_us));
        let imported = crate::audio_import::import(
            &storage,
            selected,
            &helper,
            name,
            &Uuid::new_v4().to_string(),
            &|| false,
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(imported.state, "ready");
        let base = storage.audio_import_directory(&imported).unwrap();
        let manifest: crate::audio_import::ImportManifest =
            serde_json::from_slice(&fs::read(base.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.decoded_format().unwrap(), "caf");
        assert!(!base.join("audio.flac").exists());
        assert_eq!(
            before,
            crate::local_engine::Fingerprint::capture(&original, &|| false).unwrap()
        );
        let plan = storage.recorded_audio_plan(&imported.meeting_id).unwrap();
        assert_eq!(plan.tracks.len(), 1);
        assert!(storage
            .verify_asset(&plan.tracks[0].chunks[0].asset_id)
            .unwrap());
    }
    for name in ["video.m4a", "two-tracks.m4a"] {
        let original = fixtures.join(name);
        let before = fs::read(&original).unwrap();
        assert!(crate::audio_import::inspect(&original, &helper, &root, &|| false).is_err());
        assert_eq!(fs::read(original).unwrap(), before);
    }
    let original = fixtures.join("pcm.wav");
    let selected = crate::audio_import::inspect(&original, &helper, &root, &|| false).unwrap();
    let cancelled = std::cell::Cell::new(false);
    assert!(crate::audio_import::import(
        &storage,
        selected,
        &helper,
        "取消",
        &Uuid::new_v4().to_string(),
        &|| cancelled.get(),
        &|phase, _| {
            if phase == "copying" {
                cancelled.set(true);
            }
        }
    )
    .is_err());
    assert!(original.is_file());
    let archive = root.join("imports.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let restored = Storage::open(root.join("復元先")).unwrap();
    let restore = restored
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let result = restore
        .publish(
            &restored,
            &restore.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(result.meeting_ids.len() >= 6);
}
#[test]
#[ignore = "requires SHA-verified real model assets in MON_WINDOWS_PARITY_ASSETS"]
fn windows_real_worker_reuses_cpu_model_and_runs_available_vulkan() {
    let assets: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var_os("MON_WINDOWS_PARITY_ASSETS").unwrap()).unwrap(),
    )
    .unwrap();
    let tiny = &assets["be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"];
    let model = crate::local_engine::Fingerprint::capture(
        Path::new(tiny["path"].as_str().unwrap()),
        &|| false,
    )
    .unwrap();
    assert_eq!(
        model.sha256,
        "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("常駐ワーカー 日本語");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let snapshot = crate::local_engine::resident::prepare_installed(&root, "whisper")
        .unwrap()
        .unwrap();
    let config = crate::transcription::asr::AsrConfig {
        backend: None,
        executable: snapshot.resolve(&root).unwrap(),
        model,
        vad_model: None,
        vad_options: None,
        threads: 2,
        use_gpu: false,
        prompt: String::new(),
    };
    let mut pcm = Vec::new();
    for n in 0..32000 {
        pcm.extend_from_slice(
            &(((n as f64 * 440.0 * std::f64::consts::TAU / 16000.0).sin() * 2048.0) as i16)
                .to_le_bytes(),
        );
    }
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    let mut logs = Vec::new();
    {
        let _scope = crate::local_engine::resident::Scope::enter(&root, Some(&snapshot)).unwrap();
        for index in 0..2 {
            use std::os::windows::ffi::OsStrExt;
            let prefix = format!("区間 {index} ");
            // Physical directory length 251, log path length 262: reproduce
            // the Store installation's failure while keeping CWD under 260.
            let length =
                root.as_os_str().encode_wide().count() - 4 + 1 + prefix.encode_utf16().count();
            let unit = root.join(format!(
                "{prefix}{}",
                "x".repeat(251usize.saturating_sub(length))
            ));
            assert!(unit.join("stdout.log").as_os_str().encode_wide().count() > 264);
            fs::create_dir(&unit).unwrap();
            fs::write(unit.join("input.wav"), &wav).unwrap();
            crate::transcription::whisper::run_unit(
                &config,
                &unit,
                2_000_000,
                Duration::from_secs(60),
                &|| false,
            )
            .unwrap()
            .unwrap();
            // The worker stays alive, but completed unit logs must no longer
            // have writable handles when the parent seals their provenance.
            for name in ["stdout.log", "stderr.log"] {
                crate::restored_processing::read_local(&unit.join(name), 4 * 1024 * 1024, &|| {
                    false
                })
                .unwrap();
            }
            logs.push(fs::read_to_string(unit.join("stderr.log")).unwrap());
        }
    }
    let pid = |log: &str| {
        log.lines()
            .find_map(|line| {
                line.strip_prefix("mon_session: pid=")
                    .and_then(|rest| rest.split_whitespace().next())
            })
            .unwrap()
            .to_owned()
    };
    assert_eq!(pid(&logs[0]), pid(&logs[1]));
    assert!(logs[1].contains("reusing model"));
    let adapters = crate::gpu::adapters();
    eprintln!(
        "Vulkan adapters: {}",
        serde_json::to_string(&adapters).unwrap()
    );
    if adapters.is_empty() {
        eprintln!("No Vulkan hardware available; GPU inference remains unverified");
        return;
    }
    let unit = root.join("GPU 試行");
    fs::create_dir(&unit).unwrap();
    fs::write(unit.join("input.wav"), &wav).unwrap();
    let _scope = crate::local_engine::resident::Scope::enter(&root, Some(&snapshot)).unwrap();
    let mut gpu = config;
    gpu.use_gpu = true;
    crate::transcription::whisper::run_unit(
        &gpu,
        &unit,
        2_000_000,
        Duration::from_secs(60),
        &|| false,
    )
    .unwrap()
    .unwrap();
    let log = fs::read_to_string(unit.join("stderr.log")).unwrap();
    assert!(log.contains("using Vulkan0 backend"));
    assert!(adapters.iter().any(|adapter| log.contains(&adapter.name)));
}
fn synthetic_recording(scenario: &str) -> (tempfile::TempDir, Storage, String, String, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let storage = Storage::open(root.join("会議データ")).unwrap();
    let meeting = storage.create_meeting("合成入力の回復試験").unwrap();
    let config = crate::storage::RecordingConfig {
        mode: "system".into(),
        backend: "wasapi".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    let session = storage
        .prepare_recording_with_speaker_counts(
            &meeting.id,
            &config,
            &Uuid::new_v4().to_string(),
            &Default::default(),
        )
        .unwrap();
    let output = storage
        .root()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    fs::create_dir_all(&output).unwrap();
    let mut child = Command::new(helper())
        .arg("--backend")
        .arg("wasapi")
        .arg("--output")
        .arg(&output)
        .args(["--mode", "system", "--parent-pid"])
        .arg(std::process::id().to_string())
        .arg("--input-level-owner")
        .arg(&session.id)
        .args(["--synthetic", "--scenario", scenario])
        .spawn()
        .unwrap();
    storage.mark_recording_started(&session.id).unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(15) {
            child.kill().unwrap();
            panic!("synthetic capture timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(status.success());
    storage
        .request_recording_stop(&session.id, &Uuid::new_v4().to_string())
        .unwrap();
    storage.finalize_recording(&session.id).unwrap();
    (temp, storage, meeting.id, session.id, output)
}
#[test]
fn windows_parity_independent_input_recovery_and_sealed_history() {
    let (_temp, storage, meeting, session, output) = synthetic_recording("input-recovery");
    let history = crate::input_status::journal::Reader::pin(&output.canonicalize().unwrap())
        .unwrap()
        .read(&session)
        .unwrap();
    let history = serde_json::to_value(history).unwrap();
    assert_eq!(history["sealed"], true);
    for input in ["microphoneHistory", "systemHistory"] {
        assert_eq!(history[input]["lastSequence"], 2);
        assert_eq!(history[input]["events"][0]["kind"], "interrupted");
        assert_eq!(history[input]["events"][1]["kind"], "recovered");
        assert_eq!(history[input]["events"][1]["status"]["sourceEpoch"], 2);
    }
    let (plan, known) = storage.recorded_analysis_input(&meeting).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].kind, "processed");
    assert!(known.unwrap().gaps.iter().all(|g| g.track_index == 0));
    storage.verify_canonical_audio(&meeting, &|| false).unwrap();
}
#[test]
fn windows_parity_missing_microphone_keeps_pc_audio() {
    let (_temp, storage, meeting, _session, output) = synthetic_recording("system-only");
    let (plan, _) = storage.recorded_analysis_input(&meeting).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    assert_eq!(plan.tracks[0].kind, "processed");
    assert!(!output.join("microphone.caf").exists());
    assert!(!output.join("system.caf").exists());
    storage.verify_canonical_audio(&meeting, &|| false).unwrap();
}
#[test]
fn windows_model_directory_keeps_existing_bytes_and_rejects_links_and_parent_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let directory = Directory::root(&root.join("models")).unwrap();
    let (mut file, _) = directory.create("nested/model.bin").unwrap();
    use std::io::Write;
    file.write_all(b"original").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(directory.create("nested/model.bin").is_err());
    assert!(directory.create("../outside").is_err());
    assert!(directory.create("C:/outside").is_err());
    assert!(fs::rename(directory.path(), root.join("moved")).is_err());
    fs::hard_link(
        directory.path().join("nested/model.bin"),
        root.join("alias"),
    )
    .unwrap();
    assert!(crate::model_inspection::Source::open(directory.path(), "nested/model.bin").is_err());
    assert_eq!(
        fs::read(directory.path().join("nested/model.bin")).unwrap(),
        b"original"
    );
}
#[test]
fn windows_capture_stop_publishes_one_wav_and_restores_its_capture_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let store = Storage::open_with_models(root.join("data"), root.join("models")).unwrap();
    let meeting = store.create_meeting("Windows synthetic capture").unwrap();
    let config = crate::storage::RecordingConfig {
        mode: "system".into(),
        backend: "wasapi".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    let session = store
        .prepare_recording_with_speaker_counts(
            &meeting.id,
            &config,
            &Uuid::new_v4().to_string(),
            &Default::default(),
        )
        .unwrap();
    let output = store
        .root()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    fs::create_dir_all(&output).unwrap();
    let mut child = Command::new(helper())
        .arg("--backend")
        .arg("wasapi")
        .arg("--output")
        .arg(&output)
        .args(["--mode", "system", "--parent-pid"])
        .arg(std::process::id().to_string())
        .arg("--input-level-owner")
        .arg(&session.id)
        .arg("--until-stopped")
        .arg("--synthetic")
        .arg("--synthetic-clock-cases")
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if output.join("input-levels.json").is_file() && output.join("input-status.json").is_file()
        {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut levels = crate::input_levels::LevelReader::default();
    assert!(levels
        .read(&output.canonicalize().unwrap(), &session.id)
        .is_ok());
    let mut status =
        crate::input_status::StatusReader::pin(&output.canonicalize().unwrap()).unwrap();
    assert!(status.read(&session.id, Some("main")).unwrap().connected());
    store.mark_recording_started(&session.id).unwrap();
    // Exercise the live IPC polling path too, including a Windows reader that
    // briefly prevents replacement of the exact file from the reported failure.
    use std::os::windows::fs::OpenOptionsExt;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(output.join("input-status.json"))
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
    });
    let observing = Instant::now();
    while observing.elapsed() < Duration::from_millis(900) {
        levels
            .read(&output.canonicalize().unwrap(), &session.id)
            .unwrap();
        assert!(status.read(&session.id, Some("main")).unwrap().connected());
        std::thread::sleep(Duration::from_millis(25));
    }
    release.join().unwrap();
    store
        .request_recording_stop(&session.id, &Uuid::new_v4().to_string())
        .unwrap();
    fs::write(output.join("stop.request"), b"").unwrap();
    while child.try_wait().unwrap().is_none() {
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(child.wait().unwrap().success());
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("capture-clock.json")).unwrap()).unwrap();
    assert_eq!(diagnostic["state"], "saved");
    for lane in ["microphone", "system"] {
        assert_eq!(diagnostic[lane]["equalTimestampPackets"], 1);
        assert_eq!(diagnostic[lane]["reversedTimestampPackets"], 2);
        assert_eq!(diagnostic[lane]["timestampErrorPackets"], 1);
        assert!(diagnostic[lane]["savedFrames"].as_u64().unwrap() > 480 * 7);
        assert_eq!(
            diagnostic[lane]["savedFrames"].as_u64().unwrap(),
            diagnostic[lane]["observedPackets"].as_u64().unwrap() * 480
        );
    }
    store.finalize_recording(&session.id).unwrap();
    assert_eq!(
        store.get_meeting(&meeting.id).unwrap().unwrap().lifecycle,
        "recorded"
    );
    let (plan, known) = store.recorded_analysis_input(&meeting.id).unwrap();
    assert_eq!(plan.tracks.len(), 1);
    let known = known.unwrap();
    for track in 0..1 {
        assert!(known.gaps.iter().any(|gap| gap.track_index == track
            && gap.start_us == 70_000
            && gap.end_us == 90_000
            && gap.reason_code == "meeting_input_missing"
            && !gap.recovered));
    }
    for track in &plan.tracks {
        for chunk in &track.chunks {
            assert!(store.verify_asset(&chunk.asset_id).unwrap());
        }
    }
    let archive = root.join("recording-roundtrip.monbackup");
    store.create_backup_archive(&archive, &|| false).unwrap();
    let destination = Storage::open(root.join("restored-data")).unwrap();
    let restore = destination
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let receipt = restore
        .publish(
            &destination,
            &restore.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(receipt.meeting_ids.len(), 1);
    assert_eq!(
        destination
            .recorded_analysis_input(&receipt.meeting_ids[0])
            .unwrap()
            .0
            .tracks
            .len(),
        1
    );
    let sessions = destination.closed_input_recordings(None).unwrap().0;
    let restored_session = sessions
        .iter()
        .find(|row| row.meeting_id == receipt.meeting_ids[0])
        .unwrap();
    let history = destination
        .closed_input_snapshot(&restored_session.meeting_id, &restored_session.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        history.source_owner_id(),
        session.id,
        "portable history preserves the original producer owner"
    );
    store
        .ensure_meeting_mix(&meeting.id, &|| false, &|_, _| {})
        .unwrap();
    let (mix, _) = store.recorded_analysis_input(&meeting.id).unwrap();
    assert_eq!(mix.tracks.len(), 1);
    assert_eq!(mix.tracks[0].kind, "processed");
}
#[test]
fn windows_bundled_model_catalog_is_complete_and_target_bound() {
    let root = crate::windows_runtime::resources().unwrap();
    let options = crate::model_official_catalog::options_windows(&root).unwrap();
    assert_eq!(options.len(), 20);
    if let Some(path) = std::env::var_os("MON_WINDOWS_MODEL_OPTIONS") {
        // Feed the exact IPC shape and Rust digests to the production TS reader.
        fs::write(path, serde_json::to_vec(&options).unwrap()).unwrap();
    }
    let bytes = crate::windows_runtime::catalog(&root).unwrap();
    let choices: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(choices.len(), 20);
    assert!(choices
        .iter()
        .any(|choice| choice["recipe"]["manifest"]["backend"]["id"]
            == crate::transcription::asr::NEMO_TDT));
    let mut engines = std::collections::BTreeSet::new();
    for choice in choices {
        let recipe =
            Recipe::parse(serde_json::to_vec(&choice["recipe"]).unwrap().as_slice()).unwrap();
        recipe.review().unwrap();
        crate::windows_runtime::validate_recipe(&root, &recipe).unwrap();
        assert_eq!(
            recipe.manifest.target,
            crate::model_inspection::Target::WindowsX86_64
        );
        engines.insert(
            choice["recipe"]["manifest"]["engine"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(engines.len(), 4);
}

#[test]
#[ignore = "downloads fixed real model weights into an explicitly provided isolated QA directory"]
fn windows_real_models_download_place_check_and_apply() {
    let _installed = crate::local_engine::resident::InstalledAdmission::enter();
    #[derive(serde::Deserialize)]
    struct Choice {
        id: String,
        recipe: Recipe,
    }
    let root = PathBuf::from(
        std::env::var_os("MON_WINDOWS_INTEGRATION_ROOT").expect("isolated QA directory required"),
    );
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let store = Storage::open_with_models(root.join("data"), root.join("models")).unwrap();
    let bundle = crate::windows_runtime::resources().unwrap();
    let choices: Vec<Choice> =
        serde_json::from_slice(&crate::windows_runtime::catalog(&bundle).unwrap()).unwrap();
    let state_path = root.join("model-state.json");
    let mut state: serde_json::Value = fs::read(&state_path)
        .ok()
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap_or(serde_json::json!({}));
    let offline = std::env::var_os("MON_WINDOWS_PARITY_ASSETS")
        .map(|path| serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap());
    let ids = if offline.is_some() {
        choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
    } else {
        vec![
            "windows-whisper-194-tiny",
            "windows-whisper-194-base",
            "windows-sherpa-1138-pyannote3-eres2net-base",
            "windows-llama-b11533-qwen25-15b-q4km",
        ]
    };
    for id in ids {
        let choice = choices.iter().find(|c| c.id == id).unwrap();
        let hash = choice.recipe.hash().unwrap();
        let download = if state[id]["recipeHash"] == hash && state[id]["download"].is_string() {
            state[id]["download"].as_str().unwrap().to_owned()
        } else {
            let operation = Uuid::new_v4().to_string();
            let mut latest = Instant::now();
            eprintln!("Downloading {id}");
            if let Some(index) = &offline {
                crate::model_download::windows_offline_parity_stage(
                    &root.join("downloads"),
                    &bundle,
                    &operation,
                    &choice.recipe,
                    &hash,
                    index,
                )
                .unwrap();
            } else {
                crate::model_download::download_platform(
                    &root.join("downloads"),
                    &bundle,
                    None,
                    &operation,
                    &choice.recipe,
                    &hash,
                    &|| false,
                    &mut |done, total| {
                        if latest.elapsed().as_secs() >= 5 || done == total {
                            eprintln!("{id}: {done}/{total}");
                            latest = Instant::now();
                        }
                    },
                )
                .unwrap();
            }
            state[id] = serde_json::json!({"recipeHash":hash,"download":operation});
            fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
            operation
        };
        let readback =
            crate::model_download::readback(&root.join("downloads"), &download, &hash, &|| false)
                .unwrap();
        let review = readback
            .receipt
            .as_ref()
            .expect("completed model download")
            .report
            .review_hash
            .clone();
        let package = state[id]["package"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        crate::model_download::place(
            &store,
            &root.join("models"),
            &root.join("downloads"),
            &download,
            &hash,
            &package,
            &review,
            &|| false,
        )
        .unwrap();
        state[id]["package"] = serde_json::json!(package);
        fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        let check = state[id]["check"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        eprintln!("Compatibility check {id}");
        let summary = crate::model_compatibility::check(
            &store,
            &root.join("models"),
            &check,
            &package,
            &review,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            summary.state,
            crate::model_compatibility::CheckState::Succeeded,
            "{:?}",
            summary
        );
        state[id]["check"] = serde_json::json!(check);
        fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        let preview = crate::model_application::preview_with_sensitivity(
            &store,
            &root.join("models"),
            crate::storage::model_profiles::ProfileName::Standard,
            &package,
            &review,
            &check,
            None,
            None,
            None,
            None,
            &|| false,
        )
        .unwrap();
        crate::model_application::apply(
            &store,
            &root.join("models"),
            &Uuid::new_v4().to_string(),
            &preview,
            &|| false,
        )
        .unwrap();
        eprintln!("Verified and applied {id}");
    }
    crate::analysis_flow::prepare(&store, "standard", &|| false).unwrap();
}

#[test]
#[ignore = "requires isolated real-model QA and Japanese synthetic speech, never a microphone"]
fn windows_saved_speech_runs_transcription_diarization_and_minutes() {
    let _installed = crate::local_engine::resident::InstalledAdmission::enter();
    let root = PathBuf::from(
        std::env::var_os("MON_WINDOWS_INTEGRATION_ROOT").expect("isolated QA directory required"),
    )
    .canonicalize()
    .unwrap();
    let store = Storage::open_with_models(root.join("data"), root.join("models")).unwrap();
    let sample = PathBuf::from(
        std::env::var_os("MON_WINDOWS_QA_SAMPLE").expect("Japanese PCM16 WAV fixture required"),
    );
    // The speech fixture may contain an INFO/LIST chunk. Decode this fixed test
    // fixture without relaxing the application's normalized-WAV contract.
    let bytes = fs::read(sample).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut position = 12;
    let mut samples = None;
    while position + 8 <= bytes.len() {
        let size =
            u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap()) as usize;
        let start = position + 8;
        let end = start.checked_add(size).unwrap();
        assert!(end <= bytes.len());
        if &bytes[position..position + 4] == b"data" {
            samples = Some(&bytes[start..end]);
            break;
        }
        position = end + (size % 2);
    }
    assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1);
    let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let channels = u32::from(u16::from_le_bytes(bytes[22..24].try_into().unwrap()));
    assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);
    let samples = samples.unwrap();
    let frames = samples.len() as u64 / (2 * u64::from(channels));
    let pcm: Vec<u8> = samples
        .chunks_exact(2)
        .flat_map(|s| {
            (f32::from(i16::from_le_bytes(s.try_into().unwrap())) / 32768.0).to_le_bytes()
        })
        .collect();
    let meeting = store
        .create_meeting("Windows Japanese synthetic-speech end-to-end QA")
        .unwrap();
    let config = crate::storage::RecordingConfig {
        mode: "system".into(),
        backend: "wasapi".into(),
        target_pids: vec![],
        microphone_device_uid: None,
        target_application_identity: None,
    };
    let session = store
        .prepare_recording_with_speaker_counts(
            &meeting.id,
            &config,
            &Uuid::new_v4().to_string(),
            &Default::default(),
        )
        .unwrap();
    let output = store
        .root()
        .join("meetings")
        .join(&meeting.id)
        .join("audio")
        .join(&session.id);
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("microphone.caf"),
        crate::audio::tests::caf_fixture(rate, channels, 3, 32, 4, &pcm),
    )
    .unwrap();
    fs::write(
        output.join("system.caf"),
        crate::audio::tests::caf_fixture(rate, channels, 3, 32, 4, &vec![0u8; pcm.len()]),
    )
    .unwrap();
    let duration = (frames * 1_000_000 / u64::from(rate)) as i64;
    let track = serde_json::json!({"frames":frames,"sampleRate":rate,"channels":channels,"appendFailures":0,"callbackCount":1,"clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":frames,"timelineStartUs":0,"timelineEndUs":duration,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}]});
    let manifest = serde_json::json!({"schemaVersion":4,"captureMode":"system","backend":"wasapi","targetPIDs":[],"microphoneDeviceUID":null,"targetApplicationIdentity":null,"recordingOwnerId":session.id,"inputDiagnosticsState":"unavailable","state":"saved","errors":[],"microphone":track,"system":track,"captureClock":{"domain":"query_performance_counter","originUtcMs":1700000000000i64,"uncertaintyNs":0},"captureEnd":{"method":"acquisition_closed","microphoneClosedUs":duration,"systemClosedUs":duration,"endUs":duration}});
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    store.mark_recording_started(&session.id).unwrap();
    store
        .request_recording_stop(&session.id, &Uuid::new_v4().to_string())
        .unwrap();
    store.finalize_recording(&session.id).unwrap();
    crate::analysis_flow::advance(&store, &|| false, &|f| {
        eprintln!("Flow: {} {:?}", f.stage, f.error_message)
    })
    .unwrap();
    let job = store.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(job.kind, "transcription");
    eprintln!("Transcribing Japanese synthetic speech");
    crate::transcription::process(&store, &job.id, &|| false, &|j| {
        eprintln!("ASR: {} {}", j.state, j.completed_units)
    })
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&store, &|| false, &|f| {
        eprintln!("Flow: {} {:?}", f.stage, f.error_message)
    })
    .unwrap();
    let job = store.acquire_local_inference_job().unwrap().unwrap();
    assert_eq!(job.kind, "diarization");
    crate::diarization::process(&store, &job.id, &|| false, &|j| {
        eprintln!("Diarization: {}", j.state)
    })
    .unwrap()
    .unwrap();
    crate::analysis_flow::advance(&store, &|| false, &|f| {
        eprintln!("Flow: {} {:?}", f.stage, f.error_message)
    })
    .unwrap();
    let current = store.load_current_analysis(&meeting.id).unwrap().unwrap();
    assert!(!current.utterances.is_empty());
    let preferences = store.minutes_preferences().unwrap().payload.standard;
    let engine = crate::generation::llama::LlamaConfig {
        executable: crate::local_engine::Fingerprint::capture(
            Path::new(&preferences.executable_path),
            &|| false,
        )
        .unwrap(),
        model: crate::local_engine::Fingerprint::capture(
            Path::new(&preferences.model_path),
            &|| false,
        )
        .unwrap(),
        threads: preferences.threads,
        context_tokens: preferences.context_tokens,
        prediction_tokens: preferences.prediction_tokens,
        gpu_layers: 0,
        seed: preferences.seed,
    };
    let request = crate::storage::MinutesJobRequest {
        meeting_id: meeting.id.clone(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: current.id.clone(),
        expected_content_hash: current.content_hash.clone(),
        expected_meeting_lock_version: store
            .get_meeting(&meeting.id)
            .unwrap()
            .unwrap()
            .lock_version,
        expected_draft: None,
    };
    let job = crate::generation::markdown_job::enqueue(
        &store,
        request,
        engine,
        "standard",
        preferences.prompt_budget,
        None,
    )
    .unwrap();
    let claimed = store
        .acquire_job_kind(Some(crate::storage::JobKind::MinutesGeneration))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, job.id);
    crate::generation::markdown_job::process(&store, &job.id, &|| false, &|j| {
        eprintln!("Minutes: {} {}", j.state, j.completed_units)
    })
    .unwrap()
    .unwrap();
    assert!(!store
        .get_minutes_markdown(&meeting.id)
        .unwrap()
        .unwrap()
        .markdown
        .is_empty());
    eprintln!(
        "Windows full analysis succeeded for isolated meeting {}",
        meeting.id
    );
}
