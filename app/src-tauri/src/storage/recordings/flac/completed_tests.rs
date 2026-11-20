use super::*;

// Identical compact PCM and measured clocks feed both the previous continuous
// source path and the new sealed-prefix path; only physical grouping differs.
fn compact_fixture(gap: bool) -> Fixture {
    let fixture = Fixture::new(2, 2000, gap);
    let manifest_path = fixture.base.join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    for (lane, seed) in [("microphone", 17_u64), ("system", 31_u64)] {
        let mut previous = None::<String>;
        let mut total = 0_u64;
        let mut whole_pcm = Vec::new();
        for sequence in 1..=2 {
            let path = fixture.base.join(format!("raw-{lane}-{sequence:08}.json"));
            let mut entry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let frames = entry["frames"].as_u64().unwrap() * 2;
            let pcm: Vec<_> = (0..frames)
                .flat_map(|frame| {
                    let value = (((total + frame + seed) * 7919) % 60001) as i32 - 30000;
                    (value as i16).to_le_bytes()
                })
                .collect();
            let mut audio = mon_core::media::caf::voice_header(frames).unwrap().to_vec();
            audio.extend(&pcm);
            write(
                &fixture.base.join(format!("raw-{lane}-{sequence:08}.caf")),
                &audio,
            );
            entry["schemaVersion"] = json!(3);
            entry["sampleEncoding"] = json!("pcm16");
            entry["sourceFrameStart"] = json!(total);
            entry["frames"] = json!(frames);
            entry["sampleRate"] = json!(16000);
            entry["channels"] = json!(1);
            entry["audioBytes"] = json!(audio.len());
            entry["audioSHA256"] = json!(hash(&audio));
            entry["previousEntrySHA256"] = json!(previous);
            for map in entry["clockSegments"].as_array_mut().unwrap() {
                for key in ["sourceFrameStart", "sourceFrameEnd"] {
                    map[key] = json!(map[key].as_u64().unwrap() * 2);
                }
            }
            let bytes = serde_json::to_vec(&entry).unwrap();
            previous = Some(hash(&bytes));
            write(&path, &bytes);
            whole_pcm.extend(pcm);
            total += frames;
        }
        let tip = json!({"schemaVersion":2,"ownerId":fixture.session.id,"lane":lane,
            "sealedSegments":2,"sealedFrames":total,"lastEntrySHA256":previous,"producerClosed":true});
        write(
            &fixture.base.join(format!("raw-{lane}.json")),
            &serde_json::to_vec(&tip).unwrap(),
        );
        let track = &mut manifest[lane];
        track["frames"] = json!(total);
        track["sampleRate"] = json!(16000);
        track["channels"] = json!(1);
        track["rawSegments"] = tip;
        track["rawSegments"]["failure"] = Value::Null;
        for map in track["clockSegments"].as_array_mut().unwrap() {
            for key in ["sourceFrameStart", "sourceFrameEnd"] {
                map[key] = json!(map[key].as_u64().unwrap() * 2);
            }
        }
        let mut audio = mon_core::media::caf::voice_header(total).unwrap().to_vec();
        audio.extend(whole_pcm);
        write(&fixture.base.join(format!("{lane}.caf")), &audio);
    }
    write(&manifest_path, &serde_json::to_vec(&manifest).unwrap());
    fixture
}

fn wave(fixture: &Fixture) -> Vec<u8> {
    let plan = fixture
        .storage
        .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
        .unwrap();
    let path = fixture
        .storage
        .meeting_mix_path(&fixture.session.meeting_id, &plan.tracks[0])
        .unwrap();
    std::fs::read(path).unwrap()
}

fn pending_wave(fixture: &Fixture) -> PathBuf {
    let directories: Vec<_> =
        std::fs::read_dir(fixture.base.parent().unwrap().join("meeting-wave"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.join("legacy-canonical-wav.json").is_file())
            .collect();
    assert_eq!(directories.len(), 1);
    let directory = &directories[0];
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(directory.join("wave-publication.json")).unwrap())
            .unwrap();
    directory.join(receipt["name"].as_str().unwrap())
}

fn assert_no_flac(fixture: &Fixture) {
    fn visit(directory: &Path) {
        for item in std::fs::read_dir(directory).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                visit(&path);
            } else {
                assert_ne!(
                    path.extension().and_then(|s| s.to_str()),
                    Some("flac"),
                    "{}",
                    path.display()
                );
            }
        }
    }
    visit(fixture.base.parent().unwrap());
    assert!(!fixture.base.join(COMPLETED_PLAN).exists());
    assert!(!fixture.base.join(PARTIAL_PLAN).exists());
}

// Explicitly reproduce an older producer workspace. Current finalization must
// never be used to mint FLAC merely to exercise its retained relocation reader.
fn legacy_completed_publication(storage: &Storage, session: &str) -> Publication {
    let session = load_session(&storage.connection.lock().unwrap(), session)
        .unwrap()
        .unwrap();
    let base = Path::new("meetings")
        .join(&session.meeting_id)
        .join("audio")
        .join(&session.id);
    let proof = storage
        .checked_capture_file(&base.join("manifest.json"), None)
        .unwrap();
    let manifest = storage.read_capture_manifest(&proof).unwrap();
    let prefixes = storage
        .verify_completed_raw_prefixes(&base, &manifest, &session.id)
        .unwrap();
    let origin = prefixes
        .iter()
        .filter(|prefix| prefix.frames() > 0)
        .map(Prefix::first_capture_nanoseconds)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .min()
        .unwrap();
    Publication::prepare_completed(storage, &base, &proof.hash, origin, &prefixes, &|| false)
        .unwrap()
}

#[test]
fn completed_canonical_maps_accept_the_full_raw_recording_capacity() {
    let prefix = crate::recording_segments::maximum_prefix();
    let expected: Vec<_> = prefix
        .entries()
        .flat_map(|entry| {
            entry
                .clock_segments
                .iter()
                .map(|part| {
                    let mut part = part.clone();
                    part.source_frame_start += entry.source_frame_start as i64;
                    part.source_frame_end += entry.source_frame_start as i64;
                    part.timeline_start_us += entry.capture_origin_nanoseconds / 1000;
                    part.timeline_end_us += entry.capture_origin_nanoseconds / 1000;
                    part
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let owner = "735027d4-a03e-43d3-830a-21e767dfaf88";
    let manifest: CaptureManifest = serde_json::from_value(json!({
        "schemaVersion":4,"captureMode":"system","backend":"wasapi","targetPIDs":[],
        "state":"saved","errors":[],"recordingOwnerId":owner,"clockMappingSource":"sealed_raw_v1",
        "microphone":{"frames":prefix.frames(),"sampleRate":16000,"channels":1,"appendFailures":0,
            "clockSegments":expected,"rawSegments":{"schemaVersion":2,"ownerId":owner,"lane":"microphone",
                "sealedSegments":prefix.entries().len(),"sealedFrames":prefix.frames(),"lastEntrySHA256":"a".repeat(64),"producerClosed":true,"failure":null}},
        "system":{"frames":0,"appendFailures":0,"clockSegments":[]},"systemAudioUnreceived":true
    })).unwrap();
    let (actual, system) = canonical_maps(&manifest, &[prefix]).unwrap();
    assert_eq!(actual, expected);
    assert!(system.is_empty());
}

#[test]
fn completed_prefix_without_continuous_caf_matches_previous_wave_and_keeps_capture_gaps() {
    for gap in [false, true] {
        let previous = compact_fixture(gap);
        let path = previous.base.join("manifest.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for lane in ["microphone", "system"] {
            manifest[lane]
                .as_object_mut()
                .unwrap()
                .remove("rawSegments");
        }
        write(&path, &serde_json::to_vec(&manifest).unwrap());
        previous
            .storage
            .finalize_recording(&previous.session.id)
            .unwrap();

        let current = compact_fixture(gap);
        for lane in ["microphone", "system"] {
            std::fs::remove_file(current.base.join(format!("{lane}.caf"))).unwrap();
        }
        let result = current
            .storage
            .finalize_recording(&current.session.id)
            .unwrap();
        assert_eq!(result.state, "saved");
        assert_eq!(wave(&current), wave(&previous));
        assert_no_flac(&current);
        assert!(!current.base.join(PARTIAL_PLAN).exists());
        assert!(!current.base.join("microphone.caf").exists());
        assert!(!current.base.join("system.caf").exists());
        let gaps: Vec<(i64, i64, String)> = current
            .storage
            .connection
            .lock()
            .unwrap()
            .prepare("SELECT start_us,end_us,reason_code FROM audio_gaps ORDER BY start_us,end_us")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut expected = Vec::new();
        if gap {
            expected.push((4_050_000, 4_700_000, "meeting_input_missing".into()));
        }
        let last_input = if gap { 5_000_000 } else { 4_300_000 };
        expected.push((
            last_input,
            last_input + 600_000,
            "meeting_input_missing".into(),
        ));
        assert_eq!(gaps, expected);
    }
}

#[test]
fn completed_wav_tampering_is_rejected_and_verified_raw_is_retained_for_retry() {
    for duplicate in [false, true] {
        let fixture = compact_fixture(true);
        let original = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
        fixture.fail_db();
        assert!(fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .is_err());
        fixture.assert_no_registration();
        let path = pending_wave(&fixture);
        let verified = std::fs::read(&path).unwrap();
        let mut corrupt = verified.clone();
        if duplicate {
            corrupt.extend_from_slice(&verified[44..]);
        } else {
            corrupt.truncate(corrupt.len() / 2);
        }
        write(&path, &corrupt);
        let unknown = path.parent().unwrap().join("keep-unknown.part");
        write(&unknown, b"not owned by a checkpoint");
        fixture.allow_db();
        assert!(fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .is_err());
        fixture.assert_no_registration();
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(
            std::fs::read(&unknown).unwrap(),
            b"not owned by a checkpoint"
        );
        assert_eq!(
            std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap(),
            original
        );
        assert_no_flac(&fixture);
        // The test restores the same owned inode from its known verified bytes.
        // Publication itself must never overwrite the tampered completed WAV.
        write(&path, &verified);
        let saved = fixture
            .storage
            .finalize_recording(&fixture.session.id)
            .unwrap();
        assert_eq!(saved.state, "saved");
        assert_eq!(wave(&fixture), verified);
        assert_eq!(
            std::fs::read(&unknown).unwrap(),
            b"not owned by a checkpoint"
        );
        assert_eq!(
            fixture
                .storage
                .finalize_recording(&fixture.session.id)
                .unwrap(),
            saved
        );
    }
}

#[test]
fn completed_prefix_failed_db_commit_reuses_the_owned_wav_without_rewriting_pcm() {
    let fixture = compact_fixture(true);
    for lane in ["microphone", "system"] {
        std::fs::remove_file(fixture.base.join(format!("{lane}.caf"))).unwrap();
    }
    let original = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
    fixture.fail_db();
    assert!(fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .is_err());
    fixture.assert_no_registration();
    assert_eq!(
        std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap(),
        original
    );
    let path = pending_wave(&fixture);
    let stamp = Stamp::read(&File::open(&path).unwrap()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let receipt = std::fs::read(path.parent().unwrap().join("wave-publication.json")).unwrap();
    assert_no_flac(&fixture);
    fixture.allow_db();
    drop(fixture.storage);
    let reopened = Storage::open(fixture.temp.path().to_owned()).unwrap();
    assert_eq!(
        reopened.recover_completed_recordings().unwrap(),
        vec![fixture.session.id.clone()]
    );
    stamp.require(&File::open(&path).unwrap()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::read(path.parent().unwrap().join("wave-publication.json")).unwrap(),
        receipt
    );
    let canonical = reopened
        .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
        .unwrap();
    assert_eq!(canonical.tracks.len(), 1);
    assert_eq!(
        reopened
            .meeting_mix_path(&fixture.session.meeting_id, &canonical.tracks[0])
            .unwrap(),
        path
    );
    assert!(!fixture.base.join(COMPLETED_PLAN).exists());
    assert!(!fixture.base.join(PARTIAL_PLAN).exists());
    assert!(!fixture.base.join("raw-microphone-00000001.caf").exists());
}

#[test]
fn completed_prefix_keeps_unreceived_microphone_empty_without_continuous_files() {
    let fixture = compact_fixture(false);
    for lane in ["microphone", "system"] {
        std::fs::remove_file(fixture.base.join(format!("{lane}.caf"))).unwrap();
    }
    let tip = json!({"schemaVersion":2,"ownerId":fixture.session.id,"lane":"microphone",
        "sealedSegments":0,"sealedFrames":0,"lastEntrySHA256":null,"producerClosed":true});
    write(
        &fixture.base.join("raw-microphone.json"),
        &serde_json::to_vec(&tip).unwrap(),
    );
    let path = fixture.base.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["microphoneAudioUnreceived"] = json!(true);
    manifest["microphone"] = json!({"frames":0,"callbackCount":0,"appendFailures":0,"clockSegments":[],"rawSegments":tip});
    manifest["microphone"]["rawSegments"]["failure"] = Value::Null;
    manifest["captureClock"]["originUtcMs"] = json!(1700000000050_i64);
    for map in manifest["system"]["clockSegments"].as_array_mut().unwrap() {
        for key in ["timelineStartUs", "timelineEndUs"] {
            map[key] = json!(map[key].as_i64().unwrap() - 50000);
        }
    }
    for key in ["microphoneClosedUs", "systemClosedUs", "endUs"] {
        manifest["captureEnd"][key] = json!(manifest["captureEnd"][key].as_i64().unwrap() - 50000);
    }
    write(&path, &serde_json::to_vec(&manifest).unwrap());
    fixture
        .storage
        .finalize_recording(&fixture.session.id)
        .unwrap();
    assert_no_flac(&fixture);
    let canonical = fixture
        .storage
        .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
        .unwrap();
    assert_eq!(canonical.tracks.len(), 1);
    assert_eq!(canonical.tracks[0].kind, "processed");
    let publication: Value = serde_json::from_slice(
        &std::fs::read(
            pending_wave(&fixture)
                .parent()
                .unwrap()
                .join("legacy-canonical-wav.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(publication["microphone"]["frames"], 0);
    assert_eq!(publication["microphone"]["clockSegments"], json!([]));
    assert_eq!(
        publication["system"]["clockSegments"][0]["timelineStartUs"],
        0
    );
    let bytes = wave(&fixture);
    let expected_first = (((31_u64 * 7919) % 60001) as i32 - 30000) as f32 / 32768.0;
    let expected_first = (expected_first * 32767.0).round() as i16;
    assert_eq!(
        i16::from_le_bytes(bytes[44..46].try_into().unwrap()),
        expected_first
    );
    let db = fixture.storage.connection.lock().unwrap();
    let mic_tracks: i64 = db
        .query_row(
            "SELECT count(*) FROM audio_tracks WHERE kind='mic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mic_tracks, 0);
    assert!(!fixture.base.join("microphone.caf").exists());
    assert!(!fixture.base.join("system.caf").exists());
}

#[cfg(any(target_os = "macos", windows))]
fn relocate_pending(storage: Storage, plan_name: &str) -> (tempfile::TempDir, Storage) {
    use crate::bootstrap::Bootstrap;
    let source = storage.root().to_owned();
    let control = tempfile::tempdir().unwrap();
    let control_root = control.path().canonicalize().unwrap();
    let config = control_root.join("config");
    let target = control_root.join("target");
    crate::bootstrap::set_test_root(&config, &config.join("fixture-app-data"), &source);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    bootstrap.initialize(&storage).unwrap();
    crate::native_fs::private_directories(&target, false).unwrap();
    let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
    let revision = bootstrap.view().unwrap().revision;
    let operation = Uuid::new_v4().to_string();
    let confirmation = Uuid::new_v4().to_string();
    bootstrap
        .begin_move(
            &operation,
            &confirmation,
            &target,
            snapshot.manifest_hash(),
            revision,
        )
        .unwrap();
    let mut placement = bootstrap.pending_placement(&operation, &snapshot).unwrap();
    placement.copy(&snapshot, &|| false).unwrap();
    let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
    let document: Value = serde_json::from_slice(handoff.bytes()).unwrap();
    assert!(document["flac_history"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|workspace| workspace["name"] == plan_name));
    placement.install_history(&handoff, &|| false).unwrap();
    placement
        .activate_for_restart(&handoff, &confirmation, revision + 1, &|| false)
        .unwrap();
    drop(placement);
    drop(handoff);
    drop(snapshot);
    drop(bootstrap);
    drop(storage);
    let bootstrap = Bootstrap::open(&config, &source).unwrap();
    let reopened = bootstrap.open_storage().unwrap();
    assert_eq!(reopened.root(), target);
    (control, reopened)
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn completed_prefix_native_relocation_resumes_same_plan_and_chunk_ids() {
    let fixture = compact_fixture(false);
    for lane in ["microphone", "system"] {
        std::fs::remove_file(fixture.base.join(format!("{lane}.caf"))).unwrap();
    }
    drop(legacy_completed_publication(
        &fixture.storage,
        &fixture.session.id,
    ));
    fixture
        .storage
        .interrupt_recording(&fixture.session.id)
        .unwrap();
    let source = fixture.storage.root().to_owned();
    let relative = fixture.base.strip_prefix(&source).unwrap().to_owned();
    let plan_bytes = std::fs::read(fixture.base.join(COMPLETED_PLAN)).unwrap();
    let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
    let (_control, reopened) = relocate_pending(fixture.storage, COMPLETED_PLAN);
    let target = reopened.root().to_owned();
    let resumed = legacy_completed_publication(&reopened, &fixture.session.id);
    resumed.verify(&reopened).unwrap();
    drop(resumed);
    let rebound: Plan = serde_json::from_slice(
        &std::fs::read(crate::native_fs::join(&target, &relative).join(COMPLETED_PLAN)).unwrap(),
    )
    .unwrap();
    assert_eq!(rebound, plan);
    assert_eq!(
        std::fs::read(crate::native_fs::join(&target, &relative).join(COMPLETED_PLAN)).unwrap(),
        plan_bytes
    );
    // The current reader then rescues the verified old PCM directly to WAV.
    assert_eq!(
        reopened.recover_completed_recordings().unwrap(),
        vec![fixture.session.id.clone()]
    );
    let canonical = reopened
        .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
        .unwrap();
    assert_eq!(canonical.tracks.len(), 1);
    assert_eq!(canonical.tracks[0].kind, "processed");
    assert_eq!(
        std::fs::read(crate::native_fs::join(&target, &relative).join(COMPLETED_PLAN)).unwrap(),
        plan_bytes
    );
}

#[test]
#[ignore = "Owned synthetic crash child; only its dedicated parent may launch it"]
fn completed_publication_crash_child() {
    assert_eq!(
        std::env::var("MON_FINAL_FLAC_CRASH_CHILD").unwrap(),
        "synthetic-only"
    );
    let root = PathBuf::from(std::env::var_os("MON_FINAL_FLAC_CRASH_ROOT").unwrap());
    let owner = std::env::var("MON_FINAL_FLAC_CRASH_OWNER").unwrap();
    let storage = Storage::open(root).unwrap();
    drop(legacy_completed_publication(&storage, &owner));
    crash_point("completed-before-db-commit");
    panic!("owned legacy completed-publication child failed to reach its crash point");
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn partial_prefix_native_relocation_preserves_legacy_plan_before_direct_wav_rescue() {
    let fixture = partial_fixture();
    let review = fixture
        .storage
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    drop(
        review
            .legacy_publication_for_test(&fixture.storage)
            .unwrap(),
    );
    drop(review);
    let source = fixture.storage.root().to_owned();
    let relative = fixture.base.strip_prefix(&source).unwrap().to_owned();
    let plan_bytes = std::fs::read(fixture.base.join(PARTIAL_PLAN)).unwrap();
    let plan: Plan = serde_json::from_slice(&plan_bytes).unwrap();
    let (_control, reopened) = relocate_pending(fixture.storage, PARTIAL_PLAN);
    let review = reopened
        .review_partial_recording(&fixture.session.id)
        .unwrap();
    assert!(reopened.recover_partial_recording(&review, false).is_err());
    let publication = review.legacy_publication_for_test(&reopened).unwrap();
    publication.verify(&reopened).unwrap();
    drop(publication);
    let relocated_plan = crate::native_fs::join(reopened.root(), &relative).join(PARTIAL_PLAN);
    let resumed: Plan = serde_json::from_slice(&std::fs::read(&relocated_plan).unwrap()).unwrap();
    assert_eq!(resumed, plan);
    assert_eq!(std::fs::read(&relocated_plan).unwrap(), plan_bytes);
    let result = reopened.recover_partial_recording(&review, true).unwrap();
    assert_eq!(result.state, "interrupted");
    assert_eq!(result.ended_at_utc_ms, None);
    let canonical = reopened
        .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
        .unwrap();
    assert_eq!(canonical.tracks.len(), 1);
    assert_eq!(canonical.tracks[0].kind, "processed");
    assert_eq!(std::fs::read(&relocated_plan).unwrap(), plan_bytes);
}

#[test]
#[ignore = "Dedicated owned-child crash/restart verification; synthetic PCM only, no capture devices"]
fn completed_publication_owned_crashes_resume_without_continuous_caf() {
    for phase in [
        "reserved",
        "created-before-identity",
        "allocated",
        "encoded",
        "published",
        "completed-before-db-commit",
    ] {
        let fixture = compact_fixture(true);
        for lane in ["microphone", "system"] {
            std::fs::remove_file(fixture.base.join(format!("{lane}.caf"))).unwrap();
        }
        let original = std::fs::read(fixture.base.join("raw-microphone-00000001.caf")).unwrap();
        let Fixture {
            temp,
            storage,
            session,
            base,
        } = fixture;
        drop(storage);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::recordings::flac::tests::completed::completed_publication_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_FINAL_FLAC_CRASH_CHILD", "synthetic-only")
            .env("MON_FINAL_FLAC_CRASH_PHASE", phase)
            .env("MON_FINAL_FLAC_CRASH_ROOT", temp.path())
            .env("MON_FINAL_FLAC_CRASH_OWNER", &session.id)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{phase}: child unexpectedly completed"
        );
        assert_eq!(
            std::fs::read(base.join("raw-microphone-00000001.caf")).unwrap(),
            original
        );
        let plan_bytes = std::fs::read(base.join(COMPLETED_PLAN)).unwrap();
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(
            reopened.recover_completed_recordings().unwrap(),
            vec![session.id.clone()],
            "{phase}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(base.join(COMPLETED_PLAN)).unwrap(),
            plan_bytes
        );
        assert!(!base.join("microphone.caf").exists());
        assert!(!base.join("system.caf").exists());
    }
}
