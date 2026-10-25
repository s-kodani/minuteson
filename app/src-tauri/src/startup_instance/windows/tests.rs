//! All OS operations below target children of this test executable and private
//! temporary roots. Never discover or stop the user's installed MON.
use super::*;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
};
use windows_sys::Win32::{Foundation::*, System::Pipes::*, UI::WindowsAndMessaging::*};
const TITLE: &str = "MON startup recovery synthetic fixture";
#[test]
fn rendezvous_separates_users_products_and_config_roots() {
    let config = Path::new(r"C:\Users\fixture\MON");
    let endpoint = endpoint_for(config, "jp.mon.desktop", "S-1-5-21-fixture");
    assert_eq!(
        endpoint,
        endpoint_for(
            Path::new(r"c:\users\FIXTURE\mon"),
            "jp.mon.desktop",
            "S-1-5-21-fixture"
        )
    );
    assert_ne!(
        endpoint,
        endpoint_for(config, "jp.mon.desktop.qa", "S-1-5-21-fixture")
    );
    assert_ne!(
        endpoint,
        endpoint_for(config, "jp.mon.desktop", "S-1-5-21-other")
    );
    assert_ne!(
        endpoint,
        endpoint_for(
            Path::new(r"C:\Users\fixture\Other"),
            "jp.mon.desktop",
            "S-1-5-21-fixture"
        )
    );
}
struct Fixture {
    child: Child,
    root: tempfile::TempDir,
    endpoint: String,
}
impl Fixture {
    fn start(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        crate::native_fs::private_directories(root.path(), true).unwrap();
        let endpoint = format!(r"\\.\pipe\mon-startup-test-{}", uuid::Uuid::new_v4());
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "startup_instance::windows::tests::process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_STARTUP_FIXTURE_ROOT", root.path())
            .env("MON_STARTUP_FIXTURE_MODE", mode)
            .env("MON_STARTUP_FIXTURE_PIPE", &endpoint)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let fixture = Self {
            child,
            root,
            endpoint,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !fixture.root.path().join("ready").exists() {
            assert!(
                Instant::now() < deadline,
                "fixture did not become ready: {mode}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        fixture
    }
    fn coordinator(&self) -> Coordinator {
        Coordinator {
            config: self.root.path().canonicalize().unwrap(),
            executable: std::env::current_exe().unwrap().canonicalize().unwrap(),
            title: TITLE.into(),
            endpoint: self.endpoint.clone(),
            guard: None,
            stop: Arc::new(AtomicBool::new(false)),
            target: None,
            target_id: None,
            message: String::new(),
        }
    }
    fn target(&self) -> process::Target {
        process::Target::open(
            self.child.id(),
            &std::env::current_exe().unwrap().canonicalize().unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "private child-process entrypoint; run only through the fixture"]
fn process_helper() {
    let root = PathBuf::from(std::env::var_os("MON_STARTUP_FIXTURE_ROOT").unwrap());
    let mode = std::env::var("MON_STARTUP_FIXTURE_MODE").unwrap();
    if mode == "resident-worker" {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    if mode.starts_with("resident-") {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "startup_instance::windows::tests::process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_STARTUP_FIXTURE_MODE", "supervisor")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if mode == "resident-orphan" {
            command.env("MON_RESIDENCY_FIXTURE_LOSS", "1");
        }
        supervisor::test_prepare(&mut command).unwrap();
        crate::local_engine::windows::initialize().unwrap();
        let worker = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "startup_instance::windows::tests::process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_STARTUP_FIXTURE_MODE", "resident-worker")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(root.join("worker-pid"), worker.id().to_string()).unwrap();
        let guid = uuid::Uuid::new_v4();
        let icon = if mode == "resident-tray" {
            Some(
                tray_icon::TrayIconBuilder::new()
                    .with_guid(guid.as_u128())
                    .with_tooltip("MON — synthetic resident test")
                    .with_icon(
                        tray_icon::Icon::from_rgba([32, 32, 32, 255].repeat(256), 16, 16).unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
        } else {
            None
        };
        let probe = crate::residency::Probe {
            tray: icon.as_ref().map(|_| guid.to_string()),
            tray_present: icon.as_ref().is_some_and(|icon| icon.rect().is_some()),
            ..Default::default()
        };
        std::fs::write(root.join("entry-guid"), guid.to_string()).unwrap();
        std::fs::write(root.join("ready"), b"ready").unwrap();
        let mut last = Instant::now() - Duration::from_secs(1);
        while !root.join("stop").exists() {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if last.elapsed() >= Duration::from_secs(1) {
                last = Instant::now();
                if supervisor::pulse(&probe).unwrap_or(false) {
                    std::fs::write(root.join("drain-requested"), b"drain").unwrap();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(icon);
        return;
    }
    if mode == "supervisor" {
        let args = [
            "--exact",
            "startup_instance::windows::tests::process_helper",
            "--ignored",
            "--nocapture",
        ]
        .map(Into::into)
        .to_vec();
        supervisor::test_relay(args).unwrap();
        return;
    }
    if mode == "restart-successor" {
        std::fs::write(root.join("successor-ready"), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        return;
    }
    if mode.starts_with("restart-parent") {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "startup_instance::windows::tests::process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_STARTUP_FIXTURE_MODE", "supervisor")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        supervisor::test_prepare(&mut command).unwrap();
        // The production ordering is deliberate: the relay exists before this
        // parent and all later workers are assigned to the kill-on-close job.
        crate::local_engine::windows::initialize().unwrap();
        std::fs::write(root.join("relay-endpoint"), supervisor::test_endpoint()).unwrap();
        std::fs::write(root.join("ready"), b"ready").unwrap();
        if mode == "restart-parent" {
            supervisor::schedule().unwrap();
            return;
        }
        loop {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join(".bootstrap.lock"))
        .unwrap();
    guard.lock().unwrap();
    if mode.starts_with("pipe") {
        let endpoint = std::env::var("MON_STARTUP_FIXTURE_PIPE").unwrap();
        let mut file = pipe::server(&endpoint).unwrap();
        std::fs::write(root.join("ready"), b"ready").unwrap();
        loop {
            let connected = unsafe {
                ConnectNamedPipe(
                    std::os::windows::io::AsRawHandle::as_raw_handle(&file),
                    std::ptr::null_mut(),
                )
            } != 0
                || std::io::Error::last_os_error().raw_os_error()
                    == Some(ERROR_PIPE_CONNECTED as i32);
            if !connected {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let request = pipe::line(&mut file, Instant::now() + Duration::from_secs(1)).unwrap();
            assert_eq!(request, b"MON_ACTIVATE_V1\n");
            if mode != "pipe-hung" {
                let ack = serde_json::json!({"version":1,"pid":std::process::id(),"shown":mode=="pipe-ready"});
                file.write_all(format!("{ack}\n").as_bytes()).unwrap();
            } else {
                std::thread::sleep(Duration::from_secs(5));
            }
            std::thread::sleep(Duration::from_millis(300));
            unsafe {
                DisconnectNamedPipe(std::os::windows::io::AsRawHandle::as_raw_handle(&file));
            }
        }
    }
    if mode.starts_with("legacy") {
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                process::wide("STATIC").as_ptr(),
                process::wide(TITLE).as_ptr(),
                WS_OVERLAPPEDWINDOW,
                -10000,
                -10000,
                160,
                80,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        if mode == "legacy-minimized" {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
        std::fs::write(root.join("ready"), b"ready").unwrap();
        if mode == "legacy-hung" {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    std::fs::write(root.join("ready"), b"ready").unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn pipe_activation_pins_the_actual_peer_and_requires_successful_ack() {
    for (mode, expected) in [("pipe-ready", true), ("pipe-denied", false)] {
        let fixture = Fixture::start(mode);
        let mut coordinator = fixture.coordinator();
        assert_eq!(coordinator.open_existing().unwrap(), expected);
        assert_eq!(
            coordinator.target.as_ref().unwrap().identity(),
            fixture.target().identity()
        );
        assert!(coordinator.report().can_restart);
        assert!(fixture.target().exited() == false);
    }
}
#[test]
fn transport_timeout_retains_verified_owner_for_review_instead_of_losing_it() {
    let fixture = Fixture::start("pipe-hung");
    let (target, shown) = pipe::activate(
        &fixture.endpoint,
        &std::env::current_exe().unwrap().canonicalize().unwrap(),
        Duration::from_millis(350),
    )
    .unwrap();
    assert!(!shown);
    assert_eq!(target.identity(), fixture.target().identity());
    assert!(!target.exited());
}
#[test]
fn legacy_bootstrap_owner_is_found_and_hidden_or_minimized_window_is_restored() {
    for mode in ["legacy-hidden", "legacy-minimized"] {
        let fixture = Fixture::start(mode);
        let mut coordinator = fixture.coordinator();
        coordinator.discover(".bootstrap.lock").unwrap();
        assert_eq!(
            coordinator.target.as_ref().unwrap().identity(),
            fixture.target().identity()
        );
        assert!(coordinator
            .target
            .as_ref()
            .unwrap()
            .restore_legacy(TITLE)
            .unwrap());
        assert!(!fixture.target().exited());
    }
}
#[test]
fn reviewed_unresponsive_owner_is_stopped_and_real_bootstrap_lock_is_released() {
    let fixture = Fixture::start("legacy-hung");
    let mut coordinator = fixture.coordinator();
    // Primary recovery owns its broker, so it never contacts itself.
    let broker = coordinator.lock_file(".startup-instance.lock").unwrap();
    broker.lock().unwrap();
    coordinator.guard = Some(broker);
    coordinator.discover(".bootstrap.lock").unwrap();
    assert!(!coordinator.open_existing().unwrap());
    let token = coordinator.report().target_id.unwrap();
    assert!(!coordinator.recover(&token).unwrap());
    assert!(coordinator.target.as_ref().unwrap().exited());
    assert!(!coordinator.report().can_restart);
    assert!(coordinator
        .lock_file(".bootstrap.lock")
        .unwrap()
        .try_lock()
        .is_ok());
    assert!(
        coordinator.recover(&token).is_err(),
        "consumed consent must not replay"
    );
}
#[test]
fn wrong_consent_and_changed_generation_never_stop_the_process() {
    let fixture = Fixture::start("legacy-hung");
    let mut coordinator = fixture.coordinator();
    coordinator.set_target(fixture.target());
    let token = coordinator.report().target_id.unwrap();
    assert!(coordinator.recover("wrong-review").is_err());
    assert!(!fixture.target().exited());
    let other = Fixture::start("legacy-hung");
    coordinator.set_target(other.target());
    assert!(coordinator.recover(&token).is_err());
    assert!(!fixture.target().exited());
    assert!(!other.target().exited());
}
#[test]
fn a_recovered_owner_is_opened_instead_of_terminated_after_review() {
    let fixture = Fixture::start("pipe-ready");
    let mut coordinator = fixture.coordinator();
    coordinator.set_target(fixture.target());
    let token = coordinator.report().target_id.unwrap();
    assert!(coordinator.recover(&token).unwrap());
    assert!(!fixture.target().exited());
}
#[test]
fn wrong_executable_and_current_process_are_not_recovery_targets() {
    let fixture = Fixture::start("legacy-hung");
    assert!(process::Target::open(fixture.child.id(), Path::new(r"C:\not-mon.exe")).is_err());
    assert!(process::Target::open(std::process::id(), &std::env::current_exe().unwrap()).is_err());
    assert!(!fixture.target().exited());
}
#[test]
fn separate_config_roots_have_independent_exclusive_brokers() {
    let first = Fixture::start("legacy-hung");
    let second = Fixture::start("legacy-hung");
    let a = first.coordinator();
    let b = second.coordinator();
    let a_guard = a.lock_file(".startup-instance.lock").unwrap();
    a_guard.try_lock().unwrap();
    assert!(matches!(
        a.lock_file(".startup-instance.lock").unwrap().try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    let b_guard = b.lock_file(".startup-instance.lock").unwrap();
    b_guard.try_lock().unwrap();
    assert_ne!(a.endpoint, b.endpoint);
}
#[test]
fn approved_restart_survives_the_old_application_job_and_waits_for_parent_exit() {
    let mut fixture = Fixture::start("restart-parent");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fixture.root.path().join("successor-ready").exists() {
        assert!(
            Instant::now() < deadline,
            "independent restart successor did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        fixture.child.try_wait().unwrap().is_some(),
        "successor started before old parent exit"
    );
    let pid = std::fs::read_to_string(fixture.root.path().join("successor-ready"))
        .unwrap()
        .parse()
        .unwrap();
    let successor = process::Target::open(
        pid,
        &std::env::current_exe().unwrap().canonicalize().unwrap(),
    )
    .unwrap();
    assert!(!successor.exited(), "old job killed the restart successor");
    assert!(successor.wait_exit(Duration::from_secs(5)));
}
#[test]
fn restart_relay_rejects_other_clients_and_exits_with_unapproved_parent() {
    use std::os::windows::io::AsRawHandle;
    let mut fixture = Fixture::start("restart-parent-idle");
    let endpoint = std::fs::read_to_string(fixture.root.path().join("relay-endpoint")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut file = loop {
        match OpenOptions::new().read(true).write(true).open(&endpoint) {
            Ok(file) => break file,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => panic!("relay unavailable: {error}"),
        }
    };
    let mut relay_pid = 0;
    assert_ne!(
        unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut relay_pid) },
        0
    );
    let relay = process::Target::open(
        relay_pid,
        &std::env::current_exe().unwrap().canonicalize().unwrap(),
    )
    .unwrap();
    let _ = file.write_all(b"MON_RESTART_AFTER_PARENT_EXIT_V1\n");
    assert!(pipe::line(&mut file, Instant::now() + Duration::from_millis(500)).is_err());
    assert!(!fixture.root.path().join("successor-ready").exists());
    fixture.child.kill().unwrap();
    fixture.child.wait().unwrap();
    assert!(relay.wait_exit(Duration::from_secs(3)));
    assert!(!fixture.root.path().join("successor-ready").exists());
}
#[test]
fn missing_entries_stop_the_verified_parent_and_owned_worker_within_35_seconds() {
    let started = Instant::now();
    let mut fixture = Fixture::start("resident-orphan");
    let pid = std::fs::read_to_string(fixture.root.path().join("worker-pid"))
        .unwrap()
        .parse()
        .unwrap();
    let worker = process::Target::open(
        pid,
        &std::env::current_exe().unwrap().canonicalize().unwrap(),
    )
    .unwrap();
    let deadline = started + Duration::from_secs(35);
    while fixture.child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "orphan process exceeded its deadline"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fixture.root.path().join("drain-requested").exists(),
        "bounded graceful drain was not attempted"
    );
    assert!(
        worker.wait_exit(Duration::from_secs(5)),
        "owned worker survived parent job teardown"
    );
}
#[test]
fn shell_tray_is_a_real_return_path_and_normal_exit_removes_it() {
    let mut fixture = Fixture::start("resident-tray");
    let guid = std::fs::read_to_string(fixture.root.path().join("entry-guid")).unwrap();
    let probe = crate::residency::Probe {
        tray: Some(guid),
        ..Default::default()
    };
    assert_eq!(
        crate::residency::windows::observe(fixture.child.id(), &probe),
        crate::residency::policy::Observation::Present
    );
    std::thread::sleep(Duration::from_secs(3));
    assert!(fixture.child.try_wait().unwrap().is_none());
    std::fs::write(fixture.root.path().join("stop"), b"stop").unwrap();
    fixture.child.wait().unwrap();
    assert_ne!(
        crate::residency::windows::observe(fixture.child.id(), &probe),
        crate::residency::policy::Observation::Present
    );
}
