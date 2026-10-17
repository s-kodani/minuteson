use super::*;
pub(super) fn request() -> Request {
    Request {
        run_id: uuid::Uuid::new_v4().to_string(),
        microphone_uid: "USB input/名前 with spaces".into(),
        capture_scope: CaptureScope {
            mode: "system".into(),
            application: None,
        },
        expected_import: ImportExpectation {
            token: None,
            revision: 0,
            busy: false,
        },
    }
}
fn controller() -> (CaptureTests, tempfile::TempDir) {
    let fixture = tempfile::tempdir().unwrap();
    let controller = CaptureTests::default();
    {
        let mut state = controller.lock().unwrap();
        state.ownership.configure(fixture.path()).unwrap();
        assert_eq!(state.ownership.reconcile().phase, "clear");
    }
    (controller, fixture)
}
#[cfg(target_os = "macos")]
fn cleanup_after_unrelated_forks(directory: &files::TestDirectory) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match directory.cleanup() {
            Err(error)
                if error.ends_with("Resource temporarily unavailable (os error 35)")
                    && Instant::now() < deadline =>
            {
                // Other parallel tests can fork before their CLOEXEC copies
                // close. Each attempt still requires the real exclusive lease;
                // never unlock the shared description or ignore other errors.
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}
#[test]
#[cfg(target_os = "macos")]
fn startup_checking_and_attention_reject_new_capture_but_attention_can_exit_without_deleting() {
    let controller = CaptureTests::default();
    assert!(controller.reserve(request()).is_err());
    assert!(controller.require_empty().is_err());
    assert!(controller.require_exit_ready().is_err());
    assert!(controller.seal_exit().is_err());
    controller
        .lock()
        .unwrap()
        .ownership
        .attention("owned audio retained".into());
    assert_eq!(controller.recovery_snapshot().unwrap().phase, "attention");
    assert!(controller.reserve(request()).is_err());
    assert!(controller.require_empty().is_err());
    controller.require_exit_ready().unwrap();
    controller.seal_exit().unwrap();
    assert!(controller.reserve(request()).is_err());
    controller.abort_exit().unwrap();
    assert!(controller.recovery_blocked());
}
#[test]
#[cfg(target_os = "macos")]
fn production_owned_directory_passes_only_its_explicit_lease_to_the_helper_command() {
    let (controller, _fixture) = controller();
    let request = request();
    controller.reserve(request.clone()).unwrap();
    let directory = controller
        .lock()
        .unwrap()
        .ownership
        .allocate(request.clone())
        .unwrap();
    let mut launch =
        owned_command(std::path::Path::new("/usr/bin/true"), &directory, &request).unwrap();
    let args: Vec<_> = launch
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args.iter()
            .filter(|a| a.as_str() == "--capture-test-lease-fd")
            .count(),
        1
    );
    assert!(args.last().unwrap().parse::<i32>().unwrap() >= 3);
    let mut helper = launch.spawn().unwrap();
    drop(launch);
    assert!(helper.wait().unwrap().success());
    cleanup_after_unrelated_forks(&directory).unwrap();
}
#[test]
#[cfg(target_os = "macos")]
fn unrelated_fork_before_exec_keeps_cleanup_fenced_until_its_cloexec_copy_closes() {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::{net::UnixStream, process::CommandExt};

    let (controller, _fixture) = controller();
    let request = request();
    controller.reserve(request.clone()).unwrap();
    let directory = controller
        .lock()
        .unwrap()
        .ownership
        .allocate(request)
        .unwrap();
    let (mut parent_ready, child_ready) = UnixStream::pair().unwrap();
    let (mut parent_release, child_release) = UnixStream::pair().unwrap();
    parent_ready
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut unrelated = Command::new("/usr/bin/true");
    unsafe {
        unrelated.pre_exec(move || {
            // Only async-signal-safe syscalls in the forked child. This child
            // has no delegated capture lease; it briefly retains CLOEXEC FDs.
            let mut byte = [1_u8];
            if libc::write(child_ready.as_raw_fd(), byte.as_ptr().cast(), 1) != 1
                || libc::read(child_release.as_raw_fd(), byte.as_mut_ptr().cast(), 1) != 1
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let worker = std::thread::spawn(move || {
        let child = unrelated.spawn();
        drop(unrelated);
        child
    });
    let ready = parent_ready.read_exact(&mut [0_u8]);
    let rejected = ready.is_ok().then(|| directory.cleanup());
    let kept = directory.path.exists();
    let released = parent_release.write_all(&[1_u8]);
    drop(parent_release);
    let mut child = worker.join().unwrap().unwrap();
    let exited = child.wait().unwrap();
    ready.unwrap();
    released.unwrap();
    assert!(exited.success());
    assert!(rejected
        .unwrap()
        .unwrap_err()
        .ends_with("Resource temporarily unavailable (os error 35)"));
    assert!(kept);
    cleanup_after_unrelated_forks(&directory).unwrap();
    assert!(!directory.path.exists());
}
#[test]
fn test_admission_is_single_owner_one_shot_and_has_no_meeting_identity() {
    let (controller, _fixture) = controller();
    let request = request();
    let first = controller.reserve(request.clone()).unwrap();
    assert_eq!(first.run_id, request.run_id);
    assert!(first.busy);
    assert_eq!(first.phase, "preparing");
    assert!(controller.reserve(request.clone()).is_err());
    assert!(controller.reserve(super::tests::request()).is_err());
    assert!(controller.require_empty().is_err());
    assert!(controller
        .snapshot(Some(&uuid::Uuid::new_v4().to_string()))
        .is_err());
    let value = serde_json::to_value(first).unwrap();
    assert!(value.get("meetingId").is_none());
    assert!(value.get("directory").is_none());
    controller.lock().unwrap().current = None;
    assert!(controller.reserve(request).is_err());
    assert!(controller.require_empty().is_ok());
}
#[test]
fn test_exit_seal_rechecks_pending_tests_and_blocks_late_capture_until_abort() {
    let (controller, _fixture) = controller();
    controller.seal_exit().unwrap();
    assert!(controller.reserve(request()).is_err());
    assert!(controller.require_empty().is_err());
    assert!(controller.seal_exit().is_err());
    controller.abort_exit().unwrap();
    assert!(controller.require_empty().is_ok());
    controller.reserve(request()).unwrap();
    assert!(controller.seal_exit().is_err());
    // A ready or failed test still owns private audio until explicit discard.
    let mut state = controller.lock().unwrap();
    let run = state.current.as_mut().unwrap();
    run.task_done = true;
    run.snapshot.phase = "error".into();
    run.snapshot.busy = false;
    drop(state);
    assert!(controller.seal_exit().is_err());
}
#[test]
fn test_start_command_fixes_uid_owner_stop_parent_and_application_instance() {
    let mut r = request();
    r.capture_scope = CaptureScope {
        mode: "application".into(),
        application: Some(crate::capture_scope::ApplicationIdentity {
            pid: 123,
            bundle_identifier: "com.example.browser".into(),
            launch_time_us: 1791100000000001,
        }),
    };
    let cmd = command(
        std::path::Path::new("/fake/helper"),
        std::path::Path::new("/fake/private"),
        &r,
    )
    .unwrap();
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    for (key, value) in [
        ("--microphone-uid", r.microphone_uid.as_str()),
        ("--five-second-test", r.run_id.as_str()),
        ("--pid", "123"),
    ] {
        let i = args.iter().position(|a| a == key).unwrap();
        assert_eq!(args[i + 1], value);
    }
    assert!(!args
        .iter()
        .any(|a| a == "--until-stopped" || a == "--duration"));
    let i = args
        .iter()
        .position(|a| a == "--application-identity")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<crate::capture_scope::ApplicationIdentity>(&args[i + 1]).unwrap(),
        r.capture_scope.application.unwrap()
    );
}
#[test]
fn test_import_confirmation_rejects_new_targets_and_unconfirmed_work() {
    let idle = crate::import_controller::Snapshot::default();
    let expected = request().expected_import;
    assert!(expected.require(&idle).is_ok());
    let mut changed = idle.clone();
    changed.busy = true;
    assert!(expected.require(&changed).is_err());
    changed.busy = false;
    changed.revision = 1;
    assert!(expected.require(&changed).is_err());
    let id = uuid::Uuid::new_v4().to_string();
    let expected = ImportExpectation {
        token: Some(id.clone()),
        busy: true,
        revision: 5,
    };
    let mut running = idle;
    running.token = Some(id);
    running.revision = 20;
    running.busy = true;
    assert!(expected.require(&running).is_ok());
    running.token = Some(uuid::Uuid::new_v4().to_string());
    assert!(expected.require(&running).is_err());
}
#[test]
fn test_owned_directory_cleanup_never_deletes_unknown_or_linked_files() {
    let directory = files::TestDirectory::new().unwrap();
    let path = directory.path.clone();
    std::fs::write(path.join("unrelated"), "keep").unwrap();
    assert!(directory.cleanup().is_err());
    assert_eq!(std::fs::read(path.join("unrelated")).unwrap(), b"keep");
    std::fs::remove_file(path.join("unrelated")).unwrap();
    #[cfg(unix)]
    {
        let external = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(external.path(), path.join("microphone.caf")).unwrap();
        assert!(directory.open("microphone.caf", 1024).is_err());
        assert!(directory.cleanup().is_err());
        assert!(external.path().exists());
        std::fs::remove_file(path.join("microphone.caf")).unwrap();
    }
    directory.request_stop().unwrap();
    directory.request_stop().unwrap();
    directory.cleanup().unwrap();
    assert!(!path.exists());
    directory.cleanup().unwrap();
}
#[cfg(unix)]
#[test]
fn test_watch_deadline_and_cancel_reap_only_the_owned_fake_process() {
    let r = request();
    for cancel in [false, true] {
        let directory = files::TestDirectory::new().unwrap();
        let child = Probe(Mutex::new(
            Command::new("/bin/sleep").arg("30").spawn().unwrap(),
        ));
        let flag = AtomicBool::new(cancel);
        assert!(watch(
            &directory,
            &child,
            &r,
            &flag,
            Duration::from_millis(1),
            || Ok(())
        )
        .is_err());
        assert!(child.exited().unwrap().is_some());
        if cancel {
            assert!(directory.path.join("stop.request").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn test_failed_exit_confirmation_keeps_owner_busy_and_retries_same_process() {
    let (controller, _fixture) = controller();
    let request = request();
    controller.reserve(request.clone()).unwrap();
    let child = Arc::new(Probe(Mutex::new(
        Command::new("/bin/sleep").arg("30").spawn().unwrap(),
    )));
    controller.lock().unwrap().current.as_mut().unwrap().child = Some(child.clone());
    let mut attempts = 0;
    let unresolved = std::cell::Cell::new(0);
    reap_until_confirmed(
        || {
            attempts += 1;
            if attempts <= 2 {
                assert!(child.exited().unwrap().is_none());
                return Err("injected transient wait failure".into());
            }
            child.terminate()
        },
        |_| {
            unresolved.set(unresolved.get() + 1);
            let state = controller.lock().unwrap();
            let run = state.current.as_ref().unwrap();
            assert!(!run.task_done);
            assert!(run.snapshot.busy);
            assert!(Arc::ptr_eq(run.child.as_ref().unwrap(), &child));
            drop(state);
            assert!(controller.require_empty().is_err());
            assert!(controller.reserve(super::tests::request()).is_err());
        },
        || {},
    );
    assert_eq!(unresolved.get(), 2);
    assert_eq!(attempts, 3);
    assert!(child.exited().unwrap().is_some());
}

fn segment(from: u64, to: u64, start: u64, end: u64) -> serde_json::Value {
    serde_json::json!({"sourceFrameStart":from,"sourceFrameEnd":to,
        "timelineStartUs":start,"timelineEndUs":end,
        "mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"})
}
fn write_manifest(directory: &files::TestDirectory, manifest: &serde_json::Value) {
    std::fs::write(
        directory.path.join("manifest.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
}
fn saved_fixture() -> (files::TestDirectory, Request, serde_json::Value) {
    let directory = files::TestDirectory::new().unwrap();
    let request = request();
    let manifest = serde_json::json!({
        "schemaVersion":2,"captureMode":"system","backend":"coreaudio",
        "targetPIDs":[],"microphoneDeviceUID":request.microphone_uid,
        "targetApplicationIdentity":null,"state":"saved","errors":[],
        "captureTest":{"ownerId":request.run_id,"durationSeconds":5,
            "windowStartNs":1_000_000_000_i64,"windowEndNs":6_000_000_000_i64,
            "firstSavedCaptureNs":1_000_000_000_i64,"completed":true},
        "captureClock":{"domain":"mach_continuous_time","originUtcMs":1_700_000_000_000_i64,"uncertaintyNs":20},
        "microphone":{"frames":40000,"sampleRate":8000,"channels":2,"appendFailures":0,
            "clockSegments":[segment(0,40000,0,5_000_000)]},
        "system":{"frames":8000,"sampleRate":8000,"channels":2,"appendFailures":0,
            "clockSegments":[segment(0,4000,100_000,600_000),segment(4000,8000,1_600_000,2_100_000)]}
    });
    for (name, frames, value) in [
        ("microphone.caf", 40000, 0.125_f32),
        ("system.caf", 8000, 0.0),
    ] {
        let samples = [value.to_le_bytes(), (-value).to_le_bytes()]
            .concat()
            .repeat(frames);
        std::fs::write(
            directory.path.join(name),
            crate::audio::tests::caf_fixture(8000, 2, 3, 32, 4, &samples),
        )
        .unwrap();
    }
    write_manifest(&directory, &manifest);
    (directory, request, manifest)
}

#[test]
fn failed_initialization_reports_primary_error_without_accepting_unconfirmed_audio() {
    let (directory, request, mut manifest) = saved_fixture();
    manifest["state"] = serde_json::json!("failed");
    manifest["microphoneDeviceUID"] = serde_json::Value::Null;
    manifest["microphone"]["frames"] = serde_json::json!(0);
    manifest["system"]["frames"] = serde_json::json!(0);
    manifest["errors"] = serde_json::json!([
        "マイクの権限確認が完了しませんでした（未決定）",
        "no system samples"
    ]);
    write_manifest(&directory, &manifest);
    let failed = directory.manifest().unwrap().unwrap();
    assert!(failed
        .failure(&request)
        .unwrap()
        .unwrap()
        .starts_with("マイクの権限確認"));
    // The acceptance path still rejects a missing UID and failed state.
    assert!(failed.require_owner(&request).is_err());
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err());
    for (key, value) in [
        ("microphoneDeviceUID", serde_json::json!("another-mic")),
        ("targetPIDs", serde_json::json!([123])),
        ("captureMode", serde_json::json!("application")),
    ] {
        let mut changed = manifest.clone();
        changed[key] = value;
        write_manifest(&directory, &changed);
        assert!(directory
            .manifest()
            .unwrap()
            .unwrap()
            .failure(&request)
            .is_err());
    }
    manifest["captureTest"]["ownerId"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
    write_manifest(&directory, &manifest);
    assert!(directory
        .manifest()
        .unwrap()
        .unwrap()
        .failure(&request)
        .is_err());
}

#[test]
#[cfg(unix)]
fn initialization_failure_is_preserved_before_and_after_helper_exit() {
    for already_exited in [false, true] {
        let (directory, request, mut manifest) = saved_fixture();
        manifest["state"] = serde_json::json!("failed");
        manifest["microphoneDeviceUID"] = serde_json::Value::Null;
        manifest["errors"] = serde_json::json!(["audio engine initialization failed -123"]);
        write_manifest(&directory, &manifest);
        let mut command = if already_exited {
            Command::new("/usr/bin/false")
        } else {
            let mut c = Command::new("/bin/sleep");
            c.arg("30");
            c
        };
        let child = Probe(Mutex::new(command.spawn().unwrap()));
        if already_exited {
            child.0.lock().unwrap().wait().unwrap();
        }
        let result = watch(
            &directory,
            &child,
            &request,
            &AtomicBool::new(false),
            Duration::from_secs(1),
            || panic!("failed initialization is not recording"),
        );
        child.terminate().unwrap();
        assert_eq!(
            result.unwrap_err(),
            "audio engine initialization failed -123"
        );
        assert!(child.exited().unwrap().is_some());
    }
}

fn read_all(mut audio: crate::playback::PreparedAudio, frames: u64, value: f32) {
    assert_eq!(audio.total_frames(), frames);
    assert_eq!((audio.sample_rate, audio.channels), (8000, 2));
    assert!(audio.clock.is_none());
    assert_eq!(audio.alignment, "unverified");
    for first in (0..frames).step_by(4096) {
        let count = (frames - first).min(4096);
        let bytes = audio.read_pcm(first, count).unwrap();
        assert_eq!(bytes.len() as u64, count * 8);
        for frame in bytes.chunks_exact(8) {
            assert_eq!(f32::from_le_bytes(frame[..4].try_into().unwrap()), value);
            assert_eq!(f32::from_le_bytes(frame[4..].try_into().unwrap()), -value);
        }
    }
    assert!(audio.read_pcm(frames, 1).is_err());
}

#[test]
fn test_private_verification_and_individual_preview_preserve_pcm_and_gaps() {
    let (directory, request, _) = saved_fixture();
    let before = std::fs::read(directory.path.join("manifest.json")).unwrap();
    let ready = files::verify(&directory, &request, &crate::test_support::never_cancelled).unwrap();
    assert_eq!(ready.microphone.info.duration_ms, 5000);
    assert!(!ready.microphone.info.has_gaps);
    assert_eq!(ready.system.as_ref().unwrap().info.duration_ms, 1000);
    assert!(ready.system.as_ref().unwrap().info.has_gaps);
    read_all(
        ready
            .microphone
            .playback(&directory, "mic", &crate::test_support::never_cancelled)
            .unwrap(),
        40000,
        0.125,
    );
    read_all(
        ready
            .system
            .as_ref()
            .unwrap()
            .playback(&directory, "system", &crate::test_support::never_cancelled)
            .unwrap(),
        8000,
        0.0,
    );
    assert_eq!(
        std::fs::read(directory.path.join("manifest.json")).unwrap(),
        before
    );
}

#[test]
fn completed_microphone_test_accepts_explicitly_unreceived_pc_without_inventing_audio() {
    let (directory, request, mut manifest) = saved_fixture();
    // This is a private synthetic file, not any saved user recording.
    std::fs::remove_file(directory.path.join("system.caf")).unwrap();
    manifest["systemAudioUnreceived"] = serde_json::json!(true);
    manifest["system"] = serde_json::json!({"frames":0,"callbackCount":0,"appendFailures":0,
        "sampleRate":null,"channels":null,"clockFailure":null,"clockSegments":[]});
    write_manifest(&directory, &manifest);
    assert!(files::verify(&directory, &request, &|| true).is_err());
    let ready = files::verify(&directory, &request, &crate::test_support::never_cancelled).unwrap();
    assert!(ready.system.is_none());
    assert_eq!(ready.microphone.info.duration_ms, 5000);
    read_all(
        ready
            .microphone
            .playback(&directory, "mic", &crate::test_support::never_cancelled)
            .unwrap(),
        40000,
        0.125,
    );
    assert!(!directory.path.join("system.caf").exists());
    for (pointer, value) in [
        ("/systemAudioUnreceived", serde_json::json!(false)),
        ("/system/frames", serde_json::json!(1)),
        ("/system/callbackCount", serde_json::json!(1)),
        ("/system/callbackCount", serde_json::Value::Null),
        ("/system/sampleRate", serde_json::json!(8000)),
        ("/system/channels", serde_json::json!(2)),
        ("/system/appendFailures", serde_json::json!(1)),
        ("/system/clockFailure", serde_json::json!("clock failed")),
        ("/system/clockSegments", serde_json::Value::Null),
        (
            "/system/clockSegments",
            serde_json::json!([segment(0, 1, 0, 125)]),
        ),
        ("/captureTest/completed", serde_json::json!(false)),
        (
            "/microphoneDeviceUID",
            serde_json::json!("other microphone"),
        ),
        (
            "/captureTest/ownerId",
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        ),
        ("/errors", serde_json::json!(["permission denied"])),
    ] {
        let mut changed = manifest.clone();
        *changed
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("missing {pointer}")) = value;
        write_manifest(&directory, &changed);
        assert!(
            files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err(),
            "{pointer}"
        );
    }
    write_manifest(&directory, &manifest);
    std::fs::write(directory.path.join("system.caf"), b"corrupt or empty CAF").unwrap();
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(directory.path.join("system.caf")).unwrap();
        std::os::unix::fs::symlink("missing-system.caf", directory.path.join("system.caf"))
            .unwrap();
        assert!(
            files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err()
        );
    }
}

#[test]
fn test_private_verification_rejects_changed_owner_scope_clock_and_saved_conditions() {
    let (directory, request, original) = saved_fixture();
    for (pointer, value) in [
        (
            "/captureTest/ownerId",
            serde_json::json!(uuid::Uuid::new_v4().to_string()),
        ),
        (
            "/microphoneDeviceUID",
            serde_json::json!("other microphone"),
        ),
        ("/schemaVersion", serde_json::json!(1)),
        ("/backend", serde_json::json!("screencapturekit")),
        ("/targetPIDs", serde_json::json!([123])),
        (
            "/targetApplicationIdentity",
            serde_json::json!({"pid":123,"bundleIdentifier":"com.example.browser","launchTimeUs":1791100000000001_i64}),
        ),
        ("/state", serde_json::json!("recording")),
        ("/errors", serde_json::json!(["failed save"])),
        ("/captureTest/completed", serde_json::json!(false)),
        ("/captureTest/durationSeconds", serde_json::json!(4)),
        ("/captureTest/windowStartNs", serde_json::json!(-1)),
        (
            "/captureTest/windowEndNs",
            serde_json::json!(6_000_000_001_i64),
        ),
        ("/captureTest/firstSavedCaptureNs", serde_json::json!(null)),
        (
            "/captureTest/firstSavedCaptureNs",
            serde_json::json!(999_999_999_i64),
        ),
        (
            "/captureTest/firstSavedCaptureNs",
            serde_json::json!(6_000_000_000_i64),
        ),
        ("/captureClock/domain", serde_json::json!("host-arrival")),
        ("/captureClock/uncertaintyNs", serde_json::json!(-1)),
        ("/captureClock/originUtcMs", serde_json::json!(-1)),
        ("/microphone/frames", serde_json::json!(40001)),
        ("/microphone/frames", serde_json::json!(39999)),
        ("/microphone/sampleRate", serde_json::json!(8000.5)),
        ("/microphone/channels", serde_json::json!(1)),
        ("/microphone/appendFailures", serde_json::json!(1)),
        ("/microphone/clockSegments", serde_json::json!([])),
        (
            "/microphone/clockSegments/0/sourceFrameEnd",
            serde_json::json!(39999),
        ),
        (
            "/microphone/clockSegments/0/timelineEndUs",
            serde_json::json!(5_000_001),
        ),
        (
            "/microphone/clockSegments/0/timelineStartUs",
            serde_json::json!(1),
        ),
        (
            "/system/clockSegments/1/sourceFrameStart",
            serde_json::json!(4001),
        ),
        (
            "/system/clockSegments/1/timelineStartUs",
            serde_json::json!(599_999),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        write_manifest(&directory, &changed);
        assert!(
            files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err(),
            "accepted {pointer}"
        );
        assert!(directory.path.join("microphone.caf").exists());
        assert!(directory.path.join("system.caf").exists());
    }
    write_manifest(&directory, &original);
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_ok());
}

#[test]
fn test_private_manifest_rejects_duplicate_owner_and_oversize_or_invalid_records() {
    let (directory, request, original) = saved_fixture();
    let original_bytes = serde_json::to_vec(&original).unwrap();
    let text = String::from_utf8(original_bytes.clone()).unwrap();
    let duplicate = text.replacen("\"ownerId\":", "\"ownerId\":\"other\",\"ownerId\":", 1);
    for bytes in [
        duplicate.into_bytes(),
        vec![b' '; 256 * 1024 + 1],
        b"{}".to_vec(),
    ] {
        std::fs::write(directory.path.join("manifest.json"), &bytes).unwrap();
        assert!(
            files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err()
        );
        assert_eq!(
            std::fs::read(directory.path.join("manifest.json")).unwrap(),
            bytes
        );
    }
    std::fs::write(directory.path.join("manifest.json"), original_bytes).unwrap();
}

#[test]
fn test_private_verification_and_preview_cancel_and_refuse_modified_same_format_audio() {
    let (directory, request, _) = saved_fixture();
    assert!(files::verify(&directory, &request, &|| true).is_err());
    let ready = files::verify(&directory, &request, &crate::test_support::never_cancelled).unwrap();
    assert!(ready
        .microphone
        .playback(&directory, "mic", &|| true)
        .is_err());
    let samples = [0.25_f32.to_le_bytes(), (-0.25_f32).to_le_bytes()]
        .concat()
        .repeat(40000);
    std::fs::write(
        directory.path.join("microphone.caf"),
        crate::audio::tests::caf_fixture(8000, 2, 3, 32, 4, &samples),
    )
    .unwrap();
    assert!(ready
        .microphone
        .playback(&directory, "mic", &crate::test_support::never_cancelled)
        .is_err());
    assert!(ready
        .system
        .as_ref()
        .unwrap()
        .playback(&directory, "system", &crate::test_support::never_cancelled)
        .is_ok());
    // A fresh proof may inspect a different file, but an existing result never
    // silently rebinds itself to it.
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_ok());
}

#[cfg(unix)]
#[test]
fn test_private_directory_refuses_replaced_root_hardlinks_and_nonempty_stop() {
    let (directory, request, _) = saved_fixture();
    let parent = tempfile::tempdir().unwrap();
    let moved = parent.path().join("moved-owned-directory");
    std::fs::rename(&directory.path, &moved).unwrap();
    std::fs::create_dir(&directory.path).unwrap();
    std::fs::write(directory.path.join("keep"), b"unrelated directory").unwrap();
    assert!(directory.cleanup().is_err());
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err());
    assert_eq!(
        std::fs::read(directory.path.join("keep")).unwrap(),
        b"unrelated directory"
    );
    std::fs::remove_file(directory.path.join("keep")).unwrap();
    std::fs::remove_dir(&directory.path).unwrap();
    std::fs::rename(moved, &directory.path).unwrap();
    let external = parent.path().join("external-audio");
    std::fs::hard_link(directory.path.join("microphone.caf"), &external).unwrap();
    assert!(files::verify(&directory, &request, &crate::test_support::never_cancelled).is_err());
    assert!(directory.cleanup().is_err());
    assert!(external.exists());
    std::fs::remove_file(external).unwrap();
    std::fs::write(
        directory.path.join("stop.request"),
        b"not owned stop request",
    )
    .unwrap();
    assert!(directory.request_stop().is_err());
    assert!(directory.cleanup().is_err());
    std::fs::remove_file(directory.path.join("stop.request")).unwrap();
    directory.cleanup().unwrap();
}

#[test]
fn test_private_levels_use_the_pinned_file_and_do_not_fabricate_measured_silence() {
    let directory = files::TestDirectory::new().unwrap();
    let r = request();
    let waiting = serde_json::json!({"state":"waiting","windowFrames":null,"ageMs":null,"rms":null,"peak":null});
    let publication = serde_json::json!({"schemaVersion":1,"ownerId":r.run_id,"revision":1,
        "microphone":waiting,"system":waiting});
    std::fs::write(
        directory.path.join("input-levels.json"),
        serde_json::to_vec(&publication).unwrap(),
    )
    .unwrap();
    let mut reader = crate::input_levels::LevelReader::default();
    let info = reader
        .read_file(
            directory.open("input-levels.json", 16 * 1024).unwrap(),
            &r.run_id,
        )
        .unwrap();
    let value = serde_json::to_value(info).unwrap();
    assert_eq!(value["microphone"]["state"], "waiting");
    assert!(value["microphone"]["rms"].is_null());
    assert!(reader
        .read_file(
            directory.open("input-levels.json", 16 * 1024).unwrap(),
            &uuid::Uuid::new_v4().to_string()
        )
        .is_err());
}

#[test]
#[ignore = "requires actual Swift-generated five-second test CAF fixtures; make check-capture-test"]
fn swift_generated_five_second_test_is_verified_for_private_individual_preview() {
    let source =
        std::path::PathBuf::from(std::env::var_os("MON_CAPTURE_TEST_FIXTURE_DIR").unwrap());
    let directory = files::TestDirectory::new().unwrap();
    for name in ["manifest.json", "microphone.caf", "system.caf"] {
        std::fs::copy(source.join(name), directory.path.join(name)).unwrap();
    }
    let original = std::fs::read(directory.path.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut request = request();
    request.run_id = manifest["captureTest"]["ownerId"].as_str().unwrap().into();
    request.microphone_uid = "fixture-microphone".into();
    let ready = files::verify(&directory, &request, &crate::test_support::never_cancelled).unwrap();
    assert!(!ready.microphone.info.has_gaps);
    assert!(ready.system.as_ref().unwrap().info.has_gaps);
    assert_eq!(ready.microphone.info.duration_ms, 5000);
    assert_eq!(ready.system.as_ref().unwrap().info.duration_ms, 3900);
    read_all(
        ready
            .microphone
            .playback(&directory, "mic", &crate::test_support::never_cancelled)
            .unwrap(),
        40000,
        0.125,
    );
    read_all(
        ready
            .system
            .as_ref()
            .unwrap()
            .playback(&directory, "system", &crate::test_support::never_cancelled)
            .unwrap(),
        31200,
        0.0,
    );
    assert_eq!(
        std::fs::read(directory.path.join("manifest.json")).unwrap(),
        original
    );
}
