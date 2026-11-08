//! Private child processes exercise the restart relay and residency monitor.
use super::*;
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::Pipes::*, UI::WindowsAndMessaging::*};

struct Fixture {
    child: Child,
    root: tempfile::TempDir,
}
impl Fixture {
    fn start(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        crate::native_fs::private_directories(root.path(), true).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "startup_instance::windows::tests::process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_STARTUP_FIXTURE_ROOT", root.path())
            .env("MON_STARTUP_FIXTURE_MODE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let fixture = Self { child, root };
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
        // Existence is the parent's readiness signal. Publish only after the
        // PID is fully written so it cannot observe a created-but-empty file.
        let pending = root.join("successor-ready.pending");
        std::fs::write(&pending, std::process::id().to_string()).unwrap();
        std::fs::rename(pending, root.join("successor-ready")).unwrap();
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
    panic!("unknown fixture mode: {mode}");
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
