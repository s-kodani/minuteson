use crate::audio::{PcmFormat, VerifiedAudio};
use crate::audio_clock::{ClockChunk, ClockLayout, ClockSegment};
use crate::capture_scope::ApplicationIdentity;
use crate::playback::{AudioSpan, PreparedAudio};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

const MANIFEST_LIMIT: u64 = 256 * 1024;
const AUDIO_LIMIT: u64 = 128 * 1024 * 1024;

#[cfg(all(test, windows))]
#[test]
fn windows_five_second_synthetic_capture_validates_and_releases_audio() {
    windows_synthetic_capture_case("--synthetic-clock-overlap");
}

#[cfg(all(test, windows))]
#[test]
fn windows_five_second_clock_anomalies_validate_and_release_audio() {
    windows_synthetic_capture_case("--synthetic-clock-cases");
}

#[cfg(all(test, windows))]
fn windows_synthetic_capture_case(scenario: &str) {
    let directory = TestDirectory::new().unwrap();
    let request = super::Request {
        run_id: uuid::Uuid::new_v4().to_string(),
        microphone_uid: "synthetic-microphone".into(),
        capture_scope: crate::capture_scope::CaptureScope {
            mode: "system".into(),
            application: None,
        },
        expected_import: super::ImportExpectation {
            token: None,
            revision: 0,
            busy: false,
        },
    };
    let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe");
    let child = super::command(&binary, &directory.path, &request)
        .unwrap()
        .arg("--synthetic")
        .arg(scenario)
        .args(["--synthetic-startup-delay-ms", "500"])
        .spawn()
        .unwrap();
    let child = super::Probe(std::sync::Mutex::new(child));
    let start = std::time::Instant::now();
    loop {
        if let Some(manifest) = directory.manifest().unwrap() {
            assert_eq!(manifest.state, "starting");
            manifest.require_owner(&request).unwrap();
            assert!(verify(&directory, &request, &|| false).is_err());
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert!(child.exited().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Reproduce a Windows reader/scanner denying replacement during startup.
    // The helper must retain and atomically publish the same staged snapshot.
    use std::os::windows::fs::OpenOptionsExt;
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(directory.path.join("manifest.json"))
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(reader);
    });
    let transitions = std::cell::Cell::new(0);
    super::watch(
        &directory,
        &child,
        &request,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(15),
        || {
            transitions.set(transitions.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    release.join().unwrap();
    assert_eq!(transitions.get(), 1);
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(raw["consoleWindowPresent"], false);
    if scenario == "--synthetic-clock-overlap" {
        // Overlap now switches to the monotone sample map before any raw
        // segment is sealed, rather than adjusting an already appended map.
        assert_eq!(raw["timingEstimated"], true);
        let diagnostic: serde_json::Value =
            serde_json::from_reader(directory.open("capture-clock.json", 128 * 1024).unwrap())
                .unwrap();
        for lane in ["microphone", "system"] {
            assert_eq!(raw[lane]["clockBoundaryAdjustments"], 0);
            assert_eq!(raw[lane]["frames"], 240_000);
            assert_eq!(diagnostic[lane]["estimatedPackets"], 499);
            assert_eq!(
                diagnostic[lane]["firstAnomalies"][0]["reason"],
                "overlapping_timestamp"
            );
        }
    } else {
        assert_eq!(raw["timingEstimated"], true);
        let diagnostic: serde_json::Value =
            serde_json::from_reader(directory.open("capture-clock.json", 128 * 1024).unwrap())
                .unwrap();
        assert_eq!(diagnostic["state"], "saved");
        assert_eq!(diagnostic["ownerId"], request.run_id);
        for lane in ["microphone", "system"] {
            assert_eq!(diagnostic[lane]["equalTimestampPackets"], 1);
            assert_eq!(diagnostic[lane]["reversedTimestampPackets"], 2);
            assert_eq!(diagnostic[lane]["timestampErrorPackets"], 1);
            assert!(
                diagnostic[lane]["sampleCountFallbackPackets"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            assert!(
                diagnostic[lane]["devicePositionFallbackPackets"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            assert_eq!(diagnostic[lane]["savedFrames"], raw[lane]["frames"]);
            // All selected PCM survives. The only excluded frames are beyond
            // the shared five-second window, including the known 20 ms gap.
            let received = diagnostic[lane]["observedPackets"].as_u64().unwrap() * 480;
            assert_eq!(raw[lane]["frames"], received.min(240_000 - 960));
            let events = diagnostic[lane]["firstAnomalies"].as_array().unwrap();
            assert!(events.iter().any(|e| e["reason"] == "equal_timestamp"));
            assert!(events.iter().any(|e| e["reason"] == "reversed_timestamp"));
            assert!(events.len() <= 16);
            assert!(diagnostic[lane]["recentPackets"].as_array().unwrap().len() <= 32);
        }
    }
    let ready = verify(&directory, &request, &|| false).unwrap();
    assert!(ready.microphone.as_ref().unwrap().info.frames > 0);
    assert!(ready.system.is_some());
    directory.cleanup().unwrap();
    assert!(!directory.path.join("microphone.caf").exists());
    assert!(!directory.path.join("system.caf").exists());
    assert!(!directory.path.join("capture-clock.json").exists());
}

#[cfg(all(test, windows))]
#[test]
fn windows_five_second_initialization_failure_retains_the_primary_error() {
    let directory = TestDirectory::new().unwrap();
    let mut request = super::tests::request();
    request.microphone_uid = "synthetic-microphone".into();
    let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../probes/windows-capture/build/Release/mon-windows-capture.exe");
    let child = super::command(&binary, &directory.path, &request)
        .unwrap()
        .args(["--synthetic", "--parent-pid", "4294967295"])
        .spawn()
        .unwrap();
    let child = super::Probe(std::sync::Mutex::new(child));
    let error = super::watch(
        &directory,
        &child,
        &request,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(5),
        || panic!("failed initialization cannot enter recording"),
    )
    .unwrap_err();
    assert_eq!(error, "capture parent unavailable");
    let diagnostic: serde_json::Value =
        serde_json::from_reader(directory.open("capture-clock.json", 128 * 1024).unwrap()).unwrap();
    assert_eq!(diagnostic["state"], "failed");
    assert_eq!(diagnostic["error"], "capture parent unavailable");
    assert!(verify(&directory, &request, &|| false).is_err());
    assert!(!directory.path.join("microphone.caf").exists());
    assert!(!directory.path.join("system.caf").exists());
    child.terminate().unwrap();
}
#[cfg(unix)]
const FILES: [&str; 5] = [
    "manifest.json",
    "microphone.caf",
    "system.caf",
    "input-levels.json",
    "stop.request",
];
#[cfg(windows)]
const FILES: [&str; 11] = [
    "manifest.json",
    "microphone.caf",
    "system.caf",
    "input-levels.json",
    "stop.request",
    "input-status.json",
    "microphone-selection.status.json",
    "microphone-selection.request",
    "capture-clock.json",
    "input-events.json",
    "input-journal-status.json",
];

/// Only this newly allocated directory is owned. Cleanup never recursively
/// traverses a cache, follows links, or removes unknown files after a crash.
pub(super) struct TestDirectory {
    pub(super) path: PathBuf,
    pin: File,
    #[cfg(any(target_os = "macos", windows))]
    owner: Option<super::journal::Area>,
}
impl TestDirectory {
    #[cfg(any(test, not(target_os = "macos")))]
    pub(super) fn new() -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix("mon-owned-capture-test-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let path = directory.path().canonicalize().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        crate::windows_files::set_private(&path)?;
        #[cfg(windows)]
        let pin = crate::windows_files::open_metadata(&path)?;
        #[cfg(not(windows))]
        let pin = crate::input_levels::open_capture_directory(&path)?;
        let _ = directory.keep();
        Ok(Self {
            path,
            pin,
            #[cfg(any(target_os = "macos", windows))]
            owner: None,
        })
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn new_owned(
        registry: &std::sync::Arc<super::journal::Registry>,
        request: super::Request,
    ) -> Result<Self, String> {
        Self::restore_owned(registry.allocate(request)?)
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn restore_owned(owner: super::journal::Area) -> Result<Self, String> {
        Ok(Self {
            path: owner.path(),
            pin: owner.pin_directory()?,
            owner: Some(owner),
        })
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn recover_owned(owner: super::journal::Area) -> Result<(), String> {
        let quiet = owner.quiet()?;
        match std::fs::symlink_metadata(owner.path()) {
            // A previous collector may have removed the audio and stopped
            // before committing completion. Missing audio is not a new target.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => owner.finish(&quiet),
            Err(error) => Err(error.to_string()),
            Ok(_) => {
                let directory = Self::restore_owned(owner)?;
                directory.cleanup_files()?;
                directory.owner.as_ref().unwrap().finish(&quiet)
            }
        }
    }
    #[cfg(any(target_os = "macos", windows))]
    pub(super) fn inherit_lease(&self, command: &mut std::process::Command) -> Result<i32, String> {
        self.check_owner()?;
        self.owner
            .as_ref()
            .ok_or("テスト音声の永続所有がありません")?
            .inherit(command)
    }
    pub(super) fn release_parent_lease(&self) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(owner) = &self.owner {
            owner.release_parent_lease();
        }
    }
    fn check_owner(&self) -> Result<(), String> {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(owner) = &self.owner {
            owner.validate_directory()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
            let pinned = self.pin.metadata().map_err(|e| e.to_string())?;
            if !current.is_dir() || current.dev() != pinned.dev() || current.ino() != pinned.ino() {
                return Err(
                    "テストの一時保存先が変更されています。別のデータには触れていません".into(),
                );
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let current = crate::windows_files::directory(&self.path)?;
            if crate::windows_files::identity(&current)?
                != crate::windows_files::identity(&self.pin)?
            {
                return Err("テストの一時保存先が変更されています".into());
            }
            Ok(())
        }
    }
    pub(super) fn open(&self, name: &str, maximum: u64) -> Result<File, String> {
        if !FILES.contains(&name) {
            return Err("テスト音声の固定名が不正です".into());
        }
        self.check_owner()?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::fs::MetadataExt;
            let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
            let fd = unsafe {
                libc::openat(
                    self.pin.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > maximum {
                return Err("テストのファイルが通常ファイルではないか、大きすぎます".into());
            }
            self.check_owner()?;
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let file = crate::windows_files::read(
                &crate::windows_files::path(&self.pin)?.join(name),
                true,
            )?;
            if file.metadata().map_err(|e| e.to_string())?.len() > maximum {
                return Err("テストのファイルが大きすぎます".into());
            }
            self.check_owner()?;
            Ok(file)
        }
    }
    pub(super) fn request_stop(&self) -> Result<(), String> {
        self.check_owner()?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = c"stop.request";
            let fd = unsafe {
                libc::openat(
                    self.pin.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW
                        | libc::O_CREAT
                        | libc::O_EXCL,
                    0o600,
                )
            };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    self.open("stop.request", 0)?;
                    return Ok(());
                }
                return Err(error.to_string());
            }
            unsafe { File::from_raw_fd(fd) }
                .sync_all()
                .map_err(|e| e.to_string())
        }
        #[cfg(not(unix))]
        {
            let path = crate::windows_files::path(&self.pin)?.join("stop.request");
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => file.sync_all().map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.open("stop.request", 0).map(|_| ())
                }
                Err(e) => Err(e.to_string()),
            }
        }
    }
    pub(super) fn cleanup(&self) -> Result<(), String> {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(owner) = &self.owner {
            // A parent crash or dropped wrapper is not evidence of the
            // helper's exit. Hold a separately acquired lease while removing
            // the bounded fixed-name payload, then commit its completion.
            let quiet = owner.quiet()?;
            self.cleanup_files()?;
            return owner.finish(&quiet);
        }
        self.cleanup_files()
    }
    fn cleanup_files(&self) -> Result<(), String> {
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        self.check_owner()?;
        let entries = std::fs::read_dir(&self.path)
            .map_err(|e| e.to_string())?
            .take(FILES.len() + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // Validate the entire bounded set before removing even one file.
        if entries.len() > FILES.len() {
            return Err("一時保存先に未登録のファイルがあります。削除していません".into());
        }
        #[cfg(windows)]
        let mut identities = std::collections::BTreeMap::new();
        for entry in &entries {
            let name = entry.file_name();
            let name = name.to_str().ok_or("一時ファイル名が不正です")?;
            let validated = self.open(
                name,
                if name == "manifest.json" {
                    MANIFEST_LIMIT
                } else if name == "input-levels.json" {
                    16 * 1024
                } else if name == "capture-clock.json" {
                    128 * 1024
                } else if name == "stop.request" {
                    0
                } else {
                    AUDIO_LIMIT
                },
            )?;
            #[cfg(windows)]
            identities.insert(
                entry.file_name(),
                crate::windows_files::Identity::file(&validated)?,
            );
            #[cfg(not(windows))]
            let _ = validated;
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            for entry in entries {
                self.check_owner()?;
                let name = std::ffi::CString::new(entry.file_name().as_encoded_bytes())
                    .map_err(|e| e.to_string())?;
                if unsafe { libc::unlinkat(self.pin.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            self.check_owner()?;
            std::fs::remove_dir(&self.path).map_err(|e| e.to_string())
        }
        #[cfg(not(unix))]
        {
            for entry in entries {
                self.check_owner()?;
                let path = crate::windows_files::path(&self.pin)?.join(entry.file_name());
                let expected = identities
                    .get(&entry.file_name())
                    .ok_or("テストファイルの検証証拠がありません")?;
                crate::windows_files::remove_owned(&path, expected, false)?;
            }
            let expected = crate::windows_files::Identity::file(&self.pin)?;
            crate::windows_files::remove_owned(&self.path, &expected, true)
        }
    }
    pub(super) fn manifest(&self) -> Result<Option<Manifest>, String> {
        if !self
            .path
            .join("manifest.json")
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        let file = self.open("manifest.json", MANIFEST_LIMIT)?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&file)
            .take(MANIFEST_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().map_err(|e| e.to_string())?
                != after.modified().map_err(|e| e.to_string())?
        {
            return Err("テストの保存記録が読取り中に変更されました".into());
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string())
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Window {
    owner_id: String,
    duration_seconds: u8,
    window_start_ns: Option<i64>,
    window_end_ns: Option<i64>,
    first_saved_capture_ns: Option<i64>,
    completed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaptureClock {
    domain: String,
    origin_utc_ms: i64,
    uncertainty_ns: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    callback_count: Option<u64>,
    frames: u64,
    sample_rate: Option<f64>,
    channels: Option<u32>,
    append_failures: u64,
    clock_segments: Option<Vec<ClockSegment>>,
    clock_failure: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Manifest {
    schema_version: u8,
    capture_mode: String,
    backend: String,
    #[serde(rename = "targetPIDs")]
    target_pids: Vec<i32>,
    #[serde(rename = "microphoneDeviceUID")]
    microphone_uid: Option<String>,
    target_application_identity: Option<ApplicationIdentity>,
    pub(super) state: String,
    errors: Vec<String>,
    #[serde(default)]
    system_audio_unreceived: bool,
    #[serde(default)]
    microphone_audio_unreceived: bool,
    capture_test: Window,
    capture_clock: Option<CaptureClock>,
    microphone: Track,
    system: Track,
}
impl Manifest {
    // This path presents a failed initialization, never accepts recorded audio.
    // The requested owner/scope stay mandatory; an identity not yet obtained by
    // the helper must not conceal its primary initialization error.
    pub(super) fn failure(&self, request: &super::Request) -> Result<Option<String>, String> {
        if self.state != "failed" {
            return Ok(None);
        }
        if u32::from(self.schema_version) != crate::current_formats::CAPTURE
            || self.capture_test.owner_id != request.run_id
            || self.capture_test.duration_seconds != 5
            || self.capture_mode != request.capture_scope.mode
            || self.backend != crate::recorder::capture_backend()
            || self.target_pids
                != request
                    .capture_scope
                    .application
                    .as_ref()
                    .map(|a| vec![a.pid])
                    .unwrap_or_default()
            || self
                .target_application_identity
                .as_ref()
                .is_some_and(|identity| {
                    Some(identity) != request.capture_scope.application.as_ref()
                })
        {
            return Err("テストの所有または取得範囲が開始条件と一致しません".into());
        }
        if self
            .microphone_uid
            .as_ref()
            .is_some_and(|uid| uid != &request.microphone_uid)
        {
            return Err("テストのマイクが開始条件と一致しません".into());
        }
        let reason = self
            .errors
            .iter()
            .filter(|error| !error.trim().is_empty())
            .take(3)
            .map(|error| error.chars().take(1200).collect::<String>())
            .collect::<Vec<_>>()
            .join(" / ");
        Ok(Some(if reason.is_empty() {
            "録音ヘルパーの初期化または保存に失敗しました。権限・機器・保存先を確認してください"
                .into()
        } else {
            reason
        }))
    }

    pub(super) fn require_owner(&self, request: &super::Request) -> Result<(), String> {
        if u32::from(self.schema_version) != crate::current_formats::CAPTURE
            || self.capture_test.owner_id != request.run_id
            || self.capture_test.duration_seconds != 5
            || self.capture_mode != request.capture_scope.mode
            || self.backend != crate::recorder::capture_backend()
            || self.state != "starting"
                && self.target_application_identity != request.capture_scope.application
            || self.target_pids
                != request
                    .capture_scope
                    .application
                    .as_ref()
                    .map(|a| vec![a.pid])
                    .unwrap_or_default()
        {
            return Err("テストの所有または取得範囲が開始条件と一致しません".into());
        }
        if self.state != "starting"
            && self.microphone_uid.as_deref() != Some(&request.microphone_uid)
        {
            return Err("テストのマイクが開始条件と一致しません".into());
        }
        if self.state == "recording" || self.state == "saved" {
            let start = self
                .capture_test
                .window_start_ns
                .ok_or("テストの共通開始時刻がありません")?;
            let end = self
                .capture_test
                .window_end_ns
                .ok_or("テストの共通終了時刻がありません")?;
            if start < 0 || end.checked_sub(start) != Some(5_000_000_000) {
                return Err("テストの共通5秒枠が不正です".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TrackInfo {
    pub(super) frames: u64,
    pub(super) sample_rate: u32,
    pub(super) channels: u32,
    pub(super) duration_ms: u64,
    pub(super) has_gaps: bool,
}
pub(super) struct TrackProof {
    name: &'static str,
    size: u64,
    hash: String,
    format: PcmFormat,
    pub(super) info: TrackInfo,
}
pub(super) struct Ready {
    pub(super) microphone: Option<TrackProof>,
    pub(super) system: Option<TrackProof>,
}
impl TrackProof {
    pub(super) fn playback(
        &self,
        directory: &TestDirectory,
        kind: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedAudio, String> {
        let audio = VerifiedAudio::open_cancellable(
            directory.open(self.name, AUDIO_LIMIT)?,
            self.size,
            &self.hash,
            cancelled,
        )?;
        if audio.format.frames != self.format.frames
            || audio.format.channels != self.format.channels
            || audio.format.sample_rate != self.format.sample_rate
        {
            return Err("テスト音声の形式が変更されました".into());
        }
        Ok(PreparedAudio {
            sample_rate: self.format.sample_rate,
            channels: self.format.channels,
            start_us: 0,
            end_us: (self.format.frames * 1_000_000 / u64::from(self.format.sample_rate)) as i64,
            track_kind: kind.into(),
            alignment: "unverified".into(),
            spans: vec![AudioSpan {
                frames: self.format.frames,
                first_frame: 0,
                audio,
            }],
            clock: None,
        })
    }
}
pub(super) fn verify(
    directory: &TestDirectory,
    request: &super::Request,
    cancelled: &impl Fn() -> bool,
) -> Result<Ready, String> {
    let manifest = directory
        .manifest()?
        .ok_or("テストの保存記録がありません")?;
    manifest.require_owner(request)?;
    if manifest.state != "saved" || !manifest.errors.is_empty() || !manifest.capture_test.completed
    {
        return Err("5秒テストの停止・保存完了を確認できません".into());
    }
    let first = manifest.capture_test.first_saved_capture_ns.ok_or(
        "テストの保存音声の取得原点がありません。同じソースのヘルパーを再構築してください",
    )?;
    let start = manifest
        .capture_test
        .window_start_ns
        .ok_or("テストの開始時刻がありません")?;
    let end = manifest
        .capture_test
        .window_end_ns
        .ok_or("テストの終了時刻がありません")?;
    let clock = manifest
        .capture_clock
        .ok_or("テストの取得時計がありません")?;
    if first < start
        || first >= end
        || end.checked_sub(start) != Some(5_000_000_000)
        || !matches!(
            clock.domain.as_str(),
            "mach_continuous_time" | "query_performance_counter"
        )
        || !(0..=8_640_000_000_000_000).contains(&clock.origin_utc_ms)
        || !(0..=9_007_199_254_740_991).contains(&clock.uncertainty_ns)
    {
        return Err("テストの保存音声の共通原点が不正です".into());
    }
    let track = |name, t: Track| -> Result<TrackProof, String> {
        if cancelled() {
            return Err("テストの音声検証を取り消しました".into());
        }
        let rate = t.sample_rate.ok_or("テストの音声形式がありません")?;
        if !rate.is_finite()
            || rate.fract() != 0.0
            || !(8000.0..=384000.0).contains(&rate)
            || t.frames == 0
            || t.frames > rate as u64 * 5
            || t.append_failures != 0
            || t.clock_failure.is_some()
            || t.callback_count == Some(0)
            || !(1..=8).contains(&t.channels.unwrap_or(0))
        {
            return Err("テスト音声のフレーム数・形式・書込み状態が不正です".into());
        }
        let segments = t.clock_segments.ok_or("テストの取得時刻情報がありません")?;
        if segments.is_empty() || segments.len() > 1024 {
            return Err("テストの取得時刻情報が不正です".into());
        }
        let layout = ClockLayout::new(
            &[ClockChunk {
                frames: t.frames as i64,
                start_us: segments[0].timeline_start_us,
                end_us: segments
                    .last()
                    .ok_or("テストの取得時刻がありません")?
                    .timeline_end_us,
                segments: &segments,
            }],
            rate as u32,
        )?;
        if i128::from(first) + i128::from(layout.end_us) * 1000 > i128::from(end) {
            return Err("テスト音声の取得時刻が共通5秒枠を超えています".into());
        }
        let (format, size, hash) =
            crate::audio::inspect_recorded_caf(directory.open(name, AUDIO_LIMIT)?, cancelled)?;
        if format.frames != t.frames
            || format.sample_rate != rate as u32
            || Some(format.channels) != t.channels
        {
            return Err("テストCAFの実体と保存記録が一致しません".into());
        }
        let info = TrackInfo {
            frames: t.frames,
            sample_rate: format.sample_rate,
            channels: format.channels,
            duration_ms: t.frames * 1000 / u64::from(format.sample_rate),
            has_gaps: layout.start_us > 0
                || layout.spans.windows(2).any(|s| s[0].end_us < s[1].start_us),
        };
        Ok(TrackProof {
            name,
            size,
            hash,
            format,
            info,
        })
    };
    let system_unreceived = manifest.system_audio_unreceived;
    let microphone_unreceived = manifest.microphone_audio_unreceived;
    if system_unreceived && microphone_unreceived {
        return Err("マイク・PC音声を取得できませんでした".into());
    }
    for (unreceived, name, t) in [
        (microphone_unreceived, "microphone.caf", &manifest.microphone),
        (system_unreceived, "system.caf", &manifest.system),
    ] {
        if !unreceived { continue; }
        if t.frames != 0
            || t.callback_count != Some(0)
            || t.sample_rate.is_some()
            || t.channels.is_some()
            || t.append_failures != 0
            || t.clock_failure.is_some()
            || t.clock_segments
                .as_ref()
                .is_none_or(|segments| !segments.is_empty())
        {
            return Err(format!("{name}の未受信記録に書込み・形式・取得時刻の矛盾があります"));
        }
        // Absence is not an empty/corrupt saved CAF. Reject even a dangling
        // link; never discard a payload or pass it off as unreceived audio.
        match std::fs::symlink_metadata(directory.path.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(format!("{name}の未受信記録に音声ファイルが存在します")),
        }
    }
    let first_start = |unreceived: bool, track: &Track| {
        if unreceived { i64::MAX } else {
            track.clock_segments.as_ref().and_then(|s| s.first())
                .map(|s| s.timeline_start_us).unwrap_or(-1)
        }
    };
    if first_start(microphone_unreceived, &manifest.microphone)
        .min(first_start(system_unreceived, &manifest.system)) != 0
    {
        return Err("テストの共通原点が最初の保存音声と一致しません".into());
    }
    Ok(Ready {
        microphone: if microphone_unreceived { None } else { Some(track("microphone.caf", manifest.microphone)?) },
        system: if system_unreceived {
            None
        } else {
            Some(track("system.caf", manifest.system)?)
        },
    })
}
