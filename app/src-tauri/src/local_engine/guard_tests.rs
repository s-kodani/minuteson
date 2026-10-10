use super::*;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(super) fn executable() -> Result<PathBuf, String> {
    static BINARY: OnceLock<Result<(tempfile::TempDir, PathBuf), String>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            // Compile the exact production headless core, without a test harness's
            // stdout banner or any Tauri/UI startup. No model/device is loaded.
            let deps = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .ok_or("test binary has no parent")?
                .to_path_buf();
            let mut libs = std::fs::read_dir(deps)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("liblibc-") && n.ends_with(".rlib"))
                })
                .collect::<Vec<_>>();
            libs.sort_by_key(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()));
            let libc = libs.last().ok_or("compiled libc dependency is missing")?;
            let directory = tempfile::Builder::new()
                .prefix("mon-owned-engine-core-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            let output = directory.path().join("guard");
            let result = Command::new("rustc")
                .args([
                    "--edition=2021",
                    "-C",
                    "opt-level=1",
                    "-C",
                    "debug-assertions=yes",
                ])
                .arg("--extern")
                .arg(format!("libc={}", libc.display()))
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/local_engine/guard_harness.rs"),
                )
                .arg("-o")
                .arg(&output)
                .output()
                .map_err(|e| e.to_string())?;
            if !result.status.success() {
                return Err(format!(
                    "headless core test build failed: {}",
                    String::from_utf8_lossy(&result.stderr[..result.stderr.len().min(65_536)])
                ));
            }
            Ok((directory, output))
        })
        .as_ref()
        .map(|(_, p)| p.clone())
        .map_err(Clone::clone)
}

fn wait_for<T>(mut read: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(
            Instant::now() < end,
            "owned process did not reach the expected state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|p| *p > 1)
}
fn gone(pid: i32) -> bool {
    // Read-only probe. Never signal a PID adopted from a later scan or receipt.
    (unsafe { libc::kill(pid, 0) }) != 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}
const SCRIPT: &str = "echo $$ > engine-pid; /bin/sleep 30 & echo $! > descendant-pid; wait";
fn sleeping(directory: &Path) -> ManagedChild {
    let mut launch = OwnedCommand::new(Path::new("/bin/sh")).unwrap();
    launch
        .command
        .env_clear()
        .current_dir(directory)
        .args(["-c", SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    launch.spawn().unwrap()
}

#[test]
fn owned_engine_guard_preserves_exact_output_exit_and_environment_without_gui() {
    let mut launch = OwnedCommand::new(Path::new("/bin/sh")).unwrap();
    launch
        .command
        .env_clear()
        .env("MON_TEST_FIXED", "literal ; $(not-executed)")
        .args([
            "-c",
            "printf '%s' \"$MON_TEST_FIXED\"; printf error >&2; exit 27",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = launch.spawn().unwrap();
    let mut out = String::new();
    child
        .child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let mut err = String::new();
    child
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert_eq!(child.child.wait().unwrap().code(), Some(27));
    assert_eq!(out, "literal ; $(not-executed)");
    assert_eq!(err, "error");
    assert!(launch.spawn().is_err());
}

#[test]
fn owned_engine_guard_drop_stops_and_reaps_engine_and_descendant_group() {
    let root = tempfile::tempdir().unwrap();
    let child = sleeping(root.path());
    let guard = child.child.id() as i32;
    let engine = wait_for(|| pid(&root.path().join("engine-pid")));
    let descendant = wait_for(|| pid(&root.path().join("descendant-pid")));
    assert!(!gone(engine));
    assert!(!gone(descendant));
    drop(child);
    wait_for(|| gone(guard).then_some(()));
    wait_for(|| gone(engine).then_some(()));
    wait_for(|| gone(descendant).then_some(()));
}

#[test]
fn owned_engine_guard_normal_exit_cleans_remaining_group_before_reaping_leader() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = OwnedCommand::new(Path::new("/bin/sh")).unwrap();
    launch
        .command
        .env_clear()
        .current_dir(root.path())
        .args([
            "-c",
            "echo $$ > engine-pid; /bin/sleep 30 & echo $! > descendant-pid; exit 0",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = launch.spawn().unwrap();
    assert!(child.child.wait().unwrap().success());
    let engine = wait_for(|| pid(&root.path().join("engine-pid")));
    let descendant = wait_for(|| pid(&root.path().join("descendant-pid")));
    wait_for(|| gone(engine).then_some(()));
    wait_for(|| gone(descendant).then_some(()));
}

#[test]
fn owned_engine_guard_rejects_missing_owner_and_wrong_parent_before_engine_start() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(executable().unwrap())
        .args([
            guard_core::ENTRY,
            &std::process::id().to_string(),
            "3",
            "/bin/sh",
            "--",
            "-c",
            "touch must-not-exist",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert!(!root.path().join("must-not-exist").exists());
    // A wrong parent is rejected before any channel or engine can be adopted.
    let output = Command::new(executable().unwrap())
        .args([
            guard_core::ENTRY,
            &(std::process::id() + 1).to_string(),
            "3",
            "/usr/bin/true",
            "--",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no longer the direct parent"));
}

#[test]
#[ignore = "only the owned-engine SIGKILL parent launches this synthetic child"]
fn owned_engine_guard_sigkill_child() {
    let root = PathBuf::from(std::env::var("MON_OWNED_ENGINE_CHILD_ROOT").unwrap());
    assert!(root.is_absolute() && root.is_dir());
    assert_eq!(
        std::fs::read(root.join("owner-test-marker")).unwrap(),
        b"synthetic owned engine only"
    );
    let child = sleeping(&root);
    std::fs::write(root.join("guard-pid"), child.child.id().to_string()).unwrap();
    std::thread::sleep(Duration::from_secs(30));
    drop(child);
}

#[test]
fn owned_engine_guard_parent_sigkill_stops_owned_group_and_preserves_unrelated_process() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("owner-test-marker"),
        b"synthetic owned engine only",
    )
    .unwrap();
    let mut unrelated = ManagedChild::direct(Command::new("/bin/sleep").arg("30").spawn().unwrap());
    let mut parent = ManagedChild::direct(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "local_engine::guard_tests::owned_engine_guard_sigkill_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_OWNED_ENGINE_CHILD_ROOT", root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let guard = wait_for(|| pid(&root.path().join("guard-pid")));
    let engine = wait_for(|| pid(&root.path().join("engine-pid")));
    let descendant = wait_for(|| pid(&root.path().join("descendant-pid")));
    assert!(!gone(guard) && !gone(engine) && !gone(descendant));
    parent.child.kill().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(parent.child.wait().unwrap().signal(), Some(libc::SIGKILL));
    wait_for(|| gone(guard).then_some(()));
    wait_for(|| gone(engine).then_some(()));
    wait_for(|| gone(descendant).then_some(()));
    assert!(unrelated.child.try_wait().unwrap().is_none());
    assert_eq!(
        std::fs::read(root.path().join("owner-test-marker")).unwrap(),
        b"synthetic owned engine only"
    );
}
